use bridge_core::{migration, model::*, parser, rpc::AppServer, store::Store};
use serde_json::json;
use std::{fs, path::PathBuf};
fn main() -> Result<()> {
    let root = PathBuf::from(std::env::args().nth(1).expect("isolated root"));
    fs::create_dir_all(root.join("project"))?;
    let store = Store::new(root.join("bridge"))?;
    let settings = Settings {
        claude_home: root.join("claude").to_string_lossy().into(),
        codex_home: root.join("codex").to_string_lossy().into(),
        ..Default::default()
    };
    store.save_settings(&settings)?;
    fs::create_dir_all(&settings.codex_home)?;
    let port = std::env::args().nth(2).unwrap_or_else(|| "18439".into());
    fs::write(PathBuf::from(&settings.codex_home).join("config.toml"),format!("model = \"mock-model\"\nmodel_provider = \"mock\"\n[model_providers.mock]\nname = \"Mock\"\nbase_url = \"http://127.0.0.1:{port}/v1\"\nwire_api = \"responses\"\nrequires_openai_auth = false\n"))?;
    let source = root.join("source.jsonl");
    fs::write(
        &source,
        migration::serialize_records(&[
            json!({"type":"user","uuid":"u1","parentUuid":null,"sessionId":"source","cwd":root.join("project"),"timestamp":"2026-10-10T00:00:00Z","message":{"role":"user","content":"Remember the codeword BRIDGE_ORCHID. The next request asks for it."}}),
            json!({"type":"assistant","uuid":"a1","parentUuid":"u1","sessionId":"source","cwd":root.join("project"),"timestamp":"2026-10-10T00:01:00Z","message":{"id":"msg-1","role":"assistant","content":[{"type":"text","text":"I will remember BRIDGE_ORCHID."}]}}),
        ])?,
    )?;
    let s = parser::parse(&source, None)?;
    store.index_session(&s, 0)?;
    let req = MigrationRequest {
        key: s.summary.key.clone(),
        target: Provider::Codex,
        mode: "native".into(),
        cwd: root.join("project").to_string_lossy().into(),
        duplicate: false,
        handoff_text: None,
    };
    let migrated = migration::execute(&store, &req, &s.summary.fingerprint)?;
    println!("{}", serde_json::to_string_pretty(&migrated)?);
    let mut server = AppServer::start(&settings.codex_bin, &settings.codex_home)?;
    let read = server.call(
        "thread/read",
        json!({"threadId":migrated.target_id,"includeTurns":true}),
    )?;
    println!("READ {}", read);
    server.call(
        "thread/resume",
        json!({"threadId":migrated.target_id,"model":"mock-model","modelProvider":"mock"}),
    )?;
    let next=server.call("turn/start",json!({"threadId":migrated.target_id,"input":[{"type":"text","text":"What codeword did we agree on?"}]}))?;
    println!("TURN {}", next);
    std::thread::sleep(std::time::Duration::from_secs(3));
    drop(server);
    let target = parser::parse(
        std::path::Path::new(&migrated.target_path),
        Some(Provider::Codex),
    )?;
    store.index_session(&target, 0)?;
    let req = MigrationRequest {
        key: target.summary.key.clone(),
        target: Provider::Claude,
        mode: "native".into(),
        cwd: root.join("project").to_string_lossy().into(),
        duplicate: false,
        handoff_text: None,
    };
    let reverse = migration::execute(&store, &req, &target.summary.fingerprint)?;
    println!("CLAUDE {}", serde_json::to_string_pretty(&reverse)?);
    fs::write(
        root.join("results.json"),
        serde_json::to_vec_pretty(&json!({"codex":migrated,"claude":reverse}))?,
    )?;
    Ok(())
}
