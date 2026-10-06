use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

fn tone(rate: u32, hz: f64, seconds: f64) -> Vec<f32> {
    (0..(f64::from(rate) * seconds) as usize)
        .map(|frame| {
            (std::f64::consts::TAU * hz * frame as f64 / f64::from(rate)).sin() as f32 * 0.8
        })
        .collect()
}

#[test]
fn frequency_colors_follow_hz_at_native_rates_and_survive_stereo_phase_inversion() {
    for rate in [44_100, 48_000, 96_000] {
        for (band, hz) in [40.0, 100.0, 250.0, 700.0, 1600.0, 4000.0, 8500.0, 16000.0]
            .into_iter()
            .enumerate()
        {
            let mono = tone(rate, hz, 0.3);
            let stereo: Vec<_> = mono.iter().flat_map(|value| [*value, -*value]).collect();
            let waveform = Waveform::analyze(&stereo, 2, rate, || false).unwrap();
            let bin = waveform.range(f64::from(rate) * 0.15, f64::from(rate) * 0.25);
            let dominant = bin
                .energy
                .iter()
                .enumerate()
                .max_by(|a, b| a.1.total_cmp(b.1))
                .unwrap()
                .0;
            assert_eq!(dominant, band, "{hz} Hz at {rate} Hz: {:?}", bin.energy);
            let expected = mono.iter().map(|value| value.abs()).fold(0.0f32, f32::max);
            assert!(
                (bin.peak[0] - expected).abs() < 0.001,
                "{hz} Hz at {rate} Hz: {:?} vs {expected}",
                bin.peak
            );
            assert_eq!(bin.peak[0], bin.peak[1]);
        }
    }
}

#[test]
fn narrow_transients_and_asymmetric_channels_survive_every_zoom_level() {
    let mut pcm = vec![0.0; 48_000 * 2];
    let frame = 17_843;
    pcm[frame * 2] = -1.0;
    pcm[frame * 2 + 1] = 0.25;
    let waveform = Waveform::analyze(&pcm, 2, 48_000, || false).unwrap();
    assert_eq!(waveform.hop, 120);
    for width in [1.0, 120.0, 480.0, 4096.0, 48_000.0] {
        assert_eq!(
            waveform.range(frame as f64, frame as f64 + width).peak,
            [1.0, 0.25]
        );
    }
    assert_eq!(waveform.range(0.0, 100.0).peak, [0.0; 2]);
    assert_eq!(waveform.range(-100.0, -1.0).peak, [0.0; 2]);
    assert_eq!(waveform.range(48_000.0, 50_000.0).peak, [0.0; 2]);
}

#[test]
fn analysis_is_cancellable_bounded_and_refuses_nonfinite_sources() {
    let pcm = tone(48_000, 1000.0, 1.0);
    let checks = AtomicUsize::new(0);
    assert!(
        Waveform::analyze(&pcm, 1, 48_000, || checks.fetch_add(1, Ordering::Relaxed)
            >= 5)
        .is_none()
    );
    assert!(checks.load(Ordering::Relaxed) <= 7);
    assert!(Waveform::analyze(&[f32::NAN], 1, 48_000, || false).is_none());
    assert!(Waveform::analyze(&[0.0], 2, 48_000, || false).is_none());
    let waveform = Waveform::analyze(&pcm, 1, 48_000, || false).unwrap();
    assert!(waveform.levels[0].len() <= MAX_BINS);
    assert!(
        waveform.levels.iter().map(Vec::capacity).sum::<usize>()
            <= 2 * waveform.levels[0].len() + waveform.levels.len()
    );
    assert!(waveform.storage_bytes() < 32_000);
    assert_eq!(size_of::<StoredBin>(), 16);
    let maximum = (2 * MAX_BINS + 17) * size_of::<StoredBin>()
        + 32 * size_of::<Vec<StoredBin>>()
        + size_of::<Waveform>()
        + 2 * size_of::<usize>();
    assert!(maximum < 2_100_000);
}

#[test]
fn bands_above_source_nyquist_are_silent() {
    let pcm = tone(8000, 1000.0, 0.2);
    let waveform = Waveform::analyze(&pcm, 1, 8000, || false).unwrap();
    let bin = waveform.range(800.0, 1600.0);
    assert_eq!(bin.energy[6..], [0.0, 0.0]);
}
