//! Complete project handoff. File I/O, validation, allocation, DSP construction
//! and retirement run on the project worker. Audio copies into prepared storage
//! or swaps an owned graph at a block boundary; it never waits for that worker.
mod capture;
mod model;
mod prepare;
use super::*;
use crossbeam_channel::{bounded, Receiver, Sender};
pub use model::State;
pub use prepare::Prepared;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};

const PENDING: u8 = 0;
const APPLYING: u8 = 1;
const READY: u8 = 2;
const CANCELLED: u8 = 3;
const WAIT_LIMIT: Duration = Duration::from_secs(10);

#[derive(Debug)]
pub enum Error {
    Busy,
    Cancelled,
    Conflict,
    Invalid(String),
    Unavailable,
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Busy => f.write_str("a project operation is still pending"),
            Self::Cancelled => f.write_str("project operation cancelled"),
            Self::Conflict => {
                f.write_str("project changed during this operation; current work was preserved")
            }
            Self::Invalid(reason) => write!(f, "invalid project: {reason}"),
            Self::Unavailable => {
                f.write_str("audio renderer did not acknowledge the project operation")
            }
        }
    }
}
impl std::error::Error for Error {}

pub struct Captured {
    pub checkpoint: undo::Checkpoint,
    pub state: State,
    pub media: Vec<Arc<Sample>>,
    pub revision: u64,
    pub playback_receipts: [Option<load_receipt::Receipt>; DECKS],
}

pub struct Applied {
    pub checkpoint: undo::Checkpoint,
    pub revision: u64,
    pub playback_receipts: [Option<load_receipt::Receipt>; DECKS],
}

/// Keeps creative admission closed through the GUI's final Close event.
/// Dropping on any thread only publishes an atomic release request; the next
/// audio block reopens admission. No graph, media or worker join belongs here.
pub struct CloseGuard {
    handle: Handle,
}
impl Drop for CloseGuard {
    fn drop(&mut self) {
        self.handle
            .shared
            .release_seal
            .store(true, Ordering::Release);
    }
}
impl std::fmt::Debug for CloseGuard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("CloseGuard")
    }
}

#[derive(Clone)]
pub struct Handle {
    shared: Arc<Shared>,
}
struct Shared {
    request: Sender<Box<Task>>,
    incoming: Receiver<Box<Task>>,
    completed: Sender<Box<Task>>,
    results: Receiver<Box<Task>>,
    busy: AtomicU8,
    revision: AtomicU64,
    release_seal: AtomicBool,
    #[cfg(test)]
    wait_limit_millis: AtomicU64,
}

pub(super) struct Task {
    operation: Operation,
    stage: Arc<AtomicU8>,
    error: Option<Error>,
}
enum Operation {
    Capture(capture::Frame),
    Seal {
        expected: Option<u64>,
        guard: Option<CloseGuard>,
    },
    Install {
        prepared: Prepared,
        expected: u64,
        applied: Option<Applied>,
    },
}

impl Handle {
    pub(super) fn new() -> Self {
        let (request, incoming) = bounded(1);
        let (completed, results) = bounded(1);
        Self {
            shared: Arc::new(Shared {
                request,
                incoming,
                completed,
                results,
                busy: AtomicU8::new(0),
                revision: AtomicU64::new(0),
                release_seal: AtomicBool::new(false),
                #[cfg(test)]
                wait_limit_millis: AtomicU64::new(WAIT_LIMIT.as_millis() as u64),
            }),
        }
    }
    /// This atomic read is safe on the UI thread.
    pub fn revision(&self) -> u64 {
        self.shared.revision.load(Ordering::Acquire)
    }
    pub(super) fn edited(&self) {
        self.shared.revision.fetch_add(1, Ordering::Release);
    }

    #[cfg(test)]
    pub(crate) fn set_wait_limit_for_test(&self, limit: Duration) {
        self.shared
            .wait_limit_millis
            .store(limit.as_millis() as u64, Ordering::Release);
    }

    /// Worker only. A failed/cancelled prior operation can leave one bounded
    /// cancelled task in flight. Retire its eventual reply here, never on audio.
    fn begin(&self) -> Result<(), Error> {
        if self.shared.busy.load(Ordering::Acquire) == 2 {
            if self.shared.results.try_recv().is_ok() {
                self.shared.busy.store(0, Ordering::Release);
            }
        }
        self.shared
            .busy
            .compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire)
            .map(|_| ())
            .map_err(|_| Error::Busy)
    }

    fn exchange(&self, task: Box<Task>, cancel: &AtomicBool) -> Result<Box<Task>, Error> {
        let stage = task.stage.clone();
        if self.shared.request.try_send(task).is_err() {
            self.shared.busy.store(0, Ordering::Release);
            return Err(Error::Unavailable);
        }
        let start = Instant::now();
        #[cfg(test)]
        let wait_limit =
            Duration::from_millis(self.shared.wait_limit_millis.load(Ordering::Acquire));
        #[cfg(not(test))]
        let wait_limit = WAIT_LIMIT;
        loop {
            if let Ok(task) = self.shared.results.recv_timeout(Duration::from_millis(5)) {
                return Ok(task);
            }
            let cancelled = cancel.load(Ordering::Acquire);
            if cancelled || start.elapsed() >= wait_limit {
                // The audio side atomically claims installation. A cancellation
                // that loses that claim must wait for the authoritative result.
                if stage
                    .compare_exchange(PENDING, CANCELLED, Ordering::AcqRel, Ordering::Acquire)
                    .is_ok()
                {
                    self.shared.busy.store(2, Ordering::Release);
                    return Err(if cancelled {
                        Error::Cancelled
                    } else {
                        Error::Unavailable
                    });
                }
            }
        }
    }

    /// Worker only: coherent capture, with capacity preparation between passes.
    pub fn capture(&self, cancel: &AtomicBool) -> Result<Captured, Error> {
        self.begin()?;
        let mut task = Box::new(Task {
            operation: Operation::Capture(capture::Frame::new()),
            stage: Arc::new(AtomicU8::new(PENDING)),
            error: None,
        });
        loop {
            task = self.exchange(task, cancel)?;
            if let Some(error) = task.error.take() {
                self.shared.busy.store(0, Ordering::Release);
                return Err(error);
            }
            let Operation::Capture(frame) = &mut task.operation else {
                unreachable!()
            };
            if let Some(error) = frame.error {
                self.shared.busy.store(0, Ordering::Release);
                return Err(Error::Invalid(error.into()));
            }
            if frame.complete {
                self.shared.busy.store(0, Ordering::Release);
                let Operation::Capture(mut frame) = task.operation else {
                    unreachable!()
                };
                frame.state.deduplicate(&mut frame.media);
                frame.state.validate(&frame.media).map_err(Error::Invalid)?;
                return Ok(Captured {
                    checkpoint: frame.checkpoint,
                    state: frame.state,
                    media: frame.media,
                    revision: frame.revision,
                    playback_receipts: frame.playback_receipts,
                });
            }
            if cancel.load(Ordering::Acquire) {
                self.shared.busy.store(0, Ordering::Release);
                return Err(Error::Cancelled);
            }
            frame.prepare();
            task.stage.store(PENDING, Ordering::Release);
        }
    }

    /// Worker only. Some(revision) requires saved/clean state with no held
    /// recording. None is an explicit user decision to discard unsaved work.
    pub fn seal_for_close(
        &self,
        expected_revision: Option<u64>,
        cancel: &AtomicBool,
    ) -> Result<CloseGuard, Error> {
        self.begin()?;
        if cancel.load(Ordering::Acquire) {
            self.shared.busy.store(0, Ordering::Release);
            return Err(Error::Cancelled);
        }
        let task = Box::new(Task {
            operation: Operation::Seal {
                expected: expected_revision,
                guard: None,
            },
            stage: Arc::new(AtomicU8::new(PENDING)),
            error: None,
        });
        let mut task = self.exchange(task, cancel)?;
        self.shared.busy.store(0, Ordering::Release);
        if let Some(error) = task.error.take() {
            return Err(error);
        }
        match &mut task.operation {
            Operation::Seal { guard, .. } => guard.take().ok_or(Error::Unavailable),
            _ => unreachable!(),
        }
    }

    /// Worker only. Preparing DSP has already finished; this commits or returns
    /// Conflict/Cancelled without changing the current graph. Committed wins a
    /// cancellation that arrives after the callback's atomic application claim.
    pub fn install(
        &self,
        prepared: Prepared,
        expected_revision: u64,
        cancel: &AtomicBool,
    ) -> Result<Applied, Error> {
        self.begin()?;
        if cancel.load(Ordering::Acquire) {
            self.shared.busy.store(0, Ordering::Release);
            return Err(Error::Cancelled);
        }
        let task = Box::new(Task {
            operation: Operation::Install {
                prepared,
                expected: expected_revision,
                applied: None,
            },
            stage: Arc::new(AtomicU8::new(PENDING)),
            error: None,
        });
        let mut task = self.exchange(task, cancel)?;
        self.shared.busy.store(0, Ordering::Release);
        if let Some(error) = task.error.take() {
            return Err(error);
        }
        match &mut task.operation {
            Operation::Install { applied, .. } => applied.take().ok_or(Error::Unavailable),
            _ => unreachable!(),
        }
        // task now owns the replaced graph; its destructor runs on this worker.
    }
}

impl RtEngine {
    pub(super) fn project_tick(&mut self) {
        if let Some(task) = self.project_pending.take() {
            if let Err(error) = self.project.shared.completed.try_send(task) {
                self.project_pending = Some(error.into_inner());
            }
            return;
        }
        if self.project_sealed {
            if self
                .project
                .shared
                .release_seal
                .swap(false, Ordering::AcqRel)
            {
                self.cmd_rx.end_project_install();
                self.project_sealed = false;
            } else {
                return;
            }
        }
        let Some(mut task) = self
            .project_waiting
            .take()
            .or_else(|| self.project.shared.incoming.try_recv().ok())
        else {
            return;
        };
        let exclusive = matches!(
            task.operation,
            Operation::Install { .. } | Operation::Seal { .. }
        );
        if task.stage.load(Ordering::Acquire) == PENDING {
            // The atomic admission gate closes before inspecting the queue.
            // A producer already holding a lease may finish; audio never waits,
            // and processes those commands on following blocks before commit.
            let history_ready =
                !matches!(task.operation, Operation::Install { .. }) || self.undo.available();
            let drained = history_ready
                && (!exclusive || self.cmd_rx.begin_project_install())
                && self.cmd_rx.is_empty();
            if !drained {
                self.project_waiting = Some(task);
                return;
            }
        }
        if task
            .stage
            .compare_exchange(PENDING, APPLYING, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            task.error = Some(Error::Cancelled);
        } else {
            match &mut task.operation {
                Operation::Capture(frame) => frame.capture(self),
                Operation::Seal { expected, guard } => {
                    if expected.is_some_and(|revision| {
                        revision != self.project.revision()
                            || self.has_held_project_notes()
                            || self.cmd_rx.pending_project_ui_requests()
                    }) {
                        task.error = Some(Error::Conflict);
                    } else {
                        self.project
                            .shared
                            .release_seal
                            .store(false, Ordering::Release);
                        self.project_sealed = true;
                        *guard = Some(CloseGuard {
                            handle: self.project.clone(),
                        });
                    }
                }
                Operation::Install {
                    prepared,
                    expected,
                    applied,
                } => {
                    if self.project.revision() != *expected
                        || self.cmd_rx.pending_project_ui_requests()
                    {
                        task.error = Some(Error::Conflict);
                    } else {
                        if !self.undo.clear() {
                            task.error = Some(Error::Busy);
                        } else {
                            prepared.swap_into(self);
                            self.project.edited();
                            self.publish();
                            *applied = Some(Applied {
                                checkpoint: self.undo.checkpoint(),
                                revision: self.project.revision(),
                                playback_receipts: std::array::from_fn(|i| {
                                    self.decks[i].load_receipt.clone()
                                }),
                            });
                        }
                    }
                }
            }
        }
        if exclusive && !self.project_sealed {
            self.cmd_rx.end_project_install();
        }
        task.stage.store(READY, Ordering::Release);
        if let Err(error) = self.project.shared.completed.try_send(task) {
            self.project_pending = Some(error.into_inner());
        }
    }
}

impl RtEngine {
    pub(super) fn resume_project_clips(&mut self) {
        for track in &mut self.tracks {
            if let Some(launch) = track.project_resume.take() {
                track.playing = Some(launch);
            }
        }
    }

    /// Revision is an edit journal counter, not a transport clock. Conservative
    /// repeated assignments may retain a dirty mark; physical gates/clock and
    /// ordinary transport never increment it unless they write recorded notes.
    pub(super) fn project_command_edits(&self, command: &Command) -> bool {
        match command {
            Command::Select { .. }
            | Command::SelectDeck(_)
            | Command::SelectDeckRequested { .. }
            | Command::SetView(_)
            | Command::OpenFxTrack(_)
            | Command::OpenFxScene(_)
            | Command::CloseFx
            | Command::SetBpm(_)
            | Command::NudgeBpm(_)
            | Command::Tap(_)
            | Command::DeckSync { .. }
            | Command::DeckPitch { .. }
            | Command::DeckGain { .. }
            | Command::DeckEq { .. }
            | Command::DeckFilter { .. }
            | Command::DeckPfl { .. }
            | Command::DeckLoop { .. }
            | Command::DeckLoopIn { .. }
            | Command::DeckLoopOut { .. }
            | Command::DeckVinyl { .. }
            | Command::DeckKeylock { .. }
            | Command::DeckAudio { .. }
            | Command::DeckUnload { .. }
            | Command::DeckSeek { .. }
            | Command::Xfader(_)
            | Command::Master(_)
            | Command::CueMix(_)
            | Command::TrackGain { .. }
            | Command::ClipGain { .. }
            | Command::TrackPan { .. }
            | Command::Mute { .. }
            | Command::Solo { .. }
            | Command::Arm { .. }
            | Command::ComposeArm { .. }
            | Command::SetNotes { .. }
            | Command::FxWet { .. }
            | Command::FxSelect { .. }
            | Command::Quant(_)
            | Command::Metronome
            | Command::ToggleQuant
            | Command::DeckLoopDouble { .. }
            | Command::DeckLoopHalf { .. }
            | Command::DeckReloop { .. }
            | Command::DeckMatch
            | Command::DeckEqCut { .. }
            | Command::DeckEqSolo { .. }
            | Command::DeckPitchRange { .. }
            | Command::SamplerBank(_)
            | Command::SamplerInst(_)
            | Command::SamplerOct(_)
            | Command::FxAdd(_)
            | Command::FxToggle(_)
            | Command::FxMix { .. }
            | Command::FxParam { .. } => true,
            Command::DeckCue { deck } => !self.decks[*deck as usize % DECKS].playing,
            Command::DeckHotCue { deck, pad, del } => {
                *del || !self.decks[*deck as usize % DECKS].hotcues[*pad as usize % HOTCUES].set
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests;
