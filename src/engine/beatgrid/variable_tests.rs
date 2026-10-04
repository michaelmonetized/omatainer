use super::*;
use crate::engine::{dsp::Sample, test_alloc, Command, Engine};
use std::sync::Arc;

#[test]
fn maximum_anchor_maps_render_full_callbacks_without_heap_work() {
    let (engine, mut rt) = Engine::headless_for_test(48_000, 128);
    for deck in 0..2 {
        let mut grid = Grid::new(0.0, 120.0).unwrap();
        for index in 1..=MAX_ANCHORS {
            grid = grid.set_anchor(index as f64 * 0.005, if (index + deck) % 2 == 0 { 20.0 } else { 400.0 }).unwrap();
        }
        engine.send(Command::DeckGrid { deck: deck as u8, grid: Some(grid), ack: GridEditAck::new(), receipt: engine.initial_playback[deck].clone().unwrap() }).unwrap();
        rt.decks[deck].playing = true;
        rt.decks[deck].sync = true;
        rt.decks[deck].sync_bpm = 120.0;
    }
    let mut output = [0.0; 256];
    let mut elapsed = Vec::with_capacity(512);
    assert_eq!(test_alloc::measure(|| {
        for _ in 0..512 {
            let start = std::time::Instant::now();
            rt.process(&mut output);
            elapsed.push(start.elapsed().as_nanos() as u64);
            assert!(output.iter().all(|sample| sample.is_finite()));
        }
    }), test_alloc::Counts::default());
    for deck in &rt.decks {
        assert_eq!(deck.grid.as_ref().unwrap().anchors().len(), MAX_ANCHORS);
        assert!(deck.pos / deck.audio.as_ref().unwrap().sr as f64 > 0.32);
    }
    elapsed.sort_unstable();
    eprintln!("64 anchors on both decks, 48 kHz, 128-frame callbacks: p50={} ns p99={} ns max={} ns; callback deadline=2666667 ns; no physical device", elapsed[256], elapsed[506], elapsed[511]);
}

#[test]
fn ordered_anchors_are_continuous_invertible_and_survive_bounded_storage() {
    let grid = Grid::new(0.25, 120.0).unwrap().set_anchor(2.25, 90.0).unwrap().set_anchor(5.0, 150.0).unwrap();
    assert_eq!(grid.beat_at(2.25), Some(4.0));
    assert_eq!(grid.beat_at(5.0), Some(8.125));
    for seconds in [-1.0, 0.0, 0.25, 2.25 - 1e-9, 2.25, 2.25 + 1e-9, 4.5, 5.0, 9.5] {
        assert!((grid.seconds_at(grid.beat_at(seconds).unwrap()).unwrap() - seconds).abs() < 1e-10);
    }
    let reordered = Grid::new(0.25, 120.0).unwrap().set_anchor(5.0, 150.0).unwrap().set_anchor(2.25, 90.0).unwrap();
    assert_eq!(reordered, grid);
    assert_eq!(grid.set_anchor(2.25, 100.0).unwrap().anchors().len(), 2);
    let removed = grid.remove_anchor(0).unwrap();
    assert_eq!(removed.beat_at(5.0), Some(9.5));
    assert_eq!(removed.anchors()[0].bpm, 150.0);
    assert_eq!(grid.slip(0.1).unwrap().anchors()[1].seconds, 5.1);
    assert_eq!(grid.half_tempo().unwrap().anchors()[1].bpm, 75.0);
    assert_eq!(grid.double_tempo().unwrap().anchors()[0].bpm, 180.0);
    assert_eq!(serde_json::from_str::<Grid>(&serde_json::to_string(&grid).unwrap()).unwrap(), grid);
    assert_eq!(Grid::decode(Grid::encode(Some(grid))), Some(Some(grid)));
    let legacy: Grid = serde_json::from_str(r#"{"downbeat_seconds":0.25,"seconds_per_beat":0.5}"#).unwrap();
    assert!(legacy.anchors().is_empty());
    assert_eq!(legacy.seconds_at(8.0), Some(4.25));
    assert_eq!(test_alloc::measure(|| {
        for i in 0..1024 {
            let mapped = grid.set_anchor(3.0, 110.0).unwrap().remove_anchor(1).unwrap();
            std::hint::black_box(Grid::decode(Grid::encode(Some(mapped))));
            std::hint::black_box(grid.seconds_at(grid.beat_at(i as f64 / 31.0).unwrap()));
        }
    }), test_alloc::Counts::default());
}

#[test]
fn recorded_human_drum_performance_maps_actual_pcm_through_sync_seek_and_loop() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/beatgrid");
    let source: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("sources.json")).unwrap()).unwrap();
    let beats: Vec<f64> = source["selected_beats"].as_array().unwrap().iter().map(|beat| beat["seconds"].as_f64().unwrap()).collect();
    let audio = Arc::new(crate::engine::decode::decode_audio(&root.join("35_rock_92_beat_4-4.wav")).unwrap().sample);
    let mut grid = Grid::new(beats[0], 60.0 / (beats[1] - beats[0])).unwrap();
    for index in 1..beats.len() - 1 { grid = grid.set_anchor(beats[index], 60.0 / (beats[index + 1] - beats[index])).unwrap(); }
    for (index, seconds) in beats.iter().enumerate() {
        assert!((grid.beat_at(*seconds).unwrap() - index as f64).abs() < 1e-10);
    }
    for output_sr in [44_100, 48_000, 96_000] {
        let (_engine, mut rt) = Engine::headless_for_test(output_sr, 128);
        rt.decks[0].audio = Some(audio.clone());
        rt.decks[0].grid = Some(grid);
        rt.decks[0].sync = true;
        rt.decks[0].sync_bpm = 120.0;
        rt.decks[0].playing = true;
        let mut peaks = [0.0f32; 12];
        let counts = test_alloc::measure(|| {
            for frame in 0..output_sr as usize * 6 {
                let (left, right) = rt.render_deck(0);
                let expected = (frame + 1) as f64 * 2.0 / output_sr as f64;
                let actual = grid.beat_at(rt.decks[0].pos / audio.sr as f64).unwrap();
                assert!((actual - expected).abs() < 1e-7);
                let index = expected.round() as usize;
                if index > 0 && index < peaks.len() && (expected - index as f64).abs() < 0.035 {
                    peaks[index] = peaks[index].max(left.abs().max(right.abs()));
                }
            }
        });
        assert_eq!(counts, test_alloc::Counts::default());
        assert!(peaks[1..].iter().all(|peak| *peak > 0.02), "recorded drum transients {peaks:?}");
        let cue = beats[9] * audio.sr as f64;
        rt.decks[0].pos = cue;
        rt.apply(Command::DeckHotCue { deck: 0, pad: 0, del: false });
        assert_eq!(rt.decks[0].hotcues[0].pos, cue);
        rt.apply(Command::DeckSeek { deck: 0, frac: (beats[3] * audio.sr as f64 / audio.frames() as f64) as f32 });
        rt.quantize = true;
        rt.apply(Command::DeckLoop { deck: 0, beats: 4.0 });
        assert!((rt.decks[0].loop_len / audio.sr as f64 - (beats[7] - beats[3])).abs() < 1e-9);
        for frame in 0..output_sr as usize * 5 {
            rt.render_deck(0);
            let expected = 3.0 + (((frame + 1) as f64 * 2.0 / output_sr as f64) % 4.0);
            let actual = grid.beat_at(rt.decks[0].pos / audio.sr as f64).unwrap();
            let error = (actual - expected).abs();
            assert!(error.min((error - 4.0).abs()) < 1e-5, "recorded drum loop {actual}/{expected}");
        }
        assert!(Arc::ptr_eq(rt.decks[0].audio.as_ref().unwrap(), &audio));
        assert_eq!(rt.decks[0].hotcues[0].pos, cue);
    }
}

#[test]
fn anchor_capacity_corruption_and_invalid_continuity_are_refused() {
    let mut grid = Grid::new(0.0, 120.0).unwrap();
    for index in 1..=MAX_ANCHORS { grid = grid.set_anchor(index as f64, 100.0 + index as f64).unwrap(); }
    assert!(grid.set_anchor(65.0, 120.0).is_err());
    assert!(grid.set_anchor(64.0, 125.0).is_ok());
    assert!(grid.remove_anchor(MAX_ANCHORS).is_err());
    for (seconds, bpm) in [(0.0, 120.0), (-0.1, 120.0), (f64::NAN, 120.0), (1.0, f64::INFINITY), (1.0, 19.0), (1.0, 401.0)] {
        assert!(grid.set_anchor(seconds, bpm).is_err());
    }
    let mut json = serde_json::to_value(grid).unwrap();
    json["anchors"][1]["seconds"] = json["anchors"][0]["seconds"].clone();
    assert!(serde_json::from_value::<Grid>(json).is_err());
    let mut json = serde_json::to_value(grid).unwrap();
    json["anchors"].as_array_mut().unwrap().push(serde_json::json!({"seconds":65.0,"bpm":120.0}));
    assert!(serde_json::from_value::<Grid>(json).is_err());
    let one = Grid::new(0.0, 120.0).unwrap().set_anchor(1.0, 300.0).unwrap();
    assert!(one.double_tempo().is_err());
    let mut words = Grid::encode(Some(one)); words[6] = 1;
    assert!(Grid::decode(words).is_none());
    assert!(Grid::decode([]).is_none());
    let mut words = Grid::encode(Some(grid)); words[3] = 65;
    assert!(Grid::decode(words).is_none());
    let distant = Grid::new(-1e10, 400.0).unwrap().set_anchor(1.0, 120.0).unwrap();
    assert!(distant.set_anchor(f64::from_bits(1.0f64.to_bits() + 1), 120.0).is_err());
}

#[test]
fn match_keeps_two_deck_beat_phase_through_local_tempo_changes_and_loop_wraps() {
    let (_engine, mut rt) = Engine::headless_for_test(48_000, 128);
    let (audio, oracle, grid) = fixture(44_100, true);
    let uniform = Arc::new(Sample { name: "Constant clock reference".into(), sr: 48_000, ch: 1, data: vec![0.0; 48_000 * 10], peaks: Arc::new(vec![]), bpm: 120.0, path: String::new() });
    rt.decks[0].audio = Some(uniform);
    rt.decks[0].grid = Some(Grid::new(0.0, 120.0).unwrap());
    rt.decks[0].pos = 2.3 * 24_000.0;
    rt.decks[0].playing = true;
    rt.decks[1].audio = Some(audio);
    rt.decks[1].grid = Some(grid);
    rt.decks[1].pos = grid.seconds_at(3.8).unwrap() * 44_100.0;
    rt.decks[1].playing = true;
    rt.xfader = 0.25;
    rt.apply(Command::DeckMatch);
    assert!((grid.beat_at(rt.decks[1].pos / 44_100.0).unwrap() - 3.3).abs() < 1e-10);
    rt.decks[1].loop_on = true;
    rt.decks[1].loop_start = oracle[3] * 44_100.0;
    rt.decks[1].loop_len = (oracle[7] - oracle[3]) * 44_100.0;
    let mut error = 0.0f64;
    assert_eq!(test_alloc::measure(|| {
        for _ in 0..48_000 * 5 {
            rt.render_deck(0); rt.render_deck(1);
            let a = rt.decks[0].pos / 24_000.0;
            let b = grid.beat_at(rt.decks[1].pos / 44_100.0).unwrap();
            let phase = (a - b).rem_euclid(1.0);
            error = error.max(phase.min(1.0 - phase));
        }
    }), test_alloc::Counts::default());
    assert!(error < 1e-7, "paired deck phase drift: {error}");
}

/// Build known tempo drift with independently timed source transients.
/// Takes source rate and a drum-timing flag; returns immutable PCM, its beat-time oracle and a manual map.
fn fixture(sr: u32, drums: bool) -> (Arc<Sample>, Vec<f64>, Grid) {
    let mut seconds = vec![0.25];
    let mut periods = Vec::new();
    for beat in 0..16 {
        let period = if drums { [0.49, 0.512, 0.503, 0.478, 0.461, 0.477, 0.454, 0.469][beat % 8] } else { 0.55 - beat as f64 * 0.009 };
        periods.push(period);
        seconds.push(seconds[beat] + period);
    }
    let mut grid = Grid::new(seconds[0], 60.0 / periods[0]).unwrap();
    for beat in 1..periods.len() { grid = grid.set_anchor(seconds[beat], 60.0 / periods[beat]).unwrap(); }
    let mut data = vec![0.0; ((seconds.last().unwrap() + 1.0) * sr as f64).ceil() as usize];
    for (beat, seconds) in seconds.iter().enumerate() {
        let start = (seconds * sr as f64).round() as usize;
        for offset in 0..(sr / 200) as usize {
            let envelope = 1.0 - offset as f32 / (sr / 200) as f32;
            data[start + offset] = if drums {
                let tone = if beat % 2 == 0 { 70.0 } else { 190.0 };
                envelope * ((offset as f32 * tone * std::f32::consts::TAU / sr as f32).cos() * 0.5 + 0.5)
            } else { envelope };
        }
    }
    (Arc::new(Sample { name: if drums { "Humanized drum timing" } else { "Ramping clicks" }.into(), sr, ch: 1, data, peaks: Arc::new(vec![]), bpm: 120.0, path: String::new() }), seconds, grid)
}

#[test]
fn rendered_ramping_clicks_and_humanized_drums_keep_sync_through_seeks_and_loops() {
    for (source_sr, output_sr) in [(44_100, 48_000), (48_000, 44_100), (96_000, 48_000)] {
        for drums in [false, true] {
            let (_engine, mut rt) = Engine::headless_for_test(output_sr, 128);
            let (audio, oracle, grid) = fixture(source_sr, drums);
            let original = audio.clone();
            rt.decks[0].audio = Some(audio);
            rt.decks[0].grid = Some(grid);
            rt.decks[0].playing = true;
            rt.decks[0].sync = true;
            rt.decks[0].sync_bpm = 120.0;
            rt.decks[0].pos = oracle[0] * source_sr as f64;
            let mut maximum_beat_error = 0.0f64;
            let mut peaks = [0.0f32; 12];
            let counts = test_alloc::measure(|| {
                for frame in 0..output_sr as usize * 6 {
                    let (left, _) = rt.render_deck(0);
                    let expected = (frame + 1) as f64 * 2.0 / output_sr as f64;
                    let actual = grid.beat_at(rt.decks[0].pos / source_sr as f64).unwrap();
                    maximum_beat_error = maximum_beat_error.max((actual - expected).abs());
                    let beat = expected.round() as usize;
                    if beat > 0 && beat < peaks.len() && (expected - beat as f64).abs() < 0.004 { peaks[beat] = peaks[beat].max(left.abs()); }
                }
            });
            assert_eq!(counts, test_alloc::Counts::default());
            assert!(maximum_beat_error < 1e-7, "{source_sr}/{output_sr}: {maximum_beat_error}");
            assert!(peaks[1..].iter().all(|peak| *peak > 0.1), "rendered source beat peaks: {peaks:?}");
            assert!(Arc::ptr_eq(rt.decks[0].audio.as_ref().unwrap(), &original));
            rt.apply(Command::DeckSeek { deck: 0, frac: (oracle[3] * source_sr as f64 / original.frames() as f64) as f32 });
            rt.quantize = true;
            rt.apply(Command::DeckLoop { deck: 0, beats: 4.0 });
            assert!((rt.decks[0].loop_start / source_sr as f64 - oracle[3]).abs() < 1e-8);
            assert!((rt.decks[0].loop_len / source_sr as f64 - (oracle[7] - oracle[3])).abs() < 1e-8);
            for frame in 0..output_sr as usize * 5 {
                rt.render_deck(0);
                let expected = 3.0 + (((frame + 1) as f64 * 2.0 / output_sr as f64) % 4.0);
                let actual = grid.beat_at(rt.decks[0].pos / source_sr as f64).unwrap();
                let error = (actual - expected).abs();
                assert!(error.min((error - 4.0).abs()) < 1e-7, "loop phase {actual}/{expected}");
            }
            rt.apply(Command::DeckLoopDouble { deck: 0 });
            assert!((rt.decks[0].loop_len / source_sr as f64 - (oracle[11] - oracle[3])).abs() < 1e-8);
            rt.apply(Command::DeckLoopHalf { deck: 0 });
            assert!((rt.decks[0].loop_len / source_sr as f64 - (oracle[7] - oracle[3])).abs() < 1e-8);
        }
    }
}
