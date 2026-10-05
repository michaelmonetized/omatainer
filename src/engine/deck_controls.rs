use super::{Command, DeckTransition, RtEngine};
use serde::{Deserialize, Serialize};
use std::time::Instant;
#[cfg(test)]
mod tests;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Button {
    Reverse,
    Bleep,
    BendDown,
    BendUp,
    Delete,
    Cue,
    HotCue(u8),
    Roll(u8),
    Slice(u8),
}
impl Button {
    /// Locate a bounded button counter.
    /// Takes this validated button; returns its fixed counter index.
    fn index(self) -> usize {
        match self {
            Self::Reverse => 0,
            Self::Bleep => 1,
            Self::BendDown => 2,
            Self::BendUp => 3,
            Self::Delete => 4,
            Self::Cue => 5,
            Self::HotCue(pad) => 6 + usize::from(pad),
            Self::Roll(pad) => 14 + usize::from(pad),
            Self::Slice(pad) => 22 + usize::from(pad),
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum Control {
    Hold { button: Button, on: bool },
    Keylock,
    PitchRange,
    Strip { value: f32 },
    LoopMode,
    LoopButton { index: u8 },
    LoopToggle,
    LoopSelect,
    Reloop,
    LoopScale { double: bool },
    LoopShift { forward: bool },
    Tap,
    StartTime { value: f32 },
    StopTime { value: f32 },
    TrackStart,
    Slip,
    PadMode { mode: u8 },
    Parameter { mode: u8, up: bool, shifted: bool },
    HotLoop { pad: u8, clear: bool },
    AutoLoopPad { pad: u8 },
    ManualPad { pad: u8 },
    SyncOff,
}
impl Control {
    /// Validate a performance control before queue admission.
    /// Takes this control; returns whether its indices and values are supported.
    pub fn valid(self) -> bool {
        match self {
            Self::Strip { value } | Self::StartTime { value } | Self::StopTime { value } => {
                value.is_finite() && (0.0..=1.0).contains(&value)
            }
            Self::LoopButton { index } => index < 4,
            Self::Hold {
                button: Button::HotCue(pad),
                ..
            } => pad < 8,
            Self::Hold {
                button: Button::Roll(pad) | Button::Slice(pad),
                ..
            }
            | Self::HotLoop { pad, .. } => pad < 8,
            Self::AutoLoopPad { pad } | Self::ManualPad { pad } => pad < 8,
            Self::PadMode { mode } | Self::Parameter { mode, .. } => mode < 8,
            _ => true,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct Status {
    pub reverse: bool,
    pub bleep: bool,
    pub bend: f32,
    pub delete: bool,
    pub auto_loop: bool,
    pub loop_slot: u8,
    pub loop_edit: u8,
    pub start_seconds: f32,
    pub stop_seconds: f32,
    pub braking: bool,
    pub slip: bool,
    pub pad_mode: u8,
    pub roll: Option<u8>,
    pub slice: Option<u8>,
    pub roll_scale: i8,
    pub slice_domain: u8,
    pub hotloops: [bool; 8],
}

#[derive(Clone, Debug)]
pub(super) struct State {
    owners: [Option<(u64, Button)>; super::control::MAX_COMMANDS],
    counts: [u16; 30],
    pub forward: Option<f64>,
    preview: Option<(u64, Button)>,
    delete: bool,
    delete_used: bool,
    auto_loop: bool,
    auto_button: Option<u8>,
    loops: [Option<(f64, f64)>; 8],
    selected: usize,
    edit: u8,
    edit_ticks: Option<i64>,
    taps: [Option<Instant>; 8],
    tap_cursor: usize,
    pub start: f32,
    pub stop: f32,
    scale: f32,
    step: f32,
    remaining: u32,
    pub braking: bool,
    pub brake_rate: f32,
    slip: bool,
    pub pad_mode: u8,
    pub roll_scale: i8,
    slice_domain: u8,
    slice_quant: u8,
    performance_forward: Option<f64>,
    saved_loop: Option<(bool, f64, f64)>,
    slip_forward: Option<f64>,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct LoopHistory {
    loops: [Option<(f64, f64)>; 8],
    selected: usize,
    auto_button: Option<u8>,
}
impl Default for State {
    fn default() -> Self {
        Self {
            owners: [None; super::control::MAX_COMMANDS],
            counts: [0; 30],
            forward: None,
            preview: None,
            delete: false,
            delete_used: false,
            auto_loop: false,
            auto_button: None,
            loops: [None; 8],
            selected: 0,
            edit: 0,
            edit_ticks: None,
            taps: [None; 8],
            tap_cursor: 0,
            start: 0.0,
            stop: 0.0,
            scale: 1.0,
            step: 0.0,
            remaining: 0,
            braking: false,
            brake_rate: 0.0,
            slip: false,
            pad_mode: 0,
            roll_scale: 0,
            slice_domain: 3,
            slice_quant: 0,
            performance_forward: None,
            saved_loop: None,
            slip_forward: None,
        }
    }
}
impl State {
    /// Capture bounded loop selections for Undo.
    /// Takes this state; returns saved banks and the selected runtime slot.
    pub fn loop_history(&self) -> LoopHistory {
        LoopHistory {
            loops: self.loops,
            selected: self.selected,
            auto_button: self.auto_button,
        }
    }
    /// Restore loop selection without reviving held performance buttons.
    /// Takes the captured loop state; replaces banks and clears any stale edge-edit baseline.
    pub fn restore_loops(&mut self, history: LoopHistory) {
        self.loops = history.loops;
        self.selected = history.selected;
        self.auto_button = history.auto_button;
        self.edit = 0;
        self.edit_ticks = None;
    }
    /// Check a held button across independent input owners.
    /// Takes a button; returns whether any admitted source still holds it.
    pub fn held(&self, button: Button) -> bool {
        self.counts[button.index()] > 0
    }
    /// Turn a held Cue preview into continued playback.
    /// Takes this state; returns whether an NS7 preview owned the transport.
    pub fn latch_preview(&mut self) -> bool {
        self.preview.take().is_some()
    }
    /// Retain or release exactly one button owner.
    /// Takes source, button and pressed state; returns whether the ownership changed.
    fn hold(&mut self, source: u64, button: Button, on: bool) -> bool {
        let existing = self
            .owners
            .iter()
            .position(|owner| *owner == Some((source, button)));
        if on {
            if existing.is_some() {
                return false;
            }
            if let Some(empty) = self.owners.iter_mut().find(|owner| owner.is_none()) {
                *empty = Some((source, button));
                self.counts[button.index()] += 1;
                return true;
            }
        } else if let Some(index) = existing {
            self.owners[index] = None;
            self.counts[button.index()] -= 1;
            return true;
        }
        false
    }
    /// Read a signed playback multiplier without changing the pitch slider.
    /// Takes this state; returns direction and temporary pitch bend.
    pub fn multiplier(&self) -> f32 {
        let direction = if self.held(Button::Reverse) || self.held(Button::Bleep) {
            -1.0
        } else {
            1.0
        };
        direction
            * (1.0
                + 0.08
                    * (i32::from(self.held(Button::BendUp))
                        - i32::from(self.held(Button::BendDown))) as f32)
    }
    /// Arm a bounded transport acceleration or brake.
    /// Takes Play state, prior velocity and output rate; returns no value.
    pub fn transport(&mut self, playing: bool, velocity: f32, sr: f32) {
        self.braking = !playing && self.stop > 0.0;
        self.brake_rate = velocity;
        let seconds = if playing { self.start } else { self.stop };
        self.scale = if playing && seconds > 0.0 { 0.0 } else { 1.0 };
        self.remaining = (seconds * sr).round() as u32;
        self.step = if self.remaining == 0 {
            0.0
        } else {
            (if playing { 1.0 } else { 0.0 } - self.scale) / self.remaining as f32
        };
    }
    /// Advance transport speed by one audio frame.
    /// Takes this state; returns the current bounded multiplier.
    pub fn tick(&mut self) -> f32 {
        if self.remaining > 0 {
            self.scale = (self.scale + self.step).clamp(0.0, 1.0);
            self.remaining -= 1;
            if self.remaining == 0 {
                self.scale = if self.braking { 0.0 } else { 1.0 };
            }
        } else if self.braking {
            self.braking = false;
        }
        self.scale
    }
    /// Clear performance gestures during a safety stop.
    /// Takes this state; returns no value and retains knob preferences and loop slots.
    pub fn release(&mut self) {
        self.owners.fill(None);
        self.counts.fill(0);
        self.forward = None;
        self.performance_forward = None;
        self.saved_loop = None;
        self.slip_forward = None;
        self.preview = None;
        self.auto_button = None;
        self.delete = false;
        self.delete_used = false;
        self.braking = false;
        self.remaining = 0;
        self.scale = 1.0;
        self.edit = 0;
        self.edit_ticks = None;
    }
    /// Publish controller state without allocation.
    /// Takes this state; returns the UI and feedback values.
    pub fn status(&self) -> Status {
        Status {
            reverse: self.held(Button::Reverse),
            bleep: self.held(Button::Bleep),
            bend: (i32::from(self.held(Button::BendUp)) - i32::from(self.held(Button::BendDown)))
                as f32
                * 0.08,
            delete: self.delete,
            auto_loop: self.auto_loop,
            loop_slot: self.selected as u8,
            loop_edit: self.edit,
            start_seconds: self.start,
            stop_seconds: self.stop,
            braking: self.braking,
            slip: self.slip,
            pad_mode: self.pad_mode,
            roll: (0..8).find(|&pad| self.held(Button::Roll(pad))),
            slice: (0..8).find(|&pad| self.held(Button::Slice(pad))),
            roll_scale: self.roll_scale,
            slice_domain: self.slice_domain,
            hotloops: self.loops.map(|slot| slot.is_some()),
        }
    }
    /// Discard loop banks and preview clocks when source media changes.
    /// Takes this state; returns no value and retains physical knob/switch settings.
    pub fn media_changed(&mut self) {
        self.owners.fill(None);
        self.counts.fill(0);
        self.performance_forward = None;
        self.saved_loop = None;
        self.slip_forward = None;
        self.loops.fill(None);
        self.auto_button = None;
        self.selected = 0;
        self.edit = 0;
        self.edit_ticks = None;
        self.forward = None;
        self.preview = None;
        self.braking = false;
        self.remaining = 0;
        self.scale = 1.0;
    }
}

impl super::DeckRt {
    /// Retire temporary pad loops before clearing their owners.
    /// Takes this deck; restores its prior loop and clears all held performance gestures.
    pub(super) fn release_performance_controls(&mut self) {
        if let Some((on, start, len)) = self.controls.saved_loop.take() {
            self.loop_on = on;
            self.loop_start = start;
            self.loop_len = len;
        }
        self.controls.release();
    }
}

impl RtEngine {
    /// Advance silent slip and pad clocks independently of the audible loop.
    /// Takes deck index; advances at normal tempo and restores the slip position after vinyl release.
    pub(super) fn deck_surface_tick(&mut self, index: usize) {
        let d = &mut self.decks[index];
        let step = f64::from(if d.sync {
            d.target_rate.abs()
        } else {
            d.play_rate().abs()
        }) * d
            .audio
            .as_ref()
            .map_or(f64::from(self.sr), |audio| f64::from(audio.sr))
            / f64::from(self.sr);
        if let Some(position) = &mut d.controls.performance_forward {
            if d.playing {
                *position += step;
            }
            if let Some((true, start, len)) = d.controls.saved_loop.filter(|(_, _, len)| *len > 1.0)
            {
                if *position >= start + len {
                    *position = start + (*position - start).rem_euclid(len);
                }
            }
        }
        let scratching = d.touching
            || d.follows_spindle()
                && d.spindle
                    .as_ref()
                    .is_some_and(super::spindle::Playback::scratching);
        if d.controls.slip && scratching && d.playing {
            let position = d.controls.slip_forward.get_or_insert(d.pos);
            *position += step;
        } else if let Some(position) = d.controls.slip_forward.take() {
            if d.playing {
                d.transition_to(position, self.sr, DeckTransition::Jump);
            }
        }
    }
    /// Measure the actual audience output for the NS7's master meter mode.
    /// Takes the emitted stereo frame; updates two bounded envelopes without allocation.
    pub(super) fn observe_master_meter(&mut self, frame: [f32; 2]) {
        for (meter, sample) in self.master_meters.iter_mut().zip(frame) {
            *meter = if sample.is_finite() {
                *meter * 0.9 + sample.abs() * 0.1
            } else {
                0.0
            };
        }
    }
    /// Apply one source-owned controller gesture.
    /// Takes the source, deck and validated control; updates transport or delegates persistent edits to ordinary commands.
    pub(super) fn deck_control(&mut self, source: u64, deck: u8, control: Control) {
        if usize::from(deck) >= super::DECKS || !control.valid() {
            return;
        }
        let index = usize::from(deck);
        match control {
            Control::Hold { button, on } => {
                let d = &mut self.decks[index];
                if !d.controls.hold(source, button, on) {
                    return;
                }
                match button {
                    Button::Roll(_) | Button::Slice(_) => {
                        if on && d.controls.saved_loop.is_none() {
                            d.controls.saved_loop = Some((d.loop_on, d.loop_start, d.loop_len));
                            d.controls.performance_forward = Some(d.pos);
                        }
                        let active = if on {
                            Some(button)
                        } else {
                            d.controls
                                .owners
                                .iter()
                                .rev()
                                .flatten()
                                .map(|(_, button)| *button)
                                .find(|button| matches!(button, Button::Roll(_) | Button::Slice(_)))
                        };
                        if let Some(button) = active {
                            let (start, beats) = match button {
                                Button::Roll(pad) => (
                                    d.grid_snap(d.pos, self.sr, self.bpm),
                                    2_f32.powi(
                                        i32::from(pad) - 5 + i32::from(d.controls.roll_scale),
                                    ),
                                ),
                                Button::Slice(pad) => {
                                    let domain = 2_f64.powi(i32::from(d.controls.slice_domain));
                                    let beat = d.grid_beats_between(
                                        0.0,
                                        d.controls.performance_forward.unwrap_or(d.pos),
                                        self.sr,
                                        self.bpm,
                                    );
                                    let start = (beat / domain).floor() * domain
                                        + f64::from(pad) * domain / 8.0;
                                    (
                                        d.grid_span(0.0, start, self.sr, self.bpm),
                                        (domain
                                            / 8.0
                                            / 2_f64.powi(i32::from(d.controls.slice_quant)))
                                            as f32,
                                    )
                                }
                                _ => unreachable!(),
                            };
                            d.loop_start = start;
                            d.loop_len = d
                                .grid_span(start, f64::from(beats), self.sr, self.bpm)
                                .max(2.0);
                            d.loop_on = true;
                            d.transition_to(start, self.sr, DeckTransition::Jump);
                        } else {
                            if let Some((on, start, len)) = d.controls.saved_loop.take() {
                                d.loop_on = on;
                                d.loop_start = start;
                                d.loop_len = len;
                            }
                            if let Some(position) = d.controls.performance_forward.take() {
                                d.transition_to(position, self.sr, DeckTransition::Jump);
                            }
                        }
                    }
                    Button::Bleep => {
                        if d.controls.held(Button::Bleep) {
                            if d.controls.forward.is_none() {
                                d.controls.forward = Some(d.pos);
                            }
                        } else if let Some(position) = d.controls.forward.take() {
                            d.transition_to(position, self.sr, DeckTransition::Jump);
                        }
                    }
                    Button::Delete => {
                        if on && d.controls.counts[button.index()] == 1 {
                            d.controls.delete = !d.controls.delete;
                            d.controls.delete_used = false;
                        } else if d.controls.delete_used && !d.controls.held(Button::Delete) {
                            d.controls.delete = false;
                        }
                    }
                    Button::Cue | Button::HotCue(_) => {
                        if !on {
                            let held = d.controls.held(Button::Cue)
                                || (0..8).any(|pad| d.controls.held(Button::HotCue(pad)));
                            if !held && d.controls.preview.is_some() && d.preview_position.is_some()
                            {
                                d.controls.preview = None;
                                d.stop_preview(self.sr);
                                d.playing = false;
                            }
                            return;
                        }
                        if let Button::HotCue(pad) = button {
                            let deleting = d.controls.delete;
                            let set = d.hotcues[usize::from(pad)].set;
                            let paused = !d.playing;
                            self.apply(Command::DeckHotCue {
                                deck,
                                pad,
                                del: deleting,
                            });
                            let d = &mut self.decks[index];
                            if deleting {
                                d.controls.delete_used = true;
                                if !d.controls.held(Button::Delete) {
                                    d.controls.delete = false;
                                }
                            } else if paused && set {
                                d.playing = false;
                                d.preview_position = Some(d.pos);
                                d.controls.preview = Some((source, button));
                            }
                        } else if d.playing {
                            self.apply(Command::DeckCue { deck });
                        } else if d.preview_position.is_none() {
                            self.apply(Command::DeckCue { deck });
                            let d = &mut self.decks[index];
                            let position = d.cue_pos;
                            d.transition_to(position, self.sr, DeckTransition::Jump);
                            d.preview_position = Some(position);
                            d.controls.preview = Some((source, button));
                        }
                    }
                    _ => {}
                }
                if !matches!(button, Button::Cue | Button::HotCue(_) | Button::Delete) {
                    self.decks[index].fade_from_last_output(self.sr);
                }
            }
            Control::Keylock => self.apply(Command::DeckKeylock { deck }),
            Control::SyncOff => {
                if self.decks[index].sync {
                    self.apply(Command::DeckSync { deck });
                }
            }
            Control::PitchRange => self.apply(Command::DeckPitchRange { deck }),
            Control::Strip { value } => {
                if self.decks[index].loop_on {
                    self.apply(Command::DeckControl {
                        source,
                        deck,
                        control: Control::LoopToggle,
                    });
                }
                self.apply(Command::DeckSeek { deck, frac: value });
                let d = &mut self.decks[index];
                if d.controls.forward.is_some() {
                    d.controls.forward = Some(d.pos);
                }
            }
            Control::TrackStart => self.apply(Command::DeckSeek { deck, frac: 0.0 }),
            Control::StartTime { value } => self.decks[index].controls.start = value * 4.0,
            Control::Slip => self.decks[index].controls.slip = !self.decks[index].controls.slip,
            Control::PadMode { mode } => self.decks[index].controls.pad_mode = mode,
            Control::Parameter { mode, up, shifted } => {
                let controls = &mut self.decks[index].controls;
                let delta = if up { 1 } else { -1 };
                match mode {
                    1 | 5 if !shifted => {
                        controls.roll_scale = (controls.roll_scale + delta).clamp(-3, 3)
                    }
                    2 => {
                        if !shifted {
                            controls.slice_quant = (i16::from(controls.slice_quant)
                                + i16::from(delta))
                            .clamp(0, 3) as u8;
                        } else {
                            controls.slice_domain = (i16::from(controls.slice_domain)
                                + i16::from(delta))
                            .clamp(1, 6) as u8;
                        }
                    }
                    4 | 6 if !shifted => self.apply(Command::DeckControl {
                        source,
                        deck,
                        control: Control::LoopScale { double: up },
                    }),
                    3 | 7 => {
                        self.apply(Command::SamplerBank(if up {
                            self.sampler_bank + 1
                        } else {
                            self.sampler_bank.saturating_sub(1)
                        }));
                    }
                    _ => self.apply(Command::DeckControl {
                        source,
                        deck,
                        control: Control::LoopShift { forward: up },
                    }),
                }
            }
            Control::HotLoop { pad, clear } => {
                let d = &mut self.decks[index];
                let slot = usize::from(pad);
                if clear {
                    d.controls.loops[slot] = None;
                    return;
                }
                if let Some((start, len)) = d.controls.loops[slot] {
                    d.loop_on = true;
                    d.loop_start = start;
                    d.loop_len = len;
                    d.transition_to(start, self.sr, DeckTransition::Jump);
                } else {
                    let start = if d.hotcues[slot].set {
                        d.hotcues[slot].pos
                    } else {
                        d.grid_snap(d.pos, self.sr, self.bpm)
                    };
                    let len = if d.loop_on {
                        d.loop_len
                    } else {
                        d.grid_span(start, 4.0, self.sr, self.bpm)
                    };
                    d.controls.loops[slot] = Some((start, len));
                    d.loop_on = true;
                    d.loop_start = start;
                    d.loop_len = len;
                }
            }
            Control::AutoLoopPad { pad } => {
                let beats = 2_f32
                    .powi(i32::from(pad) - 5 + i32::from(self.decks[index].controls.roll_scale));
                let d = &mut self.decks[index];
                let length = d.grid_span(d.loop_start, f64::from(beats), self.sr, self.bpm);
                if (d.loop_len - length).abs() > 1.0 {
                    d.loop_on = false;
                }
                self.apply(Command::DeckLoop { deck, beats });
            }
            Control::ManualPad { pad } => match pad {
                0 => self.apply(Command::SelectDeck(index)),
                1 | 6 => self.deck_control(source, deck, Control::LoopToggle),
                2 => self.remember_controller_loop(index),
                3 | 7 => {
                    let d = &mut self.decks[index];
                    d.controls.selected = (d.controls.selected + if pad == 3 { 7 } else { 1 }) % 8;
                    d.controls.edit = 0;
                    d.controls.edit_ticks = None;
                    if let Some((start, len)) = d.controls.loops[d.controls.selected] {
                        d.loop_start = start;
                        d.loop_len = len;
                    } else {
                        d.clear_loop();
                    }
                    d.publish_preparation();
                }
                4 | 5 => {
                    if self.decks[index].loop_on {
                        let controls = &mut self.decks[index].controls;
                        let edit = if pad == 4 { 1 } else { 2 };
                        controls.edit = if controls.edit == edit { 0 } else { edit };
                        controls.edit_ticks = None;
                    } else {
                        self.apply(if pad == 4 {
                            Command::DeckLoopIn { deck }
                        } else {
                            Command::DeckLoopOut { deck }
                        });
                    }
                }
                _ => {}
            },
            Control::StopTime { value } => self.decks[index].controls.stop = value * 4.0,
            Control::Tap => {
                let d = &mut self.decks[index];
                let now = Instant::now();
                if d.controls
                    .taps
                    .iter()
                    .flatten()
                    .all(|last| now.saturating_duration_since(*last).as_secs_f64() > 2.0)
                {
                    d.controls.taps.fill(None);
                }
                d.controls.taps[d.controls.tap_cursor] = Some(now);
                d.controls.tap_cursor = (d.controls.tap_cursor + 1) % d.controls.taps.len();
                let mut first = now;
                let mut count = 0;
                for &tap in d.controls.taps.iter().flatten() {
                    first = first.min(tap);
                    count += 1;
                }
                let seconds = now.saturating_duration_since(first).as_secs_f64();
                if count >= 2
                    && seconds > 0.0
                    && !d
                        .load_receipt
                        .as_ref()
                        .is_some_and(super::load_receipt::Receipt::grid_is_locked)
                {
                    let bpm = (60.0 * (count - 1) as f64 / seconds).clamp(40.0, 300.0);
                    let origin = d.grid.map_or(0.0, |grid| grid.downbeat());
                    if let Ok(grid) = super::beatgrid::Grid::new(origin, bpm) {
                        d.grid = Some(grid);
                        d.publish_preparation();
                        self.project.edited();
                    }
                }
            }
            Control::LoopMode => {
                let c = &mut self.decks[index].controls;
                c.auto_loop = !c.auto_loop;
                c.auto_button = None;
                c.edit = 0;
                c.edit_ticks = None;
            }
            Control::LoopButton { index: button } => {
                if self.decks[index].controls.auto_loop {
                    let d = &mut self.decks[index];
                    if d.loop_on && d.controls.auto_button == Some(button) {
                        d.clear_loop();
                        d.controls.loops[d.controls.selected] = None;
                        d.controls.auto_button = None;
                    } else {
                        d.loop_on = true;
                        d.loop_start = if self.quantize && d.grid.is_some() {
                            d.grid_snap(d.pos, self.sr, self.bpm)
                        } else {
                            d.pos
                        };
                        d.loop_len =
                            d.grid_span(d.loop_start, f64::from(1 << button), self.sr, self.bpm);
                        d.controls.auto_button = Some(button);
                    }
                    d.transition_to(d.pos, self.sr, DeckTransition::Jump);
                    d.publish_preparation();
                    self.project.edited();
                } else {
                    self.deck_control(
                        source,
                        deck,
                        match button {
                            0 => {
                                if self.decks[index].loop_on {
                                    let c = &mut self.decks[index].controls;
                                    c.edit = if c.edit == 1 { 0 } else { 1 };
                                    c.edit_ticks = None;
                                    return;
                                }
                                self.apply(Command::DeckLoopIn { deck });
                                return;
                            }
                            1 => {
                                if self.decks[index].loop_on {
                                    let c = &mut self.decks[index].controls;
                                    c.edit = if c.edit == 2 { 0 } else { 2 };
                                    c.edit_ticks = None;
                                    return;
                                }
                                self.apply(Command::DeckLoopOut { deck });
                                self.remember_controller_loop(index);
                                return;
                            }
                            2 => Control::LoopSelect,
                            _ => Control::Reloop,
                        },
                    );
                }
                self.remember_controller_loop(index);
            }
            Control::LoopToggle => {
                if self.decks[index].loop_len <= 1.0 {
                    self.apply(Command::DeckLoop { deck, beats: 4.0 });
                    return;
                }
                let d = &mut self.decks[index];
                if d.loop_len > 1.0 {
                    d.loop_on = !d.loop_on;
                    d.transition_to(d.pos, self.sr, DeckTransition::Jump);
                    d.publish_preparation();
                    self.project.edited();
                }
            }
            Control::LoopSelect => {
                self.remember_controller_loop(index);
                let d = &mut self.decks[index];
                d.controls.edit = 0;
                d.controls.edit_ticks = None;
                d.controls.auto_button = None;
                d.controls.selected = (d.controls.selected + 1) % d.controls.loops.len();
                if let Some((start, length)) = d.controls.loops[d.controls.selected] {
                    d.loop_start = start;
                    d.loop_len = length;
                } else {
                    d.clear_loop();
                }
                d.transition_to(d.pos, self.sr, DeckTransition::Jump);
                d.publish_preparation();
                self.project.edited();
            }
            Control::Reloop => {
                let d = &mut self.decks[index];
                if d.loop_len > 1.0 {
                    d.loop_on = true;
                    d.transition_to(d.loop_start, self.sr, DeckTransition::Jump);
                    d.publish_preparation();
                    self.project.edited();
                }
            }
            Control::LoopScale { double } => {
                if self.decks[index].loop_len <= 1.0 {
                    self.apply(Command::DeckLoop { deck, beats: 4.0 });
                    self.decks[index].loop_on = false;
                }
                let active = self.decks[index].loop_on;
                self.decks[index].loop_on = self.decks[index].loop_len > 1.0;
                self.apply_plain(if double {
                    Command::DeckLoopDouble { deck }
                } else {
                    Command::DeckLoopHalf { deck }
                });
                self.decks[index].loop_on = active;
                self.decks[index].publish_preparation();
                self.remember_controller_loop(index);
            }
            Control::LoopShift { forward } => {
                let d = &mut self.decks[index];
                if d.loop_len > 1.0 {
                    let frames = d.audio.as_ref().map_or(0.0, |a| a.frames() as f64);
                    let next = d.loop_start + d.loop_len * if forward { 1.0 } else { -1.0 };
                    if next >= 0.0 && next + d.loop_len <= frames {
                        let shift = next - d.loop_start;
                        d.loop_start = next;
                        d.transition_to(
                            if d.loop_on { d.pos + shift } else { d.pos },
                            self.sr,
                            DeckTransition::Jump,
                        );
                        d.publish_preparation();
                        self.project.edited();
                    }
                }
                self.remember_controller_loop(index);
            }
        }
    }
    /// Keep the selected manual loop available for later relooping.
    /// Takes a deck index; copies its valid loop into bounded controller storage.
    fn remember_controller_loop(&mut self, deck: usize) {
        let d = &mut self.decks[deck];
        if let Some(audio) = &d.audio {
            d.loop_len = d
                .loop_len
                .min((audio.frames() as f64 - d.loop_start).max(0.0));
            if d.loop_len <= 1.0 {
                d.loop_on = false;
            }
        }
        if d.loop_len > 1.0 {
            d.controls.loops[d.controls.selected] = Some((d.loop_start, d.loop_len));
        }
        d.publish_preparation();
    }
    /// Fine tune a selected loop edge using physical encoder movement.
    /// Takes deck and accumulated ticks; returns whether loop editing owns this movement.
    pub(super) fn controller_loop_edit(&mut self, deck: usize, ticks: i64) -> bool {
        let d = &mut self.decks[deck];
        if d.controls.edit == 0 || d.vinyl {
            return false;
        }
        let Some(previous) = d.controls.edit_ticks.replace(ticks) else {
            return true;
        };
        let delta = (ticks - previous) as f64 / super::spindle::TICKS_PER_SECOND
            * d.audio.as_ref().map_or(self.sr, |a| a.sr as f32) as f64;
        let end = d.loop_start + d.loop_len;
        if d.controls.edit == 1 {
            d.loop_start = (d.loop_start + delta).clamp(0.0, (end - 64.0).max(0.0));
            d.loop_len = end - d.loop_start;
        } else {
            let frames = d.audio.as_ref().map_or(end, |a| a.frames() as f64);
            d.loop_len = (d.loop_len + delta).clamp(64.0, (frames - d.loop_start).max(64.0));
        }
        d.publish_preparation();
        self.project.edited();
        self.undo.untracked_change();
        self.remember_controller_loop(deck);
        true
    }
    /// Read the crossfader after its hardware reverse switch.
    /// Takes this renderer; returns the effective A-to-B position.
    pub(super) fn crossfader_position(&self) -> f32 {
        if self.xfader_reverse {
            1.0 - self.xfader
        } else {
            self.xfader
        }
    }
    /// Apply a crossfader move and any enabled fader-start edges.
    /// Takes the physical position; starts on leaving mute and stops/rewinds on reaching mute.
    pub(super) fn crossfader_move(&mut self, value: f32) {
        if !value.is_finite() {
            return;
        }
        let before = self.crossfader_position();
        self.xfader = value.clamp(0.0, 1.0);
        let after = self.crossfader_position();
        for deck in 0..super::DECKS {
            if !self.fader_start[deck] {
                continue;
            }
            let muted = |x| if deck == 0 { x == 1.0 } else { x == 0.0 };
            let d = &mut self.decks[deck];
            if muted(before) && !muted(after) && d.audio.is_some() {
                d.playing = true;
                d.controls.transport(true, d.rate, self.sr);
            } else if !muted(before) && muted(after) {
                d.playing = false;
                d.release_performance_controls();
                d.transition_to(0.0, self.sr, DeckTransition::Jump);
            }
        }
    }
}
