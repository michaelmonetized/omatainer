use super::*;
use dsp::VoiceOwner;

const DEST: usize = 1;
const SR: f64 = 48_000.0;
const EPS: f64 = 1e-9;

fn engine() -> RtEngine {
    let (_tx, rx) = crossbeam_channel::bounded(8);
    let mut rt = RtEngine::new(SR as f32, rx, Arc::new(Mutex::new(Snapshot::default())));
    rt.apply(Command::Stop);
    for deck in 0..DECKS {
        rt.apply(Command::DeckUnload { deck: deck as u8 });
    }
    for track in &mut rt.tracks {
        track.fx.slots.clear();
        track.gain = 1.0;
        track.pan = 0.0;
        track.clips = std::array::from_fn(|_| Clip::empty());
    }
    for chain in &mut rt.scene_fx { chain.slots.clear(); }
    rt.fx_wet = [0.0; 3];
    rt.master = 1.0;
    rt.bpm = 120.0;
    rt.quant = 0.0;
    rt.selected_track = DEST;
    rt.selected_scene = 0;
    rt
}

fn sample(rt: &mut RtEngine) -> [f32; 2] {
    let mut output = [f32::NAN; 2];
    rt.process(&mut output);
    output
}

fn frames(rt: &mut RtEngine, count: usize) -> f32 {
    let mut peak = 0.0f32;
    for _ in 0..count {
        for x in sample(rt) {
            assert!(x.is_finite());
            peak = peak.max(x.abs());
        }
    }
    peak
}

fn pad(rt: &mut RtEngine, on: bool) {
    rt.apply(Command::SamplerPad { pad: 0, on });
}

#[test]
fn instrument_pad_has_one_source_and_matches_the_selected_instrument_reference() {
    for instrument in SynthInstrument::ALL.map(SamplerInstrument::Synth) {
        for destination in [0, 1, 4, 7] {
            let mut rt = engine();
            rt.selected_track = destination;
            rt.apply(Command::SamplerInst(instrument));
            pad(&mut rt, true);
            assert_eq!(
                rt.sampler_poly
                    .voices
                    .iter()
                    .filter(|v| v.env.active())
                    .count(),
                1
            );
            assert!(
                rt.tracks
                    .iter()
                    .flat_map(|t| &t.poly.voices)
                    .all(|v| v.input != Some(InputKey::Pad(0)))
            );
            let mut reference = rt.sampler_poly.clone();
            let mut audible = false;
            for _ in 0..256 {
                let expected = reference.tick(rt.sr).tanh();
                let actual = sample(&mut rt);
                for value in actual {
                    assert!(
                        (value - expected).abs() < 1e-6,
                        "inst={instrument:?} destination={destination}: {value} != {expected}"
                    );
                }
                audible |= expected.abs() > 1e-4;
            }
            assert!(audible);
        }
    }
}

#[test]
fn both_sample_and_instrument_pads_obey_destination_mute_gain_pan_solo_and_fx() {
    for instrument in [SamplerInstrument::Samples, SamplerInstrument::Synth(SynthInstrument::Keys)] {
        for control in 0..5 {
            let mut rt = engine();
            rt.apply(Command::SamplerInst(instrument));
            match control {
                0 => rt.tracks[DEST].mute = true,
                1 => rt.tracks[DEST].gain = 0.0,
                2 => rt.tracks[DEST].pan = -1.0,
                3 => rt.tracks[DEST + 1].solo = true,
                _ => {
                    let mut effect = fx::FxSlot::new(fx::FxId::Balance, rt.sr);
                    effect.mix = 1.0;
                    effect.p[0] = 0.0;
                    rt.tracks[DEST].fx.slots.push(effect);
                }
            }
            pad(&mut rt, true);
            let mut peak = 0.0f32;
            for _ in 0..1024 {
                let [l, r] = sample(&mut rt);
                assert_eq!(r, 0.0, "control={control} inst={instrument:?}");
                if matches!(control, 0 | 1 | 3) {
                    assert_eq!(l, 0.0);
                } else {
                    peak = peak.max(l.abs());
                }
            }
            if matches!(control, 2 | 4) {
                assert!(peak > 0.01);
            }
            pad(&mut rt, false);
            frames(&mut rt, 48_000);
            assert!(rt.sampler_poly.voices.iter().all(|v| !v.env.active()));
            assert!(rt.pad_voices[0].is_none());
        }
    }
}

#[test]
fn stereo_sample_and_release_tails_keep_the_original_destination() {
    let mut rt = engine();
    rt.set_test_pad_sample(0, 0, Arc::new(Sample {
        name: "right only".into(),
        sr: SR as u32,
        ch: 2,
        data: (0..4096).flat_map(|_| [0.0, 0.25]).collect(),
        peaks: vec![].into(),
        bpm: 120.0,
        path: String::new(),
    }));
    pad(&mut rt, true);
    pad(&mut rt, false);
    rt.selected_track = DEST + 1;
    rt.tracks[DEST].gain = 0.4;
    for _ in 0..128 {
        let [l, r] = sample(&mut rt);
        assert_eq!(l, 0.0);
        assert!((r - 0.1f32.tanh()).abs() < 1e-6);
    }
    rt.tracks[DEST].mute = true;
    assert_eq!(frames(&mut rt, 4096), 0.0);
    assert!(rt.pad_voices[0].is_none());

    let mut rt = engine();
    rt.apply(Command::SamplerInst(SamplerInstrument::Synth(SynthInstrument::Keys)));
    pad(&mut rt, true);
    assert!(frames(&mut rt, 2048) > 0.01);
    rt.selected_track = DEST + 1;
    pad(&mut rt, false);
    assert_eq!(rt.pad_destinations[0], DEST);
    rt.tracks[DEST].gain = 0.0;
    // Mixer gain edits now fade for five milliseconds, while the release
    // remains routed to its captured destination. Exact silence follows.
    assert!(frames(&mut rt, 240) > 0.0);
    assert_eq!(frames(&mut rt, 23_760), 0.0);
    assert!(rt.sampler_poly.voices.iter().all(|v| !v.env.active()));
}

fn capture_fixture(arp: bool) -> RtEngine {
    let mut rt = engine();
    rt.apply(Command::SamplerInst(SamplerInstrument::Synth(SynthInstrument::Keys)));
    rt.tracks[DEST].clips[0] = Clip {
        region: None,
        kind: ClipKind::Midi,
        name: "capture".into(),
        bars: 0.25,
        notes: vec![],
        gain: 1.0,
        audio: None,
    };
    if arp {
        rt.tracks[DEST]
            .fx
            .slots
            .push(fx::FxSlot::new(fx::FxId::Arp, rt.sr));
    }
    rt.apply(Command::LaunchClip {
        track: DEST as u8,
        scene: 0,
    });
    rt.recording = true;
    frames(&mut rt, 6000); // One quarter beat into the playing one-beat clip.
    rt
}

fn input(rt: &mut RtEngine, is_pad: bool, on: bool) {
    if is_pad {
        pad(rt, on);
    } else if on {
        rt.apply(Command::LiveNoteOn {
            source: 41,
            ch: 0,
            note: 60,
            vel: 100,
        });
    } else {
        rt.apply(Command::LiveNoteOff {
            source: 41,
            ch: 0,
            note: 60,
        });
    }
}

#[test]
fn captured_midi_and_pads_monitor_once_then_join_future_loops_after_actual_release() {
    for arp in [false, true] {
        for is_pad in [false, true] {
            for record_off_while_held in [false, true] {
                let mut rt = capture_fixture(arp);
                input(&mut rt, is_pad, true);
                let initial = rt.tracks[DEST].poly.note_on_events;
                frames(&mut rt, 6000);
                if record_off_while_held {
                    rt.apply(Command::Record);
                }
                frames(&mut rt, 48_000); // Cross two full loops with input still down.
                assert_eq!(
                    rt.tracks[DEST].poly.note_on_events, initial,
                    "mirrored capture: arp={arp} pad={is_pad} record_off={record_off_while_held}"
                );
                input(&mut rt, is_pad, false);
                frames(&mut rt, 6000); // Reach beat 2.75, before first eligible beat 3.25.
                assert_eq!(rt.tracks[DEST].poly.note_on_events, initial);
                let increment = 1.0 / 24_000.0;
                while rt.beat + increment <= 3.25 + EPS {
                    sample(&mut rt);
                }
                assert_eq!(rt.tracks[DEST].poly.note_on_events, initial);
                sample(&mut rt);
                assert_eq!(
                    rt.tracks[DEST].poly.note_on_events,
                    initial + 1,
                    "recorded note did not join next eligible loop: arp={arp} pad={is_pad}"
                );
                assert_eq!(rt.tracks[DEST].clips[0].notes.len(), 1);
                let duration = if record_off_while_held { 0.25 } else { 2.25 };
                assert!((rt.tracks[DEST].clips[0].notes[0].len - duration).abs() < 1e-4);
            }
        }
    }
}

#[test]
fn capture_suppression_preserves_unrelated_same_pitch_events_and_resets_on_relaunch() {
    let mut rt = capture_fixture(false);
    rt.apply(Command::SetNotes {
        track: DEST as u8,
        scene: 0,
        notes: vec![MidiNote {
            id: crate::engine::midi_edit::NoteId::new(), muted: false,
            pitch: 60,
            start: 0.5,
            len: 0.125,
            vel: 77,
        }],
    });
    input(&mut rt, false, true);
    let initial = rt.tracks[DEST].poly.note_on_events;
    frames(&mut rt, 7000);
    assert_eq!(rt.tracks[DEST].poly.note_on_events, initial + 1);
    assert!(
        rt.tracks[DEST]
            .poly
            .voices
            .iter()
            .any(|v| v.owner == VoiceOwner::Clip && v.note() == 60)
    );
    input(&mut rt, false, false);
    rt.apply(Command::LaunchClip {
        track: DEST as u8,
        scene: 0,
    });
    assert!(rt.tracks[DEST].recorded_playback.is_empty());
    frames(&mut rt, 7000);
    assert_eq!(rt.tracks[DEST].poly.note_on_events, initial + 2);
}

#[test]
fn warmed_pad_routing_and_release_perform_no_heap_work() {
    let mut rt = engine();
    rt.apply(Command::SamplerInst(SamplerInstrument::Synth(SynthInstrument::Keys)));
    pad(&mut rt, true);
    rt.process(&mut [0.0; 128]);
    let counts = test_alloc::measure(|| {
        rt.selected_track = DEST + 1;
        rt.apply(Command::SamplerOct(1));
        pad(&mut rt, false);
        rt.process(&mut [0.0; 128]);
        pad(&mut rt, true);
        rt.process(&mut [0.0; 128]);
    });
    assert_eq!(counts, test_alloc::Counts::default());
}

#[test]
fn instantaneous_capture_cannot_replay_in_its_first_cycle_or_one_shot() {
    for looping in [false, true] {
        let mut rt = capture_fixture(false);
        rt.tracks[DEST].playing.as_mut().unwrap().looping = looping;
        input(&mut rt, true, true);
        input(&mut rt, true, false);
        let pitch = rt.tracks[DEST].clips[0].notes[0].pitch;
        assert!(rt.tracks[DEST].clips[0].notes[0].len > 0.0);
        rt.tracks[DEST].midi_schedule.trace = Some(Vec::new());
        // A subsequent ordinary append/rebuild must not chase that stored note.
        rt.tracks[DEST].clips[0].notes.push(MidiNote {
            id: crate::engine::midi_edit::NoteId::new(), muted: false,
            pitch: 92,
            start: 0.5,
            len: 0.125,
            vel: 77,
        });
        rt.tracks[DEST].clip_notes_changed(0, rt.beat);
        frames(&mut rt, 18_000);
        let captured_events = |rt: &RtEngine| {
            rt.tracks[DEST]
                .midi_schedule
                .trace
                .as_ref()
                .unwrap()
                .iter()
                .filter(|(_, gate)| matches!(gate, midi_schedule::Gate::On(p, _) if *p == pitch))
                .count()
        };
        assert_eq!(captured_events(&rt), 0);
        sample(&mut rt); // First output sample beyond the end of this clip.
        if looping {
            frames(&mut rt, 6000);
            assert_eq!(captured_events(&rt), 1);
        } else {
            assert!(rt.tracks[DEST].playing.is_none());
            assert_eq!(captured_events(&rt), 0);
            rt.apply(Command::LaunchClip {
                track: DEST as u8,
                scene: 0,
            });
            frames(&mut rt, 6001);
            assert_eq!(captured_events(&rt), 1);
        }
    }
}

#[test]
fn natural_one_shot_end_finalizes_capture_without_duplicating_monitor_voice() {
    for is_pad in [false, true] {
        let mut rt = capture_fixture(false);
        rt.tracks[DEST].playing.as_mut().unwrap().looping = false;
        input(&mut rt, is_pad, true);
        let initial = rt.tracks[DEST].poly.note_on_events;
        frames(&mut rt, 18_001);
        assert!(rt.tracks[DEST].playing.is_none());
        let finished = rt.tracks[DEST].clips[0].notes[0].len;
        assert!((finished - 0.75).abs() <= 1.0 / 24_000.0 + 1e-6);
        assert_eq!(rt.tracks[DEST].poly.note_on_events, initial);
        frames(&mut rt, 12_000);
        input(&mut rt, is_pad, false);
        assert_eq!(rt.tracks[DEST].clips[0].notes[0].len, finished);
        frames(&mut rt, 24_000);
        assert!(rt.sampler_poly.voices.iter().all(|v| !v.env.active()));
        assert!(rt.tracks[DEST].poly.voices.iter().all(|v| !v.env.active()));
    }
}
