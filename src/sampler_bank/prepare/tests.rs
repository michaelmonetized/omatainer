use super::*;
use crate::{engine::media_source::FileFingerprint, sampler_bank::{BankId, Controls, Slot}};
use std::{path::PathBuf, sync::atomic::{AtomicUsize, Ordering}};

pub(crate) struct Files(pub PathBuf);
impl Files {
    pub fn new() -> Self {
        let path = std::env::temp_dir().join(format!("omat-sampler-prepare-{}", BankId::new().unwrap()));
        std::fs::create_dir(&path).unwrap(); Self(path)
    }
    pub fn source(&self, name: &str, bytes: &[u8], catalog: &mut Catalog) -> SourceRef {
        let path = self.0.join(name); std::fs::write(&path, bytes).unwrap();
        let source = LibSource::File(path);
        let fingerprint = FileFingerprint::read(match &source { LibSource::File(path) => path, _ => unreachable!() }).unwrap();
        catalog.upsert(source.clone(), Some(fingerprint), crate::library::Metadata {
            title: name.into(), artist: String::new(), bpm: crate::ui::bpm::Bpm::UNKNOWN,
            key: String::new(), duration: None, last_play: None,
        }).unwrap();
        SourceRef { track: catalog.track(&source).unwrap().id.clone(), source, fingerprint, content_hash: None }
    }
}
impl Drop for Files { fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); } }
pub(crate) fn wav(frames: usize, sr: u32, channels: u16) -> Vec<u8> {
    let size = frames as u32 * u32::from(channels) * 2;
    let mut out = Vec::new(); out.extend_from_slice(b"RIFF"); out.extend_from_slice(&(36 + size).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt "); out.extend_from_slice(&16u32.to_le_bytes()); out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&channels.to_le_bytes()); out.extend_from_slice(&sr.to_le_bytes()); out.extend_from_slice(&(sr * u32::from(channels) * 2).to_le_bytes());
    out.extend_from_slice(&(channels * 2).to_le_bytes()); out.extend_from_slice(&16u16.to_le_bytes()); out.extend_from_slice(b"data"); out.extend_from_slice(&size.to_le_bytes());
    for i in 0..frames * channels as usize { out.extend_from_slice(&((i as i16 % 512) * 32).to_le_bytes()); } out
}
fn request(definition: Definition, catalog: &Catalog) -> Request {
    Request { epoch: 9, revision: 4, sample_rate: 48_000, operation: Operation::Definition(definition), catalog: Arc::new(catalog.clone()) }
}
fn definition(sources: &[SourceRef]) -> Definition {
    let mut definition = Definition::empty("Mixed reusable bank".into()).unwrap();
    for (i, reference) in sources.iter().enumerate() {
        definition.slots[i] = Slot { source: Some(Source::Library { reference: reference.clone() }), controls: Controls::default() };
    }
    definition
}

#[test]
fn real_mixed_formats_and_verified_moves_keep_source_rates_ranges_and_independent_import_ids() {
    let files = Files::new(); let mut catalog = Catalog::default();
    let wav = files.source("mono.wav", &wav(16_000, 16_000, 1), &mut catalog);
    let flac = files.source("stereo.flac", include_bytes!("../../../tests/fixtures/audio/tone.flac"), &mut catalog);
    let ogg = files.source("vorbis.ogg", include_bytes!("../../../tests/fixtures/audio/tone.ogg"), &mut catalog);
    let mut definition = definition(&[wav.clone(), flac, ogg]);
    definition.slots[0].controls = Controls { gain: 0.25, start_seconds: 0.125, end_seconds: Some(0.5) };
    let owner = assets::Owner::isolated_for_test(assets::Budget::limits());
    let first = run(request(definition.clone(), &catalog), &owner, || false).unwrap();
    let second = run(request(definition.clone(), &catalog), &owner, || false).unwrap();
    assert_eq!(first.verified_sources.len(), 3);
    let copy = run(Request { epoch: 9, revision: 4, sample_rate: 48_000,
        operation: Operation::Copy { bank: first.bank.clone(), name: "Copy".into() }, catalog: Arc::new(catalog.clone()) }, &owner, || false).unwrap();
    assert!(copy.verified_sources.is_empty(), "serialized/copied identities are not new measurements");
    assert_ne!(first.bank.id, second.bank.id); assert_ne!(first.bank.id, definition.id);
    assert_eq!(first.bank.data.settings.definition, Some(definition.id));
    assert_eq!(first.bank.data.ranges[0], Some((2000.0, 8000.0)));
    assert_eq!(first.bank.data.audio[0].as_ref().unwrap().sr, 16_000);
    for slot in 0..3 {
        assert!(first.bank.data.issues[slot].is_none());
        assert!(first.bank.data.audio[slot].as_ref().unwrap().data.iter().any(|sample| sample.abs() > 0.001));
        let Some(Source::Library { reference }) = &first.bank.data.settings.slots[slot].source else { panic!() };
        assert!(reference.content_hash.is_some());
    }
    let moved = files.0.join("moved.wav"); std::fs::copy(wav.path().unwrap(), &moved).unwrap();
    catalog.relocate(&crate::library::Relocate { id: wav.track.clone(), source: wav.source.clone(), fingerprint: wav.fingerprint, destination: moved.clone() }).unwrap();
    std::fs::remove_file(wav.path().unwrap()).unwrap();
    let restored = run(request(definition, &catalog), &owner, || false).unwrap();
    let Some(Source::Library { reference }) = &restored.bank.data.settings.slots[0].source else { panic!() };
    assert_eq!(reference.path().unwrap(), moved); assert_eq!(reference.track, wav.track);
    assert_eq!(restored.bank.data.audio[0].as_ref().unwrap().data, first.bank.data.audio[0].as_ref().unwrap().data);
}

#[test]
fn missing_or_damaged_reusable_sources_are_explicit_and_assignment_failure_keeps_existing_pcm() {
    let files = Files::new(); let mut catalog = Catalog::default();
    let good = files.source("good.wav", &wav(1024, 44_100, 2), &mut catalog);
    let bad = files.source("damaged.wav", b"not audio", &mut catalog);
    let absent = files.source("gone.wav", &wav(128, 8000, 1), &mut catalog);
    std::fs::remove_file(absent.path().unwrap()).unwrap();
    let owner = assets::Owner::isolated_for_test(assets::Budget::limits());
    let loaded = run(request(definition(&[good, bad.clone(), absent]), &catalog), &owner, || false).unwrap();
    assert!(loaded.bank.data.audio[0].is_some());
    for slot in [1, 2] { assert!(loaded.bank.data.audio[slot].is_none()); assert!(loaded.bank.data.issues[slot].is_some()); }
    assert!(loaded.bank.data.audio[3].is_none()); assert!(loaded.bank.data.issues[3].is_none());
    let prior = loaded.bank.clone();
    let result = run(Request { operation: Operation::Change { settings: (*prior.data.settings).clone(), bank: prior.clone(),
        assignment: Some(Assignment { slot: 0, source: bad.source, fingerprint: bad.fingerprint }), retry: None, clear: None },
        epoch: 9, revision: 4, sample_rate: 48_000, catalog: Arc::new(catalog) }, &owner, || false);
    assert!(result.is_err()); assert!(Arc::ptr_eq(prior.data.audio[0].as_ref().unwrap(), loaded.bank.data.audio[0].as_ref().unwrap()));
}

#[test]
fn bounded_decode_rejects_declared_overflow_and_descriptor_swap_without_loading_a_prefix() {
    let files = Files::new(); let mut catalog = Catalog::default();
    let source = files.source("bounded.wav", &wav(16_000, 16_000, 1), &mut catalog);
    let error = decode::decode_sampler_file(source.path().unwrap(), std::fs::File::open(source.path().unwrap()).unwrap(), 1024, || false).unwrap_err();
    assert_eq!(error.kind, decode::DecodeFailureKind::Capacity);
    assert_eq!(error.diagnostics.decoded_frames, 0);
    let replacement = files.0.join("replacement.wav"); std::fs::write(&replacement, wav(16_000, 16_000, 1)).unwrap();
    let calls = AtomicUsize::new(0);
    let result = decode_reference(&source, 128 * 1024, &|| {
        if calls.fetch_add(1, Ordering::Relaxed) == 1 { std::fs::rename(&replacement, source.path().unwrap()).unwrap(); }
        false
    });
    assert!(result.unwrap_err().to_string().contains("changed during preparation"));
}

#[test]
fn aggregate_pcm_saturation_rejects_before_excess_decode_and_returns_all_reserved_credit() {
    let files = Files::new(); let mut catalog = Catalog::default();
    let sources: Vec<_> = (0..3).map(|i| files.source(&format!("source-{i}.wav"), &wav(8000, 16000, 1), &mut catalog)).collect();
    let limits = assets::Budget { pcm_bytes: 80_000, ..assets::Budget::limits() };
    let owner = assets::Owner::isolated_for_test(limits);
    let error = run(request(definition(&sources), &catalog), &owner, || false).unwrap_err();
    assert!(error.contains("reserved sampler PCM"), "{error}");
    assert_eq!(owner.available().unwrap(), limits, "failed preparation returns its entire reservation");
    assert!(run(request(definition(&sources[..2]), &catalog), &owner, || false).is_ok());
}

#[test]
fn originals_can_be_prepared_without_any_existing_working_factory_bank() {
    let owner = assets::Owner::isolated_for_test(assets::Budget::limits());
    let catalog = Arc::new(Catalog::default());
    let mut prior_ids = Vec::new();
    for factory in [super::super::Factory::Kit, super::super::Factory::Perc, super::super::Factory::Hits] {
        for _ in 0..2 {
            let prepared = run(Request {
                epoch: 23, revision: 71, sample_rate: 44_100,
                operation: Operation::Factory { bank: factory, name: "My original copy".into() },
                catalog: catalog.clone(),
            }, &owner, || false).unwrap();
            assert_eq!(prepared.epoch, 23);
            assert!(matches!(prepared.target, sampler::Target::Append { revision: 71 }));
            assert!(prepared.bank.factory.is_none(), "the new instance is editable and source-rate owned");
            assert!(prepared.bank.data.settings.definition.is_none());
            assert_eq!(prepared.bank.data.settings.name, "My original copy");
            assert!(prepared.verified_sources.is_empty());
            assert!(!prior_ids.contains(&prepared.bank.id));
            prior_ids.push(prepared.bank.id);
            for slot in 0..SLOTS {
                assert_eq!(prepared.bank.data.settings.slots[slot].source, Some(Source::Factory { bank: factory, slot: slot as u8 }));
                assert_eq!(prepared.bank.data.audio[slot].as_ref().unwrap().sr, 44_100);
                assert!(prepared.bank.data.issues[slot].is_none());
            }
        }
    }
}
