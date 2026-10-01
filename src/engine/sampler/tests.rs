use super::*;
use crate::sampler_bank::{
    resident::{Data, Settings},
    Controls,
};

fn fixture() -> (Engine, RtEngine) {
    let (engine, mut rt) = Engine::headless_for_test(48_000, 256);
    rt.decks.iter_mut().for_each(|deck| deck.playing = false);
    rt.selected_track = 4;
    rt.process(&mut [0.0; 128]);
    (engine, rt)
}
pub(super) fn native_sample() -> Arc<Sample> {
    Arc::new(Sample {
        name: "Native-rate source".into(),
        sr: 16_000,
        ch: 2,
        data: (0..1024).flat_map(|_| [0.2, 0.4]).collect(),
        peaks: vec![[0.1, 0.2, 0.3]; 32].into(),
        bpm: 0.0,
        path: "/missing/original.wav".into(),
    })
}
fn bank(owner: &assets::Owner, audio: Arc<Sample>, controls: Controls) -> Bank {
    let mut settings = Settings::empty("User sampler".into()).unwrap();
    settings.slots[0].controls = controls;
    let mut samples = std::array::from_fn(|_| None);
    samples[0] = Some(audio);
    let data = Data::prepare(Arc::new(settings), samples, std::array::from_fn(|_| None)).unwrap();
    Bank::imported(owner.pin(data).unwrap()).unwrap()
}
pub(super) fn edit(rt: &RtEngine, bank: Bank, target: Target) -> Edit {
    Edit {
        epoch: rt.undo.checkpoint().epoch,
        target,
        bank,
        select: true,
        ack: Ack::new(),
    }
}
fn apply(engine: &Engine, rt: &mut RtEngine, edit: Edit) {
    let ack = edit.ack.clone();
    engine.send(Command::SamplerEdit(edit)).unwrap();
    let counts = test_alloc::measure(|| rt.process(&mut [0.0; 2]));
    assert_eq!((counts.allocations, counts.frees), (0, 0));
    assert_eq!(ack.state(), EditState::Applied);
}

#[test]
fn immutable_bank_edit_undo_redo_preserves_original_held_source_range_gain_and_destination() {
    let (engine, mut rt) = fixture();
    let source = native_sample();
    let controls = Controls {
        gain: 0.5,
        start_seconds: 10.0 / 16_000.0,
        end_seconds: Some(80.0 / 16_000.0),
    };
    let first = bank(&engine.sampler_assets, source.clone(), controls);
    let id = first.id;
    let request = edit(
        &rt,
        first,
        Target::Append {
            revision: rt.sampler_revision,
        },
    );
    apply(&engine, &mut rt, request);
    let index = rt.sampler_bank;
    rt.apply(Command::SamplerPad { pad: 0, on: true });
    let voice = rt.pad_voices[0].as_ref().unwrap();
    assert!(Arc::ptr_eq(&voice.audio, &source));
    assert_eq!(
        (voice.position, voice.end, voice.gain, voice.track),
        (10.0, 80.0, 0.5, 4)
    );
    rt.selected_track = 7;
    let mut next = bank(
        &engine.sampler_assets,
        native_sample(),
        Controls {
            gain: 0.0,
            ..Controls::default()
        },
    );
    next.id = id;
    let request = edit(
        &rt,
        next,
        Target::Replace {
            id,
            revision: rt.sampler_banks[index].revision,
        },
    );
    apply(&engine, &mut rt, request);
    let revision = rt.sampler_banks[index].revision;
    for command in [Command::Undo, Command::Redo] {
        let counts = test_alloc::measure(|| rt.apply(command));
        assert_eq!((counts.allocations, counts.frees), (0, 0));
        let voice = rt.pad_voices[0].as_ref().unwrap();
        assert!(Arc::ptr_eq(&voice.audio, &source));
        assert_eq!((voice.end, voice.gain, voice.track), (80.0, 0.5, 4));
    }
    assert!(rt.sampler_banks[index].revision > revision);
    assert_eq!(
        rt.sampler_banks[index].data.settings.slots[0].controls.gain,
        0.0
    );
    let output = rt.tick_pad_sources();
    assert_eq!(output[4], [0.1, 0.2]);
    assert_eq!(output[7], [0.0, 0.0]);
    rt.apply(Command::SamplerPad { pad: 0, on: false });
    assert!(
        rt.pad_voices[0].is_some(),
        "one-shot release policy remains unchanged"
    );
    rt.apply(Command::SamplerPad { pad: 0, on: true });
    let output = rt.tick_pad_sources();
    assert_eq!(
        output[7],
        [0.0, 0.0],
        "new onset captures the newly applied zero gain"
    );
}

#[test]
fn cancelled_stale_and_protected_requests_acknowledge_rejection_without_mutation() {
    let (engine, mut rt) = fixture();
    let initial = edit(
        &rt,
        bank(&engine.sampler_assets, native_sample(), Controls::default()),
        Target::Append {
            revision: rt.sampler_revision,
        },
    );
    apply(&engine, &mut rt, initial);
    let target = Target::Replace {
        id: rt.sampler_banks[3].id,
        revision: rt.sampler_banks[3].revision,
    };
    let mut replacement = bank(&engine.sampler_assets, native_sample(), Controls::default());
    replacement.id = rt.sampler_banks[3].id;
    let stale = edit(&rt, replacement.clone(), target);
    rt.apply(Command::Undo);
    rt.apply(Command::Redo);
    let before = engine.undo.checkpoint();
    let ack = stale.ack.clone();
    let counts = test_alloc::measure(|| rt.apply(Command::SamplerEdit(stale)));
    assert_eq!((counts.allocations, counts.frees), (0, 0));
    assert_eq!(ack.state(), EditState::Rejected);
    assert_eq!(engine.undo.checkpoint(), before);
    let target = Target::Replace {
        id: rt.sampler_banks[3].id,
        revision: rt.sampler_banks[3].revision,
    };
    let cancelled = edit(&rt, replacement.clone(), target);
    let ack = cancelled.ack.clone();
    engine.send(Command::SamplerEdit(cancelled)).unwrap();
    assert!(ack.cancel());
    assert_eq!(
        test_alloc::measure(|| rt.process(&mut [0.0; 2])),
        test_alloc::Counts::default()
    );
    assert_eq!(ack.state(), EditState::Rejected);
    assert_eq!(engine.undo.checkpoint(), before);

    let queued = edit(&rt, replacement.clone(), target);
    let queued_ack = queued.ack.clone();
    engine.send(Command::SamplerEdit(queued)).unwrap();
    engine.cmd.performance().set_enabled(true).unwrap();
    let refused = edit(&rt, replacement, target);
    let refused_ack = refused.ack.clone();
    assert!(matches!(
        engine.send(Command::SamplerEdit(refused)),
        Err(SubmissionError::Performance(_))
    ));
    assert_eq!(refused_ack.state(), EditState::Rejected);
    assert_eq!(
        test_alloc::measure(|| rt.process(&mut [0.0; 2])),
        test_alloc::Counts::default()
    );
    assert_eq!(queued_ack.state(), EditState::Rejected);
    assert_eq!(engine.undo.checkpoint(), before);
    assert_eq!(rt.sampler_banks.len(), 4);
}

#[test]
fn only_explicit_factory_origins_regenerate_at_new_output_rates() {
    let (engine, mut rt) = fixture();
    let source = native_sample();
    let user = bank(&engine.sampler_assets, source.clone(), Controls::default());
    let id = user.id;
    let request = edit(
        &rt,
        user,
        Target::Append {
            revision: rt.sampler_revision,
        },
    );
    apply(&engine, &mut rt, request);
    let old = rt.sampler_banks[0].data.audio[0].as_ref().unwrap().clone();
    rt.set_sample_rate(96_000).unwrap();
    for original in &rt.sampler_banks[..3] {
        assert!(original.factory.is_some());
        assert!(original
            .data
            .audio
            .iter()
            .flatten()
            .all(|sample| sample.sr == 96_000));
    }
    assert!(!Arc::ptr_eq(
        &old,
        rt.sampler_banks[0].data.audio[0].as_ref().unwrap()
    ));
    let user = rt.sampler_banks.iter().find(|bank| bank.id == id).unwrap();
    assert!(user.factory.is_none());
    assert!(Arc::ptr_eq(user.data.audio[0].as_ref().unwrap(), &source));
    assert_eq!(source.sr, 16_000);
    assert!(
        user.data
            .definition(crate::sampler_bank::BankId::new().unwrap())
            .is_err(),
        "embedded-only audio must not silently become an empty reusable slot"
    );
}

#[test]
fn cancellation_linearizes_before_history_claim_and_late_cancel_preserves_actual_commit() {
    for cancel_before_claim in [true, false] {
        let (engine, mut rt) = fixture();
        let request = edit(&rt,
            bank(&engine.sampler_assets, native_sample(), Controls::default()),
            Target::Append { revision: rt.sampler_revision });
        let ack = request.ack.clone();
        let checkpoint = engine.undo.checkpoint();
        if cancel_before_claim { assert!(ack.cancel()); }
        let mut claimed = None;
        let counts = test_alloc::measure(|| {
            claimed = rt.history_before(Command::SamplerEdit(request));
        });
        assert_eq!(counts, test_alloc::Counts::default());
        if cancel_before_claim {
            assert!(claimed.is_none());
            assert_eq!(engine.undo.checkpoint(), checkpoint);
            assert_eq!(rt.sampler_banks.len(), 3);
            assert_eq!(ack.state(), EditState::Rejected);
        } else {
            assert!(claimed.is_some());
            assert!(!ack.cancel(), "history claim is the irreversible admission boundary");
            assert_eq!(test_alloc::measure(|| rt.apply_plain(claimed.unwrap())),
                test_alloc::Counts::default());
            assert_eq!(ack.state(), EditState::Applied);
            assert_eq!(rt.sampler_banks.len(), 4);
            assert_ne!(engine.undo.checkpoint(), checkpoint);
            assert!(!ack.cancel());
        }
    }
}
