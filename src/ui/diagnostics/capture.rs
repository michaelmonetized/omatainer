//! Redacted, bounded diagnostic files. All I/O and serialization run on a worker.
use crate::engine::{audio_metrics::AudioMetrics, diagnostics::Profile, QueuePressure};
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    Arc,
};
use std::time::{Duration, Instant};

pub const MAX_SAMPLES: usize = 120;
pub const MAX_BYTES: usize = 2 * 1024 * 1024;
pub const INTERVAL: Duration = Duration::from_millis(250);
pub const DURATION: Duration = Duration::from_secs(30);
static NEXT_FILE: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Metadata {
    pub app_version: String,
    pub revision: String,
    pub os: String,
    pub architecture: String,
    pub logical_cpus: usize,
    pub audio_backend: Option<String>,
    pub output_format: Option<String>,
    pub midi_status_entries: usize,
    pub tracks: usize,
    pub scenes: usize,
    pub midi_clips: usize,
    pub audio_clips: usize,
    pub session_bpm: f32,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Sample {
    pub elapsed_ms: u64,
    pub audio: AudioMetrics,
    pub commands: QueuePressure,
    pub ui_update_ns: Option<u64>,
    /// received, queued, dispatched, coalesced, dropped, resets, oversized, disconnected
    pub midi: [u64; 8],
    /// pending, capacity, accepted, dispatched, rejected
    pub library_queue: [u64; 5],
    pub profile: Option<Profile>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Report {
    pub schema: u32,
    pub metadata: Metadata,
    pub samples: Vec<Sample>,
}
impl Report {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema != 1 || self.samples.len() > MAX_SAMPLES {
            return Err("Unsupported or oversized diagnostic report".into());
        }
        for field in [
            &self.metadata.app_version,
            &self.metadata.revision,
            &self.metadata.os,
            &self.metadata.architecture,
        ] {
            if field.len() > 128 {
                return Err("Oversized metadata field".into());
            }
        }
        if self
            .metadata
            .audio_backend
            .as_ref()
            .is_some_and(|x| x.len() > 128)
            || self
                .metadata
                .output_format
                .as_ref()
                .is_some_and(|x| x.len() > 128)
        {
            return Err("Oversized hardware field".into());
        }
        if !self.metadata.session_bpm.is_finite() {
            return Err("Invalid session tempo".into());
        }
        let mut previous = 0;
        for sample in &self.samples {
            if sample.elapsed_ms > DURATION.as_millis() as u64 || sample.elapsed_ms < previous {
                return Err("Invalid capture timeline".into());
            }
            previous = sample.elapsed_ms;
            if let Some(profile) = &sample.profile {
                if profile.costs.len() > crate::engine::diagnostics::POINTS
                    || profile.sample_rate == 0
                {
                    return Err("Invalid load profile".into());
                }
                for cost in &profile.costs {
                    if !cost.valid() {
                        return Err("Invalid device identity".into());
                    }
                }
            }
        }
        Ok(())
    }
}

pub struct Capture {
    pub report: Option<Report>,
    started: Option<Instant>,
    next: Option<Instant>,
}
impl Default for Capture {
    fn default() -> Self {
        Self {
            report: None,
            started: None,
            next: None,
        }
    }
}
impl Capture {
    pub fn start(&mut self, metadata: Metadata, now: Instant) {
        self.report = Some(Report {
            schema: 1,
            metadata,
            samples: Vec::with_capacity(MAX_SAMPLES),
        });
        self.started = Some(now);
        self.next = Some(now);
    }
    pub fn running(&self) -> bool {
        self.started.is_some()
    }
    pub fn stop(&mut self) {
        self.started = None;
        self.next = None;
    }
    pub fn cancel(&mut self) {
        self.stop();
        self.report = None;
    }
    pub fn due(&self, now: Instant) -> bool {
        self.next.is_some_and(|next| now >= next)
    }
    pub fn record(&mut self, mut sample: Sample, now: Instant) {
        let Some(started) = self.started else { return };
        let elapsed = now.saturating_duration_since(started);
        if elapsed > DURATION {
            self.stop();
            return;
        }
        if !self.due(now) {
            return;
        }
        sample.elapsed_ms = elapsed.as_millis() as u64;
        let report = self.report.as_mut().unwrap();
        report.samples.push(sample);
        if report.samples.len() >= MAX_SAMPLES || elapsed >= DURATION {
            self.stop();
        } else {
            self.next = Some(now + INTERVAL);
        }
    }
}

pub enum Completed {
    Exported,
    Reopened(Report),
    Cancelled,
    Failed(String),
}
pub struct Worker {
    result: crossbeam_channel::Receiver<Completed>,
    cancel: Arc<AtomicBool>,
}
impl Worker {
    pub fn start(
        path: PathBuf,
        report: Option<Report>,
        #[cfg(test)] gate: Option<Arc<std::sync::Barrier>>,
    ) -> std::io::Result<Self> {
        let (sender, result) = crossbeam_channel::bounded(1);
        let cancel = Arc::new(AtomicBool::new(false));
        let cancelled = cancel.clone();
        std::thread::Builder::new()
            .name("omatainer-diagnostics".into())
            .spawn(move || {
                #[cfg(test)]
                if let Some(gate) = gate {
                    gate.wait();
                }
                let outcome = match report {
                    Some(report) => export(&path, &report, &cancelled).map(|_| Completed::Exported),
                    None => reopen(&path, &cancelled).map(Completed::Reopened),
                };
                let value = match outcome {
                    Ok(done) => done,
                    Err(error) if error == "Cancelled" => Completed::Cancelled,
                    Err(error) => Completed::Failed(error),
                };
                let _ = sender.send(value);
            })?;
        Ok(Self { result, cancel })
    }
    pub fn poll(&self) -> Option<Completed> {
        match self.result.try_recv() {
            Ok(done) => Some(done),
            Err(crossbeam_channel::TryRecvError::Empty) => None,
            Err(crossbeam_channel::TryRecvError::Disconnected) => {
                Some(Completed::Failed("Diagnostic worker disconnected".into()))
            }
        }
    }
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Release);
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.cancel();
    }
}
fn check(cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::Acquire) {
        Err("Cancelled".into())
    } else {
        Ok(())
    }
}

pub(super) fn export(path: &Path, report: &Report, cancel: &AtomicBool) -> Result<(), String> {
    check(cancel)?;
    report.validate()?;
    let bytes = serde_json::to_vec(report).map_err(|error| error.to_string())?;
    if bytes.len() > MAX_BYTES {
        return Err("Diagnostic export exceeds 2 MiB".into());
    }
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let temporary = parent.join(format!(
        ".omatainer-diagnostics-{}-{}.tmp",
        std::process::id(),
        NEXT_FILE.fetch_add(1, Ordering::Relaxed)
    ));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temporary)
        .map_err(|error| error.to_string())?;
    let result = (|| {
        for chunk in bytes.chunks(4096) {
            check(cancel)?;
            file.write_all(chunk).map_err(|error| error.to_string())?;
        }
        file.sync_all().map_err(|error| error.to_string())?;
        check(cancel)?;
        // Same-filesystem atomic publication that never overwrites an existing
        // file or follows a destination symlink. A successful publish wins a
        // concurrent late cancellation and is reported truthfully as exported.
        fs::hard_link(&temporary, path).map_err(|error| error.to_string())?;
        Ok(())
    })();
    drop(file);
    let _ = fs::remove_file(&temporary);
    result
}
pub(super) fn reopen(path: &Path, cancel: &AtomicBool) -> Result<Report, String> {
    check(cancel)?;
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .map_err(|error| error.to_string())?;
    let metadata = file.metadata().map_err(|error| error.to_string())?;
    if !metadata.is_file() || metadata.len() > MAX_BYTES as u64 {
        return Err("Diagnostic input must be a regular file of at most 2 MiB".into());
    }
    let mut bytes = Vec::new();
    (&mut file)
        .take(MAX_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    check(cancel)?;
    if bytes.len() > MAX_BYTES {
        return Err("Diagnostic input exceeds 2 MiB".into());
    }
    let report: Report = serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
    report.validate()?;
    check(cancel)?;
    Ok(report)
}
