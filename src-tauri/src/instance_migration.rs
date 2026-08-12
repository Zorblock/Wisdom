use crate::RuntimeState;
use crate::instances::{self, Instance};
use crate::minecraft;
use crate::modloaders::{self, ModLoader};
use crate::modrinth::{self, MigrationPreview};
use crate::storage;
use anyhow::{Context, Result, bail};
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Emitter, State};

const BACKUP_FOLDER: &str = "version-migration-backup";

#[tauri::command]
pub async fn preview_instance_migration(
    state: State<'_, RuntimeState>,
    instance_id: String,
    version: String,
    loader: ModLoader,
) -> Result<MigrationPreview, String> {
    ensure_stopped(&state, &instance_id)?;
    run_blocking(move || {
        let root = storage::user_data_dir()?;
        ensure_known_minecraft_version(&root, &version)?;
        let instance = instances::load(&root, &instance_id)?;
        let managed_mod_count = modrinth::managed_mod_count(&root, &instance)?;
        if managed_mod_count == 0 {
            return Ok(MigrationPreview {
                from_version: instance.version,
                to_version: version,
                loader: loader.pretty_name().to_owned(),
                managed_mod_count: 0,
                changes: Vec::new(),
                dependency_count: 0,
                unavailable: Vec::new(),
            });
        }
        if !loader.supports_mods() {
            return Ok(MigrationPreview {
                from_version: instance.version,
                to_version: version,
                loader: loader.pretty_name().to_owned(),
                managed_mod_count,
                changes: Vec::new(),
                dependency_count: 0,
                unavailable: vec![
                    "Vanilla cannot load managed mods. Remove the installed mods first.".to_owned(),
                ],
            });
        }
        modloaders::check_version_support(loader, &version)?;
        modrinth::preview_migration(&root, &instance_id, &version, loader)
    })
    .await
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn migrate_instance(
    app: AppHandle,
    state: State<'_, RuntimeState>,
    instance_id: String,
    name: String,
    version: String,
    loader: ModLoader,
    ram_mb: Option<u32>,
    jvm_args: Option<String>,
    game_args: Option<String>,
) -> Result<Instance, String> {
    ensure_stopped(&state, &instance_id)?;
    let _ = app.emit("status", "Checking version and mod compatibility...");
    let result = run_blocking(move || {
        let root = storage::user_data_dir()?;
        migrate(
            &root,
            &instance_id,
            &name,
            &version,
            loader,
            ram_mb,
            jvm_args,
            game_args,
        )
    })
    .await;
    if result.is_ok() {
        let _ = app.emit("status", "Instance migration completed.");
    }
    result
}

#[allow(clippy::too_many_arguments)]
fn migrate(
    root: &Path,
    instance_id: &str,
    name: &str,
    version: &str,
    loader: ModLoader,
    ram_mb: Option<u32>,
    jvm_args: Option<String>,
    game_args: Option<String>,
) -> Result<Instance> {
    let original = instances::load(root, instance_id)?;
    ensure_known_minecraft_version(root, version)?;
    if original.version == version && original.loader == loader {
        return instances::update(
            root,
            instance_id,
            name,
            version,
            loader,
            ram_mb,
            jvm_args,
            game_args,
        );
    }
    if !modrinth::has_installed_mods(root, &original)? {
        return instances::update(
            root,
            instance_id,
            name,
            version,
            loader,
            ram_mb,
            jvm_args,
            game_args,
        );
    }
    modloaders::check_version_support(loader, version)?;
    let preview = modrinth::preview_migration(root, instance_id, version, loader)?;
    if !preview.unavailable.is_empty() {
        bail!(
            "Migration blocked because compatible versions are unavailable:\n{}",
            preview.unavailable.join("\n")
        );
    }

    let backup = create_backup(root, &original)?;
    let migration = (|| {
        instances::update(
            root,
            instance_id,
            name,
            version,
            loader,
            ram_mb,
            jvm_args,
            game_args,
        )?;
        modrinth::update_all(root, instance_id)
            .context("Could not install all mods for the target version")?;
        instances::load(root, instance_id)
    })();

    match migration {
        Ok(instance) => {
            let _ =
                remove_scoped_directory(&backup, backup.parent().unwrap_or(root), BACKUP_FOLDER);
            Ok(instance)
        }
        Err(error) => {
            restore_backup(root, &original, &backup)
                .context("Migration failed and the automatic rollback also failed")?;
            Err(error.context("The migration was rolled back; the original instance is intact"))
        }
    }
}

fn create_backup(root: &Path, instance: &Instance) -> Result<PathBuf> {
    let instance_dir = instances::game_dir(root, instance);
    let wisdom_dir = instance_dir.join(".wisdom");
    let backup = wisdom_dir.join(BACKUP_FOLDER);
    if backup.exists() {
        bail!(
            "An unfinished migration backup exists at {}",
            backup.display()
        );
    }
    fs::create_dir_all(&backup)?;
    let result = (|| {
        fs::copy(
            instance_dir.join("instance.json"),
            backup.join("instance.json"),
        )?;
        let manifest = wisdom_dir.join("mods.json");
        if manifest.is_file() {
            fs::copy(manifest, backup.join("mods.json"))?;
        }
        let mods = instance_dir.join("mods");
        if mods.is_dir() {
            copy_directory(&mods, &backup.join("mods"))?;
        }
        Ok::<_, anyhow::Error>(())
    })();
    if let Err(error) = result {
        let _ = remove_scoped_directory(&backup, &wisdom_dir, BACKUP_FOLDER);
        return Err(error.context("Could not create a safe migration backup"));
    }
    Ok(backup)
}

fn restore_backup(root: &Path, original: &Instance, backup: &Path) -> Result<()> {
    let instance_dir = instances::game_dir(root, original);
    let mods = instance_dir.join("mods");
    if mods.exists() {
        remove_scoped_directory(&mods, &instance_dir, "mods")?;
    }
    let backup_mods = backup.join("mods");
    if backup_mods.is_dir() {
        fs::rename(backup_mods, &mods)?;
    }
    let manifest = instance_dir.join(".wisdom").join("mods.json");
    let backup_manifest = backup.join("mods.json");
    if backup_manifest.is_file() {
        fs::copy(backup_manifest, manifest)?;
    } else if manifest.exists() {
        fs::remove_file(manifest)?;
    }
    instances::save(root, original)?;
    let backup_parent = backup.parent().context("Migration backup has no parent")?;
    remove_scoped_directory(backup, backup_parent, BACKUP_FOLDER)?;
    Ok(())
}

fn copy_directory(source: &Path, destination: &Path) -> Result<()> {
    fs::create_dir_all(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let target = destination.join(entry.file_name());
        if file_type.is_dir() {
            copy_directory(&entry.path(), &target)?;
        } else if file_type.is_file() {
            fs::copy(entry.path(), target)?;
        } else {
            bail!("Migration backup does not support links inside the mods folder");
        }
    }
    Ok(())
}

fn ensure_known_minecraft_version(root: &Path, version: &str) -> Result<()> {
    let (_, versions) = minecraft::load_versions(root)?;
    if !versions.iter().any(|entry| entry.id == version) {
        bail!("Minecraft version {version} was not found");
    }
    Ok(())
}

fn remove_scoped_directory(path: &Path, parent: &Path, expected_name: &str) -> Result<()> {
    if path.parent() != Some(parent) || path.file_name() != Some(OsStr::new(expected_name)) {
        bail!("Refusing to remove an invalid migration path");
    }
    fs::remove_dir_all(path)?;
    Ok(())
}

fn ensure_stopped(state: &State<'_, RuntimeState>, instance_id: &str) -> Result<(), String> {
    if state
        .running_instances
        .lock()
        .map_err(|_| "Could not read instance status".to_owned())?
        .contains(instance_id)
    {
        return Err("Stop Minecraft before changing its version or mod loader".to_owned());
    }
    Ok(())
}

async fn run_blocking<T, F>(operation: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> anyhow::Result<T> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(operation)
        .await
        .map_err(|error| format!("Internal error: {error}"))?
        .map_err(|error| format!("{error:#}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copies_nested_migration_backups() {
        let parent = std::env::temp_dir();
        let name = format!("wisdom-migration-test-{}", rand::random::<u64>());
        let test_root = parent.join(&name);
        let source = test_root.join("source");
        let destination = test_root.join("destination");
        fs::create_dir_all(source.join("nested")).unwrap();
        fs::write(source.join("nested").join("example.jar"), b"mod").unwrap();
        copy_directory(&source, &destination).unwrap();
        assert_eq!(
            fs::read(destination.join("nested").join("example.jar")).unwrap(),
            b"mod"
        );
        remove_scoped_directory(&test_root, &parent, &name).unwrap();
    }

    #[test]
    fn refuses_unscoped_recursive_removal() {
        assert!(
            remove_scoped_directory(Path::new("mods"), Path::new("elsewhere"), "mods").is_err()
        );
    }
}
