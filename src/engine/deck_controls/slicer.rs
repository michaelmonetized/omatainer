use super::{
    super::{DeckRt, DeckTransition},
    Button, QUANTIZE_DIVISIONS,
};
use serde::Serialize;

type Owner = (u64, Option<u32>, Button);

#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct Status {
    pub repeating: bool,
    pub division: Option<u8>,
    pub bounds: Option<[f64; 9]>,
    pub active: Option<u8>,
    pub pending: Option<u8>,
    pub pending_beat: Option<f64>,
}

#[derive(Clone, Copy, Debug)]
struct Pending {
    owner: Owner,
    remaining: f64,
    beat: f64,
}

#[derive(Clone, Debug, Default)]
pub(super) struct State {
    repeating: bool,
    division: Option<u8>,
    start: Option<f64>,
    bounds: Option<[f64; 9]>,
    original_loop: Option<(bool, f64, f64)>,
    active: Option<Owner>,
    pending: Option<Pending>,
    interrupted: bool,
}
impl State {
    /// Publish the confirmed slice domain and trigger.
    /// Takes this runtime; returns fixed source bounds and independent repeat/onset settings without allocation.
    pub(super) fn status(&self) -> Status {
        let pad = |owner: Owner| {
            if let Button::Slice(pad) = owner.2 {
                Some(pad)
            } else {
                None
            }
        };
        Status {
            repeating: self.repeating,
            division: self.division,
            bounds: self.bounds,
            active: self.active.and_then(pad),
            pending: self.pending.and_then(|p| pad(p.owner)),
            pending_beat: self.pending.filter(|p|matches!(p.owner.2,Button::Slice(_))).map(|p|p.beat),
        }
    }
    /// Read the applied roll from its retained input owner.
    /// Takes this temporary-loop runtime; returns a zero-based Roll pad or no active roll.
    pub(super) fn active_roll(&self) -> Option<u8> {
        self.active.and_then(|owner| if let Button::Roll(pad)=owner.2 {Some(pad)}else{None})
    }
    /// Read a source-owned roll waiting for its musical onset.
    /// Takes this temporary-loop runtime; returns pad and due beat without allocating.
    pub(super) fn pending_roll(&self) -> Option<(u8,f64)> {
        self.pending.and_then(|pending| if let Button::Roll(pad)=pending.owner.2 {Some((pad,pending.beat))}else{None})
    }
    /// Configure only validated transient slicer settings.
    /// Takes repeating-domain behavior and onset division; changes no source metadata or stored project state.
    pub(super) fn configure(&mut self, repeating: bool, division: Option<u8>) {
        self.invalidate_bounds();
        self.repeating = repeating;
        self.division = division;
        self.interrupted = false;
    }
    /// Discard cached source intervals after a phrase setting changes.
    /// Takes this runtime; retains unrelated active pad ownership while forcing fresh mapped bounds.
    pub(super) fn invalidate_bounds(&mut self) {
        self.start = None;
        self.bounds = None;
    }
    /// Retire source-owned slicer work while keeping settings.
    /// Takes this runtime; clears bounds, pending owners and fixed-loop history until an explicit restart.
    pub(super) fn clear(&mut self) {
        self.start = None;
        self.bounds = None;
        self.original_loop = None;
        self.active = None;
        self.pending = None;
        self.interrupted = true;
    }
    /// Arm the selected mode after an explicit transport start.
    /// Takes this runtime; allows the next rendered frame to capture its current source domain.
    pub(in crate::engine) fn arm(&mut self) {
        self.interrupted = false;
    }
}

impl DeckRt {
    /// Arm phrase slicing after deliberate transport or grid changes.
    /// Takes this deck; allows its next rendered frame to capture the current source domain.
    pub(in crate::engine) fn arm_slicer(&mut self) {
        self.controls.slicer.arm();
    }

    /// Restore temporary and repeating loops at deliberate mode or transport boundaries.
    /// Takes this deck; retires pending slices without seeking away from the requested playhead.
    pub(in crate::engine) fn leave_slicer(&mut self) {
        let owned = self.controls.slicer.original_loop.is_some()
            || self.controls.slicer.pending.is_some()
            || self
                .controls
                .slicer
                .active
                .is_some_and(|owner| matches!(owner.2, Button::Slice(_)));
        if owned {
            if let Some((on, start, len)) = self.controls.saved_loop.take() {
                self.loop_on = on;
                self.loop_start = start;
                self.loop_len = len;
            }
            if let Some((on, start, len)) = self.controls.slicer.original_loop.take() {
                self.loop_on = on;
                self.loop_start = start;
                self.loop_len = len;
            }
            self.controls.performance_forward = None;
            for owner in &mut self.controls.owners {
                if owner.is_some_and(|owner| matches!(owner.2, Button::Slice(_))) {
                    *owner = None;
                }
            }
            self.controls.counts[22..].fill(0);
        }
        self.controls.slicer.clear();
    }
    /// Locate an eight-slice musical domain in original source coordinates.
    /// Takes the current background clock; updates exact mapped and source-clipped bounds, retaining a repeating domain.
    fn refresh_slicer(&mut self, sr: f32, bpm: f32) -> bool {
        if self.audio.is_none() {
            self.controls.slicer.bounds = None;
            return false;
        }
        let domain = 2_f64.powi(i32::from(self.controls.slice_domain));
        let position = self.controls.performance_forward.unwrap_or(self.pos);
        let beat = self.grid_beat_at(position, sr, bpm);
        let start = if self.controls.slicer.repeating {
            self.controls
                .slicer
                .start
                .unwrap_or_else(|| (beat / domain).floor() * domain)
        } else {
            (beat / domain).floor() * domain
        };
        let changed = self.controls.slicer.start != Some(start);
        self.controls.slicer.start = Some(start);
        let extent = self.audio.as_ref().unwrap().frames() as f64;
        let bounds = if changed || self.controls.slicer.bounds.is_none() {
            std::array::from_fn(|i| {
                self.grid_position_at(start + i as f64 * domain / 8.0, sr, bpm)
                    .clamp(0.0, extent)
            })
        } else {
            self.controls.slicer.bounds.unwrap()
        };
        self.controls.slicer.bounds = Some(bounds);
        if self.controls.slicer.repeating
            && self.controls.pad_mode == 2
            && self.playing
            && !self.controls.slicer.interrupted
            && self.controls.slicer.original_loop.is_none()
            && bounds[8] - bounds[0] >= 2.0
        {
            self.controls.slicer.original_loop = Some(self.controls.saved_loop.unwrap_or((
                self.loop_on,
                self.loop_start,
                self.loop_len,
            )));
            self.loop_on = true;
            self.loop_start = bounds[0];
            self.loop_len = bounds[8] - bounds[0];
            if self.controls.saved_loop.is_some() {
                self.controls.saved_loop = Some((true, self.loop_start, self.loop_len));
            }
        }
        changed
    }
    /// Apply one admitted slice or roll through its original held owner.
    /// Takes the held owner and clock; starts only source-bounded intervals and uses the existing jump envelope.
    fn activate_temporary_pad(&mut self, owner: Owner, sr: f32, bpm: f32) {
        let (start, end) = match owner.2 {
            Button::Roll(pad) => {
                let start = if self.controls.quantize==Some(true)&&self.playing {self.controls.performance_forward.unwrap_or(self.pos)}else{self.pos};
                let beats = 2_f64.powi(i32::from(pad) - 5 + i32::from(self.controls.roll_scale));
                (start, start + self.grid_span(start, beats, sr, bpm))
            }
            Button::Slice(pad) => {
                self.refresh_slicer(sr, bpm);
                let Some(bounds) = self.controls.slicer.bounds else {
                    return;
                };
                let start = bounds[usize::from(pad)];
                let beat = self.controls.slicer.start.unwrap()
                    + f64::from(pad) * 2_f64.powi(i32::from(self.controls.slice_domain)) / 8.0;
                let beats = 2_f64.powi(i32::from(self.controls.slice_domain))
                    / 8.0
                    / 2_f64.powi(i32::from(self.controls.slice_quant));
                (
                    start,
                    self.grid_position_at(beat + beats, sr, bpm)
                        .min(bounds[usize::from(pad) + 1]),
                )
            }
            _ => return,
        };
        let extent = self.audio.as_ref().map_or(0.0, |a| a.frames() as f64);
        let start = start.clamp(0.0, extent);
        let end = end.clamp(0.0, extent);
        if end - start < 2.0 {
            self.controls.slicer.active = None;
            return;
        }
        self.loop_start = start;
        self.loop_len = end - start;
        self.loop_on = true;
        self.controls.slicer.active = Some(owner);
        self.transition_to(start, sr, DeckTransition::Jump);
    }
    /// Schedule a slice or roll against the advancing background rather than the audible repeat.
    /// Takes an admitted owner and clock; queues a bounded musical delay or applies an immediate/stopped trigger.
    fn request_temporary_pad(&mut self, owner: Owner, sr: f32, bpm: f32) {
        self.controls.slicer.pending = None;
        let division=match owner.2 {
            Button::Slice(_)=>self.controls.slicer.division,
            Button::Roll(_) if self.controls.quantize==Some(true)=>Some(self.controls.quantize_division),
            _=>None,
        };
        if self.playing {
            if let Some(index) = division {
                let beat = self.grid_beat_at(
                    self.controls.performance_forward.unwrap_or(self.pos),
                    sr,
                    bpm,
                );
                let division = QUANTIZE_DIVISIONS[usize::from(index)];
                let next = ((beat - 1e-9) / division).ceil() * division;
                let remaining = (next - beat).max(0.0);
                if remaining > 1e-9 {
                    self.controls.slicer.pending = Some(Pending {
                        owner,
                        remaining,
                        beat: next,
                    });
                    return;
                }
            }
        }
        self.activate_temporary_pad(owner, sr, bpm);
    }
    /// Resolve temporary pads while preserving independent release ownership.
    /// Takes the original owner, gate and clock; retains one background root, cancels only its queued onset and restores after the final owner.
    pub(super) fn temporary_pad(&mut self, owner: Owner, on: bool, sr: f32, bpm: f32) {
        if on && self.controls.saved_loop.is_none() {
            self.controls.saved_loop = Some((self.loop_on, self.loop_start, self.loop_len));
            self.controls.performance_forward = Some(self.pos);
        }
        if on {
            self.controls.slicer.interrupted = false;
            self.request_temporary_pad(owner, sr, bpm);
            return;
        }
        if self
            .controls
            .slicer
            .pending
            .is_some_and(|p| p.owner == owner)
        {
            self.controls.slicer.pending = None;
        }
        let active = self
            .controls
            .owners
            .iter()
            .flatten()
            .copied()
            .find(|owner| matches!(owner.2, Button::Roll(_) | Button::Slice(_)));
        if let Some(active) = active {
            if self.controls.slicer.active != Some(active)
                && self
                    .controls
                    .slicer
                    .pending
                    .is_none_or(|p| p.owner != active)
            {
                self.request_temporary_pad(active, sr, bpm);
            }
        } else {
            self.controls.slicer.active = None;
            self.controls.slicer.pending = None;
            if let Some((on, start, len)) = self.controls.saved_loop.take() {
                self.loop_on = on;
                self.loop_start = start;
                self.loop_len = len;
            }
            if let Some(position) = self.controls.performance_forward.take() {
                if self.controls.slip_forward.is_none() {
                    self.transition_to(position, sr, DeckTransition::Jump);
                }
            }
        }
    }
    /// Execute due triggers and move domains before the next source sample.
    /// Takes the background frame's musical advance and clock; stays bounded and performs no heap work.
    pub(super) fn tick_slicer(&mut self, beat_step: f64, sr: f32, bpm: f32) {
        if !self.playing {
            if self.controls.slicer.original_loop.is_some() {
                self.leave_slicer();
            }
            self.controls.slicer.pending = None;
            return;
        }
        if self.controls.pad_mode == 2 && !self.controls.slicer.interrupted {
            let changed = self.refresh_slicer(sr, bpm);
            if changed && !self.controls.slicer.repeating {
                if let Some(owner) = self
                    .controls
                    .slicer
                    .active
                    .filter(|o| matches!(o.2, Button::Slice(_)))
                {
                    self.activate_temporary_pad(owner, sr, bpm);
                }
            }
        }
        if let Some(mut pending) = self.controls.slicer.pending {
            if !self.controls.owners.contains(&Some(pending.owner)) {
                self.controls.slicer.pending = None;
                return;
            }
            if pending.remaining <= 1e-9 {
                self.controls.slicer.pending = None;
                self.activate_temporary_pad(pending.owner, sr, bpm);
            } else {
                pending.remaining = (pending.remaining - beat_step).max(0.0);
                self.controls.slicer.pending = Some(pending);
            }
        }
    }
}

#[cfg(test)]
mod tests;
