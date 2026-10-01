//! Worker-only, bounded recovery journals. Explicit native project saves are
//! never modified. Two-second capture cadence belongs to the caller; this store
//! commits complete edit-state records, not synchronous per-command transactions.
mod config;
mod files;
mod journal;
mod storage;
pub use config::*;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
pub use storage::{discard, discover, lookup_exact, recover, session_digest, Store};

const RECORD_LIMIT: usize = 8 * 1024 * 1024;
const SEGMENT_LIMIT: u64 = 64 * 1024 * 1024;
const RECORDS_LIMIT: u64 = 256;
const SESSIONS_LIMIT: usize = 128;
const FILES_LIMIT: usize = 100_000;
const REPORT_LIMIT: usize = 32;
const IO_CHUNK: usize = 64 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RecordMeta {
    pub epoch: u64,
    pub revision: u64,
    pub view_revision: u64,
    pub saved_path: Option<PathBuf>,
    pub captured_unix_ms: u64,
}
impl RecordMeta {
    fn validate(&self) -> Result<(), Error> {
        if self.saved_path.as_ref().is_some_and(|p| {
            p.to_str()
                .is_none_or(|s| s.len() > 4096 || s.contains('\0'))
        }) {
            return Err(Error::invalid("invalid saved-project path metadata"));
        }
        Ok(())
    }
}
#[derive(Clone, Debug)]
pub struct Candidate {
    pub session: String,
    pub sequence: u64,
    pub metadata: RecordMeta,
    root: PathBuf,
    segment: String,
    digest: [u8; 32],
    report: Vec<String>,
}
impl Candidate {
    /// Bind local UI actions to the exact verified journal record. This does
    /// not export project content, paths or a substitute latest candidate.
    pub(crate) fn record_digest(&self) -> [u8; 32] { self.digest }
}
#[derive(Debug, Default)]
pub struct Inventory {
    pub candidates: Vec<Candidate>,
    pub warnings: Vec<String>,
    pub usage_bytes: u64,
}
#[derive(Debug)]
pub struct Commit {
    pub durable: bool,
    pub committed_unix_ms: u64,
    pub sequence: u64,
    pub usage_bytes: u64,
    pub warning: Option<String>,
}
#[derive(Debug)]
pub struct Recovered<T> {
    pub bundle: crate::project_file::Bundle<T>,
    pub metadata: RecordMeta,
    pub report: Vec<String>,
}
#[derive(Debug)]
pub enum Error {
    Cancelled,
    Invalid(String),
    Io {
        stage: &'static str,
        source: std::io::Error,
    },
}
impl Error {
    fn invalid(message: impl Into<String>) -> Self {
        Self::Invalid(message.into())
    }
    fn io(stage: &'static str, source: std::io::Error) -> Self {
        Self::Io { stage, source }
    }
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Cancelled => f.write_str("Recovery work cancelled before commit"),
            Self::Invalid(message) => write!(f, "Recovery: {message}"),
            Self::Io { stage, source } => write!(f, "Recovery {stage}: {source}"),
        }
    }
}
impl std::error::Error for Error {}
impl From<crate::project_file::Error> for Error {
    fn from(error: crate::project_file::Error) -> Self {
        if matches!(error, crate::project_file::Error::Cancelled) {
            Self::Cancelled
        } else {
            Self::invalid(error.to_string())
        }
    }
}
fn check(cancel: &AtomicBool) -> Result<(), Error> {
    if cancel.load(Ordering::Acquire) {
        Err(Error::Cancelled)
    } else {
        Ok(())
    }
}
fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u64::MAX as u128) as u64
}
fn report(rows: &mut Vec<String>, message: impl AsRef<str>) {
    if rows.len() < REPORT_LIMIT {
        let text = message.as_ref();
        let mut end = text.len().min(2048);
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        rows.push(text[..end].to_owned());
    }
}
#[cfg(test)]
pub(crate) mod testing;
