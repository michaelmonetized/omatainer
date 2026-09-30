//! Bounded dequeue work. Musical events stay FIFO; only adjacent assignments to
//! the same absolute parameter may be replaced by their latest value.
use super::Command;
use crossbeam_channel::Receiver;
use serde::Serialize;

pub const COMMANDS_PER_BLOCK: usize = 32;

/// A producer handle deliberately contains no renderer or renderer lock.
#[derive(Clone)]
pub struct CommandPort {
    sender: crossbeam_channel::Sender<Command>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SubmissionError {
    Full,
    Disconnected,
}

impl std::fmt::Display for SubmissionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Full => "The control queue is full. Retry the action.",
            Self::Disconnected => "Audio has disconnected. Restart Omatainer before retrying.",
        })
    }
}

impl std::error::Error for SubmissionError {}

impl CommandPort {
    pub fn new(sender: crossbeam_channel::Sender<Command>) -> Self {
        Self { sender }
    }

    /// Success means queued for a subsequent audio block, not executed yet.
    /// The producer receives failures immediately; it never waits on audio.
    pub fn send(&self, command: Command) -> Result<(), SubmissionError> {
        self.sender.try_send(command).map_err(|error| match error {
            crossbeam_channel::TrySendError::Full(_) => SubmissionError::Full,
            crossbeam_channel::TrySendError::Disconnected(_) => SubmissionError::Disconnected,
        })
    }
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
                ch: 0,
                note: 60,
                vel: 100,
            },
            Command::Xfader(0.3),
            Command::Xfader(0.4),
            Command::LiveNoteOff { ch: 0, note: 60 },
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
