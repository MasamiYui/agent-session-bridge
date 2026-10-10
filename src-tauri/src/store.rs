use crate::{model::*, parser};
use rusqlite::{params, Connection, OptionalExtension};
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
};
use walkdir::WalkDir;

pub struct Store {
    pub root: PathBuf,
}
impl Store {
    pub fn new(root: PathBuf) -> Result<Self> {
        fs::create_dir_all(&root)?;
        let s = Self { root };
        let c = s.connect()?;
        c.execute_batch("PRAGMA journal_mode=WAL; CREATE TABLE IF NOT EXISTS sessions(key TEXT PRIMARY KEY,path TEXT NOT NULL,fingerprint TEXT NOT NULL,mtime INTEGER NOT NULL,size INTEGER NOT NULL,summary TEXT NOT NULL,organization TEXT NOT NULL DEFAULT '{}',present INTEGER NOT NULL DEFAULT 1); CREATE VIRTUAL TABLE IF NOT EXISTS search USING fts5(key UNINDEXED,title,body,tokenize='unicode61'); CREATE TABLE IF NOT EXISTS operations(id TEXT PRIMARY KEY,identity TEXT NOT NULL,status TEXT NOT NULL,request TEXT NOT NULL,result TEXT,created_at TEXT NOT NULL); CREATE TABLE IF NOT EXISTS settings(id INTEGER PRIMARY KEY CHECK(id=1),json TEXT NOT NULL); CREATE TABLE IF NOT EXISTS project_mappings(source TEXT PRIMARY KEY,target TEXT NOT NULL);")?;
        Ok(s)
    }
    pub fn connect(&self) -> Result<Connection> {
        let c = Connection::open(self.root.join("bridge.sqlite"))?;
        c.busy_timeout(std::time::Duration::from_secs(10))?;
        Ok(c)
    }
    pub fn settings(&self) -> Result<Settings> {
        let c = self.connect()?;
        let value: Option<String> = c
            .query_row("SELECT json FROM settings WHERE id=1", [], |r| r.get(0))
            .optional()?;
        Ok(value
            .and_then(|v| serde_json::from_str(&v).ok())
            .unwrap_or_default())
    }
    pub fn save_settings(&self, v: &Settings) -> Result<()> {
        self.connect()?.execute("INSERT INTO settings(id,json) VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET json=excluded.json",[serde_json::to_string(v)?])?;
        Ok(())
    }
    pub fn organization(&self, key: &str) -> Result<Organization> {
        let raw: Option<String> = self
            .connect()?
            .query_row(
                "SELECT organization FROM sessions WHERE key=?1",
                [key],
                |r| r.get(0),
            )
            .optional()?;
        Ok(raw
            .and_then(|x| serde_json::from_str(&x).ok())
            .unwrap_or_default())
    }
    pub fn update(&self, key: &str, org: &Organization) -> Result<()> {
        self.connect()?.execute(
            "UPDATE sessions SET organization=?1 WHERE key=?2",
            params![serde_json::to_string(org)?, key],
        )?;
        Ok(())
    }
    pub fn session(&self, key: &str) -> Result<Session> {
        let c = self.connect()?;
        let summary: String =
            c.query_row("SELECT summary FROM sessions WHERE key=?1", [key], |r| {
                r.get(0)
            })?;
        let summary: SessionSummary = serde_json::from_str(&summary)?;
        let mut s = parser::parse(Path::new(&summary.path), Some(summary.provider))?;
        if Path::new(&summary.path).starts_with(self.root.join("imports")) {
            crate::archive::supplement(&mut s)?;
        }
        s.summary.organization = self.organization(key)?;
        if summary.title != s.summary.title && summary.provider == Provider::Codex {
            s.summary.title = summary.title
        }
        Ok(s)
    }
    pub fn index_session(&self, s: &Session, mtime: i64) -> Result<()> {
        let mut c = self.connect()?;
        let tx = c.transaction()?;
        tx.execute("INSERT INTO sessions(key,path,fingerprint,mtime,size,summary,organization,present) VALUES(?1,?2,?3,?4,?5,?6,?7,1) ON CONFLICT(key) DO UPDATE SET path=excluded.path,fingerprint=excluded.fingerprint,mtime=excluded.mtime,size=excluded.size,summary=excluded.summary,present=1",params![s.summary.key,s.summary.path,s.summary.fingerprint,mtime,s.summary.size,serde_json::to_string(&s.summary)?,serde_json::to_string(&s.summary.organization)?])?;
        tx.execute("DELETE FROM search WHERE key=?1", [&s.summary.key])?;
        let body = s
            .messages
            .iter()
            .flat_map(|m| m.blocks.iter())
            .map(|b| format!("{} {} {}", b.text, b.name, b.input))
            .collect::<Vec<_>>()
            .join("\n");
        tx.execute(
            "INSERT INTO search(key,title,body) VALUES(?1,?2,?3)",
            params![s.summary.key, s.summary.title, body],
        )?;
        tx.commit()?;
        Ok(())
    }
    pub fn list(&self, query: &str) -> Result<Vec<SessionSummary>> {
        let c = self.connect()?;
        let mut stmt = c.prepare(
            "SELECT summary,organization FROM sessions WHERE present=1 ORDER BY mtime DESC",
        )?;
        let mut rows = stmt.query([])?;
        let mut out = Vec::new();
        let mut found = std::collections::HashSet::new();
        if !query.is_empty() {
            let escaped = query
                .split_whitespace()
                .map(|x| format!("\"{}\"*", x.replace('"', "\"\"")))
                .collect::<Vec<_>>()
                .join(" AND ");
            let mut q = c.prepare("SELECT key FROM search WHERE search MATCH ?1")?;
            if let Ok(r) = q.query_map([escaped], |r| r.get::<_, String>(0)) {
                for k in r.flatten() {
                    found.insert(k);
                }
            };
        }
        let query = query.to_lowercase();
        while let Some(r) = rows.next()? {
            let mut s: SessionSummary = serde_json::from_str(&r.get::<_, String>(0)?)?;
            s.organization = serde_json::from_str(&r.get::<_, String>(1)?).unwrap_or_default();
            if query.is_empty()
                || found.contains(&s.key)
                || s.title.to_lowercase().contains(&query)
                || s.cwd.to_lowercase().contains(&query)
                || s.organization.alias.to_lowercase().contains(&query)
                || s.organization
                    .tags
                    .join(" ")
                    .to_lowercase()
                    .contains(&query)
            {
                out.push(s)
            }
        }
        Ok(out)
    }
    pub fn scan(&self) -> Result<Vec<String>> {
        let settings = self.settings()?;
        let titles = codex_titles(Path::new(&settings.codex_home));
        let mut warnings = Vec::new();
        let mut seen = Vec::new();
        let c = self.connect()?;
        for (provider, roots) in [
            (
                Provider::Codex,
                vec![
                    PathBuf::from(&settings.codex_home).join("sessions"),
                    PathBuf::from(&settings.codex_home).join("archived_sessions"),
                ],
            ),
            (
                Provider::Claude,
                vec![PathBuf::from(&settings.claude_home).join("projects")],
            ),
            (Provider::Codex, vec![self.root.join("imports")]),
        ] {
            for root in roots {
                if !root.is_dir() {
                    continue;
                }
                for entry in WalkDir::new(root).follow_links(false).into_iter() {
                    let entry = match entry {
                        Ok(e) => e,
                        Err(e) => {
                            warnings.push(e.to_string());
                            continue;
                        }
                    };
                    let path = entry.path();
                    if !entry.file_type().is_file()
                        || path.extension().and_then(|x| x.to_str()) != Some("jsonl")
                    {
                        continue;
                    }
                    if path.components().any(|x| x.as_os_str() == "subagents") {
                        continue;
                    }
                    let meta = path.metadata()?;
                    let mtime = meta
                        .modified()?
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_millis() as i64;
                    let path_str = path.to_string_lossy();
                    seen.push(path_str.to_string());
                    let cached: Option<(i64, u64)> = c
                        .query_row(
                            "SELECT mtime,size FROM sessions WHERE path=?1",
                            [path_str.as_ref()],
                            |r| Ok((r.get(0)?, r.get(1)?)),
                        )
                        .optional()?;
                    if cached == Some((mtime, meta.len())) {
                        c.execute(
                            "UPDATE sessions SET present=1 WHERE path=?1",
                            [path_str.as_ref()],
                        )?;
                        continue;
                    }
                    match parser::parse(
                        path,
                        if path.starts_with(self.root.join("imports")) {
                            None
                        } else {
                            Some(provider)
                        },
                    ) {
                        Ok(mut s) => {
                            if path.starts_with(self.root.join("imports")) {
                                if let Err(e) = crate::archive::supplement(&mut s) {
                                    warnings.push(e.to_string());
                                    continue;
                                }
                            }
                            if let Some(t) = titles.get(&s.summary.id) {
                                s.summary.title = t.clone()
                            }
                            if let Ok(org) = self.organization(&s.summary.key) {
                                s.summary.organization = org
                            }
                            if let Err(e) = self.index_session(&s, mtime) {
                                warnings.push(e.to_string())
                            }
                        }
                        Err(e) => warnings.push(format!("{}: {e}", path.display())),
                    }
                }
            }
        }
        let mut st = c.prepare("SELECT path FROM sessions WHERE present=1")?;
        let paths = st
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        for path in paths {
            if !seen.contains(&path) {
                c.execute("UPDATE sessions SET present=0 WHERE path=?1", [path])?;
            }
        }
        Ok(warnings)
    }
}
fn codex_titles(home: &Path) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let mut paths = fs::read_dir(home)
        .ok()
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|x| x.to_str())
                .is_some_and(|x| x.starts_with("state_") && x.ends_with(".sqlite"))
        })
        .collect::<Vec<_>>();
    paths.sort();
    paths.reverse();
    for p in paths {
        if let Ok(c) = Connection::open_with_flags(p, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY) {
            if let Ok(mut st) = c.prepare("SELECT id,COALESCE(name,title) FROM threads") {
                if let Ok(rows) =
                    st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
                {
                    map.extend(rows.flatten());
                    break;
                }
            }
        }
    }
    map
}
