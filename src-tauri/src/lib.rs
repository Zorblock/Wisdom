#![allow(linker_messages)]

mod auth;
mod instances;
mod minecraft;
mod runtime;
mod storage;

use serde::Serialize;
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, State};

const MICROSOFT_CLIENT_ID: &str = "6f216a95-c659-4c83-818b-a4d2c0a6e73f";

#[derive(Default)]
struct RuntimeState {
    signing_in: AtomicBool,
    running_instances: Arc<Mutex<HashSet<String>>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LauncherData {
    account: Option<Account>,
    accounts: Vec<Account>,
    versions: Vec<VersionSummary>,
    latest_version: String,
    instances: Vec<instances::Instance>,
    running_instances: Vec<String>,
    settings: storage::LauncherSettings,
    data_directory: String,
    accent_color: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct VersionSummary {
    id: String,
    kind: String,
    release_time: Option<String>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Account {
    name: String,
    uuid: String,
    skin_url: Option<String>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct InstanceStatus {
    instance_id: String,
    running: bool,
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

impl From<storage::AccountProfile> for Account {
    fn from(profile: storage::AccountProfile) -> Self {
        Self {
            name: profile.name,
            uuid: profile.uuid,
            skin_url: profile.skin_url,
        }
    }
}

#[tauri::command]
async fn load_launcher(state: State<'_, RuntimeState>) -> Result<LauncherData, String> {
    let running_instances = state
        .running_instances
        .lock()
        .map_err(|_| "Could not read instance status".to_owned())?
        .iter()
        .cloned()
        .collect();
    run_blocking(move || {
        let root = storage::user_data_dir()?;
        storage::prepare_storage(&root)?;
        let (manifest, versions) = minecraft::load_versions(&root)?;
        let latest_version = manifest.latest.release;
        let instances = instances::load_all(&root)?;
        let (active_account, accounts) = storage::load_accounts(&root)?;
        let versions = versions
            .into_iter()
            .map(|version| VersionSummary {
                id: version.id,
                kind: version.kind,
                release_time: version.release_time,
            })
            .collect();
        Ok(LauncherData {
            account: active_account.map(Account::from),
            accounts: accounts.into_iter().map(Account::from).collect(),
            versions,
            latest_version,
            instances,
            running_instances,
            settings: storage::load_settings(&root),
            data_directory: root.to_string_lossy().to_string(),
            accent_color: storage::windows_accent_color(),
        })
    })
    .await
}

#[tauri::command]
async fn sign_in(app: AppHandle, state: State<'_, RuntimeState>) -> Result<Account, String> {
    acquire(&state.signing_in, "A sign-in is already in progress")?;
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
async fn select_account(player_uuid: String) -> Result<Account, String> {
    run_blocking(move || {
        let root = storage::user_data_dir()?;
        storage::select_account(&root, &player_uuid).map(Account::from)
    })
    .await
}

#[tauri::command]
async fn remove_account(player_uuid: String) -> Result<Option<Account>, String> {
    run_blocking(move || {
        let root = storage::user_data_dir()?;
        storage::remove_account(&root, &player_uuid).map(|account| account.map(Account::from))
    })
    .await
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
        let all = instances::load_all(&root)?;
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
async fn delete_instance(
    state: State<'_, RuntimeState>,
    instance_id: String,
) -> Result<(), String> {
    if state
        .running_instances
        .lock()
        .map_err(|_| "Could not read instance status".to_owned())?
        .contains(&instance_id)
    {
        return Err("A running instance cannot be deleted".to_owned());
    }
    run_blocking(move || {
        let root = storage::user_data_dir()?;
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
fn get_system_accent() -> String {
    storage::windows_accent_color()
}

#[tauri::command]
async fn launch(
    app: AppHandle,
    state: State<'_, RuntimeState>,
    instance_id: String,
    version: String,
) -> Result<instances::Instance, String> {
    {
        let mut running = state
            .running_instances
            .lock()
            .map_err(|_| "Could not update instance status".to_owned())?;
        if !running.insert(instance_id.clone()) {
            return Err("This instance is already running".to_owned());
        }
    }

    let operation_instance_id = instance_id.clone();
    let operation_app = app.clone();
    let result = run_blocking(move || {
        let root = storage::user_data_dir()?;
        storage::prepare_storage(&root)?;
        let instance = instances::load(&root, &operation_instance_id)?;
        let (_, versions) = minecraft::load_versions(&root)?;
        let version_entry = versions
            .into_iter()
            .find(|item| item.id == version)
            .ok_or_else(|| anyhow::anyhow!("Minecraft version {version} was not found"))?;
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
        let status_app = operation_app.clone();
        let report = move |message: String| {
            let _ = status_app.emit("status", message);
        };
        let progress_app = operation_app.clone();
        let progress = move |value: f32, message: String| {
            let _ = progress_app.emit("progress", value.clamp(0.0, 1.0));
            let _ = progress_app.emit("status", message);
        };
        let child = minecraft::install_and_launch(
            &root,
            &instances::game_dir(&root, &instance),
            &version_entry,
            &auth,
            &options,
            &report,
            &progress,
        )?;
        let updated = match instances::mark_launched(&root, &operation_instance_id, &version) {
            Ok(instance) => instance,
            Err(error) => {
                report(format!(
                    "The game is running, but its launch time could not be saved: {error}"
                ));
                instance
            }
        };
        Ok((updated, child))
    })
    .await;

    match result {
        Ok((instance, mut child)) => {
            let _ = app.emit(
                "instance-status",
                InstanceStatus {
                    instance_id: instance_id.clone(),
                    running: true,
                },
            );
            let running = Arc::clone(&state.running_instances);
            let monitor_app = app.clone();
            tauri::async_runtime::spawn_blocking(move || {
                let _ = child.wait();
                if let Ok(mut instances) = running.lock() {
                    instances.remove(&instance_id);
                }
                let _ = monitor_app.emit(
                    "instance-status",
                    InstanceStatus {
                        instance_id,
                        running: false,
                    },
                );
            });
            Ok(instance)
        }
        Err(error) => {
            if let Ok(mut instances) = state.running_instances.lock() {
                instances.remove(&instance_id);
            }
            let _ = app.emit(
                "instance-status",
                InstanceStatus {
                    instance_id,
                    running: false,
                },
            );
            Err(error)
        }
    }
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
        .map_err(|error| format!("Internal error: {error}"))?
        .map_err(|error| format!("{error:#}"))
}

pub fn run() {
    tauri::Builder::default()
        .manage(RuntimeState::default())
        .invoke_handler(tauri::generate_handler![
            load_launcher,
            sign_in,
            select_account,
            remove_account,
            sign_out,
            create_instance,
            update_instance,
            delete_instance,
            save_launcher_settings,
            open_instance_folder,
            open_data_folder,
            get_system_accent,
            launch,
        ])
        .run(tauri::generate_context!())
        .expect("Tauri application error");
}
