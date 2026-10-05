//! Read-only media validation on the existing optional decoder lane.
use super::{
    decode, media_analysis,
    media_source::{FileFingerprint, LibSource},
    performance,
};
use crate::media_location::{Failure as LocationFailure, Location};
use std::{fs::OpenOptions, io::Seek, os::unix::fs::OpenOptionsExt, time::SystemTime};

#[derive(Clone, Debug)]
pub(crate) struct Request {
    pub source: LibSource,
    pub fingerprint: Option<FileFingerprint>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Condition {
    Ready,
    Unverified,
    Missing,
    Unreadable,
    Unsupported,
    Corrupt,
    Changed,
    Limit,
}
impl Condition {
    /// Name the observed media condition.
    /// Takes this condition; returns a fixed browser label.
    pub fn label(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::Unverified => "length unverified",
            Self::Missing => "missing",
            Self::Unreadable => "unreadable",
            Self::Unsupported => "unsupported",
            Self::Corrupt => "corrupt",
            Self::Changed => "changed",
            Self::Limit => "validation limit",
        }
    }
    /// Explain how to recover or refresh this media check.
    /// Takes this condition; returns its next useful action.
    pub fn action(self) -> &'static str {
        match self {
            Self::Ready => {
                "Last checked bytes decoded successfully; revalidate after changing media."
            }
            Self::Unverified => "Compare a trusted complete copy before performing; decoding alone cannot rule out missing content.",
            Self::Missing => {
                "Reconnect the drive or relocate this saved track, then validate again."
            }
            Self::Unreadable => "Restore read access, then validate again.",
            Self::Unsupported => "Use a supported local audio format, then import and validate it.",
            Self::Corrupt => {
                "Restore a complete copy of the audio file, then scan and validate again."
            }
            Self::Changed => "Scan the current media version, then validate again.",
            Self::Limit => "This check could not certify the file within its resource limits.",
        }
    }
}
#[derive(Clone, Debug)]
pub(crate) struct Observation {
    pub condition: Condition,
    pub fingerprint: Option<FileFingerprint>,
    pub read_only: bool,
    pub detail: String,
    pub checked: SystemTime,
}
impl Observation {
    /// Bound a media-check result.
    /// Takes the condition, observed fingerprint, write access and detail; returns a timestamped observation with at most 1024 detail characters.
    fn new(
        condition: Condition,
        fingerprint: Option<FileFingerprint>,
        read_only: bool,
        detail: impl ToString,
    ) -> Self {
        Self {
            condition,
            fingerprint,
            read_only,
            detail: detail.to_string().chars().take(1024).collect(),
            checked: SystemTime::now(),
        }
    }
    /// Describe the last media check and its next action.
    /// Takes this observation; returns bounded visible and accessible status text.
    pub fn description(&self) -> String {
        format!(
            "{}{}: {} {}",
            self.condition.label(),
            if self.read_only { " · read-only" } else { "" },
            self.detail,
            self.condition.action()
        )
    }
}
/// Classify a location failure without decoding unavailable media.
/// Takes a typed resolution failure; returns an actionable media condition.
fn location_failure(error: LocationFailure) -> Observation {
    let condition = match error {
        LocationFailure::Offline | LocationFailure::NotVisible | LocationFailure::Missing => {
            Condition::Missing
        }
        LocationFailure::Unsupported | LocationFailure::Invalid => Condition::Unsupported,
        LocationFailure::Changed | LocationFailure::Ambiguous => Condition::Changed,
        LocationFailure::Capacity => Condition::Limit,
        LocationFailure::Unreadable(_) => Condition::Unreadable,
    };
    Observation::new(condition, None, false, error)
}
/// Classify a failed file read.
/// Takes the OS error; distinguishes an absent file from unavailable read access.
fn io_failure(error: std::io::Error) -> Observation {
    Observation::new(
        if error.kind() == std::io::ErrorKind::NotFound {
            Condition::Missing
        } else {
            Condition::Unreadable
        },
        None,
        false,
        error,
    )
}
struct Active<'a>(&'a dyn Fn() -> bool);
impl crate::media_tags::Cancellation for Active<'_> {
    fn cancelled(&self) -> bool {
        (self.0)()
    }
}

/// Validate a captured local source without installing or retaining its PCM.
/// Takes the source/version, request token and performance permit; returns an actionable observation or a truthful cancellation reason.
pub(super) fn run(
    request: Request,
    token: &media_analysis::Token,
    work: &performance::WorkPermit,
) -> Result<Observation, media_analysis::Failure> {
    token.check(work)?;
    let cancelled = || token.check(work).is_err();
    let result = (|| {
        if matches!(request.source, LibSource::Builtin(_)) {
            return Observation::new(Condition::Ready, None, false, "Bundled generated audio");
        }
        let location = match Location::resolve(&request.source) {
            Ok(value) => value,
            Err(error) => return location_failure(error),
        };
        let mut file = match OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(&location.path)
        {
            Ok(value) => value,
            Err(error) => return io_failure(error),
        };
        let metadata = match file.metadata() {
            Ok(value) => value,
            Err(error) => return io_failure(error),
        };
        let fingerprint = FileFingerprint::from_metadata(&metadata);
        if !metadata.is_file() {
            return Observation::new(
                Condition::Unsupported,
                Some(fingerprint),
                false,
                "Source is not a regular audio file",
            );
        }
        if request.fingerprint.is_some_and(|old| old != fingerprint) {
            return Observation::new(
                Condition::Changed,
                Some(fingerprint),
                false,
                "Saved source fingerprint no longer matches",
            );
        }
        if metadata.len() > 8 * 1024 * 1024 * 1024 {
            return Observation::new(
                Condition::Limit,
                Some(fingerprint),
                false,
                "Source exceeds the 8 GiB validation limit",
            );
        }
        if let Err(error) = location.verify_file(&file, fingerprint) {
            return location_failure(error);
        }
        use std::os::unix::ffi::OsStrExt;
        let read_only = std::ffi::CString::new(location.path.as_os_str().as_bytes()).map_or(
            true,
            |path| unsafe {
                libc::faccessat(libc::AT_FDCWD, path.as_ptr(), libc::W_OK, libc::AT_EACCESS) != 0
            },
        );
        let tags = crate::media_tags::read_tagged(&mut file, &Active(&cancelled), false).err();
        if let Err(error) = file.rewind() {
            return io_failure(error);
        }
        let descriptor = match file.try_clone() {
            Ok(value) => value,
            Err(error) => return io_failure(error),
        };
        token.set_progress(media_analysis::Stage::Decoding, 0, None);
        let decoded =
            decode::validate_audio_file(&location.path, descriptor, cancelled, |done, total| {
                token.set_progress(media_analysis::Stage::Decoding, done, total);
            });
        if let Err(error) = location.verify_file(&file, fingerprint) {
            return location_failure(error);
        }
        let mut observation = match decoded {
            Ok((diagnostics, rate, channels)) => {
                let mut detail = format!(
                    "{} decoded frames, {rate} Hz, {channels} channels.",
                    diagnostics.decoded_frames
                );
                if let Some(warning) = diagnostics.warning() {
                    detail.push(' ');
                    detail.push_str(&warning);
                }
                if let Some(error) = tags {
                    detail.push_str(" Tags need inspection: ");
                    detail.push_str(&error);
                }
                let condition = if diagnostics.warning().is_some() {
                    Condition::Unverified
                } else {
                    Condition::Ready
                };
                Observation::new(condition, Some(fingerprint), read_only, detail)
            }
            Err(error) => {
                let condition = match error.kind {
                    decode::DecodeFailureKind::Io => Condition::Unreadable,
                    decode::DecodeFailureKind::Unsupported
                    | decode::DecodeFailureKind::ResetRequired
                    | decode::DecodeFailureKind::FormatChanged => Condition::Unsupported,
                    decode::DecodeFailureKind::Capacity => Condition::Limit,
                    _ => Condition::Corrupt,
                };
                Observation::new(
                    condition,
                    Some(fingerprint),
                    read_only,
                    format!(
                        "{} ({} decoded frames)",
                        error.detail, error.diagnostics.decoded_frames
                    ),
                )
            }
        };
        observation.checked = SystemTime::now();
        observation
    })();
    token.check(work)?;
    token.set_progress(media_analysis::Stage::Ready, 1, Some(1));
    Ok(result)
}

#[cfg(test)]
mod tests;
