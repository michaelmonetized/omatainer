use super::*;
use crate::sampler_bank::{Controls, PlayMode, Playback, resident};

fn install(rt: &mut RtEngine, playback: Playback, cue: f64) -> Arc<Sample> {
    let sample = Arc::new(Sample { spectrum: None,
        name: "Cue ramp".into(),
        sr: 16_000,
        ch: 1,
        data: (0..1024).map(|frame| frame as f32 / 1024.0).collect(),
        peaks: vec![[0.1; 3]; 4].into(),
        bpm: 0.0,
        path: String::new(),
    });
    let mut settings = resident::Settings::empty("Mode fixture".into()).unwrap();
    for slot in 0..2 {
        settings.slots[slot].controls = Controls {
            gain: 0.5,
            start_seconds: 4.0 / 16_000.0,
            end_seconds: Some(32.0 / 16_000.0),
        };
        settings.slots[slot].playback = Playback {
            cue_seconds: Some(cue / 16_000.0),
            ..playback
        };
    }
    let mut audio = std::array::from_fn(|_| None);
    audio[0] = Some(sample.clone());
    audio[1] = Some(sample.clone());
    let data =
        resident::Data::prepare(Arc::new(settings), audio, std::array::from_fn(|_| None)).unwrap();
    let bank = Bank::imported(rt.sampler_assets.pin(data).unwrap()).unwrap();
    rt.apply(Command::SamplerEdit(Edit {
        epoch: rt.undo.checkpoint().epoch,
        target: Target::Append {
            revision: rt.sampler_revision,
        },
        bank,
        select: true,
        ack: Ack::new(),
    }));
    rt.sampler_inst = SamplerInstrument::Samples;
    sample
}

#[test]
fn trigger_release_retrigger_and_cue_preserve_the_captured_source_until_next_press() {
    let (_, mut rt) = Engine::headless_for_test(48_000, 256);
    let source = install(&mut rt, Playback::default(), 8.0);
    rt.apply(Command::SamplerPad { pad: 0, on: true });
    assert_eq!(rt.pad_voices[0].as_ref().unwrap().position, 8.0);
    rt.tick_pad_sources();
    rt.apply(Command::SamplerPad { pad: 0, on: false });
    assert!(rt.pad_voices[0].is_some());
    let second = install(&mut rt, Playback::default(), 12.0);
    assert!(Arc::ptr_eq(
        &rt.pad_voices[0].as_ref().unwrap().audio,
        &source
    ));
    rt.apply(Command::SamplerPad { pad: 0, on: true });
    let voice = rt.pad_voices[0].as_ref().unwrap();
    assert_eq!(voice.position, 12.0);
    assert!(Arc::ptr_eq(&voice.audio, &second));
    rt.apply(Command::SamplerPad { pad: 0, on: false });
    for _ in 0..100 {
        rt.tick_pad_sources();
    }
    assert!(rt.pad_voices[0].is_none());
}

#[test]
fn long_held_repeat_releases_its_original_voice_after_bank_and_instrument_changes_without_heap_work()
 {
    let (_, mut rt) = Engine::headless_for_test(48_000, 256);
    let source = install(
        &mut rt,
        Playback {
            mode: PlayMode::Hold,
            repeat: true,
            cue_seconds: None,
        },
        8.0,
    );
    rt.apply(Command::SamplerPad { pad: 0, on: true });
    install(&mut rt, Playback::default(), 12.0);
    rt.apply(Command::SamplerInst(SamplerInstrument::Synth(
        SynthInstrument::Analog,
    )));
    let counts = test_alloc::measure(|| {
        for _ in 0..20_000 {
            rt.tick_pad_sources();
        }
        let voice = rt.pad_voices[0].as_ref().unwrap();
        assert!(Arc::ptr_eq(&voice.audio, &source));
        assert!(voice.position >= 8.0 && voice.position <= 32.0 + 1.0 / 3.0);
        rt.apply(Command::SamplerPad { pad: 0, on: false });
    });
    assert_eq!((counts.allocations, counts.frees), (0, 0));
    assert!(rt.pad_voices[0].is_none());
}

#[test]
fn toggle_repeat_stops_on_next_press_even_when_the_selected_bank_has_trigger_mode() {
    let (_, mut rt) = Engine::headless_for_test(48_000, 256);
    install(
        &mut rt,
        Playback {
            mode: PlayMode::Toggle,
            repeat: true,
            cue_seconds: None,
        },
        8.0,
    );
    rt.apply(Command::SamplerPad { pad: 0, on: true });
    rt.apply(Command::SamplerPad { pad: 0, on: false });
    for _ in 0..1000 {
        rt.tick_pad_sources();
    }
    assert!(rt.pad_voices[0].is_some());
    install(&mut rt, Playback::default(), 12.0);
    rt.apply(Command::SamplerPad { pad: 0, on: true });
    assert!(rt.pad_voices[0].is_none());
    rt.apply(Command::SamplerPad { pad: 0, on: false });
    rt.apply(Command::SamplerPad { pad: 0, on: true });
    assert_eq!(
        rt.pad_voices[0].as_ref().unwrap().playback.mode,
        PlayMode::Trigger
    );
}

#[test]
fn repeat_retains_fractional_overshoot_inside_the_cue_range_and_stop_is_exact() {
    let (_, mut rt) = Engine::headless_for_test(48_000, 256);
    install(
        &mut rt,
        Playback {
            mode: PlayMode::Trigger,
            repeat: true,
            cue_seconds: None,
        },
        8.0,
    );
    for pad in 0..2 {
        rt.apply(Command::SamplerPad { pad, on: true });
    }
    let voice = rt.pad_voices[0].as_mut().unwrap();
    voice.position = 32.25;
    voice.rate = 1.0;
    let frame = voice.tick(48_000.0).unwrap();
    assert!((frame.0 - 8.25 / 1024.0 * 0.5).abs() < 1e-7);
    assert!((voice.position - (8.25 + 1.0 / 3.0)).abs() < 1e-7);
    let playing = rt.playing;
    let deck = rt.decks[0].playing;
    let counts = test_alloc::measure(|| rt.apply(Command::SamplerSlotStop { pad: 0 }));
    assert_eq!((counts.allocations, counts.frees), (0, 0));
    assert!(rt.pad_voices[0].is_none());
    assert!(rt.pad_voices[1].is_some());
    assert_eq!((rt.playing, rt.decks[0].playing), (playing, deck));
    rt.apply(Command::SamplerSlotStop { pad: 16 });
    assert!(rt.pad_voices[1].is_some());
}

#[test]
fn slot_stop_fences_queued_onsets_until_renderer_acknowledges_and_keeps_other_slots_admissible() {
    let (engine, mut rt) = Engine::headless_for_test(48_000, 256);
    install(
        &mut rt,
        Playback {
            mode: PlayMode::Hold,
            repeat: true,
            cue_seconds: None,
        },
        8.0,
    );
    engine
        .send(Command::SamplerPad { pad: 0, on: true })
        .unwrap();
    engine.send(Command::SamplerSlotStop { pad: 0 }).unwrap();
    assert_eq!(
        engine.send(Command::SamplerPadPressure {
            pad: 0,
            pressure: 0.7
        }),
        Err(SubmissionError::StopPending)
    );
    engine
        .send(Command::SamplerPad { pad: 1, on: true })
        .unwrap();
    rt.process(&mut [0.0; 2]);
    assert!(rt.pad_voices[0].is_none());
    assert!(rt.pad_voices[1].is_some());
    engine
        .send(Command::SamplerPad { pad: 0, on: false })
        .unwrap();
    engine
        .send(Command::SamplerPad { pad: 0, on: true })
        .unwrap();
    rt.process(&mut [0.0; 2]);
    assert!(rt.pad_voices[0].is_some());
    assert_eq!(
        engine.send(Command::SamplerSlotStop { pad: 16 }),
        Err(SubmissionError::InvalidTarget)
    );
}

#[test]
fn learned_controller_slot_stop_survives_saturated_admission_and_performance_protection() {
    use crate::engine::midi::{Action, Binding, MidiMap, MsgKind, UnmappedNotes, learn};
    let (engine, mut rt) = Engine::headless_for_test(48_000, 256);
    install(
        &mut rt,
        Playback {
            mode: PlayMode::Toggle,
            repeat: true,
            cue_seconds: None,
        },
        8.0,
    );
    for pad in 0..2 {
        rt.apply(Command::SamplerPad { pad, on: true });
    }
    rt.apply(Command::PerformanceMode(true));
    let mut input = engine.midi.open_for_test(
        &engine.cmd,
        6001,
        MidiMap {
            name: "Slot stop controller".into(),
            matchers: vec![],
            bindings: vec![],
            unmapped_notes: UnmappedNotes::Ignore,
        },
        "Fixture USB pads",
        "fixture:slot-stop",
    );
    engine
        .cmd
        .midi_learn()
        .configure(learn::Config {
            mappings: vec![learn::Mapping {
                endpoint: learn::Endpoint {
                    name: "Fixture USB pads".into(),
                    id: "fixture:slot-stop".into(),
                },
                binding: Binding {
                    kind: MsgKind::Note,
                    ch: 1,
                    data: 62,
                    action: Action::SamplerSlotStop,
                    deck: 0,
                    extra: 0,
                    relative: None,
                },
            }],
        })
        .unwrap();
    while engine.send(Command::Master(0.8)).is_ok() {}
    input.push(&[0x91, 62, 100]);
    for _ in 0..8 {
        rt.process(&mut [0.0; 2]);
    }
    assert!(rt.pad_voices[0].is_none());
    assert!(rt.pad_voices[1].is_some());
    input.push(&[0x81, 62, 0]);
    let commands = rt.command_stats.received;
    rt.process(&mut [0.0; 2]);
    assert_eq!(rt.command_stats.received, commands);
    assert!(rt.pad_voices[1].is_some());
}
