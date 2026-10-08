use super::*;
use crate::engine::session::{Axis, Reference};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Placement {
    #[default]
    PreFader,
    PostFader,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Timing {
    #[default]
    Beat,
    Manual,
}

#[derive(Clone, Copy, Debug)]
pub enum Control {
    Deck {
        deck: u8,
        enabled: bool,
    },
    Sampler {
        target: Option<Reference>,
    },
    Master(bool),
    Placement(Placement),
    Timing(Timing),
    ManualMs(f32),
    Beats(i8),
    Enabled {
        slot: u8,
        enabled: bool,
    },
    Kind {
        slot: u8,
        kind: crate::engine::FxKind,
    },
}
impl Control {
    /// Validate one native FX control.
    /// Takes the requested setting; returns whether its addresses and numbers are supported before command admission.
    pub fn valid(self) -> bool {
        match self {
            Self::Deck { deck, .. } => deck < 2,
            Self::Enabled { slot, .. } | Self::Kind { slot, .. } => slot < 3,
            Self::ManualMs(value) => value.is_finite() && (1.0..=1998.0).contains(&value),
            Self::Beats(value) => (-4..=3).contains(&value),
            Self::Sampler { .. } | Self::Master(_) | Self::Placement(_) | Self::Timing(_) => true,
        }
    }
}

pub(super) struct Processor {
    slot: crate::engine::master_fx::MasterSlot,
    active: bool,
    quiet: u32,
    input_gate: f32,
}
impl Processor {
    /// Prepare an independent stereo effect history.
    /// Takes the output rate; returns one fixed processor or an allocation failure before audio ownership.
    fn new(rate: f32) -> Result<Self, std::collections::TryReserveError> {
        let mut slot = crate::engine::master_fx::MasterSlot::try_new(rate)?;
        slot.parameter(rate, 0.5);
        Ok(Self {
            slot,
            active: false,
            quiet: 0,
            input_gate: 0.0,
        })
    }
    pub(super) fn parameter(&mut self, rate: f32, value: f32) {
        self.slot.parameter(rate, value);
    }
    pub(super) fn reset(&mut self, kind: crate::engine::FxKind) {
        self.slot.reset(kind);
        self.active = false;
        self.quiet = 0;
        self.input_gate = 0.0;
    }
    /// Render dry input and an independently draining wet history.
    /// Takes stereo input, its enabled gate, wet amount, exact delay frames, effect kind and output rate; returns the mixed frame without callback allocation.
    fn process(
        &mut self,
        input: [f32; 2],
        enabled: bool,
        wet: f32,
        frames: f32,
        kind: crate::engine::FxKind,
        rate: f32,
    ) -> [f32; 2] {
        if enabled {
            self.active = true;
        }
        if !self.active {
            return input;
        }
        let target = if enabled { 1.0 } else { 0.0 };
        let step = 1.0 / (rate * 0.002).max(1.0);
        self.input_gate += (target - self.input_gate).clamp(-step, step);
        self.slot.configure(1.0, f64::from(frames) / 0.75);
        let injection = input.map(|value| value * self.input_gate);
        let tail = self.slot.process(injection, kind, 1.0);
        let output = std::array::from_fn(|channel| {
            input[channel] * (1.0 - wet * self.input_gate) + tail[channel] * wet
        });
        if self.input_gate == 0.0 && tail.iter().all(|value| value.abs() <= 1e-7) {
            self.quiet = self.quiet.saturating_add(1);
            if self.quiet > (rate * 2.0).ceil() as u32 {
                self.reset(kind);
            }
        } else {
            self.quiet = 0;
        }
        output
    }
}

/// Prepare both units' independent deck, sampler and master processors.
/// Takes the output rate; returns three stereo slots for every fixed source, allocated outside the callback.
pub(super) fn prepare_bank(
    rate: f32,
) -> Result<[[Processor; 3]; 4], std::collections::TryReserveError> {
    let make = || -> Result<[Processor; 3], std::collections::TryReserveError> {
        Ok([
            Processor::new(rate)?,
            Processor::new(rate)?,
            Processor::new(rate)?,
        ])
    };
    Ok([make()?, make()?, make()?, make()?])
}

impl EffectBank {
    /// Resolve the echo duration from the current musical clock.
    /// Takes samples per beat and output rate; returns the exact supported delay-frame count, capped by the prepared two-second history.
    pub fn frames(&self, samples_per_beat: f64, rate: f32) -> f32 {
        let frames = match self.timing {
            Timing::Beat => samples_per_beat * 0.75 * 2_f64.powi(i32::from(self.beats)),
            Timing::Manual => f64::from(self.manual_ms) * f64::from(rate) / 1000.0,
        };
        frames.clamp(1.0, f64::from(rate) * 2.0 - 2.0) as f32
    }
}

impl State {
    /// Resolve retained sampler bus identities before the render loop.
    /// Takes the current project layout; retires old project/deleted targets and retains a draining original route after disengagement.
    pub(in crate::engine) fn resolve_fx_samplers(
        &mut self,
        layout: &crate::engine::session::Layout,
    ) {
        for bank in 0..2 {
            let target = self.status.fx[bank]
                .sampler
                .or(self.sampler_history_target[bank]);
            self.sampler_slots[bank] = target.and_then(|reference| {
                layout.tracks.iter().enumerate().find_map(|(slot, _)| {
                    layout
                        .resolves(Axis::Track, slot, reference)
                        .then_some(slot)
                })
            });
            if target.is_some() && self.sampler_slots[bank].is_none() {
                self.status.fx[bank].sampler = None;
                self.status.fx[bank].tails[2] = false;
                self.sampler_history_target[bank] = None;
                for (slot, processor) in self.deck_fx[bank][2].iter_mut().enumerate() {
                    processor.reset(self.status.fx[bank].kinds[slot]);
                }
            }
        }
    }

    /// Reconstruct fixed surface histories at the actual output rate.
    /// Takes a rate before renderer mutation; returns prepared state retaining controls and routes while retiring old-rate tail/input ownership.
    pub(in crate::engine) fn at_rate(
        &self,
        rate: f32,
    ) -> Result<Self, std::collections::TryReserveError> {
        let mut prepared = Self::new(rate)?;
        prepared.status = self.status;
        prepared.fx_recall.receipt = self.fx_recall.receipt;
        prepared.fx_recall.receipt.phase = match self.fx_recall.receipt.phase { super::super::dj_fx_recall::Phase::FadingOut | super::super::dj_fx_recall::Phase::FadingIn => super::super::dj_fx_recall::Phase::Stale, phase => phase };
        prepared.status.fx_recall = prepared.fx_recall.receipt;
        prepared.assignments = self.assignments;
        prepared.sends = self.sends;
        prepared.master_saved = self.master_saved;
        prepared.shift_owners = self.shift_owners;
        for bank in 0..2 {
            prepared.status.fx[bank].tails = [false; 4];
            prepared.sampler_history_target[bank] = prepared.status.fx[bank].sampler;
            for source in &mut prepared.deck_fx[bank] {
                for (processor, value) in source.iter_mut().zip(prepared.status.fx[bank].parameter)
                {
                    processor.parameter(rate, value);
                }
            }
        }
        prepared.status.sampler_playing = [false; 16];
        Ok(prepared)
    }

    /// Retire DJ FX histories when the unique audio owner stops its stream.
    /// Takes no data; clears fixed history generations without allocation while keeping reviewed source assignments and controls.
    pub(in crate::engine) fn reset_fx_histories(&mut self) {
        self.fx_recall.retire();
        self.status.fx_recall = self.fx_recall.receipt;
        for channel in &mut self.channel_fx { channel.reset(); }
        for bank in 0..2 {
            for source in &mut self.deck_fx[bank] {
                for (slot, processor) in source.iter_mut().enumerate() {
                    processor.reset(self.status.fx[bank].kinds[slot]);
                }
            }
            self.status.fx[bank].tails = [false; 4];
            self.sampler_history_target[bank] = self.status.fx[bank].sampler;
        }
    }

    fn fx_source(
        &mut self,
        source: usize,
        placement: Placement,
        mut input: [f32; 2],
        samples_per_beat: f64,
        rate: f32,
    ) -> [f32; 2] {
        for bank in 0..2 {
            let settings = self.status.fx[bank];
            if settings.placement != placement {
                continue;
            }
            let assigned = match source {
                0 | 1 => settings.assigned[source],
                2 => settings.sampler.is_some(),
                3 => settings.master,
                _ => false,
            };
            if (!assigned || !settings.on.iter().any(|on| *on)) && !settings.tails[source] {
                continue;
            }
            let dry = input;
            let frames = settings.frames(samples_per_beat, rate);
            let mut active = false;
            for (slot, processor) in self.deck_fx[bank][source].iter_mut().enumerate() {
                input = processor.process(
                    input,
                    assigned && settings.on[slot],
                    settings.wet[slot],
                    frames,
                    settings.kinds[slot],
                    rate,
                );
                active |= processor.active;
            }
            input = blend_recall(dry, input, self.fx_recall.gain);
            self.status.fx[bank].tails[source] = active;
        }
        input
    }

    /// Render a deck's independent FX before or after its crossfader.
    /// Takes deck, placement, original stereo frame, musical clock and output rate; returns dry plus its own retained tails.
    pub(in crate::engine) fn deck_fx_at(
        &mut self,
        deck: usize,
        placement: Placement,
        input: [f32; 2],
        samples_per_beat: f64,
        rate: f32,
    ) -> [f32; 2] {
        self.fx_source(deck, placement, input, samples_per_beat, rate)
    }

    /// Render only the explicitly selected sampler destination for each unit.
    /// Takes all pad buses, placement, musical clock and output rate; changes the original target bus while retaining every other bus exactly.
    pub(in crate::engine) fn sampler_fx_at(
        &mut self,
        buses: &mut [[f32; 2]; crate::engine::session::MAX_TRACKS],
        placement: Placement,
        samples_per_beat: f64,
        rate: f32,
    ) {
        for bank in 0..2 {
            let settings = self.status.fx[bank];
            if settings.placement != placement {
                continue;
            }
            let Some(track) = self.sampler_slots[bank] else {
                continue;
            };
            if !settings.on.iter().any(|on| *on) && !settings.tails[2] {
                continue;
            }
            let dry = buses[track];
            let mut frame = dry;
            let frames = settings.frames(samples_per_beat, rate);
            let mut active = false;
            for (slot, processor) in self.deck_fx[bank][2].iter_mut().enumerate() {
                frame = processor.process(
                    frame,
                    settings.sampler.is_some() && settings.on[slot],
                    settings.wet[slot],
                    frames,
                    settings.kinds[slot],
                    rate,
                );
                active |= processor.active;
            }
            buses[track] = blend_recall(dry, frame, self.fx_recall.gain);
            self.status.fx[bank].tails[2] = active;
            if !active && settings.sampler.is_none() {
                self.sampler_history_target[bank] = None;
            }
        }
    }

    /// Render independently assigned master FX around the master fader.
    /// Takes placement, stereo program frame, musical clock and output rate; returns unit output and retained tails before safety/limiting.
    pub(in crate::engine) fn master_fx_at(
        &mut self,
        placement: Placement,
        input: [f32; 2],
        samples_per_beat: f64,
        rate: f32,
    ) -> [f32; 2] {
        self.fx_source(3, placement, input, samples_per_beat, rate)
    }
}

impl RtEngine {
    /// Apply one admitted DJ FX edit to its exact unit.
    /// Takes bank and validated setting; returns whether the current project/source and live placement allow it, preserving state on refusal.
    pub(super) fn dj_fx_control(&mut self, bank: u8, control: Control) -> bool {
        if bank >= 2 || !control.valid() {
            return false;
        }
        let bank = usize::from(bank);
        let mut state = self.surface.status.fx[bank];
        match control {
            Control::Deck { deck, enabled } => state.assigned[usize::from(deck)] = enabled,
            Control::Master(enabled) => state.master = enabled,
            Control::Sampler { target } => {
                if target.is_some_and(|reference| {
                    !self
                        .session
                        .tracks
                        .iter()
                        .enumerate()
                        .any(|(slot, _)| self.session.resolves(Axis::Track, slot, reference))
                }) {
                    return false;
                }
                if target.is_some() && target != self.surface.sampler_history_target[bank] {
                    if state.on.iter().any(|on| *on) || state.tails[2] {
                        return false;
                    }
                    for (slot, processor) in self.surface.deck_fx[bank][2].iter_mut().enumerate() {
                        processor.reset(state.kinds[slot]);
                    }
                }
                if target.is_some() {
                    self.surface.sampler_history_target[bank] = target;
                }
                state.sampler = target;
            }
            Control::Placement(placement) => {
                if placement != state.placement
                    && (state.on.iter().any(|on| *on) || state.tails.iter().any(|tail| *tail))
                {
                    return false;
                }
                state.placement = placement;
            }
            Control::Timing(timing) => state.timing = timing,
            Control::ManualMs(value) => state.manual_ms = value,
            Control::Beats(value) => state.beats = value,
            Control::Enabled { slot, enabled } => state.on[usize::from(slot)] = enabled,
            Control::Kind { slot, kind } => {
                state.kinds[usize::from(slot)] = kind;
                for source in &mut self.surface.deck_fx[bank] {
                    source[usize::from(slot)].reset(kind);
                }
            }
        }
        self.surface.status.fx[bank] = state;
        true
    }
}

#[cfg(test)]
mod tests;

fn blend_recall(dry: [f32; 2], wet: [f32; 2], gain: f32) -> [f32; 2] {
    if gain == 1.0 { wet } else if gain == 0.0 { dry } else { std::array::from_fn(|channel| dry[channel] + (wet[channel] - dry[channel]) * gain) }
}
impl State {
    /// Advance one atomic reviewed preset at the actual output-frame boundary.
    /// Takes the current project; fades both unit contributions, applies both together at zero gain, retires their histories and resolves new sampler routes before audio is rendered.
    pub(in crate::engine) fn fx_recall_frame(&mut self, layout: &crate::engine::session::Layout) {
        if !matches!(self.fx_recall.receipt.phase, crate::engine::dj_fx_recall::Phase::FadingOut | crate::engine::dj_fx_recall::Phase::FadingIn | crate::engine::dj_fx_recall::Phase::Stale) { return; }
        if let Some(units) = self.fx_recall.frame(self.status.fx.map(Into::into), layout) {
            for bank in 0..2 {
                for source in &mut self.deck_fx[bank] {
                    for (slot, processor) in source.iter_mut().enumerate() {
                        processor.reset(self.status.fx[bank].kinds[slot]);
                        processor.reset(units[bank].kinds[slot]);
                        processor.parameter(self.fx_rate, units[bank].parameter[slot]);
                    }
                }
            }
            self.status.fx = units.map(|settings| settings.bank());
            self.sampler_history_target = self.status.fx.map(|bank| bank.sampler);
            self.resolve_fx_samplers(layout);
        }
        self.status.fx_recall = self.fx_recall.receipt;
    }
}
