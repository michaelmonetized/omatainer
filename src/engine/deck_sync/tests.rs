use super::*;
use crate::engine::{beatgrid::Grid, dsp::Sample, test_alloc, Command, Engine};
use std::sync::Arc;

fn fixture(rate: u32) -> (Engine, Box<RtEngine>) {
    let (engine, rt) = Engine::headless_for_test(rate, 256);
    let mut rt = Box::new(rt);
    rt.apply(Command::Stop);
    rt.metronome = false;
    rt.xfader = 0.5;
    rt.xfader_curve = 1.0;
    for (index, source_rate) in [8_000, 44_100].into_iter().enumerate() {
        let times: Vec<_> = (0..=32)
            .map(|beat| {
                let b = f64::from(beat);
                if index == 0 {
                    if beat <= 4 {
                        b * 0.5
                    } else if beat <= 8 {
                        2.0 + (b - 4.0) * 0.75
                    } else {
                        5.0 + (b - 8.0) * 0.4
                    }
                } else if beat <= 4 {
                    b * 2.0 / 3.0
                } else if beat <= 8 {
                    8.0 / 3.0 + (b - 4.0) * 0.4
                } else {
                    8.0 / 3.0 + 1.6 + (b - 8.0) * 0.75
                }
            })
            .collect();
        let mut data = vec![0.0; ((times[32] + 0.1) * f64::from(source_rate)).ceil() as usize * 2];
        for time in &times {
            let start = (time * f64::from(source_rate)).round() as usize;
            for frame in 0..(source_rate / 400) as usize {
                data[(start + frame) * 2 + index] = 0.4;
            }
        }
        let grid = Grid::new(0.0, if index == 0 { 120.0 } else { 90.0 })
            .unwrap()
            .with_anchor(4.0, times[4])
            .unwrap()
            .with_anchor(8.0, times[8])
            .unwrap()
            .with_anchor(32.0, times[32])
            .unwrap();
        let deck = &mut rt.decks[index];
        deck.audio = Some(Arc::new(Sample {
            name: format!("Independent click source {}", index + 1),
            sr: source_rate,
            ch: 2,
            data,
            peaks: Vec::new().into(),
            spectrum: None,
            bpm: if index == 0 { 120.0 } else { 90.0 },
            path: String::new(),
        }));
        deck.load_receipt = None;
        deck.grid = Some(grid);
        deck.sync = false;
        deck.sync_disengage_phase();
        deck.spindle = None;
        deck.loop_on = false;
        deck.playing = true;
        deck.pitch = 0.5;
        deck.rate = 1.0;
        deck.target_rate = 1.0;
        deck.keylock = false;
        deck.pos = grid
            .seconds_at(if index == 0 { 0.25 } else { 4.78 })
            .unwrap()
            * f64::from(source_rate);
        deck.transition_remaining = 0;
    }
    (engine, rt)
}
fn render(rt: &mut RtEngine, frames: usize, chunk: usize) -> Vec<f32> {
    render_observe(rt, frames, chunk, |_| {})
}
fn render_observe(
    rt: &mut RtEngine,
    frames: usize,
    chunk: usize,
    mut observe: impl FnMut(&RtEngine),
) -> Vec<f32> {
    let mut output = vec![0.0; frames * 2];
    assert_eq!(
        test_alloc::measure(|| {
            for start in (0..frames).step_by(chunk) {
                rt.process(&mut output[start * 2..(start + chunk).min(frames) * 2]);
                observe(rt);
            }
        }),
        Default::default()
    );
    output
}
fn phase(rt: &RtEngine, period: f64) -> f64 {
    let beats: [f64; 2] =
        std::array::from_fn(|i| rt.decks[i].grid_beat_at(rt.decks[i].pos, rt.sr, rt.bpm));
    (beats[0] - beats[1] + period * 0.5).rem_euclid(period) - period * 0.5
}
fn starts(pcm: &[f32], side: usize) -> Vec<usize> {
    let mut active = false;
    pcm.chunks_exact(2)
        .enumerate()
        .filter_map(|(frame, sample)| {
            let next = sample[side].abs() > 0.03;
            let onset = next && !active;
            active = next;
            onset.then_some(frame)
        })
        .collect()
}
fn save_pcm(directory: &std::path::Path, name: &str, rate: u32, pcm: &[f32]) {
    use std::io::Write;
    let bytes = (pcm.len() * 4) as u32;
    let mut file = std::fs::File::create(directory.join(name)).unwrap();
    file.write_all(b"RIFF").unwrap();
    file.write_all(&(bytes + 36).to_le_bytes()).unwrap();
    file.write_all(b"WAVEfmt ").unwrap();
    file.write_all(&16u32.to_le_bytes()).unwrap();
    file.write_all(&3u16.to_le_bytes()).unwrap();
    file.write_all(&2u16.to_le_bytes()).unwrap();
    file.write_all(&rate.to_le_bytes()).unwrap();
    file.write_all(&(rate * 8).to_le_bytes()).unwrap();
    file.write_all(&8u16.to_le_bytes()).unwrap();
    file.write_all(&32u16.to_le_bytes()).unwrap();
    file.write_all(b"data").unwrap();
    file.write_all(&bytes.to_le_bytes()).unwrap();
    for value in pcm {
        file.write_all(&value.to_le_bytes()).unwrap();
    }
}

#[test]
fn paired_emitted_clicks_keep_exact_phase_through_source_tempo_boundaries_and_both_explicit_leaders(
) {
    let mut receipts = Vec::new();
    for rate in [8_000, 44_100, 48_000] {
        for mode in [Mode::Beat, Mode::Bar] {
            let (_, mut rt) = fixture(rate);
            rt.apply(Command::DeckSyncLeader(Leader::DeckA));
            rt.apply(Command::DeckSyncMode { deck: 1, mode });
            let period = if mode == Mode::Bar { 4.0 } else { 1.0 };
            let mut first_callback_error = 0.0f64;
            let first = render_observe(&mut rt, rate as usize * 6, 257, |rt| {
                first_callback_error = first_callback_error.max(phase(rt, period).abs())
            });
            assert!(
                first_callback_error < 1e-7,
                "first callback error {first_callback_error}"
            );
            let first_error = phase(&rt, if mode == Mode::Bar { 4.0 } else { 1.0 }).abs();
            assert!(first_error < 1e-7, "first phase error {first_error}");
            let a = starts(&first, 0);
            let b = starts(&first, 1);
            assert!(
                a.len() >= 8 && a.len() == b.len(),
                "paired click counts {} / {}",
                a.len(),
                b.len()
            );
            let first_lag = a.iter().zip(&b).map(|(a, b)| a.abs_diff(*b)).max().unwrap();
            let first_count = a.len();
            assert!(
                first_lag <= (rate as usize).div_ceil(4_000),
                "first emitted click lag {first_lag}"
            );
            let old_leader = rt.decks[1].pos;
            rt.apply(Command::DeckSyncLeader(Leader::DeckB));
            assert_eq!(rt.decks[1].pos, old_leader);
            rt.apply(Command::DeckSyncMode { deck: 0, mode });
            let mut second_callback_error = 0.0f64;
            let second = render_observe(&mut rt, rate as usize * 3, 31, |rt| {
                second_callback_error = second_callback_error.max(phase(rt, period).abs())
            });
            assert!(
                second_callback_error < 1e-7,
                "second callback error {second_callback_error}"
            );
            let second_error = phase(&rt, if mode == Mode::Bar { 4.0 } else { 1.0 }).abs();
            assert!(second_error < 1e-7, "second phase error {second_error}");
            let a = starts(&second, 0);
            let b = starts(&second, 1);
            assert!(
                a.len() >= 3 && a.len() == b.len(),
                "second paired click counts {} / {}",
                a.len(),
                b.len()
            );
            let second_lag = a.iter().zip(&b).map(|(a, b)| a.abs_diff(*b)).max().unwrap();
            assert!(
                second_lag <= (rate as usize).div_ceil(4_000),
                "second emitted click lag {second_lag}"
            );
            let name = format!("paired-{rate}-{}", mode.label().to_lowercase());
            if let Some(directory) = std::env::var_os("OMATAINER_SYNC_PCM_DIR") {
                let directory = std::path::PathBuf::from(directory);
                std::fs::create_dir_all(&directory).unwrap();
                save_pcm(&directory, &format!("{name}-a.wav"), rate, &first);
                save_pcm(&directory, &format!("{name}-b.wav"), rate, &second);
            }
            receipts.push(serde_json::json!({"sample_rate":rate,"mode":mode,"a_leader_clicks":first_count,"b_leader_clicks":a.len(),"resampled_click_error_bound_frames":(rate as usize).div_ceil(4_000),"first_max_click_error_frames":first_lag,"second_max_click_error_frames":second_lag,"first_beat_error":first_error,"second_beat_error":second_error,"first_max_callback_beat_error":first_callback_error,"second_max_callback_beat_error":second_callback_error,"heap_allocations_and_frees":0}));
        }
    }
    println!(
        "SYNC_PCM_RECEIPT {}",
        serde_json::json!({"physical_devices_opened":false,"paired_source_rates":[8000,44100],"cases":receipts})
    );
}

#[test]
fn routed_decks_render_the_chosen_leader_once_and_keep_paired_clicks_aligned() {
    use crate::engine::audio::routing::{model::Model, prepared::Prepared};
    for leader in [Leader::DeckA, Leader::DeckB] {
        for mode in [Mode::Beat, Mode::Bar] {
            let (_, mut rt) = fixture(8_000);
            rt.routing = Some(Box::new(
                Prepared::new(Arc::new(Model::default()), &rt.session).unwrap(),
            ));
            rt.apply(Command::DeckSyncLeader(leader));
            rt.apply(Command::DeckSyncMode {
                deck: (1 - leader.deck().unwrap()) as u8,
                mode,
            });
            let pcm = render(&mut rt, 48_000, 127);
            assert!(phase(&rt, if mode == Mode::Bar { 4.0 } else { 1.0 }).abs() < 1e-7);
            let a = starts(&pcm, 0);
            let b = starts(&pcm, 1);
            assert!(a.len() >= 6 && a.len() == b.len());
            assert!(a.iter().zip(&b).all(|(a, b)| a.abs_diff(*b) <= 2));
        }
    }
}

#[test]
fn invalid_or_foreign_spindle_messages_do_not_retire_phase_but_performed_source_jumps_do() {
    use crate::engine::spindle::{Motion, Playback};
    let (_, mut rt) = fixture(8_000);
    rt.apply(Command::DeckSyncLeader(Leader::DeckA));
    rt.apply(Command::DeckSyncMode {
        deck: 1,
        mode: Mode::Beat,
    });
    render(&mut rt, 1, 1);
    let motion = Motion {
        ticks: 0,
        rate: 1.0,
        at: std::time::Instant::now(),
        hold: 0.01,
    };
    rt.decks[1].spindle = Some(Playback::new(1932, motion, rt.decks[1].pos / 44_100.0));
    rt.apply(Command::DeckSpindle {
        source: 1932,
        deck: 1,
        motion: Motion {
            rate: 99.0,
            ..motion
        },
    });
    assert_eq!(rt.decks[1].sync_mode(), Mode::Beat);
    rt.apply(Command::DeckSpindle {
        source: 1933,
        deck: 1,
        motion: Motion {
            rate: -1.0,
            ..motion
        },
    });
    assert_eq!(rt.decks[1].sync_mode(), Mode::Beat);
    rt.decks[1].spindle = None;
    rt.decks[1].hotcues[0] = crate::engine::HotCue {
        set: true,
        pos: 20_000.0,
    };
    rt.apply(Command::DeckHotCue {
        deck: 1,
        pad: 0,
        del: false,
    });
    assert_eq!(rt.decks[1].sync_mode(), Mode::Tempo);
    rt.apply(Command::DeckSyncMode {
        deck: 1,
        mode: Mode::Beat,
    });
    render(&mut rt, 1, 1);
    rt.apply(Command::DeckSeek { deck: 0, frac: 0.3 });
    assert_eq!(rt.decks[1].sync_mode(), Mode::Tempo);
}

#[test]
fn selecting_a_stopped_clock_seeds_its_tempo_and_one_shot_match_restores_independent_targets_with_undo(
) {
    let (_, mut rt) = fixture(8_000);
    for deck in &mut rt.decks {
        deck.playing = false;
    }
    rt.apply(Command::SetBpm(180.0));
    rt.apply(Command::DeckSyncMode {
        deck: 1,
        mode: Mode::Tempo,
    });
    assert_eq!(rt.decks[1].sync_bpm, 180.0);
    rt.apply(Command::DeckSyncLeader(Leader::DeckA));
    assert_eq!(rt.decks[1].sync_bpm, 120.0);
    rt.apply(Command::DeckSyncMode {
        deck: 1,
        mode: Mode::Bar,
    });
    rt.clear_undo_for_test();
    rt.apply(Command::DeckMatch);
    assert!(rt.deck_sync.leader.is_none());
    assert_eq!(rt.decks[1].sync_mode(), Mode::Tempo);
    render(&mut rt, 100, 31);
    assert_eq!(rt.decks[1].sync_bpm, 120.0);
    let positions = rt.decks.each_ref().map(|deck| deck.pos);
    assert_eq!(
        test_alloc::measure(|| rt.apply(Command::Undo)),
        Default::default()
    );
    assert_eq!(rt.deck_sync.leader, Some(Leader::DeckA));
    assert_eq!(rt.decks[1].sync_mode(), Mode::Bar);
    assert_eq!(rt.decks.each_ref().map(|deck| deck.pos), positions);
    rt.apply(Command::Redo);
    assert!(rt.deck_sync.leader.is_none());
    assert_eq!(rt.decks[1].sync_mode(), Mode::Tempo);
    let revision = rt.undo.checkpoint();
    rt.apply(Command::DeckSyncMode {
        deck: 255,
        mode: Mode::Beat,
    });
    assert_eq!(rt.undo.checkpoint(), revision);
}

#[test]
fn integral_loops_keep_alignment_and_incompatible_loop_lengths_retain_tempo_only() {
    let (_, mut rt) = fixture(8_000);
    rt.apply(Command::DeckSyncLeader(Leader::DeckA));
    for deck in &mut rt.decks {
        deck.loop_start = deck.grid_position_at(0.0, rt.sr, rt.bpm);
        deck.loop_len = deck.grid_position_at(4.0, rt.sr, rt.bpm) - deck.loop_start;
        deck.loop_on = true;
        deck.pos = deck.grid_position_at(0.25, rt.sr, rt.bpm);
    }
    rt.apply(Command::DeckSyncMode {
        deck: 1,
        mode: Mode::Bar,
    });
    let mut error = 0.0f64;
    render_observe(&mut rt, 48_000, 73, |rt| {
        error = error.max(phase(rt, 4.0).abs())
    });
    assert!(error < 1e-7, "loop alignment error {error}");
    assert_eq!(rt.decks[1].sync_mode(), Mode::Bar);
    rt.decks[0].loop_len = rt.decks[0].grid_position_at(3.5, rt.sr, rt.bpm);
    render(&mut rt, 32_000, 73);
    assert_eq!(rt.decks[1].sync_mode(), Mode::Tempo);
    rt.decks[1].loop_len = rt.decks[1].grid_position_at(3.5, rt.sr, rt.bpm);
    rt.apply(Command::DeckSyncMode {
        deck: 1,
        mode: Mode::Beat,
    });
    render(&mut rt, 1, 1);
    assert_eq!(rt.decks[1].sync_mode(), Mode::Tempo);
}

#[test]
fn tempo_only_and_jogged_offsets_survive_maps_until_an_explicit_phase_rearm() {
    let (_, mut rt) = fixture(8_000);
    rt.apply(Command::DeckSyncLeader(Leader::DeckA));
    rt.apply(Command::DeckSyncMode {
        deck: 1,
        mode: Mode::Tempo,
    });
    let offset = phase(&rt, 1.0);
    render(&mut rt, 48_000, 257);
    assert!((phase(&rt, 1.0) - offset).abs() < 1e-7);
    rt.apply(Command::DeckSyncMode {
        deck: 1,
        mode: Mode::Beat,
    });
    render(&mut rt, 1, 1);
    assert!(phase(&rt, 1.0).abs() < 1e-7);
    rt.apply(Command::DeckJog {
        deck: 1,
        delta: 0.25,
    });
    assert_eq!(rt.decks[1].sync_mode(), Mode::Tempo);
    let jogged = phase(&rt, 1.0);
    assert!(jogged.abs() > 0.0001);
    render(&mut rt, 8_000, 31);
    assert!((phase(&rt, 1.0) - jogged).abs() < 1e-7);
    rt.apply(Command::DeckSyncMode {
        deck: 1,
        mode: Mode::Beat,
    });
    render(&mut rt, 1, 1);
    assert!(phase(&rt, 1.0).abs() < 1e-7);
    rt.apply(Command::DeckTouch { deck: 1, on: true });
    assert_eq!(rt.decks[1].sync_mode(), Mode::Tempo);
    rt.apply(Command::DeckTouch { deck: 1, on: false });
    render(&mut rt, 100, 31);
    assert_eq!(rt.decks[1].sync_mode(), Mode::Tempo);
    rt.apply(Command::DeckSyncMode {
        deck: 1,
        mode: Mode::Bar,
    });
    render(&mut rt, 1, 1);
    assert!(phase(&rt, 4.0).abs() < 1e-7);
    rt.apply(Command::DeckJog {
        deck: 0,
        delta: -0.1,
    });
    assert_eq!(rt.decks[1].sync_mode(), Mode::Tempo);
}

#[test]
fn paused_alignment_arms_without_play_and_a_stopped_or_replaced_leader_never_chooses_a_new_clock() {
    let (_, mut rt) = fixture(8_000);
    rt.decks[0].playing = false;
    rt.decks[1].playing = false;
    rt.apply(Command::DeckSyncLeader(Leader::DeckA));
    rt.apply(Command::DeckSyncMode {
        deck: 1,
        mode: Mode::Beat,
    });
    let positions = rt.decks.each_ref().map(|deck| deck.pos);
    render(&mut rt, 100, 31);
    assert_eq!(rt.decks.each_ref().map(|deck| deck.pos), positions);
    assert!(!rt.decks[1].sync_phase_locked);
    assert_eq!(rt.decks[1].sync_mode(), Mode::Beat);
    rt.apply(Command::DeckPlay { deck: 0 });
    rt.apply(Command::DeckPlay { deck: 1 });
    render(&mut rt, 100, 31);
    assert!(rt.decks[1].sync_phase_locked);
    assert!(phase(&rt, 1.0).abs() < 1e-7);
    rt.apply(Command::DeckPlay { deck: 0 });
    render(&mut rt, 100, 31);
    assert_eq!(rt.decks[1].sync_mode(), Mode::Tempo);
    let tempo = rt.decks[1].sync_bpm;
    rt.apply(Command::DeckPlay { deck: 0 });
    render(&mut rt, 100, 31);
    assert_eq!(rt.decks[1].sync_mode(), Mode::Tempo);
    rt.apply(Command::DeckSyncMode {
        deck: 1,
        mode: Mode::Beat,
    });
    render(&mut rt, 1, 1);
    rt.decks[0].playing = false;
    rt.apply(Command::DeckUnload { deck: 0 });
    render(&mut rt, 100, 31);
    assert_eq!(rt.deck_sync.leader, Some(Leader::DeckA));
    assert_eq!(rt.decks[1].sync_mode(), Mode::Tempo);
    assert_eq!(rt.decks[1].sync_bpm, tempo);
    let replacement = rt.decks[1].audio.clone().unwrap();
    rt.apply(Command::DeckAudio {
        deck: 0,
        audio: replacement,
    });
    rt.decks[0].playing = true;
    render(&mut rt, 100, 31);
    assert_eq!(rt.decks[1].sync_mode(), Mode::Tempo);
    rt.apply(Command::DeckSyncMode {
        deck: 1,
        mode: Mode::Beat,
    });
    render(&mut rt, 1, 1);
    assert!(rt.decks[1].sync_phase_locked);
    rt.decks[0].audio = Some(Arc::new(
        (*rt.decks[0].audio.as_ref().unwrap().as_ref()).clone(),
    ));
    render(&mut rt, 1, 1);
    assert_eq!(rt.decks[1].sync_mode(), Mode::Tempo);
}

#[test]
fn native_conductor_ramps_and_pickups_share_exact_quarter_note_intervals_with_both_source_maps() {
    use crate::engine::midi_data::{Conductor, Meter, Tempo, TimingSettings};
    for rate in [8_000, 44_100, 48_000] {
        for mode in [Mode::Beat, Mode::Bar] {
            let (_, mut rt) = fixture(rate);
            let map = Conductor::native(
                960,
                vec![
                    Tempo::new(0, 120.0, true).unwrap(),
                    Tempo::new(3840, 180.0, false).unwrap(),
                ],
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
            rt.sync_midi_clock();
            rt.apply(Command::DeckSyncLeader(Leader::Transport));
            for deck in 0..2 {
                rt.apply(Command::DeckSyncMode { deck, mode });
            }
            assert!(!rt.playing);
            rt.apply(Command::Play);
            render(&mut rt, rate as usize * 4, 257);
            let expected = map.beat_at_seconds(4.0);
            assert!((rt.precise_midi_beat() - expected).abs() < 1e-8);
            let period = if mode == Mode::Bar { 4.0 } else { 1.0 };
            for deck in &rt.decks {
                let difference = deck.grid_beat_at(deck.pos, rt.sr, rt.bpm) - (expected - 0.5);
                assert!(
                    ((difference + period * 0.5).rem_euclid(period) - period * 0.5).abs() < 1e-7
                );
                assert!(deck.sync_phase_locked);
            }
            assert!(phase(&rt, period).abs() < 1e-7);
            rt.apply(Command::Stop);
            render(&mut rt, 1, 1);
            assert_eq!(rt.decks[0].sync_mode(), Mode::Tempo);
            assert_eq!(rt.decks[1].sync_mode(), Mode::Tempo);
        }
    }
}

#[test]
fn sync_intent_roundtrips_real_native_files_reopens_stopped_and_undo_changes_no_performed_playhead()
{
    use crate::engine::project::{Captured, Prepared, State as ProjectState, STATE_VERSION};
    use std::sync::atomic::AtomicBool;
    let (engine, mut rt) = fixture(8_000);
    rt.apply(Command::DeckSyncLeader(Leader::DeckA));
    rt.clear_undo_for_test();
    rt.apply(Command::DeckSyncMode {
        deck: 1,
        mode: Mode::Bar,
    });
    render(&mut rt, 100, 31);
    let positions = rt.decks.each_ref().map(|deck| deck.pos);
    assert_eq!(
        test_alloc::measure(|| rt.apply(Command::Undo)),
        Default::default()
    );
    assert_eq!(rt.deck_sync.leader, Some(Leader::DeckA));
    assert_eq!(rt.decks[1].sync_mode(), Mode::Off);
    assert_eq!(rt.decks.each_ref().map(|deck| deck.pos), positions);
    rt.apply(Command::Redo);
    assert_eq!(rt.decks[1].sync_mode(), Mode::Bar);
    assert_eq!(rt.decks.each_ref().map(|deck| deck.pos), positions);
    let handle = engine.project.clone();
    let worker = std::thread::spawn(move || handle.capture(&AtomicBool::new(false)).unwrap());
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    while !worker.is_finished() {
        rt.process(&mut []);
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    let captured: Captured = worker.join().unwrap();
    assert_eq!(captured.state.version, STATE_VERSION);
    assert_eq!(captured.state.sync_leader, Some(Leader::DeckA));
    assert_eq!(captured.state.decks[1].sync_phase, Phase::Bar);
    let path =
        std::env::temp_dir().join(format!("omatainer-sync-intent-{}.omat", std::process::id()));
    let limits = crate::project_file::Limits::default();
    crate::project_file::save(
        &path,
        &crate::project_file::Bundle {
            state: captured.state,
            media: captured.media,
        },
        crate::project_file::Overwrite::Never,
        &limits,
        &AtomicBool::new(false),
    )
    .unwrap();
    let bundle =
        crate::project_file::load::<ProjectState>(&path, &limits, &AtomicBool::new(false)).unwrap();
    std::fs::remove_file(&path).unwrap();
    let reopened =
        Prepared::from_state(bundle.state.clone(), bundle.media.clone(), 44_100).unwrap();
    assert_eq!(reopened.rt.deck_sync.leader, Some(Leader::DeckA));
    assert_eq!(reopened.rt.decks[1].sync_mode(), Mode::Bar);
    assert!(!reopened.rt.decks[0].playing && !reopened.rt.decks[1].playing && !reopened.rt.playing);
    assert!(!reopened.rt.decks[1].sync_phase_locked);
    let mut legacy = serde_json::to_value(&bundle.state).unwrap();
    legacy["version"] = 24.into();
    assert!(serde_json::from_value::<ProjectState>(legacy.clone()).is_err());
    legacy.as_object_mut().unwrap().remove("sync_leader");
    for deck in legacy["decks"].as_array_mut().unwrap() {
        deck.as_object_mut().unwrap().remove("sync_phase");
    }
    let old = serde_json::from_value::<ProjectState>(legacy).unwrap();
    assert!(old.sync_leader.is_none());
    assert!(old.decks.iter().all(|deck| deck.sync_phase.is_none()));
    Prepared::from_state(old, bundle.media, 8_000).unwrap();
}
