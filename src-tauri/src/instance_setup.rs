use crate::instances::{self, Instance};
use crate::minecraft::ProgressReporter;
use crate::{minecraft_install, modloaders, runtime, storage};
use anyhow::{Context, Result};
use mc_launcher_core::launcher::Launcher;
use std::path::Path;

pub fn prepare(
    root: &Path,
    instance: &Instance,
    progress: &ProgressReporter<'_>,
) -> Result<Instance> {
    if instance.loader.supports_mods() {
        let prepared = modloaders::prepare_installation(root, instance, progress)?;
        return instances::mark_prepared(root, &instance.id, Some(prepared.loader_version));
    }

    prepare_vanilla(root, instance, progress)?;
    instances::mark_prepared(root, &instance.id, None)
}

fn prepare_vanilla(
    root: &Path,
    instance: &Instance,
    progress: &ProgressReporter<'_>,
) -> Result<()> {
    let client = storage::http()?;
    let game_progress = |value: f32, message: String| progress(value * 0.9, message);
    minecraft_install::install_vanilla(&client, root, &instance.version, &game_progress)
        .context("Could not install Minecraft")?;

    let version = Launcher::new(root)
        .load_version(&instance.version)
        .context("Could not load the installed Minecraft version")?;
    let java_major = version
        .java_version
        .as_ref()
        .map(|java| java.major_version.max(8) as u32)
        .unwrap_or(8);
    let java_progress = |value: f32, message: String| progress(0.9 + value * 0.1, message);
    runtime::ensure_java(root, java_major, &java_progress)?;
    progress(1.0, format!("Minecraft {} is ready.", instance.version));
    Ok(())
}
