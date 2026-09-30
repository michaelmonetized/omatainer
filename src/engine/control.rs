//! Bounded dequeue work. Musical events stay FIFO; only adjacent assignments to
//! the same absolute parameter may be replaced by their latest value.
use super::Command;
use crossbeam_channel::Receiver;
use serde::Serialize;

#[cfg(test)]
mod admission_tests;

pub const COMMANDS_PER_BLOCK: usize = 32;

/// A producer handle contains no renderer or renderer lock. Its short mutex
/// serializes admission bookkeeping only; the audio consumer never acquires it.
#[derive(Clone)]
pub struct CommandPort {
    sender: crossbeam_channel::Sender<Command>,
    shared: std::sync::Arc<AdmissionShared>,
    admission: std::sync::Arc<parking_lot::Mutex<Admission>>,
    capacity: usize,
}

const STOP_LANES: usize = super::TRACKS + 1;
const MAX_COMMANDS: usize = 256;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GateKey {
    Live { source: u64, ch: u8, note: u8 },
    Pad(u8),
    Touch(u8),
}

struct Admission {
    gates: [Option<GateKey>; MAX_COMMANDS],
    held: usize,
    pending_stops: [u64; STOP_LANES],
    next_ticket: u64,
}

struct AdmissionShared {
    completed_stops: [std::sync::atomic::AtomicU64; STOP_LANES],
    connected: std::sync::atomic::AtomicBool,
    accepted: std::sync::atomic::AtomicU64,
    coalesced: std::sync::atomic::AtomicU64,
    rejected: std::sync::atomic::AtomicU64,
    last_error: std::sync::atomic::AtomicU8,
}

#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct SubmissionStats {
    pub accepted: u64,
    pub coalesced: u64,
    pub rejected: u64,
    pub last_error: Option<SubmissionError>,
}

impl AdmissionShared {
    fn stats(&self) -> SubmissionStats {
        use std::sync::atomic::Ordering::Relaxed;
        SubmissionStats {
            accepted: self.accepted.load(Relaxed),
            coalesced: self.coalesced.load(Relaxed),
            rejected: self.rejected.load(Relaxed),
            last_error: match self.last_error.load(Relaxed) {
                1 => Some(SubmissionError::Full),
                2 => Some(SubmissionError::StopPending),
                3 => Some(SubmissionError::Disconnected),
                _ => None,
            },
        }
    }
}

/// Receiver ownership stays with the callback. Completion is a fixed atomic
/// store, never a producer-mutex operation or a blocking acknowledgment send.
pub struct CommandReceiver {
    receiver: crossbeam_channel::Receiver<Command>,
    shared: Option<std::sync::Arc<AdmissionShared>>,
}

impl std::ops::Deref for CommandReceiver {
    type Target = crossbeam_channel::Receiver<Command>;
    fn deref(&self) -> &Self::Target {
        &self.receiver
    }
}

impl From<crossbeam_channel::Receiver<Command>> for CommandReceiver {
    fn from(receiver: crossbeam_channel::Receiver<Command>) -> Self {
        Self {
            receiver,
            shared: None,
        }
    }
}

impl Drop for CommandReceiver {
    fn drop(&mut self) {
        if let Some(shared) = &self.shared {
            shared
                .connected
                .store(false, std::sync::atomic::Ordering::Release);
        }
    }
}

impl CommandReceiver {
    pub(super) fn complete_stop(&self, lane: usize, ticket: u64) {
        if let Some(shared) = &self.shared {
            if let Some(completed) = shared.completed_stops.get(lane) {
                completed.store(ticket, std::sync::atomic::Ordering::Release);
            }
        }
    }

    pub(super) fn submissions(&self) -> SubmissionStats {
        self.shared
            .as_ref()
            .map(|shared| shared.stats())
            .unwrap_or_default()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SubmissionOutcome {
    Accepted,
    Coalesced,
}

impl SubmissionOutcome {
    pub fn name(self) -> &'static str {
        match self {
            Self::Accepted => "accepted",
            Self::Coalesced => "coalesced",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SubmissionError {
    Full,
    StopPending,
    Disconnected,
}

impl std::fmt::Display for SubmissionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Full => "The control queue is full. Releases remain reserved. Wait for playback to catch up, then retry the action.",
            Self::StopPending => "A stop is still pending for this target. Retry the start after the stop has completed.",
            Self::Disconnected => "Audio has disconnected. Restart Omatainer before retrying.",
        })
    }
}

impl std::error::Error for SubmissionError {}

impl CommandPort {
    pub fn channel(capacity: usize) -> (Self, CommandReceiver) {
        assert!(
            capacity > STOP_LANES + 1,
            "queue must fit dedicated stops and a gate pair"
        );
        assert!(
            capacity <= MAX_COMMANDS,
            "queue exceeds fixed reservation storage"
        );
        let (sender, receiver) = crossbeam_channel::bounded(capacity);
        let shared = std::sync::Arc::new(AdmissionShared {
            completed_stops: std::array::from_fn(|_| std::sync::atomic::AtomicU64::new(0)),
            connected: std::sync::atomic::AtomicBool::new(true),
            accepted: std::sync::atomic::AtomicU64::new(0),
            coalesced: std::sync::atomic::AtomicU64::new(0),
            rejected: std::sync::atomic::AtomicU64::new(0),
            last_error: std::sync::atomic::AtomicU8::new(0),
        });
        let port = Self {
            sender,
            shared: shared.clone(),
            admission: std::sync::Arc::new(parking_lot::Mutex::new(Admission {
                gates: [None; MAX_COMMANDS],
                held: 0,
                pending_stops: [0; STOP_LANES],
                next_ticket: 1,
            })),
            capacity,
        };
        (
            port,
            CommandReceiver {
                receiver,
                shared: Some(shared),
            },
        )
    }

    pub fn len(&self) -> usize {
        self.sender.len()
    }
    pub fn stats(&self) -> SubmissionStats {
        self.shared.stats()
    }

    /// Accepted means queued, not executed. Coalesced means an equivalent
    /// release is already ordered and no intervening accepted onset exists.
    /// No producer waits for queue capacity; only the small admission section
    /// serializes producers. Construct media/instruments before calling here.
    pub fn send(&self, mut command: Command) -> Result<SubmissionOutcome, SubmissionError> {
        use std::sync::atomic::Ordering::{Acquire, Relaxed};
        let mut state = self.admission.lock();
        let fail = |error| {
            self.shared.last_error.store(
                match error {
                    SubmissionError::Full => 1,
                    SubmissionError::StopPending => 2,
                    SubmissionError::Disconnected => 3,
                },
                Relaxed,
            );
            self.shared.rejected.fetch_add(1, Relaxed);
            Err(error)
        };
        if !self.shared.connected.load(Acquire) {
            return fail(SubmissionError::Disconnected);
        }
        for lane in 0..STOP_LANES {
            if state.pending_stops[lane] != 0
                && self.shared.completed_stops[lane].load(Acquire) == state.pending_stops[lane]
            {
                state.pending_stops[lane] = 0;
            }
        }
        let stop_lane = match command {
            Command::Stop => Some(0),
            Command::StopTrack { track } if (track as usize) < super::TRACKS => {
                Some(track as usize + 1)
            }
            _ => None,
        };
        let gate = gate_change(&command);
        let existing_gate =
            gate.and_then(|(key, _)| state.gates.iter().position(|entry| *entry == Some(key)));
        if stop_lane.is_some_and(|lane| state.pending_stops[lane] != 0)
            || gate.is_some_and(|(_, down)| !down && existing_gate.is_none())
        {
            self.shared.coalesced.fetch_add(1, Relaxed);
            return Ok(SubmissionOutcome::Coalesced);
        }
        if blocked_by_stop(&command, &state.pending_stops) {
            return fail(SubmissionError::StopPending);
        }
        let releasing = gate.is_some_and(|(_, down)| !down);
        let reserves_new_gate = gate.is_some_and(|(_, down)| down && existing_gate.is_none());
        if stop_lane.is_none()
            && !releasing
            && self.sender.len() + state.held + STOP_LANES + 1 + usize::from(reserves_new_gate)
                > self.capacity
        {
            return fail(SubmissionError::Full);
        }
        let ticket = state.next_ticket;
        if let Some(lane) = stop_lane {
            command = Command::ReservedStop {
                lane: lane as u8,
                ticket,
            };
        }
        match self.sender.try_send(command) {
            Ok(()) => {
                if let Some(lane) = stop_lane {
                    state.pending_stops[lane] = ticket;
                    state.next_ticket = ticket.wrapping_add(1).max(1);
                }
                if let Some((key, _)) = gate {
                    if reserves_new_gate {
                        let empty = state
                            .gates
                            .iter()
                            .position(Option::is_none)
                            .expect("admitted gate has reserved capacity");
                        state.gates[empty] = Some(key);
                        state.held += 1;
                    }
                    if releasing {
                        state.gates[existing_gate.unwrap()] = None;
                        state.held -= 1;
                    }
                }
                self.shared.accepted.fetch_add(1, Relaxed);
                Ok(SubmissionOutcome::Accepted)
            }
            Err(error) => {
                // Drop rejected command payloads only after releasing the
                // producer mutex (large clips/media must not extend contention).
                drop(state);
                let reason = match &error {
                    crossbeam_channel::TrySendError::Full(_) => SubmissionError::Full,
                    crossbeam_channel::TrySendError::Disconnected(_) => {
                        SubmissionError::Disconnected
                    }
                };
                drop(error);
                fail(reason)
            }
        }
    }
}

fn gate_change(command: &Command) -> Option<(GateKey, bool)> {
    match *command {
        Command::LiveNoteOn { source, ch, note, vel } => Some((GateKey::Live { source, ch: ch & 15, note }, vel != 0)),
        Command::LiveNoteOff { source, ch, note } => Some((GateKey::Live { source, ch: ch & 15, note }, false)),
        Command::SamplerPad { pad, on } => Some((GateKey::Pad(pad % 16), on)),
        Command::DeckTouch { deck, on } => Some((GateKey::Touch(deck % super::DECKS as u8), on)),
        _ => None,
    }
}

fn blocked_by_stop(command: &Command, pending: &[u64; STOP_LANES]) -> bool {
    let clip_track = match *command {
        Command::LaunchClip { track, .. } | Command::FireClip { track, .. } => Some(track as usize),
        _ => None,
    };
    let scene_start = matches!(
        command,
        Command::LaunchScene { .. }
            | Command::RestartScene { .. }
            | Command::AddScene { .. }
            | Command::ToggleScene { .. }
    );
    let transport_start = matches!(
        command,
        Command::Play | Command::TogglePlay | Command::Record
    );
    (pending[0] != 0 && (clip_track.is_some() || scene_start || transport_start))
        || clip_track.is_some_and(|track| track < super::TRACKS && pending[track + 1] != 0)
        || (scene_start && pending[1..].iter().any(|ticket| *ticket != 0))
        || (matches!(command, Command::TogglePlay)
            && pending[1..].iter().any(|ticket| *ticket != 0))
}

#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct CommandStats {
    pub received: u64,
    pub applied: u64,
    pub coalesced: u64,
    pub received_last_block: usize,
    pub applied_last_block: usize,
    pub backlog: usize,
    pub high_water: usize,
    pub budget_exhaustions: u64,
}

pub struct CommandBatch {
    pub commands: [Option<Command>; COMMANDS_PER_BLOCK],
    pub received: usize,
    pub applied: usize,
    pub backlog: usize,
    pub queue_depth: usize,
}

impl CommandBatch {
    pub fn receive(rx: &Receiver<Command>) -> Self {
        let mut batch = Self {
            commands: std::array::from_fn(|_| None),
            received: 0,
            applied: 0,
            backlog: 0,
            queue_depth: rx.len(),
        };
        for _ in 0..COMMANDS_PER_BLOCK {
            let Ok(command) = rx.try_recv() else { break };
            batch.received += 1;
            // Do not reorder even apparently independent assignments: later
            // controls may acquire coupled semantics. Events are always barriers.
            let duplicate = batch.applied > 0
                && parameter_key(&command).is_some()
                && parameter_key(&command)
                    == batch.commands[batch.applied - 1]
                        .as_ref()
                        .and_then(parameter_key);
            if !duplicate {
                batch.applied += 1;
            }
            batch.commands[batch.applied - 1] = Some(command);
        }
        batch.backlog = rx.len();
        batch
    }
}

impl CommandStats {
    pub fn record(&mut self, batch: &CommandBatch) {
        self.received = self.received.saturating_add(batch.received as u64);
        self.applied = self.applied.saturating_add(batch.applied as u64);
        self.coalesced = self
            .coalesced
            .saturating_add((batch.received - batch.applied) as u64);
        self.received_last_block = batch.received;
        self.applied_last_block = batch.applied;
        self.backlog = batch.backlog;
        self.high_water = self.high_water.max(batch.queue_depth).max(batch.backlog);
        if batch.received == COMMANDS_PER_BLOCK && batch.backlog > 0 {
            self.budget_exhaustions = self.budget_exhaustions.saturating_add(1);
        }
    }
}

// Only absolute assignments; jog, seek, note, transport, instrument changes,
// relative controls and target selection must retain every event.
fn parameter_key(command: &Command) -> Option<(u8, usize, u8)> {
    match *command {
        Command::SetBpm(_) => Some((0, 0, 0)),
        Command::Xfader(_) => Some((1, 0, 0)),
        Command::Master(_) => Some((2, 0, 0)),
        Command::CueMix(_) => Some((3, 0, 0)),
        Command::TrackGain { track, .. } => Some((4, track as usize, 0)),
        Command::TrackPan { track, .. } => Some((5, track as usize, 0)),
        Command::DeckPitch { deck, .. } => Some((6, deck as usize, 0)),
        Command::DeckGain { deck, .. } => Some((7, deck as usize, 0)),
        Command::DeckEq { deck, band, .. } => Some((8, deck as usize, band)),
        Command::DeckFilter { deck, .. } => Some((9, deck as usize, 0)),
        Command::FxWet { slot, .. } => Some((10, slot as usize, 0)),
        Command::FxMix { slot, .. } => Some((11, slot, 0)),
        Command::FxParam { slot, p, .. } => Some((12, slot, p)),
        Command::Quant(_) => Some((13, 0, 0)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parameter_updates_coalesce_without_crossing_event_or_target_boundaries() {
        let (tx, rx) = crossbeam_channel::bounded(32);
        for command in [
            Command::Xfader(0.1),
            Command::Xfader(0.8),
            Command::LiveNoteOn {
                source: 0,
                ch: 0,
                note: 60,
                vel: 100,
            },
            Command::Xfader(0.3),
            Command::Xfader(0.4),
            Command::LiveNoteOff { source: 0, ch: 0, note: 60 },
            Command::DeckPitch {
                deck: 0,
                value: 0.2,
            },
            Command::DeckPitch {
                deck: 1,
                value: 0.6,
            },
            Command::Stop,
            Command::Play,
            Command::DeckJog {
                deck: 0,
                delta: 1.0,
            },
            Command::DeckJog {
                deck: 0,
                delta: -1.0,
            },
            Command::SamplerPad { pad: 0, on: true },
            Command::SamplerPad { pad: 0, on: false },
        ] {
            tx.send(command).unwrap();
        }
        let batch = CommandBatch::receive(&rx);
        assert_eq!(batch.received, 14);
        assert_eq!(batch.applied, 12);
        let commands: Vec<_> = batch.commands.into_iter().flatten().collect();
        assert!(matches!(commands[0], Command::Xfader(v) if v == 0.8));
        assert!(matches!(commands[1], Command::LiveNoteOn { .. }));
        assert!(matches!(commands[2], Command::Xfader(v) if v == 0.4));
        assert!(matches!(commands[3], Command::LiveNoteOff { .. }));
        assert!(matches!(commands[4], Command::DeckPitch { deck: 0, .. }));
        assert!(matches!(commands[5], Command::DeckPitch { deck: 1, .. }));
        assert!(matches!(commands[6], Command::Stop));
        assert!(matches!(commands[7], Command::Play));
        assert!(matches!(commands[8], Command::DeckJog { delta, .. } if delta == 1.0));
        assert!(matches!(commands[9], Command::DeckJog { delta, .. } if delta == -1.0));
        assert!(matches!(commands[10], Command::SamplerPad { on: true, .. }));
        assert!(matches!(
            commands[11],
            Command::SamplerPad { on: false, .. }
        ));
    }

    #[test]
    fn dequeue_budget_counts_received_commands_even_when_all_coalesce() {
        let (tx, rx) = crossbeam_channel::bounded(256);
        for i in 0..256 {
            tx.send(Command::Master(i as f32 / 256.0)).unwrap();
        }
        let mut stats = CommandStats::default();
        for block in 0..8 {
            let batch = CommandBatch::receive(&rx);
            assert_eq!(batch.received, COMMANDS_PER_BLOCK);
            assert_eq!(batch.applied, 1);
            assert_eq!(batch.backlog, 256 - (block + 1) * COMMANDS_PER_BLOCK);
            stats.record(&batch);
        }
        assert_eq!(stats.received, 256);
        assert_eq!(stats.applied, 8);
        assert_eq!(stats.coalesced, 248);
        assert_eq!(stats.budget_exhaustions, 7);
        assert_eq!(stats.high_water, 256);
        assert_eq!(stats.backlog, 0);
        stats.record(&CommandBatch::receive(&rx));
        assert_eq!(stats.received_last_block, 0);
        assert_eq!(stats.applied_last_block, 0);
    }
}
