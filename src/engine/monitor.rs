use super::mixer_gain::GainPair;

#[derive(Clone, Copy, Debug)]
pub enum Control {
    Mix(f32),
    Master(bool),
    Volume(f32),
    Fader { deck: u8, value: f32 },
}

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize)]
pub struct Status {
    pub available: bool,
    pub active: bool,
    pub mix: f32,
    pub master: bool,
    pub volume: f32,
    pub faders: [f32; 2],
}

#[cfg(test)]
mod tests;
impl Default for Status {
    fn default() -> Self {
        Self {
            available: false,
            active: false,
            mix: 0.5,
            master: false,
            volume: 0.0,
            faders: [1.0; 2],
        }
    }
}

pub(super) struct Monitor {
    pub status: Status,
    fader_ramp: GainPair,
    mix_ramp: GainPair,
    level_ramp: GainPair,
    faders: [f32; 2],
    decks: [[f32; 2]; 2],
}
impl Default for Monitor {
    fn default() -> Self {
        Self {
            status: Status::default(),
            fader_ramp: GainPair::default(),
            mix_ramp: GainPair::default(),
            level_ramp: GainPair::default(),
            faders: [1.0; 2],
            decks: [[0.0; 2]; 2],
        }
    }
}

impl Monitor {
    /// Select the original NS7 monitor output.
    /// Takes the opened output plan; enables channels three and four only on the native four-channel NS7 ALSA card.
    pub fn output(&mut self, plan: &super::audio::config::Plan) {
        self.status.available = plan.has_ns7_monitor();
    }

    /// Apply one monitor control.
    /// Takes an absolute physical control; rejects nonfinite values and leaves the audience mix independent.
    pub fn apply(&mut self, control: Control) {
        match control {
            Control::Mix(value) if value.is_finite() => {
                self.status.mix = value.clamp(0.0, 1.0);
                self.status.active = true;
            }
            Control::Volume(value) if value.is_finite() => {
                self.status.volume = value.clamp(0.0, 1.0);
                self.status.active = true;
            }
            Control::Master(master) => {
                self.status.master = master;
                self.status.active = true;
            }
            Control::Fader { deck, value } if deck < 2 && value.is_finite() => {
                self.status.faders[usize::from(deck)] = value.clamp(0.0, 1.0)
            }
            _ => {}
        }
    }

    /// Prepare finite control ramps.
    /// Takes the output sample rate; updates existing gain storage without allocation.
    pub fn prepare(&mut self, rate: f32) {
        self.fader_ramp
            .prepare(self.status.faders, rate, |a, b| [a, b]);
        self.mix_ramp.prepare(
            [self.status.mix, if self.status.master { 1.0 } else { 0.0 }],
            rate,
            |mix, master| [(1.0 - mix) * (1.0 - master), mix * (1.0 - master)],
        );
        self.level_ramp.prepare(
            [
                if self.status.master { 1.0 } else { 0.0 },
                self.status.volume,
            ],
            rate,
            |master, volume| [master, volume],
        );
    }

    /// Begin one monitor sample frame.
    /// Clears the two retained deck taps and advances the channel-fader ramp once.
    pub fn begin(&mut self) {
        self.decks = [[0.0; 2]; 2];
        self.faders = self.fader_ramp.tick();
    }

    /// Retain a pre-fader deck sample.
    /// Takes a deck index and equalized stereo frame; returns that frame after its physical channel fader.
    pub fn deck(&mut self, deck: usize, frame: [f32; 2]) -> [f32; 2] {
        self.decks[deck] = frame;
        frame.map(|sample| sample * self.faders[deck])
    }

    /// Render the separate headphone pair.
    /// Takes the audience sample and output width; returns the headphone mix, or silence unless the native pair is active.
    pub fn render(&mut self, master: [f32; 2], channels: usize) -> [f32; 2] {
        let gains = self.mix_ramp.tick();
        let [master_gain, volume] = self.level_ramp.tick();
        if !self.status.available || !self.status.active || channels < 4 {
            return [0.0; 2];
        }
        std::array::from_fn(|channel| {
            (master[channel] * master_gain
                + self.decks[0][channel] * gains[0]
                + self.decks[1][channel] * gains[1])
                * volume
        })
    }
}
