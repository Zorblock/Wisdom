#![cfg_attr(windows, windows_subsystem = "windows")]

mod auth;
mod instances;
mod minecraft;
mod profile;
mod runtime;
mod storage;

use anyhow::Result;
use minecraft::ManifestVersion;
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use winreg::RegKey;
use winreg::enums::HKEY_CURRENT_USER;

slint::include_modules!();

// Public desktop-client identifier. Microsoft client IDs are intentionally not secrets.
const MICROSOFT_CLIENT_ID: &str = "6f216a95-c659-4c83-818b-a4d2c0a6e73f";
type Reporter = Arc<dyn Fn(String) + Send + Sync>;
type DownloadProgress = Arc<dyn Fn(f32, String) + Send + Sync>;

fn main() -> Result<()> {
    let data_dir = storage::user_data_dir()?;
    storage::prepare_storage(&data_dir)?;

    let window = AppWindow::new()?;
    window.set_accent_color(accent_color());
    window.set_status_text("Loading versions …".into());
    let settings = Arc::new(Mutex::new(storage::load_settings(&data_dir)));
    window.set_open_console(settings.lock().map(|settings| settings.open_console).unwrap_or(false));
    if let Ok(auth) = storage::load_auth() { update_account(&window, &data_dir, auth); }

    let versions = Arc::new(Mutex::new(Vec::<ManifestVersion>::new()));
    let selected = Arc::new(Mutex::new(String::new()));
    let instances = Arc::new(Mutex::new(Vec::<instances::Instance>::new()));
    let selected_instance = Arc::new(Mutex::new(String::new()));
    let active_login = Arc::new(Mutex::new(None::<Arc<AtomicBool>>));
    let reporter = status_reporter(window.as_weak());
    let progress = progress_reporter(window.as_weak());
    load_version_list(&window, &data_dir, &versions, &selected, &instances, &selected_instance);
    bind_version_selection(&window, &data_dir, &versions, &selected, &instances, &selected_instance);
    bind_instance_selection(&window, &versions, &selected, &instances, &selected_instance);
    bind_instance_creation(&window, &data_dir, &selected, &instances, &selected_instance);
    bind_login(&window, &data_dir, &active_login, &reporter);
    bind_cancel_login(&window, &active_login);
    bind_logout(&window);
    bind_console_setting(&window, &data_dir, &settings);
    bind_settings_dialog(&window, &data_dir, &versions, &selected, &instances, &selected_instance, &settings);
    bind_game_start(&window, &data_dir, &versions, &selected, &instances, &selected_instance, &settings, &reporter, &progress);

    window.run()?;
    Ok(())
}

fn load_version_list(window: &AppWindow, data_dir: &std::path::Path, versions: &Arc<Mutex<Vec<ManifestVersion>>>, selected: &Arc<Mutex<String>>, instances: &Arc<Mutex<Vec<instances::Instance>>>, selected_instance: &Arc<Mutex<String>>) {
    let weak = window.as_weak();
    let versions = Arc::clone(versions);
    let selected = Arc::clone(selected);
    let instances = Arc::clone(instances);
    let selected_instance = Arc::clone(selected_instance);
    let data_dir = data_dir.to_owned();
    thread::spawn(move || {
        let outcome = (|| -> Result<()> {
            let (manifest, list) = minecraft::load_versions()?;
            let default_id = manifest.latest.release;
            let mut instance_list = instances::load_or_create(&data_dir, &default_id)?;
            let active_instance = instance_list.first().cloned().expect("an instance was created");
            let selected_id = if list.iter().any(|version| version.id == active_instance.version) { active_instance.version.clone() } else { default_id.clone() };
            if selected_id != active_instance.version {
                instance_list[0].version = selected_id.clone();
                instances::save(&data_dir, &instance_list[0])?;
            }
            if let Ok(mut stored) = versions.lock() { *stored = list.clone(); }
            if let Ok(mut current) = selected.lock() { *current = selected_id.clone(); }
            if let Ok(mut stored) = instances.lock() { *stored = instance_list.clone(); }
            if let Ok(mut current) = selected_instance.lock() { *current = active_instance.id.clone(); }
            let rows: Vec<SharedString> = list.iter().map(|version| version.id.clone().into()).collect();
            let version_index = list.iter().position(|version| version.id == selected_id).unwrap_or(0) as i32;
            let instance_rows: Vec<SharedString> = instance_list.iter().map(instance_label).collect();
            let ui_weak = weak.clone();
            let _ = slint::invoke_from_event_loop(move || if let Some(ui) = ui_weak.upgrade() {
                ui.set_versions(ModelRc::new(VecModel::from(rows)));
                ui.set_selected_version(selected_id.into());
                ui.set_selected_version_index(version_index);
                ui.set_instances(ModelRc::new(VecModel::from(instance_rows)));
                ui.set_selected_instance_index(0);
                ui.set_status_text("Choose a version and start playing.".into());
            });
            Ok(())
        })();
        if let Err(error) = outcome {
            let message = format!("Could not load versions: {error:#}");
            let _ = slint::invoke_from_event_loop(move || if let Some(ui) = weak.upgrade() { ui.set_status_text(message.into()); });
        }
    });
}

fn bind_version_selection(window: &AppWindow, data_dir: &std::path::Path, versions: &Arc<Mutex<Vec<ManifestVersion>>>, selected: &Arc<Mutex<String>>, instances: &Arc<Mutex<Vec<instances::Instance>>>, selected_instance: &Arc<Mutex<String>>) {
    let versions = Arc::clone(versions);
    let selected = Arc::clone(selected);
    let instances = Arc::clone(instances);
    let selected_instance = Arc::clone(selected_instance);
    let data_dir = data_dir.to_owned();
    let weak = window.as_weak();
    window.on_select_version(move |index| {
        if let Some(version) = versions.lock().ok().and_then(|items| items.get(index.max(0) as usize).cloned()) {
            if let Ok(mut current) = selected.lock() { *current = version.id.clone(); }
            let active_id = selected_instance.lock().map(|value| value.clone()).unwrap_or_default();
            if let Ok(mut items) = instances.lock() {
                if let Some(instance) = items.iter_mut().find(|instance| instance.id == active_id) {
                    instance.version = version.id.clone();
                    let _ = instances::save(&data_dir, instance);
                }
            }
            if let Some(ui) = weak.upgrade() { ui.set_selected_version(version.id.into()); }
        }
    });
}

fn bind_instance_selection(window: &AppWindow, versions: &Arc<Mutex<Vec<ManifestVersion>>>, selected: &Arc<Mutex<String>>, instances: &Arc<Mutex<Vec<instances::Instance>>>, selected_instance: &Arc<Mutex<String>>) {
    let versions = Arc::clone(versions);
    let selected = Arc::clone(selected);
    let instances = Arc::clone(instances);
    let selected_instance = Arc::clone(selected_instance);
    let weak = window.as_weak();
    window.on_select_instance(move |index| {
        let instance = instances.lock().ok().and_then(|items| items.get(index.max(0) as usize).cloned());
        let Some(instance) = instance else { return; };
        if let Ok(mut active) = selected_instance.lock() { *active = instance.id.clone(); }
        if let Ok(mut current) = selected.lock() { *current = instance.version.clone(); }
        let version_index = versions.lock().ok().and_then(|items| items.iter().position(|version| version.id == instance.version)).unwrap_or(0) as i32;
        if let Some(ui) = weak.upgrade() {
            ui.set_selected_version(instance.version.into());
            ui.set_selected_version_index(version_index);
        }
    });
}

fn bind_instance_creation(window: &AppWindow, data_dir: &std::path::Path, selected: &Arc<Mutex<String>>, instances: &Arc<Mutex<Vec<instances::Instance>>>, selected_instance: &Arc<Mutex<String>>) {
    let data_dir = data_dir.to_owned();
    let selected = Arc::clone(selected);
    let instances = Arc::clone(instances);
    let selected_instance = Arc::clone(selected_instance);
    let weak = window.as_weak();
    window.on_create_instance(move || {
        let version = selected.lock().map(|value| value.clone()).unwrap_or_default();
        if version.is_empty() { return; }
        let Ok(mut items) = instances.lock() else { return; };
        let Ok(instance) = instances::create(&data_dir, &version, &items) else { return; };
        items.push(instance.clone());
        let index = items.len() as i32 - 1;
        let rows: Vec<SharedString> = items.iter().map(instance_label).collect();
        if let Ok(mut active) = selected_instance.lock() { *active = instance.id.clone(); }
        if let Some(ui) = weak.upgrade() {
            ui.set_instances(ModelRc::new(VecModel::from(rows)));
            ui.set_selected_instance_index(index);
            ui.set_status_text(format!("Created {} ({version}).", instance.name).into());
        }
    });
}

fn bind_login(window: &AppWindow, _data_dir: &std::path::Path, active_login: &Arc<Mutex<Option<Arc<AtomicBool>>>>, reporter: &Reporter) {
    let weak = window.as_weak();
    let active_login = Arc::clone(active_login);
    let reporter = Arc::clone(reporter);
    window.on_login(move || {
        if let Some(ui) = weak.upgrade() {
            ui.set_busy(true);
            ui.set_show_login_dialog(true);
        }
        let cancelled = Arc::new(AtomicBool::new(false));
        if let Ok(mut active) = active_login.lock() { *active = Some(Arc::clone(&cancelled)); }
        let weak_done = weak.clone();
        let report = Arc::clone(&reporter);
        let active_login = Arc::clone(&active_login);
        thread::spawn(move || {
            let outcome = auth::login(MICROSOFT_CLIENT_ID, &*report, &cancelled);
            if let Ok(mut active) = active_login.lock() { *active = None; }
            let _ = slint::invoke_from_event_loop(move || if let Some(ui) = weak_done.upgrade() {
                ui.set_busy(false);
                ui.set_show_login_dialog(false);
                match outcome {
                    Ok(auth) => { update_account(&ui, &storage::user_data_dir().unwrap_or_default(), auth); ui.set_status_text("Signed in. Ready to play.".into()); }
                    Err(_error) if cancelled.load(Ordering::Relaxed) => ui.set_status_text("Sign-in cancelled.".into()),
                    Err(error) => ui.set_status_text(format!("Sign-in failed: {error:#}").into()),
                }
            });
        });
    });
}

fn bind_cancel_login(window: &AppWindow, active_login: &Arc<Mutex<Option<Arc<AtomicBool>>>>) {
    let weak = window.as_weak();
    let active_login = Arc::clone(active_login);
    window.on_cancel_login(move || {
        if let Some(cancelled) = active_login.lock().ok().and_then(|active| active.clone()) { cancelled.store(true, Ordering::Relaxed); }
        if let Some(ui) = weak.upgrade() {
            ui.set_show_login_dialog(false);
            ui.set_status_text("Cancelling sign-in …".into());
        }
    });
}

fn bind_logout(window: &AppWindow) {
    let weak = window.as_weak();
    window.on_logout(move || {
        if storage::clear_auth().is_err() { return; }
        if let Some(ui) = weak.upgrade() {
            ui.set_account_name("".into());
            ui.set_player_head(slint::Image::default());
            ui.set_status_text("Signed out.".into());
        }
    });
}

fn bind_console_setting(window: &AppWindow, data_dir: &std::path::Path, settings: &Arc<Mutex<storage::LauncherSettings>>) {
    let data_dir = data_dir.to_owned();
    let settings = Arc::clone(settings);
    window.on_set_open_console(move |enabled| {
        if let Ok(mut current) = settings.lock() {
            current.open_console = enabled;
            let _ = storage::save_settings(&data_dir, &current);
        }
    });
}

fn bind_settings_dialog(window: &AppWindow, data_dir: &std::path::Path, versions: &Arc<Mutex<Vec<ManifestVersion>>>, selected: &Arc<Mutex<String>>, instances: &Arc<Mutex<Vec<instances::Instance>>>, selected_instance: &Arc<Mutex<String>>, settings: &Arc<Mutex<storage::LauncherSettings>>) {
    let data_dir = data_dir.to_owned();
    let versions_for_save = Arc::clone(versions);
    let selected_for_save = Arc::clone(selected);
    let instances_for_save = Arc::clone(instances);
    let selected_instance_for_save = Arc::clone(selected_instance);
    let settings_for_save = Arc::clone(settings);
    let settings_for_global = Arc::clone(settings);
    let settings_for_instance = Arc::clone(settings);
    let global_ui = window.as_weak();
    window.on_open_global_settings(move || {
        let current = settings_for_global.lock().map(|settings| settings.clone()).unwrap_or_default();
        if let Some(ui) = global_ui.upgrade() {
            ui.set_settings_global(true);
            ui.set_draft_ram(current.ram_mb.to_string().into());
            ui.set_draft_jvm_args(current.jvm_args.into());
            ui.set_draft_game_args(current.game_args.into());
            ui.set_show_settings(true);
        }
    });

    let instance_ui = window.as_weak();
    let instances_for_open = Arc::clone(instances);
    let selected_instance_for_open = Arc::clone(selected_instance);
    window.on_open_instance_settings(move || {
        let active = selected_instance_for_open.lock().map(|value| value.clone()).unwrap_or_default();
        let instance = instances_for_open.lock().ok().and_then(|items| items.iter().find(|instance| instance.id == active).cloned());
        let Some(instance) = instance else { return; };
        let defaults = settings_for_instance.lock().map(|settings| settings.clone()).unwrap_or_default();
        if let Some(ui) = instance_ui.upgrade() {
            ui.set_settings_global(false);
            ui.set_draft_name(instance.name.into());
            ui.set_draft_version(instance.version.into());
            ui.set_draft_ram(instance.ram_mb.unwrap_or(defaults.ram_mb).to_string().into());
            ui.set_draft_jvm_args(instance.jvm_args.clone().unwrap_or(defaults.jvm_args).into());
            ui.set_draft_game_args(instance.game_args.clone().unwrap_or(defaults.game_args).into());
            ui.set_draft_ram_override(instance.ram_mb.is_some());
            ui.set_draft_jvm_override(instance.jvm_args.is_some());
            ui.set_draft_game_override(instance.game_args.is_some());
            ui.set_show_settings(true);
        }
    });

    let close_ui = window.as_weak();
    window.on_close_settings(move || if let Some(ui) = close_ui.upgrade() { ui.set_show_settings(false); });

    let save_ui = window.as_weak();
    window.on_save_settings(move || {
        let Some(ui) = save_ui.upgrade() else { return; };
        let ram = ui.get_draft_ram().trim().parse::<u32>().unwrap_or(4096).clamp(512, 65_536);
        if ui.get_settings_global() {
            if let Ok(mut current) = settings_for_save.lock() {
                current.ram_mb = ram;
                current.jvm_args = ui.get_draft_jvm_args().to_string();
                current.game_args = ui.get_draft_game_args().to_string();
                if storage::save_settings(&data_dir, &current).is_ok() {
                    ui.set_show_settings(false);
                    ui.set_status_text("Global settings saved.".into());
                }
            }
            return;
        }

        let name = ui.get_draft_name().trim().to_string();
        let version = ui.get_draft_version().trim().to_string();
        if name.is_empty() || !versions_for_save.lock().map(|items| items.iter().any(|item| item.id == version)).unwrap_or(false) {
            ui.set_status_text("Enter a name and a valid Minecraft version.".into());
            return;
        }
        let active = selected_instance_for_save.lock().map(|value| value.clone()).unwrap_or_default();
        if let Ok(mut items) = instances_for_save.lock() {
            if let Some(instance) = items.iter_mut().find(|instance| instance.id == active) {
                instance.name = name;
                instance.version = version.clone();
                instance.ram_mb = ui.get_draft_ram_override().then_some(ram);
                instance.jvm_args = ui.get_draft_jvm_override().then(|| ui.get_draft_jvm_args().to_string());
                instance.game_args = ui.get_draft_game_override().then(|| ui.get_draft_game_args().to_string());
                if instances::save(&data_dir, instance).is_ok() {
                    let rows: Vec<SharedString> = items.iter().map(instance_label).collect();
                    let version_index = versions_for_save.lock().ok().and_then(|items| items.iter().position(|item| item.id == version)).unwrap_or(0) as i32;
                    if let Ok(mut current) = selected_for_save.lock() { *current = version.clone(); }
                    ui.set_instances(ModelRc::new(VecModel::from(rows)));
                    ui.set_selected_version(version.into());
                    ui.set_selected_version_index(version_index);
                    ui.set_show_settings(false);
                    ui.set_status_text("Instance settings saved.".into());
                }
            }
        }
    });
}

fn instance_label(instance: &instances::Instance) -> SharedString {
    format!("{} · {}", instance.name, instance.version).into()
}

fn bind_game_start(window: &AppWindow, data_dir: &std::path::Path, versions: &Arc<Mutex<Vec<ManifestVersion>>>, selected: &Arc<Mutex<String>>, instances: &Arc<Mutex<Vec<instances::Instance>>>, selected_instance: &Arc<Mutex<String>>, settings: &Arc<Mutex<storage::LauncherSettings>>, reporter: &Reporter, progress: &DownloadProgress) {
    let weak = window.as_weak();
    let data_dir = data_dir.to_owned();
    let versions = Arc::clone(versions);
    let selected = Arc::clone(selected);
    let instances = Arc::clone(instances);
    let selected_instance = Arc::clone(selected_instance);
    let settings = Arc::clone(settings);
    let reporter = Arc::clone(reporter);
    let progress = Arc::clone(progress);
    window.on_start_game(move || {
        let open_console = weak.upgrade().is_some_and(|ui| ui.get_open_console());
        let version_id = selected.lock().map(|value| value.clone()).unwrap_or_default();
        let version = versions.lock().ok().and_then(|items| items.iter().find(|item| item.id == version_id).cloned());
        let Some(version) = version else { return; };
        let active_id = selected_instance.lock().map(|value| value.clone()).unwrap_or_default();
        let instance = instances.lock().ok().and_then(|items| items.iter().find(|instance| instance.id == active_id).cloned());
        let Some(instance) = instance else { return; };
        let defaults = settings.lock().map(|settings| settings.clone()).unwrap_or_default();
        let options = minecraft::LaunchOptions {
            ram_mb: instance.ram_mb.unwrap_or(defaults.ram_mb).clamp(512, 65_536),
            jvm_args: instance.jvm_args.clone().unwrap_or(defaults.jvm_args),
            game_args: instance.game_args.clone().unwrap_or(defaults.game_args),
            open_console,
        };
        if let Some(ui) = weak.upgrade() {
            ui.set_busy(true);
            ui.set_show_progress(true);
            ui.set_progress_value(0.0);
        }
        let weak_done = weak.clone();
        let root = data_dir.clone();
        let game_dir = instances::game_dir(&root, &instance);
        let report = Arc::clone(&reporter);
        let progress = Arc::clone(&progress);
        thread::spawn(move || {
            let outcome = (|| -> Result<()> {
                let auth = auth::ensure_session(MICROSOFT_CLIENT_ID)?;
                minecraft::install_and_launch(&root, &game_dir, &version, &auth, &options, &*report, &*progress)
            })();
            let _ = slint::invoke_from_event_loop(move || if let Some(ui) = weak_done.upgrade() {
                ui.set_busy(false);
                ui.set_show_progress(false);
                match outcome { Ok(()) => ui.set_status_text("Minecraft started.".into()), Err(error) => ui.set_status_text(format!("Could not start: {error:#}").into()) }
            });
        });
    });
}

fn status_reporter(weak: slint::Weak<AppWindow>) -> Reporter {
    Arc::new(move |message| {
        let weak = weak.clone();
        let _ = slint::invoke_from_event_loop(move || if let Some(ui) = weak.upgrade() { ui.set_status_text(message.into()); });
    })
}

fn progress_reporter(weak: slint::Weak<AppWindow>) -> DownloadProgress {
    Arc::new(move |value, message| {
        let weak = weak.clone();
        let _ = slint::invoke_from_event_loop(move || if let Some(ui) = weak.upgrade() {
            ui.set_progress_value(value.clamp(0.0, 1.0));
            ui.set_status_text(message.into());
        });
    })
}

fn update_account(window: &AppWindow, data_dir: &std::path::Path, auth: storage::AuthState) {
    window.set_account_name(auth.player_name.clone().into());
    window.set_account_width((64 + auth.player_name.chars().count().min(24) * 8) as f32);
    let weak = window.as_weak();
    let root = data_dir.to_owned();
    thread::spawn(move || {
        let Ok(path) = profile::cached_skin(&root, &auth) else { return; };
        let _ = slint::invoke_from_event_loop(move || {
            if let (Some(ui), Ok(image)) = (weak.upgrade(), slint::Image::load_from_path(&path)) { ui.set_player_head(image); }
        });
    });
}

fn accent_color() -> slint::Color {
    let key = RegKey::predef(HKEY_CURRENT_USER).open_subkey("Software\\Microsoft\\Windows\\DWM");
    let color = key.ok().and_then(|key| key.get_value::<u32, _>("ColorizationColor").ok()).unwrap_or(0xFF6EA8FE);
    slint::Color::from_rgb_u8(((color >> 16) & 0xFF) as u8, ((color >> 8) & 0xFF) as u8, (color & 0xFF) as u8)
}
