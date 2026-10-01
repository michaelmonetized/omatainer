//! Deck-only sweep. Two non-resonant one-pole stages give a monotonic 12 dB/oct
//! response without the resonant wet/dry cancellation dips of the shared SVF.
//! The identity endpoint is a coefficient limit, not an audible cutoff jump.

#[derive(Clone, Copy, Debug, Default)]
pub struct ChannelFilter {
    pub(super) history: [f32; 2],
}

#[derive(Clone, Copy, Debug)]
pub(super) enum Curve {
    Bypass,
    LowPass(f32),
    HighPass { amount: f32, activation: f32 },
}

impl Curve {
    pub fn at(position: f32, sr: f32) -> Self {
        let distance = ((position - 0.5).abs() - 0.03).max(0.0) / 0.47;
        if distance <= f32::EPSILON || !distance.is_finite() {
            return Self::Bypass;
        }
        let distance = distance.min(1.0);
        // Ease the first fifth of each active half away from an exact identity
        // coefficient. Both the value and slope meet bypass continuously.
        let t = (distance * 5.0).min(1.0);
        let activation = t * t * (3.0 - 2.0 * t);
        if position < 0.5 {
            let cutoff = 18_000.0f32 * (60.0f32 / 18_000.0).powf(distance);
            Self::LowPass(activation * (-std::f32::consts::TAU * cutoff / sr).exp())
        } else {
            let cutoff = 20.0f32 * (8_000.0f32 / 20.0).powf(distance);
            Self::HighPass {
                amount: 1.0 - (-std::f32::consts::TAU * cutoff / sr).exp(),
                activation,
            }
        }
    }
}

impl ChannelFilter {
    pub(super) fn process(&mut self, mut sample: f32, curve: Curve) -> f32 {
        match curve {
            Curve::Bypass => {
                // Center is sample-transparent and retires old branch history.
                // A later move into either half cannot replay that old state.
                self.history = [0.0; 2];
            }
            Curve::LowPass(retention) => {
                for pole in &mut self.history {
                    *pole = sample + retention * (*pole - sample);
                    sample = *pole;
                }
            }
            Curve::HighPass { amount, activation } => {
                for pole in &mut self.history {
                    *pole += amount * (sample - *pole);
                    sample -= activation * *pole;
                }
            }
        }
        sample
    }
}

/// A full control-range move takes at most 5 ms, independent of callback size.
/// The deadband is crossed in multiple samples before switching filter branch.
pub(super) fn slew(current: f32, target: f32, sr: f32) -> f32 {
    let step = 1.0 / (0.005 * sr).max(1.0);
    current + (target - current).clamp(-step, step)
}
