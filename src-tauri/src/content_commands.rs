use crate::RuntimeState;
use crate::modrinth::{self, InstalledModView, SearchResults};
use crate::storage;
use tauri::{AppHandle, Emitter, State};

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
pub async fn set_modrinth_mod_enabled(
    app: AppHandle,
    state: State<'_, RuntimeState>,
    instance_id: String,
    project_id: String,
    enabled: bool,
) -> Result<Vec<InstalledModView>, String> {
    ensure_stopped(&state, &instance_id)?;
    let result = run_blocking(move || {
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
    let _ = app.emit("status", "Installing mod and required dependencies...");
    let result = run_blocking(move || {
        let root = storage::user_data_dir()?;
        modrinth::install(&root, &instance_id, &project_id)
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
    let result = run_blocking(move || {
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
    let _ = app.emit("status", "Updating mod and dependencies...");
    let result = run_blocking(move || {
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
    let _ = app.emit("status", "Updating compatible mods...");
    let result = run_blocking(move || {
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
