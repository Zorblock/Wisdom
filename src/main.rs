mod auth;
mod minecraft;
mod profile;
mod storage;

use anyhow::Result;
use copypasta::{ClipboardContext, ClipboardProvider};
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
type CodePresenter = Arc<dyn Fn(String) + Send + Sync>;

fn main() -> Result<()> {
    let data_dir = storage::user_data_dir()?;
    storage::prepare_storage(&data_dir)?;

    let window = AppWindow::new()?;
    window.set_accent_color(accent_color());
    window.set_status_text("Loading versions …".into());
    if let Ok(auth) = storage::load_auth() { update_account(&window, &data_dir, auth); }

    let versions = Arc::new(Mutex::new(Vec::<ManifestVersion>::new()));
    let selected = Arc::new(Mutex::new(String::new()));
    let active_login = Arc::new(Mutex::new(None::<Arc<AtomicBool>>));
    let reporter = status_reporter(window.as_weak());
    load_version_list(&window, &versions, &selected);
    bind_version_selection(&window, &versions, &selected);
    bind_login(&window, &data_dir, &active_login, &reporter);
    bind_copy_and_cancel(&window, &active_login);
    bind_game_start(&window, &data_dir, &versions, &selected, &reporter);

    window.run()?;
    Ok(())
}

fn load_version_list(window: &AppWindow, versions: &Arc<Mutex<Vec<ManifestVersion>>>, selected: &Arc<Mutex<String>>) {
    let weak = window.as_weak();
    let versions = Arc::clone(versions);
    let selected = Arc::clone(selected);
    thread::spawn(move || match minecraft::load_versions() {
        Ok((manifest, list)) => {
            let default_id = manifest.latest.release;
            if let Ok(mut stored) = versions.lock() { *stored = list.clone(); }
            if let Ok(mut current) = selected.lock() { *current = default_id.clone(); }
            let rows: Vec<SharedString> = list.iter().map(|version| version.id.clone().into()).collect();
            let release_index = list.iter().position(|version| version.id == default_id).unwrap_or(0) as i32;
            let _ = slint::invoke_from_event_loop(move || if let Some(ui) = weak.upgrade() {
                ui.set_versions(ModelRc::new(VecModel::from(rows)));
                ui.set_selected_version(default_id.into());
                ui.set_selected_version_index(release_index);
                ui.set_status_text("Choose a version and start playing.".into());
            });
        }
        Err(error) => {
            let message = format!("Could not load versions: {error:#}");
            let _ = slint::invoke_from_event_loop(move || if let Some(ui) = weak.upgrade() { ui.set_status_text(message.into()); });
        }
    });
}

fn bind_version_selection(window: &AppWindow, versions: &Arc<Mutex<Vec<ManifestVersion>>>, selected: &Arc<Mutex<String>>) {
    let versions = Arc::clone(versions);
    let selected = Arc::clone(selected);
    let weak = window.as_weak();
    window.on_select_version(move |index| {
        if let Some(version) = versions.lock().ok().and_then(|items| items.get(index.max(0) as usize).cloned()) {
            if let Ok(mut current) = selected.lock() { *current = version.id.clone(); }
            if let Some(ui) = weak.upgrade() { ui.set_selected_version(version.id.into()); }
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
            ui.set_copy_button_text("Copy code".into());
        }
        let cancelled = Arc::new(AtomicBool::new(false));
        if let Ok(mut active) = active_login.lock() { *active = Some(Arc::clone(&cancelled)); }
        let weak_done = weak.clone();
        let weak_code = weak.clone();
        let report = Arc::clone(&reporter);
        let active_login = Arc::clone(&active_login);
        let present_code: CodePresenter = Arc::new(move |code| {
            let weak_code = weak_code.clone();
            let _ = slint::invoke_from_event_loop(move || if let Some(ui) = weak_code.upgrade() {
                ui.set_login_code(code.into());
                ui.set_show_login_dialog(true);
            });
        });
        thread::spawn(move || {
            let outcome = auth::login(MICROSOFT_CLIENT_ID, &*report, &*present_code, &cancelled);
            if let Ok(mut active) = active_login.lock() { *active = None; }
            let _ = slint::invoke_from_event_loop(move || if let Some(ui) = weak_done.upgrade() {
                ui.set_busy(false);
                ui.set_show_login_dialog(false);
                ui.set_login_code("".into());
                match outcome {
                    Ok(auth) => { update_account(&ui, &storage::user_data_dir().unwrap_or_default(), auth); ui.set_status_text("Signed in. Ready to play.".into()); }
                    Err(_error) if cancelled.load(Ordering::Relaxed) => ui.set_status_text("Sign-in cancelled.".into()),
                    Err(error) => ui.set_status_text(format!("Sign-in failed: {error:#}").into()),
                }
            });
        });
    });
}

fn bind_copy_and_cancel(window: &AppWindow, active_login: &Arc<Mutex<Option<Arc<AtomicBool>>>>) {
    let weak = window.as_weak();
    window.on_copy_login_code(move || {
        let Some(ui) = weak.upgrade() else { return; };
        let code = ui.get_login_code().to_string();
        if !code.is_empty() && ClipboardContext::new().and_then(|mut clipboard| clipboard.set_contents(code)).is_ok() {
            ui.set_copy_button_text("Copied ✓".into());
        }
    });

    let weak = window.as_weak();
    let active_login = Arc::clone(active_login);
    window.on_cancel_login(move || {
        if let Some(cancelled) = active_login.lock().ok().and_then(|active| active.clone()) { cancelled.store(true, Ordering::Relaxed); }
        if let Some(ui) = weak.upgrade() {
            ui.set_show_login_dialog(false);
            ui.set_login_code("".into());
            ui.set_status_text("Cancelling sign-in …".into());
        }
    });
}

fn bind_game_start(window: &AppWindow, data_dir: &std::path::Path, versions: &Arc<Mutex<Vec<ManifestVersion>>>, selected: &Arc<Mutex<String>>, reporter: &Reporter) {
    let weak = window.as_weak();
    let data_dir = data_dir.to_owned();
    let versions = Arc::clone(versions);
    let selected = Arc::clone(selected);
    let reporter = Arc::clone(reporter);
    window.on_start_game(move || {
        let version_id = selected.lock().map(|value| value.clone()).unwrap_or_default();
        let version = versions.lock().ok().and_then(|items| items.iter().find(|item| item.id == version_id).cloned());
        let Some(version) = version else { return; };
        if let Some(ui) = weak.upgrade() { ui.set_busy(true); }
        let weak_done = weak.clone();
        let root = data_dir.clone();
        let report = Arc::clone(&reporter);
        thread::spawn(move || {
            let outcome = (|| -> Result<()> {
                let auth = auth::ensure_session(MICROSOFT_CLIENT_ID)?;
                minecraft::install_and_launch(&root, &version, &auth, &*report)
            })();
            let _ = slint::invoke_from_event_loop(move || if let Some(ui) = weak_done.upgrade() {
                ui.set_busy(false);
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

fn update_account(window: &AppWindow, data_dir: &std::path::Path, auth: storage::AuthState) {
    window.set_account_name(auth.player_name.clone().into());
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
