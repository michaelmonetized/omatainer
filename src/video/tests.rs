use super::*;
#[test]
fn rational_video_audio_boundaries_and_long_duration_timecode_do_not_drift() {
    for rate in [
        Rate {
            numerator: 24,
            denominator: 1,
        },
        Rate {
            numerator: 25,
            denominator: 1,
        },
        Rate {
            numerator: 30,
            denominator: 1,
        },
        Rate {
            numerator: 50,
            denominator: 1,
        },
        Rate {
            numerator: 60,
            denominator: 1,
        },
        Rate {
            numerator: 24000,
            denominator: 1001,
        },
        Rate {
            numerator: 30000,
            denominator: 1001,
        },
        Rate {
            numerator: 60000,
            denominator: 1001,
        },
    ] {
        for sr in [44100, 48000, 96000] {
            for frame in [0, 1, 1798, 17982, 100000, 999999] {
                assert_eq!(rate.frame(rate.seconds(frame)), frame);
                let sample = rate.sample(frame, sr);
                assert!(
                    (sample as f64 / f64::from(sr) - rate.seconds(frame)).abs()
                        <= 0.5 / f64::from(sr) + 1e-9
                );
            }
        }
    }
    let rate = Rate {
        numerator: 30000,
        denominator: 1001,
    };
    assert_eq!(rate.timecode(1800, true).unwrap(), "00:01:00;02");
    assert_eq!(rate.timecode(17982, true).unwrap(), "00:10:00;00");
    assert_eq!(rate.timecode(107892, true).unwrap(), "01:00:00;00");
    assert_eq!(rate.timecode(-1800, true).unwrap(), "-00:01:00;02");
    assert_eq!(
        Rate {
            numerator: 60000,
            denominator: 1001
        }
        .timecode(3600, true)
        .unwrap(),
        "00:01:00;04"
    );
    assert!(Rate {
        numerator: 25,
        denominator: 1
    }
    .timecode(0, true)
    .is_err());
}
