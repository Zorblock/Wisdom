use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Instance {
    pub id: String,
    pub name: String,
    pub version: String,
}

pub fn load_or_create(root: &Path, default_version: &str) -> Result<Vec<Instance>> {
    let directory = root.join("instances");
    fs::create_dir_all(&directory)?;
    let mut instances = Vec::new();
    for entry in fs::read_dir(&directory)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() { continue; }
        let path = entry.path().join("instance.json");
        let Ok(contents) = fs::read_to_string(path) else { continue; };
        let Ok(instance) = serde_json::from_str::<Instance>(&contents) else { continue; };
        if valid_id(&instance.id) { instances.push(instance); }
    }
    instances.sort_by(|left, right| left.name.to_lowercase().cmp(&right.name.to_lowercase()));
    if instances.is_empty() {
        let instance = Instance { id: "vanilla".into(), name: "Vanilla".into(), version: default_version.into() };
        save(root, &instance)?;
        instances.push(instance);
    }
    Ok(instances)
}

pub fn create(root: &Path, version: &str, existing: &[Instance]) -> Result<Instance> {
    let mut number = existing.len() + 1;
    let (id, name) = loop {
        let id = format!("instance-{number}");
        if existing.iter().all(|instance| instance.id != id) { break (id, format!("Instance {number}")); }
        number += 1;
    };
    let instance = Instance { id, name, version: version.into() };
    save(root, &instance)?;
    Ok(instance)
}

pub fn save(root: &Path, instance: &Instance) -> Result<()> {
    if !valid_id(&instance.id) { anyhow::bail!("Invalid instance id"); }
    let directory = game_dir(root, instance);
    fs::create_dir_all(&directory)?;
    fs::write(directory.join("instance.json"), serde_json::to_vec_pretty(instance)?)
        .with_context(|| format!("Could not save instance {}", instance.name))
}

pub fn game_dir(root: &Path, instance: &Instance) -> PathBuf {
    root.join("instances").join(&instance.id)
}

fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.chars().all(|character| character.is_ascii_alphanumeric() || character == '-')
}
