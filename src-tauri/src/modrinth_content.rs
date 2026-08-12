use crate::instances::{self, Instance};
use crate::modrinth::{SearchHit, SearchResults};
use crate::modrinth_versions::{self, ReleaseChannel, VersionChoice};
use crate::storage::http;
use anyhow::{Context, Result, bail};
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha512};
use std::collections::HashMap;
use std::ffi::OsStr;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

const API_BASE: &str = "https://api.modrinth.com/v2";
const MAX_PACK_SIZE: u64 = 1_073_741_824;

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ContentKind {
    Modpack,
    Resourcepack,
    Shader,
}

impl ContentKind {
    fn project_type(self) -> &'static str {
        match self {
            Self::Modpack => "modpack",
            Self::Resourcepack => "resourcepack",
            Self::Shader => "shader",
        }
    }

    fn directory(self) -> &'static str {
        match self {
            Self::Modpack => ".",
            Self::Resourcepack => "resourcepacks",
            Self::Shader => "shaderpacks",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Modpack => "modpack",
            Self::Resourcepack => "resource pack",
            Self::Shader => "shader",
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledContentView {
    pub project_id: String,
    pub title: String,
    pub version_number: String,
    pub version_type: ReleaseChannel,
    pub icon_url: Option<String>,
    pub explicit: bool,
    pub enabled: bool,
    pub compatible: bool,
    pub missing: bool,
    pub file_name: String,
    pub file_size: u64,
    pub dependency_count: usize,
    pub required_by_count: usize,
    pub update_available: bool,
    pub latest_version_number: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct PackManifest {
    #[serde(default = "manifest_version")]
    schema_version: u32,
    #[serde(default)]
    entries: Vec<InstalledContent>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct InstalledContent {
    project_id: String,
    version_id: String,
    title: String,
    version_number: String,
    #[serde(default)]
    version_type: ReleaseChannel,
    file_name: String,
    sha512: String,
    icon_url: Option<String>,
    kind: ContentKind,
    #[serde(default = "default_true")]
    enabled: bool,
    #[serde(default)]
    game_versions: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct ApiSearchResults {
    hits: Vec<ApiSearchHit>,
    offset: usize,
    total_hits: usize,
}

#[derive(Debug, Deserialize)]
struct ApiSearchHit {
    project_id: String,
    title: String,
    description: String,
    author: String,
    icon_url: Option<String>,
    downloads: u64,
    #[serde(default)]
    display_categories: Vec<String>,
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
    files: Vec<ApiFile>,
}

#[derive(Clone, Debug, Deserialize)]
struct ApiFile {
    hashes: HashMap<String, String>,
    url: String,
    filename: String,
    primary: bool,
    size: u64,
}

fn manifest_version() -> u32 {
    1
}

fn default_true() -> bool {
    true
}

pub fn search(
    root: &Path,
    instance_id: &str,
    kind: ContentKind,
    query: &str,
    index: &str,
    category: Option<&str>,
    offset: usize,
) -> Result<SearchResults> {
    let instance = instances::load(root, instance_id)?;
    search_for_version(&instance.version, kind, query, index, category, offset)
}

pub fn search_for_version(
    game_version: &str,
    kind: ContentKind,
    query: &str,
    index: &str,
    category: Option<&str>,
    offset: usize,
) -> Result<SearchResults> {
    validate_game_version(game_version)?;
    let query = query.trim();
    if query.chars().count() > 100 {
        bail!("Search query is too long");
    }
    if !matches!(
        index,
        "relevance" | "downloads" | "follows" | "newest" | "updated"
    ) {
        bail!("Invalid Modrinth sort option");
    }
    let mut facets = vec![
        vec![format!("versions:{game_version}")],
        vec![format!("project_type:{}", kind.project_type())],
    ];
    if let Some(category) = category.filter(|value| valid_category(value)) {
        facets.push(vec![format!("categories:{category}")]);
    }
    let response: ApiSearchResults = http()?
        .get(format!("{API_BASE}/search"))
        .query(&[
            ("query", query.to_owned()),
            ("facets", serde_json::to_string(&facets)?),
            ("index", index.to_owned()),
            ("offset", offset.min(10_000).to_string()),
            ("limit", "24".to_owned()),
        ])
        .send()?
        .error_for_status()?
        .json()?;
    Ok(SearchResults {
        hits: response
            .hits
            .into_iter()
            .map(|hit| SearchHit {
                project_id: hit.project_id,
                title: hit.title,
                description: hit.description,
                author: hit.author,
                icon_url: safe_icon_url(hit.icon_url),
                downloads: hit.downloads,
                categories: hit
                    .display_categories
                    .into_iter()
                    .filter(|value| valid_category(value))
                    .take(3)
                    .collect(),
            })
            .collect(),
        offset: response.offset,
        total_hits: response.total_hits,
    })
}

pub fn list_installed(
    root: &Path,
    instance_id: &str,
    kind: ContentKind,
    refresh_updates: bool,
) -> Result<Vec<InstalledContentView>> {
    if kind == ContentKind::Modpack {
        return Ok(Vec::new());
    }
    let instance = instances::load(root, instance_id)?;
    let manifest = load_manifest(root, &instance)?;
    let client = refresh_updates.then(http).transpose()?;
    let directory = content_dir(root, &instance, kind);
    let mut views = manifest
        .entries
        .iter()
        .filter(|entry| entry.kind == kind)
        .map(|entry| {
            let update = (entry.version_type == ReleaseChannel::Release)
                .then_some(())
                .and(client.as_ref().and_then(|client| {
                    latest_release(client, &entry.project_id, &instance.version).ok()
                }))
                .filter(|version| version.id != entry.version_id);
            let path = managed_path(&directory, &entry.file_name, entry.enabled).ok();
            InstalledContentView {
                project_id: entry.project_id.clone(),
                title: entry.title.clone(),
                version_number: entry.version_number.clone(),
                version_type: entry.version_type,
                icon_url: safe_icon_url(entry.icon_url.clone()),
                explicit: true,
                enabled: entry.enabled,
                compatible: entry
                    .game_versions
                    .iter()
                    .any(|value| value == &instance.version),
                missing: path.as_ref().is_none_or(|path| !path.is_file()),
                file_name: entry.file_name.clone(),
                file_size: path
                    .and_then(|path| path.metadata().ok())
                    .map(|metadata| metadata.len())
                    .unwrap_or(0),
                dependency_count: 0,
                required_by_count: 0,
                update_available: update.is_some(),
                latest_version_number: update.map(|version| version.version_number),
            }
        })
        .collect::<Vec<_>>();
    views.sort_by_key(|entry| entry.title.to_lowercase());
    Ok(views)
}

pub fn install(
    root: &Path,
    instance_id: &str,
    kind: ContentKind,
    project_id: &str,
    version_id: &str,
    report: &(dyn Fn(f32) + Send + Sync),
) -> Result<Vec<InstalledContentView>> {
    if kind == ContentKind::Modpack {
        bail!("Modpacks must be installed as new instances");
    }
    validate_id(project_id)?;
    let instance = instances::load(root, instance_id)?;
    let client = http()?;
    let project = project(&client, project_id)?;
    if project.project_type != kind.project_type() {
        bail!("The Modrinth project is not a {}", kind.label());
    }
    let version = version(&client, version_id)?;
    if version.project_id != project_id
        || !version
            .game_versions
            .iter()
            .any(|value| value == &instance.version)
    {
        bail!(
            "The selected {} version is not compatible with Minecraft {}",
            kind.label(),
            instance.version
        );
    }
    let file = select_zip(&version)?;
    let sha512 = file
        .hashes
        .get("sha512")
        .cloned()
        .context("Modrinth file has no SHA-512 hash")?;
    let mut manifest = load_manifest(root, &instance)?;
    let file_name = unique_name(&manifest, project_id, &safe_zip_name(&file.filename)?);
    let directory = content_dir(root, &instance, kind);
    fs::create_dir_all(&directory)?;
    let destination = managed_path(&directory, &file_name, true)?;
    download(&client, file, &destination, &sha512, report)?;
    let previous = manifest
        .entries
        .iter()
        .find(|entry| entry.project_id == project_id && entry.kind == kind)
        .cloned();
    let entry = InstalledContent {
        project_id: project.id,
        version_id: version.id,
        title: project.title,
        version_number: version.version_number,
        version_type: ReleaseChannel::from_api(&version.version_type).unwrap_or_default(),
        file_name,
        sha512,
        icon_url: safe_icon_url(project.icon_url),
        kind,
        enabled: true,
        game_versions: version.game_versions,
    };
    if let Some(index) = manifest
        .entries
        .iter()
        .position(|entry| entry.project_id == project_id && entry.kind == kind)
    {
        manifest.entries[index] = entry;
    } else {
        manifest.entries.push(entry);
    }
    if let Some(previous) = previous {
        let previous_path = managed_path(&directory, &previous.file_name, previous.enabled)?;
        if previous_path != destination && previous_path.is_file() {
            fs::remove_file(previous_path)?;
        }
    }
    save_manifest(root, &instance, &manifest)?;
    report(1.0);
    list_installed(root, instance_id, kind, false)
}

pub fn resolve_choice(
    root: &Path,
    instance_id: &str,
    kind: ContentKind,
    project_id: &str,
    requested: ReleaseChannel,
) -> Result<VersionChoice> {
    if kind == ContentKind::Modpack {
        bail!("Modpacks must be installed as new instances");
    }
    validate_id(project_id)?;
    let instance = instances::load(root, instance_id)?;
    let client = http()?;
    let project = project(&client, project_id)?;
    if project.project_type != kind.project_type() {
        bail!("The Modrinth project is not a {}", kind.label());
    }
    let versions = versions(&client, project_id, &instance.version)?;
    let (version, channel, requires_confirmation) = modrinth_versions::choose(
        &versions,
        requested,
        requested == ReleaseChannel::Release,
        |version| version.version_type.as_str(),
        |version| {
            version.project_id == project_id
                && version
                    .game_versions
                    .iter()
                    .any(|value| value == &instance.version)
                && select_zip(version).is_ok()
        },
    )
    .with_context(|| unavailable_message(requested, kind, &instance.version))?;
    Ok(VersionChoice {
        project_id: project.id,
        title: project.title,
        version_id: version.id.clone(),
        version_number: version.version_number.clone(),
        version_type: channel,
        requires_confirmation,
    })
}

pub fn remove(
    root: &Path,
    instance_id: &str,
    kind: ContentKind,
    project_id: &str,
) -> Result<Vec<InstalledContentView>> {
    if kind == ContentKind::Modpack {
        bail!("Modpacks are managed as instances");
    }
    let instance = instances::load(root, instance_id)?;
    let mut manifest = load_manifest(root, &instance)?;
    let index = manifest
        .entries
        .iter()
        .position(|entry| entry.project_id == project_id && entry.kind == kind)
        .context("Content is not installed")?;
    let entry = manifest.entries.remove(index);
    let path = managed_path(
        &content_dir(root, &instance, kind),
        &entry.file_name,
        entry.enabled,
    )?;
    if path.is_file() {
        fs::remove_file(path)?;
    }
    save_manifest(root, &instance, &manifest)?;
    list_installed(root, instance_id, kind, false)
}

pub fn set_enabled(
    root: &Path,
    instance_id: &str,
    kind: ContentKind,
    project_id: &str,
    enabled: bool,
) -> Result<Vec<InstalledContentView>> {
    if kind == ContentKind::Modpack {
        bail!("Modpacks are managed as instances");
    }
    let instance = instances::load(root, instance_id)?;
    let mut manifest = load_manifest(root, &instance)?;
    let entry = manifest
        .entries
        .iter_mut()
        .find(|entry| entry.project_id == project_id && entry.kind == kind)
        .context("Content is not installed")?;
    if entry.enabled != enabled {
        let directory = content_dir(root, &instance, kind);
        let source = managed_path(&directory, &entry.file_name, entry.enabled)?;
        let destination = managed_path(&directory, &entry.file_name, enabled)?;
        if !source.is_file() {
            bail!("The managed content file is missing");
        }
        if destination.exists() {
            bail!("A file with the target name already exists");
        }
        fs::rename(source, destination)?;
        entry.enabled = enabled;
        save_manifest(root, &instance, &manifest)?;
    }
    list_installed(root, instance_id, kind, false)
}

fn project(client: &Client, project_id: &str) -> Result<ApiProject> {
    Ok(client
        .get(format!("{API_BASE}/project/{project_id}"))
        .send()?
        .error_for_status()?
        .json()?)
}

fn version(client: &Client, version_id: &str) -> Result<ApiVersion> {
    validate_id(version_id)?;
    Ok(client
        .get(format!("{API_BASE}/version/{version_id}"))
        .send()?
        .error_for_status()?
        .json()?)
}

fn versions(client: &Client, project_id: &str, game_version: &str) -> Result<Vec<ApiVersion>> {
    Ok(client
        .get(format!("{API_BASE}/project/{project_id}/version"))
        .query(&[
            ("game_versions", serde_json::to_string(&[game_version])?),
            ("include_changelog", "false".to_owned()),
        ])
        .send()?
        .error_for_status()?
        .json()?)
}

fn latest_release(client: &Client, project_id: &str, game_version: &str) -> Result<ApiVersion> {
    validate_id(project_id)?;
    versions(client, project_id, game_version)?
        .into_iter()
        .find(|version| {
            version.project_id == project_id
                && version.version_type == "release"
                && version
                    .game_versions
                    .iter()
                    .any(|value| value == game_version)
                && select_zip(version).is_ok()
        })
        .with_context(|| {
            format!("No stable compatible release is available for Minecraft {game_version}")
        })
}

fn unavailable_message(channel: ReleaseChannel, kind: ContentKind, game_version: &str) -> String {
    match channel {
        ReleaseChannel::Release => format!(
            "No compatible release, beta, or alpha {} is available for Minecraft {game_version}",
            kind.label()
        ),
        ReleaseChannel::Beta => format!(
            "No compatible beta {} is available for Minecraft {game_version}",
            kind.label()
        ),
        ReleaseChannel::Alpha => format!(
            "No compatible alpha {} is available for Minecraft {game_version}",
            kind.label()
        ),
    }
}

fn select_zip(version: &ApiVersion) -> Result<&ApiFile> {
    version
        .files
        .iter()
        .find(|file| file.primary && file.filename.to_ascii_lowercase().ends_with(".zip"))
        .or_else(|| {
            version
                .files
                .iter()
                .find(|file| file.filename.to_ascii_lowercase().ends_with(".zip"))
        })
        .context("The Modrinth release has no installable ZIP file")
}

fn download(
    client: &Client,
    file: &ApiFile,
    destination: &Path,
    expected_sha512: &str,
    report: &(dyn Fn(f32) + Send + Sync),
) -> Result<()> {
    if file.size > MAX_PACK_SIZE {
        bail!("Content file is unexpectedly large");
    }
    let url = reqwest::Url::parse(&file.url)?;
    if url.scheme() != "https" {
        bail!("Content downloads must use HTTPS");
    }
    let mut response = client.get(url).send()?.error_for_status()?;
    let temporary = destination.with_extension("download");
    let mut output = File::create(&temporary)?;
    let mut hasher = Sha512::new();
    let mut received = 0u64;
    let mut buffer = [0u8; 131_072];
    loop {
        let count = response.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        received += count as u64;
        if received > MAX_PACK_SIZE {
            let _ = fs::remove_file(&temporary);
            bail!("Content download exceeded the size limit");
        }
        output.write_all(&buffer[..count])?;
        hasher.update(&buffer[..count]);
        report((received as f64 / file.size.max(1) as f64).clamp(0.0, 1.0) as f32);
    }
    output.flush()?;
    if !format!("{:x}", hasher.finalize()).eq_ignore_ascii_case(expected_sha512) {
        let _ = fs::remove_file(&temporary);
        bail!("Downloaded content checksum does not match");
    }
    if destination.exists() {
        fs::remove_file(destination)?;
    }
    fs::rename(temporary, destination)?;
    Ok(())
}

fn load_manifest(root: &Path, instance: &Instance) -> Result<PackManifest> {
    let path = manifest_path(root, instance);
    match fs::read(path) {
        Ok(bytes) => {
            let manifest: PackManifest =
                serde_json::from_slice(&bytes).context("The managed content list is corrupted")?;
            if manifest.schema_version != 1 {
                bail!("The managed content list uses an unsupported format");
            }
            Ok(manifest)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(PackManifest {
            schema_version: 1,
            entries: Vec::new(),
        }),
        Err(error) => Err(error.into()),
    }
}

fn save_manifest(root: &Path, instance: &Instance, manifest: &PackManifest) -> Result<()> {
    let path = manifest_path(root, instance);
    fs::create_dir_all(path.parent().context("Manifest path has no parent")?)?;
    let temporary = path.with_extension("tmp");
    fs::write(&temporary, serde_json::to_vec_pretty(manifest)?)?;
    if path.exists() {
        fs::remove_file(&path)?;
    }
    fs::rename(temporary, path)?;
    Ok(())
}

fn manifest_path(root: &Path, instance: &Instance) -> PathBuf {
    instances::game_dir(root, instance)
        .join(".wisdom")
        .join("content.json")
}

fn content_dir(root: &Path, instance: &Instance, kind: ContentKind) -> PathBuf {
    instances::game_dir(root, instance).join(kind.directory())
}

fn safe_zip_name(value: &str) -> Result<String> {
    if Path::new(value).file_name() != Some(OsStr::new(value))
        || !value.to_ascii_lowercase().ends_with(".zip")
        || value.chars().any(char::is_control)
    {
        bail!("Modrinth returned an unsafe content filename");
    }
    Ok(value.to_owned())
}

fn managed_path(directory: &Path, file_name: &str, enabled: bool) -> Result<PathBuf> {
    let name = safe_zip_name(file_name)?;
    let path = directory.join(if enabled {
        name
    } else {
        format!("{name}.disabled")
    });
    if path.parent() != Some(directory) {
        bail!("Invalid managed content path");
    }
    Ok(path)
}

fn unique_name(manifest: &PackManifest, project_id: &str, file_name: &str) -> String {
    if manifest
        .entries
        .iter()
        .all(|entry| entry.project_id == project_id || entry.file_name != file_name)
    {
        file_name.to_owned()
    } else {
        format!("{project_id}-{file_name}")
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

fn validate_game_version(value: &str) -> Result<()> {
    if value.is_empty() || value.len() > 64 || value.chars().any(|character| character.is_control())
    {
        bail!("Invalid Minecraft version");
    }
    Ok(())
}

fn valid_category(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 48
        && value
            .chars()
            .all(|character| character.is_ascii_lowercase() || character == '-')
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_only_flat_zip_names() {
        assert!(safe_zip_name("Complementary.zip").is_ok());
        assert!(safe_zip_name("../Complementary.zip").is_err());
        assert!(safe_zip_name("Complementary.jar").is_err());
    }

    #[test]
    fn maps_content_to_minecraft_directories() {
        assert_eq!(ContentKind::Resourcepack.directory(), "resourcepacks");
        assert_eq!(ContentKind::Shader.directory(), "shaderpacks");
    }
}
