mod auth;
mod instances;
mod minecraft;
mod runtime;
mod storage;

use serde::Serialize;
use std::sync::atomic::AtomicBool;
use tauri::{AppHandle, Emitter};

const MICROSOFT_CLIENT_ID: &str = "6f216a95-c659-4c83-818b-a4d2c0a6e73f";

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LauncherData {
    account: Option<Account>,
    versions: Vec<String>,
    latest_version: String,
    instances: Vec<instances::Instance>,
    settings: storage::LauncherSettings,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Account { name: String, uuid: String, skin_url: Option<String> }

#[tauri::command]
fn load_launcher() -> Result<LauncherData, String> {
    let root = storage::user_data_dir().map_err(|error| error.to_string())?;
    storage::prepare_storage(&root).map_err(|error| error.to_string())?;
    let (manifest, versions) = minecraft::load_versions().map_err(|error| error.to_string())?;
    let latest_version = manifest.latest.release;
    let instances = instances::load_or_create(&root, &latest_version).map_err(|error| error.to_string())?;
    let account = storage::load_auth().ok().map(|auth| Account { name: auth.player_name, uuid: auth.player_uuid, skin_url: auth.skin_url });
    Ok(LauncherData { account, versions: versions.into_iter().map(|version| version.id).collect(), latest_version, instances, settings: storage::load_settings(&root) })
}

#[tauri::command]
fn sign_in(app: AppHandle) -> Result<Account, String> {
    let cancelled = AtomicBool::new(false);
    let report = |message: String| { let _ = app.emit("status", message); };
    let auth = auth::login(MICROSOFT_CLIENT_ID, &report, &cancelled).map_err(|error| error.to_string())?;
    Ok(Account { name: auth.player_name, uuid: auth.player_uuid, skin_url: auth.skin_url })
}

#[tauri::command]
fn sign_out() -> Result<(), String> { storage::clear_auth().map_err(|error| error.to_string()) }

#[tauri::command]
fn create_instance(version: String) -> Result<instances::Instance, String> {
    let root = storage::user_data_dir().map_err(|error| error.to_string())?;
    let all = instances::load_or_create(&root, &version).map_err(|error| error.to_string())?;
    instances::create(&root, &version, &all).map_err(|error| error.to_string())
}

#[tauri::command]
fn launch(app: AppHandle, instance_id: String, version: String) -> Result<(), String> {
    let root = storage::user_data_dir().map_err(|error| error.to_string())?;
    let all = instances::load_or_create(&root, &version).map_err(|error| error.to_string())?;
    let instance = all.into_iter().find(|instance| instance.id == instance_id).ok_or("Instance not found")?;
    let (_, versions) = minecraft::load_versions().map_err(|error| error.to_string())?;
    let version = versions.into_iter().find(|item| item.id == version).ok_or("Minecraft version not found")?;
    let auth = auth::ensure_session(MICROSOFT_CLIENT_ID).map_err(|error| error.to_string())?;
    let settings = storage::load_settings(&root);
    let options = minecraft::LaunchOptions {
        ram_mb: instance.ram_mb.unwrap_or(settings.ram_mb).clamp(512, 65_536),
        jvm_args: instance.jvm_args.clone().unwrap_or(settings.jvm_args),
        game_args: instance.game_args.clone().unwrap_or(settings.game_args),
        open_console: settings.open_console,
    };
    let status_app = app.clone();
    let report = move |message: String| { let _ = status_app.emit("status", message); };
    let progress_app = app.clone();
    let progress = move |value: f32, message: String| { let _ = progress_app.emit("progress", value); let _ = progress_app.emit("status", message); };
    minecraft::install_and_launch(&root, &instances::game_dir(&root, &instance), &version, &auth, &options, &report, &progress)
        .map_err(|error| error.to_string())
}

pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![load_launcher, sign_in, sign_out, create_instance, launch])
        .run(tauri::generate_context!())
        .expect("Tauri application error");
}
