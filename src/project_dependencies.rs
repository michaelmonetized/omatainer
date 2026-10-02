//! Worker-only dependency inspection and content-qualified source relocation.
use crate::engine::{decode, dsp::Sample, media_source::{FileFingerprint, LibSource}, project::State};
use crate::media_location::{Access, Location, Snapshot};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::{HashMap, HashSet}, fs::OpenOptions, os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf}, sync::{Arc, atomic::{AtomicBool, Ordering}}};
use walkdir::WalkDir;

const MAX_FILES: usize = 4096;
const MAX_ENTRIES: usize = 100_000;
const MAX_MATCHES: usize = 4096;
const MAX_SEARCH_PCM: u64 = 8 * 1024 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Key {
    pub audio_hash: [u8; 32],
    pub original_path: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Origin {
    pub key: Key,
    pub source: LibSource,
}
/// Validate the bounded source references saved with a native project.
/// `origins` supplies immutable audio/path keys and typed replacement sources.
/// Returns an error for invalid sources, duplicate keys or excess metadata.
pub(crate) fn validate_origins(origins: &[Origin]) -> Result<(), String> {
    let mut keys = HashSet::new();
    if origins.len() > crate::project_file::DEFAULT_MEDIA_LIMIT { return Err("Too many project source aliases".into()); }
    for origin in origins {
        if origin.key.original_path.len() > 4096 || origin.key.original_path.contains('\0') || !keys.insert(&origin.key) {
            return Err("Invalid or duplicated project source alias".into());
        }
        crate::media_location::validate_root_source(&origin.source).map_err(|e| e.to_string())?;
    }
    if serde_json::to_vec(origins).map_err(|e| e.to_string())?.len() > 96 * 1024 {
        return Err("Project source alias metadata exceeds 96 KiB; use fewer relinks or shorter paths".into());
    }
    Ok(())
}
fn active(cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::Acquire) { Err("Project dependency operation cancelled".into()) } else { Ok(()) }
}
/// Identify the exact decoded signal independently of its filename or encoding.
/// `sample` supplies rate, channels and PCM bits; `cancel` bounds worker hashing.
/// Returns a SHA-256 identity including shape and every retained float bit.
pub(crate) fn audio_hash(sample: &Sample, cancel: &AtomicBool) -> Result<[u8; 32], String> {
    audio_hash_cancelled(sample, || cancel.load(Ordering::Acquire))
}
/// Hash decoded audio while honoring an owner-provided cancellation check.
/// `sample` supplies rate, channels and PCM; `cancelled` is checked each chunk.
/// Returns the exact audio identity or a cancellation error.
pub(crate) fn audio_hash_cancelled(sample: &Sample, cancelled: impl Fn() -> bool) -> Result<[u8; 32], String> {
    let mut hash = Sha256::new();
    hash.update(b"omatainer-project-pcm-v1\0"); hash.update(sample.sr.to_le_bytes());
    hash.update(sample.ch.to_le_bytes()); hash.update((sample.data.len() as u64).to_le_bytes());
    for chunk in sample.data.chunks(1024) {
        if cancelled() { return Err("Project source verification cancelled".into()); }
        let mut bytes = [0u8; 4096];
        for (value, bytes) in chunk.iter().zip(bytes.chunks_exact_mut(4)) { bytes.copy_from_slice(&value.to_bits().to_le_bytes()); }
        hash.update(&bytes[..chunk.len() * 4]);
    }
    if cancelled() { return Err("Project source verification cancelled".into()); }
    Ok(hash.finalize().into())
}

#[derive(Clone, Debug)]
pub(crate) enum Availability { Builtin, Verified, Unresolved(String) }
#[derive(Clone, Debug)]
pub(crate) struct Asset {
    pub key: Key,
    pub name: String,
    pub source: Option<LibSource>,
    pub availability: Availability,
    pub pcm_bytes: u64,
    pub frames: usize,
    pub uses: usize,
}
#[derive(Clone, Debug)]
pub(crate) struct Device {
    pub placement: String,
    pub identifier: String,
    pub state_schema: Option<u32>,
    pub state_bytes: usize,
    pub requested_on: bool,
    pub rendered_fallbacks: Option<usize>,
}
#[derive(Clone, Debug)]
pub(crate) struct MissingSource {
    pub placement: String,
    pub source: String,
    pub file_hash: Option<[u8; 32]>,
}
#[derive(Clone, Debug, Default)]
pub(crate) struct Inventory { pub assets: Vec<Asset>, pub devices: Vec<Device>, pub missing: Vec<MissingSource> }

/// Inspect saved sources while keeping embedded media and device state intact.
/// `state`, `media` and `origins` are one captured project; `cancel` stops I/O.
/// Returns every embedded dependency and every unavailable effect, or an error.
pub(crate) fn inspect(state: &State, media: &[Arc<Sample>], origins: &[Origin], cancel: &AtomicBool) -> Result<Inventory, String> {
    validate_origins(origins)?; state.validate(media)?;
    let snapshot = Snapshot::discover().map_err(|e| e.to_string())?;
    let mut inventory = Inventory::default();
    let mut known = HashMap::<Key, usize>::new();
    for (index, sample) in media.iter().enumerate() {
        active(cancel)?;
        let key = Key { audio_hash: audio_hash(sample, cancel)?, original_path: sample.path.clone() };
        let uses = state.tracks.iter().map(|track| track.drums.iter().filter(|&&i| i == index).count()
            + track.clips.iter().filter(|clip| clip.audio == Some(index)).count()).sum::<usize>()
            + state.decks.iter().filter(|deck| deck.audio == Some(index)).count()
            + state.banks.iter().map(|bank| bank.media.iter().filter(|&&i| i == Some(index)).count()).sum::<usize>()
            + state.builtin.iter().filter(|&&i| i == Some(index)).count();
        if let Some(&existing) = known.get(&key) { inventory.assets[existing].uses += uses; continue; }
        let source = origins.iter().find(|origin| origin.key == key).map(|origin| origin.source.clone())
            .or_else(|| state.banks.iter().find_map(|bank| bank.settings.as_ref().and_then(|settings|
                bank.media.iter().position(|&i| i == Some(index)).and_then(|slot| settings.slots[slot].source.as_ref())
                    .and_then(|source| match source { crate::sampler_bank::Source::Library { reference } => Some(reference.source.clone()), crate::sampler_bank::Source::Project { source, .. } => Some(source.clone()), _ => None }))))
            .or_else(|| Path::new(&sample.path).is_absolute().then(|| LibSource::File(sample.path.clone().into())));
        let pcm_bytes = sample.data.len() as u64 * 4;
        let availability = if let Some(source) = &source {
            match snapshot.resolve(source).map_err(|e| e.to_string()).and_then(|location|
                measure(&snapshot, location, pcm_bytes, cancel).map_err(|error| error.detail).and_then(|(hash, _, _)|
                    if hash == key.audio_hash { Ok(()) } else { Err("Source audio differs from the embedded project audio".into()) }))
            {
                Ok(()) => Availability::Verified,
                Err(reason) => { active(cancel)?; Availability::Unresolved(reason) }
            }
        } else { Availability::Builtin };
        known.insert(key.clone(), inventory.assets.len());
        inventory.assets.push(Asset { key, name: sample.name.clone(), source, availability, pcm_bytes, frames: sample.frames(), uses });
    }
    for (bank_index, bank) in state.banks.iter().enumerate() {
        if let Some(settings) = &bank.settings {
            for (slot, value) in settings.slots.iter().enumerate() {
                if bank.media[slot].is_some() { continue; }
                if let Some(source) = &value.source {
                    let (source, file_hash) = match source {
                        crate::sampler_bank::Source::Library { reference } => (format!("{:?}", reference.source), reference.content_hash),
                        crate::sampler_bank::Source::Project { source, .. } => (format!("{source:?}"), None),
                        crate::sampler_bank::Source::Factory { bank, slot } => (format!("Factory {} · slot {}", bank.name(), slot + 1), None),
                    };
                    inventory.missing.push(MissingSource { placement: format!("Bank {} · slot {}", bank_index + 1, slot + 1), source, file_hash });
                }
            }
        }
    }
    for (placement, effect) in state.tracks.iter().enumerate().flat_map(|(track, value)|
        value.fx.iter().enumerate().map(move |(slot, effect)| (format!("Track {} · device {}", track + 1, slot + 1), effect)))
        .chain(state.scene_fx.iter().enumerate().flat_map(|(scene, rack)| rack.iter().enumerate()
            .map(move |(slot, effect)| (format!("Scene {} · device {}", scene + 1, slot + 1), effect))))
    {
        active(cancel)?;
        if let Some(device) = &effect.offline {
            inventory.devices.push(Device { placement, identifier: device.identifier.clone(),
                state_schema: device.state.as_ref().map(|state| state.schema), state_bytes: device.state.as_ref().map_or(0, |state| state.data.len()), requested_on: effect.on, rendered_fallbacks: None });
        }
    }
    for (placement, synth, fallbacks) in state.tracks.iter().enumerate().map(|(track, value)|
        (format!("Track {} · instrument", track + 1), &value.synth, if value.kind == 0 { 0 } else { value.clips.iter().filter(|clip| clip.audio.is_some()).count() }))
        .chain(std::iter::once(("Sampler instrument".into(), &state.sampler_synth, 0))) {
        active(cancel)?;
        if let Some(device) = &synth.offline {
            inventory.devices.push(Device { placement, identifier: device.identifier.clone(),
                state_schema: device.state.as_ref().map(|state| state.schema), state_bytes: device.state.as_ref().map_or(0, |state| state.data.len()),
                requested_on: true, rendered_fallbacks: Some(fallbacks) });
        }
    }
    Ok(inventory)
}

#[derive(Clone, Debug)]
pub(crate) struct Candidate {
    pub location: Location,
    pub fingerprint: FileFingerprint,
    pub file_hash: [u8; 32],
    access: Access,
}
struct MeasureError { detail: String, unsupported: bool, before_decode: bool }
impl From<String> for MeasureError {
    fn from(detail: String) -> Self { Self { detail, unsupported: false, before_decode: false } }
}
impl From<&str> for MeasureError {
    fn from(detail: &str) -> Self { detail.to_string().into() }
}
/// Recognize headers for the local audio formats supported by this build.
/// `file` is already verified regular; returns a prefix match without seeking.
fn audio_header(file: &std::fs::File) -> Result<bool, String> {
    use std::os::unix::fs::FileExt;
    let mut bytes = [0u8; 12]; let count = file.read_at(&mut bytes, 0).map_err(|e| e.to_string())?;
    let bytes = &bytes[..count];
    Ok([b"RIFF".as_slice(), b"RF64", b"FORM", b"fLaC", b"OggS", b"ID3"].iter().any(|header| bytes.starts_with(header))
        || bytes.len() >= 2 && bytes[0] == 0xff && bytes[1] & 0xe0 == 0xe0
        || bytes.get(4..8).is_some_and(|kind| [b"ftyp".as_slice(), b"moov", b"mdat"].contains(&kind)))
}
fn measure(snapshot: &Snapshot, location: Location, pcm_bytes: u64, cancel: &AtomicBool) -> Result<([u8; 32], Candidate, u64), MeasureError> {
    active(cancel)?;
    location.recheck_with(snapshot).map_err(|e| e.to_string())?;
    let access = snapshot.access(&location.path).map_err(|e| e.to_string())?;
    let file = OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(&location.path).map_err(|e| e.to_string())?;
    let before = file.metadata().map_err(|e| e.to_string())?;
    let fingerprint = FileFingerprint::from_metadata(&before);
    if !before.is_file() || FileFingerprint::read(&location.path) != Some(fingerprint) { return Err("Source is not a stable regular file".into()); }
    let retained = file.try_clone().map_err(|e| e.to_string())?;
    let recognizable = audio_header(&file)?;
    let bound = pcm_bytes.saturating_add(4 * 1024 * 1024).min(crate::project_file::DEFAULT_PCM_LIMIT);
    let decoded = decode::decode_sampler_file(&location.path, file, bound, || cancel.load(Ordering::Acquire)).map_err(|e| {
        let unsupported = e.kind == decode::DecodeFailureKind::Unsupported
            || e.kind == decode::DecodeFailureKind::Incomplete && e.stage == decode::DecodeStage::Probe && !recognizable;
        let stable = !unsupported || retained.metadata().ok().map(|metadata| FileFingerprint::from_metadata(&metadata)) == Some(fingerprint)
            && FileFingerprint::read(&location.path) == Some(fingerprint)
            && Snapshot::discover().is_ok_and(|after| access.check(&after, &location.path).is_ok() && location.recheck_with(&after).is_ok());
        MeasureError { detail: if stable { e.to_string() } else { "Source changed during format verification".into() }, unsupported: unsupported && stable,
            before_decode: matches!(e.stage, decode::DecodeStage::Probe | decode::DecodeStage::CreateDecoder) }
    })?;
    let hash = audio_hash(&decoded.sample, cancel)?;
    let decoded_bytes = decoded.sample.data.len() as u64 * 4;
    if FileFingerprint::from_metadata(&retained.metadata().map_err(|e| e.to_string())?) != fingerprint { return Err("Source changed during audio verification".into()); }
    let file_hash = crate::library::hash_project_source(&location.path, fingerprint, || !cancel.load(Ordering::Acquire))?;
    let after = Snapshot::discover().map_err(|e| e.to_string())?;
    access.check(&after, &location.path).map_err(|e| e.to_string())?;
    location.recheck_with(&after).map_err(|e| e.to_string())?;
    if FileFingerprint::read(&location.path) != Some(fingerprint) { return Err("Source changed during verification".into()); }
    Ok((hash, Candidate { location, fingerprint, file_hash, access }, decoded_bytes))
}

#[derive(Clone, Debug)]
pub(crate) struct Search {
    pub matches: Vec<Vec<Candidate>>,
    pub complete: bool,
    pub entries: usize,
    pub files: usize,
    pub skipped_unsupported: usize,
    pub warnings: Vec<String>,
}
impl Search {
    fn warning(&mut self, message: String) {
        self.complete = false;
        if self.warnings.len() < 32 { self.warnings.push(message); }
    }
}
/// Search explicit roots for exact decoded-audio identities; never choose a match.
/// `assets` retains review order; `roots` bounds scope; `cancel` stops work.
/// Returns all bounded candidates plus an explicit incomplete-search receipt.
pub(crate) fn search(assets: &[Asset], roots: &[PathBuf], cancel: &AtomicBool) -> Result<Search, String> {
    if roots.is_empty() || roots.len() > 64 { return Err("Choose 1–64 search folders".into()); }
    let snapshot = Snapshot::discover().map_err(|e| e.to_string())?;
    let mut result = Search { matches: vec![Vec::new(); assets.len()], complete: true, entries: 0, files: 0, skipped_unsupported: 0, warnings: Vec::new() };
    let mut seen = HashSet::new();
    let mut total_pcm = 0u64;
    let mut total_source = 0u64;
    let mut matches = 0usize;
    let pcm_bound = assets.iter().filter(|asset| matches!(asset.availability, Availability::Unresolved(_))).map(|asset| asset.pcm_bytes).max().unwrap_or(0);
    if pcm_bound == 0 { return Err("No unresolved external audio sources need relinking".into()); }
    for root in roots {
        active(cancel)?;
        let root = match snapshot.identify(root).and_then(|root| snapshot.inspect(&root).map(|_| root)) {
            Ok(root) if root.path.is_dir() => root,
            Ok(_) => { result.warning(format!("Not a folder: {}", root.display())); continue; }
            Err(error) => { result.warning(format!("{}: {error}", root.display())); continue; }
        };
        let root_access = snapshot.access(&root.path).map_err(|e| e.to_string())?;
        let mut walk = WalkDir::new(&root.path).follow_links(false).max_depth(64).into_iter();
        while let Some(entry) = walk.next() {
            active(cancel)?;
            if result.entries == MAX_ENTRIES || result.files == MAX_FILES || total_pcm.saturating_add(pcm_bound + 4 * 1024 * 1024) > MAX_SEARCH_PCM || matches == MAX_MATCHES {
                result.warning("Search reached its entry, file, decoded-audio or candidate bound; narrow the folders and retry".into()); return Ok(result);
            }
            result.entries += 1;
            let entry = match entry { Ok(entry) => entry, Err(error) => { result.warning(error.to_string()); continue; } };
            if entry.file_type().is_symlink() { result.warning(format!("Symlink not searched: {}", entry.path().display())); continue; }
            if root_access.check(&snapshot, entry.path()).is_err() {
                result.warning(format!("Nested or changed mount not searched: {}", entry.path().display())); walk.skip_current_dir(); continue;
            }
            if entry.file_type().is_dir() {
                if entry.depth() == 64 { result.warning(format!("Depth limit: {}", entry.path().display())); }
                continue;
            }
            if !entry.file_type().is_file() || !seen.insert(entry.path().to_path_buf()) { continue; }
            result.files += 1;
            let location = match snapshot.identify(entry.path()) { Ok(location) => location, Err(error) => { result.warning(error.to_string()); continue; } };
            let source_bytes = entry.metadata().map(|metadata| metadata.len()).unwrap_or(MAX_SEARCH_PCM + 1);
            if total_source.saturating_add(source_bytes) > MAX_SEARCH_PCM {
                result.warning("Search reached its 8 GiB source-file bound; narrow the folders and retry".into()); return Ok(result);
            }
            total_source += source_bytes;
            let credit = pcm_bound.saturating_add(4 * 1024 * 1024).min(crate::project_file::DEFAULT_PCM_LIMIT);
            total_pcm += credit;
            let (hash, candidate, decoded) = match measure(&snapshot, location, pcm_bound, cancel) {
                Ok(measured) => measured,
                Err(error) => {
                    active(cancel)?;
                    if error.unsupported {
                        result.skipped_unsupported += 1;
                        if error.before_decode { total_pcm -= credit; }
                    } else { result.warning(format!("{}: {}", entry.path().display(), error.detail)); }
                    continue;
                }
            };
            total_pcm = total_pcm - credit + decoded;
            for (index, asset) in assets.iter().enumerate() {
                if matches!(asset.availability, Availability::Unresolved(_)) && hash == asset.key.audio_hash {
                    if matches == MAX_MATCHES { result.warning("Candidate limit; narrow the search".into()); return Ok(result); }
                    matches += 1;
                    result.matches[index].push(candidate.clone());
                }
            }
        }
    }
    Ok(result)
}

/// Reverify every reviewed choice before publishing source aliases as one batch.
/// `choices` binds immutable asset keys to candidates; `cancel` stops precommit.
/// Returns verified aliases, or rejects the entire batch without any edits.
pub(crate) fn verify_choices(choices: &[(Asset, Candidate)], cancel: &AtomicBool) -> Result<Vec<Origin>, String> {
    if choices.is_empty() || choices.len() > crate::project_file::DEFAULT_MEDIA_LIMIT { return Err("Choose 1–256 replacement sources".into()); }
    let snapshot = Snapshot::discover().map_err(|e| e.to_string())?;
    let mut origins = Vec::new();
    for (asset, candidate) in choices {
        active(cancel)?;
        candidate.access.check(&snapshot, &candidate.location.path).map_err(|e| e.to_string())?;
        if FileFingerprint::read(&candidate.location.path) != Some(candidate.fingerprint) { return Err("A reviewed source changed; search again".into()); }
        let (hash, current, _) = measure(&snapshot, candidate.location.clone(), asset.pcm_bytes, cancel).map_err(|error| error.detail)?;
        if hash != asset.key.audio_hash || current.file_hash != candidate.file_hash || current.fingerprint != candidate.fingerprint {
            return Err("A reviewed source no longer matches the project audio; no aliases were changed".into());
        }
        origins.push(Origin { key: asset.key.clone(), source: current.location.source });
    }
    validate_origins(&origins)?; active(cancel)?; Ok(origins)
}

#[cfg(test)]
mod tests;

/// Load a relinked project source and verify its retained decoded identity.
/// `source` is local or removable; `expected` is PCM identity; `limit` bounds
/// decoded bytes; `cancelled` stops disk work. Returns the exact source audio.
pub(crate) fn load_source(source: &LibSource, expected: [u8; 32], limit: u64, cancelled: impl Fn() -> bool) -> Result<Sample, LoadError> {
    crate::media_location::validate_root_source(source).map_err(|e| e.to_string())?;
    if cancelled() { return Err("Project source verification cancelled".into()); }
    let snapshot = Snapshot::discover().map_err(|e| e.to_string())?;
    let location = snapshot.resolve(source).map_err(|e| e.to_string())?;
    let access = snapshot.access(&location.path).map_err(|e| e.to_string())?;
    let file = OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK).open(&location.path).map_err(|e| e.to_string())?;
    let metadata = file.metadata().map_err(|e| e.to_string())?;
    let fingerprint = FileFingerprint::from_metadata(&metadata);
    if !metadata.is_file() || metadata.len() > MAX_SEARCH_PCM { return Err("Project source is not a regular file within 8 GiB".into()); }
    location.verify_file(&file, fingerprint).map_err(|e| e.to_string())?;
    let retained = file.try_clone().map_err(|e| e.to_string())?;
    let sample = decode::decode_sampler_file(&location.path, file, limit, &cancelled).map_err(|e| LoadError { detail: e.to_string(), capacity: e.kind == decode::DecodeFailureKind::Capacity })?.sample;
    if audio_hash_cancelled(&sample, &cancelled)? != expected { return Err("Project source differs from the retained decoded audio".into()); }
    let after = Snapshot::discover().map_err(|e| e.to_string())?;
    access.check(&after, &location.path).map_err(|e| e.to_string())?;
    location.recheck_with(&after).map_err(|e| e.to_string())?;
    location.verify_file(&retained, fingerprint).map_err(|e| e.to_string())?;
    if cancelled() { return Err("Project source verification cancelled".into()); }
    Ok(sample)
}

#[derive(Debug)]
pub(crate) struct LoadError { pub detail: String, pub capacity: bool }
impl From<String> for LoadError { fn from(detail: String) -> Self { Self { detail, capacity: false } } }
impl From<&str> for LoadError { fn from(detail: &str) -> Self { detail.to_owned().into() } }
