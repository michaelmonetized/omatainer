//! Bounded dequeue work. Musical events stay FIFO; only adjacent assignments to
//! the same absolute parameter may be replaced by their latest value.
use super::Command;
use serde::Serialize;

#[cfg(test)]
mod admission_tests;
#[cfg(test)]
mod payload_tests;
#[cfg(test)]
mod project_gate_tests;

pub const COMMANDS_PER_BLOCK: usize = 32;
/// Variable owned payload in the incoming queue, independently of undo storage.
pub const MAX_QUEUED_PAYLOAD_BYTES: usize = 256 * 1024 * 1024;

/// A producer handle contains no renderer or renderer lock. Its short mutex
/// serializes admission bookkeeping only; the audio consumer never acquires it.
#[derive(Clone)]
pub struct CommandPort {
    theme_requests: crate::theme::requests::Port,
    sender: crossbeam_channel::Sender<Command>,
    shared: std::sync::Arc<AdmissionShared>,
    admission: std::sync::Arc<parking_lot::Mutex<Admission>>,
    capacity: usize,
}

const STOP_LANES: usize = super::TRACKS + 1;
pub(super) const MAX_COMMANDS: usize = 256;
const PROJECT_CLOSED: u64 = 1 << 63;

/// One word makes closing admission atomic with a producer's claim. Only
/// producers retry a competing CAS; audio performs one fetch_or per block.
struct ProjectLease<'a>(&'a std::sync::atomic::AtomicU64);
impl<'a> ProjectLease<'a> {
    fn acquire(word: &'a std::sync::atomic::AtomicU64, release: bool) -> Option<Self> {
        use std::sync::atomic::Ordering::{AcqRel, Acquire};
        let mut state = word.load(Acquire);
        loop {
            if (state & PROJECT_CLOSED != 0 && !release)
                || state & !PROJECT_CLOSED == PROJECT_CLOSED - 1
            {
                return None;
            }
            match word.compare_exchange_weak(state, state + 1, AcqRel, Acquire) {
                Ok(_) => return Some(Self(word)),
                Err(next) => state = next,
            }
        }
    }
}
impl Drop for ProjectLease<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, std::sync::atomic::Ordering::Release);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GateKey {
    Live { source: u64, ch: u8, note: u8 },
    Pad(u8),
    Touch { source: u64, deck: u8 },
}

struct Admission {
    gates: [Option<GateKey>; MAX_COMMANDS],
    held: usize,
    pending_stops: [u64; STOP_LANES],
    next_ticket: u64,
}

struct AdmissionShared {
    project_writers: std::sync::atomic::AtomicU64,
    audio_offline: std::sync::atomic::AtomicBool,
    queued_payload_bytes: std::sync::atomic::AtomicUsize,
    payload_limit: usize,
    history_available: std::sync::atomic::AtomicBool,
    telemetry: std::sync::Arc<super::audio_metrics::Telemetry>,
    ui_requests: super::ui_requests::Mailbox,
    completed_stops: [std::sync::atomic::AtomicU64; STOP_LANES],
    connected: std::sync::atomic::AtomicBool,
    accepted: std::sync::atomic::AtomicU64,
    coalesced: std::sync::atomic::AtomicU64,
    rejected: std::sync::atomic::AtomicU64,
    last_error: std::sync::atomic::AtomicU8,
    observed_high_water: std::sync::atomic::AtomicU64,
    reserved_releases: std::sync::atomic::AtomicU64,
    full_rejections: std::sync::atomic::AtomicU64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
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
                4 => Some(SubmissionError::InvalidTarget),
                5 => Some(SubmissionError::UiUnavailable),
                6 => Some(SubmissionError::UiFull),
                7 => Some(SubmissionError::UncapturedSelection),
                8 => Some(SubmissionError::ProjectChanging),
                9 => Some(SubmissionError::HistoryBusy),
                10 => Some(SubmissionError::PayloadFull),
                11 => Some(SubmissionError::AudioUnavailable),
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
    pub fn len(&self) -> usize {
        self.receiver.len()
    }
    pub fn is_empty(&self) -> bool {
        self.receiver.is_empty()
    }

    /// Single-command consumers release credit on dequeue. The renderer batch
    /// holds all credits until its bounded receive loop has finished instead.
    pub fn try_recv(&self) -> Result<Command, crossbeam_channel::TryRecvError> {
        self.receiver
            .try_recv()
            .inspect(|command| self.release_payload(owned_payload_bytes(command)))
    }

    #[cfg(test)]
    pub fn try_iter(&self) -> impl Iterator<Item = Command> + '_ {
        std::iter::from_fn(|| self.try_recv().ok())
    }

    fn release_payload(&self, bytes: usize) {
        if bytes != 0 {
            if let Some(shared) = &self.shared {
                let before = shared
                    .queued_payload_bytes
                    .fetch_sub(bytes, std::sync::atomic::Ordering::AcqRel);
                debug_assert!(before >= bytes, "payload reservation underflow");
            }
        }
    }

    pub(super) fn set_history_available(&self, available: bool) {
        if let Some(shared) = &self.shared {
            shared
                .history_available
                .store(available, std::sync::atomic::Ordering::Release);
        }
    }

    /// Close creative command admission before inspecting/draining the queue for a
    /// project install. False means a previously admitted producer is still
    /// finishing; keep admission closed and retry on a later audio block. This
    /// never acquires a producer mutex, spins, allocates, or waits. Physical
    /// releases/stops remain admissible, including after this returns true: they
    /// are safe on a stopped replacement, and must not get lost if install aborts.
    pub(super) fn begin_project_install(&self) -> bool {
        self.shared.as_ref().is_none_or(|shared| {
            shared
                .project_writers
                .fetch_or(PROJECT_CLOSED, std::sync::atomic::Ordering::AcqRel)
                & !PROJECT_CLOSED
                == 0
        })
    }

    /// Reopen after commit or abort. The renderer must retain the closed gate
    /// until earlier leases have retired and their accepted commands drained.
    pub(super) fn end_project_install(&self) {
        if let Some(shared) = &self.shared {
            shared
                .project_writers
                .fetch_and(!PROJECT_CLOSED, std::sync::atomic::Ordering::Release);
        }
    }

    /// Owner only, while the exclusive audio seal is held. Offline service
    /// continues project I/O without admitting creative performance commands.
    pub(super) fn set_audio_offline(&self, offline: bool) {
        if let Some(shared) = &self.shared { shared.audio_offline.store(offline, std::sync::atomic::Ordering::Release); }
    }

    pub(super) fn pending_project_ui_requests(&self) -> bool {
        self.shared
            .as_ref()
            .is_some_and(|shared| shared.ui_requests.stats().pending != 0)
    }

    pub(super) fn telemetry(&self) -> std::sync::Arc<super::audio_metrics::Telemetry> {
        self.shared
            .as_ref()
            .map(|shared| shared.telemetry.clone())
            .unwrap_or_default()
    }

    pub(super) fn reject_uncaptured_ui_load(&self) {
        if let Some(shared) = &self.shared {
            // A raw audio-side command has no captured GUI selection. Report
            // failure using atomics; never acquire/drop media Arcs here or
            // silently resolve a different, later selection.
            shared.ui_requests.reject_uncaptured();
            let _ = shared.reject(SubmissionError::UncapturedSelection);
        }
    }

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
    InvalidTarget,
    UiUnavailable,
    UiFull,
    UncapturedSelection,
    ProjectChanging,
    HistoryBusy,
    PayloadFull,
    AudioUnavailable,
}

impl std::fmt::Display for SubmissionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::AudioUnavailable => "Audio output is unavailable. Save or close the session, or recover an output in Audio settings before performing.",
            Self::PayloadFull => "The control queue has reached its media and edit memory limit. Wait for playback to catch up, then retry with a smaller edit or media file.",
            Self::Full => "The control queue is full. Releases remain reserved. Wait for playback to catch up, then retry the action.",
            Self::StopPending => "A stop is still pending for this target. Retry the start after the stop has completed.",
            Self::Disconnected => "Audio has disconnected. Restart Omatainer before retrying.",
            Self::InvalidTarget => "The requested control target does not exist.",
            Self::UiUnavailable => "Library control is unavailable. Reopen Omatainer before retrying.",
            Self::UiFull => "The library request queue is full. Wait for the interface to catch up, then retry browsing or loading.",
            Self::UncapturedSelection => "Library request could not resolve the visible selection. Select an available crate item, then retry.",
            Self::HistoryBusy => "Undo storage is busy. This edit was not accepted; wait for the history worker and retry.",
            Self::ProjectChanging => "A project is being installed. This action was not accepted; retry after the project operation finishes.",
        })
    }
}

impl std::error::Error for SubmissionError {}

impl AdmissionShared {
    fn reject(&self, error: SubmissionError) -> Result<SubmissionOutcome, SubmissionError> {
        use std::sync::atomic::Ordering::Relaxed;
        if error == SubmissionError::Full {
            self.full_rejections.fetch_add(1, Relaxed);
        }
        self.last_error.store(
            match error {
                SubmissionError::Full => 1,
                SubmissionError::StopPending => 2,
                SubmissionError::Disconnected => 3,
                SubmissionError::InvalidTarget => 4,
                SubmissionError::UiUnavailable => 5,
                SubmissionError::UiFull => 6,
                SubmissionError::UncapturedSelection => 7,
                SubmissionError::ProjectChanging => 8,
                SubmissionError::HistoryBusy => 9,
                SubmissionError::PayloadFull => 10,
                SubmissionError::AudioUnavailable => 11,
            },
            Relaxed,
        );
        self.rejected.fetch_add(1, Relaxed);
        Err(error)
    }

    fn submit_ui(
        &self,
        result: Result<SubmissionOutcome, SubmissionError>,
    ) -> Result<SubmissionOutcome, SubmissionError> {
        use std::sync::atomic::Ordering::Relaxed;
        match result {
            Ok(outcome) => {
                match outcome {
                    SubmissionOutcome::Accepted => &self.accepted,
                    SubmissionOutcome::Coalesced => &self.coalesced,
                }
                .fetch_add(1, Relaxed);
                Ok(outcome)
            }
            Err(error) => self.reject(error),
        }
    }
}

#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize)]
pub struct QueuePressure {
    pub pending: usize,
    pub capacity: usize,
    pub observed_high_water: u64,
    pub reserved_releases: u64,
    pub full_rejections: u64,
    pub rejected: u64,
    pub accepted: u64,
    pub coalesced: u64,
}

impl CommandPort {
    pub fn queue_pressure(&self) -> QueuePressure {
        use std::sync::atomic::Ordering::Relaxed;
        QueuePressure {
            pending: self.sender.len(),
            capacity: self.capacity,
            observed_high_water: self.shared.observed_high_water.load(Relaxed),
            reserved_releases: self.shared.reserved_releases.load(Relaxed),
            full_rejections: self.shared.full_rejections.load(Relaxed),
            rejected: self.shared.rejected.load(Relaxed),
            accepted: self.shared.accepted.load(Relaxed),
            coalesced: self.shared.coalesced.load(Relaxed),
        }
    }
    pub fn set_profiling(&self, enabled: bool) {
        self.shared
            .telemetry
            .profiler
            .enabled
            .store(enabled, std::sync::atomic::Ordering::Relaxed);
    }
    pub fn load_profile(&self) -> Option<super::diagnostics::Profile> {
        self.shared.telemetry.profiler.read()
    }
    pub fn audio_metrics(&self) -> super::audio_metrics::AudioMetrics {
        self.shared.telemetry.read()
    }

    pub(crate) fn is_connected(&self) -> bool {
        self.shared
            .connected
            .load(std::sync::atomic::Ordering::Acquire)
    }

    pub fn channel(capacity: usize) -> (Self, CommandReceiver) {
        Self::channel_with_payload_limit(capacity, MAX_QUEUED_PAYLOAD_BYTES)
    }

    fn channel_with_payload_limit(
        capacity: usize,
        payload_limit: usize,
    ) -> (Self, CommandReceiver) {
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
            project_writers: std::sync::atomic::AtomicU64::new(0),
            audio_offline: std::sync::atomic::AtomicBool::new(false),
            queued_payload_bytes: std::sync::atomic::AtomicUsize::new(0),
            payload_limit,
            history_available: std::sync::atomic::AtomicBool::new(true),
            telemetry: std::sync::Arc::new(super::audio_metrics::Telemetry::default()),
            ui_requests: super::ui_requests::Mailbox::default(),
            completed_stops: std::array::from_fn(|_| std::sync::atomic::AtomicU64::new(0)),
            connected: std::sync::atomic::AtomicBool::new(true),
            accepted: std::sync::atomic::AtomicU64::new(0),
            coalesced: std::sync::atomic::AtomicU64::new(0),
            rejected: std::sync::atomic::AtomicU64::new(0),
            last_error: std::sync::atomic::AtomicU8::new(0),
            observed_high_water: std::sync::atomic::AtomicU64::new(0),
            reserved_releases: std::sync::atomic::AtomicU64::new(0),
            full_rejections: std::sync::atomic::AtomicU64::new(0),
        });
        let port = Self {
            theme_requests: crate::theme::requests::Port::default(),
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

    pub(crate) fn theme_requests(&self) -> &crate::theme::requests::Port { &self.theme_requests }

    pub fn len(&self) -> usize {
        self.sender.len()
    }
    pub fn stats(&self) -> SubmissionStats {
        self.shared.stats()
    }

    #[cfg(test)]
    pub(super) fn with_admission_held_for_test(&self, test: impl FnOnce()) {
        let _guard = self.admission.lock();
        test();
    }

    /// Used only by the off-callback MIDI worker. Each accepted onset reserves
    /// its release, so input overflow can retire exactly that source's gates
    /// even while ordinary engine queue capacity is exhausted.
    pub(super) fn release_midi_source(&self, source: u64) {
        let gates = self.admission.lock().gates;
        for gate in gates.into_iter().flatten() {
            let command = match gate {
                GateKey::Live {
                    source: owner,
                    ch,
                    note,
                } if owner == source => Command::LiveNoteOff { source, ch, note },
                GateKey::Touch {
                    source: owner,
                    deck,
                } if owner == source => Command::MidiDeckTouch {
                    source,
                    deck,
                    on: false,
                },
                _ => continue,
            };
            let _ = self.send(command);
        }
    }

    pub(crate) fn take_ui_receiver(&self) -> Option<super::ui_requests::Receiver> {
        self.shared.ui_requests.receiver()
    }

    pub fn ui_request_stats(&self) -> super::ui_requests::Stats {
        self.shared.ui_requests.stats()
    }

    /// Accepted means queued, not executed. Coalesced means an equivalent
    /// release is already ordered and no intervening accepted onset exists.
    /// No producer waits for queue capacity; only the small admission section
    /// serializes producers. Construct media/instruments before calling here.
    pub fn send(&self, command: Command) -> Result<SubmissionOutcome, SubmissionError> {
        self.send_after_preflight(command, || {})
    }

    fn send_after_preflight(
        &self,
        mut command: Command,
        after_preflight: impl FnOnce(),
    ) -> Result<SubmissionOutcome, SubmissionError> {
        use std::sync::atomic::Ordering::{Acquire, Relaxed};
        let fail = |error| self.shared.reject(error);
        // Covers every route, including GUI browse/load and early failures.
        // Closing admission races this atomic claim, never the producer mutex.
        let Some(_project_lease) =
            ProjectLease::acquire(&self.shared.project_writers, project_release(&command))
        else {
            return fail(SubmissionError::ProjectChanging);
        };
        if !self.shared.connected.load(Acquire) {
            return fail(SubmissionError::Disconnected);
        }
        if self.shared.audio_offline.load(Acquire) && !project_release(&command) {
            return fail(SubmissionError::AudioUnavailable);
        }
        if !self.shared.history_available.load(Acquire) && !history_monitoring(&command) {
            return fail(SubmissionError::HistoryBusy);
        }
        if let Command::Gesture { command: inner, .. } = &command {
            if !super::undo::is_gesture_edit(inner) {
                return fail(SubmissionError::InvalidTarget);
            }
        }
        if let Command::DeckLoadSelected { deck } = command {
            return self.shared.submit_ui(self.shared.ui_requests.load(deck));
        }
        if let Command::Browse(steps) = command {
            return self.shared.submit_ui(self.shared.ui_requests.browse(steps));
        }
        if matches!(&command, Command::FxSelect { slot } | Command::FxWet { slot, .. } if *slot >= 3)
        {
            return fail(SubmissionError::InvalidTarget);
        }
        after_preflight();
        let mut state = self.admission.lock();
        // The receiver may have disconnected while this producer waited for
        // another producer's bookkeeping. Never coalesce against dead audio.
        if !self.shared.connected.load(Acquire) {
            return fail(SubmissionError::Disconnected);
        }
        if self.shared.audio_offline.load(Acquire) && !project_release(&command) {
            return fail(SubmissionError::AudioUnavailable);
        }
        // A producer may have passed preflight before waiting on this mutex.
        // Recycler backpressure must also cover that waiting producer, so the
        // renderer only inherits payloads already admitted before it closed.
        if !self.shared.history_available.load(Acquire) && !history_monitoring(&command) {
            return fail(SubmissionError::HistoryBusy);
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
        let payload_bytes = owned_payload_bytes(&command);
        if !self.shared.reserve_payload(payload_bytes) {
            return fail(SubmissionError::PayloadFull);
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
                self.shared
                    .observed_high_water
                    .fetch_max(self.sender.len() as u64, Relaxed);
                self.shared
                    .reserved_releases
                    .store(state.held as u64, Relaxed);
                self.shared.accepted.fetch_add(1, Relaxed);
                Ok(SubmissionOutcome::Accepted)
            }
            Err(error) => {
                self.shared
                    .queued_payload_bytes
                    .fetch_sub(payload_bytes, std::sync::atomic::Ordering::AcqRel);
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

impl AdmissionShared {
    fn reserve_payload(&self, bytes: usize) -> bool {
        use std::sync::atomic::Ordering::{AcqRel, Acquire};
        if bytes == 0 {
            return true;
        }
        self.queued_payload_bytes
            .fetch_update(AcqRel, Acquire, |used| {
                used.checked_add(bytes)
                    .filter(|total| *total <= self.payload_limit)
            })
            .is_ok()
    }
}

/// Counts allocated capacity rather than logical lengths, without traversing
/// PCM or notes. Immutable shared samples are conservatively charged once per
/// queued reference. Fixed-size receipt/token Arcs have a separate bound from
/// the maximum number of queued commands; sample Arc headers are included here.
fn owned_payload_bytes(command: &Command) -> usize {
    use std::mem::size_of;
    match command {
        Command::Gesture { command, .. } => {
            size_of::<Command>().saturating_add(owned_payload_bytes(command))
        }
        Command::SetNotes { notes, .. } => notes
            .capacity()
            .saturating_mul(size_of::<super::MidiNote>()),
        Command::LearnCapture { param, .. } => param.capacity(),
        Command::DeckAudio { audio, .. }
        | Command::DeckDecoded { audio, .. }
        | Command::DeckLoadRequested {
            media: super::load_receipt::Media::Decoded { audio, .. },
            ..
        } => {
            size_of::<super::dsp::Sample>()
                .saturating_add(4 * size_of::<usize>()) // Sample and peaks Arc counters.
                .saturating_add(size_of::<Vec<[f32; 3]>>())
                .saturating_add(audio.data.capacity().saturating_mul(size_of::<f32>()))
                .saturating_add(audio.name.capacity())
                .saturating_add(audio.path.capacity())
                .saturating_add(audio.peaks.capacity().saturating_mul(size_of::<[f32; 3]>()))
        }
        _ => 0,
    }
}

fn project_release(command: &Command) -> bool {
    matches!(
        command,
        Command::LibraryFence { .. }
            | Command::LiveNoteOff { .. }
            | Command::LiveNoteOn { vel: 0, .. }
            | Command::SamplerPad { on: false, .. }
            | Command::DeckTouch { on: false, .. }
            | Command::MidiDeckTouch { on: false, .. }
            | Command::Stop
            | Command::StopTrack { .. }
            | Command::ReservedStop { .. }
    )
}

#[cfg(test)]
mod gui_routing_tests {
    use super::*;
    use crate::engine::media_source::{BuiltinStem, LibSource, Selection};
    use std::sync::{mpsc, Arc};
    use std::time::Duration;

    #[test]
    fn library_handoff_does_not_wait_for_audio_producer_admission() {
        let (commands, _audio) = CommandPort::channel(32);
        let gui = commands.take_ui_receiver().unwrap();
        gui.publish_selection(Some(Arc::new(Selection {
            source: LibSource::Builtin(BuiltinStem::Harmony),
            title: "Harmony".into(),
        })));
        let locked = commands.admission.lock();
        let producer = commands.clone();
        let (done, received) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            done.send(producer.send(Command::DeckLoadSelected { deck: 1 }))
                .unwrap();
        });
        let result = received.recv_timeout(Duration::from_secs(1));
        drop(locked);
        worker.join().unwrap();
        assert_eq!(result.unwrap(), Ok(SubmissionOutcome::Accepted));
        let super::super::ui_requests::Request::Load(request) =
            gui.take_requests()[0].take().unwrap()
        else {
            panic!("expected load")
        };
        assert_eq!(request.deck, 1);
        assert_eq!(
            request.selection.source,
            LibSource::Builtin(BuiltinStem::Harmony)
        );
        assert_eq!(commands.len(), 0);
    }
}

fn gate_change(command: &Command) -> Option<(GateKey, bool)> {
    match *command {
        Command::LiveNoteOn {
            source,
            ch,
            note,
            vel,
        } => Some((
            GateKey::Live {
                source,
                ch: ch & 15,
                note,
            },
            vel != 0,
        )),
        Command::LiveNoteOff { source, ch, note } => Some((
            GateKey::Live {
                source,
                ch: ch & 15,
                note,
            },
            false,
        )),
        Command::SamplerPad { pad, on } => Some((GateKey::Pad(pad % 16), on)),
        Command::DeckTouch { deck, on } => Some((
            GateKey::Touch {
                source: 0,
                deck: deck % super::DECKS as u8,
            },
            on,
        )),
        Command::MidiDeckTouch { source, deck, on } => Some((
            GateKey::Touch {
                source,
                deck: deck % super::DECKS as u8,
            },
            on,
        )),
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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
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
    pub discarded: [Option<Command>; COMMANDS_PER_BLOCK],
    pub commands: [Option<Command>; COMMANDS_PER_BLOCK],
    pub received: usize,
    pub applied: usize,
    pub backlog: usize,
    pub queue_depth: usize,
}

impl CommandBatch {
    pub fn receive(rx: &CommandReceiver) -> Self {
        Self::receive_with(rx, || {})
    }

    fn receive_with(rx: &CommandReceiver, mut after_pop: impl FnMut()) -> Self {
        let mut batch = Self {
            discarded: std::array::from_fn(|_| None),
            commands: std::array::from_fn(|_| None),
            received: 0,
            applied: 0,
            backlog: 0,
            queue_depth: rx.len(),
        };
        let mut payload_bytes = 0usize;
        for _ in 0..COMMANDS_PER_BLOCK {
            let Ok(command) = rx.receiver.try_recv() else {
                break;
            };
            payload_bytes = payload_bytes.saturating_add(owned_payload_bytes(&command));
            after_pop();
            batch.received += 1;
            // Do not reorder even apparently independent assignments: later
            // controls may acquire coupled semantics. Events are always barriers.
            let duplicate = batch.applied > 0
                && batch.commands[batch.applied - 1]
                    .as_ref()
                    .is_some_and(|previous| same_parameter(previous, &command));
            if !duplicate {
                batch.applied += 1;
            }
            let old = batch.commands[batch.applied - 1].replace(command);
            if duplicate {
                batch.discarded[batch.received - 1] = old;
            }
        }
        // Releasing credit after each pop would let producers refill a full
        // 256 MiB between every pop, retaining 32 queues' worth in this batch.
        // Holding credits through receipt caps the entire batch at one queue.
        rx.release_payload(payload_bytes);
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
        Command::ClipGain { track, scene, .. } => Some((14, track as usize, scene)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parameter_updates_coalesce_without_crossing_event_or_target_boundaries() {
        let (tx, rx) = crossbeam_channel::bounded(32);
        let rx = CommandReceiver::from(rx);
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
            Command::LiveNoteOff {
                source: 0,
                ch: 0,
                note: 60,
            },
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
        let rx = CommandReceiver::from(rx);
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

fn history_monitoring(command: &Command) -> bool {
    matches!(
        command,
        Command::LiveNoteOn { .. }
            | Command::LiveNoteOff { .. }
            | Command::SamplerPad { .. }
            | Command::DeckTouch { .. }
            | Command::MidiDeckTouch { .. }
            | Command::Stop
            | Command::StopTrack { .. }
            | Command::ReservedStop { .. }
            | Command::Play
            | Command::TogglePlay
            | Command::MidiClock { .. }
    )
}

fn same_parameter(a: &Command, b: &Command) -> bool {
    match (a, b) {
        (Command::Gesture { id: a, command: ac }, Command::Gesture { id: b, command: bc }) => {
            a == b && parameter_key(ac).is_some() && parameter_key(ac) == parameter_key(bc)
        }
        (Command::Gesture { .. }, _) | (_, Command::Gesture { .. }) => false,
        _ => parameter_key(a).is_some() && parameter_key(a) == parameter_key(b),
    }
}
