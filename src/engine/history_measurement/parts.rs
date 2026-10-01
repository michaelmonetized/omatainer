//! Fixed attribution of the deck's existing two-millisecond output envelope.
//! It does not read PCM, touch filters, or change the actual output arithmetic.
use super::LANES;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(in crate::engine) struct Part {
    pub key: u64,
    pub value: [f64; 2],
}

#[derive(Clone, Copy, Debug, Default)]
pub(in crate::engine) struct Parts {
    pub values: [Part; LANES],
    /// A fifth overlapping source cannot acquire false ownership of another.
    pub incomplete: bool,
}

impl Parts {
    pub fn insert(&mut self, key: u64, value: [f64; 2]) {
        if value == [0.0; 2] { return; }
        if key == 0 || value.iter().any(|x| !x.is_finite()) {
            self.incomplete = true;
            return;
        }
        if let Some(part) = self.values.iter_mut().find(|part| part.key == key || part.key == 0) {
            part.key = key;
            for c in 0..2 { part.value[c] += value[c]; }
        } else { self.incomplete = true; }
    }
    pub fn transition(old: Self, key: u64, input: [f32; 2], mix: f32) -> Self {
        let mut next = Self::default();
        if mix < 1.0 {
            next.incomplete = old.incomplete;
            for part in old.values {
                next.insert(part.key, part.value.map(|v| v * f64::from(1.0 - mix)));
            }
        }
        next.insert(key, input.map(|v| f64::from(v) * f64::from(mix)));
        next
    }
    pub fn residual(&self, actual: [f32; 2]) -> [f64; 2] {
        std::array::from_fn(|c| {
            let total = self.values.iter().map(|p| p.value[c]).sum::<f64>();
            (f64::from(actual[c]) - total).abs()
        })
    }
}

/// Global process-local keys are never reused, including after Undo retirement.
/// One bounded CAS attempt: concurrent registration can yield an explicit
/// unresolved zero key, rather than spin in an audio callback or wrap an ID.
pub(in crate::engine) fn fresh_key() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(1);
    let value = NEXT.load(Ordering::Relaxed);
    if value >= (1_u64 << 63) { return 0; }
    NEXT.compare_exchange(value, value + 1, Ordering::Relaxed, Ordering::Relaxed)
        .map_or(0, |_| value)
}

pub(in crate::engine) fn unresolved_key() -> u64 {
    let key = fresh_key();
    if key == 0 { 0 } else { key | (1_u64 << 63) }
}
