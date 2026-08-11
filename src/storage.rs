use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::path::{Path, PathBuf};

const USER_AGENT: &str = "WisdomLauncher/0.1 (Windows; Rust)";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthState {
    pub minecraft_access_token: String,
    pub microsoft_refresh_token: String,
    pub expires_at: DateTime<Utc>,
    pub player_name: String,
    pub player_uuid: String,
    #[serde(default)]
    pub skin_url: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct SessionProfile {
    expires_at: DateTime<Utc>,
    player_name: String,
    player_uuid: String,
    skin_url: Option<String>,
}

#[derive(Default, Serialize, Deserialize)]
struct LauncherSettings {
    #[serde(default)]
    open_console: bool,
}

pub fn user_data_dir() -> Result<PathBuf> {
    let app_data = std::env::var_os("APPDATA").context("APPDATA is not set")?;
    Ok(PathBuf::from(app_data).join("zorblock").join("userData").join("Wisdom"))
}

pub fn prepare_storage(root: &Path) -> Result<()> {
    for folder in ["cache", "versions", "libraries", "assets/objects", "assets/indexes", "instances", "natives"] {
        fs::create_dir_all(root.join(folder))?;
    }
    Ok(())
}

pub fn load_auth() -> Result<AuthState> {
    let refresh_token = credential("minecraft-refresh-token")?.get_password().context("no saved Microsoft session")?;
    let minecraft_access_token = credential("minecraft-access-token")?.get_password().context("no saved Minecraft session")?;
    let profile: SessionProfile = serde_json::from_str(&credential("minecraft-profile")?.get_password()?)?;
    Ok(AuthState { minecraft_access_token, microsoft_refresh_token: refresh_token, expires_at: profile.expires_at, player_name: profile.player_name, player_uuid: profile.player_uuid, skin_url: profile.skin_url })
}

pub fn save_auth(auth: &AuthState) -> Result<()> {
    credential("minecraft-refresh-token")?.set_password(&auth.microsoft_refresh_token)?;
    credential("minecraft-access-token")?.set_password(&auth.minecraft_access_token)?;
    let profile = SessionProfile { expires_at: auth.expires_at, player_name: auth.player_name.clone(), player_uuid: auth.player_uuid.clone(), skin_url: auth.skin_url.clone() };
    credential("minecraft-profile")?.set_password(&serde_json::to_string(&profile)?)?;
    Ok(())
}

pub fn clear_auth() -> Result<()> {
    for name in ["minecraft-refresh-token", "minecraft-access-token", "minecraft-profile"] {
        let _ = credential(name)?.delete_credential();
    }
    Ok(())
}

pub fn load_open_console(root: &Path) -> bool {
    fs::read_to_string(root.join("settings.json"))
        .ok()
        .and_then(|contents| serde_json::from_str::<LauncherSettings>(&contents).ok())
        .is_some_and(|settings| settings.open_console)
}

pub fn save_open_console(root: &Path, open_console: bool) -> Result<()> {
    let settings = LauncherSettings { open_console };
    fs::write(root.join("settings.json"), serde_json::to_vec_pretty(&settings)?)?;
    Ok(())
}

fn credential(name: &str) -> Result<keyring::Entry> {
    Ok(keyring::Entry::new("Wisdom Minecraft Launcher", name)?)
}

pub fn read_json<T: for<'a> Deserialize<'a>>(path: &Path) -> Result<T> {
    serde_json::from_reader(File::open(path).with_context(|| format!("could not open {}", path.display()))?)
        .with_context(|| format!("could not read {}", path.display()))
}

pub fn http() -> Result<Client> {
    Ok(Client::builder().user_agent(USER_AGENT).build()?)
}
