use super::*;
use dsp::VoiceOwner;

fn engine() -> RtEngine {
    let (_tx, rx) = crossbeam_channel::bounded(8);
    RtEngine::new(48_000.0, rx, Arc::new(Mutex::new(Snapshot::default())))
}

fn render(rt: &mut RtEngine, seconds: f32) {
    let mut audio = vec![0.0; (rt.sr * seconds) as usize * 2];
    rt.process(&mut audio);
    assert!(audio.iter().all(|x| x.is_finite()));
}

fn chord() -> Clip {
    Clip {
        kind: ClipKind::Midi,
        name: "Sustained chord".into(),
        bars: 16.0,
        notes: [60, 64, 67]
            .map(|pitch| MidiNote {
                pitch,
                start: 0.0,
                len: 32.0,
                vel: 100,
            })
            .into(),
        gain: 1.0,
        audio: None,
    }
}

fn prepare(rt: &mut RtEngine, arp: bool) {
    rt.playing = false;
    rt.beat = 0.0;
    rt.quant = 0.0;
    rt.bpm = 120.0;
    for track in &mut rt.tracks {
        track.stop_clip();
        track.clips = std::array::from_fn(|_| Clip::empty());
        track.poly = Poly::new(rt.sr, 2, 16);
        track.fx.slots.clear();
    }
    rt.tracks[1].clips[0] = chord();
    rt.tracks[1].clips[1] = Clip {
        notes: Vec::new(),
        ..chord()
    };
    rt.tracks[2].clips[2] = chord();
    if arp {
        rt.tracks[1].fx.slots.push(fx::FxSlot::new(fx::FxId::Arp, rt.sr));
    }
    rt.apply(Command::LaunchClip { track: 2, scene: 2 });
    rt.apply(Command::LaunchClip { track: 1, scene: 0 });
    for note in [60, 64, 67] {
        rt.apply(Command::LiveNoteOn {
            note,
            vel: 100,
            ch: 0,
        });
    }
    render(rt, 0.6);
    assert!(rt.tracks[1]
        .poly
        .voices
        .iter()
        .any(|v| { v.owner == VoiceOwner::Clip && matches!(v.env.stage, 1..=3) }));
    assert_eq!(live_held(&rt.tracks[1]), 3);
    assert_eq!(rt.tracks[1].arp_note.is_some(), arp);
}

fn live_held(track: &TrackRt) -> usize {
    track
        .poly
        .voices
        .iter()
        .filter(|v| v.owner == VoiceOwner::Live && matches!(v.env.stage, 1..=3))
        .count()
}

fn assert_clip_releasing(track: &TrackRt) {
    let voices: Vec<_> = track
        .poly
        .voices
        .iter()
        .filter(|v| v.owner == VoiceOwner::Clip && v.env.active())
        .collect();
    assert!(!voices.is_empty(), "the envelope tail must not be cut off");
    assert!(voices.iter().all(|v| v.env.stage == 4));
    assert!(voices.iter().any(|v| v.env.level > 0.0));
    assert_eq!(track.arp_note, None);
}

#[test]
fn clip_stop_and_replacement_paths_release_only_owned_notes_and_arp() {
    let mut rt = engine();
    let paths = [
        (Command::StopTrack { track: 1 }, true),
        (Command::ToggleScene { scene: 0 }, true),
        (Command::LaunchClip { track: 1, scene: 7 }, true),
        (Command::LaunchScene { scene: 7 }, false),
        (Command::LaunchClip { track: 1, scene: 1 }, true),
        (
            Command::FireClip {
                track: 1,
                scene: 1,
                looping: false,
            },
            true,
        ),
        (Command::RestartScene { scene: 1 }, false),
        (Command::AddScene { scene: 1 }, true),
        (Command::LaunchScene { scene: 1 }, false),
        (Command::Stop, false),
        (Command::TogglePlay, false),
        (
            Command::SetNotes {
                track: 1,
                scene: 0,
                notes: Vec::new(),
            },
            true,
        ),
    ];
    for arp in [false, true] {
        for (command, other_unaffected) in &paths {
            prepare(&mut rt, arp);
            let other = rt.tracks[2].poly.voices.clone();
            rt.apply(command.clone());
            assert_clip_releasing(&rt.tracks[1]);
            assert_eq!(live_held(&rt.tracks[1]), 3, "{command:?}, arp={arp}");
            if *other_unaffected {
                for (before, after) in other.iter().zip(&rt.tracks[2].poly.voices) {
                    assert_eq!(before.env.stage, after.env.stage, "{command:?}");
                    assert_eq!(before.env.level, after.env.level, "{command:?}");
                }
                assert_eq!(rt.tracks[2].playing.unwrap().scene, 2);
            }
            // The longest synth release is 0.8 seconds. Live notes remain held.
            render(&mut rt, 1.0);
            assert!(
                rt.tracks[1]
                    .poly
                    .voices
                    .iter()
                    .all(|v| { v.owner != VoiceOwner::Clip || !v.env.active() }),
                "{command:?}, arp={arp}"
            );
            assert_eq!(live_held(&rt.tracks[1]), 3, "{command:?}, arp={arp}");
        }
    }
}

#[test]
fn one_shot_clip_completion_releases_long_notes_and_arp() {
    let mut rt = engine();
    for arp in [false, true] {
        prepare(&mut rt, arp);
        rt.tracks[1].clips[0].bars = 0.5;
        rt.tracks[1].playing.as_mut().unwrap().looping = false;
        render(&mut rt, 0.5);
        assert!(rt.tracks[1].playing.is_none());
        assert_clip_releasing(&rt.tracks[1]);
        render(&mut rt, 1.0);
        assert!(rt.tracks[1]
            .poly
            .voices
            .iter()
            .all(|v| { v.owner != VoiceOwner::Clip || !v.env.active() }));
        assert_eq!(live_held(&rt.tracks[1]), 3);
        assert!(rt.tracks[2]
            .poly
            .voices
            .iter()
            .any(|v| { v.owner == VoiceOwner::Clip && v.env.stage == 3 }));
    }
}

#[test]
fn same_pitch_live_and_clip_note_offs_are_independent_in_either_order() {
    for live_first in [false, true] {
        let mut poly = Poly::new(48_000.0, 2, 8);
        if live_first {
            poly.note_on(60, 0.8);
            poly.note_on_clip(60, 0.8);
        } else {
            poly.note_on_clip(60, 0.8);
            poly.note_on(60, 0.8);
        }
        assert_eq!(poly.voices.iter().filter(|v| v.env.active()).count(), 2);
        poly.note_off_clip(60);
        assert!(poly
            .voices
            .iter()
            .any(|v| v.owner == VoiceOwner::Live && v.env.stage == 1));
        poly.note_on_clip(60, 0.8);
        poly.note_off(60);
        assert!(poly
            .voices
            .iter()
            .any(|v| v.owner == VoiceOwner::Clip && v.env.stage == 1));
        poly.release_clip();
        assert!(poly.voices.iter().all(|v| matches!(v.env.stage, 0 | 4)));
    }
}

#[test]
fn stolen_voices_get_the_new_owners_note_off_policy() {
    let mut poly = Poly::new(48_000.0, 2, 1);
    poly.note_on_clip(60, 0.8);
    poly.note_on(64, 0.8);
    poly.release_clip();
    assert_eq!(poly.voices[0].env.stage, 1);
    poly.note_on_clip(67, 0.8);
    poly.note_off(67);
    assert_eq!(poly.voices[0].env.stage, 1);
    poly.release_clip();
    assert_eq!(poly.voices[0].env.stage, 4);
}

#[test]
fn stopping_a_drum_clip_keeps_finite_one_shot_tails() {
    let mut rt = engine();
    rt.quant = 0.0;
    rt.tracks[0].clips[0] = chord();
    rt.tracks[0].clips[0].notes.truncate(1);
    rt.tracks[0].clips[0].notes[0].pitch = 46;
    rt.apply(Command::LaunchClip { track: 0, scene: 0 });
    render(&mut rt, 0.01);
    assert!(rt.tracks[0].drum_pos.iter().any(Option::is_some));
    rt.apply(Command::StopTrack { track: 0 });
    assert!(rt.tracks[0].drum_pos.iter().any(Option::is_some));
    render(&mut rt, 2.0);
    assert!(rt.tracks[0].drum_pos.iter().all(Option::is_none));
}
