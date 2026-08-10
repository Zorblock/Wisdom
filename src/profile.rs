use crate::storage::{AuthState, http};
use anyhow::{Context, Result};
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

pub fn cached_skin(root: &Path, auth: &AuthState) -> Result<PathBuf> {
    let url = auth.skin_url.as_ref().context("No Minecraft skin is available for this account")?;
    let path = root.join("cache").join("skins").join(format!("{}.png", auth.player_uuid));
    if path.exists() { return Ok(path); }
    fs::create_dir_all(path.parent().context("Skin cache has no parent directory")?)?;
    let bytes = http()?.get(url).send()?.error_for_status()?.bytes()?;
    File::create(&path)?.write_all(&bytes)?;
    Ok(path)
}
