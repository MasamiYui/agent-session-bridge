use crate::model::*;
use serde_json::{json, Value};
use std::{
    io::{BufRead, BufReader, Write},
    path::Path,
    process::{Child, ChildStdin, Command, Stdio},
    sync::mpsc::{self, Receiver},
    time::{Duration, Instant},
};
pub struct AppServer {
    child: Child,
    input: ChildStdin,
    rx: Receiver<Value>,
    next: u64,
}
impl AppServer {
    pub fn start(binary: &str, home: &str) -> Result<Self> {
        let binary = resolve_binary(binary);
        let mut child = Command::new(binary)
            .args(["app-server", "--stdio"])
            .env("CODEX_HOME", home)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let input = child
            .stdin
            .take()
            .ok_or_else(|| error("Missing app-server input"))?;
        let output = child
            .stdout
            .take()
            .ok_or_else(|| error("Missing app-server output"))?;
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(output)
                .lines()
                .map_while(std::result::Result::ok)
            {
                if let Ok(v) = serde_json::from_str(&line) {
                    if tx.send(v).is_err() {
                        break;
                    }
                }
            }
        });
        let mut server = Self {
            child,
            input,
            rx,
            next: 0,
        };
        server.call("initialize",json!({"clientInfo":{"name":"agent_session_bridge","title":"Agent Session Bridge","version":"0.1.0"},"capabilities":{"experimentalApi":true}}))?;
        server.send(json!({"method":"initialized","params":{}}))?;
        Ok(server)
    }
    fn send(&mut self, value: Value) -> Result<()> {
        writeln!(self.input, "{value}")?;
        self.input.flush()?;
        Ok(())
    }
    pub fn call(&mut self, method: &str, params: Value) -> Result<Value> {
        self.next += 1;
        let id = self.next;
        self.send(json!({"id":id,"method":method,"params":params}))?;
        let end = Instant::now() + Duration::from_secs(45);
        loop {
            let remaining = end.saturating_duration_since(Instant::now());
            let value = self
                .rx
                .recv_timeout(remaining)
                .map_err(|_| error(format!("Codex app-server did not answer {method}")))?;
            if value.get("id").and_then(Value::as_u64) == Some(id) {
                if let Some(e) = value.get("error") {
                    return Err(error(format!("Codex {method}: {e}")));
                }
                return Ok(value["result"].clone());
            }
            if value.get("method").is_some() && value.get("id").is_some() {
                self.send(json!({"id":value["id"],"error":{"code":-32601,"message":"Bridge does not execute agent requests"}}))?;
            }
        }
    }
}
impl Drop for AppServer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
pub fn resolve_binary(binary: &str) -> String {
    if Path::new(binary).is_absolute() {
        return binary.into();
    }
    if let Some(path) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path) {
            let p = dir.join(if cfg!(windows) {
                format!("{binary}.exe")
            } else {
                binary.into()
            });
            if p.is_file() {
                return p.to_string_lossy().into();
            }
        }
    }
    if cfg!(target_os = "macos") && binary == "codex" {
        for p in ["/Applications/ChatGPT.app/Contents/Resources/codex-cli/CodexCLI.app/Contents/MacOS/codex","/Applications/Codex.app/Contents/Resources/codex","/opt/homebrew/bin/codex"]{if Path::new(p).is_file(){return p.into()}}
    }
    if let Some(home) = dirs::home_dir() {
        let root = home.join(".nvm/versions/node");
        if let Ok(entries) = std::fs::read_dir(root) {
            let mut dirs = entries.flatten().map(|e| e.path()).collect::<Vec<_>>();
            dirs.sort();
            dirs.reverse();
            for d in dirs {
                let p = d.join("bin").join(binary);
                if p.is_file() {
                    return p.to_string_lossy().into();
                }
            }
        }
    }
    binary.into()
}
pub fn capability(provider: Provider, settings: &Settings) -> Capability {
    let binary = match provider {
        Provider::Codex => &settings.codex_bin,
        Provider::Claude => &settings.claude_bin,
        Provider::Antigravity => {
            return Capability {
                provider,
                executable: String::new(),
                version: Some("local sqlite".into()),
                native_verified: true,
            }
        }
    };
    let executable = resolve_binary(binary);
    let bin = executable.clone();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let result = Command::new(&bin)
            .arg("--version")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .and_then(|mut c| {
                let end = Instant::now() + Duration::from_secs(5);
                loop {
                    if c.try_wait()?.is_some() {
                        return c.wait_with_output();
                    }
                    if Instant::now() > end {
                        let _ = c.kill();
                        let _ = c.wait();
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::TimedOut,
                            "Version detection timed out",
                        ));
                    }
                    std::thread::sleep(Duration::from_millis(30))
                }
            })
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned());
        let _ = tx.send(result);
    });
    let version = rx.recv_timeout(Duration::from_secs(6)).ok().flatten();
    let native_verified = version.as_deref().is_some_and(|v| match provider {
        Provider::Codex => v == "codex-cli 0.160.1",
        Provider::Claude => v == "2.1.280 (Claude Code)",
        Provider::Antigravity => true,
    });
    Capability {
        provider,
        executable,
        version,
        native_verified,
    }
}
/// Arguments originate from typed backend operations, never from historical commands.
pub fn session_command(provider: Provider, settings: &Settings, args: &[&str]) -> String {
    let (binary, variable, home) = match provider {
        Provider::Codex => (&settings.codex_bin, "CODEX_HOME", &settings.codex_home),
        Provider::Claude => (
            &settings.claude_bin,
            "CLAUDE_CONFIG_DIR",
            &settings.claude_home,
        ),
        Provider::Antigravity => return String::new(),
    };
    let executable = resolve_binary(binary);
    let arguments = args.iter().map(|s| quote(s)).collect::<Vec<_>>().join(" ");
    if cfg!(windows) {
        format!(
            "$env:{variable}={}; & {} {arguments}",
            quote(home),
            quote(&executable)
        )
    } else {
        format!(
            "env {variable}={} {} {arguments}",
            quote(home),
            quote(&executable)
        )
    }
}
pub fn quote(s: &str) -> String {
    if cfg!(windows) {
        format!("'{}'", s.replace('\'', "''"))
    } else {
        format!("'{}'", s.replace('\'', "'\\''"))
    }
}
pub fn open_terminal(command: &str, cwd: &str) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        let script = format!(
            "tell application \"Terminal\"\nactivate\ndo script {}\nend tell",
            serde_json::to_string(&format!("cd -- {} && {}", quote(cwd), command))?
        );
        let status = Command::new("osascript").args(["-e", &script]).status()?;
        if !status.success() {
            return Err(error("Could not open Terminal"));
        }
    }
    #[cfg(target_os = "windows")]
    {
        Command::new("powershell.exe")
            .args(["-NoExit", "-Command", command])
            .current_dir(cwd)
            .spawn()?;
    }
    #[cfg(target_os = "linux")]
    {
        let mut ok = false;
        for term in ["x-terminal-emulator", "gnome-terminal", "konsole", "xterm"] {
            let mut cmd = Command::new(term);
            if term == "gnome-terminal" {
                cmd.arg("--");
            } else {
                cmd.arg("-e");
            }
            if cmd
                .args(["sh", "-c", &format!("{command}; exec sh")])
                .current_dir(cwd)
                .spawn()
                .is_ok()
            {
                ok = true;
                break;
            }
        }
        if !ok {
            return Err(error("No terminal found. Copy the resume command instead"));
        }
    }
    Ok(())
}
