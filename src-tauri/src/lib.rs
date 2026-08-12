#![allow(linker_messages)]

mod auth;
mod content_commands;
mod downloads;
mod instance_logs;
mod instance_migration;
mod instance_setup;
mod instances;
mod minecraft;
mod minecraft_install;
mod modloaders;
mod modrinth;
mod runtime;
mod storage;

use serde::Serialize;
use std::collections::{HashMap, HashSet, VecDeque};
use std::io::{BufRead, BufReader, Read};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, Manager, State, WebviewUrl, WebviewWindowBuilder};

const MICROSOFT_CLIENT_ID: &str = "6f216a95-c659-4c83-818b-a4d2c0a6e73f";

#[derive(Default)]
pub(crate) struct RuntimeState {
    signing_in: AtomicBool,
    pub(crate) running_instances: Arc<Mutex<HashSet<String>>>,
    pub(crate) content_operations: Arc<Mutex<()>>,
    console_logs: Arc<Mutex<HashMap<String, VecDeque<ConsoleLine>>>>,
    main_window_hidden: Arc<AtomicBool>,
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

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct VersionCatalog {
    versions: Vec<VersionSummary>,
    latest_version: String,
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

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ConsoleLine {
    instance_id: String,
    sequence: u64,
    timestamp: String,
    stream: String,
    message: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ConsoleSession {
    instance_id: String,
    name: String,
    version: String,
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
async fn refresh_version_catalog() -> Result<VersionCatalog, String> {
    run_blocking(move || {
        let root = storage::user_data_dir()?;
        let (manifest, versions) = minecraft::refresh_versions(&root)?;
        Ok(VersionCatalog {
            versions: versions
                .into_iter()
                .map(|version| VersionSummary {
                    id: version.id,
                    kind: version.kind,
                    release_time: version.release_time,
                })
                .collect(),
            latest_version: manifest.latest.release,
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
async fn create_instance(
    app: AppHandle,
    name: String,
    version: String,
    loader: modloaders::ModLoader,
) -> Result<instances::Instance, String> {
    run_blocking(move || {
        let root = storage::user_data_dir()?;
        storage::prepare_storage(&root)?;
        let all = instances::load_all(&root)?;
        let instance = instances::create(&root, &name, &version, loader, &all)?;
        let progress_app = app.clone();
        let progress = move |value: f32, message: String| {
            let _ = progress_app.emit("progress", value.clamp(0.0, 1.0));
            let _ = progress_app.emit("status", message);
        };
        match instance_setup::prepare(&root, &instance, &progress) {
            Ok(instance) => Ok(instance),
            Err(error) => {
                let cleanup_error = instances::delete(&root, &instance.id).err();
                if let Some(cleanup_error) = cleanup_error {
                    anyhow::bail!(
                        "Could not prepare the instance: {error}. The incomplete instance could not be removed: {cleanup_error}"
                    );
                }
                Err(error.context("Could not prepare the instance"))
            }
        }
    })
    .await
}

#[tauri::command]
async fn update_instance(
    instance_id: String,
    name: String,
    version: String,
    loader: modloaders::ModLoader,
    ram_mb: Option<u32>,
    jvm_args: Option<String>,
    game_args: Option<String>,
) -> Result<instances::Instance, String> {
    run_blocking(move || {
        let root = storage::user_data_dir()?;
        let current = instances::load(&root, &instance_id)?;
        if (current.version != version || current.loader != loader)
            && modrinth::has_installed_mods(&root, &current)?
        {
            anyhow::bail!(
                "Remove installed mods before changing the Minecraft version or mod loader"
            );
        }
        instances::update(
            &root,
            &instance_id,
            &name,
            &version,
            loader,
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
    if settings.launch_behavior == storage::LaunchBehavior::Close {
        settings.open_console = false;
    }
    let root = storage::user_data_dir().map_err(|error| error.to_string())?;
    storage::prepare_storage(&root).map_err(|error| error.to_string())?;
    storage::save_settings(&root, &settings).map_err(|error| error.to_string())?;
    Ok(settings)
}

#[tauri::command]
fn apply_launch_behavior(
    app: AppHandle,
    state: State<'_, RuntimeState>,
    instance_id: String,
) -> Result<(), String> {
    let running_instances = state
        .running_instances
        .lock()
        .map_err(|_| "Could not read instance status".to_owned())?;
    if !running_instances.contains(&instance_id) {
        return Ok(());
    }

    let root = storage::user_data_dir().map_err(|error| error.to_string())?;
    let result = match storage::load_settings(&root).launch_behavior {
        storage::LaunchBehavior::KeepOpen => Ok(()),
        storage::LaunchBehavior::Hide => {
            let window = app
                .get_webview_window("main")
                .ok_or_else(|| "Main window is not available".to_owned())?;
            state.main_window_hidden.store(true, Ordering::Release);
            if let Err(error) = window.hide() {
                state.main_window_hidden.store(false, Ordering::Release);
                return Err(format!("Could not hide the launcher: {error}"));
            }
            Ok(())
        }
        storage::LaunchBehavior::Close => {
            app.exit(0);
            Ok(())
        }
    };
    drop(running_instances);
    result
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
fn get_console_history(
    state: State<'_, RuntimeState>,
    instance_id: String,
) -> Result<Vec<ConsoleLine>, String> {
    let logs = state
        .console_logs
        .lock()
        .map_err(|_| "Could not read console history".to_owned())?;
    Ok(logs
        .get(&instance_id)
        .map(|lines| lines.iter().cloned().collect())
        .unwrap_or_default())
}

#[tauri::command]
async fn open_instance_console(app: AppHandle, instance_id: String) -> Result<(), String> {
    let lookup_id = instance_id.clone();
    let instance = run_blocking(move || {
        let root = storage::user_data_dir()?;
        instances::load(&root, &lookup_id)
    })
    .await?;
    open_console_window(&app, &instance, false).map(|_| ())
}

#[tauri::command]
async fn open_instance_logs(app: AppHandle, instance_id: String) -> Result<(), String> {
    let lookup_id = instance_id.clone();
    let instance = run_blocking(move || {
        let root = storage::user_data_dir()?;
        instances::load(&root, &lookup_id)
    })
    .await?;
    instance_logs::open_window(&app, &instance)
}

#[tauri::command]
async fn list_instance_logs(
    instance_id: String,
) -> Result<Vec<instance_logs::InstanceLogFile>, String> {
    run_blocking(move || {
        let root = storage::user_data_dir()?;
        instance_logs::list(&root, &instance_id)
    })
    .await
}

#[tauri::command]
async fn read_instance_log(
    instance_id: String,
    file_name: String,
) -> Result<Vec<instance_logs::InstanceLogLine>, String> {
    run_blocking(move || {
        let root = storage::user_data_dir()?;
        instance_logs::read(&root, &instance_id, &file_name)
    })
    .await
}

fn console_window_label(instance_id: &str) -> String {
    format!("console-{instance_id}")
}

fn open_console_window(
    app: &AppHandle,
    instance: &instances::Instance,
    reset_existing: bool,
) -> Result<String, String> {
    let label = console_window_label(&instance.id);
    if let Some(window) = app.get_webview_window(&label) {
        let _ = window.set_title(&format!("Wisdom Console — {}", instance.name));
        let _ = window.show();
        let _ = window.set_focus();
        if reset_existing {
            let _ = app.emit_to(
                &label,
                "minecraft-console-reset",
                ConsoleSession {
                    instance_id: instance.id.clone(),
                    name: instance.name.clone(),
                    version: instance.version.clone(),
                },
            );
        }
        return Ok(label);
    }

    let url = format!(
        "index.html?console=1&instanceId={}&name={}&version={}",
        urlencoding::encode(&instance.id),
        urlencoding::encode(&instance.name),
        urlencoding::encode(&instance.version)
    );
    WebviewWindowBuilder::new(app, &label, WebviewUrl::App(url.into()))
        .title(format!("Wisdom Console — {}", instance.name))
        .inner_size(860.0, 540.0)
        .min_inner_size(620.0, 360.0)
        .resizable(true)
        .center()
        .build()
        .map_err(|error| format!("Could not open the game console: {error}"))?;
    Ok(label)
}

fn store_and_emit_console_line(
    app: &AppHandle,
    label: &str,
    logs: &Arc<Mutex<HashMap<String, VecDeque<ConsoleLine>>>>,
    sequence: &AtomicU64,
    instance_id: &str,
    stream: &str,
    message: String,
) {
    let message = message.trim_end_matches(['\r', '\n']).to_owned();
    if message.is_empty() {
        return;
    }
    let line = ConsoleLine {
        instance_id: instance_id.to_owned(),
        sequence: sequence.fetch_add(1, Ordering::Relaxed),
        timestamp: chrono::Local::now().format("%H:%M:%S").to_string(),
        stream: stream.to_owned(),
        message,
    };
    if let Ok(mut all_logs) = logs.lock() {
        let history = all_logs.entry(instance_id.to_owned()).or_default();
        history.push_back(line.clone());
        while history.len() > 5_000 {
            history.pop_front();
        }
    }
    let _ = app.emit_to(label, "minecraft-console-line", line);
}

fn log4j_attribute<'a>(event: &'a str, name: &str) -> Option<&'a str> {
    let marker = format!(r#"{name}=""#);
    let value = event.split_once(&marker)?.1;
    value.split_once('"').map(|(value, _)| value)
}

fn log4j_cdata(event: &str, element: &str) -> Option<String> {
    let marker = format!("<log4j:{element}");
    let value = event.split_once(&marker)?.1;
    let value = value.split_once("<![CDATA[")?.1;
    value
        .split_once("]]>")
        .map(|(value, _)| value.trim().to_owned())
}

fn format_log4j_event(event: &str) -> Option<String> {
    if !event.contains("<log4j:Event") {
        return None;
    }
    let level = log4j_attribute(event, "level").unwrap_or("INFO");
    let thread = log4j_attribute(event, "thread").unwrap_or("Minecraft");
    let message = log4j_cdata(event, "Message").unwrap_or_default();
    let throwable = log4j_cdata(event, "Throwable").unwrap_or_default();
    let mut formatted = format!("[{thread}/{level}] {message}");
    if !throwable.is_empty() {
        formatted.push('\n');
        formatted.push_str(&throwable);
    }
    Some(formatted.trim_end().to_owned())
}

fn is_log4j_markup(value: &str) -> bool {
    let value = value.trim();
    value.starts_with("<?xml")
        || value.starts_with("<log4j:Events")
        || value.starts_with("</log4j:Events")
        || value.starts_with("<log4j:Event")
        || value.starts_with("</log4j:Event")
        || value.starts_with("<log4j:Message")
        || value.starts_with("</log4j:Message")
        || value.starts_with("<log4j:Throwable")
        || value.starts_with("</log4j:Throwable")
}

fn spawn_console_reader<R: Read + Send + 'static>(
    reader: R,
    app: AppHandle,
    label: String,
    logs: Arc<Mutex<HashMap<String, VecDeque<ConsoleLine>>>>,
    sequence: Arc<AtomicU64>,
    instance_id: String,
    stream: &'static str,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let mut reader = BufReader::new(reader);
        let mut bytes = Vec::new();
        let mut log4j_event = String::new();
        loop {
            bytes.clear();
            match reader.read_until(b'\n', &mut bytes) {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    let line = String::from_utf8_lossy(&bytes);
                    if log4j_event.is_empty() {
                        if let Some(start) = line.find("<log4j:Event") {
                            log4j_event.push_str(&line[start..]);
                        } else if !is_log4j_markup(&line) {
                            store_and_emit_console_line(
                                &app,
                                &label,
                                &logs,
                                &sequence,
                                &instance_id,
                                stream,
                                line.into_owned(),
                            );
                        }
                    } else {
                        log4j_event.push_str(&line);
                    }

                    if log4j_event.contains("</log4j:Event>") {
                        if let Some(message) = format_log4j_event(&log4j_event) {
                            store_and_emit_console_line(
                                &app,
                                &label,
                                &logs,
                                &sequence,
                                &instance_id,
                                stream,
                                message,
                            );
                        }
                        log4j_event.clear();
                    } else if log4j_event.len() > 1_048_576 {
                        log4j_event.clear();
                    }
                }
            }
        }
        if !log4j_event.is_empty() {
            if let Some(message) = format_log4j_event(&log4j_event) {
                store_and_emit_console_line(
                    &app,
                    &label,
                    &logs,
                    &sequence,
                    &instance_id,
                    stream,
                    message,
                );
            }
        }
    })
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
        if instance.loader.supports_mods() && version != instance.version {
            anyhow::bail!("Edit the instance to change the Minecraft version of a modded profile");
        }
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
        let game_dir = instances::game_dir(&root, &instance);
        let (child, installed_loader_version) = if instance.loader.supports_mods() {
            let launched = modloaders::install_and_launch(
                &root, &game_dir, &instance, &auth, &options, &report, &progress,
            )?;
            (launched.child, Some(launched.loader_version))
        } else {
            (
                minecraft::install_and_launch(
                    &root,
                    &game_dir,
                    &version_entry,
                    &auth,
                    &options,
                    &report,
                    &progress,
                )?,
                None,
            )
        };
        let saved = if let Some(loader_version) = installed_loader_version {
            instances::mark_modded_launched(&root, &operation_instance_id, &version, loader_version)
        } else {
            instances::mark_launched(&root, &operation_instance_id, &version)
        };
        let updated = match saved {
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
            let console_logs = Arc::clone(&state.console_logs);
            let console_label = if child.stdout.is_some() || child.stderr.is_some() {
                if let Ok(mut logs) = console_logs.lock() {
                    logs.insert(instance_id.clone(), VecDeque::new());
                }
                match open_console_window(&app, &instance, true) {
                    Ok(label) => Some(label),
                    Err(error) => {
                        let _ = app.emit("status", error);
                        Some(console_window_label(&instance_id))
                    }
                }
            } else {
                None
            };
            let sequence = Arc::new(AtomicU64::new(1));
            let mut console_readers = Vec::new();
            if let (Some(label), Some(stdout)) = (console_label.as_ref(), child.stdout.take()) {
                console_readers.push(spawn_console_reader(
                    stdout,
                    app.clone(),
                    label.clone(),
                    Arc::clone(&console_logs),
                    Arc::clone(&sequence),
                    instance_id.clone(),
                    "stdout",
                ));
            }
            if let (Some(label), Some(stderr)) = (console_label.as_ref(), child.stderr.take()) {
                console_readers.push(spawn_console_reader(
                    stderr,
                    app.clone(),
                    label.clone(),
                    Arc::clone(&console_logs),
                    Arc::clone(&sequence),
                    instance_id.clone(),
                    "stderr",
                ));
            }

            let running = Arc::clone(&state.running_instances);
            let main_window_hidden = Arc::clone(&state.main_window_hidden);
            let monitor_app = app.clone();
            tauri::async_runtime::spawn_blocking(move || {
                let exit = child.wait();
                for reader in console_readers {
                    let _ = reader.join();
                }
                if let Some(label) = console_label {
                    let message = match exit {
                        Ok(status) => format!("Process exited with {status}."),
                        Err(error) => format!("Could not read the process exit status: {error}"),
                    };
                    store_and_emit_console_line(
                        &monitor_app,
                        &label,
                        &console_logs,
                        &sequence,
                        &instance_id,
                        "system",
                        message,
                    );
                }
                let no_games_running = if let Ok(mut instances) = running.lock() {
                    instances.remove(&instance_id);
                    instances.is_empty()
                } else {
                    false
                };
                if no_games_running && main_window_hidden.swap(false, Ordering::AcqRel) {
                    if let Some(window) = monitor_app.get_webview_window("main") {
                        let _ = window.show();
                        let _ = window.set_focus();
                    }
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
            refresh_version_catalog,
            sign_in,
            select_account,
            remove_account,
            sign_out,
            create_instance,
            update_instance,
            delete_instance,
            save_launcher_settings,
            apply_launch_behavior,
            open_instance_folder,
            open_data_folder,
            get_system_accent,
            get_console_history,
            open_instance_console,
            open_instance_logs,
            list_instance_logs,
            read_instance_log,
            content_commands::list_instance_mods,
            content_commands::search_modrinth,
            content_commands::install_modrinth_mod,
            content_commands::remove_modrinth_mod,
            content_commands::set_modrinth_mod_enabled,
            content_commands::update_modrinth_mod,
            content_commands::update_all_modrinth_mods,
            instance_migration::preview_instance_migration,
            instance_migration::migrate_instance,
            launch,
        ])
        .setup(|app| {
            #[cfg(debug_assertions)]
            {
                let handle = app.handle().clone();
                ctrlc::set_handler(move || handle.exit(0))?;
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("Tauri application error");
}
