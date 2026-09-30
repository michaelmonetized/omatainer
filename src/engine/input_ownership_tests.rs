use super::*;
use dsp::VoiceOwner;

fn engine() -> RtEngine {
    let (_tx, rx) = crossbeam_channel::bounded(16);
    let mut rt = RtEngine::new(48_000.0, rx, Arc::new(Mutex::new(Snapshot::default())));
    rt.selected_track = 1;
    rt
}

fn midi(source: u64, ch: u8, note: u8) -> InputKey {
    InputKey::Midi { source, ch, note }
}

fn on(rt: &mut RtEngine, source: u64, ch: u8, note: u8) {
    rt.apply(Command::LiveNoteOn {
        source,
        ch,
        note,
        vel: 100,
    });
}

fn off(rt: &mut RtEngine, source: u64, ch: u8, note: u8) {
    rt.apply(Command::LiveNoteOff { source, ch, note });
}

fn held(poly: &Poly, input: InputKey) -> Vec<u8> {
    poly.voices
        .iter()
        .filter(|voice| voice.input == Some(input) && matches!(voice.env.stage, 1..=3))
        .map(|voice| voice.note)
        .collect()
}

#[test]
fn midi_releases_original_track_after_selection_and_scene_changes() {
    let mut rt = engine();
    for change in [
        Command::Select { track: 2, scene: 3 },
        Command::LaunchScene { scene: 0 },
        Command::RestartScene { scene: 0 },
    ] {
        rt.selected_track = 1;
        on(&mut rt, 11, 0, 60);
        rt.apply(change);
        rt.selected_track = 2;
        on(&mut rt, 22, 0, 60);
        off(&mut rt, 11, 0, 60);
        assert!(held(&rt.tracks[1].poly, midi(11, 0, 60)).is_empty());
        assert_eq!(held(&rt.tracks[2].poly, midi(22, 0, 60)), [60]);
        assert!(rt.tracks[1]
            .poly
            .voices
            .iter()
            .filter(|v| v.input == Some(midi(11, 0, 60)))
            .all(|v| matches!(v.env.stage, 0 | 4)));
        off(&mut rt, 22, 0, 60);
    }
    let mut output = [0.0; 512];
    for _ in 0..375 {
        rt.process(&mut output);
    }
    assert!(
        rt.tracks
            .iter()
            .flat_map(|track| &track.poly.voices)
            .filter(|voice| voice.input.is_some())
            .all(|voice| !voice.env.active()),
        "released input voices stayed sustained after two seconds"
    );
}

#[test]
fn equal_pitch_devices_channels_and_clip_are_independent_owners() {
    let mut rt = engine();
    let keys = [(7, 0), (7, 1), (u64::MAX, 0)];
    for (source, ch) in keys {
        on(&mut rt, source, ch, 60);
    }
    rt.tracks[1].poly.note_on_clip(60, 0.8);
    for (source, ch) in keys {
        assert_eq!(held(&rt.tracks[1].poly, midi(source, ch, 60)), [60]);
    }
    off(&mut rt, 999, 0, 60);
    off(&mut rt, 7, 2, 60);
    for (source, ch) in keys {
        assert_eq!(held(&rt.tracks[1].poly, midi(source, ch, 60)), [60]);
    }
    rt.apply(Command::StopTrack { track: 1 });
    assert!(rt.tracks[1]
        .poly
        .voices
        .iter()
        .filter(|v| v.owner == VoiceOwner::Clip)
        .all(|v| matches!(v.env.stage, 0 | 4)));
    for (index, (source, ch)) in keys.into_iter().enumerate() {
        off(&mut rt, source, ch, 60);
        for (other, other_ch) in &keys[index + 1..] {
            assert_eq!(held(&rt.tracks[1].poly, midi(*other, *other_ch, 60)), [60]);
        }
    }
}

#[test]
fn retrigger_moves_one_gate_and_old_stolen_gate_cannot_release_new_owner() {
    let mut rt = engine();
    on(&mut rt, 1, 0, 60);
    rt.apply(Command::Select { track: 2, scene: 3 });
    on(&mut rt, 1, 0, 60);
    assert!(held(&rt.tracks[1].poly, midi(1, 0, 60)).is_empty());
    assert_eq!(held(&rt.tracks[2].poly, midi(1, 0, 60)), [60]);
    rt.tracks[2].poly = Poly::new(rt.sr, 1, 1);
    on(&mut rt, 1, 0, 60);
    on(&mut rt, 2, 0, 60);
    off(&mut rt, 1, 0, 60);
    assert_eq!(held(&rt.tracks[2].poly, midi(2, 0, 60)), [60]);
    off(&mut rt, 2, 0, 60);
    assert!(held(&rt.tracks[2].poly, midi(2, 0, 60)).is_empty());

    let mut poly = Poly::new(rt.sr, 1, 2);
    poly.note_on_input(60, 1.0, midi(1, 0, 60));
    poly.note_on_input(60, 1.0, midi(2, 0, 60));
    poly.voices[0].env.stage = 0;
    poly.note_on_input(60, 1.0, midi(2, 0, 60));
    assert_eq!(
        held(&poly, midi(2, 0, 60)),
        [60],
        "retrigger duplicated a live gate into a free earlier slot"
    );
}

#[test]
fn held_pads_keep_original_destination_pitch_and_release_after_control_changes() {
    let mut rt = engine();
    rt.apply(Command::SamplerInst(1));
    rt.apply(Command::SamplerPad { pad: 0, on: true });
    let original = rt.pad_targets[0].unwrap().pitch;
    on(&mut rt, 9, 0, original);
    rt.tracks[1].poly.note_on_clip(original, 0.8);
    rt.apply(Command::Select { track: 2, scene: 4 });
    rt.apply(Command::SamplerOct(1));
    assert_eq!(rt.pad_targets[0].unwrap().track, 1);
    assert_eq!(rt.pad_targets[0].unwrap().pitch, original + 12);
    assert!(held(&rt.tracks[1].poly, InputKey::Pad(0)).is_empty());
    assert_eq!(rt.pad_destinations[0], 1);
    assert_eq!(held(&rt.sampler_poly, InputKey::Pad(0)), [original + 12]);
    assert_eq!(held(&rt.tracks[1].poly, midi(9, 0, original)), [original]);
    assert!(rt.tracks[1]
        .poly
        .voices
        .iter()
        .any(|v| v.owner == VoiceOwner::Clip && v.note == original));
    // Instrument changes reset the source pool as before; an old release must
    // not target this newly selected track or a different physical pad.
    rt.apply(Command::SamplerInst(2));
    rt.apply(Command::SamplerPad { pad: 1, on: true });
    let other = rt.pad_targets[1].unwrap().pitch;
    rt.apply(Command::SamplerPad { pad: 0, on: false });
    assert!(held(&rt.tracks[1].poly, InputKey::Pad(0)).is_empty());
    assert!(held(&rt.sampler_poly, InputKey::Pad(0)).is_empty());
    assert!(held(&rt.tracks[2].poly, InputKey::Pad(1)).is_empty());
    assert_eq!(rt.pad_destinations[1], 2);
    assert_eq!(held(&rt.sampler_poly, InputKey::Pad(1)), [other]);
    assert_eq!(held(&rt.tracks[1].poly, midi(9, 0, original)), [original]);
    rt.apply(Command::SamplerPad { pad: 1, on: false });
    assert!(held(&rt.tracks[2].poly, InputKey::Pad(1)).is_empty());
    assert!(held(&rt.sampler_poly, InputKey::Pad(1)).is_empty());
}

#[test]
fn equal_pitch_pad_gates_are_independent_and_one_shots_keep_their_tails() {
    let mut rt = engine();
    rt.apply(Command::SamplerInst(1));
    // These two physical pad identities map to the same natural note.
    assert_eq!(sampler_pitch(1, 3, 1), sampler_pitch(1, 3, 9));
    rt.apply(Command::SamplerPad { pad: 1, on: true });
    rt.apply(Command::SamplerPad { pad: 9, on: true });
    let pitch = rt.pad_targets[9].unwrap().pitch;
    rt.apply(Command::SamplerPad { pad: 1, on: false });
    assert!(held(&rt.tracks[1].poly, InputKey::Pad(9)).is_empty());
    assert_eq!(rt.pad_destinations[9], 1);
    assert_eq!(held(&rt.sampler_poly, InputKey::Pad(9)), [pitch]);
    rt.apply(Command::SamplerInst(-1));
    rt.apply(Command::Select { track: 3, scene: 5 });
    rt.apply(Command::SamplerOct(-1));
    rt.apply(Command::SamplerPad { pad: 9, on: false });
    assert!(held(&rt.tracks[1].poly, InputKey::Pad(9)).is_empty());
    rt.apply(Command::SamplerPad { pad: 9, on: true });
    rt.apply(Command::SamplerPad { pad: 9, on: false });
    assert!(
        rt.pad_voices[9].is_some(),
        "sample one-shots should ring after release"
    );
}

#[test]
fn direct_zero_velocity_on_releases_only_its_original_input() {
    let mut rt = engine();
    on(&mut rt, 1, 2, 60);
    on(&mut rt, 2, 2, 60);
    rt.selected_track = 3;
    rt.apply(Command::LiveNoteOn {
        source: 1,
        ch: 2,
        note: 60,
        vel: 0,
    });
    assert!(held(&rt.tracks[1].poly, midi(1, 2, 60)).is_empty());
    assert_eq!(held(&rt.tracks[1].poly, midi(2, 2, 60)), [60]);
}

#[test]
fn ownership_on_release_and_pad_octave_changes_allocate_nothing() {
    let mut rt = engine();
    rt.apply(Command::SamplerInst(1));
    let counts = test_alloc::measure(|| {
        for source in 1..=4 {
            on(&mut rt, source, 0, 60);
        }
        rt.apply(Command::SamplerPad { pad: 0, on: true });
        rt.selected_track = 2;
        rt.apply(Command::SamplerOct(1));
        for source in 1..=4 {
            off(&mut rt, source, 0, 60);
        }
        rt.apply(Command::SamplerPad { pad: 0, on: false });
    });
    assert_eq!(counts, test_alloc::Counts::default());
}
