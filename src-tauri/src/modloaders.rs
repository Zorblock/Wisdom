use crate::downloads::{DownloadJob, download_jobs};
use crate::instances::Instance;
use crate::minecraft::{LaunchOptions, ProgressReporter, parse_extra_arguments};
use crate::storage::{AuthState, http};
use anyhow::{Context, Result, bail};
use mc_launcher_core::account::Account;
use mc_launcher_core::command::builder::LaunchOptions as CoreLaunchOptions;
use mc_launcher_core::core::version::VersionJson;
use mc_launcher_core::install::loader::{
    InstallerInvocation, run_loader_installer, write_loader_profile,
};
use mc_launcher_core::launcher::Launcher;
use mc_launcher_core::loader::{LoaderKind, fabric, forge, neoforge, quilt};
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ModLoader {
    #[default]
    Vanilla,
    Fabric,
    Quilt,
    Forge,
    Neoforge,
}

impl ModLoader {
    pub fn supports_mods(self) -> bool {
        self != Self::Vanilla
    }

    pub fn modrinth_name(self) -> Option<&'static str> {
        match self {
            Self::Vanilla => None,
            Self::Fabric => Some("fabric"),
            Self::Quilt => Some("quilt"),
            Self::Forge => Some("forge"),
            Self::Neoforge => Some("neoforge"),
        }
    }

    pub(crate) fn pretty_name(self) -> &'static str {
        match self {
            Self::Vanilla => "Vanilla",
            Self::Fabric => "Fabric",
            Self::Quilt => "Quilt",
            Self::Forge => "Forge",
            Self::Neoforge => "NeoForge",
        }
    }
}

pub struct ModdedLaunch {
    pub child: Child,
    pub loader_version: String,
}

pub fn check_version_support(loader: ModLoader, game_version: &str) -> Result<()> {
    match loader {
        ModLoader::Vanilla => bail!("Managed mods cannot be migrated to Vanilla"),
        ModLoader::Fabric => {
            let version = fabric::latest_stable_loader(&fabric::list_loader_versions()?)?
                .version
                .clone();
            fabric::fetch_profile(game_version, &version)
                .with_context(|| format!("Fabric does not support Minecraft {game_version}"))?;
        }
        ModLoader::Quilt => {
            let version = quilt::latest_loader(&quilt::list_loader_versions()?)?
                .version
                .clone();
            quilt::fetch_profile(game_version, &version)
                .with_context(|| format!("Quilt does not support Minecraft {game_version}"))?;
        }
        ModLoader::Forge => {
            let versions = forge::list_forge_versions()?;
            forge::latest_for_minecraft(&versions, game_version)
                .with_context(|| format!("Forge does not support Minecraft {game_version}"))?;
        }
        ModLoader::Neoforge => {
            let versions = neoforge::list_neoforge_versions()?;
            neoforge::latest_for_minecraft(&versions, game_version)
                .with_context(|| format!("NeoForge does not support Minecraft {game_version}"))?;
        }
    }
    Ok(())
}

pub fn install_and_launch(
    root: &Path,
    game_dir: &Path,
    instance: &Instance,
    auth: &AuthState,
    options: &LaunchOptions,
    report: &(dyn Fn(String) + Send + Sync),
    progress: &ProgressReporter<'_>,
) -> Result<ModdedLaunch> {
    if !instance.loader.supports_mods() {
        bail!("A mod loader was not selected");
    }

    let launcher = Launcher::new(root);
    report(format!(
        "Preparing {} for Minecraft {}...",
        instance.loader.pretty_name(),
        instance.version
    ));
    let client = http()?;
    let vanilla_progress = |value: f32, message: String| {
        progress(value * 0.42, message);
    };
    crate::minecraft_install::install_vanilla(&client, root, &instance.version, &vanilla_progress)
        .context("Could not install the Minecraft base version")?;
    let vanilla = launcher
        .load_version(&instance.version)
        .context("Could not load the Minecraft version profile")?;
    let java_major = vanilla
        .java_version
        .as_ref()
        .map(|version| version.major_version.max(8) as u32)
        .unwrap_or(8);
    let java_progress = |value: f32, message: String| progress(0.42 + value * 0.04, message);
    let java = crate::runtime::ensure_java(root, java_major, &java_progress)?;

    progress(
        0.45,
        format!("Installing {}...", instance.loader.pretty_name()),
    );
    let (profile_id, loader_version) =
        install_loader(&client, &launcher, root, instance, &java, progress)?;
    let version = launcher
        .load_version(&profile_id)
        .with_context(|| format!("Could not load the {profile_id} profile"))?;
    let natives_dir = root.join("versions").join(&profile_id).join("natives");
    let account = Account::Microsoft {
        username: auth.player_name.clone(),
        uuid: auth.player_uuid.clone(),
        access_token: auth.minecraft_access_token.clone(),
    };
    let command = launcher
        .build_launch_command_from_version(
            &version,
            CoreLaunchOptions {
                account,
                java_executable: Some(java.clone()),
                game_directory: Some(game_dir.to_path_buf()),
                natives_directory: Some(natives_dir.clone()),
                launcher_name: "Wisdom".to_owned(),
                launcher_version: env!("CARGO_PKG_VERSION").to_owned(),
                ..Default::default()
            },
        )
        .context("Could not create the modded Minecraft launch command")?;

    let mut args = Vec::new();
    args.push(format!("-Xmx{}M", options.ram_mb));
    args.extend(parse_extra_arguments(&options.jvm_args)?);
    if let Some(logging_argument) = prepare_logging(root, &version, progress)? {
        args.push(logging_argument);
    }
    args.extend(command.args);
    replace_flag_value(
        &mut args,
        "--version",
        format!("{} / Wisdom", instance.version),
    );
    replace_flag_value(&mut args, "--clientId", options.client_id.clone());
    args.extend(parse_extra_arguments(&options.game_args)?);

    #[cfg(windows)]
    let executable = if options.open_console {
        command.executable
    } else {
        let javaw = command.executable.with_file_name("javaw.exe");
        if javaw.exists() {
            javaw
        } else {
            command.executable
        }
    };
    #[cfg(not(windows))]
    let executable = command.executable;

    let mut process = Command::new(executable);
    process.args(args).current_dir(&command.working_dir);
    for (key, value) in command.env {
        process.env(key, value);
    }
    if let Some(path) = std::env::var_os("PATH") {
        let paths = std::env::split_paths(&path).chain([natives_dir]);
        if let Ok(path) = std::env::join_paths(paths) {
            process.env("PATH", path);
        }
    }
    if options.open_console {
        process.stdout(Stdio::piped()).stderr(Stdio::piped());
    } else {
        process.stdout(Stdio::null()).stderr(Stdio::null());
    }
    #[cfg(windows)]
    process.creation_flags(0x0800_0000);
    progress(1.0, "Starting modded Minecraft...".to_owned());
    let child = process
        .spawn()
        .context("Could not start modded Minecraft")?;
    Ok(ModdedLaunch {
        child,
        loader_version,
    })
}

fn install_loader(
    client: &Client,
    launcher: &Launcher,
    root: &Path,
    instance: &Instance,
    java: &Path,
    progress: &ProgressReporter<'_>,
) -> Result<(String, String)> {
    let requested = instance.loader_version.clone();
    match instance.loader {
        ModLoader::Vanilla => bail!("A mod loader was not selected"),
        ModLoader::Fabric => {
            let loader_version = match requested {
                Some(version) => version,
                None => fabric::latest_stable_loader(&fabric::list_loader_versions()?)?
                    .version
                    .clone(),
            };
            let profile = fabric::fetch_profile(&instance.version, &loader_version)
                .context("Fabric does not support this Minecraft version")?;
            let profile_progress = |value: f32, message: String| {
                progress(0.46 + value * 0.42, message);
            };
            let profile_id =
                write_and_install_profile(client, launcher, root, &profile, &profile_progress)?;
            Ok((profile_id, loader_version))
        }
        ModLoader::Quilt => {
            let loader_version = match requested {
                Some(version) => version,
                None => quilt::latest_loader(&quilt::list_loader_versions()?)?
                    .version
                    .clone(),
            };
            let profile = quilt::fetch_profile(&instance.version, &loader_version)
                .context("Quilt does not support this Minecraft version")?;
            let profile_progress = |value: f32, message: String| {
                progress(0.46 + value * 0.42, message);
            };
            let profile_id =
                write_and_install_profile(client, launcher, root, &profile, &profile_progress)?;
            Ok((profile_id, loader_version))
        }
        ModLoader::Forge => {
            let loader_version = match requested {
                Some(version) => version,
                None => {
                    let versions = forge::list_forge_versions()?;
                    forge::latest_for_minecraft(&versions, &instance.version)?.to_owned()
                }
            };
            let installer_progress = |value: f32, message: String| {
                progress(0.46 + value * 0.12, message);
            };
            let installer = download_installer(
                root,
                "forge",
                &loader_version,
                &forge::installer_url(&loader_version),
                &installer_progress,
            )?;
            progress(0.59, "Installing Forge...".to_owned());
            run_loader_installer(&InstallerInvocation {
                loader: LoaderKind::Forge,
                java_executable: java.to_path_buf(),
                installer_path: installer,
                minecraft_dir: root.to_path_buf(),
            })?;
            let profile_id = forge::forge_installed_version_id(&loader_version)?;
            let profile = launcher.load_version(&profile_id)?;
            let profile_progress = |value: f32, message: String| {
                progress(0.60 + value * 0.28, message);
            };
            crate::minecraft_install::install_profile(client, root, &profile, &profile_progress)?;
            Ok((profile_id, loader_version))
        }
        ModLoader::Neoforge => {
            let loader_version = match requested {
                Some(version) => version,
                None => {
                    let versions = neoforge::list_neoforge_versions()?;
                    neoforge::latest_for_minecraft(&versions, &instance.version)?.to_owned()
                }
            };
            let installer_progress = |value: f32, message: String| {
                progress(0.46 + value * 0.12, message);
            };
            let installer = download_installer(
                root,
                "neoforge",
                &loader_version,
                &neoforge::installer_url(&loader_version),
                &installer_progress,
            )?;
            progress(0.59, "Installing NeoForge...".to_owned());
            run_loader_installer(&InstallerInvocation {
                loader: LoaderKind::NeoForge,
                java_executable: java.to_path_buf(),
                installer_path: installer,
                minecraft_dir: root.to_path_buf(),
            })?;
            let profile_id =
                neoforge::neoforge_installed_version_id(&instance.version, &loader_version);
            let profile = launcher.load_version(&profile_id)?;
            let profile_progress = |value: f32, message: String| {
                progress(0.60 + value * 0.28, message);
            };
            crate::minecraft_install::install_profile(client, root, &profile, &profile_progress)?;
            Ok((profile_id, loader_version))
        }
    }
}

fn write_and_install_profile(
    client: &Client,
    launcher: &Launcher,
    root: &Path,
    profile: &VersionJson,
    progress: &ProgressReporter<'_>,
) -> Result<String> {
    let profile_id = profile
        .id
        .clone()
        .context("Loader profile has no version ID")?;
    write_loader_profile(root, profile)?;
    let merged = launcher.load_version(&profile_id)?;
    crate::minecraft_install::install_profile(client, root, &merged, progress)?;
    Ok(profile_id)
}

fn download_installer(
    root: &Path,
    loader: &str,
    version: &str,
    url: &str,
    progress: &ProgressReporter<'_>,
) -> Result<PathBuf> {
    let destination = root
        .join("versions")
        .join(".installers")
        .join(format!("{loader}-{version}-installer.jar"));
    if destination.is_file() && destination.metadata()?.len() > 0 {
        return Ok(destination);
    }
    download_jobs(
        &http()?,
        vec![DownloadJob {
            url: url.to_owned(),
            destination: destination.clone(),
            checksum: None,
        }],
        &format!("{loader} installer"),
        progress,
    )?;
    Ok(destination)
}

fn prepare_logging(
    root: &Path,
    version: &VersionJson,
    progress: &ProgressReporter<'_>,
) -> Result<Option<String>> {
    let Some(logging) = version.logging.get("client") else {
        return Ok(None);
    };
    let path = root
        .join("assets")
        .join("log_configs")
        .join(&logging.file.id);
    if !path.is_file() || sha1_file(&path)? != logging.file.sha1 {
        fs::create_dir_all(path.parent().context("Log config path has no parent")?)?;
        progress(0.9, "Downloading log configuration...".to_owned());
        let bytes = http()?
            .get(&logging.file.url)
            .send()?
            .error_for_status()?
            .bytes()?;
        let actual = format!("{:x}", Sha1::digest(&bytes));
        if !actual.eq_ignore_ascii_case(&logging.file.sha1) {
            bail!("Log configuration checksum does not match");
        }
        fs::write(&path, bytes)?;
    }
    Ok(Some(
        logging.argument.replace("${path}", &path.to_string_lossy()),
    ))
}

fn sha1_file(path: &Path) -> Result<String> {
    let mut input = File::open(path)?;
    let mut hasher = Sha1::new();
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

fn replace_flag_value(arguments: &mut [String], flag: &str, value: String) {
    if let Some(index) = arguments.iter().position(|argument| argument == flag)
        && let Some(current) = arguments.get_mut(index + 1)
    {
        *current = value;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loader_names_match_modrinth_facets() {
        assert_eq!(ModLoader::Fabric.modrinth_name(), Some("fabric"));
        assert_eq!(ModLoader::Neoforge.modrinth_name(), Some("neoforge"));
        assert_eq!(ModLoader::Vanilla.modrinth_name(), None);
    }

    #[test]
    fn replaces_named_argument_values() {
        let mut args = vec!["--version".to_owned(), "old".to_owned()];
        replace_flag_value(&mut args, "--version", "new".to_owned());
        assert_eq!(args[1], "new");
    }
}
