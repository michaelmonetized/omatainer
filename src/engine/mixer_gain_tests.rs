use super::*;
use mixer_gain::{crossfader_gains, pan_gains, GainPair};
use std::hint::black_box;

fn fixture(count: usize, legacy: bool) -> RtEngine {
    let (_tx, rx) = crossbeam_channel::bounded(32);
    let mut rt = RtEngine::new(48_000.0, rx, Arc::new(Mutex::new(Snapshot::default())));
    rt.apply(Command::Stop);
    rt.legacy_gain_math = legacy;
    rt.tracks.truncate(count);
    for (index, track) in rt.tracks.iter_mut().enumerate() {
        track.kind = 2;
        track.gain = 0.4 + index as f32 * 0.06;
        track.pan = -1.0 + index as f32 * 2.0 / 7.0;
        track.fx.slots.clear();
        track.poly = Poly::new(rt.sr, SynthInstrument::Keys, 8);
        track.poly.note_on(48 + index as u8, 0.4);
        track.clips = std::array::from_fn(|_| Clip::empty());
    }
    for chain in &mut rt.scene_fx {
        chain.slots.clear();
    }
    rt.fx_wet = [0.0; 3];
    rt.xfader = 0.27;
    rt.xfader_curve = 0.35;
    rt.master = 0.8;
    rt.cue_mix = 0.0;
    // Long constant decks keep the benchmark active without filesystem/audio IO.
    for (index, deck) in rt.decks.iter_mut().enumerate() {
        deck.audio = Some(Arc::new(Sample {
            name: "gain reference".into(),
            sr: 48_000,
            ch: 2,
            data: vec![0.025 * (index + 1) as f32; 500_000],
            peaks: vec![],
            bpm: 120.0,
            path: String::new(),
        }));
        deck.playing = true;
        deck.sync = false;
        deck.keylock = false;
        deck.gain = 1.0;
        deck.pos = 0.0;
    }
    rt
}

#[test]
fn exact_static_curves_are_cached_for_crossfader_and_track_pan() {
    for curve in [0.0, 0.35, 1.0] {
        for step in 0..=256 {
            let x = step as f32 / 256.0;
            let mut cache = GainPair::default();
            cache.prepare([x, curve], 48_000.0, crossfader_gains);
            let expected = xfader_gains(x, curve);
            for _ in 0..4 {
                assert_eq!(
                    cache.tick().map(f32::to_bits),
                    [expected.0, expected.1].map(f32::to_bits)
                );
            }
            cache.prepare([x, curve], 48_000.0, crossfader_gains);
            assert_eq!(cache.calculations, 1);
        }
    }
    for gain in [0.0, 0.125, 0.5, 0.8, 1.0, 1.5] {
        for step in 0..=256 {
            let pan = step as f32 / 128.0 - 1.0;
            let mut cache = GainPair::default();
            cache.prepare([gain, pan], 48_000.0, pan_gains);
            assert_eq!(
                cache.tick().map(f32::to_bits),
                [
                    (1.0 - pan.max(0.0)).sqrt() * gain,
                    (1.0 + pan.min(0.0)).sqrt() * gain
                ]
                .map(f32::to_bits)
            );
        }
    }
}

#[test]
fn finite_ramps_retarget_continuously_and_reach_exact_curve_endpoints_at_all_rates() {
    for sr in [44_100.0, 48_000.0, 96_000.0] {
        for (curve, start, target, reverse) in [
            (
                pan_gains as fn(f32, f32) -> [f32; 2],
                [0.8, -1.0],
                [1.2, 1.0],
                [0.0, 0.0],
            ),
            (
                crossfader_gains as fn(f32, f32) -> [f32; 2],
                [0.0, 0.35],
                [1.0, 0.35],
                [0.25, 1.0],
            ),
        ] {
            let mut cache = GainPair::default();
            cache.prepare(start, sr, curve);
            let start_gain = cache.tick();
            cache.prepare(target, sr, curve);
            let frames = (sr * 0.005).round() as usize;
            let target_gain = curve(target[0], target[1]);
            for frame in 1..=17 {
                let actual = cache.tick();
                for c in 0..2 {
                    let expected = start_gain[c]
                        + (target_gain[c] - start_gain[c]) * frame as f32 / frames as f32;
                    assert!((actual[c] - expected).abs() < 2e-6);
                }
            }
            let before = cache.tick();
            cache.prepare(reverse, sr, curve);
            let end = curve(reverse[0], reverse[1]);
            let mut previous = before;
            for frame in 1..=frames {
                // Repeated block preparation must not restart a ramp.
                if frame % 7 == 0 {
                    cache.prepare(reverse, sr, curve);
                }
                let actual = cache.tick();
                for c in 0..2 {
                    assert!(
                        (actual[c] - previous[c]).abs()
                            <= (end[c] - before[c]).abs() / frames as f32 + 1e-5
                    );
                    assert!(
                        actual[c] >= before[c].min(end[c]) - 1e-5
                            && actual[c] <= before[c].max(end[c]) + 1e-5
                    );
                }
                previous = actual;
            }
            assert_eq!(previous.map(f32::to_bits), end.map(f32::to_bits));
            assert_eq!(cache.calculations, 3);
        }
    }
}

#[test]
fn static_full_renderer_matches_previous_gain_math_at_fixed_track_counts() {
    for count in [1, 4, 8] {
        let mut cached = fixture(count, false);
        let mut old = fixture(count, true);
        let mut a = [0.0; 256];
        let mut b = [0.0; 256];
        let mut energy = 0.0;
        for _ in 0..64 {
            cached.process(&mut a);
            old.process(&mut b);
            assert_eq!(a, b, "tracks={count}");
            energy += a.iter().map(|sample| sample.abs()).sum::<f32>();
        }
        assert!(energy > 1.0);
        assert_eq!(old.xfader_gain.calculations, 0);
        assert!(old
            .tracks
            .iter()
            .all(|track| track.mixer_gain.calculations == 0));
        assert_eq!(cached.xfader_gain.calculations, 1);
        assert!(cached
            .tracks
            .iter()
            .all(|track| track.mixer_gain.calculations == 1));
    }
}

#[test]
fn control_sweeps_are_block_partition_invariant_and_warmed_rendering_stays_allocation_free() {
    let mut whole = fixture(8, false);
    let mut split = fixture(8, false);
    let mut output = [0.0; 512];
    let mut divided = [0.0; 512];
    for index in 0..32 {
        let x = if index % 2 == 0 { 0.0 } else { 1.0 };
        for rt in [&mut whole, &mut split] {
            rt.apply(Command::Xfader(x));
            rt.apply(Command::TrackPan { track: 0, value: x });
            rt.apply(Command::TrackGain {
                track: 0,
                value: 0.2 + x,
            });
        }
        whole.process(&mut output);
        let mut start = 0;
        for frames in [1, 7, 17, 31, 64, 136] {
            split.process(&mut divided[start..start + frames * 2]);
            start += frames * 2;
        }
        assert_eq!(output, divided, "sweep block={index}");
        assert!(output.iter().all(|sample| sample.is_finite()));
    }
    let allocations = test_alloc::measure(|| {
        for index in 0..16 {
            whole.apply(Command::Xfader(index as f32 / 15.0));
            whole.apply(Command::TrackPan {
                track: 0,
                value: index as f32 / 15.0,
            });
            whole.process(&mut output);
        }
    });
    assert_eq!(allocations, test_alloc::Counts::default());
    // Stopped rate reset discards an old-rate transition and establishes the
    // exact current controls at the first new-rate callback.
    whole.set_sample_rate(96_000);
    whole.process(&mut output[..2]);
    assert_eq!(
        whole.xfader_gain.tick(),
        crossfader_gains(whole.xfader, whole.xfader_curve)
    );
    assert_eq!(
        whole.tracks[0].mixer_gain.tick(),
        pan_gains(whole.tracks[0].gain, whole.tracks[0].pan)
    );
}

#[test]
fn local_full_mixer_benchmark_reports_fixed_track_counts() {
    const BLOCKS: usize = 256;
    for tracks in [1, 4, 8] {
        let mut old = fixture(tracks, true);
        let mut cached = fixture(tracks, false);
        let mut samples = [0.0; 256];
        for _ in 0..8 {
            old.process(&mut samples);
            cached.process(&mut samples);
        }
        let mut previous = Vec::new();
        let mut current = Vec::new();
        for round in 0..9 {
            for candidate in if round % 2 == 0 {
                [false, true]
            } else {
                [true, false]
            } {
                let rt = if candidate { &mut cached } else { &mut old };
                for deck in &mut rt.decks {
                    deck.pos = 0.0;
                    deck.playing = true;
                }
                let started = Instant::now();
                for _ in 0..BLOCKS {
                    rt.process(black_box(&mut samples));
                    black_box(&samples);
                }
                let elapsed = started.elapsed();
                if candidate {
                    current.push(elapsed);
                } else {
                    previous.push(elapsed);
                }
                assert!(samples.iter().any(|sample| sample.abs() > 1e-5));
            }
        }
        previous.sort();
        current.sort();
        eprintln!("full-mixer tracks={tracks}, active decks=2, frames={}, nine alternating runs: old_median={:?}, cached_median={:?}; headless elapsed only",BLOCKS*128,previous[4],current[4]);
    }
}
