use super::*;

#[test]
fn conventional_and_harmonic_labels_roundtrip_all_keys_and_refuse_partial_values() {
    for minor in [false, true] {
        for tonic in 0..12 {
            let key = Key { tonic, minor };
            assert_eq!(Key::parse(&key.conventional()), Some(key));
            assert_eq!(Key::parse(&key.harmonic()), Some(key));
            assert!(key.compatible(key));
            assert!(key.compatible(Key {
                tonic: (tonic + 7) % 12,
                minor
            }));
            assert!(!key.compatible(Key {
                tonic: (tonic + 1) % 12,
                minor
            }));
            let relative = Key {
                tonic: (tonic + if minor { 3 } else { 9 }) % 12,
                minor: !minor,
            };
            assert!(key.compatible(relative) && relative.compatible(key));
        }
    }
    assert_eq!(
        Key::parse("D♭ minor"),
        Some(Key {
            tonic: 1,
            minor: true
        })
    );
    assert_eq!(
        Key::parse("Cb"),
        Some(Key {
            tonic: 11,
            minor: false
        })
    );
    for text in [
        "", "C##", "13A", "0B", "Cmystery", "H", "A major7", "8A blah",
    ] {
        assert_eq!(Key::parse(text), None);
    }
}

#[test]
fn key_analysis_declines_silence_short_noise_and_invalid_samples_and_cancels() {
    for (rate, pcm) in [
        (16000, vec![0.0; 16000 * 8]),
        (48000, vec![0.1; 256]),
        (100, vec![0.5; 1000]),
    ] {
        let result = analyze(&pcm, 1, rate, || false, |_, _| {}).unwrap();
        assert_eq!(result.key, None);
        assert!(result.valid());
    }
    let mut seed = 123456789u32;
    let noise: Vec<_> = (0..16000 * 8)
        .map(|_| {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            (seed as f64 / u32::MAX as f64 - 0.5) as f32
        })
        .collect();
    assert!(analyze(&noise, 1, 16000, || false, |_, _| {})
        .unwrap()
        .key
        .is_none());
    for pcm in [&[f32::NAN][..], &[f32::INFINITY][..], &[][..]] {
        assert!(analyze(pcm, 1, 16000, || false, |_, _| {}).is_err());
    }
    assert!(analyze(&[0.0; 4], 0, 16000, || false, |_, _| {}).is_err());
    assert!(analyze(&noise, 1, 16000, || true, |_, _| {}).is_err());
    let checks = std::cell::Cell::new(0);
    assert!(analyze(
        &noise,
        1,
        16000,
        || {
            checks.set(checks.get() + 1);
            checks.get() > noise.len().div_ceil(4096) + 3
        },
        |_, _| {}
    )
    .is_err());
    assert!(
        Analysis {
            key: Some(Key {
                tonic: 0,
                minor: false
            }),
            ..Analysis::unknown()
        }
        .valid()
            == false
    );
    assert!(!Analysis {
        score: f64::NAN,
        ..Analysis::unknown()
    }
    .valid());
}

#[test]
fn tonal_chords_keep_key_across_native_rates_scaling_and_opposed_stereo_phase() {
    for rate in [16000, 44100, 96000] {
        let mono: Vec<_> = (0..rate * 8)
            .map(|frame| {
                [60.0, 64.0, 67.0]
                    .iter()
                    .map(|note| {
                        let frequency = 440.0 * 2.0f64.powf((note - 69.0) / 12.0);
                        (TAU * frequency * f64::from(frame) / f64::from(rate)).sin() * 0.08
                    })
                    .sum::<f64>() as f32
            })
            .collect();
        let result = analyze(&mono, 1, rate, || false, |_, _| {}).unwrap();
        assert!(result.valid());
        assert!(
            result.key.is_none_or(|key| key
                == Key {
                    tonic: 0,
                    minor: false
                }),
            "{rate}: {result:?}"
        );
        if rate == 16000 {
            assert_eq!(
                result.key,
                Some(Key {
                    tonic: 0,
                    minor: false
                })
            );
        }
        let stereo: Vec<_> = mono.iter().flat_map(|sample| [*sample, -*sample]).collect();
        let phase = analyze(&stereo, 2, rate, || false, |_, _| {}).unwrap();
        assert_eq!(phase.key, result.key);
        assert!((phase.score - result.score).abs() < 1e-9);
        let quiet: Vec<_> = mono.iter().map(|sample| sample * 0.001).collect();
        let quiet = analyze(&quiet, 1, rate, || false, |_, _| {}).unwrap();
        assert_eq!(quiet.key, result.key);
        assert!((quiet.score - result.score).abs() < 1e-6);
    }
}
