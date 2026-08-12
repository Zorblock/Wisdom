use crate::instances::{self, Instance};
use anyhow::{Context, Result, bail};
use flate2::read::GzDecoder;
use serde::Serialize;
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::time::SystemTime;
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};

const MAX_LOG_BYTES: u64 = 16 * 1024 * 1024;
const MAX_LOG_LINES: usize = 50_000;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstanceLogFile {
    name: String,
    size: u64,
    modified: String,
    modified_millis: u128,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstanceLogLine {
    sequence: usize,
    timestamp: String,
    stream: &'static str,
    message: String,
}

pub fn list(root: &Path, instance_id: &str) -> Result<Vec<InstanceLogFile>> {
    let instance = instances::load(root, instance_id)?;
    let directory = log_directory(root, &instance);
    if !directory.is_dir() {
        return Ok(Vec::new());
    }
    let mut files = fs::read_dir(directory)?
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            if !is_log_name(&name) {
                return None;
            }
            let metadata = entry.metadata().ok()?;
            if !metadata.is_file() {
                return None;
            }
            let modified_time = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
            let modified_millis = modified_time
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis();
            let modified = chrono::DateTime::<chrono::Local>::from(modified_time)
                .format("%d %b %Y, %H:%M")
                .to_string();
            Some(InstanceLogFile {
                name,
                size: metadata.len(),
                modified,
                modified_millis,
            })
        })
        .collect::<Vec<_>>();
    files.sort_by(|left, right| right.modified_millis.cmp(&left.modified_millis));
    Ok(files)
}

pub fn read(root: &Path, instance_id: &str, file_name: &str) -> Result<Vec<InstanceLogLine>> {
    validate_log_name(file_name)?;
    let instance = instances::load(root, instance_id)?;
    let path = log_directory(root, &instance).join(file_name);
    if !path.is_file() {
        bail!("Log file was not found");
    }
    let file = File::open(&path).with_context(|| format!("Could not open {file_name}"))?;
    let source: Box<dyn Read> = if file_name.ends_with(".gz") {
        Box::new(GzDecoder::new(file))
    } else {
        Box::new(file)
    };
    let mut reader = BufReader::new(source.take(MAX_LOG_BYTES));
    let mut raw = Vec::new();
    let mut lines = Vec::new();
    while lines.len() < MAX_LOG_LINES {
        raw.clear();
        if reader.read_until(b'\n', &mut raw)? == 0 {
            break;
        }
        let value = String::from_utf8_lossy(&raw)
            .trim_end_matches(['\r', '\n'])
            .to_owned();
        if value.is_empty() {
            continue;
        }
        let (timestamp, message) = split_timestamp(&value);
        lines.push(InstanceLogLine {
            sequence: lines.len(),
            timestamp,
            stream: "log",
            message,
        });
    }
    Ok(lines)
}

pub fn open_window(app: &AppHandle, instance: &Instance) -> Result<(), String> {
    let label = format!("logs-{}", instance.id);
    if let Some(window) = app.get_webview_window(&label) {
        let _ = window.show();
        let _ = window.set_focus();
        return Ok(());
    }
    let url = format!(
        "index.html?logs=1&instanceId={}&name={}&version={}",
        urlencoding::encode(&instance.id),
        urlencoding::encode(&instance.name),
        urlencoding::encode(&instance.version)
    );
    WebviewWindowBuilder::new(app, &label, WebviewUrl::App(url.into()))
        .title(format!("Wisdom Logs — {}", instance.name))
        .inner_size(920.0, 580.0)
        .min_inner_size(680.0, 400.0)
        .resizable(true)
        .center()
        .build()
        .map(|_| ())
        .map_err(|error| format!("Could not open instance logs: {error}"))
}

fn log_directory(root: &Path, instance: &Instance) -> PathBuf {
    instances::game_dir(root, instance).join("logs")
}

fn is_log_name(name: &str) -> bool {
    name.ends_with(".log") || name.ends_with(".log.gz")
}

fn validate_log_name(name: &str) -> Result<()> {
    let path = Path::new(name);
    if name.is_empty()
        || !is_log_name(name)
        || path.file_name().and_then(|value| value.to_str()) != Some(name)
    {
        bail!("Invalid log file name");
    }
    Ok(())
}

fn split_timestamp(value: &str) -> (String, String) {
    if let Some(rest) = value.strip_prefix('[')
        && let Some(end) = rest.find(']')
    {
        let timestamp = &rest[..end];
        if timestamp.len() <= 12 && timestamp.contains(':') {
            return (
                timestamp.to_owned(),
                rest[end + 1..].trim_start().to_owned(),
            );
        }
    }
    ("--:--:--".to_owned(), value.to_owned())
}
