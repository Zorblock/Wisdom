use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;
use winreg::RegKey;
use winreg::enums::HKEY_CURRENT_USER;

const USER_AGENT: &str = "WisdomLauncher/0.1 (Windows; Rust)";
const PUBLISHER: &str = "Zorblock";
const PRODUCT: &str = "Wisdom";
static USER_DATA_DIRECTORY: OnceLock<std::result::Result<PathBuf, String>> = OnceLock::new();

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthState {
    pub minecraft_access_token: String,
    pub microsoft_refresh_token: String,
    pub expires_at: DateTime<Utc>,
    pub player_name: String,
    pub player_uuid: String,
    #[serde(default)]
    pub skin_url: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountProfile {
    pub name: String,
    pub uuid: String,
    pub skin_url: Option<String>,
}

impl From<&AuthState> for AccountProfile {
    fn from(auth: &AuthState) -> Self {
        Self {
            name: auth.player_name.clone(),
            uuid: auth.player_uuid.clone(),
            skin_url: auth.skin_url.clone(),
        }
    }
}

#[derive(Serialize, Deserialize)]
struct SessionProfile {
    expires_at: DateTime<Utc>,
    player_name: String,
    player_uuid: String,
    skin_url: Option<String>,
}

#[derive(Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AccountIndex {
    active_uuid: Option<String>,
    accounts: Vec<AccountProfile>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LauncherSettings {
    #[serde(default, alias = "open_console")]
    pub open_console: bool,
    #[serde(default, alias = "show_snapshots")]
    pub show_snapshots: bool,
    #[serde(default = "default_ram_mb", alias = "ram_mb")]
    pub ram_mb: u32,
    #[serde(default, alias = "jvm_args")]
    pub jvm_args: String,
    #[serde(default, alias = "game_args")]
    pub game_args: String,
    #[serde(default, alias = "launch_behavior")]
    pub launch_behavior: LaunchBehavior,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LaunchBehavior {
    #[default]
    KeepOpen,
    Hide,
    Close,
}

fn default_ram_mb() -> u32 {
    4096
}

impl Default for LauncherSettings {
    fn default() -> Self {
        Self {
            open_console: false,
            show_snapshots: false,
            ram_mb: default_ram_mb(),
            jvm_args: String::new(),
            game_args: String::new(),
            launch_behavior: LaunchBehavior::default(),
        }
    }
}

pub fn user_data_dir() -> Result<PathBuf> {
    USER_DATA_DIRECTORY
        .get_or_init(resolve_user_data_dir)
        .clone()
        .map_err(anyhow::Error::msg)
}

fn resolve_user_data_dir() -> std::result::Result<PathBuf, String> {
    let local_app_data =
        std::env::var_os("LOCALAPPDATA").ok_or_else(|| "LOCALAPPDATA is not set".to_owned())?;
    let destination = PathBuf::from(local_app_data).join(PUBLISHER).join(PRODUCT);

    if destination.exists() {
        return Ok(destination);
    }

    let Some(roaming_app_data) = std::env::var_os("APPDATA") else {
        return Ok(destination);
    };
    let legacy = PathBuf::from(roaming_app_data)
        .join("zorblock")
        .join("userData")
        .join(PRODUCT);
    if !legacy.is_dir() {
        return Ok(destination);
    }

    migrate_legacy_data(&legacy, &destination).map_err(|error| {
        format!(
            "Could not migrate Wisdom data from {} to {}: {error}",
            legacy.display(),
            destination.display()
        )
    })?;
    Ok(destination)
}

fn migrate_legacy_data(source: &Path, destination: &Path) -> Result<()> {
    let parent = destination
        .parent()
        .context("Wisdom data path has no parent directory")?;
    fs::create_dir_all(parent)?;

    match fs::rename(source, destination) {
        Ok(()) => {
            remove_empty_legacy_parents(source);
            return Ok(());
        }
        Err(_) if destination.exists() => return Ok(()),
        Err(rename_error) => {
            let staging = parent.join(format!(".Wisdom-migration-{}", std::process::id()));
            if staging.exists() {
                bail!(
                    "Migration staging directory already exists: {}",
                    staging.display()
                );
            }
            if let Err(copy_error) = copy_directory(source, &staging) {
                let _ = fs::remove_dir_all(&staging);
                return Err(copy_error).context(format!(
                    "Moving the data failed ({rename_error}); copying it also failed"
                ));
            }
            if let Err(commit_error) = fs::rename(&staging, destination) {
                let _ = fs::remove_dir_all(&staging);
                if destination.exists() {
                    return Ok(());
                }
                return Err(commit_error).context("Could not commit the migrated data");
            }
            // The destination is complete before the old copy is removed. If cleanup
            // fails, the new location remains usable and no user data is lost.
            let _ = fs::remove_dir_all(source);
            remove_empty_legacy_parents(source);
        }
    }
    Ok(())
}

fn copy_directory(source: &Path, destination: &Path) -> Result<()> {
    fs::create_dir(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let target = destination.join(entry.file_name());
        if file_type.is_dir() {
            copy_directory(&entry.path(), &target)?;
        } else if file_type.is_file() {
            fs::copy(entry.path(), target)?;
        } else {
            bail!(
                "The legacy data contains an unsupported symbolic link: {}",
                entry.path().display()
            );
        }
    }
    Ok(())
}

fn remove_empty_legacy_parents(source: &Path) {
    let Some(user_data) = source.parent() else {
        return;
    };
    let Some(publisher) = user_data.parent() else {
        return;
    };
    let _ = fs::remove_dir(user_data);
    let _ = fs::remove_dir(publisher);
}

pub fn windows_accent_color() -> String {
    let value = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey("Software\\Microsoft\\Windows\\DWM")
        .and_then(|key| key.get_value::<u32, _>("AccentColor"));
    match value {
        Ok(abgr) => format!(
            "#{:02X}{:02X}{:02X}",
            abgr & 0xff,
            (abgr >> 8) & 0xff,
            (abgr >> 16) & 0xff
        ),
        Err(_) => "#0078D4".to_owned(),
    }
}

pub fn prepare_storage(root: &Path) -> Result<()> {
    for folder in [
        "cache",
        "versions",
        "libraries",
        "assets/objects",
        "assets/indexes",
        "instances",
        "natives",
    ] {
        fs::create_dir_all(root.join(folder))?;
    }
    Ok(())
}

pub fn load_accounts(root: &Path) -> Result<(Option<AccountProfile>, Vec<AccountProfile>)> {
    let index = load_account_index(root)?;
    let active = index
        .active_uuid
        .as_ref()
        .and_then(|uuid| index.accounts.iter().find(|account| &account.uuid == uuid))
        .cloned();
    Ok((active, index.accounts))
}

pub fn load_auth() -> Result<AuthState> {
    let root = user_data_dir()?;
    let index = load_account_index(&root)?;
    let uuid = index.active_uuid.context("No Microsoft account selected")?;
    load_auth_for(&uuid)
}

pub fn save_auth(auth: &AuthState) -> Result<()> {
    let root = user_data_dir()?;
    prepare_storage(&root)?;
    write_auth_credentials(auth)?;
    let mut index = load_account_index(&root)?;
    let profile = AccountProfile::from(auth);
    if let Some(existing) = index
        .accounts
        .iter_mut()
        .find(|item| item.uuid == profile.uuid)
    {
        *existing = profile;
    } else {
        index.accounts.push(profile);
    }
    index.active_uuid = Some(auth.player_uuid.clone());
    save_account_index(&root, &index)
}

pub fn select_account(root: &Path, uuid: &str) -> Result<AuthState> {
    let mut index = load_account_index(root)?;
    if !index.accounts.iter().any(|account| account.uuid == uuid) {
        bail!("Account not found");
    }
    let auth = load_auth_for(uuid)?;
    index.active_uuid = Some(uuid.to_owned());
    save_account_index(root, &index)?;
    Ok(auth)
}

pub fn remove_account(root: &Path, uuid: &str) -> Result<Option<AuthState>> {
    let mut index = load_account_index(root)?;
    let original_len = index.accounts.len();
    index.accounts.retain(|account| account.uuid != uuid);
    if index.accounts.len() == original_len {
        bail!("Account not found");
    }
    if index.active_uuid.as_deref() == Some(uuid) {
        index.active_uuid = index.accounts.first().map(|account| account.uuid.clone());
    }
    let next_uuid = index.active_uuid.clone();
    let next_account = next_uuid.as_deref().map(load_auth_for).transpose()?;
    save_account_index(root, &index)?;
    for kind in ["refresh", "access", "profile"] {
        let _ = account_credential(kind, uuid)?.delete_credential();
    }
    Ok(next_account)
}

pub fn clear_auth() -> Result<()> {
    let root = user_data_dir()?;
    let index = load_account_index(&root)?;
    if let Some(uuid) = index.active_uuid {
        let _ = remove_account(&root, &uuid)?;
    }
    Ok(())
}

fn load_account_index(root: &Path) -> Result<AccountIndex> {
    let path = root.join("accounts.json");
    let mut index = match fs::read(&path) {
        Ok(contents) => serde_json::from_slice::<AccountIndex>(&contents)
            .context("Account list is corrupted")?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => AccountIndex::default(),
        Err(error) => return Err(error).context("Could not read the account list"),
    };

    if index.accounts.is_empty()
        && let Ok(legacy) = load_legacy_auth()
    {
        write_auth_credentials(&legacy)?;
        index.active_uuid = Some(legacy.player_uuid.clone());
        index.accounts.push(AccountProfile::from(&legacy));
        save_account_index(root, &index)?;
        clear_legacy_credentials();
    }

    if index
        .active_uuid
        .as_ref()
        .is_none_or(|uuid| !index.accounts.iter().any(|account| &account.uuid == uuid))
    {
        index.active_uuid = index.accounts.first().map(|account| account.uuid.clone());
    }
    Ok(index)
}

fn save_account_index(root: &Path, index: &AccountIndex) -> Result<()> {
    fs::create_dir_all(root)?;
    let temporary = root.join("accounts.json.tmp");
    let destination = root.join("accounts.json");
    fs::write(&temporary, serde_json::to_vec_pretty(index)?)?;
    replace_file(&temporary, &destination)
}

fn load_auth_for(uuid: &str) -> Result<AuthState> {
    validate_uuid(uuid)?;
    let refresh_token = account_credential("refresh", uuid)?
        .get_password()
        .context("Microsoft sign-in is missing")?;
    let minecraft_access_token = account_credential("access", uuid)?
        .get_password()
        .context("Minecraft sign-in is missing")?;
    let profile: SessionProfile = serde_json::from_str(
        &account_credential("profile", uuid)?
            .get_password()
            .context("Account profile is missing")?,
    )?;
    if profile.player_uuid != uuid {
        bail!("Account profile is corrupted");
    }
    Ok(AuthState {
        minecraft_access_token,
        microsoft_refresh_token: refresh_token,
        expires_at: profile.expires_at,
        player_name: profile.player_name,
        player_uuid: profile.player_uuid,
        skin_url: profile.skin_url,
    })
}

fn write_auth_credentials(auth: &AuthState) -> Result<()> {
    validate_uuid(&auth.player_uuid)?;
    account_credential("refresh", &auth.player_uuid)?
        .set_password(&auth.microsoft_refresh_token)?;
    account_credential("access", &auth.player_uuid)?.set_password(&auth.minecraft_access_token)?;
    let profile = SessionProfile {
        expires_at: auth.expires_at,
        player_name: auth.player_name.clone(),
        player_uuid: auth.player_uuid.clone(),
        skin_url: auth.skin_url.clone(),
    };
    account_credential("profile", &auth.player_uuid)?
        .set_password(&serde_json::to_string(&profile)?)?;
    Ok(())
}

fn load_legacy_auth() -> Result<AuthState> {
    let refresh_token = credential("minecraft-refresh-token")?.get_password()?;
    let minecraft_access_token = credential("minecraft-access-token")?.get_password()?;
    let profile: SessionProfile =
        serde_json::from_str(&credential("minecraft-profile")?.get_password()?)?;
    Ok(AuthState {
        minecraft_access_token,
        microsoft_refresh_token: refresh_token,
        expires_at: profile.expires_at,
        player_name: profile.player_name,
        player_uuid: profile.player_uuid,
        skin_url: profile.skin_url,
    })
}

fn clear_legacy_credentials() {
    for name in [
        "minecraft-refresh-token",
        "minecraft-access-token",
        "minecraft-profile",
    ] {
        if let Ok(entry) = credential(name) {
            let _ = entry.delete_credential();
        }
    }
}

fn validate_uuid(uuid: &str) -> Result<()> {
    if uuid.is_empty()
        || uuid.len() > 40
        || !uuid
            .chars()
            .all(|character| character.is_ascii_hexdigit() || character == '-')
    {
        bail!("Invalid account ID");
    }
    Ok(())
}

fn account_credential(kind: &str, uuid: &str) -> Result<keyring::Entry> {
    validate_uuid(uuid)?;
    credential(&format!("minecraft-{kind}:{uuid}"))
}

pub fn load_settings(root: &Path) -> LauncherSettings {
    fs::read_to_string(root.join("settings.json"))
        .ok()
        .and_then(|contents| serde_json::from_str::<LauncherSettings>(&contents).ok())
        .unwrap_or_default()
}

pub fn save_settings(root: &Path, settings: &LauncherSettings) -> Result<()> {
    let temporary = root.join("settings.json.tmp");
    let destination = root.join("settings.json");
    fs::write(&temporary, serde_json::to_vec_pretty(settings)?)?;
    replace_file(&temporary, &destination)
}

fn credential(name: &str) -> Result<keyring::Entry> {
    Ok(keyring::Entry::new("Wisdom Minecraft Launcher", name)?)
}

fn replace_file(source: &Path, destination: &Path) -> Result<()> {
    if destination.exists() {
        fs::remove_file(destination)?;
    }
    fs::rename(source, destination)?;
    Ok(())
}

pub fn read_json<T: for<'a> Deserialize<'a>>(path: &Path) -> Result<T> {
    serde_json::from_reader(
        File::open(path).with_context(|| format!("could not open {}", path.display()))?,
    )
    .with_context(|| format!("could not read {}", path.display()))
}

pub fn http() -> Result<Client> {
    Ok(Client::builder()
        .user_agent(USER_AGENT)
        .connect_timeout(Duration::from_secs(6))
        .timeout(Duration::from_secs(30))
        .build()?)
}

#[cfg(test)]
mod tests {
    use super::{migrate_legacy_data, validate_uuid};
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn validates_account_ids() {
        assert!(validate_uuid("069a79f444e94726a5befca90e38aaf5").is_ok());
        assert!(validate_uuid("../credential").is_err());
    }

    #[test]
    fn migrates_legacy_data_without_losing_nested_files() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "wisdom-storage-migration-{}-{unique}",
            std::process::id()
        ));
        let source = root.join("roaming").join("Wisdom");
        let destination = root.join("local").join("Zorblock").join("Wisdom");
        fs::create_dir_all(source.join("instances").join("test")).unwrap();
        fs::write(
            source.join("instances").join("test").join("instance.json"),
            b"migration-test",
        )
        .unwrap();

        migrate_legacy_data(&source, &destination).unwrap();

        assert!(!source.exists());
        assert_eq!(
            fs::read(
                destination
                    .join("instances")
                    .join("test")
                    .join("instance.json")
            )
            .unwrap(),
            b"migration-test"
        );
        fs::remove_dir_all(root).unwrap();
    }
}
