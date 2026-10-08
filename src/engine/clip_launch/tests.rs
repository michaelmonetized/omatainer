use super::*;
use crate::engine::midi_schedule::Gate;
use crate::engine::{test_alloc, Clip, ClipKind, Command, Engine, MidiNote, RtEngine};
use std::sync::atomic::AtomicBool;

fn fixture() -> (Engine, Box<RtEngine>) {
    let (engine, rt) = Engine::headless_for_test(8000, 256);
    let mut rt = Box::new(rt);
    rt.apply(Command::Stop);
    rt.apply(Command::SetBpm(120.0));
    rt.quant = 0.0;
    for scene in 0..3 {
        let mut clip = Clip::empty();
        clip.kind = ClipKind::Midi;
        clip.bars = if scene == 0 { 2.0 } else { 0.5 };
        clip.notes = vec![MidiNote { variation: None,
            id: crate::engine::midi_edit::NoteId::new(),
            channel: 0,
            release_vel: 64,
            pitch: 60 + scene as u8 * 7,
            start: 0.0,
            len: 1.75,
            vel: 100,
            muted: false,
            source_timing: None,
        }];
        rt.tracks[2].clips[scene] = clip;
    }
    rt.tracks[2].midi_schedule.sample_trace = Some(Vec::with_capacity(256));
    (engine, rt)
}
fn press(rt: &mut RtEngine, source: u64, scene: u16) {
    rt.apply(Command::ClipPress(Press {
        source,
        key: u32::from(scene),
        target: Target::Slot {
            track: 2,
            scene,
            looping: true,
        },
    }));
}
fn release(rt: &mut RtEngine, source: u64, scene: u16) {
    rt.apply(Command::ClipRelease(Release {
        source,
        key: u32::from(scene),
    }));
}
fn render(rt: &mut RtEngine, frames: usize) {
    let mut out = vec![0.0; frames * 2];
    rt.process(&mut out);
    assert!(out.iter().all(|s| s.is_finite()));
}
fn policy(rt: &mut RtEngine, scene: usize, mode: Mode, grid: Grid) {
    rt.tracks[2].clips[scene].properties.launch = Policy {
        mode,
        grid,
        legato: false,
    };
}
fn capture(engine: &Engine, rt: &mut RtEngine) -> crate::engine::project::Captured {
    let handle = engine.project.clone();
    let job = std::thread::spawn(move || handle.capture(&AtomicBool::new(false)).unwrap());
    let start = std::time::Instant::now();
    while !job.is_finished() {
        rt.process(&mut []);
        assert!(start.elapsed() < std::time::Duration::from_secs(10));
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    job.join().unwrap()
}
#[test]
fn queued_switch_keeps_the_previous_owner_until_the_exact_sample_boundary_without_heap_work() {
    let (_engine, mut rt) = fixture();
    press(&mut rt, 41, 0);
    release(&mut rt, 41, 0);
    render(&mut rt, 80);
    rt.apply(Command::Select { track: 2, scene: 0 });
    rt.apply(Command::LiveNoteOn {
        source: 93,
        ch: 0,
        note: 77,
        vel: 99,
    });
    policy(&mut rt, 1, Mode::Trigger, Grid::Quarter);
    assert_eq!(
        test_alloc::measure(|| press(&mut rt, 42, 1)),
        Default::default()
    );
    release(&mut rt, 42, 1);
    assert_eq!(rt.tracks[2].playing.unwrap().scene, 0);
    assert_eq!(rt.tracks[2].launch.queued_scene(), Some(1));
    render(&mut rt, 3920);
    assert_eq!(rt.tracks[2].playing.unwrap().scene, 0);
    let mut out = [0.0; 2];
    assert_eq!(
        test_alloc::measure(|| rt.process(&mut out)),
        Default::default()
    );
    assert_eq!(rt.tracks[2].playing.unwrap().scene, 1);
    assert!(rt.tracks[2]
        .midi_schedule
        .sample_trace
        .as_ref()
        .unwrap()
        .contains(&(4000, Gate::On(67, 100))));
    assert!(rt.tracks[2]
        .poly
        .voices
        .iter()
        .any(|v| v.owner == crate::engine::dsp::VoiceOwner::Live
            && v.note() == 77
            && matches!(v.env.stage, 1..=3)));
}
#[test]
fn actual_queued_audio_matches_an_independent_boundary_switch_oracle() {
    for (bpm, quant) in [(60.0, 0.25), (123.0, 0.5), (180.0, 1.0)] {
        let (_engine, mut rt) = fixture();
        let (_reference_engine, mut reference) = fixture();
        for engine in [&mut *rt, &mut *reference] {
            engine.apply(Command::SetBpm(bpm));
            engine.tracks[2].kind = 4;
            for (scene, value) in [(0, 0.1), (1, -0.08)] {
                let clip = &mut engine.tracks[2].clips[scene];
                clip.kind = ClipKind::Audio;
                clip.notes.clear();
                clip.audio = Some(std::sync::Arc::new(crate::engine::Sample {
                    name: "boundary PCM".into(),
                    sr: 8000,
                    ch: 2,
                    data: vec![value; 16000],
                    peaks: Default::default(),
                    spectrum: None,
                    bpm: 120.0,
                    path: String::new(),
                }));
            }
            engine.apply(Command::LaunchClip { track: 2, scene: 0 });
        }
        let mut actual = vec![0.0; 160];
        let mut expected = actual.clone();
        rt.process(&mut actual);
        reference.process(&mut expected);
        assert_eq!(actual, expected);
        rt.quant = quant;
        press(&mut rt, 41, 1);
        release(&mut rt, 41, 1);
        let boundary = (f64::from(quant) * 60.0 / f64::from(bpm) * 8000.0 + 1e-7).floor() as usize;
        actual.resize((boundary - 80) * 2, 0.0);
        expected.resize(actual.len(), 0.0);
        rt.process(&mut actual);
        reference.process(&mut expected);
        assert_eq!(actual, expected, "prior PCM changed before boundary");
        reference.apply(Command::LaunchClip { track: 2, scene: 1 });
        actual.resize(256, 0.0);
        expected.resize(256, 0.0);
        rt.process(&mut actual);
        reference.process(&mut expected);
        assert_eq!(rt.tracks[2].playing.unwrap().scene, 1);
        assert_eq!(
            actual, expected,
            "audio transition differed from independent frame boundary"
        );
    }
}
#[test]
fn early_gate_release_cancels_its_onset_and_old_release_preserves_the_next_queue() {
    let (_engine, mut rt) = fixture();
    policy(&mut rt, 0, Mode::Gate, Grid::Immediate);
    policy(&mut rt, 1, Mode::Gate, Grid::Quarter);
    press(&mut rt, 41, 0);
    render(&mut rt, 80);
    press(&mut rt, 42, 1);
    release(&mut rt, 42, 1);
    assert_eq!(rt.tracks[2].launch.queued_scene(), None);
    assert_eq!(rt.tracks[2].playing.unwrap().scene, 0);
    press(&mut rt, 42, 1);
    release(&mut rt, 41, 0);
    render(&mut rt, 1);
    assert!(rt.tracks[2].playing.is_none());
    assert_eq!(rt.tracks[2].launch.queued_scene(), Some(1));
    render(&mut rt, 3920);
    assert_eq!(rt.tracks[2].playing.unwrap().scene, 1);
    release(&mut rt, 42, 1);
    assert!(rt.tracks[2].launch.stopping());
    rt.apply(Command::ClipCancel { track: 2 });
    assert!(!rt.tracks[2].launch.stopping());
    render(&mut rt, 4000);
    assert_eq!(rt.tracks[2].playing.unwrap().scene, 1);
}
#[test]
fn toggle_uses_press_edges_and_can_cancel_both_queued_start_and_stop() {
    let (_engine, mut rt) = fixture();
    policy(&mut rt, 0, Mode::Toggle, Grid::Quarter);
    press(&mut rt, 41, 0);
    render(&mut rt, 80);
    let id = rt.tracks[2].launch.active_id;
    press(&mut rt, 41, 0);
    assert_eq!(id, rt.tracks[2].launch.active_id);
    release(&mut rt, 41, 0);
    press(&mut rt, 41, 0);
    assert!(rt.tracks[2].launch.stopping());
    release(&mut rt, 41, 0);
    press(&mut rt, 41, 0);
    assert!(!rt.tracks[2].launch.stopping());
    release(&mut rt, 41, 0);
    policy(&mut rt, 1, Mode::Toggle, Grid::Quarter);
    press(&mut rt, 42, 1);
    release(&mut rt, 42, 1);
    assert_eq!(rt.tracks[2].launch.queued_scene(), Some(1));
    press(&mut rt, 42, 1);
    release(&mut rt, 42, 1);
    assert_eq!(rt.tracks[2].launch.queued_scene(), None);
    render(&mut rt, 4000);
    assert_eq!(rt.tracks[2].playing.unwrap().scene, 0);
}
#[test]
fn repeat_retriggers_on_musical_samples_and_immediate_uses_the_clip_length() {
    for (grid, period) in [(Grid::Eighth, 2000), (Grid::Immediate, 8000)] {
        let (_engine, mut rt) = fixture();
        policy(&mut rt, 1, Mode::Repeat, grid);
        press(&mut rt, 41, 1);
        render(&mut rt, period * 3 + 1);
        let ons: Vec<_> = rt.tracks[2]
            .midi_schedule
            .sample_trace
            .as_ref()
            .unwrap()
            .iter()
            .filter_map(|(frame, event)| (*event == Gate::On(67, 100)).then_some(*frame))
            .collect();
        assert_eq!(
            ons,
            vec![0, period as u64, period as u64 * 2, period as u64 * 3]
        );
        release(&mut rt, 41, 1);
        assert!(rt.tracks[2].launch.repeat.is_none());
        render(&mut rt, period + 1);
        assert!(rt.tracks[2].playing.is_none());
    }
    for (grid, quantum, expected) in [
        (Grid::Eighth, 0.0, vec![0, 1000, 3000, 5000]),
        (Grid::Global, 0.5, vec![0, 1000, 3000, 5000]),
        (Grid::Immediate, 0.5, vec![0, 8000, 16000, 24000]),
    ] {
        let (_engine, mut rt) = fixture();
        rt.apply(Command::SongSeek(3.25));
        rt.quant = quantum;
        policy(&mut rt, 1, Mode::Repeat, grid);
        assert_eq!(
            test_alloc::measure(|| press(&mut rt, 41, 1)),
            Default::default()
        );
        let mut out = vec![0.0; (expected.last().unwrap() + 1) * 2];
        assert_eq!(
            test_alloc::measure(|| rt.process(&mut out)),
            Default::default()
        );
        let ons: Vec<_> = rt.tracks[2]
            .midi_schedule
            .sample_trace
            .as_ref()
            .unwrap()
            .iter()
            .filter_map(|(frame, event)| (*event == Gate::On(67, 100)).then_some(*frame as usize))
            .collect();
        assert_eq!(ons, expected, "stopped position 3.25 with {grid:?}");
    }
}
#[test]
fn bar_grids_and_repeat_follow_pickups_odd_meters_and_fragmentary_changes_without_heap() {
    use crate::engine::midi_data::{Conductor, Meter, Tempo, TimingSettings};
    let map = Conductor::native(
        960,
        vec![Tempo::new(0, 120.0, false).unwrap()],
        [(0, 7, 3), (3360, 5, 2), (8160, 4, 2)]
            .into_iter()
            .map(|(tick, numerator, denominator_power)| Meter {
                tick,
                numerator,
                denominator_power,
                clocks: 24,
                thirty_seconds: 8,
            })
            .collect(),
        TimingSettings {
            pickup: 0.5,
            ..Default::default()
        },
    )
    .unwrap();
    let boundaries = map.bar_boundaries(0.0, 80.0, 64);
    for bars in [1, 2, 4, 8] {
        for beat in [0.0, 0.25, 0.5, 0.5001, 3.4999, 3.5, 3.5001, 8.5, 12.5, 19.0] {
            let oracle = boundaries
                .iter()
                .find(|(b, number)| *b >= beat && (number - 1) % bars == 0)
                .unwrap()
                .0;
            let mut actual = 0.0;
            assert_eq!(
                test_alloc::measure(|| actual = map.next_bar_boundary(beat, bars)),
                Default::default()
            );
            assert_eq!(actual, oracle, "grid {bars}, position {beat}");
        }
    }
    let (_engine, mut rt) = fixture();
    rt.conductor = Some(map);
    rt.playing = true;
    policy(&mut rt, 1, Mode::Repeat, Grid::Bar);
    assert_eq!(
        test_alloc::measure(|| press(&mut rt, 41, 1)),
        Default::default()
    );
    assert_eq!(rt.tracks[2].launch.queued.unwrap().when, 0.5);
    let mut out = [0.0; 2];
    assert_eq!(
        test_alloc::measure(|| {
            for _ in 0..50001 {
                rt.process(&mut out);
            }
        }),
        Default::default()
    );
    let ons: Vec<_> = rt.tracks[2]
        .midi_schedule
        .sample_trace
        .as_ref()
        .unwrap()
        .iter()
        .filter_map(|(frame, event)| (*event == Gate::On(67, 100)).then_some(*frame))
        .collect();
    assert_eq!(ons, [2000, 14000, 34000, 50000]);
    release(&mut rt, 41, 1);
    assert!(rt.tracks[2].launch.repeat.is_none());
    assert_eq!(rt.tracks[2].launch.stopping.unwrap().when, 16.5);
    render(&mut rt, 16000);
    assert!(rt.tracks[2].playing.is_none());
}
#[test]
fn legato_transfers_unequal_musical_phase_and_chases_only_the_new_clip_notes() {
    let (_engine, mut rt) = fixture();
    rt.tracks[2].clips[0].notes[0].len = 7.5;
    press(&mut rt, 41, 0);
    release(&mut rt, 41, 0);
    render(&mut rt, 13000);
    policy(&mut rt, 1, Mode::Trigger, Grid::Immediate);
    rt.tracks[2].clips[1].properties.launch.legato = true;
    rt.tracks[2].clips[1].notes[0].start = 1.0;
    rt.tracks[2].clips[1].notes[0].len = 0.75;
    assert_eq!(
        test_alloc::measure(|| press(&mut rt, 42, 1)),
        Default::default()
    );
    render(&mut rt, 1);
    let playing = rt.tracks[2].playing.unwrap();
    assert_eq!(playing.scene, 1);
    assert!((playing.last_beat - 1.25).abs() < 0.001);
    assert!(rt.tracks[2]
        .poly
        .voices
        .iter()
        .any(|v| v.owner == crate::engine::dsp::VoiceOwner::Clip
            && v.note() == 67
            && matches!(v.env.stage, 1..=3)));
    assert!(rt.tracks[2]
        .poly
        .voices
        .iter()
        .filter(|v| v.owner == crate::engine::dsp::VoiceOwner::Clip && v.note() == 60)
        .all(|v| !matches!(v.env.stage, 1..=3)));
}
#[test]
fn native_policy_and_presets_migrate_legacy_defaults_and_never_resume_held_modes() {
    let (engine, mut rt) = fixture();
    policy(&mut rt, 1, Mode::Gate, Grid::TwoBars);
    rt.tracks[2].clips[1].properties.launch.legato = true;
    press(&mut rt, 41, 1);
    let captured = capture(&engine, &mut rt);
    captured.state.validate(&captured.media).unwrap();
    let raw = serde_json::to_value(&captured.state).unwrap();
    let root = std::env::temp_dir().join(format!(
        "clip-launch-project-{}.omat",
        crate::engine::midi_edit::NoteId::new()
    ));
    crate::project_file::save(
        &root,
        &crate::project_file::Bundle {
            state: captured.state,
            media: captured.media.clone(),
        },
        crate::project_file::Overwrite::Never,
        &Default::default(),
        &AtomicBool::new(false),
    )
    .unwrap();
    let reopened = crate::project_file::load::<crate::engine::project::State>(
        &root,
        &Default::default(),
        &AtomicBool::new(false),
    )
    .unwrap();
    let state = reopened.state;
    assert_eq!(
        state.tracks[2].clips[1].properties.launch,
        rt.tracks[2].clips[1].properties.launch
    );
    std::fs::remove_file(root).unwrap();
    let prepared =
        crate::engine::project::Prepared::from_state(state.clone(), captured.media.clone(), 8000)
            .unwrap();
    assert!(prepared.rt.tracks[2].project_resume.is_none());
    let mut legacy = raw.clone();
    legacy["version"] = 20.into();
    assert!(serde_json::from_value::<crate::engine::project::State>(legacy.clone()).is_err());
    legacy["tracks"][2]["clips"][1]["properties"]
        .as_object_mut()
        .unwrap()
        .remove("launch");
    assert!(
        serde_json::from_value::<crate::engine::project::State>(legacy)
            .unwrap()
            .tracks[2]
            .clips[1]
            .properties
            .launch
            .is_default()
    );
    let bundle = crate::engine::clip_management::preset::Preset::capture(
        state.tracks[2].clips[1].clone(),
        &captured.media,
        &AtomicBool::new(false),
    )
    .unwrap();
    let root = std::env::temp_dir().join(format!(
        "clip-launch-preset-{}.omatclip",
        crate::engine::midi_edit::NoteId::new()
    ));
    crate::engine::clip_management::preset::write(&bundle, &root, &AtomicBool::new(false)).unwrap();
    let reopened =
        crate::engine::clip_management::preset::read(&root, &AtomicBool::new(false)).unwrap();
    assert_eq!(
        reopened.state.clip.properties.launch,
        rt.tracks[2].clips[1].properties.launch
    );
    std::fs::remove_file(root).unwrap();
    let mut preset = serde_json::to_value(reopened.state).unwrap();
    assert_eq!(preset["clip_schema"], crate::engine::project::STATE_VERSION);
    let mut previous = preset.clone();
    previous["clip_schema"] = 21.into();
    serde_json::from_value::<crate::engine::clip_management::preset::Preset>(previous)
        .unwrap()
        .validate(&[], &AtomicBool::new(false))
        .unwrap();
    preset["clip_schema"] = 20.into();
    assert!(
        serde_json::from_value::<crate::engine::clip_management::preset::Preset>(preset.clone())
            .is_err()
    );
    preset["clip"]["properties"]
        .as_object_mut()
        .unwrap()
        .remove("launch");
    let preset: crate::engine::clip_management::preset::Preset =
        serde_json::from_value(preset).unwrap();
    preset.validate(&[], &AtomicBool::new(false)).unwrap();
    let mut unknown = raw;
    unknown["tracks"][2]["clips"][1]["properties"]["launch"]["wrong"] = true.into();
    assert!(serde_json::from_value::<crate::engine::project::State>(unknown).is_err());
}
#[test]
fn stale_targets_and_releases_refuse_without_renderer_allocation_and_seek_retains_the_owner() {
    let (_engine, mut rt) = fixture();
    rt.surface.status.track_offset = crate::engine::session::MAX_TRACKS;
    assert_eq!(
        test_alloc::measure(|| rt.apply(Command::ClipPress(Press {
            source: 42,
            key: 8,
            target: Target::Apc(0)
        }))),
        Default::default()
    );
    policy(&mut rt, 0, Mode::Gate, Grid::Immediate);
    press(&mut rt, 41, 0);
    render(&mut rt, 80);
    rt.seek_timeline(0.1);
    release(&mut rt, 41, 0);
    render(&mut rt, 1);
    assert!(rt.tracks[2].playing.is_none());
    press(&mut rt, 41, 0);
    policy(&mut rt, 1, Mode::Gate, Grid::Quarter);
    press(&mut rt, 42, 1);
    rt.session = crate::engine::session::Layout::legacy(
        (0..rt.tracks.len()).map(|_| String::new()),
        rt.scene_fx.len(),
    );
    assert_eq!(
        test_alloc::measure(|| release(&mut rt, 41, 0)),
        Default::default()
    );
    rt.beat = 2.0;
    assert_eq!(
        test_alloc::measure(|| rt.clip_launch_tick(2)),
        Default::default()
    );
    assert_eq!(rt.tracks[2].playing.unwrap().scene, 0);
    assert!(rt.tracks[2].launch.queued.is_none());
}
#[test]
fn identity_wrapped_holds_reserve_release_and_cannot_cross_a_pending_stop() {
    let (engine, mut rt) = fixture();
    policy(&mut rt, 0, Mode::Gate, Grid::Immediate);
    let command = crate::engine::session::Scoped::qualify_layout(
        Command::ClipPress(Press {
            source: 41,
            key: 0,
            target: Target::Slot {
                track: 2,
                scene: 0,
                looping: true,
            },
        }),
        &rt.session,
    )
    .unwrap();
    engine.send(command).unwrap();
    rt.process(&mut []);
    assert_eq!(rt.tracks[2].playing.unwrap().scene, 0);
    while engine.send(Command::NudgeBpm(0.0)).is_ok() {}
    assert_eq!(
        engine
            .send(Command::ClipRelease(Release { source: 41, key: 0 }))
            .unwrap(),
        crate::engine::SubmissionOutcome::Accepted
    );
    for _ in 0..16 {
        rt.process(&mut [0.0; 2]);
    }
    assert!(rt.tracks[2].playing.is_none());
    engine.send(Command::StopTrack { track: 2 }).unwrap();
    let command = crate::engine::session::Scoped::qualify_layout(
        Command::ClipPress(Press {
            source: 42,
            key: 0,
            target: Target::Slot {
                track: 2,
                scene: 0,
                looping: true,
            },
        }),
        &rt.session,
    )
    .unwrap();
    assert_eq!(
        engine.send(command),
        Err(crate::engine::SubmissionError::StopPending)
    );
    rt.process(&mut []);
    assert!(rt.tracks[2].playing.is_none());
}
#[test]
fn emergency_recovery_discards_clip_holds_before_the_same_control_is_pressed_again() {
    let (engine, mut rt) = fixture();
    policy(&mut rt, 0, Mode::Gate, Grid::Immediate);
    let input = Press {
        source: 41,
        key: 0,
        target: Target::Slot {
            track: 2,
            scene: 0,
            looping: true,
        },
    };
    engine.send(Command::ClipPress(input)).unwrap();
    render(&mut rt, 1);
    assert_eq!(rt.tracks[2].playing.unwrap().scene, 0);
    engine
        .send(Command::SafetyStop(
            crate::engine::performance::Safety::Stop,
        ))
        .unwrap();
    let mut out = [0.0; 2];
    assert_eq!(
        test_alloc::measure(|| rt.process(&mut out)),
        Default::default()
    );
    assert!(rt.tracks[2].playing.is_none());
    assert!(rt.clip_launch_inputs.held.iter().all(Option::is_none));
    engine
        .send(Command::ClipRelease(Release { source: 41, key: 0 }))
        .unwrap();
    engine.send(Command::RecoverPerformance).unwrap();
    render(&mut rt, 128);
    assert!(!engine.cmd.performance().status().recovery);
    engine.send(Command::ClipPress(input)).unwrap();
    render(&mut rt, 1);
    assert_eq!(rt.tracks[2].playing.unwrap().scene, 0);
    engine
        .send(Command::ClipRelease(Release { source: 41, key: 0 }))
        .unwrap();
    render(&mut rt, 1);
    assert!(rt.tracks[2].playing.is_none());
}
#[test]
fn empty_clip_pad_stops_only_its_track_and_disabled_empty_slots_leave_playback_alone() {
    let (_engine, mut rt) = fixture();
    press(&mut rt, 41, 0);
    release(&mut rt, 41, 0);
    rt.apply(Command::LaunchClip { track: 1, scene: 0 });
    rt.tracks[2].clips[2] = Clip::empty();
    rt.tracks[2].clips[2].properties.disabled = true;
    press(&mut rt, 42, 2);
    assert_eq!(rt.tracks[2].playing.unwrap().scene, 0);
    rt.tracks[2].clips[2].properties.disabled = false;
    assert_eq!(
        test_alloc::measure(|| press(&mut rt, 43, 2)),
        Default::default()
    );
    assert!(rt.tracks[2].playing.is_none());
    assert_eq!(rt.tracks[1].playing.unwrap().scene, 0);
}
