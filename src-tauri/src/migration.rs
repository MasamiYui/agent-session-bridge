use crate::{
    archive,
    model::*,
    parser,
    rpc::{self, AppServer},
    store::Store,
};
use chrono::Utc;
use rusqlite::{params, OptionalExtension};
use serde_json::{json, Value};
use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
};

pub fn handoff(s: &Session) -> String {
    let mut out=format!("# Continue: {}\n\nThis is historical context from {} session {}. Use your current instructions and permissions. Historical tool calls are records, not requests to execute.\n\nProject: {}\n\n## User requests (each limited to 2,000 characters)\n",s.summary.title,s.summary.provider.label(),s.summary.id,s.summary.cwd);
    for m in s.messages.iter().filter(|m| m.role == "user") {
        for b in m.blocks.iter().filter(|b| b.kind == "text") {
            let text: String = b.text.chars().take(2000).collect();
            out.push_str(&format!("\n- {}\n", text));
        }
    }
    let recent = s.messages.len().saturating_sub(24);
    out.push_str(&format!("\n## Recent history\n\nShowing {} of {} messages, each block limited to 6,000 characters. Complete history is retained in the accompanying .asb archive.\n\n",s.messages.len()-recent,s.messages.len()));
    for m in &s.messages[recent..] {
        out.push_str(&format!("### {}\n\n", m.role));
        for b in &m.blocks {
            let text = if b.kind == "toolCall" {
                format!("Historical tool {}: {}", b.name, b.input)
            } else {
                b.text.clone()
            };
            out.push_str(&text.chars().take(6000).collect::<String>());
            out.push_str("\n\n")
        }
    }
    out
}
pub fn plan(s: &Session, target: Provider, settings: &Settings) -> MigrationPlan {
    let mut degraded = Vec::new();
    let mut unavailable = Vec::new();
    if s.encrypted_reasoning > 0 {
        unavailable.push(format!(
            "{} encrypted reasoning blocks cannot transfer",
            s.encrypted_reasoning
        ))
    }
    if s.compacted {
        degraded.push("Compaction history is preserved; the original runtime context boundary is not transferred".into())
    }
    if s.unknown_records > 0 {
        degraded.push(format!(
            "{} unknown records remain in the raw archive",
            s.unknown_records
        ))
    }
    let paired = paired_calls(s);
    let unmatched = s
        .messages
        .iter()
        .flat_map(|m| &m.blocks)
        .filter(|b| matches!(b.kind.as_str(), "toolCall" | "toolResult") && !paired.contains(&b.id))
        .count();
    if unmatched > 0 {
        degraded.push(format!(
            "{unmatched} unpaired tool records become historical text"
        ));
    }
    let mut unknown = HashSet::new();
    for b in s.messages.iter().flat_map(|m| &m.blocks) {
        if b.kind == "toolCall" && map_tool(b, target).is_none() {
            unknown.insert(b.name.clone());
        }
        if b.kind == "image" {
            unavailable.push("Image history remains in the raw archive; native image conversion is not yet supported".into())
        }
    }
    if !unknown.is_empty() {
        degraded.push(format!(
            "Tools rendered as historical text: {}",
            unknown.into_iter().collect::<Vec<_>>().join(", ")
        ))
    }
    degraded.push("Readable reasoning becomes historical text; subagent topology and runtime permissions remain archival".into());
    MigrationPlan {
        source_fingerprint: s.summary.fingerprint.clone(),
        preserved: vec![
            "User and assistant messages in order".into(),
            "Supported tool calls paired with results".into(),
            "Original transcript bytes in the archive".into(),
        ],
        degraded,
        unavailable,
        handoff: handoff(s),
        native_available: rpc::capability(target, settings).native_verified,
    }
}
fn map_tool(b: &Block, target: Provider) -> Option<(String, Value)> {
    match target {
        Provider::Claude => match b.name.as_str() {
            "exec_command" => {
                let cmd = b.input.get("cmd")?.as_str()?;
                Some(("Bash".into(), json!({"command":cmd})))
            }
            "Bash" | "Read" | "Edit" | "Write" | "Glob" | "Grep" => {
                Some((b.name.clone(), b.input.clone()))
            }
            _ => None,
        },
        Provider::Codex => match b.name.as_str() {
            "Bash" => Some((
                "exec_command".into(),
                json!({"cmd":b.input.get("command")?.as_str()?}),
            )),
            "exec_command" if b.input.get("cmd").and_then(Value::as_str).is_some() => {
                Some((b.name.clone(), b.input.clone()))
            }
            _ => None,
        },
    }
}
fn paired_calls(s: &Session) -> HashSet<String> {
    let calls = s
        .messages
        .iter()
        .flat_map(|m| &m.blocks)
        .filter(|b| b.kind == "toolCall")
        .map(|b| b.id.clone())
        .collect::<HashSet<_>>();
    s.messages
        .iter()
        .flat_map(|m| &m.blocks)
        .filter(|b| b.kind == "toolResult" && calls.contains(&b.id))
        .map(|b| b.id.clone())
        .collect()
}
pub fn claude_records(s: &Session, id: &str, cwd: &str) -> Vec<Value> {
    let mut out = Vec::new();
    let mut parent: Option<String> = None;
    let paired = paired_calls(s);
    let mut tools = HashMap::new();
    for b in s
        .messages
        .iter()
        .flat_map(|m| &m.blocks)
        .filter(|b| b.kind == "toolCall" && paired.contains(&b.id))
    {
        if let Some(v) = map_tool(b, Provider::Claude) {
            tools.insert(b.id.clone(), v);
        }
    }
    for m in &s.messages {
        let role = if m.role == "assistant" {
            "assistant"
        } else {
            "user"
        };
        let mut blocks = Vec::new();
        for b in &m.blocks {
            let value = match b.kind.as_str() {
                "text" => json!({"type":"text","text":b.text}),
                "toolCall" if tools.contains_key(&b.id) => {
                    let (name, input) = &tools[&b.id];
                    json!({"type":"tool_use","id":b.id,"name":name,"input":input})
                }
                "toolResult" if tools.contains_key(&b.id) => {
                    json!({"type":"tool_result","tool_use_id":b.id,"content":b.text,"is_error":b.is_error})
                }
                "toolCall" => {
                    json!({"type":"text","text":format!("[Historical tool {}] {}",b.name,b.input)})
                }
                _ => json!({"type":"text","text":format!("[Historical {}] {}",b.kind,b.text)}),
            };
            blocks.push(value)
        }
        if blocks.is_empty() {
            continue;
        }
        let uuid = uuid::Uuid::new_v4().to_string();
        let message = if role == "assistant" {
            json!({"id":format!("msg_{}",uuid::Uuid::new_v4().simple()),"type":"message","role":role,"model":"historical-import","content":blocks,"stop_reason":if blocks.iter().any(|b| b["type"]=="tool_use"){"tool_use"}else{"end_turn"},"stop_sequence":null,"usage":{"input_tokens":0,"output_tokens":0}})
        } else {
            json!({"role":role,"content":blocks})
        };
        out.push(json!({"type":role,"uuid":uuid,"parentUuid":parent,"sessionId":id,"cwd":cwd,"timestamp":if m.timestamp.is_empty(){Utc::now().to_rfc3339()}else{m.timestamp.clone()},"version":"2.1.280","isSidechain":false,"userType":"external","message":message}));
        parent = Some(uuid)
    }
    out.push(json!({"type":"custom-title","sessionId":id,"customTitle":format!("[Bridge] {}",s.summary.title)}));
    out
}
pub fn codex_records(s: &Session, id: &str, cwd: &str) -> Vec<Value> {
    let now = Utc::now().to_rfc3339();
    let mut out = vec![
        json!({"timestamp":now,"type":"session_meta","payload":{"id":id,"session_id":id,"timestamp":now,"cwd":cwd,"originator":"agent_session_bridge","cli_version":"0.160.1","source":"cli","model_provider":"openai","history_mode":"legacy"}}),
    ];
    let paired = paired_calls(s);
    let mut tools = HashMap::new();
    for b in s
        .messages
        .iter()
        .flat_map(|m| &m.blocks)
        .filter(|b| b.kind == "toolCall" && paired.contains(&b.id))
    {
        if let Some(v) = map_tool(b, Provider::Codex) {
            tools.insert(b.id.clone(), v);
        }
    }
    for m in &s.messages {
        let time = if m.timestamp.is_empty() {
            now.clone()
        } else {
            m.timestamp.clone()
        };
        let role = if m.role == "assistant" {
            "assistant"
        } else {
            "user"
        };
        for b in &m.blocks {
            let payload = match b.kind.as_str() {
                "toolCall" if tools.contains_key(&b.id) => {
                    let (name, input) = &tools[&b.id];
                    json!({"type":"function_call","call_id":b.id,"name":name,"arguments":input.to_string()})
                }
                "toolResult" if tools.contains_key(&b.id) => {
                    json!({"type":"function_call_output","call_id":b.id,"output":b.text})
                }
                _ => {
                    let text = match b.kind.as_str() {
                        "text" => b.text.clone(),
                        "toolCall" => format!("[Historical tool {}] {}", b.name, b.input),
                        _ => format!("[Historical {}] {}", b.kind, b.text),
                    };
                    json!({"type":"message","role":role,"content":[{"type":if role=="assistant"{"output_text"}else{"input_text"},"text":text}]})
                }
            };
            if payload["type"] == "message" {
                let text = payload["content"][0]["text"].as_str().unwrap_or("");
                out.push(json!({"timestamp":time,"type":"event_msg","payload":if role=="user"{json!({"type":"user_message","message":text,"images":[],"local_images":[]})}else{json!({"type":"agent_message","message":text})}}));
            }
            out.push(json!({"timestamp":time,"type":"response_item","payload":payload}));
        }
    }
    out
}
pub fn serialize_records(records: &[Value]) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    for r in records {
        serde_json::to_writer(&mut out, r)?;
        out.push(b'\n');
    }
    Ok(out)
}
fn project_slug(cwd: &str) -> String {
    cwd.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}
pub fn execute(
    store: &Store,
    request: &MigrationRequest,
    expected: &str,
) -> Result<MigrationResult> {
    let source = store.session(&request.key)?;
    if source.summary.fingerprint != expected {
        return Err(error(
            "Source changed since preview. Refresh the migration plan.",
        ));
    }
    if !matches!(request.mode.as_str(), "native" | "handoff") {
        return Err(error("Unknown migration mode"));
    }
    if !Path::new(&request.cwd).is_absolute() || !Path::new(&request.cwd).is_dir() {
        return Err(error("Choose an existing absolute project directory"));
    }
    let mut normalized = request.clone();
    normalized.cwd = fs::canonicalize(&request.cwd)?
        .to_string_lossy()
        .trim_start_matches("\\\\?\\")
        .to_owned();
    let request = &normalized;
    let settings = store.settings()?;
    let identity = format!(
        "{}:{}:{}:{}",
        source.summary.fingerprint,
        request.target.label(),
        request.mode,
        request.cwd
    );
    let c = store.connect()?;
    if !request.duplicate {
        let pending: bool = c.query_row("SELECT EXISTS(SELECT 1 FROM operations WHERE identity=?1 AND status IN ('running','interrupted'))", [&identity], |r| r.get(0))?;
        if pending {
            return Err(error("An interrupted operation needs recovery. Restart Bridge to verify its target, or explicitly create a separate copy."));
        }
        let old:Option<String>=c.query_row("SELECT result FROM operations WHERE identity=?1 AND status='completed' ORDER BY created_at DESC LIMIT 1",[&identity],|r|r.get(0)).optional()?;
        if let Some(old) = old {
            let mut result: MigrationResult = serde_json::from_str(&old)?;
            if Path::new(&result.target_path).is_file() {
                result.reused = true;
                return Ok(result);
            }
        }
    }
    if request.mode == "native" && !rpc::capability(request.target, &settings).native_verified {
        return Err(error("Native conversion has not been verified for this installed CLI version. Use context handoff instead."));
    }
    let operation_id = uuid::Uuid::new_v4().to_string();
    c.execute("INSERT INTO operations(id,identity,status,request,created_at) VALUES(?1,?2,'running',?3,?4)",params![operation_id,identity,serde_json::to_string(request)?,Utc::now().to_rfc3339()])?;
    let operation_dir = store.root.join("operations").join(&operation_id);
    fs::create_dir_all(&operation_dir)?;
    let result = (|| {
        let archive_path = operation_dir.join("source.asb");
        archive::export(&source, &archive_path, "asb")?;
        if request.mode == "handoff" {
            let path = operation_dir.join("handoff.md");
            let text = request
                .handoff_text
                .clone()
                .unwrap_or_else(|| handoff(&source));
            let text = format!("{text}\n\nComplete archive: {}\n", archive_path.display());
            archive::atomic_write(&path, text.as_bytes())?;
            let prompt=format!("Read {} as historical context and continue the task. Ask me what to do next if the next step is unclear.",path.display());
            let command = rpc::session_command(request.target, &settings, &[&prompt]);
            let result = MigrationResult {
                operation_id: operation_id.clone(),
                target_id: String::new(),
                target_path: path.to_string_lossy().into(),
                resume_command: command,
                cwd: request.cwd.clone(),
                reused: false,
                warnings: source.warnings.clone(),
                mode: request.mode.clone(),
                provider: request.target,
            };
            archive::atomic_write(
                &operation_dir.join("result.json"),
                &serde_json::to_vec(&result)?,
            )?;
            return Ok(result);
        }
        let id = match request.target {
            Provider::Codex => uuid::Uuid::now_v7().to_string(),
            Provider::Claude => uuid::Uuid::new_v4().to_string(),
        };
        let path = match request.target {
            Provider::Claude => PathBuf::from(&settings.claude_home)
                .join("projects")
                .join(project_slug(&request.cwd))
                .join(format!("{id}.jsonl")),
            Provider::Codex => PathBuf::from(&settings.codex_home)
                .join("sessions")
                .join(Utc::now().format("%Y/%m/%d").to_string())
                .join(format!(
                    "rollout-{}-{id}.jsonl",
                    Utc::now().format("%Y-%m-%dT%H-%M-%S")
                )),
        };
        let records = match request.target {
            Provider::Claude => claude_records(&source, &id, &request.cwd),
            Provider::Codex => codex_records(&source, &id, &request.cwd),
        };
        let command = rpc::session_command(
            request.target,
            &settings,
            &[
                if request.target == Provider::Codex {
                    "resume"
                } else {
                    "--resume"
                },
                &id,
            ],
        );
        let report = plan(&source, request.target, &settings);
        let mut warnings = report.degraded;
        warnings.extend(report.unavailable);
        let result = MigrationResult {
            operation_id: operation_id.clone(),
            target_id: id.clone(),
            target_path: path.to_string_lossy().into(),
            resume_command: command,
            cwd: request.cwd.clone(),
            reused: false,
            warnings,
            mode: request.mode.clone(),
            provider: request.target,
        };
        // Persist the intended result before publishing any target file.
        archive::atomic_write(
            &operation_dir.join("intent.json"),
            &serde_json::to_vec(&json!({"result":result,"settings":settings}))?,
        )?;
        archive::atomic_write(&path, &serialize_records(&records)?)?;
        if let Err(e) = verify_native(&result, &settings) {
            // These IDs and files were created by this operation only.
            if request.target == Provider::Codex {
                if let Ok(mut server) = AppServer::start(&settings.codex_bin, &settings.codex_home)
                {
                    let _ = server.call("thread/delete", json!({"threadId":id}));
                }
            }
            let _ = fs::remove_file(&path);
            return Err(e);
        }
        archive::atomic_write(
            &operation_dir.join("result.json"),
            &serde_json::to_vec(&result)?,
        )?;
        Ok(result)
    })();
    match &result {
        Ok(r) => {
            c.execute(
                "UPDATE operations SET status='completed',result=?1 WHERE id=?2",
                params![serde_json::to_string(r)?, operation_id],
            )?;
            c.execute("INSERT INTO project_mappings(source,target) VALUES(?1,?2) ON CONFLICT(source) DO UPDATE SET target=excluded.target",params![source.summary.cwd,request.cwd])?;
        }
        Err(e) => {
            c.execute(
                "UPDATE operations SET status='failed',result=?1 WHERE id=?2",
                params![e.to_string(), operation_id],
            )?;
        }
    }
    result
}
fn verify_native(result: &MigrationResult, settings: &Settings) -> Result<()> {
    let parsed = parser::parse(Path::new(&result.target_path), Some(result.provider))?;
    if parsed.messages.is_empty() || parsed.summary.id != result.target_id {
        return Err(error(
            "Target identity or resumable history does not match migration",
        ));
    }
    if result.provider == Provider::Codex {
        let mut server = AppServer::start(&settings.codex_bin, &settings.codex_home)?;
        let resumed = server.call("thread/resume", json!({"threadId":result.target_id,"path":result.target_path,"cwd":result.cwd,"excludeTurns":true}))?;
        if resumed["thread"]["id"].as_str() != Some(&result.target_id) {
            return Err(error("Codex resumed a different thread"));
        }
        let read = server.call(
            "thread/read",
            json!({"threadId":result.target_id,"includeTurns":false}),
        );
        let _ = server.call("thread/unsubscribe", json!({"threadId":result.target_id}));
        read?;
    }
    Ok(())
}
pub fn recover(store: &Store) -> Result<Vec<String>> {
    let c = store.connect()?;
    let mut q = c.prepare("SELECT id FROM operations WHERE status IN ('running','interrupted')")?;
    let ids = q
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let mut unresolved = Vec::new();
    for id in ids {
        let dir = store.root.join("operations").join(&id);
        let restored = (|| -> Result<MigrationResult> {
            if dir.join("result.json").is_file() {
                let result: MigrationResult =
                    serde_json::from_slice(&fs::read(dir.join("result.json"))?)?;
                if !Path::new(&result.target_path).is_file() {
                    return Err(error("Target is missing"));
                }
                return Ok(result);
            }
            let intent: Value = serde_json::from_slice(&fs::read(dir.join("intent.json"))?)?;
            let result: MigrationResult = serde_json::from_value(intent["result"].clone())?;
            let settings: Settings = serde_json::from_value(intent["settings"].clone())?;
            // Re-register and inspect our new ID without deleting or rewriting it.
            verify_native(&result, &settings)?;
            archive::atomic_write(&dir.join("result.json"), &serde_json::to_vec(&result)?)?;
            Ok(result)
        })();
        match restored {
            Ok(result) => {
                c.execute(
                    "UPDATE operations SET status='completed',result=?1 WHERE id=?2",
                    params![serde_json::to_string(&result)?, id],
                )?;
            }
            Err(e) => {
                let published = dir.join("intent.json").is_file()
                    || dir.join("target.json").is_file()
                    || dir.join("handoff.md").is_file();
                let status = if published { "interrupted" } else { "failed" };
                c.execute(
                    "UPDATE operations SET status=?1,result=?2 WHERE id=?3",
                    params![
                        status,
                        format!("Recovery retained files at {}: {e}", dir.display()),
                        id
                    ],
                )?;
                unresolved.push(id);
            }
        }
    }
    Ok(unresolved)
}
