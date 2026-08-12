use crate::minecraft::ProgressReporter;
use anyhow::{Context, Result, bail};
use reqwest::blocking::Client;
use sha1::{Digest, Sha1};
use sha2::Sha256;
use std::collections::VecDeque;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

const MIN_DOWNLOAD_WORKERS: usize = 8;
const MAX_DOWNLOAD_WORKERS: usize = 16;
const BUFFER_SIZE: usize = 256 * 1024;
const PROGRESS_INTERVAL: Duration = Duration::from_millis(100);
const SPEED_INTERVAL: Duration = Duration::from_millis(400);

#[derive(Clone, Debug)]
pub enum Checksum {
    Sha1(String),
    Sha256(String),
}

#[derive(Clone, Debug)]
pub struct DownloadJob {
    pub url: String,
    pub destination: PathBuf,
    pub checksum: Option<Checksum>,
}

struct TransferProgress {
    received: Vec<u64>,
    totals: Vec<u64>,
    completed: usize,
    transferred: u64,
    speed_bytes: u64,
    speed: f64,
    speed_at: Instant,
    emitted_at: Instant,
}

impl TransferProgress {
    fn new(count: usize) -> Self {
        let now = Instant::now();
        Self {
            received: vec![0; count],
            totals: vec![0; count],
            completed: 0,
            transferred: 0,
            speed_bytes: 0,
            speed: 0.0,
            speed_at: now,
            emitted_at: now.checked_sub(PROGRESS_INTERVAL).unwrap_or(now),
        }
    }

    fn update(
        &mut self,
        index: usize,
        received: u64,
        total: u64,
        count: usize,
        category: &str,
        force: bool,
    ) -> Option<(f32, String)> {
        let previous = self.received[index];
        self.received[index] = received;
        if total > 0 {
            self.totals[index] = total;
        }
        self.transferred = self
            .transferred
            .saturating_add(received.saturating_sub(previous));

        let now = Instant::now();
        let speed_elapsed = now.duration_since(self.speed_at);
        if speed_elapsed >= SPEED_INTERVAL {
            self.speed = self.transferred.saturating_sub(self.speed_bytes) as f64
                / speed_elapsed.as_secs_f64();
            self.speed_bytes = self.transferred;
            self.speed_at = now;
        }
        if !force && now.duration_since(self.emitted_at) < PROGRESS_INTERVAL {
            return None;
        }
        self.emitted_at = now;

        let active_fraction = self
            .received
            .iter()
            .zip(&self.totals)
            .filter(|(_, total)| **total > 0)
            .map(|(received, total)| (*received as f64 / *total as f64).clamp(0.0, 1.0))
            .sum::<f64>();
        let value = ((self.completed as f64 + active_fraction) / count as f64).clamp(0.0, 1.0);
        let percent = (value * 100.0).round() as u32;
        Some((
            value as f32,
            format!(
                "Downloading {category} · {percent}% · {} · {}/{} files",
                format_speed(self.speed),
                self.completed,
                count
            ),
        ))
    }

    fn finish(&mut self, index: usize, count: usize, category: &str) -> (f32, String) {
        self.received[index] = 0;
        self.totals[index] = 0;
        self.completed += 1;
        self.update(index, 0, 0, count, category, true).unwrap()
    }
}

pub fn download_jobs(
    client: &Client,
    jobs: Vec<DownloadJob>,
    category: &str,
    progress: &ProgressReporter<'_>,
) -> Result<()> {
    if jobs.is_empty() {
        progress(1.0, format!("{category} are ready."));
        return Ok(());
    }

    let count = jobs.len();
    progress(
        0.0,
        format!("Downloading {category} · 0% · 0 B/s · 0/{count} files"),
    );
    let queue = Arc::new(Mutex::new(
        jobs.into_iter().enumerate().collect::<VecDeque<_>>(),
    ));
    let state = Arc::new(Mutex::new(TransferProgress::new(count)));
    let failure = Arc::new(Mutex::new(None::<String>));
    let cancelled = Arc::new(AtomicBool::new(false));
    let workers = worker_count(count);

    thread::scope(|scope| {
        for _ in 0..workers {
            let client = client.clone();
            let queue = Arc::clone(&queue);
            let state = Arc::clone(&state);
            let failure = Arc::clone(&failure);
            let cancelled = Arc::clone(&cancelled);
            scope.spawn(move || {
                while !cancelled.load(Ordering::Relaxed) {
                    let task = queue.lock().ok().and_then(|mut queue| queue.pop_front());
                    let Some((index, job)) = task else { break };
                    let result = download_one(&client, &job, |received, total| {
                        let snapshot = state.lock().ok().and_then(|mut state| {
                            state.update(index, received, total, count, category, false)
                        });
                        if let Some((value, message)) = snapshot {
                            progress(value, message);
                        }
                    });
                    if let Err(error) = result {
                        cancelled.store(true, Ordering::Relaxed);
                        if let Ok(mut stored) = failure.lock() {
                            *stored = Some(format!("Download failed ({}): {error:#}", job.url));
                        }
                        break;
                    }
                    if let Ok(mut state) = state.lock() {
                        let (value, message) = state.finish(index, count, category);
                        progress(value, message);
                    }
                }
            });
        }
    });

    if let Some(error) = failure.lock().ok().and_then(|error| error.clone()) {
        bail!(error);
    }
    progress(1.0, format!("{category} are ready."));
    Ok(())
}

fn worker_count(job_count: usize) -> usize {
    let suggested = thread::available_parallelism()
        .map(usize::from)
        .unwrap_or(MIN_DOWNLOAD_WORKERS)
        .saturating_mul(2)
        .clamp(MIN_DOWNLOAD_WORKERS, MAX_DOWNLOAD_WORKERS);
    job_count.min(suggested)
}

fn download_one(
    client: &Client,
    job: &DownloadJob,
    mut on_progress: impl FnMut(u64, u64),
) -> Result<()> {
    if reusable(&job.destination, job.checksum.as_ref())? {
        return Ok(());
    }
    fs::create_dir_all(
        job.destination
            .parent()
            .context("Download target has no parent directory")?,
    )?;
    let mut response = client.get(&job.url).send()?.error_for_status()?;
    let total = response.content_length().unwrap_or(0);
    let temporary = job.destination.with_extension("download");
    let result = (|| -> Result<()> {
        let mut output = File::create(&temporary)?;
        let mut sha1 = Sha1::new();
        let mut sha256 = Sha256::new();
        let mut received = 0u64;
        let mut buffer = vec![0u8; BUFFER_SIZE];
        loop {
            let amount = response.read(&mut buffer)?;
            if amount == 0 {
                break;
            }
            output.write_all(&buffer[..amount])?;
            match job.checksum {
                Some(Checksum::Sha1(_)) => sha1.update(&buffer[..amount]),
                Some(Checksum::Sha256(_)) => sha256.update(&buffer[..amount]),
                None => {}
            }
            received += amount as u64;
            on_progress(received, total);
        }
        output.flush()?;
        verify_download(job.checksum.as_ref(), sha1, sha256)?;
        replace_file(&temporary, &job.destination)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn reusable(path: &Path, checksum: Option<&Checksum>) -> Result<bool> {
    if !path.is_file() {
        return Ok(false);
    }
    match checksum {
        Some(Checksum::Sha1(expected)) => {
            Ok(hash_file::<Sha1>(path)?.eq_ignore_ascii_case(expected))
        }
        Some(Checksum::Sha256(expected)) => {
            Ok(hash_file::<Sha256>(path)?.eq_ignore_ascii_case(expected))
        }
        None => Ok(true),
    }
}

fn hash_file<D: Digest + Default>(path: &Path) -> Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = D::default();
    let mut buffer = vec![0u8; BUFFER_SIZE];
    loop {
        let amount = file.read(&mut buffer)?;
        if amount == 0 {
            break;
        }
        hasher.update(&buffer[..amount]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

fn verify_download(checksum: Option<&Checksum>, sha1: Sha1, sha256: Sha256) -> Result<()> {
    let matches = match checksum {
        Some(Checksum::Sha1(expected)) => {
            format!("{:x}", sha1.finalize()).eq_ignore_ascii_case(expected)
        }
        Some(Checksum::Sha256(expected)) => {
            format!("{:x}", sha256.finalize()).eq_ignore_ascii_case(expected)
        }
        None => true,
    };
    if !matches {
        bail!("Downloaded file checksum does not match");
    }
    Ok(())
}

fn replace_file(source: &Path, destination: &Path) -> Result<()> {
    if destination.exists() {
        fs::remove_file(destination)?;
    }
    fs::rename(source, destination)?;
    Ok(())
}

fn format_speed(bytes_per_second: f64) -> String {
    if bytes_per_second >= 1024.0 * 1024.0 {
        format!("{:.1} MB/s", bytes_per_second / (1024.0 * 1024.0))
    } else if bytes_per_second >= 1024.0 {
        format!("{:.0} KB/s", bytes_per_second / 1024.0)
    } else {
        format!("{bytes_per_second:.0} B/s")
    }
}
