use bridge_core::{archive, migration, model::*, parser, store::Store};
use serde_json::{json, Value};
use std::{fs, path::Path};
fn write(path: &Path, records: &[Value]) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, migration::serialize_records(records).unwrap()).unwrap();
}
fn claude_fixture() -> Vec<Value> {
    vec![
        json!({"type":"user","uuid":"u","parentUuid":null,"sessionId":"original","cwd":"/tmp","timestamp":"2026-10-10T00:00:00Z","message":{"role":"user","content":"Investigate needle-marker cache bug"}}),
        json!({"type":"assistant","uuid":"a","parentUuid":"u","sessionId":"original","cwd":"/tmp","timestamp":"2026-10-10T00:01:00Z","message":{"id":"msg-a","role":"assistant","content":[{"type":"text","text":"Working on it"},{"type":"tool_use","id":"call-1","name":"Bash","input":{"command":"printf hello"}}]}}),
        json!({"type":"user","uuid":"r","parentUuid":"a","sessionId":"original","cwd":"/tmp","timestamp":"2026-10-10T00:02:00Z","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"call-1","content":"hello"}]}}),
    ]
}
#[test]
fn claude_chain_discards_abandoned_branch_and_merges_streamed_blocks() {
    let t = tempfile::tempdir().unwrap();
    let p = t.path().join("c.jsonl");
    let mut records = claude_fixture();
    records.insert(1,json!({"type":"assistant","uuid":"abandoned","parentUuid":"u","message":{"role":"assistant","content":"Should not appear"}}));
    records.push(json!({"type":"assistant","uuid":"a2","parentUuid":"r","sessionId":"original","message":{"id":"msg-shared","role":"assistant","content":[{"type":"thinking","thinking":"Reason"}]}}));
    records.push(json!({"type":"assistant","uuid":"a3","parentUuid":"a2","sessionId":"original","message":{"id":"msg-shared","role":"assistant","content":[{"type":"text","text":"Answer"}]}}));
    write(&p, &records);
    let s = parser::parse(&p, None).unwrap();
    assert_eq!(s.messages.len(), 4);
    assert!(!parser::markdown(&s).contains("Should not appear"));
    assert_eq!(s.messages[3].blocks.len(), 2);
}
#[test]
fn codex_legacy_and_paginated_dual_events_do_not_duplicate_messages() {
    let t = tempfile::tempdir().unwrap();
    let p = t.path().join("x.jsonl");
    write(
        &p,
        &[
            json!({"type":"session_meta","payload":{"id":"id","cwd":"/tmp","history_mode":"paginated"}}),
            json!({"type":"event_msg","payload":{"type":"task_started","turn_id":"t"}}),
            json!({"type":"event_msg","payload":{"type":"item_completed","item":{"type":"UserMessage","content":[{"type":"text","text":"Hello"}]}}}),
            json!({"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"Hello"}]}}),
            json!({"type":"compacted","payload":{"message":"Compact boundary"}}),
            json!({"type":"response_item","payload":{"type":"reasoning","encrypted_content":"opaque","summary":[]}}),
            json!({"type":"future_record","payload":{"secret":"retained"}}),
        ],
    );
    let s = parser::parse(&p, None).unwrap();
    assert_eq!(s.messages.iter().filter(|m| m.role == "user").count(), 1);
    assert!(s.compacted);
    assert_eq!(s.encrypted_reasoning, 1);
    assert_eq!(s.unknown_records, 1);
}
#[test]
fn malformed_trailing_line_is_reported_and_raw_export_is_byte_exact() {
    let t = tempfile::tempdir().unwrap();
    let p = t.path().join("c.jsonl");
    write(&p, &claude_fixture());
    let mut bytes = fs::read(&p).unwrap();
    bytes.extend(b"{\"type\":");
    fs::write(&p, &bytes).unwrap();
    let s = parser::parse(&p, None).unwrap();
    assert_eq!(s.unknown_records, 1);
    let exported = t.path().join("out.jsonl");
    archive::export(&s, &exported, "jsonl").unwrap();
    assert_eq!(bytes, fs::read(exported).unwrap());
}
#[test]
fn bidirectional_conversion_keeps_text_order_and_tool_pairs() {
    let t = tempfile::tempdir().unwrap();
    let source = t.path().join("source.jsonl");
    write(&source, &claude_fixture());
    let s = parser::parse(&source, None).unwrap();
    let p = t.path().join("codex.jsonl");
    write(&p, &migration::codex_records(&s, "new-id", "/tmp"));
    let c = parser::parse(&p, None).unwrap();
    assert_eq!(c.summary.id, "new-id");
    let tool = c
        .messages
        .iter()
        .flat_map(|m| &m.blocks)
        .find(|b| b.kind == "toolCall")
        .unwrap();
    assert_eq!(tool.name, "exec_command");
    assert_eq!(tool.input["cmd"], "printf hello");
    let p2 = t.path().join("claude.jsonl");
    write(&p2, &migration::claude_records(&c, "another-id", "/tmp"));
    let r = parser::parse(&p2, None).unwrap();
    let blocks = r
        .messages
        .iter()
        .flat_map(|m| &m.blocks)
        .collect::<Vec<_>>();
    assert!(blocks
        .iter()
        .any(|b| b.kind == "toolCall" && b.id == "call-1" && b.name == "Bash"));
    assert!(blocks
        .iter()
        .any(|b| b.kind == "toolResult" && b.id == "call-1" && b.text == "hello"));
    assert!(blocks[0].text.contains("needle-marker"));
}
#[test]
fn archive_roundtrip_preserves_bytes_metadata_and_subagents() {
    let t = tempfile::tempdir().unwrap();
    let path = t.path().join("source.jsonl");
    write(&path, &claude_fixture());
    let mut s = parser::parse(&path, None).unwrap();
    s.summary.organization.favorite = true;
    s.summary.organization.tags = vec!["review".into()];
    let sub = path
        .with_file_name("source")
        .join("subagents/agent-test.jsonl");
    write(&sub, &claude_fixture());
    let zip = t.path().join("out.asb");
    archive::export(&s, &zip, "asb").unwrap();
    let preview = archive::preview_import(&zip).unwrap();
    assert!(preview.summary.organization.favorite);
    let store = Store::new(t.path().join("bridge")).unwrap();
    let imported = archive::import(&store, &zip).unwrap();
    assert_eq!(
        fs::read(path).unwrap(),
        fs::read(&imported.summary.path).unwrap()
    );
    assert!(imported.summary.organization.favorite);
    assert_eq!(store.list("").unwrap().len(), 1);
    archive::import(&store, &zip).unwrap();
    assert_eq!(store.list("").unwrap().len(), 1);
}
#[test]
fn malicious_archives_and_overwrites_are_rejected() {
    use std::io::Write;
    let t = tempfile::tempdir().unwrap();
    let p = t.path().join("evil.asb");
    let mut z = zip::ZipWriter::new(fs::File::create(&p).unwrap());
    z.start_file("../escape.jsonl", zip::write::SimpleFileOptions::default())
        .unwrap();
    z.write_all(b"evil").unwrap();
    z.finish().unwrap();
    assert!(archive::preview_import(&p).is_err());
    assert!(!t.path().parent().unwrap().join("escape.jsonl").exists());
    let dest = t.path().join("out");
    archive::atomic_write(&dest, b"first").unwrap();
    assert!(archive::atomic_write(&dest, b"second").is_err());
    assert_eq!(fs::read(dest).unwrap(), b"first");
}
#[test]
fn organization_and_full_text_index_survive_refresh_without_source_changes() {
    let t = tempfile::tempdir().unwrap();
    let source = t.path().join("claude/projects/project/source.jsonl");
    write(&source, &claude_fixture());
    let before = fs::read(&source).unwrap();
    let store = Store::new(t.path().join("bridge")).unwrap();
    let settings = Settings {
        claude_home: t.path().join("claude").to_string_lossy().into(),
        codex_home: t.path().join("codex").to_string_lossy().into(),
        ..Default::default()
    };
    store.save_settings(&settings).unwrap();
    store.scan().unwrap();
    let summary = store.list("needle-marker").unwrap().pop().unwrap();
    let org = Organization {
        alias: "Investigation".into(),
        favorite: true,
        tags: vec!["bug".into()],
        ..Default::default()
    };
    store.update(&summary.key, &org).unwrap();
    store.scan().unwrap();
    assert_eq!(
        store.list("Investigation").unwrap()[0].organization.alias,
        "Investigation"
    );
    assert_eq!(before, fs::read(source).unwrap());
}
#[test]
fn changed_source_is_rejected_before_migration() {
    let t = tempfile::tempdir().unwrap();
    let p = t.path().join("source.jsonl");
    write(&p, &claude_fixture());
    let s = parser::parse(&p, None).unwrap();
    let store = Store::new(t.path().join("store")).unwrap();
    store.index_session(&s, 0).unwrap();
    let req = MigrationRequest {
        key: s.summary.key.clone(),
        target: Provider::Codex,
        mode: "handoff".into(),
        cwd: t.path().to_string_lossy().into(),
        duplicate: false,
        handoff_text: None,
    };
    let mut raw = fs::read(&p).unwrap();
    raw.extend(b"\n");
    fs::write(p, raw).unwrap();
    let err = migration::execute(&store, &req, &s.summary.fingerprint).unwrap_err();
    assert!(err.to_string().contains("Source changed"));
}
#[test]
fn handoff_is_editable_and_duplicate_import_reuses_result() {
    let t = tempfile::tempdir().unwrap();
    let p = t.path().join("source.jsonl");
    write(&p, &claude_fixture());
    let s = parser::parse(&p, None).unwrap();
    let store = Store::new(t.path().join("store")).unwrap();
    store.index_session(&s, 0).unwrap();
    let req = MigrationRequest {
        key: s.summary.key.clone(),
        target: Provider::Codex,
        mode: "handoff".into(),
        cwd: t.path().to_string_lossy().into(),
        duplicate: false,
        handoff_text: Some("Edited context".into()),
    };
    let r = migration::execute(&store, &req, &s.summary.fingerprint).unwrap();
    assert!(fs::read_to_string(&r.target_path)
        .unwrap()
        .contains("Edited context"));
    let r2 = migration::execute(&store, &req, &s.summary.fingerprint).unwrap();
    assert!(r2.reused);
    assert_eq!(r.operation_id, r2.operation_id);
}
#[test]
fn project_sync_is_bidirectional_incremental_and_does_not_copy_generated_targets() {
    let t = tempfile::tempdir().unwrap();
    let project = t.path().join("project");
    fs::create_dir(&project).unwrap();
    let claude = t
        .path()
        .join("claude/projects")
        .join(project.display().to_string().replace('/', "-"))
        .join("source.jsonl");
    let codex = t.path().join("codex/sessions/2026/10/10/rollout.jsonl");
    let mut claude_records = claude_fixture();
    for record in &mut claude_records {
        if let Some(object) = record.as_object_mut() {
            object.insert("cwd".into(), json!(project));
        }
    }
    write(&claude, &claude_records);
    write(
        &codex,
        &[
            json!({"type":"session_meta","payload":{"id":"codex-source","cwd":project,"originator":"codex"}}),
            json!({"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"Codex request"}]}}),
        ],
    );
    let store = Store::new(t.path().join("bridge")).unwrap();
    store
        .save_settings(&Settings {
            claude_home: t.path().join("claude").to_string_lossy().into(),
            codex_home: t.path().join("codex").to_string_lossy().into(),
            codex_bin: "missing-codex".into(),
            claude_bin: "missing-claude".into(),
            ..Default::default()
        })
        .unwrap();
    let cwd = project.to_string_lossy().into_owned();
    let first = bridge_core::project_sync::plan(&store, &cwd).unwrap();
    assert_eq!(first.items.len(), 4);
    assert!(first.items.iter().all(|item| item.action == "create"
        && item.mode
            == if item.target == Provider::Antigravity {
                "native"
            } else {
                "handoff"
            }));
    for item in &first.items {
        bridge_core::project_sync::execute(
            &store,
            &cwd,
            &MigrationRequest {
                key: item.key.clone(),
                target: item.target,
                mode: item.mode.clone(),
                cwd: cwd.clone(),
                duplicate: false,
                handoff_text: None,
            },
            &item.fingerprint,
        )
        .unwrap();
    }
    let second = bridge_core::project_sync::plan(&store, &cwd).unwrap();
    assert_eq!(second.items.len(), 4);
    assert!(second
        .items
        .iter()
        .all(|item| item.action == "skip" && item.reason == "synced"));

    let mut changed_records = claude_records;
    changed_records.push(json!({"type":"user","uuid":"new","parentUuid":"r","sessionId":"original","cwd":project,"timestamp":"2026-10-10T00:03:00Z","message":{"role":"user","content":"A new user request"}}));
    write(&claude, &changed_records);
    let later = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(4_000_000_000);
    fs::File::open(&claude)
        .unwrap()
        .set_modified(later)
        .unwrap();
    let changed = bridge_core::project_sync::plan(&store, &cwd).unwrap();
    assert_eq!(
        changed
            .items
            .iter()
            .filter(|item| item.action == "create")
            .count(),
        2
    );

    let generated = t.path().join("codex/sessions/2026/10/10/generated.jsonl");
    write(
        &generated,
        &migration::codex_records(&parser::parse(&claude, None).unwrap(), "generated-id", &cwd),
    );
    let result = MigrationResult {
        operation_id: "origin".into(),
        target_id: "generated-id".into(),
        target_path: generated.to_string_lossy().into(),
        resume_command: String::new(),
        cwd: cwd.clone(),
        reused: false,
        warnings: Vec::new(),
        mode: "native".into(),
        provider: Provider::Codex,
    };
    store
        .connect()
        .unwrap()
        .execute(
            "INSERT INTO operations(id,identity,status,request,result,created_at) VALUES('origin','generated','completed',?1,?2,'2026-10-10T00:00:00Z')",
            rusqlite::params![
                serde_json::json!({"key":"generated","target":"claude","mode":"native","cwd":cwd,"duplicate":false}).to_string(),
                serde_json::to_string(&result).unwrap()
            ],
        )
        .unwrap();
    store.scan().unwrap();
    let generated_item = bridge_core::project_sync::plan(&store, &cwd)
        .unwrap()
        .items
        .into_iter()
        .find(|item| item.id == "generated-id")
        .unwrap();
    assert_eq!(generated_item.reason, "generated");
    let blocked = bridge_core::project_sync::execute(
        &store,
        &cwd,
        &MigrationRequest {
            key: generated_item.key,
            target: generated_item.target,
            mode: generated_item.mode,
            cwd: cwd.clone(),
            duplicate: false,
            handoff_text: None,
        },
        &generated_item.fingerprint,
    )
    .unwrap_err();
    assert!(blocked.to_string().contains("generated"));
}
#[test]
fn thousand_sessions_and_large_transcript_can_be_indexed() {
    let t = tempfile::tempdir().unwrap();
    let p = t.path().join("large.jsonl");
    let mut f = fs::File::create(&p).unwrap();
    use std::io::Write;
    let text = "x".repeat(512 * 1024);
    for i in 0..102 {
        writeln!(f,"{}",json!({"type":"user","uuid":format!("{i}"),"parentUuid":if i>0{Some(format!("{}",i-1))}else{None},"sessionId":"large","cwd":"/tmp","message":{"role":"user","content":text}})).unwrap();
    }
    drop(f);
    assert!(p.metadata().unwrap().len() > 50 * 1024 * 1024);
    let s = parser::parse(&p, None).unwrap();
    assert_eq!(s.messages.len(), 102);
    let store = Store::new(t.path().join("store")).unwrap();
    let mut small = s.clone();
    small.messages.clear();
    for i in 0..1000 {
        small.summary.key = format!("key-{i}");
        small.summary.title = format!("session-{i}");
        store.index_session(&small, i).unwrap();
    }
    assert_eq!(store.list("").unwrap().len(), 1000);
}

#[test]
fn recovery_finishes_published_result_and_preserves_source() {
    let t = tempfile::tempdir().unwrap();
    let path = t.path().join("source.jsonl");
    write(&path, &claude_fixture());
    let source = parser::parse(&path, None).unwrap();
    let original = fs::read(&path).unwrap();
    let store = Store::new(t.path().join("bridge")).unwrap();
    store.index_session(&source, 0).unwrap();
    let request = MigrationRequest {
        key: source.summary.key.clone(),
        target: Provider::Claude,
        mode: "handoff".into(),
        cwd: t.path().to_string_lossy().into(),
        duplicate: false,
        handoff_text: None,
    };
    let result = migration::execute(&store, &request, &source.summary.fingerprint).unwrap();
    // Simulate termination after the durable result but before the SQLite commit.
    store
        .connect()
        .unwrap()
        .execute(
            "UPDATE operations SET status='running',result=NULL WHERE id=?1",
            [&result.operation_id],
        )
        .unwrap();
    assert!(migration::recover(&store).unwrap().is_empty());
    let reused = migration::execute(&store, &request, &source.summary.fingerprint).unwrap();
    assert!(reused.reused);
    assert_eq!(reused.target_path, result.target_path);
    assert_eq!(fs::read(path).unwrap(), original);
}

#[test]
fn imported_archive_reexports_subagents_and_owned_attachments() {
    let t = tempfile::tempdir().unwrap();
    let path = t.path().join("source.jsonl");
    write(&path, &claude_fixture());
    let sub = t.path().join("source/subagents/agent.jsonl");
    write(&sub, &claude_fixture());
    let attachment = t.path().join("note.txt");
    fs::write(&attachment, "attachment content").unwrap();
    let mut source = parser::parse(&path, None).unwrap();
    source.attachments.push(Attachment {
        source: attachment.to_string_lossy().into(),
        archive_path: "attachments/note.txt".into(),
        available: true,
    });
    let zip = t.path().join("first.asb");
    archive::export(&source, &zip, "asb").unwrap();
    let store = Store::new(t.path().join("bridge")).unwrap();
    let imported = archive::import(&store, &zip).unwrap();
    fs::remove_file(attachment).unwrap();
    fs::remove_file(sub).unwrap();
    let second = t.path().join("second.asb");
    archive::export(
        &store.session(&imported.summary.key).unwrap(),
        &second,
        "asb",
    )
    .unwrap();
    let mut z = zip::ZipArchive::new(fs::File::open(second).unwrap()).unwrap();
    assert!(z.by_name("raw/subagents/agent.jsonl").is_ok());
    use std::io::Read;
    let mut body = String::new();
    z.by_name("attachments/note.txt")
        .unwrap()
        .read_to_string(&mut body)
        .unwrap();
    assert_eq!(body, "attachment content");
}

#[test]
fn mixed_paginated_items_and_empty_assistant_chunks_are_retained() {
    let t = tempfile::tempdir().unwrap();
    let path = t.path().join("mixed.jsonl");
    write(
        &path,
        &[
            json!({"type":"session_meta","payload":{"id":"mixed","cwd":"/tmp"}}),
            json!({"type":"event_msg","payload":{"type":"task_started","turn_id":"t"}}),
            json!({"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"Question"}]}}),
            json!({"type":"event_msg","payload":{"type":"item_completed","item":{"type":"AgentMessage","content":[{"type":"text","text":"Item-only answer"}]}}}),
        ],
    );
    assert!(parser::markdown(&parser::parse(&path, None).unwrap()).contains("Item-only answer"));
    write(
        &path,
        &[
            json!({"type":"assistant","uuid":"a","parentUuid":null,"message":{"id":"same","role":"assistant","content":[]}}),
            json!({"type":"assistant","uuid":"b","parentUuid":"a","message":{"id":"same","role":"assistant","content":[{"type":"text","text":"Final chunk"}]}}),
        ],
    );
    assert_eq!(
        parser::parse(&path, None).unwrap().messages[0].blocks[0].text,
        "Final chunk"
    );
}

fn proto_varint(mut value: u64) -> Vec<u8> {
    let mut out = Vec::new();
    loop {
        let mut byte = (value & 0x7f) as u8;
        value >>= 7;
        if value > 0 {
            byte |= 0x80;
        }
        out.push(byte);
        if value == 0 {
            break;
        }
    }
    out
}
fn proto_key(field: u64, wire: u8) -> Vec<u8> {
    proto_varint((field << 3) | u64::from(wire))
}
fn proto_bytes(field: u64, bytes: &[u8]) -> Vec<u8> {
    let mut out = proto_key(field, 2);
    out.extend(proto_varint(bytes.len() as u64));
    out.extend(bytes);
    out
}
fn proto_int(field: u64, value: u64) -> Vec<u8> {
    let mut out = proto_key(field, 0);
    out.extend(proto_varint(value));
    out
}
fn proto_string(field: u64, value: &str) -> Vec<u8> {
    proto_bytes(field, value.as_bytes())
}
#[test]
fn antigravity_sqlite_steps_become_readable_project_sessions() {
    let t = tempfile::tempdir().unwrap();
    let home = t.path().join("antigravity");
    let project = t.path().join("project");
    fs::create_dir_all(home.join("conversations")).unwrap();
    fs::create_dir(&project).unwrap();
    let id = "11111111-1111-4111-8111-111111111111";
    let database = home.join(format!("conversations/{id}.db"));
    let connection = rusqlite::Connection::open(&database).unwrap();
    connection
        .execute_batch(
            "CREATE TABLE steps(idx INTEGER PRIMARY KEY,step_type INTEGER,step_payload BLOB)",
        )
        .unwrap();
    let stamp = proto_bytes(
        1,
        &proto_int(1, 1_791_600_000)
            .into_iter()
            .chain(proto_int(2, 0))
            .collect::<Vec<_>>(),
    );
    let meta = proto_bytes(5, &proto_bytes(1, &stamp));
    let user_body = proto_string(2, "Investigate the Antigravity index")
        .into_iter()
        .chain(proto_string(3, "duplicate"))
        .collect::<Vec<_>>();
    let user = proto_int(1, 14)
        .into_iter()
        .chain(meta.clone())
        .chain(proto_bytes(19, &user_body))
        .collect::<Vec<_>>();
    let assistant = proto_int(1, 15)
        .into_iter()
        .chain(meta.clone())
        .chain(proto_bytes(20, &proto_string(1, "The index is readable.")))
        .collect::<Vec<_>>();
    let call = proto_string(1, "call-1")
        .into_iter()
        .chain(proto_string(2, "search_web"))
        .chain(proto_string(3, "{\"query\":\"index\"}"))
        .collect::<Vec<_>>();
    let result = proto_bytes(2, &proto_string(1, "one matching conversation"));
    let tool = proto_int(1, 132)
        .into_iter()
        .chain(proto_bytes(
            5,
            &proto_bytes(1, &stamp)
                .into_iter()
                .chain(proto_bytes(4, &call))
                .collect::<Vec<_>>(),
        ))
        .chain(proto_bytes(140, &result))
        .collect::<Vec<_>>();
    let failure = proto_int(1, 17)
        .into_iter()
        .chain(meta)
        .chain(proto_bytes(
            24,
            &proto_bytes(3, &proto_string(2, "Execution stopped")),
        ))
        .collect::<Vec<_>>();
    for (index, kind, payload) in [
        (0i64, 14i64, user),
        (1, 15, assistant),
        (2, 132, tool),
        (3, 17, failure),
    ] {
        connection
            .execute(
                "INSERT INTO steps(idx,step_type,step_payload) VALUES(?1,?2,?3)",
                rusqlite::params![index, kind, payload],
            )
            .unwrap();
    }
    let summaries = rusqlite::Connection::open(home.join("conversation_summaries.db")).unwrap();
    summaries
        .execute_batch("CREATE TABLE conversation_summaries(conversation_id TEXT PRIMARY KEY,title TEXT,preview TEXT,workspace_uris TEXT)")
        .unwrap();
    summaries
        .execute(
            "INSERT INTO conversation_summaries(conversation_id,title,preview,workspace_uris) VALUES(?1,'Readable index','preview',?2)",
            rusqlite::params![id, json!([format!("file://{}", project.display())]).to_string()],
        )
        .unwrap();
    let session = parser::parse(&database, Some(Provider::Antigravity)).unwrap();
    assert_eq!(session.summary.title, "Readable index");
    assert_eq!(session.summary.cwd, project.to_string_lossy());
    assert_eq!(
        session.messages[0].blocks[0].text,
        "Investigate the Antigravity index"
    );
    assert_eq!(session.messages[1].blocks[0].text, "The index is readable.");
    assert_eq!(session.messages[2].blocks[0].name, "search_web");
    assert_eq!(
        session.messages[3].blocks[0].text,
        "one matching conversation"
    );
    assert!(session.messages[4].blocks[0].is_error);
    let store = Store::new(t.path().join("bridge")).unwrap();
    store
        .save_settings(&Settings {
            codex_home: t.path().join("codex").to_string_lossy().into(),
            claude_home: t.path().join("claude").to_string_lossy().into(),
            antigravity_home: home.to_string_lossy().into(),
            ..Default::default()
        })
        .unwrap();
    assert!(store.scan().unwrap().is_empty());
    let listed = store.list("matching conversation").unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].provider, Provider::Antigravity);
    let request = MigrationRequest {
        key: listed[0].key.clone(),
        target: Provider::Antigravity,
        mode: "native".into(),
        cwd: project.to_string_lossy().into(),
        duplicate: false,
        handoff_text: None,
    };
    let written = migration::execute(&store, &request, &listed[0].fingerprint).unwrap();
    assert_ne!(written.target_id, id);
    let copied = parser::parse(
        std::path::Path::new(&written.target_path),
        Some(Provider::Antigravity),
    )
    .unwrap();
    assert_eq!(copied.summary.provider, Provider::Antigravity);
    assert_eq!(
        copied.summary.cwd,
        fs::canonicalize(&project).unwrap().to_string_lossy()
    );
    assert_eq!(
        copied.messages[0].blocks[0].text,
        "Investigate the Antigravity index"
    );
    assert_eq!(copied.messages[1].blocks[0].text, "The index is readable.");
    assert_eq!(copied.messages[2].blocks[0].name, "search_web");
    assert_eq!(
        copied.messages[3].blocks[0].text,
        "one matching conversation"
    );
    assert!(copied.messages[4].blocks[0].is_error);
    let plan = bridge_core::project_sync::plan(&store, &project.to_string_lossy()).unwrap();
    assert!(plan
        .items
        .iter()
        .any(|item| item.source == Provider::Antigravity
            && item.target == Provider::Codex
            && item.action == "create"));
    assert!(plan
        .items
        .iter()
        .any(|item| item.source == Provider::Antigravity
            && item.target == Provider::Claude
            && item.action == "create"));
    let handoff = MigrationRequest {
        target: Provider::Claude,
        mode: "handoff".into(),
        ..request
    };
    let result = migration::execute(&store, &handoff, &listed[0].fingerprint).unwrap();
    assert!(fs::read_to_string(result.target_path)
        .unwrap()
        .contains("Investigate the Antigravity index"));
}
