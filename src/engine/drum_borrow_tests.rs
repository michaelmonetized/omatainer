use super::*;
use std::hint::black_box;

fn engine() -> RtEngine {
    let (_tx, rx) = crossbeam_channel::bounded(32);
    RtEngine::new(48_000.0, rx, Arc::new(Mutex::new(Snapshot::default())))
}

// The pre-change loop is retained only as a benchmark/output oracle. Its six
// handle clones/drops are exactly the overhead removed from production.
fn previous_tick(rt: &mut RtEngine, track: usize) -> f32 {
    let samples = rt.tracks[track].drum_samples.clone();
    let mut output = 0.0;
    for slot in &mut rt.tracks[track].drum_pos {
        if let Some(voice) = slot {
            let sample = &samples[voice.sample];
            output += sample.at(voice.position).0 * voice.clip_gain * voice.velocity;
            voice.position += sample.sr as f64 / rt.sr as f64;
            if voice.position >= sample.frames() as f64 {
                *slot = None;
            }
        }
    }
    output
}

fn reset(rt: &mut RtEngine, voices: usize) {
    rt.tracks[0].drum_pos = std::array::from_fn(|index| {
        (index < voices).then_some(DrumVoice {
            sample: index % 6,
            position: 0.0,
            velocity: 1.0,
            clip_gain: ((index % 3) + 1) as f32 * 0.5,
        })
    });
}

#[test]
fn borrowed_drum_loop_matches_previous_samples_positions_and_retirement() {
    let mut actual = engine();
    let mut reference = engine();
    reference.tracks[0].drum_samples = actual.tracks[0].drum_samples.clone();
    for count in [0, 1, 6, 16] {
        for sr in [44_100.0, 48_000.0, 96_000.0] {
            actual.sr = sr;
            reference.sr = sr;
            reset(&mut actual, count);
            reset(&mut reference, count);
            let mut energy = 0.0;
            for frame in 0..120_000 {
                let value = actual.tick_drums(0);
                energy += value.abs();
                assert_eq!(
                    value,
                    previous_tick(&mut reference, 0),
                    "voices={count} sr={sr} frame={frame}"
                );
                assert_eq!(actual.tracks[0].drum_pos, reference.tracks[0].drum_pos);
            }
            assert!(actual.tracks[0].drum_pos.iter().all(Option::is_none));
            assert_eq!(energy > 0.0, count > 0);
        }
    }
}

#[test]
fn idle_and_active_drum_rendering_have_no_heap_activity_and_rate_replacement_retires_old_bank() {
    let mut rt = engine();
    // Install a unique private bank so lifetime assertions do not count other
    // tracks sharing factory samples. Runtime rate replacement clears voices.
    rt.tracks[0].drum_samples = std::array::from_fn(|i| {
        Arc::new(Sample {
            name: format!("private-{i}"),
            sr: 48_000,
            ch: 1,
            data: vec![0.25; 1024],
            peaks: vec![].into(),
            bpm: 120.0,
            path: String::new(),
        })
    });
    let old = Arc::downgrade(&rt.tracks[0].drum_samples[0]);
    for count in [0, 16] {
        reset(&mut rt, count);
        let counts = test_alloc::measure(|| {
            for _ in 0..256 {
                black_box(rt.tick_drums(0));
            }
        });
        assert_eq!(counts, test_alloc::Counts::default());
        assert!(rt.tracks[0]
            .drum_samples
            .iter()
            .all(|sample| Arc::strong_count(sample) == 1));
    }
    rt.set_sample_rate(44_100).unwrap();
    assert!(old.upgrade().is_none());
    assert!(rt.tracks[0].drum_pos.iter().all(Option::is_none));
    assert_eq!(rt.tick_drums(0), 0.0);
    rt.trig_drum(0, 36, 1.0);
    assert!(rt.tracks[0].drum_pos[0].is_some());
    let counts = test_alloc::measure(|| {
        for _ in 0..64 {
            black_box(rt.tick_drums(0));
        }
    });
    assert_eq!(counts, test_alloc::Counts::default());
}

#[test]
fn local_drum_loop_benchmark_reports_idle_and_active_elapsed_time() {
    const FRAMES: usize = 100_000;
    let mut old = engine();
    let mut new = engine();
    old.tracks[0].drum_samples = std::array::from_fn(|i| {
        Arc::new(Sample {
            name: format!("bench-{i}"),
            sr: 48_000,
            ch: 1,
            data: vec![0.01 * (i + 1) as f32; FRAMES + 2048],
            peaks: vec![].into(),
            bpm: 120.0,
            path: String::new(),
        })
    });
    new.tracks[0].drum_samples = old.tracks[0].drum_samples.clone();
    for voices in [0, 1, 16] {
        reset(&mut old, voices);
        reset(&mut new, voices);
        for _ in 0..1024 {
            black_box(previous_tick(&mut old, 0));
            black_box(new.tick_drums(0));
        }
        let mut previous = Vec::new();
        let mut borrowed = Vec::new();
        for round in 0..9 {
            for candidate in if round % 2 == 0 {
                [false, true]
            } else {
                [true, false]
            } {
                let rt = if candidate { &mut new } else { &mut old };
                reset(rt, voices);
                let before = Instant::now();
                if candidate {
                    for _ in 0..FRAMES {
                        black_box(rt.tick_drums(0));
                    }
                } else {
                    for _ in 0..FRAMES {
                        black_box(previous_tick(rt, 0));
                    }
                }
                let elapsed = before.elapsed();
                if candidate {
                    borrowed.push(elapsed);
                } else {
                    previous.push(elapsed);
                }
                assert_eq!(
                    rt.tracks[0].drum_pos.iter().flatten().count(),
                    voices,
                    "active benchmark must never become idle"
                );
            }
        }
        previous.sort();
        borrowed.sort();
        eprintln!("drum-loop voices={voices}, frames={FRAMES}, nine alternating runs: previous_median={:?}, borrowed_median={:?}; local headless wall time only", previous[4], borrowed[4]);
    }
}
