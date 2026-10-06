use super::*;
use crate::sampler_bank::{resident, BankId, Controls};

fn native(rt: &RtEngine) -> sampler::Bank {
    let mut settings = resident::Settings::empty("Embedded user bank".into()).unwrap();
    settings.slots[5].controls = Controls {
        gain: 0.37,
        start_seconds: 0.002,
        end_seconds: Some(0.015),
    };
    let mut audio = std::array::from_fn(|_| None);
    audio[5] = Some(Arc::new(Sample { spectrum: None,
        name: "Missing original; valid embedded PCM".into(),
        sr: 16_000,
        ch: 1,
        data: vec![0.25; 480],
        peaks: vec![[0.1; 3]; 8].into(),
        bpm: 0.0,
        path: "/missing/original.wav".into(),
    }));
    let data =
        resident::Data::prepare(Arc::new(settings), audio, std::array::from_fn(|_| None)).unwrap();
    sampler::Bank::imported(rt.sampler_assets.pin(data).unwrap()).unwrap()
}
fn append(rt: &mut RtEngine, bank: sampler::Bank) {
    rt.apply(Command::SamplerEdit(sampler::Edit {
        epoch: rt.undo.checkpoint().epoch,
        target: sampler::Target::Append {
            revision: rt.sampler_revision,
        },
        bank,
        select: true,
        ack: sampler::Ack::new(),
    }));
}
fn drive_without_heap<T: Send + 'static>(
    rt: &mut RtEngine,
    work: impl FnOnce() -> T + Send + 'static,
) -> T {
    let worker = std::thread::spawn(work);
    wait_until(|| {
        assert_eq!(
            super::super::super::test_alloc::measure(|| rt.process(&mut [])),
            super::super::super::test_alloc::Counts::default()
        );
        worker.is_finished()
    });
    worker.join().unwrap()
}

#[test]
fn state_four_roundtrips_sparse_native_pcm_and_controls_without_reading_missing_originals() {
    let (engine, mut live) = Engine::headless_for_test(48_000, 144);
    let bank = native(&live);
    let id = bank.id;
    let source = bank.data.audio[5].as_ref().unwrap().clone();
    append(&mut live, bank);
    let handle = engine.project.clone();
    let saved =
        drive_without_heap(&mut live, move || handle.capture(&AtomicBool::new(false))).unwrap();
    assert_eq!(saved.state.version, STATE_VERSION);
    let saved_bank = &saved.state.banks[3];
    assert_eq!(saved_bank.instance, Some(id));
    assert_eq!(saved_bank.media.iter().flatten().count(), 1);
    assert_eq!(
        saved_bank.settings.as_ref().unwrap().slots[5].controls.gain,
        0.37
    );
    let bytes = serde_json::to_vec(&saved.state).unwrap();
    let state: State = serde_json::from_slice(&bytes).unwrap();
    state.validate(&saved.media).unwrap();
    let mut prepared = Prepared::from_state(state, saved.media.clone(), 96_000).unwrap();
    let restored = &prepared.rt.sampler_banks[3];
    assert_eq!(restored.id, id);
    assert!(restored.factory.is_none());
    assert!(Arc::ptr_eq(
        restored.data.audio[5].as_ref().unwrap(),
        &source
    ));
    assert_eq!(restored.data.ranges[5], Some((32.0, 240.0)));
    prepared.rt.set_sample_rate(44_100).unwrap();
    assert!(Arc::ptr_eq(
        prepared.rt.sampler_banks[3].data.audio[5].as_ref().unwrap(),
        &source
    ));
    assert_eq!(source.sr, 16_000);
    prepared.rt.sampler_bank = 3;
    prepared.rt.sampler_inst = SamplerInstrument::Samples;
    prepared.rt.apply(Command::SamplerPad { pad: 5, on: true });
    assert_eq!(prepared.rt.pad_voices[5].as_ref().unwrap().gain, 0.37);
    assert_eq!(prepared.rt.pad_voices[5].as_ref().unwrap().position, 32.0);
    assert!(prepared.rt.sampler_banks[3]
        .data
        .definition(BankId::new().unwrap())
        .is_err());
}

#[test]
fn legacy_banks_named_kit_are_embedded_and_never_regenerated_from_the_name() {
    let original = rt();
    let saved = captured(&original);
    for version in [1, 2, 3] {
        let mut json = serde_json::to_value(&saved.state).unwrap();
        json["version"] = version.into();
        legacy_midi_fields(&mut json);
        for bank in json["banks"].as_array_mut().unwrap() {
            let bank = bank.as_object_mut().unwrap();
            bank.remove("instance");
            bank.remove("settings");
        }
        if version < 3 {
            for deck in json["decks"].as_array_mut().unwrap() {
                deck.as_object_mut().unwrap().remove("grid");
            }
        }
        if version < 2 {
            for deck in json["decks"].as_array_mut().unwrap() {
                deck.as_object_mut().unwrap().remove("cue_styles");
            }
        }
        let legacy: State = serde_json::from_value(json).unwrap();
        legacy.validate(&saved.media).unwrap();
        let mut restored = Prepared::from_state(legacy, saved.media.clone(), 96_000).unwrap();
        assert_eq!(restored.rt.sampler_banks[0].name(), "Kit");
        assert!(restored
            .rt
            .sampler_banks
            .iter()
            .all(|bank| bank.factory.is_none()));
        let prior = restored.rt.sampler_banks[0].data.audio[0]
            .as_ref()
            .unwrap()
            .clone();
        restored.rt.set_sample_rate(44_100).unwrap();
        assert!(Arc::ptr_eq(
            &prior,
            restored.rt.sampler_banks[0].data.audio[0].as_ref().unwrap()
        ));
        assert_eq!(prior.sr, 48_000);
        assert!(restored.rt.sampler_banks.iter().all(|bank| bank
            .data
            .settings
            .slots
            .iter()
            .all(|slot| slot.source.is_none() && slot.controls == Controls::default())));
        let recaptured = captured(&restored.rt);
        assert_eq!(recaptured.state.version, STATE_VERSION);
        assert!(recaptured
            .state
            .banks
            .iter()
            .all(|bank| bank.instance.is_some() && bank.settings.is_some()));
    }
}

#[test]
fn actual_project_install_keeps_renderer_owner_and_retires_previous_voice_history_and_capture_off_callback(
) {
    let (engine, mut live) = Engine::headless_for_test(48_000, 144);
    let bank = native(&live);
    let source = bank.data.audio[5].as_ref().unwrap().clone();
    let weak = Arc::downgrade(&source);
    append(&mut live, bank);
    live.apply(Command::SamplerPad { pad: 5, on: true });
    let handle = engine.project.clone();
    let saved =
        drive_without_heap(&mut live, move || handle.capture(&AtomicBool::new(false))).unwrap();
    let old_owner = live.sampler_assets.clone();
    let revision = engine.project.revision();
    let handle = engine.project.clone();
    drive_without_heap(&mut live, move || {
        handle.install(
            Prepared::empty(48_000).unwrap(),
            revision,
            &AtomicBool::new(false),
        )
    })
    .unwrap();
    assert!(live.sampler_assets.same_owner(&old_owner));
    assert!(live.pad_voices.iter().all(Option::is_none));
    live.undo.publish();
    assert_eq!(engine.undo.view().cursor, 0);
    drop(source);
    assert!(
        weak.upgrade().is_some(),
        "the captured project still owns its original PCM"
    );
    let prepared = Prepared::from_state(saved.state, saved.media, 48_000).unwrap();
    assert!(prepared.rt.sampler_assets.same_owner(&old_owner));
    let revision = engine.project.revision();
    let handle = engine.project.clone();
    drive_without_heap(&mut live, move || {
        handle.install(prepared, revision, &AtomicBool::new(false))
    })
    .unwrap();
    assert!(weak.upgrade().is_some());
    let revision = engine.project.revision();
    let handle = engine.project.clone();
    drive_without_heap(&mut live, move || {
        handle.install(
            Prepared::empty(48_000).unwrap(),
            revision,
            &AtomicBool::new(false),
        )
    })
    .unwrap();
    live.publish_for_test();
    live.publish_for_test();
    wait_until(|| weak.upgrade().is_none());
}

#[test]
fn sampler_playback_modes_roundtrip_and_legacy_headers_refuse_even_default_or_null_fields() {
    let mut original = rt();
    let mut bank = native(&original);
    let mut settings = (*bank.data.settings).clone();
    settings.slots[5].playback = crate::sampler_bank::Playback {
        mode: crate::sampler_bank::PlayMode::Toggle, repeat: true, cue_seconds: Some(0.005),
    };
    let data = resident::Data::prepare(Arc::new(settings), bank.data.audio.clone(), bank.data.issues.clone()).unwrap();
    bank.data = original.sampler_assets.pin(data).unwrap();
    append(&mut original, bank);
    let saved = captured(&original);
    let raw = serde_json::to_value(&saved.state).unwrap();
    let state: State = serde_json::from_value(raw.clone()).unwrap();
    state.validate(&saved.media).unwrap();
    let mut restored = Prepared::from_state(state, saved.media.clone(), 96_000).unwrap();
    restored.rt.sampler_inst = SamplerInstrument::Samples;
    restored.rt.apply(Command::SamplerPad { pad: 5, on: true });
    let voice = restored.rt.pad_voices[5].as_ref().unwrap();
    assert_eq!(voice.position, 80.0);
    assert_eq!(voice.playback.mode, crate::sampler_bank::PlayMode::Toggle);
    assert!(voice.playback.repeat);
    let mut legacy = raw;
    legacy["version"] = serde_json::json!(11);
    assert!(serde_json::from_value::<State>(legacy.clone()).is_err());
    for field in [serde_json::json!(crate::sampler_bank::Playback::default()), serde_json::Value::Null] {
        legacy["banks"][3]["settings"]["slots"][5]["playback"] = field;
        assert!(serde_json::from_value::<State>(legacy.clone()).is_err());
    }
    legacy["banks"][3]["settings"]["slots"][5].as_object_mut().unwrap().remove("playback");
    let migrated: State = serde_json::from_value(legacy).unwrap();
    migrated.validate(&saved.media).unwrap();
    assert_eq!(migrated.banks[3].settings.as_ref().unwrap().slots[5].playback, crate::sampler_bank::Playback::default());
    let mut incorrect = saved.state;
    incorrect.version = 11;
    assert!(incorrect.validate(&saved.media).is_err());
}
