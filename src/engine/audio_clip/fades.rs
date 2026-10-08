use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Fades {
    pub fade_in: f64,
    pub fade_out: f64,
    pub in_curve: f32,
    pub out_curve: f32,
    pub automatic: bool,
}
impl Fades {
    /// Omit an unchanged envelope from compatible saved audio.
    /// Takes these settings; returns true when no fade or automatic edge is requested.
    pub(crate) fn is_default(&self) -> bool {
        *self == Self::default()
    }
    /// Check complete fade geometry before rendering.
    /// Takes clip duration in quarter notes; returns whether durations/curves remain finite, bounded and nonoverlapping.
    pub(crate) fn valid(self, duration: f64) -> bool {
        duration.is_finite()
            && duration > 0.0
            && [self.fade_in, self.fade_out]
                .into_iter()
                .all(|value| value.is_finite() && (0.0..=duration).contains(&value))
            && self.fade_in + self.fade_out <= duration
            && [self.in_curve, self.out_curve]
                .into_iter()
                .all(|curve| curve.is_finite() && (-1.0..=1.0).contains(&curve))
    }
    /// Evaluate one coherent channel gain at the emitted musical position.
    /// Takes elapsed/duration quarter notes and quarter notes per second; returns a bounded multiplier with optional four-millisecond edges.
    pub(crate) fn gain(self, elapsed: f64, duration: f64, beats_per_second: f64) -> f32 {
        if !elapsed.is_finite()
            || !duration.is_finite()
            || duration <= 0.0
            || elapsed < 0.0
            || elapsed >= duration
        {
            return 0.0;
        }
        let shape = |progress: f64, curve: f32| {
            progress.clamp(0.0, 1.0).powf(2.0f64.powf(f64::from(curve)))
        };
        let mut gain = 1.0;
        if self.fade_in > 0.0 && elapsed < self.fade_in {
            gain *= shape(elapsed / self.fade_in, self.in_curve);
        }
        if self.fade_out > 0.0 && elapsed > duration - self.fade_out {
            gain *= 1.0
                - shape(
                    (elapsed - duration + self.fade_out) / self.fade_out,
                    self.out_curve,
                );
        }
        if self.automatic && beats_per_second.is_finite() && beats_per_second > 0.0 {
            let edge = (beats_per_second * 0.004).clamp(0.0, duration * 0.5);
            if edge > 0.0 {
                if self.fade_in == 0.0 {
                    gain *= (elapsed / edge).clamp(0.0, 1.0);
                }
                if self.fade_out == 0.0 {
                    gain *= ((duration - elapsed) / edge).clamp(0.0, 1.0);
                }
            }
        }
        gain as f32
    }
}
