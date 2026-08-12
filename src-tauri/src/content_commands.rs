use crate::RuntimeState;
use crate::modrinth::{self, InstalledModView, SearchResults};
use crate::modrinth_content::{self, ContentKind, InstalledContentView};
use crate::storage;
use crate::{instances, modpack};
use serde::Serialize;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, State};

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ModInstallProgressEvent {
    instance_id: String,
    project_id: String,
    progress: f32,
}

#[tauri::command]
pub async fn list_instance_mods(
    instance_id: String,
    refresh_updates: bool,
) -> Result<Vec<InstalledModView>, String> {
    run_blocking(move || {
        let root = storage::user_data_dir()?;
        modrinth::list_installed(&root, &instance_id, refresh_updates)
    })
    .await
}

#[tauri::command]
pub async fn search_modrinth(
    instance_id: String,
    query: String,
    index: String,
    category: Option<String>,
    offset: usize,
) -> Result<SearchResults, String> {
    run_blocking(move || {
        let root = storage::user_data_dir()?;
        modrinth::search(
            &root,
            &instance_id,
            &query,
            &index,
            category.as_deref(),
            offset,
        )
    })
    .await
}

#[tauri::command]
pub async fn list_instance_content(
    instance_id: String,
    content_type: ContentKind,
    refresh_updates: bool,
) -> Result<Vec<InstalledContentView>, String> {
    run_blocking(move || {
        let root = storage::user_data_dir()?;
        modrinth_content::list_installed(&root, &instance_id, content_type, refresh_updates)
    })
    .await
}

#[tauri::command]
pub async fn search_modrinth_content(
    instance_id: String,
    content_type: ContentKind,
    query: String,
    index: String,
    category: Option<String>,
    offset: usize,
) -> Result<SearchResults, String> {
    run_blocking(move || {
        let root = storage::user_data_dir()?;
        modrinth_content::search(
            &root,
            &instance_id,
            content_type,
            &query,
            &index,
            category.as_deref(),
            offset,
        )
    })
    .await
}

#[tauri::command]
pub async fn install_modrinth_content(
    app: AppHandle,
    state: State<'_, RuntimeState>,
    instance_id: String,
    content_type: ContentKind,
    project_id: String,
) -> Result<Vec<InstalledContentView>, String> {
    ensure_stopped(&state, &instance_id)?;
    let operation_lock = Arc::clone(&state.content_operations);
    let progress_app = app.clone();
    let progress_instance_id = instance_id.clone();
    let progress_project_id = project_id.clone();
    run_blocking(move || {
        let _guard = operation_lock
            .lock()
            .map_err(|_| anyhow::anyhow!("Could not lock content operations"))?;
        let root = storage::user_data_dir()?;
        let report = move |progress: f32| {
            let _ = progress_app.emit(
                "mod-install-progress",
                ModInstallProgressEvent {
                    instance_id: progress_instance_id.clone(),
                    project_id: progress_project_id.clone(),
                    progress: progress.clamp(0.0, 1.0),
                },
            );
        };
        modrinth_content::install(&root, &instance_id, content_type, &project_id, &report)
    })
    .await
}

#[tauri::command]
pub async fn remove_modrinth_content(
    state: State<'_, RuntimeState>,
    instance_id: String,
    content_type: ContentKind,
    project_id: String,
) -> Result<Vec<InstalledContentView>, String> {
    ensure_stopped(&state, &instance_id)?;
    let operation_lock = Arc::clone(&state.content_operations);
    run_blocking(move || {
        let _guard = operation_lock
            .lock()
            .map_err(|_| anyhow::anyhow!("Could not lock content operations"))?;
        let root = storage::user_data_dir()?;
        modrinth_content::remove(&root, &instance_id, content_type, &project_id)
    })
    .await
}

#[tauri::command]
pub async fn set_modrinth_content_enabled(
    state: State<'_, RuntimeState>,
    instance_id: String,
    content_type: ContentKind,
    project_id: String,
    enabled: bool,
) -> Result<Vec<InstalledContentView>, String> {
    ensure_stopped(&state, &instance_id)?;
    let operation_lock = Arc::clone(&state.content_operations);
    run_blocking(move || {
        let _guard = operation_lock
            .lock()
            .map_err(|_| anyhow::anyhow!("Could not lock content operations"))?;
        let root = storage::user_data_dir()?;
        modrinth_content::set_enabled(&root, &instance_id, content_type, &project_id, enabled)
    })
    .await
}

#[tauri::command]
pub async fn install_modrinth_modpack(
    app: AppHandle,
    state: State<'_, RuntimeState>,
    source_instance_id: String,
    project_id: String,
) -> Result<instances::Instance, String> {
    let (root, instance, plan) = run_blocking(move || {
        let root = storage::user_data_dir()?;
        let source = instances::load(&root, &source_instance_id)?;
        let resolved = modpack::resolve(&project_id, &source.version)?;
        let existing = instances::load_all(&root)?;
        let instance = instances::create(
            &root,
            &resolved.name,
            &resolved.game_version,
            resolved.loader,
            &existing,
        )?;
        modpack::save_pending(&root, &instance, &resolved.plan)?;
        Ok((root, instance, resolved.plan))
    })
    .await?;
    let job_root = root.clone();
    let job_instance = instance.clone();
    let start = state.installations.start_job(
        app,
        instance.clone(),
        format!("Preparing {}...", instance.name),
        move |progress| modpack::install(&job_root, &job_instance, plan, progress.as_ref()),
    );
    if let Err(error) = start {
        let _ = instances::delete(&root, &instance.id);
        return Err(error.to_string());
    }
    Ok(instance)
}

#[tauri::command]
pub async fn set_modrinth_mod_enabled(
    app: AppHandle,
    state: State<'_, RuntimeState>,
    instance_id: String,
    project_id: String,
    enabled: bool,
) -> Result<Vec<InstalledModView>, String> {
    ensure_stopped(&state, &instance_id)?;
    let operation_lock = Arc::clone(&state.content_operations);
    let result = run_blocking(move || {
        let _guard = operation_lock
            .lock()
            .map_err(|_| anyhow::anyhow!("Could not lock mod operations"))?;
        let root = storage::user_data_dir()?;
        modrinth::set_enabled(&root, &instance_id, &project_id, enabled)
    })
    .await;
    if result.is_ok() {
        let _ = app.emit(
            "status",
            if enabled {
                "Mod enabled."
            } else {
                "Mod disabled."
            },
        );
    }
    result
}

#[tauri::command]
pub async fn install_modrinth_mod(
    app: AppHandle,
    state: State<'_, RuntimeState>,
    instance_id: String,
    project_id: String,
) -> Result<Vec<InstalledModView>, String> {
    ensure_stopped(&state, &instance_id)?;
    let operation_lock = Arc::clone(&state.content_operations);
    let progress_app = app.clone();
    let progress_instance_id = instance_id.clone();
    let progress_project_id = project_id.clone();
    let _ = app.emit("status", "Installing mod and required dependencies...");
    let result = run_blocking(move || {
        let _guard = operation_lock
            .lock()
            .map_err(|_| anyhow::anyhow!("Could not lock mod operations"))?;
        let root = storage::user_data_dir()?;
        let report = move |progress: f32| {
            let _ = progress_app.emit(
                "mod-install-progress",
                ModInstallProgressEvent {
                    instance_id: progress_instance_id.clone(),
                    project_id: progress_project_id.clone(),
                    progress: progress.clamp(0.0, 1.0),
                },
            );
        };
        modrinth::install(&root, &instance_id, &project_id, &report)
    })
    .await;
    if result.is_ok() {
        let _ = app.emit("status", "Mod installed.");
    }
    result
}

#[tauri::command]
pub async fn remove_modrinth_mod(
    app: AppHandle,
    state: State<'_, RuntimeState>,
    instance_id: String,
    project_id: String,
) -> Result<Vec<InstalledModView>, String> {
    ensure_stopped(&state, &instance_id)?;
    let operation_lock = Arc::clone(&state.content_operations);
    let result = run_blocking(move || {
        let _guard = operation_lock
            .lock()
            .map_err(|_| anyhow::anyhow!("Could not lock mod operations"))?;
        let root = storage::user_data_dir()?;
        modrinth::remove(&root, &instance_id, &project_id)
    })
    .await;
    if result.is_ok() {
        let _ = app.emit("status", "Mod removed.");
    }
    result
}

#[tauri::command]
pub async fn update_modrinth_mod(
    app: AppHandle,
    state: State<'_, RuntimeState>,
    instance_id: String,
    project_id: String,
) -> Result<Vec<InstalledModView>, String> {
    ensure_stopped(&state, &instance_id)?;
    let operation_lock = Arc::clone(&state.content_operations);
    let _ = app.emit("status", "Updating mod and dependencies...");
    let result = run_blocking(move || {
        let _guard = operation_lock
            .lock()
            .map_err(|_| anyhow::anyhow!("Could not lock mod operations"))?;
        let root = storage::user_data_dir()?;
        modrinth::update(&root, &instance_id, &project_id)
    })
    .await;
    if result.is_ok() {
        let _ = app.emit("status", "Mod updated.");
    }
    result
}

#[tauri::command]
pub async fn update_all_modrinth_mods(
    app: AppHandle,
    state: State<'_, RuntimeState>,
    instance_id: String,
) -> Result<Vec<InstalledModView>, String> {
    ensure_stopped(&state, &instance_id)?;
    let operation_lock = Arc::clone(&state.content_operations);
    let _ = app.emit("status", "Updating compatible mods...");
    let result = run_blocking(move || {
        let _guard = operation_lock
            .lock()
            .map_err(|_| anyhow::anyhow!("Could not lock mod operations"))?;
        let root = storage::user_data_dir()?;
        modrinth::update_all(&root, &instance_id)
    })
    .await;
    if result.is_ok() {
        let _ = app.emit("status", "Mods are up to date.");
    }
    result
}

fn ensure_stopped(state: &State<'_, RuntimeState>, instance_id: &str) -> Result<(), String> {
    if state
        .running_instances
        .lock()
        .map_err(|_| "Could not read instance status".to_owned())?
        .contains(instance_id)
    {
        return Err("Stop Minecraft before changing installed mods".to_owned());
    }
    Ok(())
}

async fn run_blocking<T, F>(operation: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> anyhow::Result<T> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(operation)
        .await
        .map_err(|error| format!("Internal error: {error}"))?
        .map_err(|error| format!("{error:#}"))
}
