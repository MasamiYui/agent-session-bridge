use crate::model::*;
use chrono::{DateTime, Utc};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    fs::File,
    io::{BufRead, BufReader, Read},
    path::Path,
};

pub fn fingerprint(path: &Path) -> Result<String> {
    let mut f = File::open(path)?;
    let mut h = Sha256::new();
    let mut buf = [0; 65536];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(format!("{:x}", h.finalize()))
}
fn str_at<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k).and_then(Value::as_str).unwrap_or("")
}
fn content_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Array(a) => a
            .iter()
            .filter_map(|v| {
                v.get("text")
                    .or_else(|| v.get("thinking"))
                    .and_then(Value::as_str)
            })
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}
fn block(kind: &str, id: &str, name: &str, input: Value, text: String) -> Block {
    Block {
        kind: kind.into(),
        id: id.into(),
        name: name.into(),
        input,
        text,
        is_error: false,
    }
}
pub fn parse(path: &Path, provider_hint: Option<Provider>) -> Result<Session> {
    let metadata = path.metadata()?;
    let mut records = Vec::new();
    let mut hash = Sha256::new();
    let mut malformed = 0;
    let mut warnings = Vec::new();
    let mut reader = BufReader::new(File::open(path)?);
    let mut line = Vec::new();
    loop {
        line.clear();
        if reader.read_until(b'\n', &mut line)? == 0 {
            break;
        }
        hash.update(&line);
        if line.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        match serde_json::from_slice::<Value>(&line) {
            Ok(v) => records.push(v),
            Err(_) => malformed += 1,
        }
    }
    let after = path.metadata()?;
    if after.len() != metadata.len() || after.modified()? != metadata.modified()? {
        return Err(error("Source changed while reading; refresh and retry"));
    }
    if records.is_empty() {
        return Err(error("No readable session records"));
    }
    let provider = provider_hint.unwrap_or_else(|| {
        if records.iter().any(|v| str_at(v, "type") == "session_meta") {
            Provider::Codex
        } else {
            Provider::Claude
        }
    });
    let modified: DateTime<Utc> = metadata.modified()?.into();
    let mut session = Session {
        summary: SessionSummary {
            key: String::new(),
            id: String::new(),
            provider,
            title: String::new(),
            cwd: String::new(),
            updated_at: modified.to_rfc3339(),
            message_count: 0,
            path: path.to_string_lossy().into(),
            fingerprint: format!("{:x}", hash.finalize()),
            size: metadata.len(),
            source_archived: path
                .components()
                .any(|c| c.as_os_str() == "archived_sessions"),
            organization: Organization::default(),
        },
        messages: Vec::new(),
        warnings: Vec::new(),
        attachments: Vec::new(),
        unknown_records: malformed,
        encrypted_reasoning: 0,
        compacted: false,
    };
    if malformed > 0 {
        warnings.push(format!(
            "{malformed} incomplete or malformed records were skipped; raw bytes remain exportable"
        ));
    }
    match provider {
        Provider::Codex => parse_codex(&records, &mut session),
        Provider::Claude => parse_claude(&records, &mut session),
    };
    if session.summary.id.is_empty() {
        session.summary.id = path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .into()
    }
    session.summary.key = format!(
        "{}:{}:{}",
        provider.label(),
        session.summary.id,
        &session.summary.fingerprint[..12]
    );
    // Identity must survive appended messages; path distinguishes duplicate IDs in different stores.
    let location = format!("{:x}", Sha256::digest(path.to_string_lossy().as_bytes()));
    session.summary.key = format!(
        "{}:{}:{}",
        provider.label(),
        session.summary.id,
        &location[..12]
    );
    if session.summary.title.is_empty() {
        session.summary.title = session
            .messages
            .iter()
            .find(|m| m.role == "user")
            .and_then(|m| m.blocks.iter().find(|b| b.kind == "text"))
            .map(|b| b.text.chars().take(100).collect())
            .unwrap_or_else(|| session.summary.id.clone())
    }
    session.summary.message_count = session.messages.len();
    session.warnings.extend(warnings);
    if session.unknown_records > malformed {
        session.warnings.push(format!(
            "{} unrecognized records retained in the original transcript",
            session.unknown_records - malformed
        ));
    }
    if session.compacted {
        session.warnings.push("Compaction boundaries are preserved as historical metadata; encrypted context cannot be reconstructed".into())
    }
    Ok(session)
}
fn push_message(s: &mut Session, role: &str, time: &str, id: &str, blocks: Vec<Block>) {
    if !blocks.is_empty() {
        s.messages.push(Message {
            role: role.into(),
            timestamp: time.into(),
            id: id.into(),
            blocks,
        })
    }
}
fn parse_codex(records: &[Value], s: &mut Session) {
    let message_key = |turn: &str, role: &str, text: &str| format!("{turn}:{role}:{text}");
    let mut response_messages = HashSet::new();
    let mut response_tools = HashSet::new();
    let mut turn = String::new();
    for r in records {
        let p = &r["payload"];
        if str_at(r, "type") == "event_msg" && str_at(p, "type") == "task_started" {
            turn = str_at(p, "turn_id").into();
        }
        if str_at(r, "type") == "response_item" {
            if str_at(p, "type") == "message" {
                response_messages.insert(message_key(
                    &turn,
                    str_at(p, "role"),
                    &content_text(&p["content"]),
                ));
            }
            if matches!(str_at(p, "type"), "function_call" | "custom_tool_call") {
                response_tools.insert(str_at(p, "call_id").to_owned());
            }
        }
    }
    let mut seen = HashSet::new();
    let mut seen_events = HashSet::new();
    turn.clear();
    for r in records {
        let p = &r["payload"];
        let time = str_at(r, "timestamp");
        match str_at(r, "type") {
            "session_meta" => {
                s.summary.id = str_at(p, "id").into();
                s.summary.cwd = str_at(p, "cwd").into();
            }
            "response_item" => {
                let sig = format!("{}:{}", turn, p);
                if !str_at(p, "id").is_empty() && !seen.insert(sig) {
                    continue;
                }
                match str_at(p, "type") {
                    "message" => {
                        let mut blocks = Vec::new();
                        if let Some(a) = p["content"].as_array() {
                            for c in a {
                                match str_at(c, "type") {
                                    "input_text" | "output_text" | "text" => {
                                        blocks.push(Block::text("text", str_at(c, "text")))
                                    }
                                    "input_image" => {
                                        let image = str_at(c, "image_url");
                                        blocks.push(Block::text("image", image));
                                    }
                                    _ => {
                                        s.unknown_records += 1;
                                        blocks.push(Block::text(
                                            "metadata",
                                            format!("Historical content: {c}"),
                                        ))
                                    }
                                }
                            }
                        }
                        push_message(s, str_at(p, "role"), time, str_at(p, "id"), blocks)
                    }
                    "function_call" | "custom_tool_call" => {
                        let input = p
                            .get("arguments")
                            .and_then(Value::as_str)
                            .and_then(|x| serde_json::from_str(x).ok())
                            .or_else(|| p.get("input").cloned())
                            .unwrap_or(Value::Null);
                        push_message(
                            s,
                            "assistant",
                            time,
                            str_at(p, "id"),
                            vec![block(
                                "toolCall",
                                str_at(p, "call_id"),
                                str_at(p, "name"),
                                input,
                                String::new(),
                            )],
                        )
                    }
                    "function_call_output" | "custom_tool_call_output" => push_message(
                        s,
                        "tool",
                        time,
                        str_at(p, "id"),
                        vec![block(
                            "toolResult",
                            str_at(p, "call_id"),
                            "",
                            Value::Null,
                            p.get("output")
                                .map(|v| {
                                    if let Some(s) = v.as_str() {
                                        s.into()
                                    } else {
                                        v.to_string()
                                    }
                                })
                                .unwrap_or_default(),
                        )],
                    ),
                    "reasoning" => {
                        let text = content_text(&p["summary"]);
                        if !text.is_empty() {
                            push_message(
                                s,
                                "assistant",
                                time,
                                str_at(p, "id"),
                                vec![Block::text("reasoning", text)],
                            )
                        }
                        if p.get("encrypted_content")
                            .and_then(Value::as_str)
                            .is_some_and(|x| !x.is_empty())
                        {
                            s.encrypted_reasoning += 1
                        }
                    }
                    _ => {
                        s.unknown_records += 1;
                        push_message(
                            s,
                            "system",
                            time,
                            "",
                            vec![Block::text("metadata", format!("Historical record: {p}"))],
                        )
                    }
                }
            }
            "event_msg" => match str_at(p, "type") {
                "task_started" => turn = str_at(p, "turn_id").into(),
                "user_message"
                    if !response_messages.contains(&message_key(
                        &turn,
                        "user",
                        str_at(p, "message"),
                    )) && seen_events.insert(message_key(
                        &turn,
                        "user",
                        str_at(p, "message"),
                    )) =>
                {
                    push_message(
                        s,
                        "user",
                        time,
                        "",
                        vec![Block::text("text", str_at(p, "message"))],
                    )
                }
                "agent_message"
                    if !response_messages.contains(&message_key(
                        &turn,
                        "assistant",
                        str_at(p, "message"),
                    )) && seen_events.insert(message_key(
                        &turn,
                        "assistant",
                        str_at(p, "message"),
                    )) =>
                {
                    push_message(
                        s,
                        "assistant",
                        time,
                        "",
                        vec![Block::text("text", str_at(p, "message"))],
                    )
                }
                "item_completed" => {
                    let it = &p["item"];
                    match str_at(it, "type") {
                        "UserMessage" | "AgentMessage"
                            if {
                                let role = if str_at(it, "type") == "UserMessage" {
                                    "user"
                                } else {
                                    "assistant"
                                };
                                let text = content_text(&it["content"]);
                                let key = message_key(&turn, role, &text);
                                !response_messages.contains(&key) && seen_events.insert(key)
                            } =>
                        {
                            push_message(
                                s,
                                if str_at(it, "type") == "UserMessage" {
                                    "user"
                                } else {
                                    "assistant"
                                },
                                time,
                                str_at(it, "id"),
                                vec![Block::text("text", content_text(&it["content"]))],
                            )
                        }
                        "CommandExecution" if !response_tools.contains(str_at(it, "id")) => {
                            push_message(
                                s,
                                "assistant",
                                time,
                                "",
                                vec![block(
                                    "toolCall",
                                    str_at(it, "id"),
                                    "exec_command",
                                    json!({"cmd":it["command"]}),
                                    String::new(),
                                )],
                            );
                            push_message(
                                s,
                                "tool",
                                time,
                                "",
                                vec![block(
                                    "toolResult",
                                    str_at(it, "id"),
                                    "",
                                    Value::Null,
                                    str_at(it, "aggregated_output").into(),
                                )],
                            );
                        }
                        "UserMessage" | "AgentMessage" | "CommandExecution" => {}
                        _ => {
                            s.unknown_records += 1;
                            push_message(
                                s,
                                "system",
                                time,
                                "",
                                vec![Block::text("metadata", format!("Historical item: {it}"))],
                            )
                        }
                    }
                }
                "context_compacted" => s.compacted = true,
                _ => {}
            },
            "compacted" => {
                s.compacted = true;
                let msg = str_at(p, "message");
                if !msg.is_empty() {
                    push_message(s, "system", time, "", vec![Block::text("metadata", msg)])
                }
            }
            "world_state" | "turn_context" | "token_usage_record" | "retained_context" => {}
            _ => s.unknown_records += 1,
        }
    }
}
fn parse_claude(records: &[Value], s: &mut Session) {
    let mut by_id: HashMap<String, usize> = HashMap::new();
    let mut leaf = String::new();
    for (i, r) in records.iter().enumerate() {
        let t = str_at(r, "type");
        if matches!(t, "user" | "assistant" | "system") && !str_at(r, "uuid").is_empty() {
            by_id.insert(str_at(r, "uuid").into(), i);
            leaf = str_at(r, "uuid").into()
        }
        if t == "last-prompt" && !str_at(r, "leafUuid").is_empty() {
            leaf = str_at(r, "leafUuid").into()
        }
    }
    let mut chain = HashSet::new();
    let mut cursor = leaf;
    while let Some(&i) = by_id.get(&cursor) {
        if !chain.insert(i) {
            break;
        }
        cursor = str_at(&records[i], "parentUuid").into()
    }
    let mut merged: HashMap<String, usize> = HashMap::new();
    let mut seen_blocks = HashSet::new();
    for (i, r) in records.iter().enumerate() {
        if s.summary.id.is_empty() {
            s.summary.id = str_at(r, "sessionId").into()
        }
        if !str_at(r, "cwd").is_empty() {
            s.summary.cwd = str_at(r, "cwd").into()
        }
        match str_at(r, "type") {
            "custom-title" => s.summary.title = str_at(r, "customTitle").into(),
            "summary" => {
                s.compacted = true;
            }
            "system" if str_at(r, "subtype") == "compact_boundary" => s.compacted = true,
            "user" | "assistant" => {
                if !chain.is_empty() && !chain.contains(&i) {
                    continue;
                }
                let msg = &r["message"];
                let role = str_at(msg, "role");
                let id = str_at(msg, "id");
                let mut blocks = Vec::new();
                if let Some(text) = msg["content"].as_str() {
                    blocks.push(Block::text("text", text))
                }
                if let Some(content) = msg["content"].as_array() {
                    for c in content {
                        match str_at(c, "type") {
                            "text" => blocks.push(Block::text("text", str_at(c, "text"))),
                            "thinking" => {
                                blocks.push(Block::text("reasoning", str_at(c, "thinking")))
                            }
                            "tool_use" => blocks.push(block(
                                "toolCall",
                                str_at(c, "id"),
                                str_at(c, "name"),
                                c["input"].clone(),
                                String::new(),
                            )),
                            "tool_result" => {
                                let mut b = block(
                                    "toolResult",
                                    str_at(c, "tool_use_id"),
                                    "",
                                    Value::Null,
                                    content_text(&c["content"]),
                                );
                                b.is_error = c["is_error"].as_bool().unwrap_or(false);
                                blocks.push(b)
                            }
                            "image" => blocks.push(Block::text(
                                "image",
                                format!("Image ({})", str_at(&c["source"], "type")),
                            )),
                            "redacted_thinking" => s.encrypted_reasoning += 1,
                            _ => {
                                s.unknown_records += 1;
                                blocks.push(Block::text(
                                    "metadata",
                                    format!("Historical content: {c}"),
                                ))
                            }
                        }
                    }
                }
                if !id.is_empty() {
                    blocks.retain(|b| {
                        seen_blocks.insert(format!(
                            "{id}:{}",
                            serde_json::to_string(b).unwrap_or_default()
                        ))
                    });
                    if let Some(&mi) = merged.get(id) {
                        s.messages[mi].blocks.extend(blocks);
                        continue;
                    }
                    if !blocks.is_empty() {
                        merged.insert(id.into(), s.messages.len());
                    }
                }
                push_message(
                    s,
                    if role.is_empty() {
                        str_at(r, "type")
                    } else {
                        role
                    },
                    str_at(r, "timestamp"),
                    str_at(r, "uuid"),
                    blocks,
                )
            }
            "attachment" => {
                if let Some(path) = r.pointer("/attachment/path").and_then(Value::as_str) {
                    let p = Path::new(path);
                    s.attachments.push(Attachment {
                        source: path.into(),
                        archive_path: format!("attachments/{}", s.attachments.len()),
                        available: p
                            .canonicalize()
                            .ok()
                            .zip(
                                Path::new(&s.summary.path)
                                    .parent()
                                    .and_then(|p| p.canonicalize().ok()),
                            )
                            .is_some_and(|(file, root)| file.starts_with(root) && file.is_file()),
                    });
                }
            }
            "system"
            | "queue-operation"
            | "last-prompt"
            | "file-history-snapshot"
            | "file-history-delta"
            | "atis-latch"
            | "agent-name"
            | "cost-state"
            | "progress" => {}
            _ => s.unknown_records += 1,
        }
    }
}
pub fn markdown(session: &Session) -> String {
    let mut out = format!(
        "# {}\n\nSource: {} · {}\nProject: {}\n\n",
        session.summary.title,
        session.summary.provider.label(),
        session.summary.id,
        session.summary.cwd
    );
    for msg in &session.messages {
        out.push_str(&format!("## {} · {}\n\n", msg.role, msg.timestamp));
        for b in &msg.blocks {
            match b.kind.as_str() {
                "toolCall" => out.push_str(&format!(
                    "**Tool: {}** (`{}`)\n\n```json\n{}\n```\n\n",
                    b.name,
                    b.id,
                    serde_json::to_string_pretty(&b.input).unwrap_or_default()
                )),
                "toolResult" => {
                    out.push_str(&format!("**Tool result** (`{}`)\n\n{}\n\n", b.id, b.text))
                }
                _ => out.push_str(&format!("{}\n\n", b.text)),
            }
        }
    }
    out
}
