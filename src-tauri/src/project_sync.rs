use crate::{migration, model::*, rpc, store::Store};
use std::{collections::HashSet, fs, path::Path};

#[derive(Clone)]
struct Operation {
    identity: String,
    status: String,
    request: MigrationRequest,
    result: Option<MigrationResult>,
}
fn operations(store: &Store) -> Result<Vec<Operation>> {
    let c = store.connect()?;
    let mut q = c.prepare("SELECT id,identity,status,request,result FROM operations")?;
    let rows = q.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?,
            r.get::<_, Option<String>>(4)?,
        ))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (id, identity, status, request, raw) = row?;
        let request = serde_json::from_str(&request)?;
        let result = raw.and_then(|s| serde_json::from_str(&s).ok()).or_else(|| {
            // An interrupted publication is also an origin; never copy it back.
            let dir = store.root.join("operations").join(id);
            fs::read(dir.join("result.json"))
                .ok()
                .and_then(|b| serde_json::from_slice(&b).ok())
                .or_else(|| {
                    let intent: serde_json::Value =
                        serde_json::from_slice(&fs::read(dir.join("intent.json")).ok()?).ok()?;
                    serde_json::from_value(intent["result"].clone()).ok()
                })
        });
        out.push(Operation {
            identity,
            status,
            request,
            result,
        });
    }
    Ok(out)
}
fn canonical(cwd: &str) -> Result<String> {
    if !Path::new(cwd).is_absolute() || !Path::new(cwd).is_dir() {
        return Err(error("Choose an existing absolute project directory"));
    }
    Ok(fs::canonicalize(cwd)?
        .to_string_lossy()
        .trim_start_matches("\\\\?\\")
        .to_owned())
}
fn eligible(s: &SessionSummary, cwd: &str) -> bool {
    s.cwd == cwd && !s.organization.archived && !s.organization.removed && !s.source_archived
}
fn targets(source: Provider) -> [Provider; 2] {
    match source {
        Provider::Codex => [Provider::Claude, Provider::Antigravity],
        Provider::Claude => [Provider::Codex, Provider::Antigravity],
        Provider::Antigravity => [Provider::Codex, Provider::Claude],
    }
}
fn classify(
    s: &SessionSummary,
    target: Provider,
    canonical_cwd: &str,
    mode: &str,
    ops: &[Operation],
) -> ProjectSyncItem {
    let generated = ops.iter().any(|op| {
        op.result.as_ref().is_some_and(|r| {
            r.mode == "native"
                && r.provider == s.provider
                && (r.target_id == s.id || r.target_path == s.path)
        })
    });
    let same_version = |op: &Operation| {
        op.request.key == s.key
            && op.request.target == target
            && op.request.mode == mode
            && op.identity == migration::identity(&s.fingerprint, target, mode, canonical_cwd)
            && canonical(&op.request.cwd)
                .ok()
                .is_some_and(|path| path == canonical_cwd)
    };
    let existing = ops.iter().any(|op| {
        same_version(op)
            && op.status == "completed"
            && op
                .result
                .as_ref()
                .is_some_and(|r| Path::new(&r.target_path).is_file())
    });
    let interrupted = ops
        .iter()
        .any(|op| same_version(op) && matches!(op.status.as_str(), "running" | "interrupted"));
    let reason = if generated {
        "generated"
    } else if existing {
        "synced"
    } else if interrupted {
        "recovery"
    } else {
        ""
    };
    ProjectSyncItem {
        key: s.key.clone(),
        id: s.id.clone(),
        title: if s.organization.alias.is_empty() {
            s.title.clone()
        } else {
            s.organization.alias.clone()
        },
        source: s.provider,
        target,
        mode: mode.into(),
        action: if reason.is_empty() { "create" } else { "skip" }.into(),
        fingerprint: s.fingerprint.clone(),
        reason: reason.into(),
    }
}
pub fn plan(store: &Store, cwd: &str) -> Result<ProjectSyncPlan> {
    let normalized = canonical(cwd)?;
    store.scan()?;
    let settings = store.settings()?;
    let verified = |provider| rpc::capability(provider, &settings).native_verified;
    let ops = operations(store)?;
    let mut items = Vec::new();
    for summary in store.list("")?.into_iter().filter(|s| eligible(s, cwd)) {
        // Re-read pending sources so a same-millisecond append is not hidden by scan cache.
        let s = store.session(&summary.key)?;
        if !eligible(&s.summary, cwd) {
            continue;
        }
        for target in targets(s.summary.provider) {
            items.push(classify(
                &s.summary,
                target,
                &normalized,
                if verified(target) {
                    "native"
                } else {
                    "handoff"
                },
                &ops,
            ));
        }
    }
    Ok(ProjectSyncPlan {
        cwd: cwd.into(),
        items,
    })
}
pub fn execute(
    store: &Store,
    cwd: &str,
    request: &MigrationRequest,
    expected: &str,
) -> Result<MigrationResult> {
    let normalized = canonical(cwd)?;
    let s = store.session(&request.key)?;
    if !eligible(&s.summary, cwd) {
        return Err(error(
            "Session no longer belongs to this active project. Preview sync again.",
        ));
    }
    if s.summary.fingerprint != expected {
        return Err(error("Source changed since preview. Preview sync again."));
    }
    let item = classify(
        &s.summary,
        request.target,
        &normalized,
        &request.mode,
        &operations(store)?,
    );
    if item.target != request.target
        || request.cwd != cwd
        || request.duplicate
        || request.handoff_text.is_some()
    {
        return Err(error("Invalid project sync request"));
    }
    if matches!(item.reason.as_str(), "generated" | "recovery") {
        return Err(error(format!(
            "Session cannot be synchronized: {}. Preview sync again.",
            item.reason
        )));
    }
    // Completed versions reuse migration results; changed histories get a new copy.
    // Neither a source nor an independently continued target is overwritten.
    migration::execute(store, request, expected)
}
pub fn validate_items(items: &[ProjectSyncItem]) -> Result<()> {
    let mut copies = HashSet::new();
    if items.is_empty()
        || items
            .iter()
            .any(|i| i.action != "create" || !copies.insert((&i.key, i.target)))
    {
        return Err(error(
            "Preview pending project sessions before starting sync",
        ));
    }
    Ok(())
}
