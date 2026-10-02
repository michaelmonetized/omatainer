//! One bank preparation job, executed by the existing media decoder worker.
//! Every target/source is captured at admission; GUI selection is never read.
use super::{assets, resident, Definition, Source, SourceRef, SLOTS};
use crate::{
    engine::{decode, media_source::{FileFingerprint, LibSource}, sampler},
    library::Catalog,
};
use sha2::{Digest, Sha256};
use std::{fs::OpenOptions, io::{Read, Seek}, os::unix::fs::OpenOptionsExt, sync::Arc};

#[derive(Clone, Debug)]
pub(crate) struct Assignment {
    pub slot: u8,
    pub source: LibSource,
    pub fingerprint: FileFingerprint,
}
#[derive(Clone, Debug)]
pub(crate) enum Operation {
    Empty { name: String },
    Factory { bank: super::Factory, name: String },
    Copy { bank: sampler::Bank, name: String },
    Definition(Definition),
    Change {
        bank: sampler::Bank,
        settings: resident::Settings,
        assignment: Option<Assignment>,
        retry: Option<u8>,
        clear: Option<u8>,
    },
}
#[derive(Clone, Debug)]
pub(crate) struct Request {
    pub epoch: u64,
    pub revision: u64,
    pub sample_rate: u32,
    pub operation: Operation,
    pub catalog: Arc<Catalog>,
    pub origins: Vec<crate::project_dependencies::Origin>,
}
#[derive(Debug)]
pub(crate) struct Prepared {
    pub epoch: u64,
    pub target: sampler::Target,
    pub bank: sampler::Bank,
    /// Fresh measurements only: never copied from project/definition metadata.
    /// Persist after this request's renderer acknowledgement is Applied.
    pub verified_sources: Vec<SourceRef>,
}
impl Prepared {
    pub fn command(self, ack: sampler::Ack) -> crate::engine::Command {
        crate::engine::Command::SamplerEdit(sampler::Edit {
            epoch: self.epoch, target: self.target, bank: self.bank, select: true, ack,
        })
    }
}

/// Audio/reservation limits count all simultaneously resident and pending
/// sampler sources. A job does not eagerly allocate its reserved credit.
/// Missing reusable sources remain explicit missing slots; a failed individual
/// replacement instead preserves the current bank by returning an error.
pub(crate) fn run(
    request: Request,
    owner: &assets::Owner,
    cancelled: impl Fn() -> bool,
) -> Result<Prepared, String> {
    let check = || if cancelled() { Err("sampler preparation cancelled".to_string()) } else { Ok(()) };
    check()?;
    crate::project_dependencies::validate_origins(&request.origins)?;
    let target;
    let id;
    let mut settings;
    let mut audio = std::array::from_fn(|_| None);
    let mut issues = std::array::from_fn(|_| None);
    let mut reload = [false; SLOTS];
    let tolerate_missing;
    match request.operation {
        Operation::Empty { name } => {
            settings = resident::Settings::empty(name)?;
            id = super::BankId::new()?;
            target = sampler::Target::Append { revision: request.revision };
            tolerate_missing = false;
        }
        Operation::Factory { bank, name } => {
            let originals = crate::engine::sampler::factory_data(owner, request.sample_rate)?;
            let original = &originals[bank.index()];
            settings = (*original.settings).clone();
            settings.name = name;
            settings.definition = None;
            audio = original.audio.clone();
            id = super::BankId::new()?;
            target = sampler::Target::Append { revision: request.revision };
            tolerate_missing = false;
        }
        Operation::Copy { bank, name } => {
            settings = (*bank.data.settings).clone();
            settings.name = name;
            settings.definition = None;
            audio = bank.data.audio.clone();
            issues = bank.data.issues.clone();
            id = super::BankId::new()?;
            target = sampler::Target::Append { revision: request.revision };
            tolerate_missing = false;
        }
        Operation::Definition(definition) => {
            definition.validate()?;
            settings = resident::Settings { name: definition.name, definition: Some(definition.id), slots: definition.slots };
            id = super::BankId::new()?;
            target = sampler::Target::Append { revision: request.revision };
            reload = std::array::from_fn(|slot| settings.slots[slot].source.is_some());
            tolerate_missing = true;
        }
        Operation::Change { bank, settings: next, assignment, retry, clear } => {
            if bank.factory.is_some() { return Err("copy a factory bank before editing its slots".into()); }
            settings = next;
            settings.validate()?;
            if settings.definition != bank.data.settings.definition {
                return Err("an edit cannot redirect its reusable definition identity".into());
            }
            // Source substitutions and clears are explicit operations. This
            // also distinguishes an embedded-only populated slot from empty.
            for slot in 0..SLOTS {
                if settings.slots[slot].source != bank.data.settings.slots[slot].source
                { return Err("slot source changed without a captured library assignment".into()); }
                audio[slot] = bank.data.audio[slot].clone(); issues[slot] = bank.data.issues[slot].clone();
            }
            if let Some(slot) = clear {
                let slot = slot as usize;
                if slot >= SLOTS || assignment.is_some() || retry.is_some() { return Err("invalid combined sampler clear request".into()); }
                settings.slots[slot] = super::Slot::default();
                audio[slot] = None; issues[slot] = None;
            }
            if let Some(assignment) = assignment {
                let slot = assignment.slot as usize;
                if slot >= SLOTS { return Err("invalid sampler slot".into()); }
                let track = request.catalog.track_for_version(&assignment.source, Some(assignment.fingerprint))
                    .ok_or("selected source is no longer in the local library")?;
                let version = track.versions.iter().find(|version| version.fingerprint == Some(assignment.fingerprint))
                    .ok_or("selected source version is unavailable")?;
                let reference = SourceRef { track: track.id.clone(), source: assignment.source,
                    fingerprint: assignment.fingerprint, content_hash: version.content_hash };
                reference.validate()?;
                settings.slots[slot].source = Some(Source::Library { reference });
                // New sources start with a full range. The explicit gain remains.
                settings.slots[slot].controls.start_seconds = 0.0;
                settings.slots[slot].controls.end_seconds = None;
                reload[slot] = true;
            }
            if let Some(slot) = retry {
                let slot = slot as usize;
                if slot >= SLOTS || settings.slots[slot].source.is_none() { return Err("no reusable source to retry in this slot".into()); }
                reload[slot] = true;
            }
            target = sampler::Target::Replace { id: bank.id, revision: bank.revision };
            id = bank.id;
            tolerate_missing = false;
        }
    }
    if settings.definition == Some(id) { return Err("working/reusable identity collision".into()); }
    settings.validate()?;
    check()?;
    let factories = if settings.slots.iter().enumerate().any(|(i, slot)| reload[i] && matches!(slot.source, Some(Source::Factory { .. }))) {
        Some(crate::engine::sampler::factory_data(owner, request.sample_rate)?)
    } else { None };
    let available = owner.available().map_err(|e| e.to_string())?;
    let sources = reload.iter().filter(|reload| **reload).count();
    // Two MiB comfortably bounds 16 fixed peak arrays, source strings and
    // settings. Existing shared samples are accounted by their original pins.
    let budget = assets::Budget { banks: 1, settings: 1, samples: sources,
        pcm_bytes: if sources == 0 { 0 } else { available.pcm_bytes },
        metadata_bytes: available.metadata_bytes.min(2 * 1024 * 1024) };
    if budget.metadata_bytes < 256 * 1024 || sources > available.samples {
        return Err("sampler asset capacity is full; preparation was not started".into());
    }
    let reservation = owner.reserve(budget).map_err(|e| e.to_string())?;
    let mut pcm_remaining = budget.pcm_bytes;
    let mut verified_sources = Vec::with_capacity(SLOTS);
    for slot in 0..SLOTS {
        check()?;
        if !reload[slot] { continue; }
        let source = settings.slots[slot].source.clone().ok_or("missing slot source")?;
        let result: Result<_, SourceError> = match source {
            Source::Project { source, audio_hash } => {
                crate::project_dependencies::load_source(&source, audio_hash, pcm_remaining, &cancelled)
                    .map(|sample| (Arc::new(sample), Source::Project { source, audio_hash })).map_err(|error| SourceError { message: error.detail, missing: !error.capacity })
            }
            Source::Library { reference } if audio[slot].as_ref().is_some_and(|sample| request.origins.iter().any(|origin| origin.key.original_path == sample.path)) => {
                let sample = audio[slot].as_ref().unwrap();
                let hash = crate::project_dependencies::audio_hash_cancelled(sample, &cancelled)?;
                if let Some(origin) = request.origins.iter().find(|origin| origin.key.original_path == sample.path && origin.key.audio_hash == hash) {
                    crate::project_dependencies::load_source(&origin.source, hash, pcm_remaining, &cancelled)
                        .map(|sample| (Arc::new(sample), Source::Project { source: origin.source.clone(), audio_hash: hash })).map_err(|error| SourceError { message: error.detail, missing: !error.capacity })
                } else {
                    reference.resolve(&request.catalog).map_err(SourceError::from).and_then(|mut reference| {
                        let (sample, hash) = decode_reference(&reference, pcm_remaining, &cancelled)?;
                        reference.content_hash = Some(hash); Ok((Arc::new(sample), Source::Library { reference }))
                    })
                }
            }
            Source::Library { reference } => {
                reference.resolve(&request.catalog).map_err(SourceError::from).and_then(|mut reference| {
                    let (sample, hash) = decode_reference(&reference, pcm_remaining, &cancelled)?;
                    reference.content_hash = Some(hash);
                    Ok((Arc::new(sample), Source::Library { reference }))
                })
            }
            Source::Factory { bank, slot } => {
                let data = factories.as_ref().ok_or("factory preparation unavailable")?;
                let source = data[bank.index()].audio.get(slot as usize).and_then(Option::as_ref)
                    .ok_or("invalid original factory slot")?.clone();
                Ok((source, Source::Factory { bank, slot }))
            }
        };
        match result {
            Ok((sample, source)) => {
                let bytes = sample.data.capacity() as u64 * 4;
                // Shared factory PCM already has its own registered credit.
                if matches!(source, Source::Library { .. } | Source::Project { .. }) {
                    pcm_remaining = pcm_remaining.checked_sub(bytes).ok_or("sampler aggregate PCM limit exceeded")?;
                    if let Source::Library { reference } = &source { verified_sources.push(reference.clone()); }
                }
                settings.slots[slot].controls.frames(sample.sr, sample.frames())?;
                settings.slots[slot].source = Some(source);
                audio[slot] = Some(sample);
                issues[slot] = None;
            }
            Err(error) if tolerate_missing && error.missing && !cancelled() => {
                audio[slot] = None;
                issues[slot] = Some(error.message.chars().take(2048).collect());
            }
            Err(error) => return Err(error.message),
        }
    }
    check()?;
    let data = resident::Data::prepare(Arc::new(settings), audio, issues)?;
    let data = reservation.publish(Arc::new(data)).map_err(|e| e.to_string())?;
    check()?;
    Ok(Prepared { epoch: request.epoch, target, verified_sources,
        bank: sampler::Bank { id, revision: 0, factory: None, data } })
}

#[derive(Debug)]
struct SourceError { message: String, missing: bool }
impl From<String> for SourceError { fn from(message: String) -> Self { Self { message, missing: true } } }
impl From<&str> for SourceError { fn from(message: &str) -> Self { message.to_string().into() } }
impl std::fmt::Display for SourceError { fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { self.message.fmt(f) } }

fn decode_reference(
    reference: &SourceRef,
    pcm_limit: u64,
    cancelled: &impl Fn() -> bool,
) -> Result<(crate::engine::dsp::Sample, [u8; 32]), SourceError> {
    let location=crate::media_location::Location::resolve(&reference.source).map_err(|e|e.to_string())?;
    let path=location.path.as_path();
    let mut file = OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path).map_err(|e| format!("sampler source unavailable: {e}"))?;
    let metadata = file.metadata().map_err(|e| e.to_string())?;
    if !metadata.is_file() || metadata.len() > 8 * 1024 * 1024 * 1024
        || FileFingerprint::from_metadata(&metadata) != reference.fingerprint
        || FileFingerprint::read(path) != Some(reference.fingerprint)
    { return Err("sampler source changed or exceeds the 8 GiB source-file limit".into()); }
    let stable = |file: &std::fs::File| -> Result<(), String> {
        if cancelled() { return Err("sampler preparation cancelled".into()); }
        location.verify_file(file,reference.fingerprint).map_err(|e|format!("sampler source changed during preparation: {e}"))?;
        if FileFingerprint::from_metadata(&file.metadata().map_err(|e| e.to_string())?) != reference.fingerprint
            || FileFingerprint::read(path) != Some(reference.fingerprint)
        { return Err("sampler source changed during preparation".into()); }
        Ok(())
    };
    // Hash and decode one open description. Symlink swaps or path reuse never
    // cause a different file to be admitted under the captured identity.
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    let mut total = 0u64;
    loop {
        if cancelled() { return Err("sampler preparation cancelled".into()); }
        let bytes = file.read(&mut buffer).map_err(|e| e.to_string())?;
        if bytes == 0 { break; }
        total += bytes as u64;
        if total > metadata.len() { return Err("sampler source grew during verification".into()); }
        hash.update(&buffer[..bytes]);
    }
    if total != metadata.len() { return Err("sampler source was truncated during verification".into()); }
    let hash: [u8; 32] = hash.finalize().into();
    if reference.content_hash.is_some_and(|expected| expected != hash) {
        return Err("sampler source bytes differ from the saved assignment".into());
    }
    stable(&file)?;
    file.rewind().map_err(|e| e.to_string())?;
    let copy = file.try_clone().map_err(|e| e.to_string())?;
    let decoded = decode::decode_sampler_file(path, copy, pcm_limit, cancelled).map_err(|e| SourceError {
        missing: e.kind != decode::DecodeFailureKind::Capacity, message: e.to_string(),
    })?;
    stable(&file)?;
    Ok((decoded.sample, hash))
}

#[cfg(test)]
mod tests;
