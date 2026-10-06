use super::{Command, RtEngine, View};

#[derive(Clone, Copy, Debug)]
pub enum Input {
    Shift {
        source: u64,
        on: bool,
    },
    Apc {
        channel: u8,
        control: u8,
        value: u8,
        note: bool,
    },
    FxValue {
        bank: u8,
        slot: u8,
        parameter: bool,
        value: f32,
    },
    FxButton {
        bank: u8,
        control: u8,
        delta: i16,
    },
    SamplerVolume(f32),
    SamplerPressure {
        source: u64,
        pad: u8,
        value: f32,
    },
    TrackSend {
        track: u8,
        send: u8,
        value: f32,
    },
    Recording(bool),
}
impl Input {
    /// Validate a surface packet before admission.
    /// Takes this decoded packet; returns whether every address is a MIDI data byte.
    pub fn valid(self) -> bool {
        match self {
            Self::Apc {
                channel,
                control,
                value,
                ..
            } => channel < 16 && control < 128 && value < 128,
            Self::FxValue {
                bank, slot, value, ..
            } => bank < 2 && slot < 3 && value.is_finite() && (0.0..=1.0).contains(&value),
            Self::FxButton {
                bank,
                control,
                delta,
            } => bank < 2 && control < 128 && (-64..=63).contains(&delta),
            Self::SamplerVolume(value) => value.is_finite() && (0.0..=1.0).contains(&value),
            Self::SamplerPressure { pad, value, .. } => {
                pad < 16 && value.is_finite() && (0.0..=1.0).contains(&value)
            }
            Self::TrackSend { track, send, value } => {
                usize::from(track) < super::session::MAX_TRACKS
                    && send < 2
                    && value.is_finite()
                    && (0.0..=1.0).contains(&value)
            }
            Self::Recording(_) | Self::Shift { .. } => true,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, serde::Serialize)]
pub struct Status {
    pub track_offset: usize,
    pub scene_offset: usize,
    pub knob_mode: u8,
    pub send: u8,
    pub device: usize,
    pub parameter_bank: usize,
    pub device_lock: Option<usize>,
    pub bank_lock: bool,
    pub shift: bool,
    pub assignments: [u8; 8],
    pub sends: [[f32; 2]; 8],
    pub device_values: [f32; 8],
    pub device_on: bool,
    pub device_master: bool,
    pub master_parameter: [f32; 3],
    pub fx: [EffectBank; 2],
    pub sampler_volume: f32,
    pub sampler_playing: [bool; 16],
}

#[derive(Clone, Copy, Debug, serde::Serialize)]
pub struct EffectBank {
    pub kinds: [super::FxKind; 3],
    pub wet: [f32; 3],
    pub on: [bool; 3],
    pub parameter: [f32; 3],
    pub beats: i8,
    pub assigned: [bool; 2],
}
impl Default for EffectBank {
    fn default() -> Self {
        Self {
            kinds: [
                super::FxKind::Echo,
                super::FxKind::Reverb,
                super::FxKind::Filter,
            ],
            wet: [0.5; 3],
            on: [false; 3],
            parameter: [0.5; 3],
            beats: 0,
            assigned: [false; 2],
        }
    }
}

pub(super) struct State {
    pub status: Status,
    pub assignments: [u8; super::session::MAX_TRACKS],
    pub sends: [[f32; 2]; super::session::MAX_TRACKS],
    processors: [super::master_fx::MasterSlot; 2],
    send_input: [[f32; 2]; 2],
    deck_fx: [[[super::master_fx::MasterSlot; 3]; 2]; 2],
    pad_owners: [Option<(u64, u8)>; super::control::MAX_COMMANDS],
    pad_gain: [f32; 16],
    master_saved: [f32; 3],
    shift_owners: [Option<u64>; super::control::MAX_COMMANDS],
    track_gain: [super::mixer_gain::GainPair; super::session::MAX_TRACKS],
}
impl State {
    /// Prepare the two stereo send processors outside the audio callback.
    /// Takes the output rate; returns fixed control state and fallibly allocated histories.
    pub fn new(sr: f32) -> Result<Self, std::collections::TryReserveError> {
        let mut status = Status {
            sampler_volume: 1.0,
            ..Status::default()
        };
        status.fx[0].assigned[0] = true;
        status.fx[1].assigned[1] = true;
        Ok(Self {
            status,
            assignments: [0; super::session::MAX_TRACKS],
            sends: [[0.0; 2]; super::session::MAX_TRACKS],
            processors: [
                super::master_fx::MasterSlot::try_new(sr)?,
                super::master_fx::MasterSlot::try_new(sr)?,
            ],
            send_input: [[0.0; 2]; 2],
            pad_owners: [None; super::control::MAX_COMMANDS],
            pad_gain: [1.0; 16],
            master_saved: [0.5; 3],
            shift_owners: [None; super::control::MAX_COMMANDS],
            deck_fx: [prepare_bank(sr)?, prepare_bank(sr)?],
            track_gain: std::array::from_fn(|_| {
                let mut ramp = super::mixer_gain::GainPair::default();
                ramp.prepare([1.0; 2], sr, |left, right| [left, right]);
                ramp
            }),
        })
    }
    /// Prepare assigned track gains once per callback block.
    /// Takes output rate and current crossfader controls; keeps cuts on the same short ramp as the deck mixer.
    pub fn prepare(&mut self, sr: f32, position: f32, curve: f32) {
        let gains = super::mixer_gain::crossfader_gains(position, curve);
        for (index, ramp) in self.track_gain.iter_mut().enumerate() {
            let gain = match self.assignments[index] {
                1 => gains[0],
                2 => gains[1],
                _ => 1.0,
            };
            ramp.prepare([gain, gain], sr, |left, right| [left, right]);
        }
    }
    /// Route one track through its crossfader assignment and sends.
    /// Takes track index and stereo output; returns the assigned dry output.
    pub fn track(&mut self, track: usize, input: [f32; 2]) -> [f32; 2] {
        let gains = self.track_gain[track].tick();
        let output = std::array::from_fn(|channel| input[channel] * gains[channel]);
        for (bus, amount) in self.send_input.iter_mut().zip(self.sends[track]) {
            for channel in 0..2 {
                bus[channel] += output[channel] * amount;
            }
        }
        output
    }
    /// Render both accumulated sends and clear their input frame.
    /// Takes samples per beat; returns wet stereo reverb and echo, including their tails.
    pub fn render_sends(&mut self, spb: f64) -> [f32; 2] {
        let mut output = [0.0; 2];
        for (index, input) in std::mem::take(&mut self.send_input).into_iter().enumerate() {
            self.processors[index].configure(1.0, spb);
            let kind = [super::FxKind::Reverb, super::FxKind::Echo][index];
            let wet = self.processors[index].process(input, kind, 1.0);
            for channel in 0..2 {
                output[channel] += wet[channel];
            }
        }
        output
    }
    /// Render independent Pioneer effect banks for one assigned deck.
    /// Takes deck, stereo input and beat duration; returns the serial effect output.
    pub fn deck(&mut self, deck: usize, mut input: [f32; 2], spb: f64) -> [f32; 2] {
        for (bank, processors) in self.status.fx.iter().zip(&mut self.deck_fx) {
            if !bank.assigned[deck] || !bank.on.iter().any(|on| *on) {
                continue;
            }
            for (index, processor) in processors[deck].iter_mut().enumerate() {
                let wet = if bank.on[index] { bank.wet[index] } else { 0.0 };
                processor.configure(wet, spb * 2_f64.powi(i32::from(bank.beats)));
                input = processor.process(input, bank.kinds[index], wet);
            }
        }
        input
    }
}

impl RtEngine {
    /// Resolve APC controls against the current session window.
    /// Takes a validated input; applies real transport, mixer, clip and effect commands.
    pub(super) fn surface_input(&mut self, input: Input) {
        if !input.valid() {
            return;
        }
        match input {
            Input::Shift { source, on } => {
                let owners = &mut self.surface.shift_owners;
                if let Some(owner) = owners.iter_mut().find(|owner| **owner == Some(source)) {
                    if !on {
                        *owner = None;
                    }
                } else if on {
                    if let Some(owner) = owners.iter_mut().find(|owner| owner.is_none()) {
                        *owner = Some(source);
                    }
                }
                self.surface.status.shift = owners.iter().any(Option::is_some);
            }
            Input::Apc {
                channel,
                control,
                value,
                note,
            } => self.apc_input(channel, control, value, note),
            Input::SamplerVolume(value) => self.surface.status.sampler_volume = value,
            Input::TrackSend { track, send, value } => {
                self.surface.sends[usize::from(track)][usize::from(send)] = value
            }
            Input::Recording(on) => {
                if self.recording != on {
                    self.apply(Command::Record);
                }
            }
            Input::SamplerPressure { source, pad, value } => {
                if !self.surface.pad_owners.contains(&Some((source, pad))) {
                    return;
                }
                if value == 0.0 {
                    self.surface_sampler(source, pad, false, 0.0);
                } else if let Some(voice) = &mut self.pad_voices[usize::from(pad)] {
                    voice.gain = self.surface.pad_gain[usize::from(pad)] * value;
                }
                for voice in &mut self.sampler_poly.voices {
                    if voice.input == Some(super::dsp::InputKey::Pad(pad)) {
                        voice.vel = 0.9 * value;
                    }
                }
            }
            Input::FxValue {
                bank,
                slot,
                parameter,
                value,
            } => {
                let state = &mut self.surface.status.fx[usize::from(bank)];
                if parameter {
                    state.parameter[usize::from(slot)] = value;
                    for deck in &mut self.surface.deck_fx[usize::from(bank)] {
                        deck[usize::from(slot)].parameter(self.sr, value);
                    }
                } else {
                    state.wet[usize::from(slot)] = value;
                }
            }
            Input::FxButton {
                bank,
                control,
                delta,
            } => {
                let bank = usize::from(bank);
                let state = &mut self.surface.status.fx[bank];
                match control {
                    0x47..=0x49 => {
                        let slot = usize::from(control - 0x47);
                        state.on[slot] = !state.on[slot];
                    }
                    0x63..=0x65 => {
                        let slot = usize::from(control - 0x63);
                        state.kinds[slot] = state.kinds[slot].next();
                        for deck in &mut self.surface.deck_fx[bank] {
                            deck[slot].reset(state.kinds[slot]);
                        }
                    }
                    0 => state.beats = (i16::from(state.beats) + delta).clamp(-4, 3) as i8,
                    0x10 => {
                        for parameter in &mut state.parameter {
                            *parameter = (*parameter + f32::from(delta) / 127.0).clamp(0.0, 1.0);
                        }
                    }
                    0x40 | 0x43 => state.beats = 0,
                    0x4c | 0x50 | 0x4e | 0x52 => state.assigned[0] = !state.assigned[0],
                    0x4d | 0x51 | 0x4f | 0x53 => state.assigned[1] = !state.assigned[1],
                    _ => {}
                }
                if control == 0x10 {
                    for deck in &mut self.surface.deck_fx[bank] {
                        for (processor, value) in deck.iter_mut().zip(state.parameter) {
                            processor.parameter(self.sr, value);
                        }
                    }
                }
            }
        }
    }
    /// Keep sampler gates owned by their physical source.
    /// Takes source, pad, gate and attack level; releases a pad only when its last source releases it.
    pub(super) fn surface_sampler(&mut self, source: u64, pad: u8, on: bool, pressure: f32) {
        if pad >= 16 || !pressure.is_finite() || !(0.0..=1.0).contains(&pressure) {
            return;
        }
        let key = (source, pad);
        let owner = self
            .surface
            .pad_owners
            .iter()
            .position(|owner| *owner == Some(key));
        if on {
            if owner.is_some() {
                return;
            }
            let Some(slot) = self
                .surface
                .pad_owners
                .iter_mut()
                .find(|owner| owner.is_none())
            else {
                return;
            };
            *slot = Some(key);
            self.apply_sampler_pad(pad, true, pressure);
            if let Some(voice) = &self.pad_voices[usize::from(pad)] {
                self.surface.pad_gain[usize::from(pad)] = if pressure > 0.0 {
                    voice.gain / pressure
                } else {
                    0.0
                };
            }
        } else if let Some(owner) = owner {
            self.surface.pad_owners[owner] = None;
            if !self
                .surface
                .pad_owners
                .iter()
                .flatten()
                .any(|(_, owned)| *owned == pad)
            {
                self.apply_sampler_pad(pad, false, pressure);
            }
        }
    }
    /// Clear held controller inputs after a safety stop.
    /// Takes the renderer; clears source ownership without restoring any transport.
    pub(super) fn release_surface_inputs(&mut self) {
        self.surface.pad_owners.fill(None);
        self.surface.shift_owners.fill(None);
        self.surface.status.shift = false;
    }
    fn apc_input(&mut self, channel: u8, control: u8, value: u8, note: bool) {
        let status = self.surface.status;
        let track = status.track_offset + usize::from(channel);
        let value_f = f32::from(value) / 127.0;
        let delta = if value < 64 {
            i16::from(value)
        } else {
            i16::from(value) - 128
        };
        if note {
            if control == 0x62 {
                self.surface.status.shift = value != 0;
                return;
            }
            if value == 0 {
                return;
            }
            let command = match control {
                0..=0x27 => {
                    let track = status.track_offset + usize::from(control % 8);
                    let scene = status.scene_offset + usize::from(control / 8);
                    if track >= self.tracks.len() || scene >= self.scene_fx.len() {
                        return;
                    }
                    if status.shift {
                        Command::Select { track, scene }
                    } else {
                        Command::LaunchClip {
                            track: track as u8,
                            scene: scene as u16,
                        }
                    }
                }
                0x30..=0x34 | 0x42 if channel < 8 && track < self.tracks.len() => match control {
                    0x30 => Command::Arm { track: track as u8 },
                    0x31 => Command::Solo { track: track as u8 },
                    0x32 => Command::Mute { track: track as u8 },
                    0x33 => {
                        if status.device_lock.is_none() {
                            self.surface.status.device_master = false;
                        }
                        Command::Select {
                            track,
                            scene: self.selected_scene,
                        }
                    }
                    0x34 => Command::StopTrack { track: track as u8 },
                    _ => {
                        self.surface.assignments[track] = (self.surface.assignments[track] + 1) % 3;
                        return;
                    }
                },
                0x3a | 0x3b => {
                    let count = if status.device_master {
                        3
                    } else {
                        self.tracks
                            .get(status.device_lock.unwrap_or(self.selected_track))
                            .map_or(0, |track| track.fx.slots.len())
                    };
                    self.surface.status.device = if control == 0x3a {
                        status.device.saturating_sub(1)
                    } else {
                        (status.device + 1).min(count.saturating_sub(1))
                    };
                    return;
                }
                0x3c | 0x3d => {
                    self.surface.status.parameter_bank = usize::from(control == 0x3d);
                    return;
                }
                0x3e => {
                    if status.device_master {
                        let slot = status.device.min(2);
                        let wet = if self.fx_wet[slot] > 0.0 {
                            self.surface.master_saved[slot] = self.fx_wet[slot];
                            0.0
                        } else {
                            self.surface.master_saved[slot]
                        };
                        self.apply(Command::FxWet {
                            slot: slot as u8,
                            value: wet,
                        });
                        return;
                    }
                    if let Some(slot) = self
                        .tracks
                        .get_mut(status.device_lock.unwrap_or(self.selected_track))
                        .and_then(|track| track.fx.slots.get_mut(status.device))
                    {
                        slot.on = !slot.on;
                        self.project.edited();
                    }
                    return;
                }
                0x3f => {
                    self.surface.status.device_lock = if status.device_lock.is_some() {
                        None
                    } else {
                        Some(if status.device_master {
                            super::session::MAX_TRACKS
                        } else {
                            self.selected_track
                        })
                    };
                    return;
                }
                0x40 => Command::SetView(if self.view == View::Compose {
                    View::Session
                } else {
                    View::Compose
                }),
                0x41 => {
                    if self.fx_view >= 0 {
                        Command::CloseFx
                    } else {
                        Command::OpenFxTrack(self.selected_track as u8)
                    }
                }
                0x50 => {
                    if status.device_lock.is_none() {
                        self.surface.status.device_master = true;
                        self.surface.status.device = status.device.min(2);
                    }
                    Command::CloseFx
                }
                0x51 => {
                    for track in 0..self.tracks.len() {
                        self.apply(Command::StopTrack { track: track as u8 });
                    }
                    return;
                }
                0x52..=0x56 => Command::LaunchScene {
                    scene: (status.scene_offset + usize::from(control - 0x52)) as u16,
                },
                0x57..=0x59 => {
                    self.surface.status.knob_mode = control - 0x57;
                    if control == 0x58 && status.knob_mode == 1 {
                        self.surface.status.send ^= 1;
                    }
                    return;
                }
                0x5a => Command::Metronome,
                0x5b => Command::Play,
                0x5c => Command::Stop,
                0x5d | 0x66 => Command::Record,
                0x5e..=0x61 => {
                    if status.bank_lock {
                        return;
                    }
                    match control {
                        0x5e => {
                            self.surface.status.scene_offset = status
                                .scene_offset
                                .saturating_sub(if status.shift { 5 } else { 1 })
                        }
                        0x5f => {
                            self.surface.status.scene_offset = (status.scene_offset
                                + if status.shift { 5 } else { 1 })
                            .min(self.scene_fx.len().saturating_sub(5))
                        }
                        0x60 => {
                            self.surface.status.track_offset = (status.track_offset
                                + if status.shift { 8 } else { 1 })
                            .min(self.tracks.len().saturating_sub(8))
                        }
                        _ => {
                            self.surface.status.track_offset = status
                                .track_offset
                                .saturating_sub(if status.shift { 8 } else { 1 })
                        }
                    }
                    return;
                }
                0x63 => Command::Tap(std::time::Instant::now()),
                0x64 | 0x65 => Command::NudgeBpm(if control == 0x64 { -0.1 } else { 0.1 }),
                0x67 => {
                    self.surface.status.bank_lock = !status.bank_lock;
                    return;
                }
                _ => return,
            };
            self.apply(command);
        } else {
            let command = match control {
                7 if channel < 8 && track < self.tracks.len() => Command::TrackGain {
                    track: track as u8,
                    value: value_f,
                },
                0x0d => Command::NudgeBpm(f32::from(delta) * if status.shift { 0.1 } else { 1.0 }),
                0x0e => Command::Master(value_f),
                0x0f => Command::Xfader(value_f),
                0x10..=0x17 => {
                    if status.device_master {
                        let index = usize::from(control - 0x10);
                        match index {
                            0 | 2 | 4 => self.apply(Command::FxWet {
                                slot: (index / 2) as u8,
                                value: value_f,
                            }),
                            1 | 3 | 5 => {
                                let slot = index / 2;
                                self.surface.status.master_parameter[slot] = value_f;
                                self.master_fx[slot].parameter(self.sr, value_f);
                            }
                            6 => self.apply(Command::CueMix(value_f)),
                            _ => self.apply(Command::Master(value_f)),
                        }
                        return;
                    }
                    let Some(track) = self
                        .tracks
                        .get_mut(status.device_lock.unwrap_or(self.selected_track))
                    else {
                        return;
                    };
                    let parameter = usize::from(control - 0x10) + status.parameter_bank * 8;
                    let slot_index = status.device + parameter / 5;
                    if let Some(slot) = track.fx.slots.get_mut(slot_index) {
                        slot.set_control(
                            if parameter % 5 == 0 {
                                None
                            } else {
                                Some((parameter % 5 - 1) as u8)
                            },
                            value_f,
                        );
                        self.project.edited();
                    }
                    return;
                }
                0x2f => Command::Monitor(super::monitor::Control::Volume(
                    (self.monitor.status.volume + f32::from(delta) / 127.0).clamp(0.0, 1.0),
                )),
                0x30..=0x37 => {
                    let track = status.track_offset + usize::from(control - 0x30);
                    if track >= self.tracks.len() {
                        return;
                    }
                    match status.knob_mode {
                        1 => {
                            self.surface.sends[track][usize::from(status.send)] = value_f;
                            return;
                        }
                        2 => Command::TrackGain {
                            track: track as u8,
                            value: value_f * 1.5,
                        },
                        _ => Command::TrackPan {
                            track: track as u8,
                            value: value_f,
                        },
                    }
                }
                0x40 if value != 0 => Command::Record,
                _ => return,
            };
            self.apply(command);
        }
    }
    /// Publish the values represented by the current controller window.
    /// Takes the renderer; returns fixed feedback data without cloning processor storage.
    pub(super) fn surface_status(&self) -> Status {
        let mut status = self.surface.status;
        for (index, voice) in self.pad_voices.iter().enumerate() {
            status.sampler_playing[index] = voice.is_some();
        }
        for voice in &self.sampler_poly.voices {
            if let Some(super::dsp::InputKey::Pad(pad)) = voice.input {
                if voice.env.active() {
                    status.sampler_playing[usize::from(pad)] = true;
                }
            }
        }
        for index in 0..8 {
            let track = status.track_offset + index;
            if track < super::session::MAX_TRACKS {
                status.assignments[index] = self.surface.assignments[track];
                status.sends[index] = self.surface.sends[track];
            }
            if status.device_master {
                status.device_values[index] = match index {
                    0 | 2 | 4 => self.fx_wet[index / 2],
                    1 | 3 | 5 => status.master_parameter[index / 2],
                    6 => self.cue_mix,
                    _ => self.master,
                };
                status.device_on = self.fx_wet[status.device.min(2)] > 0.0;
                continue;
            }
            let parameter = index + status.parameter_bank * 8;
            if let Some(slot) = self
                .tracks
                .get(status.device_lock.unwrap_or(self.selected_track))
                .and_then(|track| track.fx.slots.get(status.device + parameter / 5))
            {
                status.device_values[index] = if parameter % 5 == 0 {
                    slot.mix
                } else {
                    slot.p[parameter % 5 - 1]
                };
                if index == 0 {
                    status.device_on = slot.on;
                }
            }
        }
        status
    }
}

/// Prepare stereo histories for one effect bank.
/// Takes the output rate; returns both decks' processors with the published initial parameters.
fn prepare_bank(
    sr: f32,
) -> Result<[[super::master_fx::MasterSlot; 3]; 2], std::collections::TryReserveError> {
    let mut bank = [
        [
            super::master_fx::MasterSlot::try_new(sr)?,
            super::master_fx::MasterSlot::try_new(sr)?,
            super::master_fx::MasterSlot::try_new(sr)?,
        ],
        [
            super::master_fx::MasterSlot::try_new(sr)?,
            super::master_fx::MasterSlot::try_new(sr)?,
            super::master_fx::MasterSlot::try_new(sr)?,
        ],
    ];
    for deck in &mut bank {
        for processor in deck {
            processor.parameter(sr, 0.5);
        }
    }
    Ok(bank)
}
