use super::*;

fn fixture() -> RtEngine {
    let (_tx, rx) = crossbeam_channel::bounded(64);
    let mut rt = RtEngine::new(48_000.0, rx, Arc::new(Mutex::new(Snapshot::default())));
    rt.apply(Command::Stop);
    rt.bpm = 120.0;
    rt.quant = 0.0;
    rt.apply(Command::SamplerInst(SamplerInstrument::Synth(SynthInstrument::Keys)));
    rt
}

fn pad(rt: &mut RtEngine, id: u8) {
    rt.apply(Command::SamplerPad { pad: id, on: true });
    rt.apply(Command::SamplerPad { pad: id, on: false });
}

fn counts(rt: &RtEngine) -> Vec<usize> {
    rt.tracks
        .iter()
        .flat_map(|t| t.clips.iter().map(|c| c.notes.len()))
        .collect()
}

#[test]
fn selection_and_stop_restart_leave_pads_monitor_only() {
    let mut rt = fixture();
    rt.apply(Command::Select { track: 4, scene: 3 });
    assert_eq!(rt.tracks[4].clips[3].kind, ClipKind::Empty);
    assert_eq!(rt.compose_target, None);
    pad(&mut rt, 0);
    rt.apply(Command::ComposeArm { track: 4, scene: 3 });
    pad(&mut rt, 1);
    assert_eq!(rt.tracks[4].clips[3].notes.len(), 1);
    rt.apply(Command::Stop);
    rt.apply(Command::TogglePlay);
    assert!(rt.playing);
    assert_eq!(rt.compose_target, None);
    let before = counts(&rt);
    pad(&mut rt, 2);
    assert_eq!(counts(&rt), before);
}

#[test]
fn armed_target_survives_browsing_scene_launch_and_play_without_moving_monitor() {
    for instrument in [SamplerInstrument::Samples, SamplerInstrument::Synth(SynthInstrument::Keys)] {
        let mut rt = fixture();
        rt.apply(Command::SamplerInst(instrument));
        rt.apply(Command::ComposeArm { track: 4, scene: 3 });
        for command in [
            Command::Select { track: 2, scene: 1 },
            Command::Play,
            Command::LaunchClip { track: 0, scene: 0 },
            Command::LaunchScene { scene: 0 },
            Command::RestartScene { scene: 1 },
        ] {
            rt.apply(command);
            let before = counts(&rt);
            rt.apply(Command::SamplerPad { pad: 0, on: true });
            assert_eq!(
                rt.compose_target,
                Some(ComposeTarget { track: 4, scene: 3 })
            );
            if instrument == SamplerInstrument::Samples {
                assert_eq!(rt.pad_voices[0].as_ref().unwrap().3, 4);
            } else {
                assert_eq!(rt.pad_destinations[0], 4);
            }
            rt.apply(Command::SamplerPad { pad: 0, on: false });
            let mut expected = before;
            expected[4 * SCENES + 3] += 1;
            assert_eq!(counts(&rt), expected);
        }
    }
}

#[test]
fn explicit_disarm_and_retarget_finalize_pad_capture_without_releasing_live_input() {
    let mut rt = fixture();
    rt.apply(Command::ComposeArm { track: 4, scene: 3 });
    rt.apply(Command::SamplerPad { pad: 0, on: true });
    rt.note_recording.clock += 0.5;
    rt.apply(Command::ComposeArm { track: 5, scene: 4 });
    assert_eq!(rt.tracks[4].clips[3].notes[0].len, 0.5);
    assert_eq!(rt.pad_destinations[0], 4);
    assert!(rt
        .sampler_poly
        .voices
        .iter()
        .any(|v| v.input == Some(InputKey::Pad(0)) && matches!(v.env.stage, 1..=3)));
    rt.apply(Command::SamplerPad { pad: 1, on: true });
    rt.note_recording.clock += 0.75;
    rt.apply(Command::ComposeDisarm);
    assert_eq!(rt.tracks[5].clips[4].notes[0].len, 0.75);
    let before = counts(&rt);
    rt.note_recording.clock += 1.0;
    for id in [0, 1, 2] {
        pad(&mut rt, id);
    }
    assert_eq!(counts(&rt), before);
    assert_eq!(rt.tracks[4].clips[3].notes[0].len, 0.5);
    assert_eq!(rt.tracks[5].clips[4].notes[0].len, 0.75);
}

#[test]
fn disarmed_record_workflow_still_captures_and_toggle_stop_disarms() {
    let mut rt = fixture();
    rt.apply(Command::Select { track: 4, scene: 3 });
    rt.apply(Command::Record);
    pad(&mut rt, 0);
    assert_eq!(rt.tracks[4].clips[3].notes.len(), 1);
    rt.apply(Command::Record);
    pad(&mut rt, 1);
    assert_eq!(rt.tracks[4].clips[3].notes.len(), 1);
    rt.apply(Command::ComposeArm { track: 4, scene: 3 });
    rt.apply(Command::Play);
    rt.apply(Command::TogglePlay);
    assert_eq!(rt.compose_target, None);
    assert!(!rt.recording);
}

#[test]
fn arm_validation_and_snapshot_keep_exact_destination() {
    let mut rt = fixture();
    rt.apply(Command::ComposeArm { track: 4, scene: 3 });
    for (track, scene) in [(TRACKS, 0), (0, SCENES), (usize::MAX, usize::MAX)] {
        rt.apply(Command::ComposeArm { track, scene });
        rt.apply(Command::Select { track, scene });
        assert_eq!(
            rt.compose_target,
            Some(ComposeTarget { track: 4, scene: 3 })
        );
    }
    rt.publish_for_test();
    assert_eq!(rt.snap.lock().compose_target, rt.compose_target);
    rt.apply(Command::ComposeDisarm);
    rt.publish_for_test();
    assert_eq!(rt.snap.lock().compose_target, None);
}
