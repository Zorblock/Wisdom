use crate::instance_setup;
use crate::instances::{self, Instance};
use crate::minecraft::ProgressReporter;
use crate::modloaders::ModLoader;
use crate::storage::http;
use anyhow::{Context, Result, bail};
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use sha1::Sha1;
use sha2::{Digest, Sha512};
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::time::Instant;

const API_BASE: &str = "https://api.modrinth.com/v2";
const MAX_ARCHIVE_SIZE: u64 = 1_073_741_824;
const MAX_PACK_FILE_SIZE: u64 = 2_147_483_648;
const MAX_OVERRIDE_BYTES: u64 = 4_294_967_296;
const MAX_OVERRIDE_FILES: usize = 100_000;

#[derive(Clone, Deserialize, Serialize)]
pub struct ModpackPlan {
    project_id: String,
    project_title: String,
    project_icon: Option<String>,
    version_id: String,
    version_number: String,
    file: ApiFile,
}

pub struct ResolvedModpack {
    pub name: String,
    pub game_version: String,
    pub loader: ModLoader,
    pub plan: ModpackPlan,
}

#[derive(Debug, Deserialize)]
struct ApiProject {
    id: String,
    title: String,
    icon_url: Option<String>,
    project_type: String,
}

#[derive(Clone, Debug, Deserialize)]
struct ApiVersion {
    id: String,
    project_id: String,
    version_number: String,
    version_type: String,
    #[serde(default)]
    game_versions: Vec<String>,
    #[serde(default)]
    loaders: Vec<String>,
    files: Vec<ApiFile>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct ApiFile {
    hashes: HashMap<String, String>,
    url: String,
    filename: String,
    primary: bool,
    size: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ModrinthIndex {
    format_version: u32,
    game: String,
    version_id: String,
    name: String,
    files: Vec<PackFile>,
    dependencies: HashMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PackFile {
    path: String,
    hashes: HashMap<String, String>,
    #[serde(default)]
    env: Option<PackEnvironment>,
    downloads: Vec<String>,
    file_size: u64,
}

#[derive(Debug, Deserialize)]
struct PackEnvironment {
    client: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct InstalledModpack {
    project_id: String,
    project_title: String,
    project_icon: Option<String>,
    version_id: String,
    version_number: String,
}

pub fn resolve(project_id: &str, preferred_game_version: &str) -> Result<ResolvedModpack> {
    validate_id(project_id)?;
    let client = http()?;
    let project: ApiProject = client
        .get(format!("{API_BASE}/project/{project_id}"))
        .send()?
        .error_for_status()?
        .json()?;
    if project.id != project_id || project.project_type != "modpack" {
        bail!("The selected Modrinth project is not a modpack");
    }
    let versions: Vec<ApiVersion> = client
        .get(format!("{API_BASE}/project/{project_id}/version"))
        .query(&[
            (
                "game_versions",
                serde_json::to_string(&[preferred_game_version])?,
            ),
            ("include_changelog", "false".to_owned()),
        ])
        .send()?
        .error_for_status()?
        .json()?;
    let version = versions
        .into_iter()
        .find(|version| {
            version.project_id == project_id
                && version.version_type == "release"
                && version
                    .game_versions
                    .iter()
                    .any(|value| value == preferred_game_version)
                && loader_from_names(&version.loaders).is_some()
                && select_mrpack(version).is_some()
        })
        .with_context(|| {
            format!("No stable modpack release is available for Minecraft {preferred_game_version}")
        })?;
    let loader =
        loader_from_names(&version.loaders).context("The modpack loader is unsupported")?;
    let file = select_mrpack(&version)
        .context("The stable Modrinth release has no .mrpack file")?
        .clone();
    Ok(ResolvedModpack {
        name: project.title.clone(),
        game_version: preferred_game_version.to_owned(),
        loader,
        plan: ModpackPlan {
            project_id: project.id,
            project_title: project.title,
            project_icon: safe_icon_url(project.icon_url),
            version_id: version.id,
            version_number: version.version_number,
            file,
        },
    })
}

pub fn install(
    root: &Path,
    instance: &Instance,
    plan: ModpackPlan,
    progress: &ProgressReporter<'_>,
) -> Result<()> {
    let client = http()?;
    let archive = instances::game_dir(root, instance)
        .join(".wisdom")
        .join(format!("{}.mrpack", plan.version_id));
    fs::create_dir_all(
        archive
            .parent()
            .context("Modpack cache path has no parent")?,
    )?;
    progress(0.01, format!("Downloading {}...", plan.project_title));
    download_archive(&client, &plan.file, &archive, &|value, speed| {
        progress(
            value * 0.12,
            format!(
                "Downloading {} · {}% · {}",
                plan.project_title,
                (value * 100.0).round() as u32,
                format_speed(speed)
            ),
        );
    })?;
    let index = read_index(&archive)?;
    validate_index(&index, instance)?;

    let mut configured = instance.clone();
    configured.loader_version =
        loader_version(&index.dependencies, configured.loader, &configured.version);
    instances::save(root, &configured)?;
    let base_progress = |value: f32, message: String| progress(0.12 + value * 0.48, message);
    instance_setup::prepare(root, &configured, &base_progress)?;

    install_pack_files(&client, root, &configured, &index, &|value, message| {
        progress(0.60 + value * 0.34, message);
    })?;
    progress(0.95, "Applying modpack overrides...".to_owned());
    extract_overrides(&archive, &instances::game_dir(root, &configured))?;
    let metadata = InstalledModpack {
        project_id: plan.project_id,
        project_title: plan.project_title,
        project_icon: plan.project_icon,
        version_id: plan.version_id,
        version_number: plan.version_number,
    };
    fs::write(
        instances::game_dir(root, &configured)
            .join(".wisdom")
            .join("modpack.json"),
        serde_json::to_vec_pretty(&metadata)?,
    )?;
    let _ = fs::remove_file(pending_path(root, &configured));
    progress(1.0, format!("{} is ready to play.", index.name));
    Ok(())
}

pub fn save_pending(root: &Path, instance: &Instance, plan: &ModpackPlan) -> Result<()> {
    let path = pending_path(root, instance);
    fs::create_dir_all(path.parent().context("Modpack state path has no parent")?)?;
    fs::write(path, serde_json::to_vec_pretty(plan)?)?;
    Ok(())
}

pub fn load_pending(root: &Path, instance: &Instance) -> Result<Option<ModpackPlan>> {
    match fs::read(pending_path(root, instance)) {
        Ok(bytes) => Ok(Some(
            serde_json::from_slice(&bytes).context("The pending modpack state is corrupted")?,
        )),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn pending_path(root: &Path, instance: &Instance) -> PathBuf {
    instances::game_dir(root, instance)
        .join(".wisdom")
        .join("modpack-pending.json")
}

fn install_pack_files(
    client: &Client,
    root: &Path,
    instance: &Instance,
    index: &ModrinthIndex,
    report: &(dyn Fn(f32, String) + Send + Sync),
) -> Result<()> {
    let files = index
        .files
        .iter()
        .filter(|file| {
            file.env.as_ref().and_then(|env| env.client.as_deref()) != Some("unsupported")
        })
        .collect::<Vec<_>>();
    let total = files.iter().map(|file| file.file_size).sum::<u64>().max(1);
    let mut completed = 0u64;
    for (position, file) in files.iter().enumerate() {
        let relative = safe_relative_path(&file.path)?;
        let destination = instances::game_dir(root, instance).join(relative);
        if destination
            .parent()
            .is_none_or(|parent| !parent.starts_with(instances::game_dir(root, instance)))
        {
            bail!("Modpack file escapes the instance directory");
        }
        fs::create_dir_all(destination.parent().context("Pack file has no parent")?)?;
        let url = file
            .downloads
            .iter()
            .find_map(|value| {
                reqwest::Url::parse(value)
                    .ok()
                    .filter(|url| url.scheme() == "https")
            })
            .context("A modpack file has no secure download URL")?;
        let expected = expected_hash(&file.hashes)?;
        download_pack_file(
            client,
            url,
            &destination,
            file.file_size,
            expected,
            |received, speed| {
                let current = completed.saturating_add(received.min(file.file_size));
                report(
                    (current as f64 / total as f64).clamp(0.0, 1.0) as f32,
                    format!(
                        "Downloading modpack files · {}% · {} · {}/{} files",
                        ((current as f64 / total as f64) * 100.0).round() as u32,
                        format_speed(speed),
                        position,
                        files.len()
                    ),
                );
            },
        )?;
        completed = completed.saturating_add(file.file_size);
    }
    report(
        1.0,
        format!("Downloaded {}/{} modpack files", files.len(), files.len()),
    );
    Ok(())
}

fn download_archive(
    client: &Client,
    file: &ApiFile,
    destination: &Path,
    report: &(dyn Fn(f32, f64) + Send + Sync),
) -> Result<()> {
    if file.size > MAX_ARCHIVE_SIZE {
        bail!("Modpack archive is unexpectedly large");
    }
    let expected = file
        .hashes
        .get("sha512")
        .context("Modpack archive has no SHA-512 hash")?;
    let url = reqwest::Url::parse(&file.url)?;
    if url.scheme() != "https" {
        bail!("Modpack archive must use HTTPS");
    }
    download_with_sha512(client, url, destination, file.size, expected, report)
}

fn download_pack_file(
    client: &Client,
    url: reqwest::Url,
    destination: &Path,
    expected_size: u64,
    expected: ExpectedHash<'_>,
    mut report: impl FnMut(u64, f64),
) -> Result<()> {
    if expected_size > MAX_PACK_FILE_SIZE {
        bail!("A modpack file is unexpectedly large");
    }
    if destination.is_file() && verify_file(destination, expected)? {
        report(expected_size, 0.0);
        return Ok(());
    }
    let mut response = client.get(url).send()?.error_for_status()?;
    let temporary = destination.with_extension("download");
    let mut output = File::create(&temporary)?;
    let mut sha1 = Sha1::new();
    let mut sha512 = Sha512::new();
    let mut received = 0u64;
    let started = Instant::now();
    let mut buffer = [0u8; 131_072];
    loop {
        let count = response.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        received += count as u64;
        if received > MAX_PACK_FILE_SIZE {
            let _ = fs::remove_file(&temporary);
            bail!("A modpack file exceeded the size limit");
        }
        output.write_all(&buffer[..count])?;
        sha1.update(&buffer[..count]);
        sha512.update(&buffer[..count]);
        report(
            received,
            received as f64 / started.elapsed().as_secs_f64().max(0.001),
        );
    }
    output.flush()?;
    let valid = match expected {
        ExpectedHash::Sha1(value) => format!("{:x}", sha1.finalize()).eq_ignore_ascii_case(value),
        ExpectedHash::Sha512(value) => {
            format!("{:x}", sha512.finalize()).eq_ignore_ascii_case(value)
        }
    };
    if !valid {
        let _ = fs::remove_file(&temporary);
        bail!("A downloaded modpack file failed its checksum verification");
    }
    if destination.exists() {
        fs::remove_file(destination)?;
    }
    fs::rename(temporary, destination)?;
    Ok(())
}

fn download_with_sha512(
    client: &Client,
    url: reqwest::Url,
    destination: &Path,
    expected_size: u64,
    expected: &str,
    report: &(dyn Fn(f32, f64) + Send + Sync),
) -> Result<()> {
    if destination.is_file() && verify_file(destination, ExpectedHash::Sha512(expected))? {
        report(1.0, 0.0);
        return Ok(());
    }
    let mut response = client.get(url).send()?.error_for_status()?;
    let temporary = destination.with_extension("download");
    let mut output = File::create(&temporary)?;
    let mut hasher = Sha512::new();
    let mut received = 0u64;
    let started = Instant::now();
    let mut buffer = [0u8; 131_072];
    loop {
        let count = response.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        received += count as u64;
        if received > MAX_ARCHIVE_SIZE {
            let _ = fs::remove_file(&temporary);
            bail!("Modpack archive exceeded the size limit");
        }
        output.write_all(&buffer[..count])?;
        hasher.update(&buffer[..count]);
        report(
            (received as f64 / expected_size.max(1) as f64).clamp(0.0, 1.0) as f32,
            received as f64 / started.elapsed().as_secs_f64().max(0.001),
        );
    }
    output.flush()?;
    if !format!("{:x}", hasher.finalize()).eq_ignore_ascii_case(expected) {
        let _ = fs::remove_file(&temporary);
        bail!("Downloaded modpack archive failed checksum verification");
    }
    if destination.exists() {
        fs::remove_file(destination)?;
    }
    fs::rename(temporary, destination)?;
    Ok(())
}

fn read_index(archive_path: &Path) -> Result<ModrinthIndex> {
    let file = File::open(archive_path)?;
    let mut archive = zip::ZipArchive::new(file).context("The .mrpack archive is invalid")?;
    let mut index_file = archive
        .by_name("modrinth.index.json")
        .context("The archive has no modrinth.index.json")?;
    if index_file.size() > 16 * 1024 * 1024 {
        bail!("The modpack index is unexpectedly large");
    }
    let mut bytes = Vec::with_capacity(index_file.size() as usize);
    index_file.read_to_end(&mut bytes)?;
    Ok(serde_json::from_slice(&bytes).context("The modpack index is invalid")?)
}

fn extract_overrides(archive_path: &Path, destination: &Path) -> Result<()> {
    let file = File::open(archive_path)?;
    let mut archive = zip::ZipArchive::new(file)?;
    let mut extracted_bytes = 0u64;
    let mut extracted_files = 0usize;
    // The Modrinth format requires client-overrides to win over common overrides.
    for prefix in ["overrides/", "client-overrides/"] {
        for index in 0..archive.len() {
            let mut entry = archive.by_index(index)?;
            let normalized = entry.name().replace('\\', "/");
            let Some(value) = normalized.strip_prefix(prefix) else {
                continue;
            };
            if value.is_empty() {
                continue;
            }
            let relative = safe_relative_path(value)?;
            if entry
                .unix_mode()
                .is_some_and(|mode| mode & 0o170000 == 0o120000)
            {
                bail!("Modpack overrides may not contain symbolic links");
            }
            if entry.is_dir() {
                fs::create_dir_all(destination.join(relative))?;
                continue;
            }
            extracted_files += 1;
            extracted_bytes = extracted_bytes.saturating_add(entry.size());
            if extracted_files > MAX_OVERRIDE_FILES || extracted_bytes > MAX_OVERRIDE_BYTES {
                bail!("Modpack overrides exceed safe extraction limits");
            }
            let target = destination.join(relative);
            fs::create_dir_all(target.parent().context("Override file has no parent")?)?;
            let temporary = target.with_extension("override-download");
            let mut output = File::create(&temporary)?;
            std::io::copy(&mut entry, &mut output)?;
            output.flush()?;
            if target.exists() {
                if target.is_dir() {
                    bail!("A modpack override conflicts with an existing directory");
                }
                fs::remove_file(&target)?;
            }
            fs::rename(temporary, target)?;
        }
    }
    Ok(())
}

fn validate_index(index: &ModrinthIndex, instance: &Instance) -> Result<()> {
    if index.format_version != 1 || index.game != "minecraft" {
        bail!("The modpack uses an unsupported format");
    }
    if index.version_id.trim().is_empty() || index.name.trim().is_empty() {
        bail!("The modpack index is incomplete");
    }
    let minecraft = index
        .dependencies
        .get("minecraft")
        .context("The modpack does not declare a Minecraft version")?;
    if minecraft != &instance.version {
        bail!(
            "The modpack targets Minecraft {minecraft}, not {}",
            instance.version
        );
    }
    let declared_loader = loader_from_dependencies(&index.dependencies);
    if declared_loader != Some(instance.loader) {
        bail!("The modpack loader does not match the created instance");
    }
    Ok(())
}

fn loader_from_names(loaders: &[String]) -> Option<ModLoader> {
    [
        ("fabric", ModLoader::Fabric),
        ("quilt", ModLoader::Quilt),
        ("neoforge", ModLoader::Neoforge),
        ("forge", ModLoader::Forge),
    ]
    .into_iter()
    .find_map(|(name, loader)| loaders.iter().any(|value| value == name).then_some(loader))
}

fn loader_from_dependencies(dependencies: &HashMap<String, String>) -> Option<ModLoader> {
    if dependencies.contains_key("fabric-loader") {
        Some(ModLoader::Fabric)
    } else if dependencies.contains_key("quilt-loader") {
        Some(ModLoader::Quilt)
    } else if dependencies.contains_key("neoforge") {
        Some(ModLoader::Neoforge)
    } else if dependencies.contains_key("forge") {
        Some(ModLoader::Forge)
    } else {
        Some(ModLoader::Vanilla)
    }
}

fn loader_version(
    dependencies: &HashMap<String, String>,
    loader: ModLoader,
    minecraft: &str,
) -> Option<String> {
    match loader {
        ModLoader::Vanilla => None,
        ModLoader::Fabric => dependencies.get("fabric-loader").cloned(),
        ModLoader::Quilt => dependencies.get("quilt-loader").cloned(),
        ModLoader::Neoforge => dependencies.get("neoforge").cloned(),
        ModLoader::Forge => dependencies.get("forge").map(|version| {
            if version.starts_with(&format!("{minecraft}-")) {
                version.clone()
            } else {
                format!("{minecraft}-{version}")
            }
        }),
    }
}

fn select_mrpack(version: &ApiVersion) -> Option<&ApiFile> {
    version
        .files
        .iter()
        .find(|file| file.primary && file.filename.to_ascii_lowercase().ends_with(".mrpack"))
        .or_else(|| {
            version
                .files
                .iter()
                .find(|file| file.filename.to_ascii_lowercase().ends_with(".mrpack"))
        })
}

fn safe_relative_path(value: &str) -> Result<PathBuf> {
    let path = Path::new(value);
    if path.as_os_str().is_empty()
        || path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        bail!("Modpack contains an unsafe path: {value}");
    }
    Ok(path.to_path_buf())
}

#[derive(Clone, Copy)]
enum ExpectedHash<'a> {
    Sha1(&'a str),
    Sha512(&'a str),
}

fn expected_hash(hashes: &HashMap<String, String>) -> Result<ExpectedHash<'_>> {
    if let Some(value) = hashes.get("sha512") {
        Ok(ExpectedHash::Sha512(value))
    } else if let Some(value) = hashes.get("sha1") {
        Ok(ExpectedHash::Sha1(value))
    } else {
        bail!("A modpack file has no supported checksum")
    }
}

fn verify_file(path: &Path, expected: ExpectedHash<'_>) -> Result<bool> {
    let mut input = File::open(path)?;
    let mut buffer = [0u8; 131_072];
    match expected {
        ExpectedHash::Sha1(value) => {
            let mut hasher = Sha1::new();
            loop {
                let count = input.read(&mut buffer)?;
                if count == 0 {
                    break;
                }
                hasher.update(&buffer[..count]);
            }
            Ok(format!("{:x}", hasher.finalize()).eq_ignore_ascii_case(value))
        }
        ExpectedHash::Sha512(value) => {
            let mut hasher = Sha512::new();
            loop {
                let count = input.read(&mut buffer)?;
                if count == 0 {
                    break;
                }
                hasher.update(&buffer[..count]);
            }
            Ok(format!("{:x}", hasher.finalize()).eq_ignore_ascii_case(value))
        }
    }
}

fn validate_id(value: &str) -> Result<()> {
    if value.len() < 3
        || value.len() > 64
        || !value
            .chars()
            .all(|character| character.is_ascii_alphanumeric())
    {
        bail!("Invalid Modrinth project ID");
    }
    Ok(())
}

fn safe_icon_url(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        let url = reqwest::Url::parse(&value).ok()?;
        (url.scheme() == "https"
            && url
                .host_str()
                .is_some_and(|host| host == "cdn.modrinth.com" || host.ends_with(".modrinth.com")))
        .then_some(value)
    })
}

fn format_speed(bytes_per_second: f64) -> String {
    if bytes_per_second >= 1024.0 * 1024.0 {
        format!("{:.1} MB/s", bytes_per_second / (1024.0 * 1024.0))
    } else if bytes_per_second >= 1024.0 {
        format!("{:.0} KB/s", bytes_per_second / 1024.0)
    } else {
        format!("{:.0} B/s", bytes_per_second)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_only_relative_pack_paths() {
        assert_eq!(
            safe_relative_path("mods/example.jar").unwrap(),
            PathBuf::from("mods/example.jar")
        );
        assert!(safe_relative_path("../outside.txt").is_err());
        assert!(safe_relative_path("/absolute.txt").is_err());
        assert!(safe_relative_path("").is_err());
    }

    #[test]
    fn normalizes_forge_loader_versions() {
        let dependencies = HashMap::from([("forge".to_owned(), "47.2.0".to_owned())]);
        assert_eq!(
            loader_version(&dependencies, ModLoader::Forge, "1.20.1").as_deref(),
            Some("1.20.1-47.2.0")
        );
    }

    #[test]
    fn recognizes_supported_modpack_loaders() {
        assert_eq!(
            loader_from_names(&["fabric".to_owned()]),
            Some(ModLoader::Fabric)
        );
        assert_eq!(loader_from_names(&["bukkit".to_owned()]), None);
    }
}
