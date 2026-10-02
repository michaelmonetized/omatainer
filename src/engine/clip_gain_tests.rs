use super::*;
use dsp::VoiceOwner;

const TRACK: usize = 1;

fn engine(gain: f32, kind: u8, arp: bool, live: bool) -> RtEngine {
    let (_tx, rx) = crossbeam_channel::bounded(32);
    let mut rt = RtEngine::new(48_000.0, rx, Arc::new(Mutex::new(Snapshot::default())));
    rt.apply(Command::Stop);
    rt.bpm = 120.0;
    rt.quant = 0.0;
    rt.selected_track = TRACK;
    rt.selected_scene = 0;
    let track = &mut rt.tracks[TRACK];
    track.kind = kind;
    track.poly = Poly::new(rt.sr, match kind { 0 | 1 => SynthInstrument::Analog, 2 => SynthInstrument::Keys, _ => SynthInstrument::Pad }, 16);
    track.fx.slots.clear();
    if arp {
        track.fx.slots.push(fx::FxSlot::new(fx::FxId::Arp, rt.sr));
    }
    track.gain = 1.0;
    track.pan = 0.0;
    track.clips = (0..SCENES).map(|_| Clip::empty()).collect();
    track.clips[0] = Clip {
        lanes: None,
        region: None,
        kind: ClipKind::Midi,
        name: "gain probe".into(),
        bars: 1.0,
        notes: vec![MidiNote {
            channel:0,release_vel:64,source_timing:None, id: crate::engine::midi_edit::NoteId::new(), muted: false,
            pitch: if kind == 0 { 36 } else { 60 },
            start: 0.0,
            len: 2.0,
            vel: 100,
        }],
        gain,
        audio: None,
    };
    if live {
        rt.apply(Command::LiveNoteOn {
            source: 99,
            ch: 0,
            note: if kind == 0 { 38 } else { 67 },
            vel: 100,
        });
    }
    rt.apply(Command::LaunchClip {
        track: TRACK as u8,
        scene: 0,
    });
    rt
}

fn sample(rt: &mut RtEngine) -> f32 {
    rt.beat += rt.bpm as f64 / 60.0 / rt.sr as f64;
    let (left, right, _) = rt.render_track(TRACK, false);
    assert!((left - right).abs() < 1e-6);
    assert!(left.is_finite());
    left
}

#[test]
fn zero_half_unity_affect_only_clip_synth_arp_and_drum_sources() {
    for (kind, arp) in [
        (0, false),
        (0, true),
        (1, false),
        (2, false),
        (3, false),
        (2, true),
    ] {
        let mut zero = engine(0.0, kind, arp, true);
        let mut half = engine(0.5, kind, arp, true);
        let mut unity = engine(1.0, kind, arp, true);
        let mut live = engine(1.0, kind, arp, true);
        live.apply(Command::StopTrack { track: TRACK as u8 });
        let mut clip_energy = 0.0;
        let mut live_energy = 0.0;
        for _ in 0..12_000 {
            let z = sample(&mut zero);
            let h = sample(&mut half);
            let u = sample(&mut unity);
            let l = sample(&mut live);
            assert!(
                (z - l).abs() < 2e-6,
                "kind={kind} arp={arp}: zero clip changed live {z} != {l}"
            );
            assert!(
                (h - (l + (u - l) * 0.5)).abs() < 2e-6,
                "kind={kind} arp={arp}: half clip changed live {h}"
            );
            clip_energy += (u - l).abs();
            live_energy += l.abs();
        }
        assert!(clip_energy > 1.0 && live_energy > 1.0);
    }
}

#[test]
fn same_pitch_live_input_and_clip_gain_remain_independent() {
    let mut a = engine(0.0, 2, false, false);
    let mut b = engine(1.0, 2, false, false);
    let mut only_clip = engine(1.0, 2, false, false);
    for rt in [&mut a, &mut b] {
        rt.apply(Command::LiveNoteOn {
            source: 44,
            ch: 7,
            note: 60,
            vel: 100,
        });
    }
    for _ in 0..2048 {
        let live = sample(&mut a);
        let both = sample(&mut b);
        let clip = sample(&mut only_clip);
        assert!((both - live - clip).abs() < 2e-6);
    }
    let voices = &a.tracks[TRACK].poly.voices;
    assert!(voices
        .iter()
        .any(|v| v.owner == VoiceOwner::Clip && v.clip_gain == 0.0));
    assert!(voices
        .iter()
        .any(|v| v.input.is_some() && v.clip_gain == 1.0));
}

#[test]
fn edits_and_replacement_preserve_held_notes_release_tails_and_live_gain() {
    for (kind, same_pitch) in [0, 1, 2, 3]
        .into_iter()
        .flat_map(|kind| [(kind, false), (kind, true)])
    {
        let mut reference = engine(0.5, kind, false, true);
        let mut changed = engine(0.5, kind, false, true);
        for _ in 0..512 {
            assert_eq!(sample(&mut reference), sample(&mut changed));
        }
        changed.apply(Command::ClipGain {
            track: TRACK as u8,
            scene: 0,
            value: 0.0,
        });
        for _ in 0..512 {
            assert_eq!(sample(&mut reference), sample(&mut changed));
        }
        let old = changed.tracks[TRACK].clips[0].clone();
        changed.tracks[TRACK].clips[1] = Clip {
            lanes: None,
            region: None,
            gain: 0.0,
            notes: vec![MidiNote {
                channel:0,release_vel:64,source_timing:None, id: crate::engine::midi_edit::NoteId::new(), muted: false,
                pitch: if same_pitch {
                    old.notes[0].pitch
                } else if kind == 0 {
                    46
                } else {
                    72
                },
                ..old.notes[0]
            }],
            ..old
        };
        reference.apply(Command::StopTrack { track: TRACK as u8 });
        changed.apply(Command::LaunchClip {
            track: TRACK as u8,
            scene: 1,
        });
        let mut energy = 0.0;
        for _ in 0..4000 {
            let expected = sample(&mut reference);
            let actual = sample(&mut changed);
            assert!(
                (actual - expected).abs() < 2e-6,
                "kind={kind} old tail changed {actual} != {expected}"
            );
            energy += actual.abs();
        }
        assert!(energy > 1.0);
    }
}

#[test]
fn next_onset_uses_edited_gain_and_track_gain_still_scales_the_complete_track() {
    let mut rt = engine(1.0, 2, false, true);
    sample(&mut rt);
    rt.apply(Command::ClipGain {
        track: TRACK as u8,
        scene: 0,
        value: 0.5,
    });
    rt.apply(Command::LaunchClip {
        track: TRACK as u8,
        scene: 0,
    });
    sample(&mut rt);
    assert!(rt.tracks[TRACK]
        .poly
        .voices
        .iter()
        .any(|v| v.owner == VoiceOwner::Clip && v.env.active() && v.clip_gain == 0.5));
    let mut reference = engine(0.5, 2, false, true);
    let mut quiet = engine(0.5, 2, false, true);
    quiet.apply(Command::TrackGain {
        track: TRACK as u8,
        value: 0.25,
    });
    for _ in 0..1024 {
        assert!((sample(&mut quiet) - sample(&mut reference) * 0.25).abs() < 1e-6);
    }
}

#[test]
fn clip_gain_controls_snapshot_serialization_validation_and_coalescing_keep_target_identity() {
    let mut rt = engine(1.0, 2, false, false);
    for (value, expected) in [(0.0, 0.0), (0.5, 0.5), (1.0, 1.0), (-1.0, 0.0), (5.0, 1.5)] {
        rt.apply(Command::ClipGain {
            track: TRACK as u8,
            scene: 0,
            value,
        });
        rt.publish_for_test();
        assert_eq!(rt.snap.lock().tracks[TRACK].clips[0].gain, expected);
        let json = serde_json::to_value(&rt.tracks[TRACK].clips[0]).unwrap();
        assert_eq!(json["gain"], expected);
        assert_eq!(serde_json::from_value::<Clip>(json).unwrap().gain, expected);
    }
    for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        rt.apply(Command::ClipGain {
            track: TRACK as u8,
            scene: 0,
            value,
        });
        assert_eq!(rt.tracks[TRACK].clips[0].gain, 1.5);
    }
    rt.apply(Command::ClipGain {
        track: TRACK as u8,
        scene: 255,
        value: 0.0,
    });
    rt.apply(Command::ClipGain {
        track: 255,
        scene: 0,
        value: 0.0,
    });
    assert_eq!(rt.tracks[TRACK].clips[0].gain, 1.5);
    let (port, receiver) = CommandPort::channel(32);
    let mut rt = RtEngine::new(
        48_000.0,
        receiver,
        Arc::new(Mutex::new(Snapshot::default())),
    );
    for (scene, value) in [(0, 0.1), (0, 0.5), (1, 0.25), (0, 1.0)] {
        port.send(Command::ClipGain {
            track: 1,
            scene,
            value,
        })
        .unwrap();
    }
    rt.process(&mut [0.0; 2]);
    assert_eq!(rt.tracks[1].clips[0].gain, 1.0);
    assert_eq!(rt.tracks[1].clips[1].gain, 0.25);
    assert_eq!(rt.command_stats.received_last_block, 4);
    assert_eq!(rt.command_stats.applied_last_block, 3);
}
