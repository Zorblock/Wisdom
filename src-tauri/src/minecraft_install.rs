use crate::downloads::{Checksum, DownloadJob, download_jobs};
use crate::minecraft::ProgressReporter;
use anyhow::{Context, Result};
use mc_launcher_core::compatibility::{CompatibilityPolicy, apply_compatibility};
use mc_launcher_core::core::version::VersionJson;
use mc_launcher_core::install::{assets, client, natives, vanilla};
use mc_launcher_core::net::download::{Checksum as CoreChecksum, DownloadPlan};
use mc_launcher_core::platform::Platform;
use reqwest::blocking::Client;
use std::fs;
use std::path::Path;

pub fn install_vanilla(
    client_http: &Client,
    minecraft_dir: &Path,
    version_id: &str,
    progress: &ProgressReporter<'_>,
) -> Result<()> {
    progress(0.01, format!("Resolving Minecraft {version_id}..."));
    let version = client::fetch_vanilla_version(version_id)
        .context("Could not resolve the Minecraft version")?;
    client::write_version_json(minecraft_dir, &version)?;
    install_profile(client_http, minecraft_dir, &version, progress)
}

pub fn install_profile(
    client_http: &Client,
    minecraft_dir: &Path,
    version: &VersionJson,
    progress: &ProgressReporter<'_>,
) -> Result<()> {
    let platform = Platform::current();
    let compatible = apply_compatibility(&version, platform, CompatibilityPolicy::Auto);
    let version = &compatible.version;
    let version_id = version.id.as_deref().context("Version profile has no ID")?;

    let base_plan = vanilla::plan_vanilla_downloads_for_platform(
        version,
        minecraft_dir,
        platform,
        CompatibilityPolicy::Disabled,
    )?;
    let base_progress = |value: f32, message: String| {
        progress(0.02 + value * 0.48, message);
    };
    download_core_plan(client_http, base_plan, "Minecraft files", &base_progress)?;

    if let Some(asset_index) = &version.asset_index {
        let index_path = assets::asset_index_path(minecraft_dir, &asset_index.id);
        let index = serde_json::from_slice(&fs::read(index_path)?)?;
        let asset_plan = assets::plan_asset_object_downloads_from_index(&index, minecraft_dir);
        let asset_progress = |value: f32, message: String| {
            progress(0.50 + value * 0.47, message);
        };
        download_core_plan(client_http, asset_plan, "game assets", &asset_progress)?;
    }

    progress(0.98, "Extracting Windows components...".to_owned());
    natives::extract_natives_for_platform(&version.libraries, minecraft_dir, version_id, platform)?;
    progress(1.0, format!("Minecraft {version_id} is ready."));
    Ok(())
}

fn download_core_plan(
    client: &Client,
    plan: DownloadPlan,
    category: &str,
    progress: &ProgressReporter<'_>,
) -> Result<()> {
    let jobs = plan
        .tasks
        .into_iter()
        .map(|task| DownloadJob {
            url: task.url,
            destination: task.destination,
            checksum: task.checksum.map(|checksum| match checksum {
                CoreChecksum::Sha1(value) => Checksum::Sha1(value),
                CoreChecksum::Sha256(value) => Checksum::Sha256(value),
            }),
        })
        .collect();
    download_jobs(client, jobs, category, progress)
}
