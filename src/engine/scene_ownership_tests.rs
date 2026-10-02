use super::*;

fn fixture() -> RtEngine {
    let (_tx, rx) = crossbeam_channel::bounded(16);
    let mut rt = RtEngine::new(48_000.0, rx, Arc::new(Mutex::new(Snapshot::default())));
    rt.quant = 0.0;
    for track in &mut rt.tracks {
        track.clips = (0..SCENES).map(|_| Clip::empty()).collect();
        track.fx.slots.clear();
    }
    rt.tracks[1].pan = -1.0;
    rt.tracks[2].pan = 1.0;
    rt
}

fn occupy(rt: &mut RtEngine, track: usize, scene: usize) {
    rt.tracks[track].clips[scene].kind = ClipKind::Midi;
}

#[test]
fn every_scene_panel_owns_its_settings_and_publishes_only_its_own_slots() {
    let mut rt = fixture();
    for scene in 0..SCENES {
        rt.apply(Command::OpenFxScene(scene as u16));
        rt.publish_for_test();
        assert!(rt.snap.lock().fx_slots.is_empty());
        rt.apply(Command::FxAdd(9)); // Filter has two implemented scene parameters.
        rt.apply(Command::FxMix {
            slot: 0,
            value: scene as f32 / 10.0,
        });
        rt.apply(Command::FxParam {
            slot: 0,
            p: 1,
            value: scene as f32 / 9.0,
        });
    }
    for scene in (0..SCENES).rev() {
        rt.apply(Command::CloseFx);
        rt.apply(Command::OpenFxScene(scene as u16));
        rt.publish_for_test();
        let slots = rt.snap.lock().fx_slots.clone();
        assert_eq!(slots.len(), 1);
        assert_eq!(slots[0].0, fx::FxId::Filter.name());
        assert_eq!(slots[0].2, scene as f32 / 10.0);
        assert_eq!(slots[0].3[1], scene as f32 / 9.0);
        rt.apply(Command::FxToggle(0));
        assert!(!rt.scene_fx[scene].slots[0].on);
        for previous in 0..scene {
            assert!(rt.scene_fx[previous].slots[0].on);
        }
    }
}

#[test]
fn added_scenes_process_only_the_tracks_routed_to_each_scene() {
    for effect_scene in [0, 1] {
        let mut rt = fixture();
        occupy(&mut rt, 1, 0);
        occupy(&mut rt, 2, 1);
        rt.tracks[1].poly.note_on(57, 0.8);
        rt.tracks[2].poly.note_on(81, 0.7);
        rt.apply(Command::LaunchScene { scene: 0 });
        rt.apply(Command::AddScene { scene: 1 });
        let mut balance = fx::FxSlot::new(fx::FxId::Balance, rt.sr);
        balance.mix = 1.0;
        // Kill that scene's only populated channel.
        balance.p[0] = if effect_scene == 0 { 1.0 } else { 0.0 };
        rt.scene_fx[effect_scene].slots.push(balance);
        let mut output = vec![0.0; 4096];
        rt.process(&mut output);
        assert_eq!(rt.tracks[1].scene_bus, 0);
        assert_eq!(rt.tracks[2].scene_bus, 1);
        assert!(output
            .chunks_exact(2)
            .all(|frame| frame[effect_scene] == 0.0));
        assert!(output
            .chunks_exact(2)
            .any(|frame| frame[1 - effect_scene].abs() > 0.01));
    }
}

#[test]
fn pending_launches_and_panel_selection_do_not_reroute_until_actual_clip_start() {
    let mut rt = fixture();
    occupy(&mut rt, 1, 0);
    occupy(&mut rt, 1, 3);
    rt.apply(Command::LaunchClip { track: 1, scene: 0 });
    rt.process(&mut [0.0; 32]);
    rt.beat = 0.5;
    rt.quant = 4.0;
    rt.apply(Command::LaunchClip { track: 1, scene: 3 });
    rt.process(&mut [0.0; 32]);
    assert_eq!(rt.tracks[1].scene_bus, 0);
    rt.apply(Command::OpenFxScene(7));
    rt.process(&mut [0.0; 32]);
    assert_eq!(rt.tracks[1].scene_bus, 0);
    rt.beat = 4.0;
    rt.process(&mut [0.0; 2]);
    assert_eq!(rt.tracks[1].scene_bus, 3);
    rt.apply(Command::StopTrack { track: 1 });
    rt.process(&mut [0.0; 32]);
    assert_eq!(rt.tracks[1].scene_bus, 3);
}

#[test]
fn old_scene_delay_tail_stays_on_its_own_bus_after_track_moves() {
    let mut rt = fixture();
    occupy(&mut rt, 1, 0);
    occupy(&mut rt, 1, 1);
    rt.apply(Command::LaunchClip { track: 1, scene: 0 });
    let mut delay = fx::FxSlot::new(fx::FxId::Delay, rt.sr);
    delay.mix = 1.0;
    rt.scene_fx[0].slots.push(delay);
    rt.tracks[1].poly.note_on(57, 0.8);
    rt.process(&mut [0.0; 512]);
    let mut tail = rt.scene_fx[0].clone();
    // Isolate the already seeded scene history from any more track input.
    rt.tracks[1].poly = Poly::new(rt.sr, SynthInstrument::Keys, 16);
    rt.tracks[1].eq = ThreeBand::new(rt.sr);
    rt.tracks[1].eq_right = ThreeBand::new(rt.sr);
    rt.apply(Command::LaunchClip { track: 1, scene: 1 });
    rt.scene_fx[1]
        .slots
        .push(fx::FxSlot::new(fx::FxId::Delay, rt.sr));
    let mut output = vec![0.0; 48_000 * 2];
    rt.process(&mut output);
    assert_eq!(rt.tracks[1].scene_bus, 1);
    assert!(output.iter().any(|x| x.abs() > 0.001));
    for frame in output.chunks_exact(2) {
        let expected = tail
            .process_stereo([0.0; 2], rt.sr)
            .map(|x| limiter(x * rt.master));
        assert_eq!(frame, expected);
    }
    let mut new_scene = rt.scene_fx[1].clone();
    for _ in 0..48_000 {
        assert_eq!(new_scene.process_stereo([0.0; 2], rt.sr), [0.0; 2]);
    }
}
