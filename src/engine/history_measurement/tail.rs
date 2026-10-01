use super::RETIRE_FLOOR;
use crate::engine::FxKind;

/// Maximum from the previous completed ring and new writes in this rotation.
/// An overwritten previous maximum stays conservative until the ring wraps.
/// This is O(1), requires no buffer scan, and follows actual stored f32 values.
#[derive(Clone, Copy, Debug, Default)]
pub(in crate::engine) struct RingPeak {
    previous: f64,
    current: f64,
    invalid: bool,
}
impl RingPeak {
    pub fn write(&mut self, sample: f32, wrapped: bool) {
        self.write_bound(f64::from(sample).abs(), wrapped);
    }
    pub(super) fn write_bound(&mut self, sample: f64, wrapped: bool) {
        if !sample.is_finite() || sample < 0.0 { self.invalid = true; }
        self.current = self.current.max(sample);
        if wrapped { self.previous = self.current; self.current = 0.0; }
    }
    pub(in crate::engine) fn peak(&self) -> f64 {
        if self.invalid { f64::INFINITY } else { self.previous.max(self.current) }
    }
}

#[derive(Clone, Copy, Default)]
pub(in crate::engine) struct SlotBounds {
    pub echo: [RingPeak; 2],
    pub reverb: [[RingPeak; 4]; 2],
    pub filter: [[f32; 2]; 2],
    pub echo_error: [super::error::DelayError; 2],
    pub reverb_error: [[super::error::DelayError; 4]; 2],
    pub filter_error: [[f64; 2]; 2],
    pub output_error: [f64; 2],
}

impl SlotBounds {
    pub fn reset(&mut self, kind: FxKind) {
        match kind {
            FxKind::Echo => { self.echo = Default::default(); self.echo_error = Default::default(); },
            FxKind::Reverb => { self.reverb = Default::default(); self.reverb_error = Default::default(); },
            FxKind::Filter => { self.filter = Default::default(); self.filter_error = Default::default(); },
        }
    }
    pub fn future_bound(&self, kind: FxKind, input: [f64; 2]) -> [f64; 2] {
        std::array::from_fn(|channel| {
            let history = match kind {
                FxKind::Echo => self.echo[channel].peak() + self.echo_error[channel].stored_bound(),
                FxKind::Reverb => self.reverb[channel].iter().zip(self.reverb_error[channel])
                    .map(|(peak, error)| peak.peak() + error.stored_bound()).sum::<f64>() * 0.25,
                FxKind::Filter => {
                    let [first, second] = self.filter[channel];
                    if !first.is_finite() || !second.is_finite() { f64::INFINITY }
                    else { (f64::from(first).abs() + self.filter_error[channel][0])
                        .max(f64::from(second).abs() + self.filter_error[channel][1]) }
                }
            };
            // Current maximum real-arithmetic gain is 25/7 (reverb). Four
            // includes the f32 feedback/interpolation/mix error margin. The
            // 0.1% history inflation covers the two-pole recursion through
            // 384 kHz; tests derive a stricter roundoff upper bound. The additive
            // 1e-30 dominates accumulated subnormal rounding at all supported
            // pole coefficients and feedback values, while remaining far
            // below the retirement floor. A selected
            // new kind resets, so arbitrary future kind/wet changes obey this
            // envelope without retaining inactive states.
            if history == 0.0 && input[channel] == 0.0 { 0.0 }
            else { outward((history + 4.0 * input[channel]) * 1.001 + 1e-30) }
        })
    }
}

fn outward(value: f64) -> f64 {
    if !value.is_finite() || value < 0.0 { return f64::INFINITY; }
    if value == 0.0 { return 0.0; }
    f64::from_bits(value.to_bits() + 1)
}

pub(super) fn retirement_bound(slots: &[SlotBounds; 3], kinds: [FxKind; 3]) -> [f64; 2] {
    let mut bound = [0.0; 2];
    for (slot, kind) in slots.iter().zip(kinds) { bound = slot.future_bound(kind, bound); }
    bound.map(|value| outward(value * 1.5))
}

pub(super) fn certified_negligible(slots: &[SlotBounds; 3], kinds: [FxKind; 3]) -> bool {
    retirement_bound(slots, kinds).into_iter().all(|value| value < RETIRE_FLOOR)
}
