//! Bounded typed requests for work owned by the GUI.
//!
//! The GUI publishes its selection; the MIDI dispatch worker captures that
//! exact Arc at admission. Browsing or a scan cannot change an admitted source.
//! No filesystem, decoder, or renderer work is performed by this handoff.
use super::media_source::Selection;
use super::{SubmissionError, SubmissionOutcome, DECKS};
use arc_swap::ArcSwapOption;
use crossbeam_channel::{bounded, Receiver as QueueReceiver, Sender, TrySendError};
use parking_lot::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

pub const CAPACITY: usize = 16;
pub const PER_FRAME: usize = 8;

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Stats {
    pub pending: usize,
    pub accepted: u64,
    pub dispatched: u64,
    pub rejected: u64,
}

pub struct LoadRequest {
    pub deck: u8,
    pub selection: Option<Arc<Selection>>,
}

#[derive(Default)]
struct Shared {
    attached: AtomicBool,
    selection: ArcSwapOption<Selection>,
    accepted: AtomicU64,
    dispatched: AtomicU64,
    rejected: AtomicU64,
}

pub(super) struct Mailbox {
    shared: Arc<Shared>,
    sender: Sender<LoadRequest>,
    receiver: Mutex<Option<QueueReceiver<LoadRequest>>>,
}

impl Default for Mailbox {
    fn default() -> Self {
        let (sender, receiver) = bounded(CAPACITY);
        Self {
            shared: Arc::new(Shared::default()),
            sender,
            receiver: Mutex::new(Some(receiver)),
        }
    }
}

impl Mailbox {
    pub fn receiver(&self) -> Option<Receiver> {
        // Called once during Engine construction, before MIDI connections open.
        let receiver = self.receiver.lock().take()?;
        self.shared.attached.store(true, Ordering::Release);
        Some(Receiver {
            shared: self.shared.clone(),
            receiver,
        })
    }

    pub fn load(&self, deck: u8) -> Result<SubmissionOutcome, SubmissionError> {
        let result = if deck as usize >= DECKS {
            Err(SubmissionError::InvalidTarget)
        } else if !self.shared.attached.load(Ordering::Acquire) {
            Err(SubmissionError::UiUnavailable)
        } else {
            // CommandPort is called by the MIDI dispatch worker (issue 39),
            // never by the raw MIDI callback. Rejected Arc destruction stays
            // off that callback and off the audio renderer.
            let selection = self.shared.selection.load_full();
            self.sender
                .try_send(LoadRequest { deck, selection })
                .map_err(|error| match error {
                    TrySendError::Full(_) => SubmissionError::UiFull,
                    TrySendError::Disconnected(_) => SubmissionError::UiUnavailable,
                })
        };
        match result {
            Ok(()) => {
                self.shared.accepted.fetch_add(1, Ordering::Relaxed);
                Ok(SubmissionOutcome::Accepted)
            }
            Err(error) => {
                self.shared.rejected.fetch_add(1, Ordering::Relaxed);
                Err(error)
            }
        }
    }

    pub fn reject_uncaptured(&self) {
        self.shared.rejected.fetch_add(1, Ordering::Relaxed);
    }

    pub fn stats(&self) -> Stats {
        Stats {
            pending: self.sender.len(),
            accepted: self.shared.accepted.load(Ordering::Relaxed),
            dispatched: self.shared.dispatched.load(Ordering::Relaxed),
            rejected: self.shared.rejected.load(Ordering::Relaxed),
        }
    }
}

pub struct Receiver {
    shared: Arc<Shared>,
    receiver: QueueReceiver<LoadRequest>,
}

impl Receiver {
    pub fn publish_selection(&self, selection: Option<Arc<Selection>>) {
        self.shared.selection.store(selection);
    }

    pub fn take_loads(&self) -> [Option<LoadRequest>; PER_FRAME] {
        std::array::from_fn(|_| {
            self.receiver.try_recv().ok().inspect(|_| {
                self.shared.dispatched.fetch_add(1, Ordering::Relaxed);
            })
        })
    }
}

impl Drop for Receiver {
    fn drop(&mut self) {
        self.shared.attached.store(false, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_renderer_request_rejects_without_allocating_or_touching_selection_arcs() {
        let (engine, mut renderer) = crate::engine::Engine::headless_for_test(48000, 32);
        engine
            .ui_requests
            .publish_selection(Some(Arc::new(Selection {
                source: crate::engine::media_source::LibSource::File("private.wav".into()),
                title: "private".into(),
            })));
        let counts = crate::engine::test_alloc::measure(|| {
            renderer.apply(crate::engine::Command::DeckLoadSelected { deck: 0 });
        });
        assert_eq!(counts.allocations, 0);
        assert_eq!(counts.frees, 0);
        assert_eq!(
            engine.cmd.stats().last_error,
            Some(SubmissionError::UncapturedSelection)
        );
        assert_eq!(engine.cmd.ui_request_stats().pending, 0);
    }
}
