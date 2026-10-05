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
            } => pad < 5,
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
}

#[derive(Clone, Debug)]
pub(super) struct State {
    owners: [Option<(u64, Button)>; super::control::MAX_COMMANDS],
    counts: [u16; 11],
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
            counts: [0; 11],
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
        }
    }
    /// Discard loop banks and preview clocks when source media changes.
    /// Takes this state; returns no value and retains physical knob/switch settings.
    pub fn media_changed(&mut self) {
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

impl RtEngine {
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
                                || (0..5).any(|pad| d.controls.held(Button::HotCue(pad)));
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
                    return;
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
                d.controls.release();
                d.transition_to(0.0, self.sr, DeckTransition::Jump);
            }
        }
    }
}
