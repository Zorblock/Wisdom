use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Instance {
    pub id: String,
    pub name: String,
    pub version: String,
    #[serde(default, alias = "ram_mb")]
    pub ram_mb: Option<u32>,
    #[serde(default, alias = "jvm_args")]
    pub jvm_args: Option<String>,
    #[serde(default, alias = "game_args")]
    pub game_args: Option<String>,
    #[serde(default)]
    pub last_played: Option<DateTime<Utc>>,
}

pub fn load_or_create(root: &Path, default_version: &str) -> Result<Vec<Instance>> {
    let directory = root.join("instances");
    fs::create_dir_all(&directory)?;
    let mut instances = Vec::new();
    for entry in fs::read_dir(&directory)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let path = entry.path().join("instance.json");
        let Ok(contents) = fs::read_to_string(path) else {
            continue;
        };
        let Ok(instance) = serde_json::from_str::<Instance>(&contents) else {
            continue;
        };
        if valid_id(&instance.id) && valid_name(&instance.name) {
            instances.push(instance);
        }
    }
    instances.sort_by(|left, right| {
        right
            .last_played
            .cmp(&left.last_played)
            .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
    });
    if instances.is_empty() {
        let instance = Instance {
            id: "vanilla".into(),
            name: "Vanilla".into(),
            version: default_version.into(),
            ram_mb: None,
            jvm_args: None,
            game_args: None,
            last_played: None,
        };
        save(root, &instance)?;
        instances.push(instance);
    }
    Ok(instances)
}

pub fn create(root: &Path, name: &str, version: &str, existing: &[Instance]) -> Result<Instance> {
    let name = clean_name(name)?;
    let base_id = slug(&name);
    let mut id = base_id.clone();
    let mut number = 2;
    while existing.iter().any(|instance| instance.id == id)
        || root.join("instances").join(&id).exists()
    {
        id = format!("{base_id}-{number}");
        number += 1;
    }
    let instance = Instance {
        id,
        name,
        version: validate_version(version)?,
        ram_mb: None,
        jvm_args: None,
        game_args: None,
        last_played: None,
    };
    save(root, &instance)?;
    Ok(instance)
}

pub fn update(
    root: &Path,
    id: &str,
    name: &str,
    version: &str,
    ram_mb: Option<u32>,
    jvm_args: Option<String>,
    game_args: Option<String>,
) -> Result<Instance> {
    let mut instance = load(root, id)?;
    instance.name = clean_name(name)?;
    instance.version = validate_version(version)?;
    instance.ram_mb = ram_mb.map(|value| value.clamp(512, 65_536));
    instance.jvm_args = normalize_optional(jvm_args);
    instance.game_args = normalize_optional(game_args);
    save(root, &instance)?;
    Ok(instance)
}

pub fn mark_launched(root: &Path, id: &str, version: &str) -> Result<Instance> {
    let mut instance = load(root, id)?;
    instance.version = validate_version(version)?;
    instance.last_played = Some(Utc::now());
    save(root, &instance)?;
    Ok(instance)
}

pub fn delete(root: &Path, id: &str) -> Result<()> {
    if !valid_id(id) {
        bail!("Ungültige Instanz-ID");
    }
    let directory = root.join("instances").join(id);
    let instances_root = root.join("instances");
    if directory.parent() != Some(instances_root.as_path()) {
        bail!("Ungültiger Instanzpfad");
    }
    if !directory.join("instance.json").is_file() {
        bail!("Instanz wurde nicht gefunden");
    }
    fs::remove_dir_all(&directory)
        .with_context(|| format!("Instanz {id} konnte nicht gelöscht werden"))
}

pub fn save(root: &Path, instance: &Instance) -> Result<()> {
    if !valid_id(&instance.id) {
        bail!("Ungültige Instanz-ID");
    }
    let directory = game_dir(root, instance);
    fs::create_dir_all(&directory)?;
    let temporary = directory.join("instance.json.tmp");
    fs::write(&temporary, serde_json::to_vec_pretty(instance)?)
        .with_context(|| format!("Instanz {} konnte nicht gespeichert werden", instance.name))?;
    replace_file(&temporary, &directory.join("instance.json"))
}

pub fn load(root: &Path, id: &str) -> Result<Instance> {
    if !valid_id(id) {
        bail!("Ungültige Instanz-ID");
    }
    let instance: Instance = serde_json::from_slice(
        &fs::read(root.join("instances").join(id).join("instance.json"))
            .context("Instanz wurde nicht gefunden")?,
    )?;
    if instance.id != id {
        bail!("Instanzdaten sind beschädigt");
    }
    Ok(instance)
}

pub fn game_dir(root: &Path, instance: &Instance) -> PathBuf {
    root.join("instances").join(&instance.id)
}

fn clean_name(name: &str) -> Result<String> {
    let name = name.trim();
    if !valid_name(name) {
        bail!("Der Name muss zwischen 1 und 48 Zeichen lang sein");
    }
    Ok(name.to_owned())
}

fn valid_name(name: &str) -> bool {
    !name.trim().is_empty() && name.chars().count() <= 48 && !name.chars().any(char::is_control)
}

fn validate_version(version: &str) -> Result<String> {
    let version = version.trim();
    if version.is_empty()
        || version.len() > 64
        || !version.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_')
        })
    {
        bail!("Ungültige Minecraft-Version");
    }
    Ok(version.to_owned())
}

fn slug(name: &str) -> String {
    let value = name
        .to_ascii_lowercase()
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character
            } else {
                '-'
            }
        })
        .collect::<String>();
    let value = value
        .split('-')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    if value.is_empty() {
        "instance".into()
    } else {
        value
    }
}

fn normalize_optional(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        let trimmed = value.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_owned())
    })
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '-')
}

fn replace_file(source: &Path, destination: &Path) -> Result<()> {
    if destination.exists() {
        fs::remove_file(destination)?;
    }
    fs::rename(source, destination)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_safe_slugs() {
        assert_eq!(slug(" Better MC ++ "), "better-mc");
        assert_eq!(slug("世界"), "instance");
    }

    #[test]
    fn validates_versions() {
        assert!(validate_version("1.21.8").is_ok());
        assert!(validate_version("../bad").is_err());
    }
}
