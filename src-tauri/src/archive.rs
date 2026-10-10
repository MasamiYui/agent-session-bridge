use crate::{model::*, parser, store::Store};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};
use walkdir::WalkDir;
use zip::{write::SimpleFileOptions, ZipArchive, ZipWriter};
const MAX_ENTRY: u64 = 256 * 1024 * 1024;
const MAX_TOTAL: u64 = 1024 * 1024 * 1024;
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    pub format: String,
    pub version: u32,
    pub session: Session,
    pub raw_sha256: String,
    pub missing_attachments: Vec<String>,
}
pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| error("Missing parent directory"))?;
    fs::create_dir_all(parent)?;
    let tmp = parent.join(format!(".asb-{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut f = OpenOptions::new().write(true).create_new(true).open(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        fs::hard_link(&tmp, path)
            .map_err(|e| error(format!("Refusing to overwrite {}: {e}", path.display())))?;
        Ok(())
    })();
    let _ = fs::remove_file(tmp);
    result
}
pub fn export(session: &Session, path: &Path, format: &str) -> Result<()> {
    match format {
        "markdown" => atomic_write(path, parser::markdown(session).as_bytes()),
        "jsonl" => atomic_write(path, &fs::read(&session.summary.path)?),
        "asb" => {
            let raw = fs::read(&session.summary.path)?;
            if raw.len() as u64 > MAX_ENTRY {
                return Err(error("Transcript exceeds 256 MB archive entry limit"));
            }
            if format!("{:x}", Sha256::digest(&raw)) != session.summary.fingerprint {
                return Err(error("Source changed during export; refresh and retry"));
            }
            let manifest = Manifest {
                format: "agent-session-bridge".into(),
                version: 1,
                session: session.clone(),
                raw_sha256: session.summary.fingerprint.clone(),
                missing_attachments: session
                    .attachments
                    .iter()
                    .filter(|a| !a.available)
                    .map(|a| a.source.clone())
                    .collect(),
            };
            let mut z = ZipWriter::new(std::io::Cursor::new(Vec::new()));
            let opt =
                SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
            z.start_file("manifest.json", opt)?;
            z.write_all(&serde_json::to_vec_pretty(&manifest)?)?;
            z.start_file("raw/session.jsonl", opt)?;
            z.write_all(&raw)?;
            for a in &session.attachments {
                if a.available {
                    if fs::metadata(&a.source)?.len() > MAX_ENTRY {
                        return Err(error("Attachment exceeds 256 MB archive entry limit"));
                    }
                    z.start_file(&a.archive_path, opt)?;
                    std::io::copy(&mut File::open(&a.source)?, &mut z)?;
                }
            }
            let src = Path::new(&session.summary.path);
            if let Some(stem) = src.file_stem() {
                let subs = src.with_file_name(stem).join("subagents");
                if subs.is_dir() {
                    for e in WalkDir::new(&subs)
                        .follow_links(false)
                        .into_iter()
                        .flatten()
                        .filter(|e| e.file_type().is_file())
                    {
                        if e.metadata().map(|m| m.len()).unwrap_or(MAX_ENTRY + 1) > MAX_ENTRY {
                            return Err(error("Sidechain exceeds archive entry limit"));
                        }
                        let relative = e
                            .path()
                            .strip_prefix(&subs)
                            .map_err(|e| error(e.to_string()))?;
                        z.start_file(
                            format!(
                                "raw/subagents/{}",
                                relative.to_string_lossy().replace('\\', "/")
                            ),
                            opt,
                        )?;
                        std::io::copy(&mut File::open(e.path())?, &mut z)?;
                    }
                }
            }
            let data = z.finish()?.into_inner();
            atomic_write(path, &data)
        }
        _ => Err(error("Unsupported export format")),
    }
}
fn inspect_zip(path: &Path) -> Result<ZipArchive<File>> {
    let mut z = ZipArchive::new(File::open(path)?)?;
    let mut total = 0u64;
    if z.len() > 10000 {
        return Err(error("Archive contains too many entries"));
    }
    let mut names = HashSet::new();
    for i in 0..z.len() {
        let f = z.by_index(i)?;
        if !names.insert(f.name().trim_end_matches('/').to_lowercase()) {
            return Err(error("Duplicate or case-colliding archive path"));
        }
        let p = f
            .enclosed_name()
            .ok_or_else(|| error("Unsafe archive path"))?;
        if p.components()
            .any(|c| !matches!(c, std::path::Component::Normal(_)))
            || f.name().contains('\\')
            || f.name().contains(':')
            || p.components().any(|c| {
                let name = c.as_os_str().to_string_lossy();
                let stem = name.split('.').next().unwrap_or("").to_uppercase();
                name.ends_with(['.', ' '])
                    || matches!(
                        stem.as_str(),
                        "CON"
                            | "PRN"
                            | "AUX"
                            | "NUL"
                            | "COM1"
                            | "COM2"
                            | "COM3"
                            | "COM4"
                            | "COM5"
                            | "COM6"
                            | "COM7"
                            | "COM8"
                            | "COM9"
                            | "LPT1"
                            | "LPT2"
                            | "LPT3"
                            | "LPT4"
                            | "LPT5"
                            | "LPT6"
                            | "LPT7"
                            | "LPT8"
                            | "LPT9"
                    )
            })
        {
            return Err(error("Unsafe archive path"));
        }
        if f.unix_mode().is_some_and(|m| m & 0o170000 == 0o120000) {
            return Err(error("Archive symlinks are not supported"));
        }
        if f.size() > MAX_ENTRY {
            return Err(error("Archive entry exceeds 256 MB"));
        }
        total = total
            .checked_add(f.size())
            .ok_or_else(|| error("Archive size overflow"))?;
        if total > MAX_TOTAL {
            return Err(error("Archive exceeds 1 GB expanded size"));
        }
    }
    Ok(z)
}
pub fn preview_import(path: &Path) -> Result<Session> {
    if path.extension().and_then(|s| s.to_str()) == Some("asb") {
        let mut z = inspect_zip(path)?;
        let mut manifest = String::new();
        z.by_name("manifest.json")?.read_to_string(&mut manifest)?;
        let m: Manifest = serde_json::from_str(&manifest)?;
        if m.format != "agent-session-bridge" || m.version != 1 {
            return Err(error("Unsupported archive version"));
        }
        let mut raw = Vec::new();
        z.by_name("raw/session.jsonl")?.read_to_end(&mut raw)?;
        if format!("{:x}", Sha256::digest(&raw)) != m.raw_sha256 {
            return Err(error("Archive transcript checksum mismatch"));
        }
        // Canonical messages always come from the checksum-verified raw transcript.
        // A manifest cannot replace messages or nominate arbitrary local attachments.
        let mut tmp = tempfile::NamedTempFile::new()?;
        tmp.write_all(&raw)?;
        let mut session = parser::parse(tmp.path(), None)?;
        session.summary.organization = m.session.summary.organization;
        session.summary.title = m.session.summary.title;
        session.summary.path = path.to_string_lossy().into();
        session.attachments = m
            .session
            .attachments
            .into_iter()
            .map(|mut a| {
                a.available =
                    safe_attachment_entry(&a.archive_path) && z.by_name(&a.archive_path).is_ok();
                a
            })
            .collect();
        Ok(session)
    } else {
        parser::parse(path, None)
    }
}
fn safe_attachment_entry(name: &str) -> bool {
    name.starts_with("attachments/")
        && !name.contains(['\\', ':'])
        && Path::new(name)
            .components()
            .all(|c| matches!(c, std::path::Component::Normal(_)))
}
/// Restore only collected files owned by this imported directory; never use manifest source paths.
pub fn supplement(session: &mut Session) -> Result<()> {
    let raw = Path::new(&session.summary.path);
    let parent = raw
        .parent()
        .ok_or_else(|| error("Missing import directory"))?;
    let manifest = parent.join("manifest.json");
    if !parent.join(".complete").is_file() || !manifest.is_file() {
        // Imported loose JSONL must not nominate arbitrary files on this computer.
        for attachment in &mut session.attachments {
            attachment.available = false;
        }
        return Ok(());
    }
    let m: Manifest = serde_json::from_slice(&fs::read(manifest)?)?;
    if m.raw_sha256 != session.summary.fingerprint {
        return Err(error("Imported archive transcript changed"));
    }
    session.summary.title = m.session.summary.title;
    session.attachments = m
        .session
        .attachments
        .into_iter()
        .map(|mut a| {
            if safe_attachment_entry(&a.archive_path) {
                let local = parent.join(&a.archive_path);
                a.available = local.is_file();
                a.source = local.to_string_lossy().into();
            } else {
                a.available = false;
            }
            a
        })
        .collect();
    Ok(())
}
pub fn import(store: &Store, path: &Path) -> Result<Session> {
    let preview = preview_import(path)?;
    let identity = parser::fingerprint(path)?;
    let imports = store.root.join("imports");
    fs::create_dir_all(&imports)?;
    let dest = imports.join(&identity);
    if !dest.join(".complete").is_file() {
        if dest.exists() {
            return Err(error("Incomplete earlier import retained for inspection; remove its Bridge import directory before retrying"));
        }
        let staging = tempfile::tempdir_in(&store.root)?;
        if path.extension().and_then(|s| s.to_str()) == Some("asb") {
            let mut z = inspect_zip(path)?;
            for i in 0..z.len() {
                let mut f = z.by_index(i)?;
                if f.is_dir() {
                    continue;
                }
                let name = f.name().to_owned();
                let target = if name == "raw/session.jsonl" {
                    staging.path().join("session.jsonl")
                } else if let Some(relative) = name.strip_prefix("raw/subagents/") {
                    staging.path().join("session/subagents").join(relative)
                } else if name == "manifest.json" || safe_attachment_entry(&name) {
                    staging.path().join(&name)
                } else {
                    continue;
                };
                let mut bytes = Vec::new();
                f.read_to_end(&mut bytes)?;
                atomic_write(&target, &bytes)?;
            }
        } else {
            atomic_write(&staging.path().join("session.jsonl"), &fs::read(path)?)?;
        }
        if parser::fingerprint(path)? != identity {
            return Err(error("Import source changed; preview again"));
        }
        if parser::fingerprint(&staging.path().join("session.jsonl"))?
            != preview.summary.fingerprint
        {
            return Err(error("Imported transcript differs from preview"));
        }
        atomic_write(&staging.path().join(".complete"), b"1")?;
        // Every extraction is complete before the directory becomes visible to scans.
        if let Err(e) = fs::rename(staging.path(), &dest) {
            if !dest.join(".complete").is_file() {
                return Err(e.into());
            }
        }
    }
    let mut s = parser::parse(&dest.join("session.jsonl"), Some(preview.summary.provider))?;
    supplement(&mut s)?;
    s.summary.organization = preview.summary.organization;
    s.summary.title = preview.summary.title;
    store.index_session(&s, 0)?;
    s.summary.organization = store.organization(&s.summary.key)?;
    Ok(s)
}
pub fn unique_export_path(dir: &Path, title: &str, extension: &str) -> PathBuf {
    let safe = title
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                '\0'.max(c)
            } else {
                '_'
            }
        })
        .take(80)
        .collect::<String>();
    dir.join(format!(
        "{}-{}.{}",
        if safe.is_empty() { "session" } else { &safe },
        uuid::Uuid::new_v4(),
        extension
    ))
}
