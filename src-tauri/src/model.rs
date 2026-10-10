use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum BridgeError {
    #[error("{0}")]
    Message(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Sql(#[from] rusqlite::Error),
    #[error(transparent)]
    Zip(#[from] zip::result::ZipError),
}
pub type Result<T> = std::result::Result<T, BridgeError>;
pub fn error(s: impl Into<String>) -> BridgeError {
    BridgeError::Message(s.into())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    Codex,
    Claude,
}
impl Provider {
    pub fn label(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::Claude => "claude",
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Organization {
    pub alias: String,
    pub tags: Vec<String>,
    pub favorite: bool,
    pub archived: bool,
    pub removed: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSummary {
    pub key: String,
    pub id: String,
    pub provider: Provider,
    pub title: String,
    pub cwd: String,
    pub updated_at: String,
    pub message_count: usize,
    pub path: String,
    pub fingerprint: String,
    pub size: u64,
    pub source_archived: bool,
    pub organization: Organization,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Block {
    pub kind: String,
    pub text: String,
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub input: Value,
    #[serde(default)]
    pub is_error: bool,
}
impl Block {
    pub fn text(kind: &str, text: impl Into<String>) -> Self {
        Self {
            kind: kind.into(),
            text: text.into(),
            id: String::new(),
            name: String::new(),
            input: Value::Null,
            is_error: false,
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Message {
    pub role: String,
    pub timestamp: String,
    pub id: String,
    pub blocks: Vec<Block>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Attachment {
    pub source: String,
    pub archive_path: String,
    pub available: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Session {
    pub summary: SessionSummary,
    pub messages: Vec<Message>,
    pub warnings: Vec<String>,
    pub attachments: Vec<Attachment>,
    pub unknown_records: usize,
    pub encrypted_reasoning: usize,
    pub compacted: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub codex_home: String,
    pub claude_home: String,
    pub codex_bin: String,
    pub claude_bin: String,
    pub export_dir: String,
    pub language: String,
    pub theme: String,
}
impl Default for Settings {
    fn default() -> Self {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        Self {
            codex_home: std::env::var("CODEX_HOME")
                .unwrap_or_else(|_| home.join(".codex").to_string_lossy().into()),
            claude_home: std::env::var("CLAUDE_CONFIG_DIR")
                .unwrap_or_else(|_| home.join(".claude").to_string_lossy().into()),
            codex_bin: "codex".into(),
            claude_bin: "claude".into(),
            export_dir: home.join("Downloads").to_string_lossy().into(),
            language: "system".into(),
            theme: "system".into(),
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Capability {
    pub provider: Provider,
    pub executable: String,
    pub version: Option<String>,
    pub native_verified: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MigrationRequest {
    pub key: String,
    pub target: Provider,
    pub mode: String,
    pub cwd: String,
    #[serde(default)]
    pub duplicate: bool,
    #[serde(default)]
    pub handoff_text: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MigrationPlan {
    pub source_fingerprint: String,
    pub preserved: Vec<String>,
    pub degraded: Vec<String>,
    pub unavailable: Vec<String>,
    pub handoff: String,
    pub native_available: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MigrationResult {
    pub operation_id: String,
    pub target_id: String,
    pub target_path: String,
    pub resume_command: String,
    pub cwd: String,
    pub reused: bool,
    pub warnings: Vec<String>,
    pub mode: String,
    pub provider: Provider,
}
