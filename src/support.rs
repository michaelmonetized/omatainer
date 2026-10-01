//! Local, inspectable support evidence. No upload client and no raw user data.
//! Collection and persistence run outside real-time callbacks; callback evidence
//! comes from the engine's existing fixed atomic counters.
mod model;
pub mod storage;
pub mod worker;
pub use model::*;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

pub const MAX_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_EVENTS: usize = 512;
pub const MAX_SAMPLES: usize = 120;
pub const MAX_RECOVERIES: usize = 8;
pub const MAX_RUNS: usize = 8;
pub const MAX_STORAGE_BYTES: u64 = 40 * 1024 * 1024;

#[derive(Debug)]
pub enum Error {
    Cancelled,
    Busy,
    Invalid(&'static str),
    Io {
        stage: &'static str,
        source: std::io::Error,
    },
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Cancelled => f.write_str("Support operation cancelled before publication"),
            Self::Busy => {
                f.write_str("Support storage is in use; retry when the current operation finishes")
            }
            Self::Invalid(message) => write!(f, "Support report: {message}"),
            Self::Io { stage, source } => write!(f, "Support {stage}: {source}"),
        }
    }
}
impl std::error::Error for Error {}
impl Error {
    fn io(stage: &'static str, source: std::io::Error) -> Self {
        Self::Io { stage, source }
    }
    pub fn class(&self) -> FailureClass {
        match self {
            Self::Cancelled => FailureClass::Cancelled,
            Self::Busy => FailureClass::Busy,
            Self::Invalid(_) => FailureClass::Invalid,
            Self::Io { source, .. } => FailureClass::from_io(source),
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
pub fn default_path(state: Option<&Path>, home: Option<&Path>) -> Result<PathBuf, Error> {
    let base = state
        .filter(|p| p.is_absolute())
        .map(Path::to_owned)
        .or_else(|| {
            home.filter(|p| p.is_absolute())
                .map(|p| p.join(".local/state"))
        })
        .ok_or(Error::Invalid(
            "an absolute XDG_STATE_HOME or HOME is required",
        ))?;
    Ok(base.join("omatainer/support"))
}

#[cfg(test)]
mod tests;
