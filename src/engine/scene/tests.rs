use super::*;
use crate::engine::{
    clip_launch::{Grid, Policy},
    midi_edit::Outcome,
    midi_schedule::Gate,
    project::{Captured, Prepared},
    session::{Action, Axis, Request},
    test_alloc, Clip, ClipKind, Command, Engine, MidiNote, RtEngine,
};
use std::sync::atomic::AtomicBool;

fn fixture(rate: u32) -> (Engine, Box<RtEngine>) {
    let (engine, rt) = Engine::headless_for_test(rate, 256);
    let mut rt = Box::new(rt);
    rt.apply(Command::Stop);
    rt.apply(Command::SetBpm(120.0));
    rt.quant = 0.0;
    for track in &mut rt.tracks {
        for clip in &mut track.clips {
            *clip = Clip::empty();
        }
    }
    for track in 0..4 {
        for scene in 0..2 {
            if scene == 1 && track >= 2 {
                continue;
            }
            let mut clip = Clip::empty();
            clip.kind = ClipKind::Midi;
            clip.bars = 8.0;
            clip.notes = vec![MidiNote { variation: None,
                id: crate::engine::midi_edit::NoteId::new(),
                channel: 0,
                release_vel: 64,
                pitch: 60 + scene as u8 * 12 + track as u8,
                start: 0.0,
                len: 24.0,
                vel: 100,
                muted: false,
                source_timing: None,
            }];
            clip.properties.launch = Policy {
                grid: Grid::EightBars,
                ..Default::default()
            };
            rt.tracks[track].clips[scene] = clip;
        }
        rt.tracks[track].kind = 2;
        rt.tracks[track].midi_schedule.sample_trace = Some(Vec::with_capacity(64));
    }
    (engine, rt)
}
fn capture(engine: &Engine, rt: &mut RtEngine) -> Captured {
    let handle = engine.project.clone();
    let worker = std::thread::spawn(move || handle.capture(&AtomicBool::new(false)).unwrap());
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    while !worker.is_finished() {
        rt.process(&mut []);
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    worker.join().unwrap()
}
fn render(rt: &mut RtEngine, frames: usize, chunk: usize) {
    let mut out = vec![0.0; chunk * 2];
    assert_eq!(
        test_alloc::measure(|| {
            for begin in (0..frames).step_by(chunk) {
                rt.process(&mut out[..(frames - begin).min(chunk) * 2]);
            }
        }),
        Default::default()
    );
}
fn properties(tempo: f64, numerator: u8, power: u8, grid: Grid, empty: Empty) -> Properties {
    Properties {
        tempo_micros: Some((60_000_000.0 / tempo).round() as u32),
        meter: Some(Signature {
            numerator,
            denominator_power: power,
        }),
        grid,
        empty,
    }
}
#[test]
fn whole_scene_changes_at_one_sample_in_odd_meter_with_independent_tempo_and_downbeat_oracles() {
    for rate in [8000, 44100, 48000] {
        for chunk in [31, 257] {
            let (_, mut rt) = fixture(rate);
            rt.session.scenes[0].scene = properties(120.0, 7, 3, Grid::Immediate, Empty::Stop);
            rt.session.scenes[1].scene = properties(180.0, 4, 2, Grid::Bar, Empty::Keep);
            rt.metronome = true;
            rt.metro.trace = Some(Vec::with_capacity(64));
            assert_eq!(
                test_alloc::measure(|| rt.apply(Command::LaunchScene { scene: 0 })),
                Default::default()
            );
            render(&mut rt, 100, chunk);
            let continuing = rt.tracks[2].playing.unwrap();
            assert_eq!(
                test_alloc::measure(|| rt.apply(Command::LaunchScene { scene: 1 })),
                Default::default()
            );
            assert_eq!(rt.scenes.pending.unwrap().when, 3.5);
            let boundary = (f64::from(rate) * 1.75).round() as usize;
            render(&mut rt, boundary - 100, chunk);
            assert_eq!(rt.bpm, 120.0);
            assert_eq!(rt.tracks[0].playing.unwrap().scene, 0);
            render(&mut rt, 1, chunk);
            for track in 0..2 {
                assert_eq!(rt.tracks[track].playing.unwrap().scene, 1);
                assert!(
                    rt.tracks[track]
                        .midi_schedule
                        .sample_trace
                        .as_ref()
                        .unwrap()
                        .contains(&(boundary as u64, Gate::On(72 + track as u8, 100))),
                    "{rate}/{chunk} {:?}",
                    rt.tracks[track].midi_schedule.sample_trace
                );
            }
            assert_eq!(
                rt.tracks[2].playing.unwrap().midi_start_beat,
                continuing.midi_start_beat
            );
            assert_eq!(rt.tracks[2].playing.unwrap().scene, 0);
            assert_eq!(rt.scenes.timing.unwrap().signature, Signature::default());
            assert!(rt
                .metro
                .trace
                .as_ref()
                .unwrap()
                .contains(&(boundary as u64, true)));
            assert_eq!(rt.scenes.timing.unwrap().anchor, 3.5);
            assert!(
                (rt.timeline_seconds() - (boundary + 1) as f64 / f64::from(rate)).abs() < 1e-12
            );
            let actual_bpm = f64::from(rt.bpm);
            render(&mut rt, 1000, chunk);
            assert!(
                (rt.precise_midi_beat() - (3.5 + 1001.0 * actual_bpm / 60.0 / f64::from(rate)))
                    .abs()
                    < 1e-9
            );
            let timing = rt.scenes.timing.unwrap();
            assert_eq!(timing.position(3.5).1, 0.0);
            assert_eq!(timing.boundary(3.7, 1), 7.5);
            assert_eq!(rt.clip_boundary(Grid::Bar), 7.5);
        }
    }
}
#[test]
fn empty_keep_preserves_exact_pcm_and_stop_releases_only_at_the_boundary() {
    let (_, mut actual) = fixture(8000);
    let (_, mut oracle) = fixture(8000);
    for rt in [&mut actual, &mut oracle] {
        rt.tracks[2].kind = 4;
        let clip = &mut rt.tracks[2].clips[0];
        clip.kind = ClipKind::Audio;
        clip.notes.clear();
        clip.audio = Some(std::sync::Arc::new(crate::engine::Sample {
            name: "continuing scene PCM".into(),
            sr: 8000,
            ch: 2,
            data: vec![0.05; 64000],
            peaks: Default::default(),
            spectrum: None,
            bpm: 120.0,
            path: String::new(),
        }));
        rt.session.scenes[0].scene.grid = Grid::Immediate;
        rt.apply(Command::LaunchScene { scene: 0 });
    }
    let mut a = vec![0.0; 128];
    let mut b = a.clone();
    actual.process(&mut a);
    oracle.process(&mut b);
    assert_eq!(a, b);
    actual.session.scenes[2].scene = Properties {
        empty: Empty::Keep,
        grid: Grid::Immediate,
        ..Default::default()
    };
    actual.apply(Command::LaunchScene { scene: 2 });
    assert_eq!(
        test_alloc::measure(|| actual.process(&mut a)),
        Default::default()
    );
    oracle.process(&mut b);
    assert_eq!(a, b);
    actual.session.scenes[2].scene = Properties {
        empty: Empty::Stop,
        grid: Grid::Quarter,
        ..Default::default()
    };
    actual.apply(Command::LaunchScene { scene: 2 });
    assert!(actual.tracks[2].playing.is_some());
    render(&mut actual, 4000 - 128, 31);
    assert!(actual.tracks[2].playing.is_some());
    render(&mut actual, 1, 31);
    assert!(actual.tracks[2].playing.is_none());
}
#[test]
fn additive_scene_disabled_cells_legato_and_physical_owners_survive_switches() {
    let (_, mut rt) = fixture(8000);
    rt.session.scenes[0].scene.grid = Grid::Immediate;
    rt.apply(Command::LaunchScene { scene: 0 });
    render(&mut rt, 2000, 31);
    rt.apply(Command::Select { track: 2, scene: 0 });
    rt.apply(Command::LiveNoteOn {
        source: 91,
        ch: 0,
        note: 77,
        vel: 99,
    });
    rt.tracks[1].clips[1].properties.disabled = true;
    rt.tracks[0].clips[1].properties.launch.legato = true;
    rt.session.scenes[1].scene.grid = Grid::Immediate;
    rt.apply(Command::AddScene { scene: 1 });
    assert_eq!(rt.tracks[0].playing.unwrap().midi_start_beat, 0.0);
    assert_eq!(rt.tracks[1].playing.unwrap().scene, 0);
    assert_eq!(rt.tracks[2].playing.unwrap().scene, 0);
    rt.apply(Command::LaunchScene { scene: 1 });
    render(&mut rt, 1, 31);
    assert!(rt.tracks[2].playing.is_none());
    assert_eq!(rt.tracks[1].playing.unwrap().scene, 0);
    assert!(rt.tracks[2]
        .poly
        .voices
        .iter()
        .any(|voice| voice.owner == crate::engine::dsp::VoiceOwner::Live
            && voice.note() == 77
            && matches!(voice.env.stage, 1..=3)));
}
#[test]
fn metadata_worker_transactions_undo_redo_stale_cancel_and_identity_changes_are_atomic() {
    let (_, mut rt) = fixture(8000);
    let id = rt.session.scenes[1].id;
    let value = properties(95.0, 7, 3, Grid::Bar, Empty::Keep);
    let make = |rt: &RtEngine, value| {
        Request::metadata(
            &rt.session,
            rt.undo.checkpoint().epoch,
            Action::SceneProperties {
                id,
                properties: value,
            },
        )
        .unwrap()
    };
    let (request, ack) = make(&rt, value);
    let (stale, stale_ack) = make(&rt, Default::default());
    let command = Command::session_edit(request);
    assert_eq!(
        test_alloc::measure(|| rt.apply(command)),
        Default::default()
    );
    assert_eq!(ack.state(), Outcome::Applied);
    assert_eq!(rt.session.scenes[1].scene, value);
    rt.apply(Command::session_edit(stale));
    assert_eq!(stale_ack.state(), Outcome::Rejected);
    rt.apply(Command::Undo);
    assert!(rt.session.scenes[1].scene.is_default());
    rt.apply(Command::Redo);
    assert_eq!(rt.session.scenes[1].scene, value);
    let (cancelled, cancel_ack) = make(&rt, Default::default());
    cancel_ack.cancel();
    rt.apply(Command::session_edit(cancelled));
    assert_eq!(cancel_ack.state(), Outcome::Cancelled);
    assert_eq!(rt.session.scenes[1].scene, value);
    rt.session.scenes[0].scene.grid = Grid::Immediate;
    rt.apply(Command::LaunchScene { scene: 0 });
    render(&mut rt, 100, 31);
    rt.apply(Command::LaunchScene { scene: 1 });
    let pending = rt.scenes.pending.unwrap();
    let (move_scene, _) = Request::metadata(
        &rt.session,
        rt.undo.checkpoint().epoch,
        Action::Move {
            axis: Axis::Scene,
            id,
            position: 0,
        },
    )
    .unwrap();
    rt.apply(Command::session_edit(move_scene));
    assert_eq!(rt.scenes.pending, Some(pending));
    let (edit, _) = make(&rt, properties(110.0, 4, 2, Grid::Bar, Empty::Stop));
    rt.apply(Command::session_edit(edit));
    assert!(rt.scenes.pending.is_none());
    assert_eq!(rt.scenes.error, Some(Error::ChangedScene));
    rt.apply(Command::LaunchScene { scene: 1 });
    let old = rt.session.scenes[1].id;
    rt.session.delete(Axis::Scene, old).unwrap();
    rt.scene_metadata_changed();
    assert!(rt.scenes.pending.is_none());
    let (new, slot) = rt
        .session
        .create(Axis::Scene, "Replacement".into(), None, 0)
        .unwrap();
    assert_eq!(slot, 1);
    assert_ne!(old, new);
    assert!(rt.session.scenes[1].scene.is_default());
}
#[test]
fn scene_queue_latest_toggle_cancel_stop_seek_safety_and_project_replacement_never_replay() {
    let (_, mut rt) = fixture(8000);
    rt.session.scenes[0].scene.grid = Grid::Immediate;
    rt.apply(Command::LaunchScene { scene: 0 });
    render(&mut rt, 100, 31);
    for scene in [1, 2] {
        rt.session.scenes[scene].scene.grid = Grid::Bar;
        rt.apply(Command::LaunchScene {
            scene: scene as u16,
        });
        assert!(rt.scenes.pending.is_some());
    }
    assert_eq!(rt.scenes.pending.unwrap().scene.id, rt.session.scenes[2].id);
    rt.apply(Command::ToggleScene { scene: 2 });
    assert!(rt.scenes.pending.is_none());
    assert_eq!(rt.tracks[0].playing.unwrap().scene, 0);
    rt.apply(Command::LaunchScene { scene: 1 });
    rt.apply(Command::CancelScene);
    assert!(rt.scenes.pending.is_none());
    rt.apply(Command::LaunchScene { scene: 1 });
    rt.apply(Command::TimelineSeek(0.1));
    assert!(rt.scenes.pending.is_none());
    rt.apply(Command::LaunchScene { scene: 1 });
    rt.apply(Command::Stop);
    assert!(rt.scenes.pending.is_none());
    rt.apply(Command::Play);
    rt.apply(Command::LaunchScene { scene: 1 });
    rt.apply(Command::SafetyStop(
        crate::engine::performance::Safety::Stop,
    ));
    assert!(rt.scenes.pending.is_none());
    rt.apply(Command::Play);
    rt.apply(Command::LaunchScene { scene: 1 });
    let mut replacement = Prepared::empty(8000).unwrap();
    replacement.swap_into(&mut rt);
    assert!(rt.scenes.pending.is_none());
    assert!(rt.scenes.active.is_none());
}
#[test]
fn native_state_23_file_roundtrip_retains_scene_timing_but_not_pending_work_and_legacy_guards_are_strict(
) {
    let (engine, mut rt) = fixture(8000);
    rt.session.scenes[0].scene = properties(105.0, 7, 3, Grid::Immediate, Empty::Keep);
    rt.apply(Command::LaunchScene { scene: 0 });
    render(&mut rt, 100, 31);
    rt.session.scenes[1].scene.grid = Grid::Bar;
    rt.apply(Command::LaunchScene { scene: 1 });
    assert!(rt.scenes.pending.is_some());
    let saved = capture(&engine, &mut rt);
    assert_eq!(saved.state.version, crate::engine::project::STATE_VERSION);
    assert_eq!(saved.state.scene_timing, rt.scenes.timing);
    let path = std::env::temp_dir().join(format!(
        "omatainer-scene-properties-{}.omat",
        crate::sampler_bank::BankId::new().unwrap()
    ));
    let limits = crate::project_file::Limits::default();
    let cancel = AtomicBool::new(false);
    crate::project_file::save(
        &path,
        &crate::project_file::Bundle {
            state: saved.state.clone(),
            media: saved.media.clone(),
        },
        crate::project_file::Overwrite::Never,
        &limits,
        &cancel,
    )
    .unwrap();
    let reopened =
        crate::project_file::load::<crate::engine::project::State>(&path, &limits, &cancel)
            .unwrap();
    assert_eq!(reopened.state.session, saved.state.session);
    assert_eq!(reopened.state.scene_timing, saved.state.scene_timing);
    let prepared = Prepared::from_state(reopened.state, reopened.media, 8000).unwrap();
    assert_eq!(prepared.rt.scenes.timing, rt.scenes.timing);
    assert!(prepared.rt.scenes.pending.is_none());
    assert!(prepared.rt.scenes.active.is_none());
    std::fs::remove_file(path).unwrap();
    let mut raw = serde_json::to_value(&saved.state).unwrap();
    raw["version"] = 22.into();
    assert!(serde_json::from_value::<crate::engine::project::State>(raw.clone()).is_err());
    raw["scene_timing"] = serde_json::Value::Null;
    assert!(serde_json::from_value::<crate::engine::project::State>(raw).is_err());
    let mut legacy = saved.state.clone();
    legacy.scene_timing = None;
    for item in &mut legacy.session.as_mut().unwrap().scenes {
        item.scene = Default::default();
    }
    legacy.version = 22;
    let raw = serde_json::to_value(&legacy).unwrap();
    serde_json::from_value::<crate::engine::project::State>(raw.clone())
        .unwrap()
        .validate(&saved.media)
        .unwrap();
    for field in [
        serde_json::Value::Null,
        serde_json::to_value(Properties::default()).unwrap(),
    ] {
        let mut value = raw.clone();
        value["session"]["scenes"][0]["scene"] = field;
        assert!(serde_json::from_value::<crate::engine::project::State>(value).is_err());
    }
    let mut invalid = saved.state.clone();
    invalid.session.as_mut().unwrap().tracks[0].scene.empty = Empty::Keep;
    assert!(invalid.validate(&saved.media).is_err());
    let mut invalid = saved.state.clone();
    invalid.scene_timing.as_mut().unwrap().anchor = f64::NAN;
    assert!(invalid.validate(&saved.media).is_err());
}
#[test]
fn odd_scene_count_in_click_labels_and_rate_changes_use_the_selected_meter_without_heap_work() {
    use crate::engine::midi_data::{Conductor, Meter, Tempo, TimingSettings};
    let (_, mut rt) = fixture(8000);
    rt.conductor = Some(
        Conductor::native(
            960,
            vec![Tempo::new(0, 120.0, false).unwrap()],
            vec![Meter {
                tick: 0,
                numerator: 4,
                denominator_power: 2,
                clocks: 24,
                thirty_seconds: 8,
            }],
            TimingSettings {
                count_in: 1,
                subdivision: 2,
                pickup: 0.5,
                ..Default::default()
            },
        )
        .unwrap(),
    );
    rt.session.scenes[0].scene = properties(120.0, 7, 3, Grid::Immediate, Empty::Stop);
    rt.metro.trace = Some(Vec::with_capacity(64));
    rt.apply(Command::TogglePlay);
    assert!(rt.count_in.is_some());
    assert!(rt.conductor.is_none());
    render(&mut rt, 14000, 31);
    assert_eq!(rt.precise_midi_beat(), 0.0);
    assert_eq!(rt.metro.trace.as_ref().unwrap().len(), 14);
    assert_eq!(rt.metro.trace.as_ref().unwrap()[0], (0, true));
    render(&mut rt, 1, 31);
    assert!(rt.count_in.is_none());
    assert_eq!(
        rt.tracks[0].midi_schedule.sample_trace.as_ref().unwrap()[0].0,
        14000
    );
    let timing = rt.scenes.timing.unwrap();
    timing.validate().unwrap();
    assert_eq!(timing.signature.numerator, 7);
    assert_eq!(timing.position(3.5).1, 0.0);
    rt.apply(Command::Stop);
    rt.set_sample_rate(48000).unwrap();
    rt.apply(Command::Play);
    assert!(rt.count_in.is_some());
    render(&mut rt, 84000, 257);
    assert_eq!(rt.count_in.as_ref().unwrap().remaining(), 0.0);
}
#[test]
fn tempo_only_scene_inherits_pickup_meter_and_native_conductor_undo_restores_the_flat_scene_clock()
{
    use crate::engine::midi_data::{Conductor, Meter, Tempo, TimingSettings};
    let (engine, mut rt) = fixture(8000);
    let map = Conductor::native(
        960,
        vec![Tempo::new(0, 120.0, false).unwrap()],
        vec![Meter {
            tick: 0,
            numerator: 7,
            denominator_power: 3,
            clocks: 12,
            thirty_seconds: 8,
        }],
        TimingSettings {
            pickup: 0.5,
            ..Default::default()
        },
    )
    .unwrap();
    rt.conductor = Some(map.clone());
    rt.session.scenes[0].scene = Properties {
        tempo_micros: Some(600000),
        grid: Grid::Immediate,
        ..Default::default()
    };
    rt.apply(Command::LaunchScene { scene: 0 });
    let timing = rt.scenes.timing.unwrap();
    timing.validate().unwrap();
    assert_eq!(timing.anchor, 0.5);
    assert_eq!(timing.signature.numerator, 7);
    rt.apply(Command::Stop);
    let (request, ack) = crate::engine::midi_interchange::Request::prepare_timing(
        capture(&engine, &mut rt),
        Some(map.clone()),
        || false,
    )
    .unwrap();
    rt.apply(Command::MidiImport(request));
    assert_eq!(ack.state(), Outcome::Applied);
    assert!(rt.scenes.timing.is_none());
    assert!(rt.conductor.is_some());
    rt.apply(Command::Undo);
    assert_eq!(rt.scenes.timing, Some(timing));
    assert!(rt.conductor.is_none());
    rt.apply(Command::Redo);
    assert!(rt.scenes.timing.is_none());
    assert!(rt.conductor.is_some());
    rt.apply(Command::Undo);
    let (stale, stale_ack) = crate::engine::midi_interchange::Request::prepare_timing(
        capture(&engine, &mut rt),
        Some(map),
        || false,
    )
    .unwrap();
    rt.session.scenes[1].scene = Properties {
        meter: Some(Signature::default()),
        grid: Grid::Immediate,
        ..Default::default()
    };
    rt.apply(Command::LaunchScene { scene: 1 });
    rt.apply(Command::Stop);
    rt.apply(Command::MidiImport(stale));
    assert_eq!(stale_ack.state(), Outcome::Rejected);
}
#[test]
fn scene_switch_finishes_a_real_recording_take_and_preserves_the_held_key_until_release() {
    use crate::engine::{dsp::VoiceOwner, InputKey};
    let (_, mut rt) = fixture(8000);
    rt.selected_track = 1;
    rt.selected_scene = 0;
    rt.session.scenes[0].scene.grid = Grid::Immediate;
    rt.apply(Command::LaunchScene { scene: 0 });
    rt.apply(Command::Record);
    rt.apply(Command::LiveNoteOn {
        source: 511,
        ch: 3,
        note: 65,
        vel: 100,
    });
    render(&mut rt, 800, 31);
    let input = InputKey::Midi {
        source: 511,
        ch: 3,
        note: 65,
    };
    assert!(rt.recording);
    rt.session.scenes[1].scene.grid = Grid::Immediate;
    rt.apply(Command::LaunchScene { scene: 1 });
    assert!(rt.tracks[1].clips[0]
        .notes
        .iter()
        .any(|note| note.pitch == 65 && note.len > 0.0));
    assert!(rt.tracks[1]
        .poly
        .voices
        .iter()
        .any(|voice| voice.input == Some(input)
            && voice.owner == VoiceOwner::Live
            && matches!(voice.env.stage, 1..=3)));
    rt.apply(Command::LiveNoteOff {
        source: 511,
        ch: 3,
        note: 65,
    });
    render(&mut rt, 1, 31);
    assert!(!rt.tracks[1]
        .poly
        .voices
        .iter()
        .any(|voice| voice.input == Some(input) && matches!(voice.env.stage, 1..=3)));
}
#[test]
fn tempo_only_scenes_preserve_the_full_native_meter_range_and_pickup_phase() {
    use crate::engine::midi_data::{Conductor, Meter, Tempo, TimingSettings};
    for numerator in [1, 7, 32, 255] {
        for power in [0, 3, 7] {
            let (_, mut rt) = fixture(8000);
            let unit = 4.0 / f64::from(1u32 << power);
            let pickup = unit * 0.5;
            rt.conductor = Some(
                Conductor::native(
                    960,
                    vec![Tempo::new(0, 120.0, false).unwrap()],
                    vec![Meter {
                        tick: 0,
                        numerator,
                        denominator_power: power,
                        clocks: 1,
                        thirty_seconds: 8,
                    }],
                    TimingSettings {
                        pickup,
                        ..Default::default()
                    },
                )
                .unwrap(),
            );
            rt.session.scenes[0].scene = Properties {
                tempo_micros: Some(600000),
                grid: Grid::Immediate,
                ..Default::default()
            };
            rt.apply(Command::LaunchScene { scene: 0 });
            let timing = rt.scenes.timing.unwrap();
            timing.validate().unwrap();
            assert_eq!(timing.anchor, pickup);
            assert_eq!(
                timing.signature,
                Signature {
                    numerator,
                    denominator_power: power
                }
            );
            assert_eq!(timing.position(pickup).1, 0.0);
            assert_eq!(timing.click_between(pickup, pickup + 0.00001), Some(true));
            let value = Properties {
                meter: Some(Signature {
                    numerator,
                    denominator_power: power,
                }),
                ..Default::default()
            };
            assert!(value.valid());
        }
    }
    for signature in [
        Signature {
            numerator: 0,
            denominator_power: 2,
        },
        Signature {
            numerator: 4,
            denominator_power: 8,
        },
    ] {
        assert!(!Properties {
            meter: Some(signature),
            ..Default::default()
        }
        .valid());
    }
}
