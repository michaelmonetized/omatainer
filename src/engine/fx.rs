//! Per-track / per-scene FX chain. Slots are stackable; order is the chain.

mod bypass;
mod parameters;
pub use parameters::Control;
#[cfg(test)]
mod parameter_tests;
#[cfg(test)]
mod neutral_tests;
#[cfg(test)]
mod stereo_mix_tests;

use crate::engine::dsp::{rate_blend, Delay, OnePole, Reverb, Svf};

#[cfg(test)]
mod tests;
#[cfg(test)]
mod wet_mix_tests;

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FxId {
    Comp,
    Spread,
    Balance,
    Reverb,
    Chorus,
    Delay,
    Gate,
    Arp,
    Dist,
    Filter,
    Eq3,
    Eq5,
    Eq8,
}

impl FxId {
    pub fn all() -> &'static [FxId] {
        &[
            FxId::Comp,
            FxId::Spread,
            FxId::Balance,
            FxId::Reverb,
            FxId::Chorus,
            FxId::Delay,
            FxId::Gate,
            FxId::Arp,
            FxId::Dist,
            FxId::Filter,
            FxId::Eq3,
            FxId::Eq5,
            FxId::Eq8,
        ]
    }
    pub fn name(self) -> &'static str {
        match self {
            FxId::Comp => "comp",
            FxId::Spread => "spread",
            FxId::Balance => "balance",
            FxId::Reverb => "reverb",
            FxId::Chorus => "chorus",
            FxId::Delay => "delay",
            FxId::Gate => "gate",
            FxId::Arp => "arp",
            FxId::Dist => "drive",
            FxId::Filter => "filter",
            FxId::Eq3 => "eq3",
            FxId::Eq5 => "eq5",
            FxId::Eq8 => "eq8",
        }
    }
}

/// A slot owns its processor history. Parameter edits keep that history; bypass
/// fades to dry over 5 ms, then freezes it. Re-enabling fades the retained
/// tail back in over 5 ms. A reversal continues from the current fade level.
/// Delete or replace the slot to discard its history. Effect type is immutable.
#[derive(Clone, Debug)]
pub struct FxSlot {
    id: FxId,
    pub on: bool,
    pub mix: f32,
    pub p: [f32; 4],
    sample_rate: f32,
    state: FxState,
    bypass: bypass::Bypass,
}

#[derive(Clone, Debug)]
enum FxState {
    None,
    Envelope { level: f32, blend: f32 },
    Spread([Delay; 2]),
    Reverb([Reverb; 2]),
    Chorus { delays: [Delay; 2], phase: f32 },
    Delay([Delay; 2]),
    Filter([Svf; 2]),
    Eq([[OnePole; 8]; 2]),
}

impl FxSlot {
    pub(super) fn required_storage(id:FxId,sr:f32)->usize {
        let frames=match id {
            FxId::Spread=>2*((sr*0.02) as usize).max(64),FxId::Chorus=>2*((sr*0.05) as usize).max(64),FxId::Delay=>2*((sr*2.0) as usize).max(64),
            FxId::Reverb=>2*[3011.0,4057.0,5059.0,2333.0].iter().map(|n|((n*sr/48000.0).round() as usize).max(64)).sum::<usize>(),_=>0,
        };frames*std::mem::size_of::<f32>()
    }
    pub(super) fn storage_bytes(&self) -> usize {
        match &self.state {
            FxState::Spread(d) | FxState::Delay(d) | FxState::Chorus { delays: d, .. } =>
                d.iter().map(Delay::storage_bytes).sum(),
            FxState::Reverb(r) => r.iter().map(Reverb::storage_bytes).sum(),
            _ => 0,
        }
    }
    /// Build the requested processor before inserting it into a chain. No state
    /// is shared by effect type or allocated lazily during sample processing.
    pub fn new(id: FxId, sr: f32) -> Self {
        let p = match id {
            FxId::Comp => [0.4, 0.35, 0.2, 0.0],
            FxId::Spread => [0.5, 0.0, 0.0, 0.0],
            FxId::Balance => [0.5, 0.0, 0.0, 0.0],
            FxId::Reverb => [0.35, 0.5, 0.0, 0.0],
            FxId::Chorus => [0.4, 0.3, 0.0, 0.0],
            FxId::Delay => [0.4, 0.35, 0.0, 0.0],
            FxId::Gate => [0.15, 0.4, 0.0, 0.0],
            FxId::Arp => [0.25, 0.5, 0.0, 0.0],
            FxId::Dist => [0.25, 0.0, 0.0, 0.0],
            FxId::Filter => [0.5, 0.3, 0.0, 0.0],
            FxId::Eq3 | FxId::Eq5 | FxId::Eq8 => [0.5, 0.5, 0.5, 0.5],
        };
        let state = match id {
            FxId::Comp | FxId::Gate => FxState::Envelope {
                level: 0.0,
                blend: rate_blend(if id == FxId::Comp { 0.005 } else { 0.02 }, sr),
            },
            FxId::Spread => {
                FxState::Spread(std::array::from_fn(|_| Delay::new((sr * 0.02) as usize)))
            }
            FxId::Reverb => FxState::Reverb(std::array::from_fn(|_| Reverb::at_sample_rate(sr))),
            FxId::Chorus => FxState::Chorus {
                delays: std::array::from_fn(|_| {
                    let mut delay = Delay::new((sr * 0.05) as usize);
                    delay.fb = 0.0;
                    delay
                }),
                phase: 0.0,
            },
            FxId::Delay => FxState::Delay(std::array::from_fn(|_| {
                let mut delay = Delay::new((sr * 2.0) as usize);
                delay.time_samples = sr * 0.25;
                delay
            })),
            FxId::Filter => FxState::Filter([Svf::default(); 2]),
            FxId::Eq3 | FxId::Eq5 | FxId::Eq8 => FxState::Eq([eq_filters(sr); 2]),
            FxId::Balance | FxId::Arp | FxId::Dist => FxState::None,
        };
        Self {
            id,
            on: true,
            mix: 0.5,
            p,
            sample_rate: sr,
            state,
            bypass: bypass::Bypass::default(),
        }
    }

    pub fn set_control(&mut self, parameter: Option<u8>, value: f32) -> bool {
        if !value.is_finite()
            || !self.id.controls().iter().any(|control| control.parameter == parameter)
        {
            return false;
        }
        if let Some(index) = parameter {
            self.p[index as usize] = value.clamp(0.0, 1.0);
        } else {
            self.mix = value.clamp(0.0, 1.0);
        }
        true
    }

    pub fn id(&self) -> FxId {
        self.id
    }

    /// Output must be stopped. Reallocate only this slot's processor storage,
    /// discard its tail, and preserve effect order, bypass and all controls.
    pub fn set_sample_rate(&mut self, sr: f32) {
        if self.sample_rate != sr {
            self.state = Self::new(self.id, sr).state;
            self.sample_rate = sr;
            self.bypass = bypass::Bypass::default();
        }
    }

    pub(super) fn tick_stereo(&mut self, input: [f32; 2], sr: f32) -> [f32; 2] {
        let level = self.bypass.next(self.on, sr);
        if level == 0.0 {
            return input;
        }
        let processed = self.process_enabled(input, sr);
        if level == 1.0 || processed == input {
            processed
        } else {
            std::array::from_fn(|channel| {
                input[channel] * (1.0 - level) + processed[channel] * level
            })
        }
    }

    fn process_enabled(&mut self, input: [f32; 2], sr: f32) -> [f32; 2] {
        let wet = match &mut self.state {
            FxState::Envelope { level: env, blend } => {
                // Link the two channels' gain reduction without sharing the
                // detector with another compressor or gate in the rack.
                let level = input[0].abs().max(input[1].abs());
                *env = *env * (1.0 - *blend) + level * *blend;
                let gain = if self.id == FxId::Comp {
                    let threshold = 0.05 + self.p[0] * 0.4;
                    let reduction = if *env > threshold {
                        threshold / env.max(1e-6)
                    } else {
                        1.0
                    };
                    1.0 - self.p[1] + self.p[1] * reduction
                } else {
                    if *env < self.p[0] {
                        0.05
                    } else {
                        1.0
                    }
                };
                input.map(|x| x * gain)
            }
            FxState::Spread(delays) => {
                let width = self.p[0];
                if width == 0.5 {
                    // A neutral spread never touches a delay tap (including
                    // Delay's minimum one-sample delay) or advances its buffers.
                    return input;
                }
                let delayed: [f32; 2] = std::array::from_fn(|channel| {
                    let delay = &mut delays[channel];
                    delay.time_samples = (width - 0.5).abs() * sr * 0.012;
                    delay.mix = 1.0;
                    delay.fb = 0.0;
                    delay.tick(input[channel])
                });
                // Compute the full-width result here; the common slot path
                // applies dry/wet mix once, preserving this position in series.
                if width > 0.5 {
                    [input[0], delayed[1]]
                } else if width < 0.5 {
                    let narrow = (0.5 - width) * 2.0;
                    let mid = (delayed[0] + delayed[1]) * 0.5;
                    input.map(|x| x * (1.0 - narrow) + mid * narrow)
                } else {
                    input
                }
            }
            FxState::Reverb(reverbs) => std::array::from_fn(|channel| {
                reverbs[channel].mix = 1.0;
                reverbs[channel].tick(input[channel])
            }),
            FxState::Chorus { delays, phase } => {
                *phase = (*phase + 0.7 / sr) % 1.0;
                std::array::from_fn(|channel| {
                    delays[channel].time_samples =
                        sr * (0.008 + 0.006 * (*phase * std::f32::consts::TAU).sin());
                    delays[channel].mix = 1.0;
                    delays[channel].tick(input[channel])
                })
            }
            FxState::Delay(delays) => std::array::from_fn(|channel| {
                delays[channel].fb = self.p[1];
                delays[channel].mix = 1.0;
                delays[channel].tick(input[channel])
            }),
            FxState::Filter(filters) => std::array::from_fn(|channel| {
                filters[channel].process(
                    input[channel],
                    120.0 + self.p[1] * 8000.0,
                    0.35,
                    sr,
                    self.p[0],
                )
            }),
            FxState::Eq(filters) => {
                let bands = match self.id {
                    FxId::Eq3 => 3,
                    FxId::Eq5 => 5,
                    _ => 8,
                };
                let filtered = std::array::from_fn(|channel| {
                    eq_n(&mut filters[channel], input[channel], self.p, bands)
                });
                // Keep histories warm while flat, but avoid cancellation and
                // recombination roundoff at the neutral EQ setting.
                if self.p[..3] == [0.5; 3] { input } else { filtered }
            }
            FxState::None => match self.id {
                FxId::Balance => {
                    let pan = (self.p[0] * 2.0 - 1.0).clamp(-1.0, 1.0);
                    [
                        input[0] * (1.0 - pan.max(0.0)).sqrt(),
                        input[1] * (1.0 + pan.min(0.0)).sqrt(),
                    ]
                }
                FxId::Dist => input.map(|x| (x * (1.0 + self.p[0] * 8.0)).tanh()),
                // Arpeggiation is an upstream MIDI event processor.
                _ => input,
            },
        };
        // Processors supply fully wet output. Interpolate exactly once here:
        // mix=0 is dry, mix=1 is wet, and intermediate values are linear.
        if self.mix == 0.0 || wet == input {
            input
        } else if self.mix == 1.0 {
            wet
        } else {
            std::array::from_fn(|channel| input[channel] * (1.0 - self.mix) + wet[channel] * self.mix)
        }
    }
}

fn eq_filters(sr: f32) -> [OnePole; 8] {
    [80.0, 160.0, 320.0, 640.0, 1280.0, 2560.0, 5120.0, 9000.0]
        .map(|frequency| OnePole::lpf(sr, frequency))
}

#[derive(Clone, Debug)]
pub struct FxChain {
    pub slots: Vec<FxSlot>,
}

impl FxChain {
    pub fn new(_sr: f32) -> Self {
        Self { slots: Vec::new() }
    }

    pub fn set_sample_rate(&mut self, sr: f32) {
        for slot in &mut self.slots {
            slot.set_sample_rate(sr);
        }
    }

    pub fn tick(&mut self, x: f32, sr: f32) -> f32 {
        self.process_stereo([x, x], sr)[0]
    }

    pub fn tick_stereo(&mut self, x: f32, sr: f32) -> (f32, f32) {
        let [left, right] = self.process_stereo([x, x], sr);
        (left, right)
    }

    pub fn process_stereo(&mut self, mut frame: [f32; 2], sr: f32) -> [f32; 2] {
        for slot in &mut self.slots {
            frame = slot.tick_stereo(frame, sr);
        }
        frame
    }
}

fn eq_n(lp: &mut [crate::engine::dsp::OnePole; 8], x: f32, p: [f32; 4], n: usize) -> f32 {
    let n = n.clamp(3, 8);
    let mut prev = 0.0f32;
    let mut acc = 0.0;
    for i in 0..n {
        let c = lp[i.min(7)].tick(x);
        let band = if i + 1 == n { x - prev } else { c - prev };
        prev = c;
        let t = i as f32 / (n - 1) as f32;
        let g = if t < 0.5 {
            p[0] + (p[1] - p[0]) * (t * 2.0)
        } else {
            p[1] + (p[2] - p[1]) * ((t - 0.5) * 2.0)
        };
        acc += band * (0.25 + g * 1.5);
    }
    acc
}
