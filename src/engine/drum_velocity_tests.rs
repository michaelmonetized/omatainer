use super::*;

fn engine() -> RtEngine {
    let (_tx, rx) = crossbeam_channel::bounded(32);
    let mut rt = RtEngine::new(48_000.0, rx, Arc::new(Mutex::new(Snapshot::default())));
    rt.apply(Command::Stop);
    rt.selected_track = 0;
    rt.selected_scene = 0;
    rt.bpm = 120.0;
    rt.quant = 0.0;
    rt.tracks[0].gain = 1.0;
    rt.tracks[0].fx.slots.clear();
    rt.tracks[0].clips = (0..SCENES).map(|_| Clip::empty()).collect();
    rt
}

fn clip(notes: Vec<MidiNote>, gain: f32) -> Clip {
    Clip {
        audio_region: None, lanes: None,
        region: None,
        kind: ClipKind::Midi,
        name: "velocity".into(),
        bars: 1.0,
        notes,
        gain,
        audio: None,
    }
}
fn note(pitch: u8, start: f32, len: f32, vel: u8) -> MidiNote {
    MidiNote {
        channel:0,release_vel:64,source_timing:None, id: crate::engine::midi_edit::NoteId::new(), muted: false,
        pitch,
        start,
        len,
        vel,
    }
}
fn frame(rt: &mut RtEngine) -> f32 {
    rt.beat += rt.bpm as f64 / 60.0 / rt.sr as f64;
    rt.render_track(0, false).0
}

#[test]
fn live_and_clip_hits_have_monotonic_energy_and_exact_velocity_ratios_for_every_drum() {
    for from_clip in [false, true] {
        for pitch in [36, 38, 42, 39, 46, 45] {
            let mut energies = [0.0; 3];
            for (index, velocity) in [1, 64, 127].into_iter().enumerate() {
                let mut rt = engine();
                if from_clip {
                    rt.tracks[0].clips[0] = clip(vec![note(pitch, 0.0, 0.25, velocity)], 1.0);
                    rt.apply(Command::LaunchClip { track: 0, scene: 0 });
                } else {
                    rt.apply(Command::LiveNoteOn {
                        source: 11,
                        ch: 9,
                        note: pitch,
                        vel: velocity,
                    });
                }
                for _ in 0..4096 {
                    energies[index] += frame(&mut rt).abs() as f64;
                }
                assert!(rt.tracks[0]
                    .drum_pos
                    .iter()
                    .flatten()
                    .all(|voice| voice.velocity == velocity as f32 / 127.0));
            }
            assert!(energies[0] > 0.0 && energies[0] < energies[1] && energies[1] < energies[2]);
            assert!((energies[0] / energies[2] - 1.0 / 127.0).abs() < 1e-7);
            assert!((energies[1] / energies[2] - 64.0 / 127.0).abs() < 1e-7);
            eprintln!(
                "drum pitch={pitch} clip={from_clip} abs-energy velocity1/64/127={energies:?}"
            );
        }
    }
}

#[test]
fn overlapping_same_sample_hits_keep_independent_velocity_and_clip_gain() {
    let mut rt = engine();
    let sample = rt.tracks[0].drum_samples[0].clone();
    rt.apply(Command::LiveNoteOn {
        source: 1,
        ch: 9,
        note: 36,
        vel: 1,
    });
    for position in 0..128 {
        assert!((rt.tick_drums(0) - sample.at(position as f64).0 / 127.0).abs() < 1e-8);
    }
    rt.tracks[0].clips[0] = clip(vec![note(36, 0.0, 1.0, 64)], 0.5);
    rt.apply(Command::LaunchClip { track: 0, scene: 0 });
    let first = frame(&mut rt);
    assert!(
        (first - (sample.at(128.0).0 / 127.0 + sample.at(0.0).0 * 0.5 * (64.0 / 127.0))).abs()
            < 1e-7
    );
    let voices: Vec<_> = rt.tracks[0].drum_pos.iter().flatten().copied().collect();
    assert_eq!(voices.len(), 2);
    assert_eq!(
        (voices[0].velocity, voices[0].clip_gain),
        (1.0 / 127.0, 1.0)
    );
    assert_eq!(
        (voices[1].velocity, voices[1].clip_gain),
        (64.0 / 127.0, 0.5)
    );
    for offset in 1..2048 {
        let expected = sample.at(128.0 + offset as f64).0 / 127.0
            + sample.at(offset as f64).0 * 0.5 * (64.0 / 127.0);
        assert!((frame(&mut rt) - expected).abs() < 2e-7);
    }
}

#[test]
fn zero_velocity_never_creates_or_steals_a_hit_and_release_keeps_finite_tails() {
    let mut rt = engine();
    rt.apply(Command::LiveNoteOn {
        source: 1,
        ch: 9,
        note: 36,
        vel: 0,
    });
    assert!(rt.tracks[0].drum_pos.iter().all(Option::is_none));
    for i in 0..16 {
        rt.trig_drum(0, 36, (i + 1) as f32 / 127.0);
    }
    rt.tick_drums(0);
    let before = rt.tracks[0].drum_pos;
    rt.apply(Command::LiveNoteOn {
        source: 1,
        ch: 9,
        note: 36,
        vel: 0,
    });
    rt.apply(Command::LiveNoteOff {
        source: 1,
        ch: 9,
        note: 36,
    });
    for velocity in [0.0, -1.0, f32::NAN, f32::INFINITY] {
        rt.trig_drum(0, 36, velocity);
    }
    assert_eq!(rt.tracks[0].drum_pos, before);
    rt.tracks[0].clips[0] = clip(vec![note(36, 0.0, 1.0, 0)], 1.0);
    rt.apply(Command::LaunchClip { track: 0, scene: 0 });
    frame(&mut rt);
    assert!(rt.tracks[0]
        .drum_pos
        .iter()
        .flatten()
        .all(|voice| voice.position == 2.0));
    assert_eq!(rt.tracks[0].drum_pos[0].unwrap().velocity, 1.0 / 127.0);
}

#[test]
fn arp_drum_velocity_tracks_active_duplicates_edits_visibility_and_loop_rebuilds() {
    let mut rt = engine();
    rt.tracks[0]
        .fx
        .slots
        .push(fx::FxSlot::new(fx::FxId::Arp, rt.sr));
    rt.tracks[0].clips[0] = clip(
        vec![
            note(36, 0.0, 2.0, 32),
            note(36, 0.25, 0.5, 127),
            note(38, 0.0, 2.0, 64),
        ],
        1.0,
    );
    rt.apply(Command::LaunchClip { track: 0, scene: 0 });
    for (beat, expected) in [(0.0, 32), (0.25, 64), (0.5, 127), (0.75, 64), (1.0, 32)] {
        rt.tracks[0].drum_pos.fill(None);
        rt.beat = beat + 2.0 * midi_schedule::BEAT_EPSILON;
        rt.render_track(0, false);
        assert_eq!(
            rt.tracks[0].drum_pos[0].unwrap().velocity,
            expected as f32 / 127.0
        );
    }
    rt.apply(Command::SetNotes {
        track: 0,
        scene: 0,
        notes: vec![note(36, 0.0, 2.0, 100)],
    });
    rt.tracks[0].drum_pos.fill(None);
    rt.beat = 1.25 + 2.0 * midi_schedule::BEAT_EPSILON;
    rt.render_track(0, false);
    assert_eq!(rt.tracks[0].drum_pos[0].unwrap().velocity, 100.0 / 127.0);
    rt.tracks[0].drum_pos.fill(None);
    rt.beat = 4.0 + 2.0 * midi_schedule::BEAT_EPSILON;
    rt.render_track(0, false);
    assert_eq!(rt.tracks[0].drum_pos[0].unwrap().velocity, 100.0 / 127.0);
    let mut cache = arp::ChordCache::default();
    let notes = [note(36, 0.0, 1.0, 32), note(36, 0.0, 1.0, 127)];
    cache.refresh_visible(&notes, 0.0, -1.0, 4.0, |index| index == 0);
    assert_eq!(cache.velocity(36), 32);
    cache.invalidate();
    cache.refresh(&notes, 0.0, -1.0, 4.0);
    assert_eq!(cache.velocity(36), 127);
    let rebuilt = cache.rebuilds;
    cache.refresh(&notes, 0.2, 0.1, 4.0);
    assert_eq!(cache.rebuilds, rebuilt);
    cache.refresh(&notes, 1.0, 0.9, 4.0);
    assert_eq!(cache.velocity(36), 0);
}
