use super::{
    model::{Direction, Group, Model},
    prepared::Frame,
};
use crate::engine::dsp::ThreeBand;
use serde::{Deserialize, Serialize};
pub(crate) mod control;
#[cfg(test)]
pub(crate) mod tests;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Channel {
    pub input: Option<u64>,
    pub gain: f32,
    pub mute: bool,
    pub tone: bool,
    pub eq_db: [f32; 3],
    pub master: Option<u64>,
    pub booth: Option<u64>,
    pub record: Option<u64>,
}
impl Default for Channel {
    fn default() -> Self {
        Self {
            input: None,
            gain: 1.0,
            mute: true,
            tone: false,
            eq_db: [0.0; 3],
            master: None,
            booth: None,
            record: None,
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Override {
    #[default]
    Automatic,
    Held,
    Off,
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Duck {
    pub enabled: bool,
    pub threshold: f32,
    pub reduction_db: f32,
    pub attack_ms: f32,
    pub release_ms: f32,
    pub mode: Override,
}
impl Default for Duck {
    fn default() -> Self {
        Self {
            enabled: false,
            threshold: 0.02,
            reduction_db: 12.0,
            attack_ms: 20.0,
            release_ms: 250.0,
            mode: Override::Automatic,
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Configuration {
    pub channels: [Channel; 2],
    pub duck: Duck,
}
impl Configuration {
    /// Validate voice and line routing without allocating.
    /// Takes the retained alias graph; returns a static refusal for invalid controls, duplicate physical feeds or unsafe monitoring destinations.
    pub(crate) fn validate(&self, model: Option<&Model>) -> Result<(), &'static str> {
        let finite = |v: f32, a: f32, b: f32| v.is_finite() && (a..=b).contains(&v);
        if !finite(self.duck.threshold, 0.00001, 1.0)
            || !finite(self.duck.reduction_db, 0.0, 40.0)
            || !finite(self.duck.attack_ms, 1.0, 2000.0)
            || !finite(self.duck.release_ms, 1.0, 10000.0)
        {
            return Err("Talkover needs finite threshold, reduction and attack/release values");
        }
        for c in &self.channels {
            if !finite(c.gain, 0.0, 4.0) || c.eq_db.iter().any(|&v| !finite(v, -18.0, 18.0)) {
                return Err("Mic/aux gain or tone is outside its supported range");
            }
            let Some(id) = c.input else {
                if c.master.is_some() || c.booth.is_some() || c.record.is_some() {
                    return Err("Choose an input before including it in a mix");
                }
                continue;
            };
            let m = model.ok_or("Mic/aux requires a retained input/output alias graph")?;
            let input = m
                .port(id, Direction::Input)
                .filter(|p| (1..=2).contains(&p.channels.len()))
                .ok_or("Mic/aux input needs one or two available channels")?;
            for connection in &m.connections {
                if let Group::Input(source) = connection.source.group {
                    let port = m
                        .port(source, Direction::Input)
                        .ok_or("Input alias was removed")?;
                    if connection.map.iter().any(|p| p.gain != 0.0)
                        && port.channels.iter().any(|p| input.channels.contains(p))
                    {
                        return Err("These input channels already have manual routes; remove those routes or choose other channels");
                    }
                }
            }
            for (target, direction) in [
                (c.master, Direction::Output),
                (c.booth, Direction::Output),
                (c.record, Direction::Record),
            ] {
                if let Some(target) = target {
                    let port = m
                        .port(target, direction)
                        .filter(|p| (1..=2).contains(&p.channels.len()))
                        .ok_or("Mic/aux destination needs one or two available channels")?;
                    if m.monitor_output == Some(target) {
                        return Err("Headphone monitoring cannot be a mic/aux destination");
                    }
                    if direction == Direction::Output
                        && port.channels.iter().any(|p| {
                            m.ports
                                .iter()
                                .filter(|other| {
                                    other.direction == Direction::Output && other.id != target
                                })
                                .any(|other| other.channels.contains(p))
                        })
                    {
                        return Err("Mic/aux output channels overlap another output alias");
                    }
                }
            }
            if c.master.is_some() && c.master == c.booth {
                return Err("Master and booth need distinct output aliases");
            }
        }
        if let Some(m) = model {
            if let (Some(a), Some(b)) = (self.channels[0].input, self.channels[1].input) {
                let a = m
                    .port(a, Direction::Input)
                    .ok_or("Mic input alias was removed")?;
                let b = m
                    .port(b, Direction::Input)
                    .ok_or("Aux input alias was removed")?;
                if a.channels.iter().any(|p| b.channels.contains(p)) {
                    return Err("Mic and aux cannot monitor overlapping physical input channels");
                }
            }
        }
        for targets in [
            (self.channels[0].master, self.channels[1].master),
            (self.channels[0].booth, self.channels[1].booth),
            (self.channels[0].record, self.channels[1].record),
        ] {
            if let (Some(a), Some(b)) = targets {
                if a != b {
                    return Err(
                        "Both channels must agree on the master, booth and recording mix aliases",
                    );
                }
            }
        }
        let master = self.channels.iter().find_map(|c| c.master);
        let booth = self.channels.iter().find_map(|c| c.booth);
        if master.is_some() && master == booth {
            return Err("Master and booth need distinct output aliases");
        }
        Ok(())
    }
    /// Read whether offline rendering needs a physical source.
    /// Takes no arguments; returns true if any unmuted audible role uses a live input.
    pub(crate) fn needs_input(&self) -> bool {
        self.channels.iter().any(|c| {
            c.input.is_some()
                && !c.mute
                && c.gain > 0.0
                && (c.master.is_some() || c.booth.is_some() || c.record.is_some())
        })
    }
}
#[derive(Clone, Copy, Debug, Default, Serialize)]
pub(crate) struct Meter {
    pub available: bool,
    pub input_peak: f32,
    pub output_peak: f32,
}
#[derive(Clone, Copy, Debug, Serialize)]
pub(crate) struct Status {
    pub configuration: Option<Configuration>,
    pub meters: [Meter; 2],
    pub duck_gain: f32,
}
impl Default for Status {
    fn default() -> Self {
        Self {
            configuration: None,
            meters: [Meter::default(); 2],
            duck_gain: 1.0,
        }
    }
}
#[derive(Clone, Copy)]
struct Ramp {
    value: f32,
    target: f32,
    step: f32,
    left: u32,
}
impl Ramp {
    fn new(value: f32) -> Self {
        Self {
            value,
            target: value,
            step: 0.0,
            left: 0,
        }
    }
    fn set(&mut self, target: f32, frames: u32) {
        if self.target != target {
            self.target = target;
            self.left = frames.max(1);
            self.step = (target - self.value) / self.left as f32;
        }
    }
    fn tick(&mut self) -> f32 {
        if self.left > 0 {
            self.left -= 1;
            self.value = if self.left == 0 {
                self.target
            } else {
                self.value + self.step
            };
        }
        self.value
    }
}
struct Voice {
    eq: [ThreeBand; 2],
    eq_gain: [Ramp; 3],
    gain: Ramp,
    availability: Ramp,
    last: [f32; 2],
}
impl Voice {
    fn new(rate: f32) -> Self {
        Self {
            eq: [ThreeBand::new(rate); 2],
            eq_gain: [Ramp::new(1.0); 3],
            gain: Ramp::new(0.0),
            availability: Ramp::new(0.0),
            last: [0.0; 2],
        }
    }
}
pub(crate) struct Mixer {
    configuration: Option<Configuration>,
    active: Option<Configuration>,
    changing: bool,
    rate: f32,
    decay: f32,
    voices: [Voice; 2],
    frames: [[f32; 2]; 2],
    valid: [bool; 2],
    meters: [Meter; 2],
    duck: f32,
    attack: f32,
    release: f32,
    reduction: f32,
}
impl Mixer {
    /// Prepare bounded microphone and auxiliary processing.
    /// Takes a validated configuration and sample rate; returns fixed filters, gain ramps, meters and a talkover envelope without devices or workers.
    pub(crate) fn new(configuration: Option<Configuration>, rate: f32) -> Self {
        let mut m = Self {
            configuration: None,
            active: None,
            changing: false,
            rate,
            decay: (-1.0 / (rate * 0.2)).exp(),
            voices: std::array::from_fn(|_| Voice::new(rate)),
            frames: [[0.0; 2]; 2],
            valid: [false; 2],
            meters: [Meter::default(); 2],
            duck: 1.0,
            attack: 0.0,
            release: 0.0,
            reduction: 1.0,
        };
        m.set(configuration);
        m
    }
    /// Read persistent controls.
    /// Takes this mixer; returns copyable configuration without transient filter histories or held sample data.
    pub(crate) fn configuration(&self) -> Option<Configuration> {
        self.configuration
    }
    /// Update validated scalar controls and alias selection.
    /// Takes a configuration; preserves matching source histories, ramps gains and prepares new coefficients without allocating or changing an input device.
    pub(crate) fn set(&mut self, configuration: Option<Configuration>) {
        self.configuration = configuration;
        let topology = |cfg: Option<Configuration>| {
            cfg.unwrap_or_default()
                .channels
                .map(|c| (c.input, c.master, c.booth, c.record))
        };
        self.changing = topology(self.active) != topology(configuration);
        if self.changing {
            let frames = self.ramp_frames();
            for voice in &mut self.voices {
                voice.gain.set(0.0, frames);
            }
        } else {
            self.activate(configuration);
        }
    }
    fn ramp_frames(&self) -> u32 {
        (self.rate * 0.005).round().max(1.0) as u32
    }
    fn activate(&mut self, configuration: Option<Configuration>) {
        let old = self.active;
        let frames = self.ramp_frames();
        for i in 0..2 {
            let c = configuration.unwrap_or_default().channels[i];
            if old.and_then(|c| c.channels[i].input) != c.input {
                self.voices[i] = Voice::new(self.rate);
            }
            self.voices[i].gain.set(
                if c.mute || c.input.is_none() {
                    0.0
                } else {
                    c.gain
                },
                frames,
            );
            for (b, gain) in self.voices[i].eq_gain.iter_mut().enumerate() {
                gain.set(
                    if c.tone {
                        10_f32.powf(c.eq_db[b] / 20.0)
                    } else {
                        1.0
                    },
                    frames,
                );
            }
        }
        let d = configuration.unwrap_or_default().duck;
        self.attack = (-1.0 / (self.rate * d.attack_ms * 0.001)).exp();
        self.release = (-1.0 / (self.rate * d.release_ms * 0.001)).exp();
        self.reduction = 10_f32.powf(-d.reduction_db / 20.0);
        self.active = configuration;
        self.changing = false;
    }
    /// Prepare filters after a stopped device's rate changes.
    /// Takes a new sample rate; rebuilds bounded histories while retaining saved controls.
    pub(crate) fn set_sample_rate(&mut self, rate: f32) {
        *self = Self::new(self.configuration, rate);
    }
    /// Identify the selected audience mix for master monitoring.
    /// Takes this mixer; returns its active program alias without changing PFL.
    pub(crate) fn program_alias(&self) -> Option<u64> {
        self.active
            .and_then(|c| c.channels.iter().find_map(|v| v.master))
    }
    /// Begin one voice frame.
    /// Takes this mixer; clears only current contributions and source-continuity flags, retaining meters and DSP history.
    pub(crate) fn begin(&mut self) {
        if self.changing && self.voices.iter().all(|v| v.gain.value == 0.0) {
            self.activate(self.configuration);
        }
        for voice in &mut self.voices {
            voice.gain.tick();
            for gain in &mut voice.eq_gain {
                gain.tick();
            }
        }
        self.frames = [[0.0; 2]; 2];
        self.valid = [false; 2];
        for meter in &mut self.meters {
            meter.available = false;
            meter.input_peak *= self.decay;
            meter.output_peak *= self.decay;
        }
    }
    /// Process a selected raw alias once.
    /// Takes its stable alias, native frame, width and continuity; retains mono/stereo order, optional tone and smoothed gain while missing sources fade safely.
    pub(crate) fn feed(&mut self, alias: u64, frame: Frame, width: usize, complete: bool) {
        let Some(cfg) = self.active else {
            return;
        };
        for i in 0..2 {
            let c = cfg.channels[i];
            if c.input != Some(alias) {
                continue;
            }
            let source = [frame[0], frame[if width == 1 { 0 } else { 1 }]];
            let available =
                complete && (1..=2).contains(&width) && source.iter().all(|v| v.is_finite());
            self.valid[i] = available;
            let v = &mut self.voices[i];
            v.availability.set(
                if available { 1.0 } else { 0.0 },
                (self.rate * 0.005).round().max(1.0) as u32,
            );
            if available {
                v.last = source;
            }
            let bands = v.eq_gain.map(|g| g.value);
            let gain = v.gain.value * v.availability.tick();
            let mut output = [0.0; 2];
            for ch in 0..2 {
                v.eq[ch].low_g = bands[0];
                v.eq[ch].mid_g = bands[1];
                v.eq[ch].high_g = bands[2];
                let filtered = v.eq[ch].tick(v.last[ch]);
                output[ch] = if bands == [1.0; 3] {
                    v.last[ch] * gain
                } else {
                    filtered * gain
                };
            }
            if output.iter().any(|v| !v.is_finite()) {
                output = [0.0; 2];
                self.valid[i] = false;
            }
            self.frames[i] = output;
            let meter = &mut self.meters[i];
            meter.available = self.valid[i];
            meter.input_peak = meter.input_peak.max(if available {
                source[0].abs().max(source[1].abs())
            } else {
                0.0
            });
            meter.output_peak = meter.output_peak.max(output[0].abs().max(output[1].abs()));
        }
    }
    /// Advance intentional music ducking.
    /// Takes this mixer's processed mic frame; returns the finite music gain for this sample, before voice addition.
    pub(crate) fn music_gain(&mut self) -> f32 {
        let cfg = self.active.unwrap_or_default();
        let mic = cfg.channels[0];
        let threshold = self.frames[0][0].abs().max(self.frames[0][1].abs()) >= cfg.duck.threshold;
        let active = cfg.duck.enabled
            && match cfg.duck.mode {
                Override::Held => true,
                Override::Off => false,
                Override::Automatic => {
                    !mic.mute
                        && (mic.master.is_some() || mic.booth.is_some())
                        && self.valid[0]
                        && threshold
                }
            };
        let target = if active { self.reduction } else { 1.0 };
        let coefficient = if target < self.duck {
            self.attack
        } else {
            self.release
        };
        self.duck = target + (self.duck - target) * coefficient;
        if !self.duck.is_finite() {
            self.duck = 1.0;
        }
        self.duck
    }
    /// Add independently selected voice destinations.
    /// Takes a graph destination, width, frame, continuity and current master level; adds ordered voice samples without feeding headphone outputs or double-routing inputs.
    pub(crate) fn add(
        &self,
        group: Group,
        width: usize,
        frame: &mut Frame,
        valid: &mut bool,
        master: f32,
    ) {
        let Some(cfg) = self.active else {
            return;
        };
        if cfg.channels.iter().any(|c| match group {
            Group::Output(id) => c.master == Some(id) || c.booth == Some(id),
            Group::Record(id) => c.record == Some(id),
            _ => false,
        }) {
            for value in frame.iter_mut().take(width) {
                *value *= self.duck;
            }
        }
        for i in 0..2 {
            let c = cfg.channels[i];
            let level = match group {
                Group::Output(id) if c.master == Some(id) => master,
                Group::Output(id) if c.booth == Some(id) => 1.0,
                Group::Record(id) if c.record == Some(id) => master,
                _ => continue,
            };
            if width == 1 {
                frame[0] += (self.frames[i][0] + self.frames[i][1]) * 0.5 * level;
            } else {
                for ch in 0..2 {
                    frame[ch] += self.frames[i][ch] * level;
                }
            }
            if !c.mute && c.gain > 0.0 {
                *valid &= self.valid[i];
            }
        }
    }
    /// Sample the native meters without disturbing processing.
    /// Takes this mixer; returns control identity, source/processed peaks and current duck gain, with a 200 ms peak decay.
    pub(crate) fn status(&self) -> Status {
        let status = Status {
            configuration: self.configuration,
            meters: self.meters,
            duck_gain: self.duck,
        };
        status
    }
}
