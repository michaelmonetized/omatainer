//! A separate stereo monitor bus never replaces program samples.
use super::mixer_gain::GainPair;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    Pfl,
    #[default]
    DeckMix,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(
    tag = "op",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum Control {
    Mix(f32),
    Master(bool),
    Volume(f32),
    Fader { deck: u8, value: f32 },
    Blend(f32),
    Source(Source),
    Split(bool),
    Pfl { deck: u8, enabled: bool },
    Tone(u8),
    CancelTone,
}
impl Control {
    /// Validate a monitor gesture.
    /// Takes a decoded control; returns whether every channel and level is bounded.
    pub fn valid(self) -> bool {
        match self {
            Self::Mix(v) | Self::Volume(v) | Self::Blend(v) => {
                v.is_finite() && (0.0..=1.0).contains(&v)
            }
            Self::Fader { deck, value } => {
                deck < 2 && value.is_finite() && (0.0..=1.0).contains(&value)
            }
            Self::Pfl { deck, .. } => deck < 2,
            Self::Tone(channel) => channel < 2,
            _ => true,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Status {
    pub available: bool,
    pub active: bool,
    pub output_alias: Option<u64>,
    pub channels: Option<[usize; 2]>,
    pub mix: f32,
    pub master: bool,
    pub volume: f32,
    pub blend: f32,
    pub source: Source,
    pub split: bool,
    pub faders: [f32; 2],
    pub meters: [f32; 2],
    pub tone: Option<u8>,
}
impl Default for Status {
    fn default() -> Self {
        Self {
            available: false,
            active: false,
            output_alias: None,
            channels: None,
            mix: 0.5,
            master: false,
            volume: 0.0,
            blend: 0.0,
            source: Source::default(),
            split: false,
            faders: [1.0; 2],
            meters: [0.0; 2],
            tone: None,
        }
    }
}

pub(super) struct Monitor {
    pub status: Status,
    fader_ramp: GainPair,
    mix_ramp: GainPair,
    level_ramp: GainPair,
    blend_ramp: GainPair,
    source_ramp: GainPair,
    split_ramp: GainPair,
    faders: [f32; 2],
    decks: [[f32; 2]; 2],
    native: bool,
    opened_channels: Option<usize>,
    tone_frame: u32,
    routing_epoch: u64,
}
impl Default for Monitor {
    fn default() -> Self {
        Self {
            status: Status::default(),
            fader_ramp: GainPair::default(),
            mix_ramp: GainPair::default(),
            level_ramp: GainPair::default(),
            blend_ramp: GainPair::default(),
            source_ramp: GainPair::default(),
            split_ramp: GainPair::default(),
            faders: [1.0; 2],
            decks: [[0.0; 2]; 2],
            native: false,
            opened_channels: None,
            tone_frame: 0,
            routing_epoch: 0,
        }
    }
}
impl Monitor {
    /// Select an opened output.
    /// Takes the backend plan; retires tones and retains the original NS7 fallback only for its native four-channel output.
    pub fn output(&mut self, plan: &super::audio::config::Plan) {
        self.cancel_tone();
        self.native = plan.has_ns7_monitor();
        self.opened_channels = Some(usize::from(plan.channels));
        self.route(None, true, usize::from(plan.channels));
    }

    /// Resolve the separate monitor pair.
    /// Takes a prepared explicit alias, fallback permission and actual channel count; publishes availability without reassigning absent channels.
    pub fn route(&mut self, explicit: Option<(u64, [usize; 2])>, fallback: bool, channels: usize) {
        let alias = explicit.map(|value| value.0);
        let pair = explicit
            .map(|value| value.1)
            .or_else(|| (fallback && self.native).then_some([2, 3]));
        let available = pair.is_some_and(|pair| pair.iter().all(|channel| *channel < channels));
        if self.status.output_alias != alias
            || self.status.channels != pair
            || self.status.available != available
        {
            self.cancel_tone();
        }
        self.status.output_alias = alias;
        self.status.channels = pair;
        self.status.available = available;
        if !available {
            self.status.meters = [0.0; 2];
        }
    }

    /// Apply one monitor control.
    /// Takes a validated gesture; changes only headphone settings and physical deck faders.
    pub fn apply(&mut self, control: Control) {
        if !control.valid() {
            return;
        }
        match control {
            Control::Mix(value) => {
                self.status.mix = value;
                self.status.source = Source::DeckMix;
                self.status.active = true;
            }
            Control::Volume(value) => {
                self.status.volume = value;
                self.status.active = true;
            }
            Control::Master(value) => {
                self.status.master = value;
                self.status.source = Source::DeckMix;
                self.status.active = true;
            }
            Control::Fader { deck, value } => self.status.faders[usize::from(deck)] = value,
            Control::Blend(value) => {
                self.status.blend = value;
                self.status.active = true;
            }
            Control::Source(value) => {
                self.status.source = value;
                self.status.active = true;
            }
            Control::Split(value) => {
                self.status.split = value;
                self.status.active = true;
            }
            Control::Pfl { .. } => {}
            Control::Tone(channel) if self.status.available => {
                self.status.tone = Some(channel);
                self.tone_frame = 0;
            }
            Control::Tone(_) => {}
            Control::CancelTone => self.cancel_tone(),
        }
    }

    /// Stop a routing check.
    /// Takes current monitor state; removes its finite tone immediately without changing level or program audio.
    pub fn cancel_tone(&mut self) {
        self.status.tone = None;
        self.tone_frame = 0;
    }

    /// Prepare finite control ramps.
    /// Takes the output rate; updates fixed gain storage without allocation.
    pub fn prepare(&mut self, rate: f32) {
        self.fader_ramp
            .prepare(self.status.faders, rate, |a, b| [a, b]);
        self.mix_ramp
            .prepare([self.status.mix, 0.0], rate, |mix, _| [1.0 - mix, mix]);
        self.level_ramp.prepare(
            [
                if self.status.master { 1.0 } else { 0.0 },
                self.status.volume,
            ],
            rate,
            |a, b| [a, b],
        );
        self.blend_ramp
            .prepare([self.status.blend, 0.0], rate, |blend, _| {
                [1.0 - blend, blend]
            });
        self.source_ramp.prepare(
            [
                if self.status.source == Source::Pfl {
                    1.0
                } else {
                    0.0
                },
                0.0,
            ],
            rate,
            |v, _| [v, 1.0 - v],
        );
        self.split_ramp.prepare(
            [if self.status.split { 1.0 } else { 0.0 }, 0.0],
            rate,
            |v, _| [v, 1.0 - v],
        );
    }

    /// Begin one sample frame.
    /// Clears retained deck taps and advances the physical channel-fader ramp once.
    pub fn begin(&mut self) {
        self.decks = [[0.0; 2]; 2];
        self.faders = self.fader_ramp.tick();
    }

    /// Retain a pre-fader deck sample.
    /// Takes a deck index and equalized stereo frame; returns the physical channel-fader result.
    pub fn deck(&mut self, deck: usize, frame: [f32; 2]) -> [f32; 2] {
        self.decks[deck] = frame;
        frame.map(|sample| sample * self.faders[deck])
    }

    /// Read a retained cue source.
    /// Takes a deck index; returns its stereo sample before the physical channel fader.
    pub fn tap(&self, deck: usize) -> [f32; 2] {
        self.decks[deck]
    }

    /// Render the separate headphone pair.
    /// Takes audience and selected cue samples plus sample rate; returns finite stereo monitoring or silence when its pair is unavailable.
    pub fn render(&mut self, master: [f32; 2], pfl: [f32; 2], rate: u32) -> [f32; 2] {
        let gains = self.mix_ramp.tick();
        let [master_gain, volume] = self.level_ramp.tick();
        let blend = self.blend_ramp.tick();
        let source = self.source_ramp.tick();
        let split = self.split_ramp.tick();
        let cue: [f32; 2] =
            std::array::from_fn(|c| self.decks[0][c] * gains[0] + self.decks[1][c] * gains[1]);
        let selected: [f32; 2] = std::array::from_fn(|c| pfl[c] * source[0] + cue[c] * source[1]);
        let mixed: [f32; 2] = std::array::from_fn(|c| {
            (pfl[c] * blend[0] + master[c] * blend[1]) * source[0]
                + (cue[c] * (1.0 - master_gain) + master[c] * master_gain) * source[1]
        });
        let mono = [
            0.5 * (selected[0] + selected[1]),
            0.5 * (master[0] + master[1]),
        ];
        let mut output: [f32; 2] = std::array::from_fn(|c| {
            if self.status.available && self.status.active {
                (mixed[c] * split[1] + mono[c] * split[0]) * volume
            } else {
                0.0
            }
        });
        if let Some(channel) = self.status.tone {
            let rate = rate.max(1);
            if self.status.available && self.tone_frame < rate {
                let frame = self.tone_frame;
                let ramp = (rate / 100).max(1) as f32;
                let envelope = (frame as f32 / ramp)
                    .min((rate - 1 - frame) as f32 / ramp)
                    .clamp(0.0, 1.0);
                output = [0.0; 2];
                output[usize::from(channel)] = 0.01
                    * envelope
                    * (std::f32::consts::TAU * 997.0 * frame as f32 / rate as f32).sin();
                self.tone_frame += 1;
                if self.tone_frame == rate {
                    self.cancel_tone();
                }
            } else {
                self.cancel_tone();
            }
        }
        for (meter, value) in self.status.meters.iter_mut().zip(&mut output) {
            if !value.is_finite() {
                *value = 0.0;
            }
            *meter = *meter * 0.9 + value.abs() * 0.1;
        }
        output
    }
}

impl super::RtEngine {
    /// Render monitoring after immediate musical commands.
    /// Takes main and cue samples; cancels a routing tone on the exact frame that playback or contact starts.
    pub(super) fn render_monitor(&mut self, master: [f32; 2], cue: [f32; 2]) -> [f32; 2] {
        if self.monitor.status.tone.is_some() && !self.monitor_can_apply(Control::Tone(0)) {
            self.monitor.cancel_tone();
        }
        self.monitor.render(master, cue, self.sr as u32)
    }
    /// Check a monitor control against live state.
    /// Takes a bounded gesture; returns false when a routing tone would run on an unavailable pair or during playback/protection.
    pub(super) fn monitor_can_apply(&self, control: Control) -> bool {
        control.valid()
            && (!matches!(control, Control::Tone(_))
                || (self.monitor.status.available
                    && !self.playing
                    && !self.recording
                    && !self.decks.iter().any(|deck| deck.playing || deck.touching)
                    && !self.performance.status().protected))
    }

    /// Maintain monitor routing at a block boundary.
    /// Takes the rendered width and whether audio frames exist; publishes exact availability and cancels checks on playback or project/routing replacement.
    pub(super) fn maintain_monitor(&mut self, channels: usize, frames: bool) {
        self.monitor.status.blend = self.cue_mix;
        let epoch = self.routing_pipe.recorder.epoch();
        if self.monitor.routing_epoch != epoch {
            self.monitor.cancel_tone();
            self.monitor.routing_epoch = epoch;
        }
        let width = if frames {
            channels
        } else {
            self.monitor.opened_channels.unwrap_or(channels)
        };
        let explicit = self
            .routing
            .as_ref()
            .and_then(|routing| routing.monitor_output);
        let fallback = self
            .routing
            .as_ref()
            .is_none_or(|routing| routing.monitor_channels_free);
        self.monitor.route(explicit, fallback, width);
        if self.playing
            || self.recording
            || self.decks.iter().any(|deck| deck.playing || deck.touching)
            || self.performance.status().protected
        {
            self.monitor.cancel_tone();
        }
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod qualification_tests;
