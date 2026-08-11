use crate::minecraft::ProgressReporter;
use crate::storage::http;
use anyhow::{Context, Result, bail};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

#[derive(Deserialize)]
struct AdoptiumRelease {
    binary: AdoptiumBinary,
}
#[derive(Deserialize)]
struct AdoptiumBinary {
    package: AdoptiumPackage,
}
#[derive(Clone, Deserialize)]
struct AdoptiumPackage {
    link: String,
    checksum: String,
}

pub fn ensure_java(root: &Path, major: u32, progress: &ProgressReporter) -> Result<PathBuf> {
    let java_root = root.join("java");
    let target = java_root.join(major.to_string());
    if let Some(java) = find_java(&target) {
        return Ok(java);
    }
    fs::create_dir_all(&java_root)?;
    progress(0.0, format!("Java {major} wird vorbereitet …"));

    let api = format!(
        "https://api.adoptium.net/v3/assets/latest/{major}/hotspot?architecture=x64&image_type=jre&os=windows&vendor=eclipse"
    );
    let release: Vec<AdoptiumRelease> = http()?.get(api).send()?.error_for_status()?.json()?;
    let package = release
        .first()
        .context("Für diese Minecraft-Version ist keine passende Java-Laufzeit verfügbar")?
        .binary
        .package
        .clone();
    let archive = root
        .join("cache")
        .join(format!("temurin-{major}-windows-x64.zip"));
    if !archive.exists()
        || sha256_file(&archive).is_ok_and(|actual| !actual.eq_ignore_ascii_case(&package.checksum))
    {
        download_archive(&package, &archive, progress)?;
    }
    if !sha256_file(&archive)?.eq_ignore_ascii_case(&package.checksum) {
        bail!("Prüfsumme des Java-Archivs stimmt nicht überein")
    }

    let temporary = java_root.join(format!(".{major}.installing"));
    if temporary.exists() {
        fs::remove_dir_all(&temporary)?;
    }
    fs::create_dir_all(&temporary)?;
    progress(0.0, format!("Java {major} wird installiert …"));
    extract_zip(&archive, &temporary)?;
    if target.exists() {
        fs::remove_dir_all(&target)?;
    }
    fs::rename(&temporary, &target)?;
    let java = find_java(&target).context("Das Java-Archiv enthält keine java.exe")?;
    progress(1.0, format!("Java {major} ist bereit"));
    Ok(java)
}

fn download_archive(
    package: &AdoptiumPackage,
    destination: &Path,
    progress: &ProgressReporter,
) -> Result<()> {
    let mut response = http()?.get(&package.link).send()?.error_for_status()?;
    let total = response.content_length().unwrap_or(0);
    let temporary = destination.with_extension("download");
    let mut output = File::create(&temporary)?;
    let mut hasher = Sha256::new();
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
        let amount = if total == 0 {
            0.0
        } else {
            received as f32 / total as f32
        };
        progress(
            amount,
            format!("Java wird heruntergeladen · {}%", (amount * 100.0) as u32),
        );
    }
    let actual = format!("{:x}", hasher.finalize());
    if !actual.eq_ignore_ascii_case(&package.checksum) {
        fs::remove_file(&temporary)?;
        bail!("Prüfsumme des Java-Downloads stimmt nicht überein")
    }
    if destination.exists() {
        fs::remove_file(destination)?;
    }
    fs::rename(temporary, destination)?;
    Ok(())
}

fn extract_zip(archive: &Path, destination: &Path) -> Result<()> {
    let mut zip = zip::ZipArchive::new(File::open(archive)?)?;
    for index in 0..zip.len() {
        let mut entry = zip.by_index(index)?;
        let Some(relative) = entry.enclosed_name() else {
            continue;
        };
        let output = destination.join(relative);
        if entry.is_dir() {
            fs::create_dir_all(&output)?;
            continue;
        }
        if let Some(parent) = output.parent() {
            fs::create_dir_all(parent)?;
        }
        std::io::copy(&mut entry, &mut File::create(output)?)?;
    }
    Ok(())
}

fn find_java(folder: &Path) -> Option<PathBuf> {
    let entries = fs::read_dir(folder).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if let Some(java) = find_java(&path) {
                return Some(java);
            }
        } else if path
            .file_name()
            .is_some_and(|name| name.eq_ignore_ascii_case("java.exe"))
            && path.parent().is_some_and(|parent| {
                parent
                    .file_name()
                    .is_some_and(|name| name.eq_ignore_ascii_case("bin"))
            })
        {
            return Some(path);
        }
    }
    None
}

fn sha256_file(path: &Path) -> Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 131_072];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}
