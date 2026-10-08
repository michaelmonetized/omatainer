use super::dsp::Svf;
use std::f64::consts::{PI, TAU};

// Independent double-precision nodal solve of the trapezoidal integrators,
// rather than the production a1/a2/a3 coefficient expansion. See equations
// (3), (4) and (2): https://cytomic.com/files/dsp/SvfLinearTrapOptimised2.pdf
#[derive(Default)]
struct Reference {
    s1: f64,
    s2: f64,
}

impl Reference {
    fn tick(&mut self, x: f32, cutoff: f32, res: f32, sr: f32, morph: f32) -> f64 {
        let hz = (cutoff as f64).clamp(48_000.0 * 0.0001 / PI, sr as f64 * 0.45);
        let g = (PI * hz / sr as f64).tan();
        let k = 2.0 - (res as f64).clamp(0.0, 0.95) * 1.8;
        let bp = (self.s1 + g * (x as f64 - self.s2)) / (1.0 + g * k + g * g);
        let lp = self.s2 + g * bp;
        let hp = x as f64 - k * bp - lp;
        self.s1 = 2.0 * bp - self.s1;
        self.s2 = 2.0 * lp - self.s2;
        let morph = morph as f64;
        if morph < 0.5 {
            lp * (1.0 - morph * 2.0) + bp * morph * 2.0
        } else {
            bp * (2.0 - morph * 2.0) + hp * (morph * 2.0 - 1.0)
        }
    }
}

#[test]
fn retained_coefficients_preserve_exact_pcm_through_modulation_and_rate_changes_without_heap_work() {
    let mut filter = Svf::default();
    let mut s1 = 0.0_f32;
    let mut s2 = 0.0_f32;
    let counts = crate::engine::test_alloc::measure(|| {
        for frame in 0..64000 {
            let sr = [8000.0_f32,44100.0,48000.0,96000.0,192000.0][frame / 12800];
            let cutoff = if frame % 1024 < 512 { 1700.0 } else { -100.0 + (frame % 1024) as f32 * 70.0 };
            let res = [0.0_f32,0.35,0.95,1.5,-0.5][frame / 4000 % 5];
            let morph = (frame % 101) as f32 / 100.0;
            let x = ((frame as f32 * 0.017).sin() + if frame % 307 == 0 { 0.5 } else { 0.0 }) * 0.2;
            const MIN_HZ: f32 = 48000.0 * 0.0001 / std::f32::consts::PI;
            let max_hz = 0.45 * sr;
            let hz = cutoff.clamp(MIN_HZ.min(max_hz),max_hz);
            let g = (std::f32::consts::PI * hz / sr).tan();
            let k = 2.0 - res.clamp(0.0,0.95) * 1.8;
            let a1 = 1.0 / (1.0 + g * (g + k));
            let a2 = g * a1;
            let a3 = g * a2;
            let v3 = x - s2;
            let v1 = a1 * s1 + a2 * v3;
            let v2 = s2 + a2 * s1 + a3 * v3;
            s1 = 2.0 * v1 - s1; s2 = 2.0 * v2 - s2;
            let hp = x - k * v1 - v2;
            let expected = if morph < 0.5 { let t = morph * 2.0; v2 * (1.0-t) + v1*t }
                else { let t = (morph-0.5)*2.0; v1*(1.0-t)+hp*t };
            assert_eq!(filter.process(x,cutoff,res,sr,morph).to_bits(),expected.to_bits(),"frame={frame}");
            assert_eq!(filter.ic1eq.to_bits(),s1.to_bits()); assert_eq!(filter.ic2eq.to_bits(),s2.to_bits());
        }
    });
    assert_eq!(counts, Default::default());
}

#[test]
fn svf_impulse_and_chirp_match_independent_nodal_reference() {
    let mut maximum_error = 0.0_f64;
    for sr in [8_000.0, 44_100.0, 48_000.0, 96_000.0, 192_000.0] {
        for cutoff in [
            1_000.0,
            6_000.0,
            6_875.0,
            8_200.0,
            12_000.0,
            sr * 0.42,
            sr * 0.45,
        ] {
            for res in [0.0, 0.5, 0.95] {
                for morph in [0.0, 0.25, 0.5, 0.75, 1.0] {
                    for chirp in [false, true] {
                        let mut filter = Svf::default();
                        let mut reference = Reference::default();
                        let mut phase = 0.0_f64;
                        for frame in 0..8_192 {
                            // Sweep 0.001..0.49 cycles/sample using identical
                            // input samples for both implementations.
                            phase += TAU * (0.001 + 0.489 * frame as f64 / 8_192.0);
                            let x = if chirp {
                                phase.sin() as f32 * 0.25
                            } else {
                                (frame == 0) as u8 as f32
                            };
                            let expected = reference.tick(x, cutoff, res, sr, morph);
                            let actual = filter.process(x, cutoff, res, sr, morph) as f64;
                            let error = (actual - expected).abs();
                            maximum_error = maximum_error.max(error);
                            assert!(error < 2e-5, "sr={sr} cutoff={cutoff} res={res} morph={morph} frame={frame}: {actual} != {expected}");
                        }
                    }
                }
            }
        }
    }
    eprintln!("SVF impulse/chirp maximum absolute reference error: {maximum_error}");
}

fn measured_gain(sr: f64, cutoff: f32, probe: f64, res: f32, morph: f32) -> f64 {
    let mut filter = Svf::default();
    let mut input = 0.0;
    let mut output = 0.0;
    // Half a second of settling followed by a half-second energy measurement.
    for frame in 0..sr as usize {
        let x = (TAU * probe * frame as f64 / sr).sin() as f32 * 0.25;
        let y = filter.process(x, cutoff, res, sr as f32, morph);
        if frame >= sr as usize / 2 {
            input += (x as f64).powi(2);
            output += (y as f64).powi(2);
        }
    }
    (output / input).sqrt()
}

#[test]
fn svf_upper_cutoffs_are_distinct_and_match_analytical_sine_response() {
    for sr in [44_100.0, 48_000.0, 96_000.0] {
        let mut previous = 0.0;
        let mut gains = Vec::new();
        for cutoff in [3_000.0, 6_000.0, 6_875.0, 8_200.0, 12_000.0, 18_000.0] {
            let gain = measured_gain(sr, cutoff, 10_000.0, 0.0, 0.0);
            assert!(
                gain > previous + 0.02,
                "upper controls plateaued: {sr} {cutoff} {previous} -> {gain}"
            );
            gains.push(gain);
            previous = gain;
            for probe in [200.0, 2_000.0, 7_000.0, 10_000.0, 18_000.0] {
                for res in [0.0, 0.95] {
                    let omega = (PI * probe / sr).tan() / (PI * cutoff as f64 / sr).tan();
                    let k = 2.0 - res as f64 * 1.8;
                    let denominator = ((1.0 - omega * omega).powi(2) + (k * omega).powi(2)).sqrt();
                    for (morph, numerator) in [(0.0, 1.0), (0.5, omega), (1.0, omega * omega)] {
                        let expected = numerator / denominator;
                        let actual = measured_gain(sr, cutoff, probe, res, morph);
                        assert!((actual - expected).abs() < 2e-4, "sr={sr} cutoff={cutoff} probe={probe} res={res} morph={morph}: {actual} != {expected}");
                    }
                }
            }
        }
        eprintln!("SVF {sr} Hz LP gains at 10 kHz for cutoffs 3/6/6.875/8.2/12/18 kHz: {gains:?}");
    }
}

#[test]
fn svf_frequency_limits_and_resonance_extremes_stay_finite_and_decay() {
    for sr in [8_000.0, 44_100.0, 48_000.0, 96_000.0, 192_000.0] {
        let min_hz = 48_000.0 * 0.0001 / std::f32::consts::PI;
        for cutoff in [
            -f32::MAX,
            0.0,
            min_hz,
            20.0,
            sr * 0.42,
            sr * 0.45,
            sr,
            f32::MAX,
        ] {
            let clamped = cutoff.clamp(min_hz, sr * 0.45);
            for res in [-100.0, 0.0, 0.5, 0.95, 100.0] {
                for morph in [0.0, 0.5, 1.0] {
                    let mut filter = Svf::default();
                    let mut boundary = Svf::default();
                    for frame in 0..sr as usize {
                        let x = if frame < 128 {
                            ((frame * 17 % 31) as f32 - 15.0) / 30.0
                        } else {
                            0.0
                        };
                        let y = filter.process(x, cutoff, res, sr, morph);
                        let expected = boundary.process(x, clamped, res, sr, morph);
                        assert_eq!(y.to_bits(), expected.to_bits());
                        assert!(y.is_finite() && y.abs() < 4.0);
                        assert!(filter.ic1eq.is_finite() && filter.ic2eq.is_finite());
                    }
                    // Even the ~1.53 Hz high-resonance lower limit decays;
                    // higher cutoffs have essentially completed their tail.
                    let state = filter.ic1eq.abs() + filter.ic2eq.abs();
                    assert!(state < 0.02, "{sr} {cutoff} {res} {morph}: {state}");
                }
            }
        }
    }
}
