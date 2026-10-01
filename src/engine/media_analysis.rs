//! Optional source analysis on the existing media decoder worker. A prepared
//! result is not a catalog commit; its token and WorkPermit follow publication.
use super::{decode, dsp, media_source::FileFingerprint, performance};
use crate::{
    sampler_bank::SourceRef,
    track_analysis::{self, Fields, Prepared},
};
use sha2::{Digest, Sha256};
use std::{
    fs::OpenOptions,
    io::{Read, Seek},
    os::unix::fs::OpenOptionsExt,
    sync::{
        atomic::{AtomicU64, AtomicU8, Ordering},
        Arc,
    },
    time::{SystemTime, UNIX_EPOCH},
};

const ACTIVE: u8 = 0;
const CANCELLED: u8 = 1;
const PREEMPTED: u8 = 2;
const PROTECTED: u8 = 3;
const CLAIMED: u8 = 4;
const UNKNOWN: u64 = u32::MAX as u64;
const MAX_SOURCE_BYTES: u64 = 8 * 1024 * 1024 * 1024;

#[derive(Clone, Debug)]
pub(crate) struct Request {
    pub reference: SourceRef,
    pub fields: Fields,
}
impl Request {
    pub fn validate(&self) -> Result<(), Failure> {
        self.reference.validate().map_err(Failure::error)?;
        if !self.fields.valid() {
            return Err(Failure::error("choose at least one analysis field"));
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Failure {
    Cancelled,
    Preempted,
    Protected,
    Failed(String),
}
impl Failure {
    fn error(detail: impl ToString) -> Self {
        let mut text = detail.to_string();
        if text.len() > 2048 {
            let mut end = 2048;
            while !text.is_char_boundary(end) {
                end -= 1;
            }
            text.truncate(end);
        }
        Self::Failed(text)
    }
}
impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Cancelled => f.write_str("Analysis cancelled before publication"),
            Self::Preempted => f.write_str("Analysis deferred for an explicit media load"),
            Self::Protected => f.write_str("Analysis cancelled by performance protection"),
            Self::Failed(error) => f.write_str(error),
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum Stage {
    Queued,
    Hashing,
    Decoding,
    Tempo,
    Waveform,
    Ready,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Progress {
    pub stage: Stage,
    pub millionths: Option<u32>,
}
#[derive(Clone, Debug)]
pub(crate) struct Token {
    pub id: u64,
    current: Arc<AtomicU64>,
    state: Arc<AtomicU8>,
    progress: Arc<AtomicU64>,
}
impl Token {
    pub(super) fn new(id: u64, current: Arc<AtomicU64>) -> Self {
        Self {
            id,
            current,
            state: Arc::new(AtomicU8::new(ACTIVE)),
            progress: Arc::new(AtomicU64::new(UNKNOWN)),
        }
    }
    pub(super) fn same_generation(&self) -> bool {
        self.current.load(Ordering::Acquire) == self.id
    }
    pub fn is_current(&self) -> bool {
        self.same_generation() && self.failure().is_none()
    }
    /// Once publication is claimed, cancellation cannot relabel a committed
    /// catalog write as cancelled. The metadata owner retains its actual result.
    pub fn cancel(&self) -> bool {
        self.abort(CANCELLED)
    }
    pub(super) fn preempt(&self) -> bool {
        self.abort(PREEMPTED)
    }
    fn abort(&self, reason: u8) -> bool {
        self.state
            .compare_exchange(ACTIVE, reason, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }
    pub fn failure(&self) -> Option<Failure> {
        match self.state.load(Ordering::Acquire) {
            CANCELLED => Some(Failure::Cancelled),
            PREEMPTED => Some(Failure::Preempted),
            PROTECTED => Some(Failure::Protected),
            _ => None,
        }
    }
    /// Call under WorkPermit::commit's guard immediately before irreversible
    /// publication. New request admission cancels old state before replacing
    /// current identity, making this CAS the shared cancellation boundary.
    pub fn claim_publication(&self) -> Result<(), Failure> {
        if self.current.load(Ordering::Acquire) != self.id {
            return Err(Failure::Cancelled);
        }
        if self.progress().stage != Stage::Ready {
            return Err(Failure::error("analysis is not ready for publication"));
        }
        self.state
            .compare_exchange(ACTIVE, CLAIMED, Ordering::AcqRel, Ordering::Acquire)
            .map(|_| ())
            .map_err(|_| {
                self.failure()
                    .unwrap_or_else(|| Failure::error("analysis publication already claimed"))
            })
    }
    pub fn progress(&self) -> Progress {
        let value = self.progress.load(Ordering::Acquire);
        let stage = match value >> 32 {
            0 => Stage::Queued,
            1 => Stage::Hashing,
            2 => Stage::Decoding,
            3 => Stage::Tempo,
            4 => Stage::Waveform,
            _ => Stage::Ready,
        };
        let part = value & u32::MAX as u64;
        Progress {
            stage,
            millionths: (part != UNKNOWN).then_some(part as u32),
        }
    }
    fn set_progress(&self, stage: Stage, done: u64, total: Option<u64>) {
        let fraction = total
            .filter(|total| *total > 0)
            .map(|total| ((u128::from(done.min(total)) * 1_000_000) / u128::from(total)) as u64)
            .unwrap_or(UNKNOWN);
        self.progress
            .store(((stage as u64) << 32) | fraction, Ordering::Release);
    }
    pub(super) fn check(&self, work: &performance::WorkPermit) -> Result<(), Failure> {
        if work.cancelled() {
            self.abort(PROTECTED);
        }
        if let Some(reason) = self.failure() {
            return Err(reason);
        }
        if self.current.load(Ordering::Acquire) != self.id {
            return Err(Failure::Cancelled);
        }
        Ok(())
    }
}

pub(super) fn run(
    request: Request,
    token: &Token,
    work: &performance::WorkPermit,
) -> Result<Prepared, Failure> {
    run_with(request, token, work, |_| {})
}
fn run_with(
    request: Request,
    token: &Token,
    work: &performance::WorkPermit,
    checkpoint: impl Fn(Stage),
) -> Result<Prepared, Failure> {
    request.validate()?;
    token.check(work)?;
    let cancelled = || token.check(work).is_err();
    let path = request.reference.path().map_err(Failure::error)?;
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .map_err(Failure::error)?;
    let metadata = file.metadata().map_err(Failure::error)?;
    if !metadata.is_file()
        || metadata.len() > MAX_SOURCE_BYTES
        || FileFingerprint::from_metadata(&metadata) != request.reference.fingerprint
    {
        return Err(Failure::error(
            "analysis source changed or exceeds the 8 GiB source-file limit",
        ));
    }
    let stable = |file: &std::fs::File| -> Result<(), Failure> {
        token.check(work)?;
        if FileFingerprint::from_metadata(&file.metadata().map_err(Failure::error)?)
            != request.reference.fingerprint
            || FileFingerprint::read(path) != Some(request.reference.fingerprint)
        {
            return Err(Failure::error(
                "analysis source changed during verification",
            ));
        }
        Ok(())
    };
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    let mut total = 0u64;
    token.set_progress(Stage::Hashing, 0, Some(metadata.len()));
    loop {
        token.check(work)?;
        let n = file.read(&mut buffer).map_err(Failure::error)?;
        if n == 0 {
            break;
        }
        total += n as u64;
        if total > metadata.len() {
            return Err(Failure::error("analysis source grew during verification"));
        }
        hasher.update(&buffer[..n]);
        token.set_progress(Stage::Hashing, total, Some(metadata.len()));
    }
    if total != metadata.len() {
        return Err(Failure::error(
            "analysis source was truncated during verification",
        ));
    }
    let hash: [u8; 32] = hasher.finalize().into();
    if request
        .reference
        .content_hash
        .is_some_and(|saved| saved != hash)
    {
        return Err(Failure::error(
            "analysis source bytes differ from the captured version",
        ));
    }
    checkpoint(Stage::Hashing);
    stable(&file)?;
    file.rewind().map_err(Failure::error)?;
    token.set_progress(Stage::Decoding, 0, None);
    let decoded = decode::decode_analysis_file(
        path,
        file.try_clone().map_err(Failure::error)?,
        cancelled,
        |done, total| token.set_progress(Stage::Decoding, done, total),
    );
    token.check(work)?;
    let decoded = decoded.map_err(Failure::error)?;
    checkpoint(Stage::Decoding);
    stable(&file)?;
    let sample = decoded.sample;
    let bpm = if request.fields.bpm {
        token.set_progress(Stage::Tempo, 0, None);
        let result = dsp::estimate_bpm_with_cancel(&sample.data, sample.ch, sample.sr, cancelled);
        token.check(work)?;
        result.map_err(|_| Failure::Cancelled)?
    } else {
        None
    };
    let waveform = if request.fields.waveform {
        token.set_progress(Stage::Waveform, 0, None);
        let result = dsp::peaks_3band_with_cancel(
            &sample.data,
            sample.ch,
            track_analysis::MAX_WAVEFORM_BINS,
            cancelled,
        );
        token.check(work)?;
        Some(track_analysis::cache::Waveform {
            sample_rate: sample.sr,
            channels: sample.ch,
            frames: sample.frames() as u64,
            bands: result.ok_or(Failure::Cancelled)?,
        })
    } else {
        None
    };
    stable(&file)?;
    let mut reference = request.reference;
    reference.content_hash = Some(hash);
    let prepared = Prepared {
        reference,
        fields: request.fields,
        at_unix_ms: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(Failure::error)?
            .as_millis()
            .try_into()
            .map_err(Failure::error)?,
        bpm,
        duration: sample.frames() as f64 / f64::from(sample.sr),
        waveform,
    };
    // All resident PCM dies on this single worker, before Ready is published.
    drop(sample);
    token.check(work)?;
    token.set_progress(Stage::Ready, 1, Some(1));
    Ok(prepared)
}

#[cfg(test)]
pub(crate) mod tests;
