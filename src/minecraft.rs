use crate::storage::{AuthState, http, read_json};
use anyhow::{Context, Result, bail};
use reqwest::blocking::Client;
use serde::Deserialize;
use sha1::{Digest, Sha1};
use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
#[cfg(windows)]
use std::os::windows::process::CommandExt;

const MANIFEST_URL: &str = "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json";
pub type ProgressReporter = dyn Fn(f32, String) + Send + Sync;

#[derive(Clone, Debug, Deserialize)]
pub struct VersionManifest { pub latest: LatestVersions, pub versions: Vec<ManifestVersion> }
#[derive(Clone, Debug, Deserialize)]
pub struct LatestVersions { pub release: String }
#[derive(Clone, Debug, Deserialize)]
pub struct ManifestVersion { pub id: String, pub url: String, #[serde(rename = "type")] pub kind: String }
#[derive(Debug, Deserialize)]
struct VersionMeta { id: String, #[serde(rename = "mainClass")] main_class: String, downloads: VersionDownloads, #[serde(rename = "assetIndex")] asset_index: Option<Download>, #[serde(rename = "javaVersion")] java_version: Option<JavaVersion>, libraries: Vec<Library>, arguments: Option<LaunchArguments>, #[serde(rename = "minecraftArguments")] minecraft_arguments: Option<String> }
#[derive(Debug, Deserialize)]
struct JavaVersion { #[serde(rename = "majorVersion")] major_version: u32 }
#[derive(Debug, Deserialize)]
struct VersionDownloads { client: Download }
#[derive(Clone, Debug, Deserialize)]
struct Download { url: String, path: Option<String>, sha1: Option<String> }
#[derive(Debug, Deserialize)]
struct Library { downloads: Option<LibraryDownloads>, rules: Option<Vec<Rule>>, natives: Option<HashMap<String, String>>, extract: Option<Extract> }
#[derive(Debug, Deserialize)]
struct LibraryDownloads { artifact: Option<Download>, classifiers: Option<HashMap<String, Download>> }
#[derive(Clone, Debug, Deserialize)]
struct Extract { exclude: Option<Vec<String>> }
#[derive(Debug, Deserialize)]
struct LaunchArguments { game: Vec<Argument>, jvm: Vec<Argument> }
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum Argument { Plain(String), Conditional { rules: Option<Vec<Rule>>, value: ArgumentValue } }
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum ArgumentValue { One(String), Many(Vec<String>) }
#[derive(Debug, Deserialize)]
struct Rule { action: String, os: Option<RuleOs>, features: Option<HashMap<String, bool>> }
#[derive(Debug, Deserialize)]
struct RuleOs { name: Option<String> }
#[derive(Debug, Deserialize)]
struct AssetIndex { objects: HashMap<String, AssetObject> }
#[derive(Debug, Deserialize)]
struct AssetObject { hash: String }

pub fn load_versions() -> Result<(VersionManifest, Vec<ManifestVersion>)> {
    let manifest: VersionManifest = http()?.get(MANIFEST_URL).send()?.error_for_status()?.json()?;
    let mut list: Vec<_> = manifest.versions.iter().filter(|v| v.kind == "release" || v.kind == "snapshot").cloned().collect();
    list.sort_by_key(|v| if v.kind == "release" { 0 } else { 1 });
    Ok((manifest, list))
}

pub fn install_and_launch(root: &Path, game_dir: &Path, entry: &ManifestVersion, auth: &AuthState, open_console: bool, report: &(dyn Fn(String) + Send + Sync), progress: &ProgressReporter) -> Result<()> {
    let client = http()?;
    report(format!("Loading Minecraft {} metadata …", entry.id));
    let meta: VersionMeta = client.get(&entry.url).send()?.error_for_status()?.json()?;
    let java = crate::runtime::ensure_java(root, meta.java_version.as_ref().map(|version| version.major_version).unwrap_or(8), progress)?;
    let version_dir = root.join("versions").join(&meta.id);
    fs::create_dir_all(&version_dir)?;
    let client_jar = version_dir.join(format!("{}.jar", meta.id));
    download_file(&client, &meta.downloads.client, &client_jar, "Minecraft client", progress)?;

    report("Downloading libraries and Windows components …".into());
    let mut classpath = vec![client_jar];
    let natives_dir = root.join("natives").join(&meta.id);
    fs::create_dir_all(&natives_dir)?;
    let mut libraries = Vec::new();
    let mut native_archives = Vec::new();
    let mut seen_libraries = HashSet::new();
    let mut seen_natives = HashSet::new();
    for library in &meta.libraries {
        if !rules_allow(library.rules.as_deref()) { continue; }
        let Some(downloads) = &library.downloads else { continue; };
        if let Some(artifact) = &downloads.artifact {
            let path = library_path(root, artifact)?;
            if seen_libraries.insert(path.clone()) {
                libraries.push((artifact.clone(), path.clone()));
                classpath.push(path);
            }
        }
        if let Some(classifier) = native_classifier(library) {
            if let Some(native) = downloads.classifiers.as_ref().and_then(|values| values.get(&classifier)) {
                let path = library_path(root, native)?;
                if seen_natives.insert(path.clone()) {
                    native_archives.push((native.clone(), path, library.extract.clone()));
                }
            }
        }
    }
    download_batch(&client, libraries, "libraries", progress)?;
    let native_downloads = native_archives.iter().map(|(download, path, _)| (download.clone(), path.clone())).collect();
    download_batch(&client, native_downloads, "Windows components", progress)?;
    for (_, archive, extract) in native_archives { extract_native(&archive, &natives_dir, extract.as_ref())?; }

    let assets_root = root.join("assets");
    let assets_index_name = if let Some(asset_index) = &meta.asset_index {
        report("Downloading game assets …".into());
        let index_path = assets_root.join("indexes").join(format!("{}.json", asset_index.path.clone().unwrap_or_else(|| asset_index.url.rsplit('/').next().unwrap_or("index.json").to_owned())));
        download_file(&client, asset_index, &index_path, "Asset index", progress)?;
        let index: AssetIndex = read_json(&index_path)?;
        let mut seen_hashes = HashSet::new();
        let mut missing_assets = Vec::new();
        for asset in index.objects.values() {
            if !seen_hashes.insert(asset.hash.clone()) { continue; }
            let path = assets_root.join("objects").join(&asset.hash[0..2]).join(&asset.hash);
            if !path.exists() {
                missing_assets.push((
                    Download { url: format!("https://resources.download.minecraft.net/{}/{}", &asset.hash[0..2], asset.hash), path: None, sha1: Some(asset.hash.clone()) },
                    path,
                ));
            }
        }
        download_batch(&client, missing_assets, "assets", progress)?;
        meta.asset_index.as_ref().and_then(|d| d.path.clone()).unwrap_or_else(|| index_path.file_stem().unwrap_or_default().to_string_lossy().to_string())
    } else { String::new() };

    report("Starting Java …".into());
    fs::create_dir_all(game_dir)?;
    let classpath_text = std::env::join_paths(&classpath)?.to_string_lossy().to_string();
    let substitutions = HashMap::from([
        ("${auth_player_name}", auth.player_name.clone()), ("${version_name}", meta.id.clone()), ("${game_directory}", game_dir.to_string_lossy().to_string()), ("${assets_root}", assets_root.to_string_lossy().to_string()),
        ("${assets_index_name}", assets_index_name), ("${auth_uuid}", auth.player_uuid.clone()), ("${auth_access_token}", auth.minecraft_access_token.clone()), ("${user_type}", "msa".into()),
        ("${version_type}", "Wisdom".into()), ("${natives_directory}", natives_dir.to_string_lossy().to_string()), ("${classpath}", classpath_text), ("${classpath_separator}", ";".into()),
        ("${launcher_name}", "Wisdom".into()), ("${launcher_version}", env!("CARGO_PKG_VERSION").into()),
    ]);
    let (mut jvm_args, game_args) = build_arguments(&meta, &substitutions)?;
    if !jvm_args.iter().any(|arg| arg == "-cp" || arg == "-classpath") { jvm_args.extend(["-cp".to_owned(), substitutions["${classpath}"].clone()]); }
    let mut game = Command::new(java);
    game.args(&jvm_args).arg(&meta.main_class).args(&game_args).current_dir(game_dir);
    #[cfg(windows)]
    game.creation_flags(if open_console { 0x0000_0010 } else { 0x0800_0000 });
    game.spawn().context("Could not start Java. Install Java 21+ or set JAVA_HOME")?;
    Ok(())
}

fn download_file(client: &Client, source: &Download, destination: &Path, label: &str, progress: &ProgressReporter) -> Result<()> {
    download_to_disk(client, source, destination, |received, total| {
        let amount = if total == 0 { 0.0 } else { received as f32 / total as f32 };
        progress(amount, format!("Downloading {label} · {}%", (amount * 100.0) as u32));
    })
}

fn download_to_disk(client: &Client, source: &Download, destination: &Path, mut on_progress: impl FnMut(u64, u64)) -> Result<()> {
    if destination.exists() && source.sha1.as_ref().is_none_or(|hash| sha1_file(destination).is_ok_and(|actual| actual.eq_ignore_ascii_case(hash))) { return Ok(()); }
    fs::create_dir_all(destination.parent().context("Download target has no parent directory")?)?;
    let mut response = client.get(&source.url).send()?.error_for_status()?;
    let total = response.content_length().unwrap_or(0);
    let temporary = destination.with_extension("download");
    let mut output = File::create(&temporary)?;
    let mut hasher = Sha1::new();
    let mut received = 0u64;
    let mut buffer = [0u8; 131_072];
    loop {
        let count = response.read(&mut buffer)?;
        if count == 0 { break; }
        output.write_all(&buffer[..count])?;
        hasher.update(&buffer[..count]);
        received += count as u64;
        on_progress(received, total);
    }
    if let Some(expected) = &source.sha1 {
        let actual = format!("{:x}", hasher.finalize());
        if !actual.eq_ignore_ascii_case(expected) { fs::remove_file(&temporary)?; bail!("Checksum mismatch for {}", source.url); }
    }
    fs::rename(temporary, destination)?;
    Ok(())
}

fn download_batch(client: &Client, tasks: Vec<(Download, PathBuf)>, category: &str, progress: &ProgressReporter) -> Result<()> {
    if tasks.is_empty() {
        progress(1.0, format!("All {category} are ready."));
        return Ok(());
    }

    let total = tasks.len();
    progress(0.0, format!("Downloading {category} · 0/{total}"));
    let (sender, receiver) = mpsc::channel();
    for task in tasks { sender.send(task).expect("download queue should be open"); }
    drop(sender);

    let queue = Arc::new(Mutex::new(receiver));
    let completed = Arc::new(AtomicUsize::new(0));
    let failure = Arc::new(Mutex::new(None::<String>));

    thread::scope(|scope| {
        for _ in 0..total.min(8) {
            let client = client.clone();
            let queue = Arc::clone(&queue);
            let completed = Arc::clone(&completed);
            let failure = Arc::clone(&failure);
            scope.spawn(move || loop {
                if failure.lock().ok().and_then(|error| error.clone()).is_some() { break; }
                let task = queue.lock().ok().and_then(|queue| queue.recv().ok());
                let Some((source, destination)) = task else { break; };
                if let Err(error) = download_to_disk(&client, &source, &destination, |_, _| {}) {
                    if let Ok(mut stored) = failure.lock() { *stored = Some(format!("Could not download {}: {error:#}", source.url)); }
                    break;
                }
                let count = completed.fetch_add(1, Ordering::Relaxed) + 1;
                progress(count as f32 / total as f32, format!("Downloading {category} · {count}/{total}"));
            });
        }
    });

    if let Some(error) = failure.lock().ok().and_then(|error| error.clone()) { bail!(error); }
    Ok(())
}

fn sha1_file(path: &Path) -> Result<String> {
    let mut file = File::open(path)?; let mut hasher = Sha1::new(); let mut buffer = [0u8; 8192];
    loop { let read = file.read(&mut buffer)?; if read == 0 { break; } hasher.update(&buffer[..read]); }
    Ok(format!("{:x}", hasher.finalize()))
}
fn library_path(root: &Path, download: &Download) -> Result<PathBuf> { Ok(root.join("libraries").join(download.path.as_ref().context("Library has no path")?)) }
fn native_classifier(library: &Library) -> Option<String> { library.natives.as_ref()?.get("windows").map(|name| name.replace("${arch}", "64")) }
fn extract_native(archive: &Path, destination: &Path, extract: Option<&Extract>) -> Result<()> {
    let mut zip = zip::ZipArchive::new(File::open(archive)?)?;
    for index in 0..zip.len() {
        let mut entry = zip.by_index(index)?; let name = entry.name().replace('\\', "/");
        if entry.is_dir() || name.starts_with("META-INF/") || extract.and_then(|e| e.exclude.as_ref()).is_some_and(|excluded| excluded.iter().any(|prefix| name.starts_with(prefix))) { continue; }
        let output = destination.join(&name); if let Some(parent) = output.parent() { fs::create_dir_all(parent)?; }
        std::io::copy(&mut entry, &mut File::create(output)?)?;
    }
    Ok(())
}
fn rules_allow(rules: Option<&[Rule]>) -> bool {
    let Some(rules) = rules else { return true; }; let mut allowed = false;
    for rule in rules {
        let os_matches = rule.os.as_ref().and_then(|os| os.name.as_ref()).is_none_or(|name| name == "windows");
        let feature_matches = rule.features.as_ref().is_none_or(|features| features.values().all(|wanted| !wanted));
        if os_matches && feature_matches { allowed = rule.action == "allow"; }
    }
    allowed
}
fn argument_values(argument: &Argument) -> Vec<String> {
    match argument {
        Argument::Plain(value) => vec![value.clone()],
        Argument::Conditional { rules, value } if rules_allow(rules.as_deref()) => match value { ArgumentValue::One(value) => vec![value.clone()], ArgumentValue::Many(values) => values.clone() },
        Argument::Conditional { .. } => vec![],
    }
}
fn replace_variables(value: String, replacements: &HashMap<&str, String>) -> String { replacements.iter().fold(value, |result, (needle, replacement)| result.replace(*needle, replacement)) }
fn build_arguments(meta: &VersionMeta, replacements: &HashMap<&str, String>) -> Result<(Vec<String>, Vec<String>)> {
    if let Some(arguments) = &meta.arguments {
        let jvm = arguments.jvm.iter().flat_map(argument_values).map(|value| replace_variables(value, replacements)).collect();
        let game = arguments.game.iter().flat_map(argument_values).map(|value| replace_variables(value, replacements)).collect();
        return Ok((jvm, game));
    }
    let game = meta.minecraft_arguments.as_ref().context("Version has no launch arguments")?.split_whitespace().map(|value| replace_variables(value.to_owned(), replacements)).collect();
    Ok((vec![], game))
}
