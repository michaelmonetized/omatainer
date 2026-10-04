use super::*;
use crate::engine::{test_alloc, Command, Engine};
use std::sync::Arc;

#[test]
#[ignore = "Controlled optimized callback qualification with two full 32-anchor maps"]
fn maximum_anchor_maps_keep_hybrid_callbacks_bounded_without_heap_work() {
    assert!(
        !cfg!(debug_assertions),
        "Run this workload in the optimized profile"
    );
    for frames in [128, 256, 1024] {
        let (engine, mut rt) =
            crate::engine::performance_workload_tests::prepared_at("hybrid", 48_000);
        let mut grid = Grid::new(0.0, 240.0).unwrap();
        let mut seconds = 0.0;
        for beat in 1..=MAX_ANCHORS {
            seconds += 0.21 + (beat as f64 * 0.7).sin() * 0.025;
            grid = grid.with_anchor(beat as f64, seconds).unwrap();
        }
        for deck in 0..2 {
            assert!(seconds < rt.decks[deck].audio.as_ref().unwrap().frames() as f64 / 48_000.0);
            rt.decks[deck].grid = Some(grid);
            rt.decks[deck].sync = true;
            rt.decks[deck].sync_bpm = 127.0;
            rt.decks[deck].loop_on = false;
            rt.decks[deck].pos = grid.seconds_at(2.0).unwrap() * 48_000.0;
            rt.apply(Command::DeckLoop {
                deck: deck as u8,
                beats: 24.0,
            });
        }
        let mut callback = crate::engine::audio::OutputCallback::new(rt, 2);
        let mut output = vec![0.0f32; frames * 2];
        for _ in 0..128 {
            callback.render(&mut output);
        }
        let mut cpu = Vec::with_capacity(4096);
        let mut wall = Vec::with_capacity(4096);
        let mut energy = 0.0f64;
        for _ in 0..4096 {
            assert_eq!(
                test_alloc::measure(|| callback.render(&mut output)),
                test_alloc::Counts::default()
            );
            let measurement = engine.cmd.audio_metrics().last_callback.unwrap();
            cpu.push(measurement.render_cpu_ns.unwrap());
            wall.push(measurement.elapsed_ns);
            assert!(output.iter().all(|sample| sample.is_finite()));
            energy += output
                .iter()
                .map(|sample| f64::from(*sample).powi(2))
                .sum::<f64>();
        }
        cpu.sort_unstable();
        wall.sort_unstable();
        let deadline = frames as u64 * 1_000_000_000 / 48_000;
        assert!(
            cpu[4055] < deadline * 3 / 4,
            "128/256/1024-frame render CPU p99 exceeds 75% of its software budget"
        );
        assert!(energy > 0.01);
        let rt = callback.renderer_for_test();
        assert!(rt
            .decks
            .iter()
            .all(|deck| deck.grid.unwrap().anchors().len() == MAX_ANCHORS));
        println!(
            "VARIABLE_GRID_MAX_CALLBACK {}",
            serde_json::json!({"frames":frames,"callbacks":4096,
            "sample_rate":48000,"tempo_anchors_per_deck":32,"decks":2,"deadline_ns":deadline,
            "p99_render_cpu_ns":cpu[4055],"max_render_cpu_ns":cpu[4095],"p99_callback_wall_ns":wall[4055],
            "max_callback_wall_ns":wall[4095],"energy":energy,"heap_allocations_and_frees":0,
            "notes":rt.tracks.iter().map(|track| track.clips[0].notes.len()).sum::<usize>()})
        );
    }
}

/// Build a tempo ramp with independent source click positions.
/// Returns original mono PCM, its manual anchor map and the source beat times.
fn ramping_click() -> (Arc<crate::engine::dsp::Sample>, Grid, Vec<f64>) {
    let mut times = vec![0.0];
    for beat in 0..32 {
        times.push(times.last().unwrap() + 0.36 + beat as f64 * 0.0075);
    }
    let sr = 8_000u32;
    let mut data = vec![0.0; ((times[32] + 0.25) * f64::from(sr)).ceil() as usize];
    for &time in &times {
        let start = (time * f64::from(sr)).round() as usize;
        for frame in 0..80 {
            data[start + frame] += 0.3 * (frame as f32 * 0.8).cos() * (-0.07 * frame as f32).exp();
        }
    }
    let mut grid = Grid::new(0.0, 60.0 / 0.36).unwrap();
    for (beat, &time) in times.iter().enumerate().skip(1) {
        grid = grid.with_anchor(beat as f64, time).unwrap();
    }
    (
        Arc::new(crate::engine::dsp::Sample {
            name: "Original ramping click".into(),
            sr,
            ch: 1,
            bpm: 0.0,
            peaks: Arc::new(vec![]),
            path: "fixture:ramping-click".into(),
            data,
        }),
        grid,
        times,
    )
}

/// Render source audio through the real deck and compare its mapped clock.
/// Takes original PCM, a reviewed grid and output rate; checks sync, loop, seek and zero heap work.
fn exercise_audio(
    sample: Arc<crate::engine::dsp::Sample>,
    grid: Grid,
    sr: u32,
) -> serde_json::Value {
    use sha2::{Digest, Sha256};
    let pcm_hash = |audio: &crate::engine::dsp::Sample| {
        let mut hash = Sha256::new();
        for value in &audio.data {
            hash.update(value.to_bits().to_le_bytes());
        }
        format!("{:x}", hash.finalize())
    };
    let before = pcm_hash(&sample);
    let (_engine, mut rt) = Engine::headless_for_test(sr, 128);
    rt.apply(Command::DeckAudio {
        deck: 0,
        audio: sample.clone(),
    });
    rt.decks[0].grid = Some(grid);
    rt.decks[0].gain = 1.0;
    rt.decks[0].sync = true;
    rt.decks[0].sync_bpm = 120.0;
    rt.decks[0].playing = true;
    let source_rate = f64::from(sample.sr);
    let mut max_beat_error = 0.0f64;
    let mut energy = 0.0f64;
    let mut output = vec![0.0; sr as usize * 2];
    let initial = grid.beat_at(0.0).unwrap();
    let mut rendered = 0usize;
    let total = ((grid.beat_at(sample.frames() as f64 / source_rate).unwrap() - 1.0 - initial)
        * f64::from(sr)
        / 2.0) as usize;
    while rendered < total {
        let frames = (total - rendered).min(output.len() / 2);
        let counts = test_alloc::measure(|| {
            for frame in output[..frames * 2].chunks_exact_mut(2) {
                let (left, right) = rt.render_deck(0);
                frame.copy_from_slice(&[left, right]);
            }
        });
        assert_eq!(counts, test_alloc::Counts::default());
        rendered += frames;
        let expected = initial + rendered as f64 * 2.0 / f64::from(sr);
        let actual = grid.beat_at(rt.decks[0].pos / source_rate).unwrap();
        max_beat_error = max_beat_error.max((actual - expected).abs());
        assert!(
            (actual - expected).abs() < 1e-5,
            "{sr}: beat {actual} != {expected}"
        );
        energy += output[..frames * 2]
            .iter()
            .map(|v| f64::from(*v).powi(2))
            .sum::<f64>();
        assert!(output[..frames * 2].iter().all(|v| v.is_finite()));
    }
    assert!(energy > 0.01);
    rt.decks[0].pos = grid.seconds_at(3.0).unwrap() * source_rate;
    rt.apply(Command::DeckLoop {
        deck: 0,
        beats: 6.0,
    });
    assert_eq!(
        rt.decks[0].loop_start,
        grid.seconds_at(3.0).unwrap() * source_rate
    );
    let start = rt.decks[0].loop_start;
    let end = start + rt.decks[0].loop_len;
    assert!((grid.beat_at(end / source_rate).unwrap() - 9.0).abs() < 1e-10);
    let mut loop_energy = 0.0f64;
    let counts = test_alloc::measure(|| {
        for frame in 0..(sr as usize * 7) {
            let (left, right) = rt.render_deck(0);
            loop_energy += f64::from(left * left + right * right);
            assert!(rt.decks[0].pos >= start && rt.decks[0].pos < end);
            if frame % 128 == 0 {
                let expected = 3.0 + ((frame + 1) as f64 * 2.0 / f64::from(sr)).rem_euclid(6.0);
                let actual = grid.beat_at(rt.decks[0].pos / source_rate).unwrap();
                assert!((actual - expected).abs() < 2e-5 || (actual - expected).abs() > 5.99998);
            }
        }
    });
    assert_eq!(counts, test_alloc::Counts::default());
    assert!(loop_energy > 0.01);
    rt.decks[0].loop_on = false;
    for fraction in [0.15f32, 0.47, 0.72] {
        rt.apply(Command::DeckSeek {
            deck: 0,
            frac: fraction,
        });
        let beat = grid.beat_at(rt.decks[0].pos / source_rate).unwrap();
        rt.process(&mut [0.0; 256]);
        let expected = beat + 128.0 * 2.0 / f64::from(sr);
        assert!((grid.beat_at(rt.decks[0].pos / source_rate).unwrap() - expected).abs() < 1e-5);
    }
    assert!(Arc::ptr_eq(&sample, rt.decks[0].audio.as_ref().unwrap()));
    assert_eq!(pcm_hash(rt.decks[0].audio.as_ref().unwrap()), before);
    serde_json::json!({"output_rate": sr, "source_rate": sample.sr, "source_pcm_sha256": before,
        "rendered_frames": rendered, "max_sync_beat_error": max_beat_error, "energy": energy,
        "loop_energy": loop_energy, "loop_frames": sr * 7, "heap_allocations_and_frees": 0})
}

#[test]
fn synced_loop_wrap_preserves_fractional_beats_at_opposing_segment_tempos() {
    let sr = 48_000;
    let (_engine, mut rt) = Engine::headless_for_test(sr, 128);
    let grid = Grid::new(0.0, 400.0)
        .unwrap()
        .with_anchor(4.0, 0.6)
        .unwrap()
        .with_anchor(8.0, 12.6)
        .unwrap();
    let audio = Arc::new(crate::engine::dsp::Sample {
        name: "Extreme segments".into(),
        sr: 8_000,
        ch: 1,
        bpm: 0.0,
        peaks: Arc::new(vec![]),
        path: "fixture:extreme-grid".into(),
        data: vec![0.1; 160_000],
    });
    rt.apply(Command::DeckAudio { deck: 0, audio });
    rt.decks[0].grid = Some(grid);
    rt.decks[0].sync = true;
    rt.decks[0].sync_bpm = 127.0;
    rt.decks[0].playing = true;
    rt.decks[0].pos = grid.seconds_at(3.25).unwrap() * 8_000.0;
    rt.quantize = false;
    rt.apply(Command::DeckLoop {
        deck: 0,
        beats: 6.0,
    });
    let mut max_error = 0.0f64;
    assert_eq!(
        test_alloc::measure(|| {
            for frame in 0..sr as usize * 12 {
                rt.render_deck(0);
                let expected =
                    3.25 + ((frame + 1) as f64 * 127.0 / (60.0 * f64::from(sr))).rem_euclid(6.0);
                let actual = grid.beat_at(rt.decks[0].pos / 8_000.0).unwrap();
                let error = (actual - expected)
                    .abs()
                    .min(6.0 - (actual - expected).abs());
                max_error = max_error.max(error);
                assert!(error < 2e-6, "beat {actual} != {expected}");
            }
        }),
        test_alloc::Counts::default()
    );
    println!("VARIABLE_GRID_WRAP {{\"output_rate\":{sr},\"rendered_frames\":{},\"max_beat_error\":{max_error}}}", sr * 12);
}

#[test]
fn ramping_click_pcm_is_audible_on_each_synced_beat_and_cross_anchor_loop() {
    let (audio, grid, _times) = ramping_click();
    for sr in [44_100, 48_000, 96_000] {
        let report = exercise_audio(audio.clone(), grid, sr);
        let (_engine, mut rt) = Engine::headless_for_test(sr, 128);
        rt.apply(Command::DeckAudio {
            deck: 0,
            audio: audio.clone(),
        });
        rt.decks[0].grid = Some(grid);
        rt.decks[0].gain = 1.0;
        rt.decks[0].sync = true;
        rt.decks[0].sync_bpm = 120.0;
        rt.decks[0].playing = true;
        let mut output = vec![0.0f32; sr as usize * 16];
        for frame in &mut output {
            *frame = rt.render_deck(0).0;
        }
        for beat in 1..32 {
            let expected = sr as usize * beat / 2;
            let before = output[expected - (sr / 100) as usize..expected - (sr / 1000) as usize]
                .iter()
                .map(|v| v.abs())
                .fold(0.0f32, f32::max);
            let peak = output[expected..expected + (sr / 100) as usize]
                .iter()
                .map(|v| v.abs())
                .fold(0.0f32, f32::max);
            assert!(
                peak > 0.1 && before < 1e-5,
                "beat {beat}: peak={peak}, before={before}"
            );
        }
        println!("VARIABLE_GRID_CLICK {report}");
    }
}

#[test]
fn recorded_human_drum_pcm_survives_sync_cross_anchor_loops_and_seeks() {
    use sha2::{Digest, Sha256};
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/beatgrid/11_rock_100_beat_4-4.wav");
    assert_eq!(
        format!("{:x}", Sha256::digest(std::fs::read(&path).unwrap())),
        "6e4f313f3dee5482bdc328fbd54893cc3a0f7fe9e960045b837b123e0a17dd4a"
    );
    let sample = Arc::new(crate::engine::decode::decode_audio(&path).unwrap().sample);
    assert_eq!(
        (sample.sr, sample.ch, sample.frames()),
        (44_100, 2, 423_360)
    );
    let mut grid = Grid::new(0.0, 100.0).unwrap();
    for (beat, seconds) in [
        (4.0, 2.3925),
        (8.0, 4.7925),
        (12.0, 7.20125),
        (15.0, 8.99875),
    ] {
        grid = grid.with_anchor(beat, seconds).unwrap();
    }
    for sr in [44_100, 48_000, 96_000] {
        println!(
            "VARIABLE_GRID_HUMAN_DRUM {}",
            exercise_audio(sample.clone(), grid, sr)
        );
    }
}

#[test]
fn ordered_anchors_roundtrip_pickups_boundaries_and_source_rate_without_heap_work() {
    let grid = Grid::new(0.25, 120.0)
        .unwrap()
        .with_anchor(4.0, 2.25)
        .unwrap()
        .with_anchor(8.0, 3.75)
        .unwrap()
        .with_anchor(12.0, 6.75)
        .unwrap();
    assert_eq!(grid.bpm(), 120.0);
    assert_eq!(grid.bpm_at(2.25), Some(160.0));
    assert_eq!(grid.bpm_at(3.75), Some(80.0));
    for beat in [
        -8.0, -0.5, 0.0, 3.999999, 4.0, 4.000001, 7.5, 8.0, 11.75, 12.0, 40.0,
    ] {
        let seconds = grid.seconds_at(beat).unwrap();
        assert!((grid.beat_at(seconds).unwrap() - beat).abs() < 1e-10);
        for rate in [8000.0, 44100.0, 48000.0, 96000.0] {
            assert!((grid.beat_at(seconds * rate / rate).unwrap() - beat).abs() < 1e-10);
        }
    }
    assert_eq!(Grid::decode(Grid::encode(Some(grid))), Some(Some(grid)));
    assert_eq!(
        serde_json::from_slice::<Grid>(&serde_json::to_vec(&grid).unwrap()).unwrap(),
        grid
    );
    assert_eq!(
        test_alloc::measure(|| {
            for _ in 0..1024 {
                std::hint::black_box(grid.beat_at(3.6));
                std::hint::black_box(grid.seconds_at(8.125));
                std::hint::black_box(Grid::decode(Grid::encode(Some(grid))));
            }
        }),
        test_alloc::Counts::default()
    );
}

#[test]
fn bounded_anchor_editing_rejects_discontinuity_and_corrupt_storage_without_partial_changes() {
    let base = Grid::new(0.0, 120.0)
        .unwrap()
        .with_anchor(4.0, 2.0)
        .unwrap()
        .with_anchor(8.0, 3.5)
        .unwrap();
    for (beat, seconds) in [
        (0.0, 1.0),
        (-1.0, 1.0),
        (6.0, 2.0),
        (6.0, 3.5),
        (6.0, 2.001),
        (6.0, 100.0),
        (f64::NAN, 3.0),
        (6.0, f64::INFINITY),
    ] {
        assert!(base.with_anchor(beat, seconds).is_err());
    }
    let replaced = base.with_anchor(8.0, 4.0).unwrap();
    assert_eq!(replaced.anchors().len(), 2);
    let removed = base.without_anchor(0).unwrap();
    assert_eq!(removed.anchors().len(), 1);
    assert_eq!(removed.seconds_at(8.0), Some(3.5));
    let constant = removed.without_anchor(0).unwrap();
    assert!(constant.anchors().is_empty());
    assert_eq!(constant.seconds_at(8.0), Some(3.5));
    assert!(base.without_anchor(2).is_err());
    let mut full = Grid::new(0.0, 120.0).unwrap();
    for index in 1..=MAX_ANCHORS {
        full = full.with_anchor(index as f64, index as f64 * 0.5).unwrap();
    }
    assert!(full.with_anchor(33.0, 16.5).is_err());
    assert!(full.with_anchor(16.0, 8.01).is_ok());
    let mut value = serde_json::to_value(full).unwrap();
    value["anchors"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({"beat":33,"seconds":16.5}));
    assert!(serde_json::from_value::<Grid>(value).is_err());
    for mutate in [0, 1, 2] {
        let mut words = Grid::encode(Some(base));
        match mutate {
            0 => words[3] = 33,
            1 => words[4] = 0,
            _ => words[WORDS - 1] = 1,
        }
        assert!(Grid::decode(words).is_none());
    }
    let slipped = base.slip(0.125).unwrap();
    assert_eq!(slipped.seconds_at(8.0), Some(3.625));
    let stretched = base.stretch(60.0).unwrap();
    assert_eq!(stretched.seconds_at(8.0), Some(7.0));
    assert_eq!(stretched.anchors().len(), 2);
    assert_eq!(stretched.double_tempo().unwrap(), base);
    let legacy = serde_json::json!({"downbeat_seconds":0.25,"seconds_per_beat":0.5});
    assert_eq!(
        serde_json::from_value::<Grid>(legacy).unwrap(),
        Grid::new(0.25, 120.0).unwrap()
    );
}

#[test]
fn actual_renderer_loops_sync_seek_and_undo_use_local_segments_without_changing_audio() {
    let (engine, mut rt) = Engine::headless_for_test(48000, 256);
    let a = Grid::new(0.0, 120.0)
        .unwrap()
        .with_anchor(4.0, 2.0)
        .unwrap()
        .with_anchor(8.0, 3.5)
        .unwrap();
    let b = Grid::new(0.0, 120.0)
        .unwrap()
        .with_anchor(4.0, 2.0)
        .unwrap()
        .with_anchor(8.0, 5.0)
        .unwrap();
    let audio = rt.decks[0].audio.clone().unwrap();
    for (deck, grid) in [(0, a), (1, b)] {
        let receipt = engine.initial_playback[deck].clone().unwrap();
        rt.apply(Command::DeckGrid {
            deck: deck as u8,
            grid: Some(grid),
            receipt,
            ack: GridEditAck::new(),
        });
    }
    rt.decks[0].pos = a.seconds_at(3.0).unwrap() * 48000.0;
    rt.quantize = true;
    assert_eq!(
        test_alloc::measure(|| rt.apply(Command::DeckLoop {
            deck: 0,
            beats: 4.0
        })),
        test_alloc::Counts::default()
    );
    assert!(
        (rt.decks[0].loop_len / 48000.0
            - (a.seconds_at(7.0).unwrap() - a.seconds_at(3.0).unwrap()))
        .abs()
            < 1e-10
    );
    rt.apply(Command::DeckLoopDouble { deck: 0 });
    assert!(
        (rt.decks[0].loop_len / 48000.0
            - (a.seconds_at(11.0).unwrap() - a.seconds_at(3.0).unwrap()))
        .abs()
            < 1e-10
    );
    rt.apply(Command::DeckLoopHalf { deck: 0 });
    assert!(
        (rt.decks[0].loop_len / 48000.0
            - (a.seconds_at(7.0).unwrap() - a.seconds_at(3.0).unwrap()))
        .abs()
            < 1e-10
    );
    rt.decks[0].loop_on = false;
    rt.decks[0].pos = a.seconds_at(5.25).unwrap() * 48000.0;
    rt.decks[1].pos = b.seconds_at(5.8).unwrap() * 48000.0;
    rt.decks[0].playing = true;
    rt.decks[1].playing = true;
    rt.xfader = 0.25;
    rt.apply(Command::DeckMatch);
    assert!((b.beat_at(rt.decks[1].pos / 48000.0).unwrap() - 5.25).abs() < 1e-10);
    assert_eq!(rt.decks[1].sync_bpm, 160.0);
    rt.process(&mut [0.0; 2]);
    assert!((rt.decks[1].target_rate - 2.0).abs() < 1e-6);
    rt.decks[1].pos = b.seconds_at(2.0).unwrap() * 48000.0;
    rt.process(&mut [0.0; 2]);
    assert!((rt.decks[1].target_rate - 160.0 / 120.0).abs() < 1e-6);
    rt.xfader = 0.75;
    rt.apply(Command::DeckMatch);
    assert_eq!(
        rt.decks[0].sync_bpm, 160.0,
        "An anchored synced favorite retains its playing musical clock"
    );
    rt.apply(Command::DeckSeek { deck: 0, frac: 0.5 });
    assert!(std::sync::Arc::ptr_eq(
        &audio,
        rt.decks[0].audio.as_ref().unwrap()
    ));
    assert!(a.beat_at(rt.decks[0].pos / f64::from(audio.sr)).is_some());
    let changed = a.with_anchor(8.0, 4.0).unwrap();
    let receipt = engine.initial_playback[0].clone().unwrap();
    rt.clear_undo_for_test();
    rt.apply(Command::DeckGrid {
        deck: 0,
        grid: Some(changed),
        receipt,
        ack: GridEditAck::new(),
    });
    assert_eq!(rt.decks[0].grid, Some(changed));
    assert_eq!(
        test_alloc::measure(|| rt.apply(Command::Undo)),
        test_alloc::Counts::default()
    );
    assert_eq!(rt.decks[0].grid, Some(a));
    assert!(std::sync::Arc::ptr_eq(
        &audio,
        rt.decks[0].audio.as_ref().unwrap()
    ));
}
