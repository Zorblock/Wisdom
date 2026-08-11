mod auth;
mod instances;
mod minecraft;
mod runtime;
mod storage;

use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{AppHandle, Emitter, State};

const MICROSOFT_CLIENT_ID: &str = "6f216a95-c659-4c83-818b-a4d2c0a6e73f";

#[derive(Default)]
struct RuntimeState {
    signing_in: AtomicBool,
    launching: AtomicBool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LauncherData {
    account: Option<Account>,
    versions: Vec<VersionSummary>,
    latest_version: String,
    instances: Vec<instances::Instance>,
    settings: storage::LauncherSettings,
    data_directory: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct VersionSummary {
    id: String,
    kind: String,
    release_time: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Account {
    name: String,
    uuid: String,
    skin_url: Option<String>,
}

impl From<storage::AuthState> for Account {
    fn from(auth: storage::AuthState) -> Self {
        Self {
            name: auth.player_name,
            uuid: auth.player_uuid,
            skin_url: auth.skin_url,
        }
    }
}

#[tauri::command]
async fn load_launcher() -> Result<LauncherData, String> {
    run_blocking(|| {
        let root = storage::user_data_dir()?;
        storage::prepare_storage(&root)?;
        let (manifest, versions) = minecraft::load_versions(&root)?;
        let latest_version = manifest.latest.release;
        let instances = instances::load_or_create(&root, &latest_version)?;
        let account = storage::load_auth().ok().map(Account::from);
        let versions = versions
            .into_iter()
            .map(|version| VersionSummary {
                id: version.id,
                kind: version.kind,
                release_time: version.release_time,
            })
            .collect();
        Ok(LauncherData {
            account,
            versions,
            latest_version,
            instances,
            settings: storage::load_settings(&root),
            data_directory: root.to_string_lossy().to_string(),
        })
    })
    .await
}

#[tauri::command]
async fn sign_in(app: AppHandle, state: State<'_, RuntimeState>) -> Result<Account, String> {
    acquire(&state.signing_in, "Eine Anmeldung läuft bereits")?;
    let result = run_blocking(move || {
        let cancelled = AtomicBool::new(false);
        let report = |message: String| {
            let _ = app.emit("status", message);
        };
        auth::login(MICROSOFT_CLIENT_ID, &report, &cancelled).map(Account::from)
    })
    .await;
    state.signing_in.store(false, Ordering::Release);
    result
}

#[tauri::command]
fn sign_out() -> Result<(), String> {
    storage::clear_auth().map_err(|error| error.to_string())
}

#[tauri::command]
async fn create_instance(name: String, version: String) -> Result<instances::Instance, String> {
    run_blocking(move || {
        let root = storage::user_data_dir()?;
        storage::prepare_storage(&root)?;
        let all = instances::load_or_create(&root, &version)?;
        instances::create(&root, &name, &version, &all)
    })
    .await
}

#[tauri::command]
async fn update_instance(
    instance_id: String,
    name: String,
    version: String,
    ram_mb: Option<u32>,
    jvm_args: Option<String>,
    game_args: Option<String>,
) -> Result<instances::Instance, String> {
    run_blocking(move || {
        let root = storage::user_data_dir()?;
        instances::update(
            &root,
            &instance_id,
            &name,
            &version,
            ram_mb,
            jvm_args,
            game_args,
        )
    })
    .await
}

#[tauri::command]
async fn delete_instance(instance_id: String) -> Result<(), String> {
    run_blocking(move || {
        let root = storage::user_data_dir()?;
        let all = instances::load_or_create(&root, "unknown")?;
        if all.len() <= 1 {
            anyhow::bail!("Die letzte Instanz kann nicht gelöscht werden");
        }
        instances::delete(&root, &instance_id)
    })
    .await
}

#[tauri::command]
fn save_launcher_settings(
    mut settings: storage::LauncherSettings,
) -> Result<storage::LauncherSettings, String> {
    settings.ram_mb = settings.ram_mb.clamp(512, 65_536);
    let root = storage::user_data_dir().map_err(|error| error.to_string())?;
    storage::prepare_storage(&root).map_err(|error| error.to_string())?;
    storage::save_settings(&root, &settings).map_err(|error| error.to_string())?;
    Ok(settings)
}

#[tauri::command]
fn open_instance_folder(instance_id: String) -> Result<(), String> {
    let root = storage::user_data_dir().map_err(|error| error.to_string())?;
    let instance = instances::load(&root, &instance_id).map_err(|error| error.to_string())?;
    open::that(instances::game_dir(&root, &instance)).map_err(|error| error.to_string())
}

#[tauri::command]
fn open_data_folder() -> Result<(), String> {
    let root = storage::user_data_dir().map_err(|error| error.to_string())?;
    storage::prepare_storage(&root).map_err(|error| error.to_string())?;
    open::that(root).map_err(|error| error.to_string())
}

#[tauri::command]
async fn launch(
    app: AppHandle,
    state: State<'_, RuntimeState>,
    instance_id: String,
    version: String,
) -> Result<instances::Instance, String> {
    acquire(&state.launching, "Minecraft wird bereits vorbereitet")?;
    let result = run_blocking(move || {
        let root = storage::user_data_dir()?;
        storage::prepare_storage(&root)?;
        let instance = instances::load(&root, &instance_id)?;
        let (_, versions) = minecraft::load_versions(&root)?;
        let version_entry = versions
            .into_iter()
            .find(|item| item.id == version)
            .ok_or_else(|| anyhow::anyhow!("Minecraft-Version {version} wurde nicht gefunden"))?;
        let auth = auth::ensure_session(MICROSOFT_CLIENT_ID)?;
        let settings = storage::load_settings(&root);
        let options = minecraft::LaunchOptions {
            ram_mb: instance
                .ram_mb
                .unwrap_or(settings.ram_mb)
                .clamp(512, 65_536),
            jvm_args: instance.jvm_args.clone().unwrap_or(settings.jvm_args),
            game_args: instance.game_args.clone().unwrap_or(settings.game_args),
            open_console: settings.open_console,
            client_id: MICROSOFT_CLIENT_ID.to_owned(),
        };
        let status_app = app.clone();
        let report = move |message: String| {
            let _ = status_app.emit("status", message);
        };
        let progress_app = app.clone();
        let progress = move |value: f32, message: String| {
            let _ = progress_app.emit("progress", value.clamp(0.0, 1.0));
            let _ = progress_app.emit("status", message);
        };
        minecraft::install_and_launch(
            &root,
            &instances::game_dir(&root, &instance),
            &version_entry,
            &auth,
            &options,
            &report,
            &progress,
        )?;
        instances::mark_launched(&root, &instance_id, &version)
    })
    .await;
    state.launching.store(false, Ordering::Release);
    result
}

fn acquire(flag: &AtomicBool, message: &str) -> Result<(), String> {
    flag.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .map(|_| ())
        .map_err(|_| message.to_owned())
}

async fn run_blocking<T, F>(operation: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> anyhow::Result<T> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(operation)
        .await
        .map_err(|error| format!("Interner Fehler: {error}"))?
        .map_err(|error| format!("{error:#}"))
}

pub fn run() {
    tauri::Builder::default()
        .manage(RuntimeState::default())
        .invoke_handler(tauri::generate_handler![
            load_launcher,
            sign_in,
            sign_out,
            create_instance,
            update_instance,
            delete_instance,
            save_launcher_settings,
            open_instance_folder,
            open_data_folder,
            launch,
        ])
        .run(tauri::generate_context!())
        .expect("Tauri application error");
}
