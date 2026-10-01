//! Small application receipts for UI media loads. Queue acceptance is distinct
//! from renderer application, and replacing/unloading media retires its receipt.
use super::{dsp::Sample, media_load::LoadToken};
use std::sync::{
    atomic::{AtomicU64, AtomicU8, Ordering},
    Arc,
};

#[derive(Clone, Debug)]
pub struct Receipt(Arc<Inner>);

#[derive(Debug)]
struct Inner {
    state: AtomicU8,
    last_play: AtomicU64,
    // Capture civil time before callback ownership; only monotonic elapsed
    // time is queried at an actual playback onset, never for every sample.
    wall_origin: u64,
    clock_origin: std::time::Instant,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum State {
    Pending = 0,
    Current = 1,
    Unavailable = 2,
    Superseded = 3,
    Applying = 4,
}

impl Receipt {
    pub fn new() -> Self {
        let wall_origin = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
            .min(u64::MAX as u128) as u64;
        Self(Arc::new(Inner {
            state: AtomicU8::new(State::Pending as u8),
            last_play: AtomicU64::new(0),
            wall_origin,
            clock_origin: std::time::Instant::now(),
        }))
    }
    pub fn state(&self) -> State {
        match self.0.state.load(Ordering::Acquire) {
            0 => State::Pending,
            1 => State::Current,
            2 => State::Unavailable,
            4 => State::Applying,
            _ => State::Superseded,
        }
    }
    pub fn last_play(&self) -> Option<std::time::SystemTime> {
        let nanos = self.0.last_play.load(Ordering::Acquire);
        (nanos != 0).then(|| std::time::UNIX_EPOCH + std::time::Duration::from_nanos(nanos))
    }
    pub(super) fn record_playback(&self) {
        if self.state() != State::Current {
            return;
        }
        let elapsed = self
            .0
            .clock_origin
            .elapsed()
            .as_nanos()
            .min(u64::MAX as u128) as u64;
        self.0.last_play.store(
            self.0.wall_origin.saturating_add(elapsed).max(1),
            Ordering::Release,
        );
    }
    pub(super) fn claim(&self) -> bool {
        self.0
            .state
            .compare_exchange(
                State::Pending as u8,
                State::Applying as u8,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_ok()
    }
    pub fn cancel_pending(&self) {
        let _ = self.0.state.compare_exchange(
            State::Pending as u8,
            State::Superseded as u8,
            Ordering::AcqRel,
            Ordering::Acquire,
        );
    }
    pub fn supersede(&self) {
        self.0
            .state
            .store(State::Superseded as u8, Ordering::Release);
    }
    pub(super) fn finish(&self, state: State) {
        // Only the renderer that claimed this request can finish it.
        let _ = self.0.state.compare_exchange(
            State::Applying as u8,
            state as u8,
            Ordering::AcqRel,
            Ordering::Acquire,
        );
    }
}

#[derive(Clone, Debug)]
pub enum Media {
    Builtin(u8),
    Decoded {
        token: LoadToken,
        audio: Arc<Sample>,
    },
}
