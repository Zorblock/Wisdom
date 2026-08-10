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
}

pub fn user_data_dir() -> Result<PathBuf> {
    let app_data = std::env::var_os("APPDATA").context("APPDATA is not set")?;
    Ok(PathBuf::from(app_data).join("zorblock").join("userData").join("Wisdom"))
}

pub fn prepare_storage(root: &Path) -> Result<()> {
    for folder in ["cache", "versions", "libraries", "assets/objects", "assets/indexes", "game", "natives"] {
        fs::create_dir_all(root.join(folder))?;
    }
    Ok(())
}

pub fn load_auth() -> Result<AuthState> {
    let serialized = keyring::Entry::new("Wisdom Minecraft Launcher", "minecraft-session")?
        .get_password().context("keine gespeicherte Microsoft-Sitzung")?;
    Ok(serde_json::from_str(&serialized)?)
}

pub fn save_auth(auth: &AuthState) -> Result<()> {
    keyring::Entry::new("Wisdom Minecraft Launcher", "minecraft-session")?
        .set_password(&serde_json::to_string(auth)?)?;
    Ok(())
}

pub fn read_json<T: for<'a> Deserialize<'a>>(path: &Path) -> Result<T> {
    serde_json::from_reader(File::open(path).with_context(|| format!("{} öffnen", path.display()))?)
        .with_context(|| format!("{} lesen", path.display()))
}

pub fn http() -> Result<Client> {
    Ok(Client::builder().user_agent(USER_AGENT).build()?)
}
