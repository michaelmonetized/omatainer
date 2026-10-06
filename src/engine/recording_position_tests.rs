use super::*;

fn fixture(bars: f32) -> RtEngine {
    let (_tx, rx) = crossbeam_channel::bounded(16);
    let mut rt = RtEngine::new(48_000.0, rx, Arc::new(Mutex::new(Snapshot::default())));
    rt.selected_track = 1;
    rt.apply(Command::SamplerInst(SamplerInstrument::Synth(SynthInstrument::Keys)));
    rt.selected_scene = 2;
    rt.quant = 0.0;
    rt.bpm = 120.0;
    rt.beat = 5.375;
    rt.tracks[1].clips[2] = Clip {
        audio_region: None, lanes: None,
        region: None,
        kind: ClipKind::Midi,
        name: "Capture".into(),
        bars,
        notes: vec![],
        gain: 1.0,
        audio: None,
    };
    rt
}

fn capture(rt: &mut RtEngine, pad: bool) {
    rt.recording = true;
    if pad {
        rt.apply(Command::SamplerPad { pad: 3, on: true });
        rt.apply(Command::SamplerPad { pad: 3, on: false });
    } else {
        rt.apply(Command::LiveNoteOn {
            source: 41,
            ch: 2,
            note: 91,
            vel: 103,
        });
        rt.apply(Command::LiveNoteOff {
            source: 41,
            ch: 2,
            note: 91,
        });
    }
}

#[test]
fn live_and_pad_capture_use_launch_origin_and_full_clip_length() {
    for bars in [1.0, 2.0, 4.0] {
        for pad in [false, true] {
            let mut rt = fixture(bars);
            rt.apply(Command::LaunchClip { track: 1, scene: 2 });
            let origin = rt.tracks[1].playing.unwrap().start_beat;
            let length = bars as f64 * 4.0;
            for offset in [0.0, length - 0.5, length * 2.0 + 0.75] {
                rt.beat = origin + offset;
                capture(&mut rt, pad);
                let note = rt.tracks[1].clips[2].notes.last().unwrap();
                assert_eq!(note.start, offset.rem_euclid(length) as f32);
                assert_eq!(note.vel, if pad { 110 } else { 103 });
            }
            assert_eq!(rt.tracks[1].clips[2].notes.len(), 3);
        }
    }
}

#[test]
fn pending_targets_monitor_without_writing_and_stopped_targets_use_zero_cursor() {
    for pad in [false, true] {
        let mut rt = fixture(4.0);
        rt.playing = true;
        rt.quant = 4.0;
        rt.apply(Command::LaunchClip { track: 1, scene: 2 });
        let start = rt.tracks[1].playing.unwrap().start_beat;
        assert!(start > rt.beat);
        capture(&mut rt, pad);
        assert!(rt.tracks[1].clips[2].notes.is_empty());
        let monitor = if pad {
            &rt.sampler_poly
        } else {
            &rt.tracks[1].poly
        };
        assert!(monitor.voices.iter().any(|v| v.env.stage == 4));
        rt.beat = start;
        capture(&mut rt, pad);
        assert_eq!(rt.tracks[1].clips[2].notes[0].start, 0.0);
        rt.apply(Command::StopTrack { track: 1 });
        rt.beat = 101.25;
        capture(&mut rt, pad);
        assert_eq!(rt.tracks[1].clips[2].notes[1].start, 0.0);
        // A different playing scene cannot supply the compose target's origin.
        rt.tracks[1].playing = Some(PlayingClip {
            scene: 0,
            start_beat: 100.0,
            midi_start_beat: 100.0,
            last_beat: 0.0,
            looping: true,
        });
        capture(&mut rt, pad);
        assert_eq!(rt.tracks[1].clips[2].notes[2].start, 0.0);
    }
    let mut rt = fixture(2.0);
    rt.apply(Command::ComposeArm { track: rt.selected_track, scene: rt.selected_scene });
    rt.apply(Command::SamplerPad { pad: 2, on: true });
    assert_eq!(rt.tracks[1].clips[2].notes[0].start, 0.0);
    assert!(!rt.playing);
}

#[test]
fn captured_later_bar_positions_replay_at_the_same_local_sample_boundary() {
    for bars in [1.0, 2.0, 4.0] {
        let mut record = fixture(bars);
        record.apply(Command::LaunchClip { track: 1, scene: 2 });
        let offset = bars as f64 * 4.0 - 0.5;
        record.beat += offset;
        capture(&mut record, false);
        let mut replay = fixture(bars);
        replay.apply(Command::SetNotes {
            track: 1,
            scene: 2,
            notes: record.tracks[1].clips[2].notes.clone(),
        });
        replay.apply(Command::LaunchClip { track: 1, scene: 2 });
        let origin = replay.beat;
        let step = replay.bpm as f64 / 60.0 / replay.sr as f64;
        let mut first = None;
        for frame in 1..=((offset / step).ceil() as usize + 3) {
            replay.beat = origin + frame as f64 * step;
            replay.render_track(1, false);
            if replay.tracks[1]
                .poly
                .voices
                .iter()
                .any(|v| matches!(v.env.stage, 1..=3))
            {
                first = Some(frame as f64 * step);
                break;
            }
        }
        let actual = first.expect("recorded note never replayed");
        assert!(
            (actual - offset).abs() <= step * 1.01,
            "{actual} vs {offset}"
        );
    }
}
