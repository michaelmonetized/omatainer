//! Absolute forward-error bounds for the isolated linear master contribution.
//! Coefficients are the actual rounded f32 coefficients used by the renderer.
//! This qualifies removal of that linear contribution at the limiter input;
//! it does not assert bitwise equivalence to a differently rounded whole mix.
use super::tail::RingPeak;

const U: f64 = f32::EPSILON as f64 / 2.0;
const TINY: f64 = f32::from_bits(1) as f64;
fn gamma(n: f64) -> f64 { n * U / (1.0 - n * U) }
fn up(value: f64) -> f64 {
    if !value.is_finite() || value < 0.0 { f64::INFINITY }
    else if value == 0.0 { 0.0 }
    else { let inflated = value * (1.0 + 1e-12); f64::from_bits(inflated.to_bits() + 1) }
}
fn rounding(n: f64, size: f64) -> f64 {
    if size == 0.0 { 0.0 } else { up(gamma(n) * size + n * TINY) }
}

#[derive(Clone, Copy, Debug, Default)]
pub(in crate::engine) struct DelayError { stored: RingPeak }
impl DelayError {
    pub fn stored_bound(&self) -> f64 { self.stored.peak() }
    pub fn tick(&mut self, amplitude: f64, x: f32, y: f32, input: f64,
        feedback: f32, wet: f32, wrapped: bool) -> f64
    {
        // a + (b-a)*f: convex exact interpolation, three rounded operations.
        let delayed = up(self.stored.peak() * (1.0 + gamma(3.0) * 3.0)
            + rounding(3.0, 3.0 * amplitude));
        let x = f64::from(x).abs();
        let y = f64::from(y).abs();
        let feedback = f64::from(feedback).abs();
        let stored = up(input + feedback * delayed + rounding(2.0, x + feedback * y));
        self.stored.write_bound(stored, wrapped);
        let dry = f64::from(1.0 - wet).abs();
        let wet = f64::from(wet).abs();
        up(dry * input + wet * delayed + rounding(3.0, dry * x + wet * y))
    }
}

pub(in crate::engine) fn pole(input: f64, previous: f64, x: f32, z: f32, a: f32) -> f64 {
    let a = f64::from(a);
    let x = f64::from(x).abs();
    let z = f64::from(z).abs();
    up((1.0 - a).abs() * input + a.abs() * previous
        + rounding(3.0, x + a.abs() * (z + x)))
}

pub(in crate::engine) fn mix(input: f64, filtered: f64, x: f32, y: f32, wet: f32) -> f64 {
    let dry = f64::from(1.0 - wet).abs();
    let wet = f64::from(wet).abs();
    up(dry * input + wet * filtered
        + rounding(3.0, dry * f64::from(x).abs() + wet * f64::from(y).abs()))
}

pub(in crate::engine) fn sum_four(errors: [f64; 4], magnitude: f64) -> f64 {
    up(errors.into_iter().sum::<f64>() * 0.25 + rounding(4.0, magnitude))
}
