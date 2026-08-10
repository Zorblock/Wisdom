use anyhow::{Context, Result, bail};
use chrono::{DateTime, Duration, Utc};
use reqwest::blocking::Client;
use reqwest::header::AUTHORIZATION;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha1::{Digest, Sha1};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration as StdDuration;
use winreg::RegKey;
use winreg::enums::HKEY_CURRENT_USER;

slint::include_modules!();

const MANIFEST_URL: &str = "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json";
const USER_AGENT_VALUE: &str = "WisdomLauncher/0.1 (Windows; Rust)";

#[derive(Clone, Debug, Deserialize)]
struct VersionManifest {
    latest: LatestVersions,
    versions: Vec<ManifestVersion>,
}

#[derive(Clone, Debug, Deserialize)]
struct LatestVersions {
    release: String,
}

#[derive(Clone, Debug, Deserialize)]
struct ManifestVersion {
    id: String,
    url: String,
    #[serde(rename = "type")]
    kind: String,
}

#[derive(Debug, Deserialize)]
struct VersionMeta {
    id: String,
    #[serde(rename = "mainClass")]
    main_class: String,
    downloads: VersionDownloads,
    #[serde(rename = "assetIndex")]
    asset_index: Option<Download>,
    libraries: Vec<Library>,
    arguments: Option<LaunchArguments>,
    #[serde(rename = "minecraftArguments")]
    minecraft_arguments: Option<String>,
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

#[derive(Debug, Deserialize)]
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
    Conditional { rules: Option<Vec<Rule>>, value: ArgumentValue },
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
}

#[derive(Debug, Deserialize)]
struct AssetIndex {
    objects: HashMap<String, AssetObject>,
}

#[derive(Debug, Deserialize)]
struct AssetObject {
    hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Config {
    microsoft_client_id: String,
}

impl Default for Config {
    fn default() -> Self {
        Self { microsoft_client_id: String::new() }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AuthState {
    minecraft_access_token: String,
    microsoft_refresh_token: String,
    expires_at: DateTime<Utc>,
    player_name: String,
    player_uuid: String,
}

#[derive(Debug, Deserialize)]
struct DeviceCode {
    device_code: String,
    user_code: String,
    verification_uri: String,
    message: String,
    interval: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct OAuthToken {
    access_token: String,
    refresh_token: String,
}

#[derive(Debug, Deserialize)]
struct XboxResponse {
    #[serde(rename = "Token")]
    token: String,
    #[serde(rename = "DisplayClaims")]
    display_claims: XboxClaims,
}

#[derive(Debug, Deserialize)]
struct XboxClaims {
    xui: Vec<XboxUser>,
}

#[derive(Debug, Deserialize)]
struct XboxUser {
    uhs: String,
}

#[derive(Debug, Deserialize)]
struct MinecraftLogin {
    access_token: String,
    expires_in: i64,
}

#[derive(Debug, Deserialize)]
struct MinecraftProfile {
    id: String,
    name: String,
}

type StatusReporter = Arc<dyn Fn(String) + Send + Sync>;

fn main() -> Result<()> {
    let data_dir = user_data_dir()?;
    prepare_storage(&data_dir)?;
    let config = load_config(&data_dir)?;
    let window = AppWindow::new()?;
    window.set_accent_color(accent_color());
    window.set_status_text("Versionen werden geladen …".into());
    window.set_account_name(load_auth(&data_dir).map(|a| a.player_name).unwrap_or_default().into());

    let versions = Arc::new(Mutex::new(Vec::<ManifestVersion>::new()));
    let selected = Arc::new(Mutex::new(String::new()));
    let reporter = status_reporter(window.as_weak());

    {
        let weak = window.as_weak();
        let versions = Arc::clone(&versions);
        let selected = Arc::clone(&selected);
        let data_dir = data_dir.clone();
        thread::spawn(move || {
            match load_versions(&data_dir) {
                Ok((manifest, list)) => {
                    let default_id = manifest.latest.release;
                    if let Ok(mut stored) = versions.lock() { *stored = list.clone(); }
                    if let Ok(mut current) = selected.lock() { *current = default_id.clone(); }
                    let rows: Vec<SharedString> = list.iter().map(|v| v.id.clone().into()).collect();
                    let release_index = list.iter().position(|v| v.id == default_id).unwrap_or(0) as i32;
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(ui) = weak.upgrade() {
                            ui.set_versions(ModelRc::new(VecModel::from(rows)));
                            ui.set_selected_version(default_id.into());
                            ui.set_selected_version_index(release_index);
                            ui.set_status_text("Wähle eine Version und starte dein Spiel.".into());
                        }
                    });
                }
                Err(error) => {
                    let message = format!("Versionen konnten nicht geladen werden: {error:#}");
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(ui) = weak.upgrade() { ui.set_status_text(message.into()); }
                    });
                }
            }
        });
    }

    window.on_select_version({
        let versions = Arc::clone(&versions);
        let selected = Arc::clone(&selected);
        let weak = window.as_weak();
        move |index| {
            if let Some(item) = versions.lock().ok().and_then(|items| items.get(index.max(0) as usize).cloned()) {
                if let Ok(mut current) = selected.lock() { *current = item.id.clone(); }
                if let Some(ui) = weak.upgrade() { ui.set_selected_version(item.id.into()); }
            }
        }
    });

    let microsoft_client_id = config.microsoft_client_id.clone();
    window.on_login({
        let weak = window.as_weak();
        let data_dir = data_dir.clone();
        let reporter = Arc::clone(&reporter);
        let client_id = microsoft_client_id.clone();
        move || {
            if client_id.trim().is_empty() {
                if let Some(ui) = weak.upgrade() {
                    ui.set_status_text("Bitte trage zuerst deine Azure Client-ID in userData\\Wisdom\\config.json ein.".into());
                }
                return;
            }
            if let Some(ui) = weak.upgrade() { ui.set_busy(true); }
            let weak_done = weak.clone();
            let client_id = client_id.clone();
            let login_data_dir = data_dir.clone();
            let report = Arc::clone(&reporter);
            thread::spawn(move || {
                let outcome = microsoft_login(&client_id, &login_data_dir, &report);
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = weak_done.upgrade() {
                        ui.set_busy(false);
                        match outcome {
                            Ok(auth) => {
                                ui.set_account_name(auth.player_name.into());
                                ui.set_status_text("Microsoft-Konto bestätigt. Du kannst jetzt starten.".into());
                            }
                            Err(error) => ui.set_status_text(format!("Anmeldung fehlgeschlagen: {error:#}").into()),
                        }
                    }
                });
            });
        }
    });

    window.on_start_game({
        let weak = window.as_weak();
        let versions = Arc::clone(&versions);
        let selected = Arc::clone(&selected);
        let data_dir = data_dir.clone();
        let client_id = microsoft_client_id;
        let reporter = Arc::clone(&reporter);
        move || {
            let version_id = selected.lock().map(|value| value.clone()).unwrap_or_default();
            let manifest_version = versions.lock().ok().and_then(|items| items.iter().find(|item| item.id == version_id).cloned());
            let Some(manifest_version) = manifest_version else { return; };
            if let Some(ui) = weak.upgrade() { ui.set_busy(true); }
            let weak_done = weak.clone();
            let report = Arc::clone(&reporter);
            let launch_data_dir = data_dir.clone();
            let launch_client_id = client_id.clone();
            thread::spawn(move || {
                let outcome = (|| -> Result<()> {
                    let auth = ensure_session(&launch_data_dir, &launch_client_id)?;
                    install_and_launch(&launch_data_dir, &manifest_version, &auth, &report)
                })();
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = weak_done.upgrade() {
                        ui.set_busy(false);
                        match outcome {
                            Ok(()) => ui.set_status_text("Minecraft wurde gestartet.".into()),
                            Err(error) => ui.set_status_text(format!("Start fehlgeschlagen: {error:#}").into()),
                        }
                    }
                });
            });
        }
    });

    window.run()?;
    Ok(())
}

fn user_data_dir() -> Result<PathBuf> {
    let app_data = std::env::var_os("APPDATA").context("APPDATA is not set")?;
    Ok(PathBuf::from(app_data).join("zorblock").join("userData").join("Wisdom"))
}

fn prepare_storage(root: &Path) -> Result<()> {
    for folder in ["cache", "versions", "libraries", "assets/objects", "assets/indexes", "game", "natives"] {
        fs::create_dir_all(root.join(folder))?;
    }
    let path = root.join("config.json");
    if !path.exists() { write_json(&path, &Config::default())?; }
    Ok(())
}

fn load_config(root: &Path) -> Result<Config> { read_json(&root.join("config.json")) }
fn credential_entry() -> Result<keyring::Entry> {
    Ok(keyring::Entry::new("Wisdom Minecraft Launcher", "minecraft-session")?)
}

fn load_auth(_root: &Path) -> Result<AuthState> {
    let serialized = credential_entry()?.get_password().context("keine gespeicherte Microsoft-Sitzung")?;
    Ok(serde_json::from_str(&serialized)?)
}

fn save_auth(_root: &Path, auth: &AuthState) -> Result<()> {
    credential_entry()?.set_password(&serde_json::to_string(auth)?)?;
    Ok(())
}

fn read_json<T: for<'a> Deserialize<'a>>(path: &Path) -> Result<T> {
    serde_json::from_reader(File::open(path).with_context(|| format!("{} öffnen", path.display()))?).with_context(|| format!("{} lesen", path.display()))
}

fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let temp = path.with_extension("tmp");
    serde_json::to_writer_pretty(File::create(&temp)?, value)?;
    fs::rename(temp, path)?;
    Ok(())
}

fn http() -> Result<Client> {
    Ok(Client::builder().user_agent(USER_AGENT_VALUE).build()?)
}

fn load_versions(_root: &Path) -> Result<(VersionManifest, Vec<ManifestVersion>)> {
    let client = http()?;
    let manifest: VersionManifest = client.get(MANIFEST_URL).send()?.error_for_status()?.json()?;
    let mut list: Vec<_> = manifest.versions.iter().filter(|v| v.kind == "release" || v.kind == "snapshot").cloned().collect();
    // Releases come first; snapshots remain available without making the default list noisy.
    list.sort_by_key(|v| if v.kind == "release" { 0 } else { 1 });
    Ok((manifest, list))
}

fn status_reporter(weak: slint::Weak<AppWindow>) -> StatusReporter {
    Arc::new(move |message| {
        let weak = weak.clone();
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(ui) = weak.upgrade() { ui.set_status_text(message.into()); }
        });
    })
}

fn microsoft_login(client_id: &str, root: &Path, report: &StatusReporter) -> Result<AuthState> {
    let client = http()?;
    report("Microsoft-Anmeldung wird geöffnet …".into());
    let device: DeviceCode = client.post("https://login.microsoftonline.com/consumers/oauth2/v2.0/devicecode")
        .form(&[("client_id", client_id), ("scope", "XboxLive.signin offline_access")])
        .send()?.error_for_status()?.json()?;
    report(format!("{} — Code: {}", device.message, device.user_code));
    let _ = open::that(&device.verification_uri);
    let token = poll_device_code(&client, client_id, &device)?;
    let auth = minecraft_authenticate(&client, &token.access_token, &token.refresh_token)?;
    save_auth(root, &auth)?;
    Ok(auth)
}

fn poll_device_code(client: &Client, client_id: &str, device: &DeviceCode) -> Result<OAuthToken> {
    let interval = device.interval.unwrap_or(5).max(2);
    for _ in 0..180 {
        thread::sleep(StdDuration::from_secs(interval));
        let response = client.post("https://login.microsoftonline.com/consumers/oauth2/v2.0/token")
            .form(&[("client_id", client_id), ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"), ("device_code", &device.device_code)])
            .send()?;
        let status = response.status();
        if status.is_success() { return Ok(response.json()?); }
        let body: Value = response.json().unwrap_or_else(|_| json!({}));
        match body["error"].as_str() {
            Some("authorization_pending") | Some("slow_down") => continue,
            Some(error) => bail!("Microsoft: {}", body["error_description"].as_str().unwrap_or(error)),
            None => bail!("Microsoft antwortete mit {status}"),
        }
    }
    bail!("Der Microsoft-Anmeldecode ist abgelaufen")
}

fn refresh_microsoft_session(client_id: &str, auth: &AuthState) -> Result<AuthState> {
    let client = http()?;
    let token: OAuthToken = client.post("https://login.microsoftonline.com/consumers/oauth2/v2.0/token")
        .form(&[("client_id", client_id), ("grant_type", "refresh_token"), ("refresh_token", &auth.microsoft_refresh_token), ("scope", "XboxLive.signin offline_access")])
        .send()?.error_for_status()?.json()?;
    minecraft_authenticate(&client, &token.access_token, &token.refresh_token)
}

fn minecraft_authenticate(client: &Client, microsoft_token: &str, refresh_token: &str) -> Result<AuthState> {
    let xbl: XboxResponse = post_json(client, "https://user.auth.xboxlive.com/user/authenticate", json!({
        "Properties": {"AuthMethod": "RPS", "SiteName": "user.auth.xboxlive.com", "RpsTicket": format!("d={microsoft_token}")},
        "RelyingParty": "http://auth.xboxlive.com", "TokenType": "JWT"
    }))?;
    let xsts: XboxResponse = post_json(client, "https://xsts.auth.xboxlive.com/xsts/authorize", json!({
        "Properties": {"SandboxId": "RETAIL", "UserTokens": [xbl.token]},
        "RelyingParty": "rp://api.minecraftservices.com/", "TokenType": "JWT"
    }))?;
    let uhs = xsts.display_claims.xui.first().context("Xbox-Konto enthält keine Benutzerkennung")?.uhs.clone();
    let minecraft: MinecraftLogin = post_json(client, "https://api.minecraftservices.com/authentication/login_with_xbox", json!({
        "identityToken": format!("XBL3.0 x={uhs};{}", xsts.token)
    }))?;
    let entitlement: Value = client.get("https://api.minecraftservices.com/entitlements/mcstore")
        .header(AUTHORIZATION, format!("Bearer {}", minecraft.access_token)).send()?.error_for_status()?.json()?;
    if entitlement["items"].as_array().is_none_or(|items| items.is_empty()) { bail!("Dieses Microsoft-Konto besitzt keine Minecraft-Java-Lizenz") }
    let profile: MinecraftProfile = client.get("https://api.minecraftservices.com/minecraft/profile")
        .header(AUTHORIZATION, format!("Bearer {}", minecraft.access_token)).send()?.error_for_status()?.json()?;
    Ok(AuthState {
        minecraft_access_token: minecraft.access_token,
        microsoft_refresh_token: refresh_token.to_owned(),
        expires_at: Utc::now() + Duration::seconds(minecraft.expires_in),
        player_name: profile.name,
        player_uuid: profile.id,
    })
}

fn post_json<T: for<'a> Deserialize<'a>>(client: &Client, url: &str, body: Value) -> Result<T> {
    let response = client.post(url).json(&body).send()?;
    if !response.status().is_success() { bail!("Dienst antwortete mit {}", response.status()); }
    Ok(response.json()?)
}

fn ensure_session(root: &Path, client_id: &str) -> Result<AuthState> {
    let auth = load_auth(root).context("Melde dich zuerst mit Microsoft an")?;
    let client = http()?;
    let valid = client.get("https://api.minecraftservices.com/minecraft/profile")
        .header(AUTHORIZATION, format!("Bearer {}", auth.minecraft_access_token)).send()
        .is_ok_and(|response| response.status().is_success());
    if valid && auth.expires_at > Utc::now() + Duration::seconds(60) { return Ok(auth); }
    if client_id.trim().is_empty() { bail!("Sitzung abgelaufen. Trage eine Azure Client-ID ein und melde dich erneut an") }
    let refreshed = refresh_microsoft_session(client_id, &auth).context("Microsoft-Sitzung konnte nicht erneuert werden")?;
    save_auth(root, &refreshed)?;
    Ok(refreshed)
}

fn install_and_launch(root: &Path, entry: &ManifestVersion, auth: &AuthState, report: &StatusReporter) -> Result<()> {
    let client = http()?;
    report(format!("Lade Minecraft {}-Metadaten …", entry.id));
    let meta: VersionMeta = client.get(&entry.url).send()?.error_for_status()?.json()?;
    let version_dir = root.join("versions").join(&meta.id);
    fs::create_dir_all(&version_dir)?;
    let client_jar = version_dir.join(format!("{}.jar", meta.id));
    download_file(&client, &meta.downloads.client, &client_jar)?;

    report("Lade Bibliotheken und Windows-Komponenten …".into());
    let mut classpath = vec![client_jar];
    let natives_dir = root.join("natives").join(&meta.id);
    fs::create_dir_all(&natives_dir)?;
    for library in &meta.libraries {
        if !rules_allow(library.rules.as_deref()) { continue; }
        let Some(downloads) = &library.downloads else { continue; };
        if let Some(artifact) = &downloads.artifact {
            let path = library_path(root, artifact)?;
            download_file(&client, artifact, &path)?;
            classpath.push(path);
        }
        if let Some(classifier) = native_classifier(library) {
            if let Some(native) = downloads.classifiers.as_ref().and_then(|values| values.get(&classifier)) {
                let path = library_path(root, native)?;
                download_file(&client, native, &path)?;
                extract_native(&path, &natives_dir, library.extract.as_ref())?;
            }
        }
    }

    let assets_root = root.join("assets");
    let assets_index_name = if let Some(asset_index) = &meta.asset_index {
        report("Lade Spielressourcen …".into());
        let index_path = assets_root.join("indexes").join(format!("{}.json", asset_index.path.clone().unwrap_or_else(|| asset_index.url.rsplit('/').next().unwrap_or("index.json").to_owned())));
        download_file(&client, asset_index, &index_path)?;
        let index: AssetIndex = read_json(&index_path)?;
        for asset in index.objects.values() {
            let path = assets_root.join("objects").join(&asset.hash[0..2]).join(&asset.hash);
            if !path.exists() {
                let download = Download { url: format!("https://resources.download.minecraft.net/{}/{}", &asset.hash[0..2], asset.hash), path: None, sha1: Some(asset.hash.clone()) };
                download_file(&client, &download, &path)?;
            }
        }
        meta.asset_index.as_ref().and_then(|d| d.path.clone()).unwrap_or_else(|| index_path.file_stem().unwrap_or_default().to_string_lossy().to_string())
    } else { String::new() };

    report("Starte Java …".into());
    let game_dir = root.join("game");
    fs::create_dir_all(&game_dir)?;
    let classpath_text = std::env::join_paths(&classpath)?.to_string_lossy().to_string();
    let substitutions = HashMap::from([
        ("${auth_player_name}", auth.player_name.clone()), ("${version_name}", meta.id.clone()),
        ("${game_directory}", game_dir.to_string_lossy().to_string()), ("${assets_root}", assets_root.to_string_lossy().to_string()),
        ("${assets_index_name}", assets_index_name), ("${auth_uuid}", auth.player_uuid.clone()),
        ("${auth_access_token}", auth.minecraft_access_token.clone()), ("${user_type}", "msa".into()),
        ("${version_type}", "Wisdom".into()), ("${natives_directory}", natives_dir.to_string_lossy().to_string()),
        ("${classpath}", classpath_text), ("${classpath_separator}", ";".into()),
        ("${launcher_name}", "Wisdom".into()), ("${launcher_version}", env!("CARGO_PKG_VERSION").into()),
    ]);
    let (mut jvm_args, game_args) = build_arguments(&meta, &substitutions)?;
    if !jvm_args.iter().any(|arg| arg == "-cp" || arg == "-classpath") { jvm_args.extend(["-cp".to_owned(), substitutions["${classpath}"].clone()]); }
    let java = std::env::var_os("JAVA_HOME").map(|home| PathBuf::from(home).join("bin").join("java.exe")).unwrap_or_else(|| PathBuf::from("java"));
    Command::new(java).args(&jvm_args).arg(&meta.main_class).args(&game_args).current_dir(&game_dir).spawn()
        .context("Java konnte nicht gestartet werden. Installiere Java 21+ oder setze JAVA_HOME")?;
    Ok(())
}

fn download_file(client: &Client, source: &Download, destination: &Path) -> Result<()> {
    if destination.exists() && source.sha1.as_ref().is_none_or(|hash| sha1_file(destination).is_ok_and(|actual| actual.eq_ignore_ascii_case(hash))) { return Ok(()); }
    let parent = destination.parent().context("Downloadziel ohne Ordner")?;
    fs::create_dir_all(parent)?;
    let bytes = client.get(&source.url).send()?.error_for_status()?.bytes()?;
    if let Some(expected) = &source.sha1 {
        let actual = format!("{:x}", Sha1::digest(&bytes));
        if !actual.eq_ignore_ascii_case(expected) { bail!("Prüfsumme stimmt nicht für {}", source.url); }
    }
    let temporary = destination.with_extension("download");
    File::create(&temporary)?.write_all(&bytes)?;
    fs::rename(temporary, destination)?;
    Ok(())
}

fn sha1_file(path: &Path) -> Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha1::new();
    let mut buffer = [0u8; 8192];
    loop { let read = file.read(&mut buffer)?; if read == 0 { break; } hasher.update(&buffer[..read]); }
    Ok(format!("{:x}", hasher.finalize()))
}

fn library_path(root: &Path, download: &Download) -> Result<PathBuf> {
    Ok(root.join("libraries").join(download.path.as_ref().context("Bibliothek ohne Pfad")?))
}

fn native_classifier(library: &Library) -> Option<String> {
    library.natives.as_ref()?.get("windows").map(|name| name.replace("${arch}", "64"))
}

fn extract_native(archive: &Path, destination: &Path, extract: Option<&Extract>) -> Result<()> {
    let file = File::open(archive)?;
    let mut zip = zip::ZipArchive::new(file)?;
    for index in 0..zip.len() {
        let mut entry = zip.by_index(index)?;
        let name = entry.name().replace('\\', "/");
        if entry.is_dir() || name.starts_with("META-INF/") || extract.and_then(|e| e.exclude.as_ref()).is_some_and(|excluded| excluded.iter().any(|prefix| name.starts_with(prefix))) { continue; }
        let output = destination.join(&name);
        if let Some(parent) = output.parent() { fs::create_dir_all(parent)?; }
        let mut file = File::create(output)?;
        std::io::copy(&mut entry, &mut file)?;
    }
    Ok(())
}

fn rules_allow(rules: Option<&[Rule]>) -> bool {
    let Some(rules) = rules else { return true; };
    let mut allowed = false;
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

fn replace_variables(value: String, replacements: &HashMap<&str, String>) -> String {
    replacements.iter().fold(value, |result, (needle, replacement)| result.replace(*needle, replacement))
}

fn build_arguments(meta: &VersionMeta, replacements: &HashMap<&str, String>) -> Result<(Vec<String>, Vec<String>)> {
    if let Some(arguments) = &meta.arguments {
        let jvm = arguments.jvm.iter().flat_map(argument_values).map(|value| replace_variables(value, replacements)).collect();
        let game = arguments.game.iter().flat_map(argument_values).map(|value| replace_variables(value, replacements)).collect();
        return Ok((jvm, game));
    }
    let game = meta.minecraft_arguments.as_ref().context("Version enthält keine Startargumente")?
        .split_whitespace().map(|value| replace_variables(value.to_owned(), replacements)).collect();
    Ok((vec![], game))
}

fn accent_color() -> slint::Color {
    let key = RegKey::predef(HKEY_CURRENT_USER).open_subkey("Software\\Microsoft\\Windows\\DWM");
    let color = key.ok().and_then(|key| key.get_value::<u32, _>("ColorizationColor").ok()).unwrap_or(0xFF6EA8FE);
    slint::Color::from_rgb_u8(((color >> 16) & 0xFF) as u8, ((color >> 8) & 0xFF) as u8, (color & 0xFF) as u8)
}
