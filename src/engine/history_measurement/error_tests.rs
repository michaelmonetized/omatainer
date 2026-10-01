use super::*;
use crate::engine::{dsp::OnePole, master_fx::MasterSlot, FxKind};

#[test]
fn filter_error_contains_independent_double_precision_recursion() {
    for rate in [8_000, 44_100, 48_000, 192_000, 384_000] {
        let mut slot = MasterSlot::new(rate as f32);
        let mut bounds = SlotBounds::default();
        let a = f64::from(OnePole::lpf(rate as f32, 1000.0).a);
        let mut reference = [[0.0_f64; 2]; 2];
        for i in 0..10_000 {
            let input = [(i as f32 * 0.091).sin() * 0.71, (i as f32 * 0.021).cos() * 0.4];
            let wet = (i % 100) as f32 / 99.0;
            let out = slot.process_bounded(input, FxKind::Filter, wet, [1e-8; 2], &mut bounds);
            for c in 0..2 {
                // Alternate signs at the edge of the admitted input interval.
                let x = f64::from(input[c]) + if i % 2 == 0 { 1e-8 } else { -1e-8 };
                reference[c][0] = x + a * (reference[c][0] - x);
                let first = reference[c][0];
                reference[c][1] = first + a * (reference[c][1] - first);
                let ideal = x * f64::from(1.0 - wet) + reference[c][1] * f64::from(wet);
                assert!((ideal - f64::from(out[c])).abs() <= bounds.output_error[c], "rate{rate} frame{i}");
                assert!(bounds.output_error[c] < 0.0001, "bound must remain useful near musical levels");
            }
        }
    }
}

#[test]
fn delay_error_contains_independent_fractional_double_precision_reference() {
    use crate::engine::dsp::Delay;
    for feedback in [0.35_f32, 0.72] {
        let mut delay = Delay::new(127);
        delay.fb = feedback;
        let mut actual_peak = tail::RingPeak::default();
        let mut error = error::DelayError::default();
        let mut reference = [0.0_f64; 127];
        let mut write = 0;
        for i in 0..20_000 {
            let x = if i < 12_000 { (i as f32 * 0.031).sin() * 0.6 } else { 0.0 };
            let ideal_x = f64::from(x) + if i < 12_000 { 1e-9 } else { 0.0 };
            delay.time_samples = 1.0 + (i % 110) as f32 + 0.375;
            delay.mix = (i % 100) as f32 / 99.0;
            // Use the actual rounded address calculation; its coefficients
            // define the shipped linear processor, not an ideal delay clock.
            let position = (write as f32 - delay.time_samples + 127.0) % 127.0;
            let at = position as usize;
            let fraction = f64::from(position.fract());
            let y = reference[at] + (reference[(at + 1) % 127] - reference[at]) * fraction;
            reference[write] = ideal_x + y * f64::from(feedback);
            write = (write + 1) % 127;
            let ideal = ideal_x * f64::from(1.0 - delay.mix) + y * f64::from(delay.mix);
            let mix = delay.mix;
            let mut bound = 0.0;
            let actual = delay.tick_traced(x, |delayed, stored, wrapped| {
                bound = error.tick(actual_peak.peak(), x, delayed,
                    if i < 12_000 { 1e-9 } else { 0.0 }, feedback, mix, wrapped);
                actual_peak.write(stored, wrapped);
            });
            assert!((ideal - f64::from(actual)).abs() <= bound + 1e-15, "frame{i} feedback{feedback}");
            assert!(bound < 0.0001);
        }
    }
}
