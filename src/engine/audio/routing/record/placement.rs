use crate::engine::{
    arrangement::{self, AudioClock, Instance, Model, Source},
    audio_clip, media_source::FileFingerprint, midi_data, project, session, ClipKind,
};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{fs::File, io::Read, os::unix::fs::{FileExt, OpenOptionsExt}, path::{Path, PathBuf}, sync::{atomic::{AtomicBool, Ordering}, Arc}};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Timing {
    schema: u32,
    source_alias: String,
    rate: u32,
    channels: u16,
    frames: u64,
    audio_sha256: String,
    conductor: Option<Arc<midi_data::Conductor>>,
    exact_bpm: Option<f64>,
    placement: Option<Placement>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Placement {
    transport_seconds_at_start: f64,
    graph_delay_frames: u32,
    source_seconds_at_start: f64,
}

/// Hash captured audio without changing the writer's file position.
/// Takes an open regular file and cancellation; returns its SHA-256 with bounded read storage or refuses oversized, changing or cancelled data.
pub(super) fn hash_file(file: &File, cancel: &AtomicBool) -> Result<String, String> {
    let metadata = file.metadata().map_err(|e| e.to_string())?;
    if !metadata.is_file() || metadata.len() > super::MAX_BYTES + 4096 {
        return Err("Recorded audio exceeds the placement file limit".into());
    }
    let mut hash = Sha256::new();
    let mut bytes = [0; 8192];
    let mut offset = 0;
    loop {
        active(cancel)?;
        let count = file.read_at(&mut bytes, offset).map_err(|e| e.to_string())?;
        if count == 0 { break; }
        offset += count as u64;
        if offset > metadata.len() { return Err("Recorded audio changed during hashing".into()); }
        hash.update(&bytes[..count]);
    }
    if offset != metadata.len() || FileFingerprint::from_metadata(&metadata) != FileFingerprint::from_metadata(&file.metadata().map_err(|e| e.to_string())?) {
        return Err("Recorded audio changed during hashing".into());
    }
    Ok(format!("{:x}", hash.finalize()))
}
fn active(cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::Acquire) { Err("Recording placement cancelled".into()) } else { Ok(()) }
}
/// Retain the source recording's complete time map.
/// Takes an optional conductor and constant tempo; returns a validated immutable map or refuses an invalid tempo.
pub(super) fn conductor(original: Option<&Arc<midi_data::Conductor>>, bpm: f32) -> Result<Arc<midi_data::Conductor>, String> {
    original.map(|c| c.prepare()).unwrap_or_else(|| {
        midi_data::Conductor { ppqn: 960, tempos: vec![midi_data::Tempo::new(0, f64::from(bpm), false)?], meters: vec![midi_data::Meter { tick: 0, numerator: 4, denominator_power: 2, clocks: 24, thirty_seconds: 8 }], native: None }.prepare()
    })
}
fn opened(path: &Path) -> Result<(crate::media_location::Location, File, FileFingerprint), String> {
    if !path.is_absolute() || !std::fs::symlink_metadata(path).map_err(|e| e.to_string())?.is_file() {
        return Err("Recording placement requires an absolute regular file".into());
    }
    let location = crate::media_location::Snapshot::discover().map_err(|e| e.to_string())?.identify(path).map_err(|e| e.to_string())?;
    let file = std::fs::OpenOptions::new().read(true).custom_flags(libc::O_NONBLOCK | libc::O_CLOEXEC | libc::O_NOFOLLOW).open(&location.path).map_err(|e| e.to_string())?;
    let metadata = file.metadata().map_err(|e| e.to_string())?;
    if !metadata.is_file() { return Err("Recording placement requires regular files".into()); }
    let fingerprint = FileFingerprint::from_metadata(&metadata);
    location.verify_file(&file, fingerprint).map_err(|e| e.to_string())?;
    Ok((location, file, fingerprint))
}

/// Retain a reviewed recording and its compensated timeline placement.
/// Holds one coherent project, immutable PCM and guarded files until explicit stopped-project admission.
pub(crate) struct Review {
    captured: project::Captured,
    model: Model,
    audio: crate::media_location::Location,
    timing: crate::media_location::Location,
    audio_fingerprint: FileFingerprint,
    timing_fingerprint: FileFingerprint,
    pub path: PathBuf,
    pub start: f64,
    pub duration: f64,
    pub trimmed_frames: u64,
    pub delay_frames: u32,
    pub target: session::Reference,
}
impl Review {
    /// Inspect complete mono/stereo audio and its original timing receipt.
    /// Takes a coherent project, recorded WAV, retained track and cancellation; returns an editable one-shot placement or preserves the project on malformed, missing, changed or discontinuous data.
    pub(crate) fn inspect(mut captured: project::Captured, path: &Path, target: session::Reference, cancel: &AtomicBool) -> Result<Arc<Self>, String> {
        active(cancel)?;
        let layout = captured.state.session.as_ref().ok_or("Session identity unavailable")?;
        if target.namespace != layout.namespace || layout.resolve(session::Axis::Track, target.id).is_none() {
            return Err("Recording destination track changed; choose it again".into());
        }
        let mut name = path.file_name().ok_or("Recording file name unavailable")?.to_os_string();
        name.push(".omatainer.json");
        let (timing, timing_file, timing_fingerprint) = opened(&path.with_file_name(name))?;
        const TIMING_LIMIT: u64 = 4 * 1024 * 1024;
        if timing_file.metadata().map_err(|e| e.to_string())?.len() > TIMING_LIMIT { return Err("Recording timing receipt exceeds its limit".into()); }
        let mut bytes = Vec::new();
        timing_file.try_clone().map_err(|e| e.to_string())?.take(TIMING_LIMIT + 1).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
        let metadata: Timing = serde_json::from_slice(&bytes).map_err(|e| format!("Recording timing receipt is unavailable or incompatible: {e}"))?;
        let placement = metadata.placement.ok_or("Recording has no continuous playing timeline; place it manually with a reviewed source offset")?;
        if metadata.schema != 1 || metadata.source_alias.parse::<u64>().ok().is_none_or(|id| id == 0)
            || !(8000..=384000).contains(&metadata.rate) || !(1..=2).contains(&metadata.channels) || metadata.frames == 0
            || metadata.audio_sha256.len() != 64 || !metadata.audio_sha256.bytes().all(|b| b.is_ascii_hexdigit())
            || !placement.transport_seconds_at_start.is_finite() || !placement.source_seconds_at_start.is_finite()
            || !(-60.0..=86400.0).contains(&placement.source_seconds_at_start)
            || (placement.transport_seconds_at_start - f64::from(placement.graph_delay_frames) / f64::from(metadata.rate) - placement.source_seconds_at_start).abs() > 0.25 / f64::from(metadata.rate)
        { return Err("Recording receipt has unsupported channels or invalid timing".into()); }
        let (audio, file, audio_fingerprint) = opened(path)?;
        if hash_file(&file, cancel)? != metadata.audio_sha256 { return Err("Recorded WAV no longer matches its timing receipt".into()); }
        let check = file.try_clone().map_err(|e| e.to_string())?;
        let used = captured.media.iter().try_fold(0u64, |sum, sample| sum.checked_add(sample.data.len() as u64 * 4)).ok_or("Recording media budget overflow")?;
        let remaining = crate::project_file::Limits::default().max_pcm_bytes.saturating_sub(used).min(super::MAX_BYTES);
        let decoded = crate::engine::decode::decode_sampler_file(path, file, remaining, || cancel.load(Ordering::Acquire)).map_err(|e| e.to_string())?;
        let sample = Arc::new(decoded.sample);
        if sample.sr != metadata.rate || sample.ch != metadata.channels || sample.frames() as u64 != metadata.frames { return Err("Recorded WAV format or length differs from its timing receipt".into()); }
        audio.verify_file(&check, audio_fingerprint).map_err(|e| e.to_string())?;
        timing.verify_file(&timing_file, timing_fingerprint).map_err(|e| e.to_string())?;
        let conductor = metadata.conductor.ok_or("Recording has no retained musical clock; place it manually after reviewing timing")?.prepare()?;
        let mut clock = AudioClock { origin: 0.0, conductor, exact_bpm: metadata.exact_bpm };
        if clock.exact_bpm.is_some_and(|bpm| !bpm.is_finite() || !(40.0..=240.0).contains(&bpm)) { return Err("Recording receipt has an invalid exact tempo".into()); }
        let trimmed_frames = (-placement.source_seconds_at_start * f64::from(sample.sr)).ceil().max(0.0) as u64;
        if trimmed_frames >= metadata.frames { return Err("Recording ends before the song begins; choose a manual placement".into()); }
        let seconds = placement.source_seconds_at_start + trimmed_frames as f64 / f64::from(sample.sr);
        let start = clock.beat_at_seconds(seconds.max(0.0));
        let duration = clock.beat_at_seconds(seconds.max(0.0) + (metadata.frames - trimmed_frames) as f64 / f64::from(sample.sr)) - start;
        clock.origin = start;
        let mut model = captured.state.arrangement.as_deref().cloned().unwrap_or_default();
        if model.sources.len() >= arrangement::MAX_SOURCES || model.instances.len() >= arrangement::MAX_INSTANCES { return Err("Song has no room for another recording source or placement".into()); }
        let source = model.identity()?;
        let id = model.identity()?;
        let mut clip = project::SavedClip::empty();
        clip.kind = ClipKind::Audio;
        clip.name = sample.name.clone();
        clip.audio = Some(captured.media.len());
        let mut region = audio_clip::Region::full(&sample, captured.state.bpm).map_err(str::to_owned)?;
        region.start = trimmed_frames;
        region.loop_start = trimmed_frames;
        clip.bars = (region.prepare(&sample).map_err(str::to_owned)?.duration_beats / 4.0) as f32;
        clip.audio_region = Some(region);
        captured.media.push(sample);
        model.sources.push(Source { id: source, clip, audio_clock: Some(clock) });
        model.instances.push(Instance { id, source, track: target, start, offset: 0.0, duration, repeating: false, gain: 1.0, fades: None, fade_link: 0, crossfade: None });
        model.validate(&captured.media, layout)?;
        active(cancel)?;
        Ok(Arc::new(Self { captured, model, audio, timing, audio_fingerprint, timing_fingerprint, path: path.to_owned(), start, duration, trimmed_frames, delay_frames: placement.graph_delay_frames, target }))
    }

    /// Admit the reviewed placement through the existing song Undo transaction.
    /// Takes fresh project state and cancellation; returns a prepared request only while project and both source files still match the review.
    pub(crate) fn prepare(&self, mut captured: project::Captured, cancel: &AtomicBool) -> Result<(Box<arrangement::edit::Request>, crate::engine::midi_edit::Ack), String> {
        active(cancel)?;
        if captured.checkpoint != self.captured.checkpoint { return Err("Project changed since recording review; review it again".into()); }
        let (_, audio, fingerprint) = opened(&self.audio.path)?;
        if fingerprint != self.audio_fingerprint { return Err("Recorded WAV changed since review".into()); }
        self.audio.verify_file(&audio, fingerprint).map_err(|e| e.to_string())?;
        let (_, timing, fingerprint) = opened(&self.timing.path)?;
        if fingerprint != self.timing_fingerprint { return Err("Recording timing receipt changed since review".into()); }
        self.timing.verify_file(&timing, fingerprint).map_err(|e| e.to_string())?;
        let mut model = self.model.clone();
        for source in &mut model.sources {
            if let Some(index) = source.clip.audio {
                let sample = self.captured.media.get(index).ok_or("Reviewed recording source disappeared")?;
                source.clip.audio = Some(captured.media.iter().position(|s| Arc::ptr_eq(s, sample)).unwrap_or_else(|| { captured.media.push(sample.clone()); captured.media.len() - 1 }));
            }
        }
        arrangement::edit::Request::prepare(captured, model, cancel)
    }
}

#[cfg(test)]
mod tests;
