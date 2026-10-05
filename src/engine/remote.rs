//! Fixed musical-time actions; producers own discovery, sockets and job lookup.
use super::{
    midi_edit::{Ack, Outcome},
    session::{Axis, Layout, Reference},
    Command, RtEngine,
};
use parking_lot::Mutex;
use std::collections::VecDeque;

pub(crate) const SCHEDULE_CAPACITY: usize = 64;
const JOB_CAPACITY: usize = 128;

#[derive(Clone, Copy, Debug)]
pub(crate) enum Action {
    Play,
    Stop,
    Scene {
        slot: usize,
        target: Reference,
    },
    Gain {
        slot: usize,
        target: Reference,
        value: f32,
    },
    Pan {
        slot: usize,
        target: Reference,
        value: f32,
    },
    Crossfader(f32),
    CrossfaderContour(f32),
    Master(f32),
}
impl Action {
    /// Resolve a prepared control.
    /// Takes a native action; returns its existing engine command without heap work.
    pub(crate) fn command(self) -> Command {
        match self {
            Self::Play => Command::Play,
            Self::Stop => Command::Stop,
            Self::Scene { slot, .. } => Command::LaunchScene { scene: slot as u16 },
            Self::Gain { slot, value, .. } => Command::TrackGain {
                track: slot as u8,
                value,
            },
            Self::Pan { slot, value, .. } => Command::TrackPan {
                track: slot as u8,
                value,
            },
            Self::Crossfader(value) => Command::Xfader(value),
            Self::CrossfaderContour(value) => Command::XfaderCurve(value),
            Self::Master(value) => Command::Master(value),
        }
    }

    /// Check the original target and value.
    /// Takes the current layout; returns false for deleted/replaced objects or invalid values.
    fn current(self, layout: &Layout) -> bool {
        match self {
            Self::Scene { slot, target } => layout.resolves(Axis::Scene, slot, target),
            Self::Gain {
                slot,
                target,
                value,
            } => {
                layout.resolves(Axis::Track, slot, target)
                    && value.is_finite()
                    && (0.0..=1.5).contains(&value)
            }
            Self::Pan {
                slot,
                target,
                value,
            } => {
                layout.resolves(Axis::Track, slot, target)
                    && value.is_finite()
                    && (0.0..=1.0).contains(&value)
            }
            Self::Crossfader(value) => value.is_finite() && (0.0..=1.0).contains(&value),
            Self::CrossfaderContour(value) => value.is_finite() && (0.0..=1.0).contains(&value),
            Self::Master(value) => value.is_finite() && (0.0..=1.5).contains(&value),
            Self::Play | Self::Stop => true,
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Request {
    pub namespace: [u64; 2],
    pub transport_epoch: u64,
    pub safety_epoch: u64,
    pub at: Option<f64>,
    pub action: Action,
    pub ack: Ack,
}
impl Request {
    /// Keep a control attached to its original session and transport.
    /// Takes the renderer; returns whether this pending request can still apply.
    fn current(&self, rt: &RtEngine) -> bool {
        self.ack.state() == Outcome::Pending
            && self.namespace == rt.session.namespace
            && self.transport_epoch == rt.transport_epoch
            && self.safety_epoch == rt.performance.safety_epoch()
            && self.action.current(&rt.session)
    }
}

#[derive(Default)]
pub(crate) struct Jobs(Mutex<VecDeque<(String, Ack)>>);
impl Jobs {
    /// Retain a reconnectable result.
    /// Takes a completion receipt; returns an opaque job ID or a bounded-capacity error.
    pub(crate) fn insert(&self, ack: Ack) -> Result<String, &'static str> {
        let mut jobs = self.0.lock();
        if jobs.len() == JOB_CAPACITY {
            let index = jobs
                .iter()
                .position(|(_, ack)| ack.state() != Outcome::Pending)
                .ok_or("All 128 retained automation jobs are pending")?;
            jobs.remove(index);
        }
        let identity = super::midi_edit::NoteId::new();
        if !identity.valid() {
            return Err("Automation job identity is unavailable");
        }
        let id = identity.to_string();
        jobs.push_back((id.clone(), ack));
        Ok(id)
    }

    /// Find a retained completion receipt.
    /// Takes the opaque ID; returns a receipt for status or cancellation, if retained.
    pub(crate) fn get(&self, id: &str) -> Option<Ack> {
        self.0
            .lock()
            .iter()
            .find(|(key, _)| key == id)
            .map(|(_, ack)| ack.clone())
    }
}

pub(super) struct Schedule {
    items: [Option<Request>; SCHEDULE_CAPACITY],
    len: usize,
}
impl Default for Schedule {
    fn default() -> Self {
        Self {
            items: std::array::from_fn(|_| None),
            len: 0,
        }
    }
}
impl Schedule {
    /// Insert a prepared action in beat order.
    /// Takes an owned request; returns it unchanged when all fixed slots are occupied.
    fn insert(&mut self, request: Request) -> Result<(), Request> {
        if self.len == SCHEDULE_CAPACITY {
            return Err(request);
        }
        let beat = request.at.unwrap();
        let mut index = self.len;
        while index > 0 && self.items[index - 1].as_ref().unwrap().at.unwrap() > beat {
            self.items[index] = self.items[index - 1].take();
            index -= 1;
        }
        self.items[index] = Some(request);
        self.len += 1;
        Ok(())
    }

    /// Remove one owned action without heap work.
    /// Takes an occupied index; returns that request and closes the fixed-array gap.
    fn remove(&mut self, index: usize) -> Request {
        let result = self.items[index].take().unwrap();
        for next in index + 1..self.len {
            self.items[next - 1] = self.items[next].take();
        }
        self.len -= 1;
        result
    }
}

impl RtEngine {
    /// Admit a prepared remote action.
    /// Takes the request; applies an immediate action or retains a future action in fixed storage.
    pub(super) fn remote_request(&mut self, request: Request) {
        let valid = request.current(self)
            && request.at.is_none_or(|beat| {
                beat.is_finite()
                    && self.playing
                    && beat > self.precise_midi_beat()
                    && beat <= self.precise_midi_beat() + 16384.0
            });
        if !valid {
            request.ack.reject();
            self.undo.retire_command(Command::Remote(request));
            return;
        }
        if request.at.is_some() {
            if let Err(request) = self.remote_schedule.insert(request) {
                request.ack.reject();
                self.undo.retire_command(Command::Remote(request));
            }
        } else {
            self.remote_apply(request);
        }
    }

    /// Cancel obsolete scheduled actions at a block boundary.
    /// Takes the renderer; retires receipts for cancelled, replaced or stopped sessions off audio.
    pub(super) fn remote_maintain(&mut self) {
        for index in (0..self.remote_schedule.len).rev() {
            if !self.remote_schedule.items[index]
                .as_ref()
                .unwrap()
                .current(self)
            {
                let request = self.remote_schedule.remove(index);
                request.ack.reject();
                self.undo.retire_command(Command::Remote(request));
            }
        }
    }

    /// Dispatch actions at their musical sample boundary.
    /// Takes the renderer; does constant work with an empty schedule and bounded work when actions are due.
    pub(super) fn remote_tick(&mut self) {
        while self.playing
            && self.remote_schedule.items[0]
                .as_ref()
                .is_some_and(|request| {
                    request.at.unwrap()
                        <= self.precise_midi_beat() + super::midi_schedule::BEAT_EPSILON
                })
        {
            let request = self.remote_schedule.remove(0);
            self.remote_apply(request);
        }
    }

    /// Apply one current action with an honest completion receipt.
    /// Takes the request; checks protection and history before mutation, then retires ownership off audio.
    fn remote_apply(&mut self, request: Request) {
        let command = request.action.command();
        if !request.current(self)
            || self
                .performance
                .check(&command, Some(self.deck_activity()))
                .is_err()
            || !request.ack.claim()
        {
            request.ack.reject();
        } else if let Some(command) = self.history_before(command) {
            self.apply_plain(command);
            match request.action {
                Action::Gain { slot, .. } | Action::Pan { slot, .. } => {
                    let track = &mut self.tracks[slot];
                    track.mixer_gain.prepare(
                        [track.gain, track.pan],
                        self.sr,
                        super::mixer_gain::pan_gains,
                    );
                }
                Action::Crossfader(_) | Action::CrossfaderContour(_) => self.xfader_gain.prepare(
                    [self.xfader, self.xfader_curve],
                    self.sr,
                    super::mixer_gain::crossfader_gains,
                ),
                _ => {}
            }
            request.ack.applied();
        } else {
            request.ack.reject();
        }
        self.undo.retire_command(Command::Remote(request));
    }
}

#[cfg(test)]
mod tests;
