//! Small application receipts for UI media loads. Queue acceptance is distinct
//! from renderer application, and replacing/unloading media retires its receipt.
use super::{dsp::Sample, media_load::LoadToken};
use std::sync::{
    atomic::{AtomicU8, Ordering},
    Arc,
};

#[derive(Clone, Debug)]
pub struct Receipt(Arc<AtomicU8>);

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
        Self(Arc::new(AtomicU8::new(State::Pending as u8)))
    }
    pub fn state(&self) -> State {
        match self.0.load(Ordering::Acquire) {
            0 => State::Pending,
            1 => State::Current,
            2 => State::Unavailable,
            4 => State::Applying,
            _ => State::Superseded,
        }
    }
    pub(super) fn claim(&self) -> bool {
        self.0
            .compare_exchange(
                State::Pending as u8,
                State::Applying as u8,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_ok()
    }
    pub fn cancel_pending(&self) {
        let _ = self.0.compare_exchange(
            State::Pending as u8,
            State::Superseded as u8,
            Ordering::AcqRel,
            Ordering::Acquire,
        );
    }
    pub fn supersede(&self) {
        self.0.store(State::Superseded as u8, Ordering::Release);
    }
    pub(super) fn finish(&self, state: State) {
        // Only the renderer that claimed this request can finish it.
        let _ = self.0.compare_exchange(
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
