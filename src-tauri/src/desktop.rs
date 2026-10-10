use crate::{archive, migration, model::*, project_sync, rpc, store::Store};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};
use tauri::{Emitter, Manager, State};
pub struct AppState {
    store: Arc<Store>,
    jobs: Mutex<HashMap<String, Arc<AtomicBool>>>,
    operation_lock: Arc<Mutex<()>>,
}
async fn work<T: Send + 'static>(
    f: impl FnOnce() -> Result<T> + Send + 'static,
) -> std::result::Result<T, String> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}
#[tauri::command]
async fn list_sessions(
    state: State<'_, AppState>,
    query: String,
) -> std::result::Result<Vec<SessionSummary>, String> {
    let store = state.store.clone();
    work(move || store.list(&query)).await
}
#[tauri::command]
async fn scan_sessions(state: State<'_, AppState>) -> std::result::Result<Vec<String>, String> {
    let store = state.store.clone();
    work(move || store.scan()).await
}
#[tauri::command]
async fn read_session(
    state: State<'_, AppState>,
    key: String,
) -> std::result::Result<Session, String> {
    let store = state.store.clone();
    work(move || store.session(&key)).await
}
#[tauri::command]
async fn update_organization(
    state: State<'_, AppState>,
    key: String,
    organization: Organization,
) -> std::result::Result<(), String> {
    let store = state.store.clone();
    work(move || store.update(&key, &organization)).await
}
#[tauri::command]
async fn get_settings(state: State<'_, AppState>) -> std::result::Result<Settings, String> {
    state.store.settings().map_err(|e| e.to_string())
}
#[tauri::command]
async fn save_settings(
    state: State<'_, AppState>,
    settings: Settings,
) -> std::result::Result<(), String> {
    state
        .store
        .save_settings(&settings)
        .map_err(|e| e.to_string())
}
#[tauri::command]
async fn capabilities(state: State<'_, AppState>) -> std::result::Result<Vec<Capability>, String> {
    let settings = state.store.settings().map_err(|e| e.to_string())?;
    work(move || {
        Ok(vec![
            rpc::capability(Provider::Codex, &settings),
            rpc::capability(Provider::Claude, &settings),
        ])
    })
    .await
}
#[tauri::command]
async fn migration_plan(
    state: State<'_, AppState>,
    key: String,
    target: Provider,
) -> std::result::Result<MigrationPlan, String> {
    let store = state.store.clone();
    work(move || {
        Ok(migration::plan(
            &store.session(&key)?,
            target,
            &store.settings()?,
        ))
    })
    .await
}
#[tauri::command]
async fn preview_import(path: String) -> std::result::Result<Session, String> {
    work(move || archive::preview_import(Path::new(&path))).await
}
#[tauri::command]
async fn import_file(
    state: State<'_, AppState>,
    path: String,
) -> std::result::Result<Session, String> {
    let store = state.store.clone();
    work(move || archive::import(&store, Path::new(&path))).await
}
#[tauri::command]
async fn export_session(
    state: State<'_, AppState>,
    key: String,
    path: String,
    format: String,
) -> std::result::Result<(), String> {
    let store = state.store.clone();
    work(move || archive::export(&store.session(&key)?, Path::new(&path), &format)).await
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct JobEvent {
    job_id: String,
    index: usize,
    total: usize,
    status: String,
    error: Option<String>,
    result: Option<MigrationResult>,
    path: Option<String>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct JobItem {
    request: MigrationRequest,
    fingerprint: String,
}
#[tauri::command]
async fn project_sync_plan(
    state: State<'_, AppState>,
    cwd: String,
) -> std::result::Result<ProjectSyncPlan, String> {
    let store = state.store.clone();
    let lock = state.operation_lock.clone();
    work(move || {
        let _guard = lock.lock().unwrap_or_else(|e| e.into_inner());
        project_sync::plan(&store, &cwd)
    })
    .await
}
#[tauri::command]
async fn start_project_sync(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    plan: ProjectSyncPlan,
) -> std::result::Result<String, String> {
    project_sync::validate_items(&plan.items).map_err(|e| e.to_string())?;
    let items = plan
        .items
        .into_iter()
        .map(|item| JobItem {
            request: MigrationRequest {
                key: item.key,
                target: item.target,
                mode: item.mode,
                cwd: plan.cwd.clone(),
                duplicate: false,
                handoff_text: None,
            },
            fingerprint: item.fingerprint,
        })
        .collect();
    run_migrations(app, state, items, Some(plan.cwd)).await
}
#[tauri::command]
async fn start_migrations(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    items: Vec<JobItem>,
) -> std::result::Result<String, String> {
    run_migrations(app, state, items, None).await
}
async fn run_migrations(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    items: Vec<JobItem>,
    project: Option<String>,
) -> std::result::Result<String, String> {
    let id = uuid::Uuid::new_v4().to_string();
    let cancelled = Arc::new(AtomicBool::new(false));
    state
        .jobs
        .lock()
        .map_err(|e| e.to_string())?
        .insert(id.clone(), cancelled.clone());
    let store = state.store.clone();
    let job_id = id.clone();
    let operation_lock = state.operation_lock.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = operation_lock.lock().unwrap_or_else(|e| e.into_inner());
        let total = items.len();
        for (index, item) in items.into_iter().enumerate() {
            if cancelled.load(Ordering::SeqCst) {
                let _ = app.emit(
                    "job-progress",
                    JobEvent {
                        job_id: job_id.clone(),
                        index,
                        total,
                        status: "cancelled".into(),
                        error: None,
                        result: None,
                        path: None,
                    },
                );
                continue;
            }
            let _ = app.emit(
                "job-progress",
                JobEvent {
                    job_id: job_id.clone(),
                    index,
                    total,
                    status: "running".into(),
                    error: None,
                    result: None,
                    path: None,
                },
            );
            let result = match &project {
                Some(cwd) => project_sync::execute(&store, cwd, &item.request, &item.fingerprint),
                None => migration::execute(&store, &item.request, &item.fingerprint),
            };
            let (status, error, result) = match result {
                Ok(r) => ("completed", None, Some(r)),
                Err(e) => ("failed", Some(e.to_string()), None),
            };
            let _ = app.emit(
                "job-progress",
                JobEvent {
                    job_id: job_id.clone(),
                    index,
                    total,
                    status: status.into(),
                    error,
                    result,
                    path: None,
                },
            );
        }
        if project.is_some() {
            let _ = store.scan();
        }
        let _ = app.emit(
            "job-progress",
            JobEvent {
                job_id: job_id.clone(),
                index: total,
                total,
                status: "finished".into(),
                error: None,
                result: None,
                path: None,
            },
        );
        if let Some(state) = app.try_state::<AppState>() {
            if let Ok(mut jobs) = state.jobs.lock() {
                jobs.remove(&job_id);
            }
        }
    });
    Ok(id)
}
#[tauri::command]
async fn start_exports(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    keys: Vec<String>,
    directory: String,
    format: String,
) -> std::result::Result<String, String> {
    let id = uuid::Uuid::new_v4().to_string();
    let cancel = Arc::new(AtomicBool::new(false));
    state
        .jobs
        .lock()
        .map_err(|e| e.to_string())?
        .insert(id.clone(), cancel.clone());
    let store = state.store.clone();
    let job_id = id.clone();
    let operation_lock = state.operation_lock.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = operation_lock.lock().unwrap_or_else(|e| e.into_inner());
        let total = keys.len();
        for (index, key) in keys.iter().enumerate() {
            if cancel.load(Ordering::SeqCst) {
                let _ = app.emit(
                    "job-progress",
                    JobEvent {
                        job_id: job_id.clone(),
                        index,
                        total,
                        status: "cancelled".into(),
                        error: None,
                        result: None,
                        path: None,
                    },
                );
                continue;
            }
            let _ = app.emit(
                "job-progress",
                JobEvent {
                    job_id: job_id.clone(),
                    index,
                    total,
                    status: "running".into(),
                    error: None,
                    result: None,
                    path: None,
                },
            );
            let result = (|| {
                let s = store.session(key)?;
                let ext = match format.as_str() {
                    "markdown" => "md",
                    "jsonl" => "jsonl",
                    _ => "asb",
                };
                let path =
                    archive::unique_export_path(Path::new(&directory), &s.summary.title, ext);
                archive::export(&s, &path, &format)?;
                Ok::<_, BridgeError>(path)
            })();
            let (status, error, path) = match result {
                Ok(p) => ("completed", None, Some(p.to_string_lossy().into())),
                Err(e) => ("failed", Some(e.to_string()), None),
            };
            let _ = app.emit(
                "job-progress",
                JobEvent {
                    job_id: job_id.clone(),
                    index,
                    total,
                    status: status.into(),
                    error,
                    result: None,
                    path,
                },
            );
        }
        let _ = app.emit(
            "job-progress",
            JobEvent {
                job_id: job_id.clone(),
                index: total,
                total,
                status: "finished".into(),
                error: None,
                result: None,
                path: None,
            },
        );
        if let Some(state) = app.try_state::<AppState>() {
            if let Ok(mut jobs) = state.jobs.lock() {
                jobs.remove(&job_id);
            }
        }
    });
    Ok(id)
}
#[tauri::command]
fn cancel_job(state: State<'_, AppState>, id: String) -> std::result::Result<(), String> {
    if let Some(c) = state.jobs.lock().map_err(|e| e.to_string())?.get(&id) {
        c.store(true, Ordering::SeqCst)
    }
    Ok(())
}
#[tauri::command]
async fn operation_history(
    state: State<'_, AppState>,
) -> std::result::Result<serde_json::Value, String> {
    let store = state.store.clone();
    work(move||{let c=store.connect()?;let mut q=c.prepare("SELECT id,status,result,created_at FROM operations ORDER BY created_at DESC LIMIT 100")?;let rows=q.query_map([],|r|Ok(serde_json::json!({"id":r.get::<_,String>(0)?,"status":r.get::<_,String>(1)?,"result":r.get::<_,Option<String>>(2)?,"createdAt":r.get::<_,String>(3)?})))?;Ok(serde_json::Value::Array(rows.collect::<std::result::Result<Vec<_>,_>>()?))}).await
}
#[tauri::command]
async fn launch_session(
    state: State<'_, AppState>,
    key: Option<String>,
    operation_id: Option<String>,
) -> std::result::Result<(), String> {
    let store = state.store.clone();
    work(move || {
        if let Some(id) = operation_id {
            let c = store.connect()?;
            let raw: String = c.query_row(
                "SELECT result FROM operations WHERE id=?1 AND status='completed'",
                [id],
                |r| r.get(0),
            )?;
            let r: MigrationResult = serde_json::from_str(&raw)?;
            return rpc::open_terminal(&r.resume_command, &r.cwd);
        }
        let s = store.session(&key.ok_or_else(|| error("Missing session"))?)?;
        let settings = store.settings()?;
        let command = rpc::session_command(
            s.summary.provider,
            &settings,
            &[
                if s.summary.provider == Provider::Codex {
                    "resume"
                } else {
                    "--resume"
                },
                &s.summary.id,
            ],
        );
        rpc::open_terminal(&command, &s.summary.cwd)
    })
    .await
}
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let root = std::env::var("ASB_DATA_DIR")
                .map(PathBuf::from)
                .unwrap_or(app.path().app_data_dir()?);
            let store = Arc::new(Store::new(root)?);
            let recovered = migration::recover(&store)?;
            if !recovered.is_empty() {
                eprintln!(
                    "{} interrupted operations retained for inspection",
                    recovered.len()
                )
            }
            app.manage(AppState {
                store: store.clone(),
                jobs: Mutex::new(HashMap::new()),
                operation_lock: Arc::new(Mutex::new(())),
            });
            let handle = app.handle().clone();
            std::thread::spawn(move || loop {
                match store.scan() {
                    Ok(warnings) => {
                        let _ = handle.emit("sessions-changed", warnings);
                    }
                    Err(e) => {
                        let _ = handle.emit("scan-error", e.to_string());
                    }
                }
                std::thread::sleep(std::time::Duration::from_secs(15));
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            list_sessions,
            scan_sessions,
            read_session,
            update_organization,
            get_settings,
            save_settings,
            capabilities,
            migration_plan,
            project_sync_plan,
            start_project_sync,
            preview_import,
            import_file,
            export_session,
            start_migrations,
            start_exports,
            cancel_job,
            operation_history,
            launch_session
        ])
        .run(tauri::generate_context!())
        .expect("Unable to start Agent Session Bridge")
}
