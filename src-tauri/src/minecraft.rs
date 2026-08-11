use crate::storage::{AuthState, http, read_json};
use anyhow::{Context, Result, bail};
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};
use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::{Read, Write};
#[cfg(windows)]
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::{Duration, SystemTime};

const MANIFEST_URL: &str = "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json";
pub type ProgressReporter = dyn Fn(f32, String) + Send + Sync;

#[derive(Clone, Debug)]
pub struct LaunchOptions {
    pub ram_mb: u32,
    pub jvm_args: String,
    pub game_args: String,
    pub open_console: bool,
    pub client_id: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct VersionManifest {
    pub latest: LatestVersions,
    pub versions: Vec<ManifestVersion>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct LatestVersions {
    pub release: String,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ManifestVersion {
    pub id: String,
    pub url: String,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(rename = "releaseTime", default)]
    pub release_time: Option<String>,
}
#[derive(Debug, Deserialize)]
struct VersionMeta {
    id: String,
    #[serde(rename = "mainClass")]
    main_class: String,
    downloads: VersionDownloads,
    #[serde(rename = "assetIndex")]
    asset_index: Option<Download>,
    #[serde(rename = "javaVersion")]
    java_version: Option<JavaVersion>,
    libraries: Vec<Library>,
    arguments: Option<LaunchArguments>,
    #[serde(rename = "minecraftArguments")]
    minecraft_arguments: Option<String>,
    logging: Option<GameLogging>,
}
#[derive(Debug, Deserialize)]
struct JavaVersion {
    #[serde(rename = "majorVersion")]
    major_version: u32,
}
#[derive(Debug, Deserialize)]
struct VersionDownloads {
    client: Download,
}
#[derive(Clone, Debug, Deserialize)]
struct Download {
    url: String,
    path: Option<String>,
    sha1: Option<String>,
    id: Option<String>,
}
#[derive(Debug, Deserialize)]
struct Library {
    downloads: Option<LibraryDownloads>,
    rules: Option<Vec<Rule>>,
    natives: Option<HashMap<String, String>>,
    extract: Option<Extract>,
}
#[derive(Debug, Deserialize)]
struct LibraryDownloads {
    artifact: Option<Download>,
    classifiers: Option<HashMap<String, Download>>,
}
#[derive(Clone, Debug, Deserialize)]
struct Extract {
    exclude: Option<Vec<String>>,
}
#[derive(Debug, Deserialize)]
struct LaunchArguments {
    game: Vec<Argument>,
    jvm: Vec<Argument>,
}
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum Argument {
    Plain(String),
    Conditional {
        rules: Option<Vec<Rule>>,
        value: ArgumentValue,
    },
}
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum ArgumentValue {
    One(String),
    Many(Vec<String>),
}
#[derive(Debug, Deserialize)]
struct Rule {
    action: String,
    os: Option<RuleOs>,
    features: Option<HashMap<String, bool>>,
}
#[derive(Debug, Deserialize)]
struct RuleOs {
    name: Option<String>,
    arch: Option<String>,
}
#[derive(Debug, Deserialize)]
struct AssetIndex {
    objects: HashMap<String, AssetObject>,
}
#[derive(Debug, Deserialize)]
struct AssetObject {
    hash: String,
}
#[derive(Debug, Deserialize)]
struct GameLogging {
    client: Option<LoggingClient>,
}
#[derive(Debug, Deserialize)]
struct LoggingClient {
    argument: String,
    file: Download,
}

pub fn load_versions(root: &Path) -> Result<(VersionManifest, Vec<ManifestVersion>)> {
    let cache = root.join("cache").join("version_manifest_v2.json");
    let cache_is_fresh = fs::metadata(&cache)
        .and_then(|metadata| metadata.modified())
        .ok()
        .and_then(|modified| SystemTime::now().duration_since(modified).ok())
        .is_some_and(|age| age < Duration::from_secs(60 * 60));
    let cached_manifest = read_json::<VersionManifest>(&cache).ok();
    let manifest = if let (true, Some(manifest)) = (cache_is_fresh, cached_manifest.as_ref()) {
        manifest.clone()
    } else {
        match http()?
            .get(MANIFEST_URL)
            .send()
            .and_then(|response| response.error_for_status())
        {
            Ok(response) => {
                let manifest: VersionManifest = response.json()?;
                let temporary = cache.with_extension("download");
                fs::write(&temporary, serde_json::to_vec(&manifest)?)?;
                replace_file(&temporary, &cache)?;
                manifest
            }
            Err(error) => match cached_manifest {
                Some(manifest) => manifest,
                None => {
                    return Err(error)
                        .context("Minecraft-Versionsliste konnte nicht geladen werden");
                }
            },
        }
    };
    let list = manifest
        .versions
        .iter()
        .filter(|v| v.kind == "release" || v.kind == "snapshot")
        .cloned()
        .collect();
    Ok((manifest, list))
}

pub fn install_and_launch(
    root: &Path,
    game_dir: &Path,
    entry: &ManifestVersion,
    auth: &AuthState,
    options: &LaunchOptions,
    report: &(dyn Fn(String) + Send + Sync),
    progress: &ProgressReporter,
) -> Result<()> {
    let client = http()?;
    report(format!("Minecraft {} wird vorbereitet …", entry.id));
    let meta: VersionMeta = client.get(&entry.url).send()?.error_for_status()?.json()?;
    let java = crate::runtime::ensure_java(
        root,
        meta.java_version
            .as_ref()
            .map(|version| version.major_version)
            .unwrap_or(8),
        progress,
    )?;
    let version_dir = root.join("versions").join(&meta.id);
    fs::create_dir_all(&version_dir)?;
    let client_jar = version_dir.join(format!("{}.jar", meta.id));
    download_file(
        &client,
        &meta.downloads.client,
        &client_jar,
        "Minecraft-Client",
        progress,
    )?;

    report("Bibliotheken und Windows-Komponenten werden geprüft …".into());
    let mut classpath = vec![client_jar];
    let natives_dir = root.join("natives").join(&meta.id);
    fs::create_dir_all(&natives_dir)?;
    let mut libraries = Vec::new();
    let mut native_archives = Vec::new();
    let mut seen_libraries = HashSet::new();
    let mut seen_natives = HashSet::new();
    for library in &meta.libraries {
        if !rules_allow(library.rules.as_deref()) {
            continue;
        }
        let Some(downloads) = &library.downloads else {
            continue;
        };
        if let Some(artifact) = &downloads.artifact {
            let path = library_path(root, artifact)?;
            if seen_libraries.insert(path.clone()) {
                libraries.push((artifact.clone(), path.clone()));
                classpath.push(path);
            }
        }
        if let Some(classifier) = native_classifier(library)
            && let Some(native) = downloads
                .classifiers
                .as_ref()
                .and_then(|values| values.get(&classifier))
        {
            let path = library_path(root, native)?;
            if seen_natives.insert(path.clone()) {
                native_archives.push((native.clone(), path, library.extract.clone()));
            }
        }
    }
    download_batch(&client, libraries, "Bibliotheken", progress)?;
    let native_downloads = native_archives
        .iter()
        .map(|(download, path, _)| (download.clone(), path.clone()))
        .collect();
    download_batch(&client, native_downloads, "Windows-Komponenten", progress)?;
    for (_, archive, extract) in native_archives {
        extract_native(&archive, &natives_dir, extract.as_ref())?;
    }

    let assets_root = root.join("assets");
    let assets_index_name = if let Some(asset_index) = &meta.asset_index {
        report("Spieldateien werden geprüft …".into());
        let index_name = asset_index
            .id
            .clone()
            .or_else(|| asset_index.path.clone())
            .unwrap_or_else(|| {
                asset_index
                    .url
                    .rsplit('/')
                    .next()
                    .unwrap_or("index")
                    .trim_end_matches(".json")
                    .to_owned()
            });
        let index_path = assets_root
            .join("indexes")
            .join(format!("{index_name}.json"));
        download_file(&client, asset_index, &index_path, "Dateiindex", progress)?;
        let index: AssetIndex = read_json(&index_path)?;
        let mut seen_hashes = HashSet::new();
        let mut missing_assets = Vec::new();
        for asset in index.objects.values() {
            if !seen_hashes.insert(asset.hash.clone()) {
                continue;
            }
            let path = assets_root
                .join("objects")
                .join(&asset.hash[0..2])
                .join(&asset.hash);
            if !path.exists() {
                missing_assets.push((
                    Download {
                        url: format!(
                            "https://resources.download.minecraft.net/{}/{}",
                            &asset.hash[0..2],
                            asset.hash
                        ),
                        path: None,
                        sha1: Some(asset.hash.clone()),
                        id: None,
                    },
                    path,
                ));
            }
        }
        download_batch(&client, missing_assets, "Spieldateien", progress)?;
        index_name
    } else {
        String::new()
    };

    report("Minecraft wird gestartet …".into());
    fs::create_dir_all(game_dir)?;
    let classpath_text = std::env::join_paths(&classpath)?
        .to_string_lossy()
        .to_string();
    let substitutions = HashMap::from([
        ("${auth_player_name}", auth.player_name.clone()),
        ("${version_name}", meta.id.clone()),
        ("${game_directory}", game_dir.to_string_lossy().to_string()),
        ("${assets_root}", assets_root.to_string_lossy().to_string()),
        ("${assets_index_name}", assets_index_name),
        ("${auth_uuid}", auth.player_uuid.clone()),
        ("${auth_access_token}", auth.minecraft_access_token.clone()),
        ("${user_type}", "msa".into()),
        ("${version_type}", "Wisdom".into()),
        (
            "${natives_directory}",
            natives_dir.to_string_lossy().to_string(),
        ),
        ("${classpath}", classpath_text),
        ("${classpath_separator}", ";".into()),
        ("${launcher_name}", "Wisdom".into()),
        ("${launcher_version}", env!("CARGO_PKG_VERSION").into()),
        ("${clientid}", options.client_id.clone()),
        ("${auth_xuid}", String::new()),
        ("${user_properties}", "{}".into()),
    ]);
    let (mut jvm_args, game_args) = build_arguments(&meta, &substitutions)?;
    if let Some(logging) = meta
        .logging
        .as_ref()
        .and_then(|logging| logging.client.as_ref())
    {
        let file_name = logging
            .file
            .id
            .as_deref()
            .unwrap_or("client-log-config.xml");
        let log_path = assets_root.join("log_configs").join(file_name);
        download_file(
            &client,
            &logging.file,
            &log_path,
            "Log-Konfiguration",
            progress,
        )?;
        jvm_args.push(
            logging
                .argument
                .replace("${path}", &log_path.to_string_lossy()),
        );
    }
    jvm_args.push(format!("-Xmx{}M", options.ram_mb));
    jvm_args.extend(parse_extra_arguments(&options.jvm_args)?);
    if !jvm_args
        .iter()
        .any(|arg| arg == "-cp" || arg == "-classpath")
    {
        jvm_args.extend(["-cp".to_owned(), substitutions["${classpath}"].clone()]);
    }
    let mut game_args = game_args;
    game_args.extend(parse_extra_arguments(&options.game_args)?);
    #[cfg(windows)]
    let java = if options.open_console {
        java
    } else {
        let javaw = java.with_file_name("javaw.exe");
        if javaw.exists() { javaw } else { java }
    };
    let mut game = Command::new(java);
    game.args(&jvm_args)
        .arg(&meta.main_class)
        .args(&game_args)
        .current_dir(game_dir);
    #[cfg(windows)]
    game.creation_flags(if options.open_console {
        0x0000_0010
    } else {
        0x0800_0000
    });
    game.spawn().context("Java konnte nicht gestartet werden")?;
    Ok(())
}

fn download_file(
    client: &Client,
    source: &Download,
    destination: &Path,
    label: &str,
    progress: &ProgressReporter,
) -> Result<()> {
    download_to_disk(client, source, destination, |received, total| {
        let amount = if total == 0 {
            0.0
        } else {
            received as f32 / total as f32
        };
        progress(
            amount,
            format!("{label} wird geladen · {}%", (amount * 100.0) as u32),
        );
    })
}

fn download_to_disk(
    client: &Client,
    source: &Download,
    destination: &Path,
    mut on_progress: impl FnMut(u64, u64),
) -> Result<()> {
    if destination.exists()
        && source.sha1.as_ref().is_none_or(|hash| {
            sha1_file(destination).is_ok_and(|actual| actual.eq_ignore_ascii_case(hash))
        })
    {
        return Ok(());
    }
    fs::create_dir_all(
        destination
            .parent()
            .context("Download target has no parent directory")?,
    )?;
    let mut response = client.get(&source.url).send()?.error_for_status()?;
    let total = response.content_length().unwrap_or(0);
    let temporary = destination.with_extension("download");
    let mut output = File::create(&temporary)?;
    let mut hasher = Sha1::new();
    let mut received = 0u64;
    let mut buffer = [0u8; 131_072];
    loop {
        let count = response.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        output.write_all(&buffer[..count])?;
        hasher.update(&buffer[..count]);
        received += count as u64;
        on_progress(received, total);
    }
    if let Some(expected) = &source.sha1 {
        let actual = format!("{:x}", hasher.finalize());
        if !actual.eq_ignore_ascii_case(expected) {
            fs::remove_file(&temporary)?;
            bail!("Prüfsumme stimmt nicht überein: {}", source.url);
        }
    }
    replace_file(&temporary, destination)?;
    Ok(())
}

fn download_batch(
    client: &Client,
    tasks: Vec<(Download, PathBuf)>,
    category: &str,
    progress: &ProgressReporter,
) -> Result<()> {
    if tasks.is_empty() {
        progress(1.0, format!("{category} sind bereit."));
        return Ok(());
    }

    let total = tasks.len();
    progress(0.0, format!("{category} werden geladen · 0/{total}"));
    let (sender, receiver) = mpsc::channel();
    for task in tasks {
        sender.send(task).expect("download queue should be open");
    }
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
            scope.spawn(move || {
                loop {
                    if failure
                        .lock()
                        .ok()
                        .and_then(|error| error.clone())
                        .is_some()
                    {
                        break;
                    }
                    let task = queue.lock().ok().and_then(|queue| queue.recv().ok());
                    let Some((source, destination)) = task else {
                        break;
                    };
                    if let Err(error) = download_to_disk(&client, &source, &destination, |_, _| {})
                    {
                        if let Ok(mut stored) = failure.lock() {
                            *stored = Some(format!(
                                "Download fehlgeschlagen ({}): {error:#}",
                                source.url
                            ));
                        }
                        break;
                    }
                    let count = completed.fetch_add(1, Ordering::Relaxed) + 1;
                    progress(
                        count as f32 / total as f32,
                        format!("{category} werden geladen · {count}/{total}"),
                    );
                }
            });
        }
    });

    if let Some(error) = failure.lock().ok().and_then(|error| error.clone()) {
        bail!(error);
    }
    Ok(())
}

fn sha1_file(path: &Path) -> Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha1::new();
    let mut buffer = [0u8; 8192];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}
fn library_path(root: &Path, download: &Download) -> Result<PathBuf> {
    Ok(root
        .join("libraries")
        .join(download.path.as_ref().context("Library has no path")?))
}
fn native_classifier(library: &Library) -> Option<String> {
    library
        .natives
        .as_ref()?
        .get("windows")
        .map(|name| name.replace("${arch}", "64"))
}
fn extract_native(archive: &Path, destination: &Path, extract: Option<&Extract>) -> Result<()> {
    let mut zip = zip::ZipArchive::new(File::open(archive)?)?;
    for index in 0..zip.len() {
        let mut entry = zip.by_index(index)?;
        let name = entry.name().replace('\\', "/");
        if entry.is_dir()
            || name.starts_with("META-INF/")
            || extract
                .and_then(|e| e.exclude.as_ref())
                .is_some_and(|excluded| excluded.iter().any(|prefix| name.starts_with(prefix)))
        {
            continue;
        }
        let Some(relative) = entry.enclosed_name() else {
            continue;
        };
        let output = destination.join(relative);
        if let Some(parent) = output.parent() {
            fs::create_dir_all(parent)?;
        }
        std::io::copy(&mut entry, &mut File::create(output)?)?;
    }
    Ok(())
}
fn rules_allow(rules: Option<&[Rule]>) -> bool {
    let Some(rules) = rules else {
        return true;
    };
    let mut allowed = false;
    for rule in rules {
        let os_matches = rule.os.as_ref().is_none_or(|os| {
            let name_matches = os.name.as_ref().is_none_or(|name| name == "windows");
            let arch_matches = os.arch.as_ref().is_none_or(|arch| match arch.as_str() {
                "x86" => std::env::consts::ARCH == "x86",
                "x86_64" | "amd64" => std::env::consts::ARCH == "x86_64",
                other => other == std::env::consts::ARCH,
            });
            name_matches && arch_matches
        });
        let feature_matches = rule
            .features
            .as_ref()
            .is_none_or(|features| features.values().all(|wanted| !wanted));
        if os_matches && feature_matches {
            allowed = rule.action == "allow";
        }
    }
    allowed
}
fn argument_values(argument: &Argument) -> Vec<String> {
    match argument {
        Argument::Plain(value) => vec![value.clone()],
        Argument::Conditional { rules, value } if rules_allow(rules.as_deref()) => match value {
            ArgumentValue::One(value) => vec![value.clone()],
            ArgumentValue::Many(values) => values.clone(),
        },
        Argument::Conditional { .. } => vec![],
    }
}
fn replace_variables(value: String, replacements: &HashMap<&str, String>) -> String {
    replacements
        .iter()
        .fold(value, |result, (needle, replacement)| {
            result.replace(*needle, replacement)
        })
}
fn build_arguments(
    meta: &VersionMeta,
    replacements: &HashMap<&str, String>,
) -> Result<(Vec<String>, Vec<String>)> {
    if let Some(arguments) = &meta.arguments {
        let jvm = arguments
            .jvm
            .iter()
            .flat_map(argument_values)
            .map(|value| replace_variables(value, replacements))
            .collect();
        let game = arguments
            .game
            .iter()
            .flat_map(argument_values)
            .map(|value| replace_variables(value, replacements))
            .collect();
        return Ok((jvm, game));
    }
    let game = meta
        .minecraft_arguments
        .as_ref()
        .context("Version has no launch arguments")?
        .split_whitespace()
        .map(|value| replace_variables(value.to_owned(), replacements))
        .collect();
    Ok((vec![], game))
}

fn replace_file(source: &Path, destination: &Path) -> Result<()> {
    if destination.exists() {
        fs::remove_file(destination)?;
    }
    fs::rename(source, destination)?;
    Ok(())
}

fn parse_extra_arguments(input: &str) -> Result<Vec<String>> {
    let mut arguments = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    let mut escaped = false;
    for character in input.chars() {
        if escaped {
            current.push(character);
            escaped = false;
        } else if character == '\\' && quoted {
            escaped = true;
        } else if character == '"' {
            quoted = !quoted;
        } else if character.is_whitespace() && !quoted {
            if !current.is_empty() {
                arguments.push(std::mem::take(&mut current));
            }
        } else {
            current.push(character);
        }
    }
    if quoted {
        bail!("Unclosed quote in launch arguments");
    }
    if escaped {
        current.push('\\');
    }
    if !current.is_empty() {
        arguments.push(current);
    }
    Ok(arguments)
}

#[cfg(test)]
mod tests {
    use super::parse_extra_arguments;

    #[test]
    fn parses_quoted_extra_arguments() {
        assert_eq!(
            parse_extra_arguments(r#"-Dname="hello world" --demo"#).unwrap(),
            ["-Dname=hello world", "--demo"]
        );
    }
}
