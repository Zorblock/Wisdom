use crate::instances::{self, Instance};
use crate::modrinth_versions::{self, ReleaseChannel, VersionChoice};
use crate::storage::http;
use anyhow::{Context, Result, bail};
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha512};
use std::collections::{HashMap, HashSet};
use std::ffi::OsStr;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

const API_BASE: &str = "https://api.modrinth.com/v2";
const MANIFEST_VERSION: u32 = 1;
const MAX_MOD_SIZE: u64 = 1_073_741_824;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResults {
    pub hits: Vec<SearchHit>,
    pub offset: usize,
    pub total_hits: usize,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchHit {
    pub project_id: String,
    pub title: String,
    pub description: String,
    pub author: String,
    pub icon_url: Option<String>,
    pub downloads: u64,
    pub categories: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledModView {
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

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MigrationPreview {
    pub from_version: String,
    pub to_version: String,
    pub loader: String,
    pub managed_mod_count: usize,
    pub changes: Vec<MigrationModChange>,
    pub dependency_count: usize,
    pub unavailable: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MigrationModChange {
    pub project_id: String,
    pub title: String,
    pub from_version: String,
    pub to_version: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct ContentManifest {
    #[serde(default = "manifest_version")]
    schema_version: u32,
    #[serde(default)]
    mods: Vec<InstalledMod>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct InstalledMod {
    project_id: String,
    version_id: String,
    title: String,
    version_number: String,
    #[serde(default)]
    version_type: ReleaseChannel,
    file_name: String,
    sha1: String,
    sha512: String,
    #[serde(default)]
    icon_url: Option<String>,
    #[serde(default)]
    explicit: bool,
    #[serde(default = "default_true")]
    enabled: bool,
    #[serde(default)]
    dependencies: Vec<String>,
    #[serde(default)]
    game_versions: Vec<String>,
    #[serde(default)]
    loaders: Vec<String>,
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
}

#[derive(Clone, Debug, Deserialize)]
struct ApiVersion {
    id: String,
    project_id: String,
    version_number: String,
    version_type: String,
    #[serde(default)]
    dependencies: Vec<ApiDependency>,
    #[serde(default)]
    game_versions: Vec<String>,
    #[serde(default)]
    loaders: Vec<String>,
    files: Vec<ApiFile>,
}

#[derive(Clone, Debug, Deserialize)]
struct ApiDependency {
    version_id: Option<String>,
    project_id: Option<String>,
    file_name: Option<String>,
    dependency_type: String,
}

#[derive(Clone, Debug, Deserialize)]
struct ApiFile {
    hashes: HashMap<String, String>,
    url: String,
    filename: String,
    primary: bool,
    size: u64,
    #[serde(default)]
    file_type: Option<String>,
}

#[derive(Serialize)]
struct UpdateRequest<'a> {
    hashes: Vec<&'a str>,
    algorithm: &'static str,
    loaders: [&'a str; 1],
    game_versions: [&'a str; 1],
}

fn manifest_version() -> u32 {
    MANIFEST_VERSION
}

fn default_true() -> bool {
    true
}

pub fn has_installed_mods(root: &Path, instance: &Instance) -> Result<bool> {
    Ok(!load_manifest(root, instance)?.mods.is_empty())
}

pub fn managed_mod_count(root: &Path, instance: &Instance) -> Result<usize> {
    Ok(load_manifest(root, instance)?.mods.len())
}

pub fn preview_migration(
    root: &Path,
    instance_id: &str,
    target_version: &str,
    target_loader: crate::modloaders::ModLoader,
) -> Result<MigrationPreview> {
    let instance = instances::load(root, instance_id)?;
    let loader = target_loader
        .modrinth_name()
        .context("Managed mods cannot be migrated to Vanilla")?;
    let manifest = load_manifest(root, &instance)?;
    let roots = manifest
        .mods
        .iter()
        .filter(|item| item.explicit)
        .collect::<Vec<_>>();
    let root_ids = roots
        .iter()
        .map(|item| item.project_id.as_str())
        .collect::<HashSet<_>>();
    let client = http()?;
    let mut resolved = HashMap::<String, ApiVersion>::new();
    let mut changes = Vec::new();
    let mut unavailable = unmanaged_mod_files(root, &instance, &manifest)?
        .into_iter()
        .map(|name| format!("{name}: unmanaged mod files cannot be migrated automatically"))
        .collect::<Vec<_>>();

    for installed in roots {
        let mut candidate = resolved.clone();
        let mut resolving = HashSet::new();
        match resolve_migration_project(
            &client,
            &installed.project_id,
            None,
            loader,
            target_version,
            ReleaseChannel::Release,
            &mut candidate,
            &mut resolving,
        ) {
            Ok(()) => {
                if let Some(version) = candidate.get(&installed.project_id) {
                    changes.push(MigrationModChange {
                        project_id: installed.project_id.clone(),
                        title: installed.title.clone(),
                        from_version: installed.version_number.clone(),
                        to_version: version.version_number.clone(),
                    });
                }
                resolved = candidate;
            }
            Err(error) => unavailable.push(format!("{}: {error:#}", installed.title)),
        }
    }
    changes.sort_by(|left, right| left.title.to_lowercase().cmp(&right.title.to_lowercase()));
    Ok(MigrationPreview {
        from_version: instance.version,
        to_version: target_version.to_owned(),
        loader: target_loader.pretty_name().to_owned(),
        managed_mod_count: manifest.mods.len(),
        changes,
        dependency_count: resolved
            .keys()
            .filter(|project_id| !root_ids.contains(project_id.as_str()))
            .count(),
        unavailable,
    })
}

pub fn search(
    root: &Path,
    instance_id: &str,
    query: &str,
    index: &str,
    category: Option<&str>,
    offset: usize,
) -> Result<SearchResults> {
    let instance = instances::load(root, instance_id)?;
    let loader = require_loader(&instance)?;
    let query = query.trim();
    if query.chars().count() > 100 {
        bail!("Search query is too long");
    }
    let index = validate_search_index(index)?;
    let mut facets = vec![
        vec![format!("categories:{loader}")],
        vec![format!("versions:{}", instance.version)],
        vec!["project_type:mod".to_owned()],
    ];
    if let Some(category) = category.map(str::trim).filter(|value| !value.is_empty()) {
        validate_category(category)?;
        facets.push(vec![format!("categories:{category}")]);
    }
    let facets = serde_json::to_string(&facets)?;
    let response: ApiSearchResults = http()?
        .get(format!("{API_BASE}/search"))
        .query(&[
            ("query", query.to_owned()),
            ("facets", facets),
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
                    .filter(|category| validate_category(category).is_ok())
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
    refresh_updates: bool,
) -> Result<Vec<InstalledModView>> {
    let instance = instances::load(root, instance_id)?;
    let loader = require_loader(&instance)?;
    let manifest = load_manifest(root, &instance)?;
    let updates = if refresh_updates {
        // Installed content must remain usable offline; update discovery is best-effort.
        fetch_updates(&manifest, loader, &instance.version).unwrap_or_default()
    } else {
        HashMap::new()
    };
    let mods_dir = mods_dir(root, &instance);
    let required_by = manifest
        .mods
        .iter()
        .flat_map(|item| item.dependencies.iter())
        .fold(HashMap::<&str, usize>::new(), |mut counts, project_id| {
            *counts.entry(project_id).or_default() += 1;
            counts
        });
    let mut result = manifest
        .mods
        .iter()
        .map(|installed| {
            let update = installed
                .explicit
                .then(|| updates.get(&installed.sha512))
                .flatten();
            InstalledModView {
                project_id: installed.project_id.clone(),
                title: installed.title.clone(),
                version_number: installed.version_number.clone(),
                version_type: installed.version_type,
                icon_url: safe_icon_url(installed.icon_url.clone()),
                explicit: installed.explicit,
                enabled: installed.enabled,
                compatible: installed
                    .game_versions
                    .iter()
                    .any(|version| version == &instance.version)
                    && installed.loaders.iter().any(|value| value == loader),
                missing: managed_file_is_missing_or_modified(&mods_dir, installed),
                file_name: installed.file_name.clone(),
                file_size: managed_file_path(&mods_dir, installed)
                    .ok()
                    .and_then(|path| path.metadata().ok())
                    .map(|metadata| metadata.len())
                    .unwrap_or(0),
                dependency_count: installed.dependencies.len(),
                required_by_count: required_by
                    .get(installed.project_id.as_str())
                    .copied()
                    .unwrap_or(0),
                update_available: update.is_some_and(|value| value.id != installed.version_id),
                latest_version_number: update
                    .filter(|value| value.id != installed.version_id)
                    .map(|value| value.version_number.clone()),
            }
        })
        .collect::<Vec<_>>();
    result.sort_by(|left, right| {
        right
            .explicit
            .cmp(&left.explicit)
            .then_with(|| left.title.to_lowercase().cmp(&right.title.to_lowercase()))
    });
    Ok(result)
}

pub fn install(
    root: &Path,
    instance_id: &str,
    project_id: &str,
    version_id: &str,
    report_progress: &(dyn Fn(f32) + Send + Sync),
) -> Result<Vec<InstalledModView>> {
    validate_project_id(project_id)?;
    let instance = instances::load(root, instance_id)?;
    let loader = require_loader(&instance)?;
    let client = http()?;
    let mut manifest = load_manifest(root, &instance)?;
    report_progress(0.0);
    let mut planned_versions = HashMap::new();
    let mut planning = HashSet::new();
    resolve_migration_project(
        &client,
        project_id,
        Some(version_id),
        loader,
        &instance.version,
        ReleaseChannel::Release,
        &mut planned_versions,
        &mut planning,
    )?;
    let total_bytes = planned_versions
        .values()
        .map(|version| select_file(version).map(|file| file.size))
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .sum::<u64>();
    let mut install_progress = ModInstallProgress::new(total_bytes, report_progress);
    let mut resolving = HashSet::new();
    let result = install_project(
        &client,
        root,
        &instance,
        loader,
        project_id,
        Some(version_id),
        true,
        &mut manifest,
        &mut resolving,
        Some(&planned_versions),
        Some(&mut install_progress),
        ReleaseChannel::Release,
    );
    if let Err(error) = result {
        garbage_collect(root, &instance, &mut manifest)?;
        save_manifest(root, &instance, &manifest)?;
        return Err(error);
    }
    save_manifest(root, &instance, &manifest)?;
    report_progress(1.0);
    list_installed(root, instance_id, true)
}

pub fn resolve_choice(
    root: &Path,
    instance_id: &str,
    project_id: &str,
    requested: ReleaseChannel,
) -> Result<VersionChoice> {
    validate_project_id(project_id)?;
    let instance = instances::load(root, instance_id)?;
    let loader = require_loader(&instance)?;
    let client = http()?;
    let project = fetch_project(&client, project_id)?;
    let versions = fetch_versions(&client, project_id, loader, &instance.version)?;
    let (version, channel, requires_confirmation) = modrinth_versions::choose(
        &versions,
        requested,
        requested == ReleaseChannel::Release,
        |version| version.version_type.as_str(),
        |version| {
            version.project_id == project_id
                && ensure_compatible(version, loader, &instance.version).is_ok()
                && select_file(version).is_ok()
        },
    )
    .with_context(|| unavailable_message(requested, loader, &instance.version))?;
    Ok(VersionChoice {
        project_id: project.id,
        title: project.title,
        version_id: version.id.clone(),
        version_number: version.version_number.clone(),
        version_type: channel,
        requires_confirmation,
    })
}

pub fn remove(root: &Path, instance_id: &str, project_id: &str) -> Result<Vec<InstalledModView>> {
    validate_project_id(project_id)?;
    let instance = instances::load(root, instance_id)?;
    require_loader(&instance)?;
    let mut manifest = load_manifest(root, &instance)?;
    let Some(item) = manifest
        .mods
        .iter_mut()
        .find(|item| item.project_id == project_id)
    else {
        bail!("Mod is not installed");
    };
    item.explicit = false;
    garbage_collect(root, &instance, &mut manifest)?;
    save_manifest(root, &instance, &manifest)?;
    list_installed(root, instance_id, true)
}

pub fn set_enabled(
    root: &Path,
    instance_id: &str,
    project_id: &str,
    enabled: bool,
) -> Result<Vec<InstalledModView>> {
    validate_project_id(project_id)?;
    let instance = instances::load(root, instance_id)?;
    require_loader(&instance)?;
    let directory = mods_dir(root, &instance);
    let mut manifest = load_manifest(root, &instance)?;
    let item = manifest
        .mods
        .iter_mut()
        .find(|item| item.project_id == project_id)
        .context("Mod is not installed")?;
    if item.enabled == enabled {
        return list_installed(root, instance_id, false);
    }
    let source = managed_file_path(&directory, item)?;
    if !source.is_file() {
        bail!("The managed mod file is missing; update the mod before changing its state");
    }
    let destination = managed_path_for(&directory, &item.file_name, enabled)?;
    if destination.exists() {
        bail!("A mod file with the target name already exists");
    }
    fs::rename(&source, &destination).context("Could not change the mod state")?;
    item.enabled = enabled;
    save_manifest(root, &instance, &manifest)?;
    list_installed(root, instance_id, false)
}

pub fn update(
    root: &Path,
    instance_id: &str,
    project_id: &str,
    version_id: &str,
) -> Result<Vec<InstalledModView>> {
    validate_project_id(project_id)?;
    let instance = instances::load(root, instance_id)?;
    let loader = require_loader(&instance)?;
    let client = http()?;
    let mut manifest = load_manifest(root, &instance)?;
    let installed = manifest
        .mods
        .iter()
        .find(|item| item.project_id == project_id)
        .context("Mod is not installed")?;
    let explicit = installed.explicit;
    let mut resolving = HashSet::new();
    install_project(
        &client,
        root,
        &instance,
        loader,
        project_id,
        Some(version_id),
        explicit,
        &mut manifest,
        &mut resolving,
        None,
        None,
        ReleaseChannel::Release,
    )?;
    garbage_collect(root, &instance, &mut manifest)?;
    save_manifest(root, &instance, &manifest)?;
    list_installed(root, instance_id, true)
}

pub fn update_all(root: &Path, instance_id: &str) -> Result<Vec<InstalledModView>> {
    let instance = instances::load(root, instance_id)?;
    let loader = require_loader(&instance)?;
    let client = http()?;
    let mut manifest = load_manifest(root, &instance)?;
    let projects = manifest
        .mods
        .iter()
        .filter(|item| item.explicit)
        .map(|item| item.project_id.clone())
        .collect::<Vec<_>>();
    for project_id in projects {
        let mut resolving = HashSet::new();
        install_project(
            &client,
            root,
            &instance,
            loader,
            &project_id,
            None,
            true,
            &mut manifest,
            &mut resolving,
            None,
            None,
            ReleaseChannel::Release,
        )?;
    }
    garbage_collect(root, &instance, &mut manifest)?;
    save_manifest(root, &instance, &manifest)?;
    list_installed(root, instance_id, true)
}

#[allow(clippy::too_many_arguments)]
fn install_project(
    client: &Client,
    root: &Path,
    instance: &Instance,
    loader: &str,
    project_id: &str,
    exact_version: Option<&str>,
    explicit: bool,
    manifest: &mut ContentManifest,
    resolving: &mut HashSet<String>,
    planned_versions: Option<&HashMap<String, ApiVersion>>,
    mut install_progress: Option<&mut ModInstallProgress<'_>>,
    allowed_channel: ReleaseChannel,
) -> Result<()> {
    validate_project_id(project_id)?;
    if !resolving.insert(project_id.to_owned()) {
        return Ok(());
    }

    let version = match planned_versions.and_then(|versions| versions.get(project_id)) {
        Some(version) => version.clone(),
        None => fetch_version_for_channel(
            client,
            project_id,
            exact_version,
            loader,
            &instance.version,
            allowed_channel,
        )?,
    };
    if version.project_id != project_id {
        bail!("Modrinth returned a version for the wrong project");
    }
    ensure_compatible(&version, loader, &instance.version)?;
    let dependency_channel =
        ReleaseChannel::from_api(&version.version_type).unwrap_or(allowed_channel);
    let project = fetch_project(client, project_id)?;
    let mut dependencies = Vec::new();
    for dependency in version
        .dependencies
        .iter()
        .filter(|dependency| dependency.dependency_type == "required")
    {
        let (dependency_project, dependency_version) = match (
            dependency.project_id.as_deref(),
            dependency.version_id.as_deref(),
        ) {
            (Some(project), version) => (project.to_owned(), version.map(str::to_owned)),
            (None, Some(version_id)) => {
                let dependency_version = fetch_version(client, version_id)?;
                (dependency_version.project_id, Some(version_id.to_owned()))
            }
            (None, None) => {
                bail!(
                    "{} requires an external dependency that cannot be downloaded automatically{}",
                    project.title,
                    dependency
                        .file_name
                        .as_deref()
                        .map(|name| format!(": {name}"))
                        .unwrap_or_default()
                )
            }
        };
        validate_project_id(&dependency_project)?;
        dependencies.push(dependency_project.clone());
        install_project(
            client,
            root,
            instance,
            loader,
            &dependency_project,
            dependency_version.as_deref(),
            false,
            manifest,
            resolving,
            planned_versions,
            install_progress.as_deref_mut(),
            dependency_channel,
        )?;
    }

    let file = select_file(&version)?;
    let sha1 = file
        .hashes
        .get("sha1")
        .cloned()
        .context("Modrinth file has no SHA-1 hash")?;
    let sha512 = file
        .hashes
        .get("sha512")
        .cloned()
        .context("Modrinth file has no SHA-512 hash")?;
    let file_name = unique_file_name(manifest, project_id, &safe_jar_name(&file.filename)?);
    let directory = mods_dir(root, instance);
    fs::create_dir_all(&directory)?;
    let previous = manifest
        .mods
        .iter()
        .find(|item| item.project_id == project_id)
        .cloned();
    let enabled = previous.as_ref().is_none_or(|item| item.enabled);
    let destination = managed_path_for(&directory, &file_name, enabled)?;
    if !destination.is_file() || sha512_file(&destination)? != sha512 {
        download_mod(client, file, &destination, &sha512, |received| {
            if let Some(progress) = install_progress.as_deref_mut() {
                progress.update_current(project_id, received, file.size);
            }
        })?;
    }
    if let Some(progress) = install_progress.as_deref_mut() {
        progress.finish_file(project_id, file.size);
    }

    let installed = InstalledMod {
        project_id: project.id,
        version_id: version.id,
        title: project.title,
        version_number: version.version_number,
        version_type: ReleaseChannel::from_api(&version.version_type).unwrap_or_default(),
        file_name: file_name.clone(),
        sha1,
        sha512,
        icon_url: safe_icon_url(project.icon_url),
        explicit: previous.as_ref().is_some_and(|item| item.explicit) || explicit,
        enabled,
        dependencies,
        game_versions: version.game_versions,
        loaders: version.loaders,
    };
    if let Some(index) = manifest
        .mods
        .iter()
        .position(|item| item.project_id == project_id)
    {
        manifest.mods[index] = installed;
    } else {
        manifest.mods.push(installed);
    }
    if let Some(previous) = previous {
        let previous_path = managed_file_path(&directory, &previous)?;
        if previous_path != destination {
            remove_managed_file(&directory, &previous)?;
        }
    }
    save_manifest(root, instance, manifest)?;
    resolving.remove(project_id);
    Ok(())
}

fn fetch_project(client: &Client, project_id: &str) -> Result<ApiProject> {
    Ok(client
        .get(format!("{API_BASE}/project/{project_id}"))
        .send()?
        .error_for_status()?
        .json()?)
}

fn resolve_migration_project(
    client: &Client,
    project_id: &str,
    exact_version: Option<&str>,
    loader: &str,
    game_version: &str,
    allowed_channel: ReleaseChannel,
    resolved: &mut HashMap<String, ApiVersion>,
    resolving: &mut HashSet<String>,
) -> Result<()> {
    validate_project_id(project_id)?;
    if let Some(existing) = resolved.get(project_id) {
        if exact_version.is_some_and(|required| required != existing.id) {
            bail!("Conflicting required versions were found for dependency {project_id}");
        }
        return Ok(());
    }
    if !resolving.insert(project_id.to_owned()) {
        return Ok(());
    }
    let version = fetch_version_for_channel(
        client,
        project_id,
        exact_version,
        loader,
        game_version,
        allowed_channel,
    )?;
    if version.project_id != project_id {
        bail!("Modrinth returned a version for the wrong project");
    }
    ensure_compatible(&version, loader, game_version)?;
    let dependency_channel =
        ReleaseChannel::from_api(&version.version_type).unwrap_or(allowed_channel);
    select_file(&version)?;
    for dependency in version
        .dependencies
        .iter()
        .filter(|dependency| dependency.dependency_type == "required")
    {
        let (dependency_project, dependency_version) = match (
            dependency.project_id.as_deref(),
            dependency.version_id.as_deref(),
        ) {
            (Some(project), version) => (project.to_owned(), version.map(str::to_owned)),
            (None, Some(version_id)) => {
                let dependency_version = fetch_version(client, version_id)?;
                (dependency_version.project_id, Some(version_id.to_owned()))
            }
            (None, None) => {
                bail!(
                    "A required external dependency cannot be downloaded automatically{}",
                    dependency
                        .file_name
                        .as_deref()
                        .map(|name| format!(": {name}"))
                        .unwrap_or_default()
                )
            }
        };
        resolve_migration_project(
            client,
            &dependency_project,
            dependency_version.as_deref(),
            loader,
            game_version,
            dependency_channel,
            resolved,
            resolving,
        )?;
    }
    resolving.remove(project_id);
    resolved.insert(project_id.to_owned(), version);
    Ok(())
}

fn fetch_version(client: &Client, version_id: &str) -> Result<ApiVersion> {
    validate_project_id(version_id)?;
    Ok(client
        .get(format!("{API_BASE}/version/{version_id}"))
        .send()?
        .error_for_status()?
        .json()?)
}

fn fetch_version_for_channel(
    client: &Client,
    project_id: &str,
    exact_version: Option<&str>,
    loader: &str,
    game_version: &str,
    allowed_channel: ReleaseChannel,
) -> Result<ApiVersion> {
    if let Some(version_id) = exact_version {
        let version = fetch_version(client, version_id)?;
        if version.project_id != project_id {
            bail!("Modrinth returned a version for the wrong project");
        }
        return Ok(version);
    }
    fetch_latest_version(client, project_id, loader, game_version, allowed_channel)
}

fn fetch_latest_version(
    client: &Client,
    project_id: &str,
    loader: &str,
    game_version: &str,
    allowed_channel: ReleaseChannel,
) -> Result<ApiVersion> {
    let versions = fetch_versions(client, project_id, loader, game_version)?;
    let channels = match allowed_channel {
        ReleaseChannel::Release => &[ReleaseChannel::Release][..],
        ReleaseChannel::Beta => &[ReleaseChannel::Release, ReleaseChannel::Beta][..],
        ReleaseChannel::Alpha => &[
            ReleaseChannel::Release,
            ReleaseChannel::Beta,
            ReleaseChannel::Alpha,
        ][..],
    };
    channels
        .iter()
        .find_map(|channel| {
            versions.iter().find(|version| {
                version.project_id == project_id
                    && version.version_type == channel.as_str()
                    && ensure_compatible(version, loader, game_version).is_ok()
                    && select_file(version).is_ok()
            })
        })
        .cloned()
        .with_context(|| unavailable_message(allowed_channel, loader, game_version))
}

fn fetch_versions(
    client: &Client,
    project_id: &str,
    loader: &str,
    game_version: &str,
) -> Result<Vec<ApiVersion>> {
    Ok(client
        .get(format!("{API_BASE}/project/{project_id}/version"))
        .query(&[
            ("loaders", serde_json::to_string(&[loader])?),
            ("game_versions", serde_json::to_string(&[game_version])?),
            ("include_changelog", "false".to_owned()),
        ])
        .send()?
        .error_for_status()?
        .json()?)
}

fn unavailable_message(channel: ReleaseChannel, loader: &str, game_version: &str) -> String {
    match channel {
        ReleaseChannel::Release => format!(
            "No compatible release, beta, or alpha is available for {loader} on Minecraft {game_version}"
        ),
        ReleaseChannel::Beta => {
            format!("No compatible beta is available for {loader} on Minecraft {game_version}")
        }
        ReleaseChannel::Alpha => {
            format!("No compatible alpha is available for {loader} on Minecraft {game_version}")
        }
    }
}

fn fetch_updates(
    manifest: &ContentManifest,
    loader: &str,
    game_version: &str,
) -> Result<HashMap<String, ApiVersion>> {
    let hashes = manifest
        .mods
        .iter()
        .filter(|item| item.explicit && !item.sha512.is_empty())
        .filter(|item| item.version_type == ReleaseChannel::Release)
        .map(|item| item.sha512.as_str())
        .collect::<Vec<_>>();
    if hashes.is_empty() {
        return Ok(HashMap::new());
    }
    let client = http()?;
    let response: HashMap<String, Option<ApiVersion>> = client
        .post(format!("{API_BASE}/version_files/update"))
        .json(&UpdateRequest {
            hashes,
            algorithm: "sha512",
            loaders: [loader],
            game_versions: [game_version],
        })
        .send()?
        .error_for_status()?
        .json()?;
    let mut updates = HashMap::new();
    for item in manifest.mods.iter().filter(|item| item.explicit) {
        let Some(candidate) = response.get(&item.sha512).and_then(Option::as_ref) else {
            continue;
        };
        let release = if candidate.version_type == ReleaseChannel::Release.as_str() {
            Some(candidate.clone())
        } else {
            fetch_latest_version(
                &client,
                &item.project_id,
                loader,
                game_version,
                ReleaseChannel::Release,
            )
            .ok()
        };
        if let Some(release) = release {
            updates.insert(item.sha512.clone(), release);
        }
    }
    Ok(updates)
}

fn ensure_compatible(version: &ApiVersion, loader: &str, game_version: &str) -> Result<()> {
    if !version.loaders.iter().any(|value| value == loader)
        || !version
            .game_versions
            .iter()
            .any(|value| value == game_version)
    {
        bail!(
            "The selected mod version is not compatible with {loader} on Minecraft {game_version}"
        );
    }
    Ok(())
}

fn select_file(version: &ApiVersion) -> Result<&ApiFile> {
    version
        .files
        .iter()
        .find(|file| file.primary && is_installable_jar(file))
        .or_else(|| version.files.iter().find(|file| is_installable_jar(file)))
        .context("The compatible Modrinth version has no installable JAR file")
}

fn is_installable_jar(file: &ApiFile) -> bool {
    file.filename.to_ascii_lowercase().ends_with(".jar")
        && !matches!(
            file.file_type.as_deref(),
            Some("sources-jar" | "dev-jar" | "javadoc-jar" | "signature")
        )
}

fn download_mod(
    client: &Client,
    file: &ApiFile,
    destination: &Path,
    expected: &str,
    mut on_progress: impl FnMut(u64),
) -> Result<()> {
    if file.size > MAX_MOD_SIZE {
        bail!("Mod file is unexpectedly large");
    }
    let url = reqwest::Url::parse(&file.url)?;
    if url.scheme() != "https" {
        bail!("Mod download must use HTTPS");
    }
    let mut response = client.get(url).send()?.error_for_status()?;
    if response
        .content_length()
        .is_some_and(|size| size > MAX_MOD_SIZE)
    {
        bail!("Mod download is unexpectedly large");
    }
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
        on_progress(received);
        if received > MAX_MOD_SIZE {
            let _ = fs::remove_file(&temporary);
            bail!("Mod download exceeded the size limit");
        }
        output.write_all(&buffer[..count])?;
        hasher.update(&buffer[..count]);
    }
    output.flush()?;
    let actual = format!("{:x}", hasher.finalize());
    if !actual.eq_ignore_ascii_case(expected) {
        fs::remove_file(&temporary)?;
        bail!("Downloaded mod checksum does not match");
    }
    if destination.exists() {
        fs::remove_file(destination)?;
    }
    fs::rename(temporary, destination)?;
    Ok(())
}

struct ModInstallProgress<'a> {
    total: u64,
    completed: u64,
    current: u64,
    finished: HashSet<String>,
    report: &'a (dyn Fn(f32) + Send + Sync),
}

impl<'a> ModInstallProgress<'a> {
    fn new(total: u64, report: &'a (dyn Fn(f32) + Send + Sync)) -> Self {
        Self {
            total: total.max(1),
            completed: 0,
            current: 0,
            finished: HashSet::new(),
            report,
        }
    }

    fn update_current(&mut self, project_id: &str, received: u64, file_size: u64) {
        if self.finished.contains(project_id) {
            return;
        }
        self.current = received.min(file_size);
        self.emit();
    }

    fn finish_file(&mut self, project_id: &str, file_size: u64) {
        if !self.finished.insert(project_id.to_owned()) {
            return;
        }
        self.completed = self.completed.saturating_add(file_size);
        self.current = 0;
        self.emit();
    }

    fn emit(&self) {
        ((self.report)(
            ((self.completed + self.current) as f64 / self.total as f64).clamp(0.0, 1.0) as f32,
        ));
    }
}

fn garbage_collect(root: &Path, instance: &Instance, manifest: &mut ContentManifest) -> Result<()> {
    loop {
        let referenced = manifest
            .mods
            .iter()
            .flat_map(|item| item.dependencies.iter().cloned())
            .collect::<HashSet<_>>();
        let removable = manifest
            .mods
            .iter()
            .filter(|item| !item.explicit && !referenced.contains(&item.project_id))
            .map(|item| item.project_id.clone())
            .collect::<HashSet<_>>();
        if removable.is_empty() {
            break;
        }
        let directory = mods_dir(root, instance);
        for item in manifest
            .mods
            .iter()
            .filter(|item| removable.contains(&item.project_id))
        {
            remove_managed_file(&directory, item)?;
        }
        manifest
            .mods
            .retain(|item| !removable.contains(&item.project_id));
    }
    Ok(())
}

fn remove_managed_file(directory: &Path, installed: &InstalledMod) -> Result<()> {
    let path = managed_file_path(directory, installed)?;
    if path.parent() != Some(directory) {
        bail!("Invalid managed mod path");
    }
    if path.exists() {
        fs::remove_file(path)?;
    }
    Ok(())
}

fn load_manifest(root: &Path, instance: &Instance) -> Result<ContentManifest> {
    let path = manifest_path(root, instance);
    let manifest = match fs::read(&path) {
        Ok(bytes) => serde_json::from_slice::<ContentManifest>(&bytes)
            .context("The managed mod list is corrupted")?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => ContentManifest {
            schema_version: MANIFEST_VERSION,
            mods: Vec::new(),
        },
        Err(error) => return Err(error.into()),
    };
    if manifest.schema_version != MANIFEST_VERSION {
        bail!("The managed mod list uses an unsupported format");
    }
    Ok(manifest)
}

fn save_manifest(root: &Path, instance: &Instance, manifest: &ContentManifest) -> Result<()> {
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
        .join("mods.json")
}

fn mods_dir(root: &Path, instance: &Instance) -> PathBuf {
    instances::game_dir(root, instance).join("mods")
}

fn unmanaged_mod_files(
    root: &Path,
    instance: &Instance,
    manifest: &ContentManifest,
) -> Result<Vec<String>> {
    let directory = mods_dir(root, instance);
    if !directory.is_dir() {
        return Ok(Vec::new());
    }
    let managed = manifest
        .mods
        .iter()
        .filter_map(|item| {
            managed_file_path(&directory, item)
                .ok()?
                .file_name()?
                .to_str()
                .map(str::to_owned)
        })
        .collect::<HashSet<_>>();
    let mut unmanaged = Vec::new();
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        let lower = name.to_ascii_lowercase();
        if (lower.ends_with(".jar") || lower.ends_with(".jar.disabled")) && !managed.contains(&name)
        {
            unmanaged.push(name);
        }
    }
    unmanaged.sort_by_key(|name| name.to_lowercase());
    Ok(unmanaged)
}

fn require_loader(instance: &Instance) -> Result<&'static str> {
    instance
        .loader
        .modrinth_name()
        .context("Select a mod loader before managing mods")
}

fn validate_project_id(value: &str) -> Result<()> {
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

fn validate_search_index(value: &str) -> Result<&'static str> {
    match value {
        "relevance" => Ok("relevance"),
        "downloads" => Ok("downloads"),
        "follows" => Ok("follows"),
        "newest" => Ok("newest"),
        "updated" => Ok("updated"),
        _ => bail!("Invalid Modrinth sort option"),
    }
}

fn validate_category(value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 48
        || !value
            .chars()
            .all(|character| character.is_ascii_lowercase() || character == '-')
    {
        bail!("Invalid Modrinth category");
    }
    Ok(())
}

fn safe_jar_name(value: &str) -> Result<String> {
    let path = Path::new(value);
    if path.file_name() != Some(OsStr::new(value))
        || !value.to_ascii_lowercase().ends_with(".jar")
        || value.chars().any(char::is_control)
    {
        bail!("Modrinth returned an unsafe mod filename");
    }
    Ok(value.to_owned())
}

fn managed_path_for(directory: &Path, file_name: &str, enabled: bool) -> Result<PathBuf> {
    let safe = safe_jar_name(file_name)?;
    let name = if enabled {
        safe
    } else {
        format!("{safe}.disabled")
    };
    let path = directory.join(name);
    if path.parent() != Some(directory) {
        bail!("Invalid managed mod path");
    }
    Ok(path)
}

fn managed_file_path(directory: &Path, installed: &InstalledMod) -> Result<PathBuf> {
    managed_path_for(directory, &installed.file_name, installed.enabled)
}

fn unique_file_name(manifest: &ContentManifest, project_id: &str, file_name: &str) -> String {
    if manifest
        .mods
        .iter()
        .all(|item| item.project_id == project_id || item.file_name != file_name)
    {
        file_name.to_owned()
    } else {
        format!("{project_id}-{file_name}")
    }
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

fn managed_file_is_missing_or_modified(directory: &Path, installed: &InstalledMod) -> bool {
    managed_file_path(directory, installed)
        .and_then(|path| {
            if !path.is_file() {
                return Ok(true);
            }
            sha512_file(&path).map(|actual| !actual.eq_ignore_ascii_case(&installed.sha512))
        })
        .unwrap_or(true)
}

fn sha512_file(path: &Path) -> Result<String> {
    let mut input = File::open(path)?;
    let mut hasher = Sha512::new();
    let mut buffer = [0u8; 131_072];
    loop {
        let count = input.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_unsafe_mod_names() {
        assert!(safe_jar_name("sodium.jar").is_ok());
        assert!(safe_jar_name("../sodium.jar").is_err());
        assert!(safe_jar_name("readme.txt").is_err());
    }

    #[test]
    fn accepts_only_stable_project_ids() {
        assert!(validate_project_id("AABBCCDD").is_ok());
        assert!(validate_project_id("bad/slug").is_err());
    }

    #[test]
    fn ignores_non_runtime_jars() {
        let file = ApiFile {
            hashes: HashMap::new(),
            url: "https://cdn.modrinth.com/example.jar".to_owned(),
            filename: "example-sources.jar".to_owned(),
            primary: false,
            size: 1,
            file_type: Some("sources-jar".to_owned()),
        };
        assert!(!is_installable_jar(&file));
    }

    #[test]
    fn old_manifests_keep_mods_enabled() {
        let installed: InstalledMod = serde_json::from_value(serde_json::json!({
            "projectId": "AABBCCDD",
            "versionId": "EEFFGGHH",
            "title": "Example",
            "versionNumber": "1.0.0",
            "fileName": "example.jar",
            "sha1": "abc",
            "sha512": "def"
        }))
        .unwrap();
        assert!(installed.enabled);
        assert!(
            managed_file_path(Path::new("mods"), &installed)
                .unwrap()
                .ends_with("example.jar")
        );
        let mut disabled = installed;
        disabled.enabled = false;
        assert!(
            managed_file_path(Path::new("mods"), &disabled)
                .unwrap()
                .ends_with("example.jar.disabled")
        );
    }

    #[test]
    fn validates_search_filters() {
        assert_eq!(validate_search_index("downloads").unwrap(), "downloads");
        assert!(validate_search_index("popular").is_err());
        assert!(validate_category("game-mechanics").is_ok());
        assert!(validate_category("../mods").is_err());
    }
}
