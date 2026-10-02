//! Native timing options and analytic integration of a BPM-linear segment.
//! Beats are quarter-note coordinates, independent of the displayed meter.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Settings {
    /// Quarter notes before the first complete bar. Zero means no pickup.
    pub pickup: f64,
    /// Equal subdivisions of a notated meter beat: 1, 2 or 4.
    pub subdivision: u8,
    /// Complete bars at the effective starting meter, before transport advances.
    pub count_in: u8,
    pub accent_gain: f32,
    pub beat_gain: f32,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            pickup: 0.0,
            subdivision: 1,
            count_in: 0,
            accent_gain: 1.0,
            beat_gain: 1.0,
        }
    }
}
impl Settings {
    pub fn validate(self, first_bar: f64, first_change: Option<f64>) -> Result<(), String> {
        if !self.pickup.is_finite()
            || self.pickup < 0.0
            || self.pickup >= first_bar
            || first_change.is_some_and(|beat| self.pickup > beat)
            || ![1, 2, 4].contains(&self.subdivision)
            || self.count_in > 4
            || !self.accent_gain.is_finite()
            || !(0.0..=2.0).contains(&self.accent_gain)
            || !self.beat_gain.is_finite()
            || !(0.0..=2.0).contains(&self.beat_gain)
        {
            return Err("Timing requires a pickup shorter than the first bar and before its first meter change, 1/2/4 subdivisions, 0–4 count-in bars and finite click gains 0–2".into());
        }
        Ok(())
    }
}

/// Integrate 60 / BPM over a segment linear in quarter-note position.
pub(super) fn seconds(offset: f64, length: f64, start_bpm: f64, end_bpm: f64) -> f64 {
    let slope = (end_bpm - start_bpm) / length;
    if slope == 0.0 {
        return offset * 60.0 / start_bpm;
    }
    // log1p retains accuracy for nearly horizontal ramps and tiny sample steps.
    60.0 / slope * (slope * offset / start_bpm).ln_1p()
}

pub(super) fn beats(seconds: f64, length: f64, start_bpm: f64, end_bpm: f64) -> f64 {
    let slope = (end_bpm - start_bpm) / length;
    if slope == 0.0 {
        return seconds * start_bpm / 60.0;
    }
    start_bpm / slope * (slope * seconds / 60.0).exp_m1()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::midi_data::{Conductor, Meter, Tempo};
    use std::sync::Arc;

    fn piece(pickup: f64, subdivision: u8) -> Arc<Conductor> {
        Conductor {
            ppqn: 960,
            tempos: vec![
                Tempo::new(0, 120.0, true).unwrap(),
                Tempo::new(8160, 180.0, false).unwrap(),
            ],
            meters: vec![
                Meter {
                    tick: 0,
                    numerator: 7,
                    denominator_power: 3,
                    clocks: 12,
                    thirty_seconds: 8,
                },
                Meter {
                    tick: 3360,
                    numerator: 5,
                    denominator_power: 2,
                    clocks: 24,
                    thirty_seconds: 8,
                },
                Meter {
                    tick: 8160,
                    numerator: 4,
                    denominator_power: 2,
                    clocks: 24,
                    thirty_seconds: 8,
                },
            ],
            native: Some(Settings {
                pickup,
                subdivision,
                ..Settings::default()
            }),
        }
        .prepare()
        .unwrap()
    }

    // An independent numerical oracle integrates the reciprocal tempo using
    // Simpson's rule; it never calls the analytic inverse or its logarithm.
    fn oracle(offset: f64, length: f64, a: f64, b: f64) -> f64 {
        let n = 20_000;
        let h = offset / f64::from(n);
        let f = |x: f64| 60.0 / (a + (b - a) * x / length);
        let mut sum = f(0.0) + f(offset);
        for i in 1..n {
            sum += f(h * f64::from(i)) * if i % 2 == 0 { 2.0 } else { 4.0 };
        }
        sum * h / 3.0
    }

    #[test]
    fn increasing_decreasing_flat_and_nearly_flat_ramps_match_independent_timestamps() {
        for (a, b) in [
            (40.0, 240.0),
            (240.0, 40.0),
            (120.0, 180.0),
            (120.0, 120.0),
            (120.0, 120.00000001),
        ] {
            for offset in [0.0, 0.000001, 0.125, 3.5, 8.5] {
                let expected = oracle(offset, 8.5, a, b);
                let got = seconds(offset, 8.5, a, b);
                assert!(
                    (got - expected).abs() < 1e-10,
                    "{a}->{b} at{offset}: {got} vs{expected}"
                );
                assert!((beats(got, 8.5, a, b) - offset).abs() < 1e-10);
            }
        }
    }

    #[test]
    fn invalid_pickup_count_in_subdivisions_and_gains_refuse() {
        let good = Settings {
            pickup: 0.5,
            subdivision: 2,
            count_in: 2,
            ..Settings::default()
        };
        good.validate(3.5, Some(3.5)).unwrap();
        for bad in [
            Settings {
                pickup: f64::NAN,
                ..good
            },
            Settings {
                pickup: -0.5,
                ..good
            },
            Settings {
                pickup: 3.5,
                ..good
            },
            Settings {
                subdivision: 3,
                ..good
            },
            Settings {
                count_in: 5,
                ..good
            },
            Settings {
                accent_gain: f32::INFINITY,
                ..good
            },
            Settings {
                beat_gain: -0.1,
                ..good
            },
        ] {
            assert!(bad.validate(3.5, Some(3.5)).is_err());
        }
        assert!(good.validate(3.5, Some(0.25)).is_err());
    }

    #[test]
    fn shared_conductor_reopens_ramps_and_labels_pickup_odd_and_fragmentary_bars() {
        let map = piece(0.5, 2);
        let value = serde_json::to_vec(&*map).unwrap();
        let reopened: Conductor = serde_json::from_slice(&value).unwrap();
        let reopened = reopened.prepare().unwrap();
        assert_eq!(map, reopened);
        for (beat, bar, within, numerator, denominator) in [
            (0.0, 0, 6.0, 7, 3),
            (0.25, 0, 6.5, 7, 3),
            (0.5, 1, 0.0, 7, 3),
            (3.0, 1, 5.0, 7, 3),
            (3.5, 2, 0.0, 5, 2),
            (8.0, 2, 4.5, 5, 2),
            (8.5, 3, 0.0, 4, 2),
            (12.5, 4, 0.0, 4, 2),
        ] {
            let (got_bar, got_within, meter) = reopened.position(beat);
            assert_eq!(
                (
                    got_bar,
                    got_within,
                    meter.numerator,
                    meter.denominator_power
                ),
                (bar, within, numerator, denominator)
            );
        }
        let a = 60000000.0 / f64::from(map.tempos[0].micros);
        let b = 60000000.0 / f64::from(map.tempos[1].micros);
        for beat in [0.0_f64, 0.5, 3.5, 5.0, 8.5, 12.5, 65536.0] {
            let expected = oracle(beat.min(8.5), 8.5, a, b) + (beat - 8.5).max(0.0) * 60.0 / b;
            assert!((reopened.seconds_at(beat) - expected).abs() < 1e-10);
            assert!((reopened.beat_at_seconds(expected) - beat).abs() < 1e-9);
            for sr in [44100, 48000, 96000] {
                assert_eq!(
                    reopened.sample_at(beat, sr).unwrap(),
                    (expected * f64::from(sr)).round() as u64
                );
            }
        }
    }

    #[test]
    fn subdivision_clicks_follow_pickup_and_meter_changes_without_duplicate_boundaries() {
        let map = piece(0.5, 2);
        for (beat, accent) in [
            (0.0, false),
            (0.25, false),
            (0.5, true),
            (0.75, false),
            (1.0, false),
            (3.5, true),
            (4.0, false),
            (8.5, true),
            (9.0, false),
            (12.5, true),
        ] {
            assert_eq!(
                map.click_between(beat, beat + 0.0001),
                Some(accent),
                "beat{beat}"
            );
            assert_eq!(
                map.click_between(beat - 0.0001, beat),
                None,
                "half-open{beat}"
            );
        }
        assert_eq!(map.click_between(0.1, 0.2), None);
        let mut invalid = (*map).clone();
        invalid.tempos.last_mut().unwrap().ramp = true;
        assert!(invalid.prepare().is_err());
        let mut invalid = (*map).clone();
        invalid.native = None;
        assert!(invalid.prepare().is_err());
        let legacy = Conductor::from_meta(960, std::iter::empty()).unwrap();
        let wire = serde_json::to_value(&*legacy).unwrap();
        assert!(wire.get("native").is_none());
        assert!(wire["tempos"][0].get("ramp").is_none());
    }

    #[test]
    fn actual_midi_playback_and_recording_clock_share_the_ramp_timestamp_oracle_without_heap() {
        use crate::engine::{midi_edit::Region, test_alloc, Command, Engine, MidiNote};
        let map = piece(0.0, 1);
        let a = 60000000.0 / f64::from(map.tempos[0].micros);
        let b = 60000000.0 / f64::from(map.tempos[1].micros);
        let timestamp =
            |beat: f64| oracle(beat.min(8.5), 8.5, a, b) + (beat - 8.5).max(0.0) * 60.0 / b;
        for sr in [44100, 48000, 96000] {
            let (engine, mut rt) = Engine::headless_for_test(sr, 256);
            rt.conductor = Some(map.clone());
            rt.quant = 0.0;
            rt.tracks[2].clips[7].region = Some(Region::full(16.0));
            let starts = [0.0, 0.5, 3.5, 5.0, 8.5, 12.5];
            let notes = starts
                .iter()
                .enumerate()
                .map(|(i, &beat)| MidiNote {
                    id: crate::engine::midi_edit::NoteId::new(),
                    muted: false,
                    pitch: 60 + i as u8,
                    vel: 100,
                    channel: 0,
                    release_vel: 64,
                    source_timing: None,
                    start: beat as f32,
                    len: 0.125,
                })
                .collect();
            engine
                .send(Command::SetNotes {
                    track: 2,
                    scene: 7,
                    notes,
                })
                .unwrap();
            rt.process(&mut []);
            rt.tracks[2].midi_schedule.sample_trace = Some(Vec::with_capacity(64));
            engine
                .send(Command::FireClip {
                    track: 2,
                    scene: 7,
                    looping: false,
                })
                .unwrap();
            rt.process(&mut []);
            let mut out = vec![0.0; 514];
            let mut rendered = 0;
            let end = (timestamp(13.0) * f64::from(sr)).ceil() as usize;
            let before = rt.note_recording.clock;
            let counts = test_alloc::measure(|| {
                while rendered < end {
                    let frames = (end - rendered).min(257);
                    rt.process(&mut out[..frames * 2]);
                    rendered += frames;
                }
            });
            assert_eq!(counts, test_alloc::Counts::default());
            assert!(out.iter().all(|x| x.is_finite()));
            assert!((rt.note_recording.clock - before - rt.precise_midi_beat()).abs() < 1e-8);
            let trace = rt.tracks[2].midi_schedule.sample_trace.take().unwrap();
            assert_eq!(trace.len(), starts.len() * 2);
            for (i, &start) in starts.iter().enumerate() {
                for (index, beat, on) in [(i * 2, start, true), (i * 2 + 1, start + 0.125, false)] {
                    let expected = (timestamp(beat) * f64::from(sr)).floor() as u64;
                    let (frame, gate) = trace[index];
                    let (got_on, pitch) = match gate {
                        crate::engine::midi_schedule::Gate::On(p, _) => (true, p),
                        crate::engine::midi_schedule::Gate::Off(p) => (false, p),
                    };
                    assert!(
                        frame.abs_diff(expected) <= 1,
                        "{sr} beat{beat}: frame{frame} expected{expected}"
                    );
                    assert_eq!((got_on, pitch), (on, 60 + i as u8));
                }
            }
        }
    }
}
