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
    deck_approval: Option<super::performance::DeckApproval>,
    deck_generation: Option<u64>,
    history_key: u64,
    state: AtomicU8,
    history_pins: AtomicU64,
    grid_lock_sequence: AtomicU64,
    grid_lock: [AtomicU64; 4],
    initial_preparation: Option<super::preparation::Preparation>,
    preparation_sequence: AtomicU64,
    preparation: [AtomicU64; super::preparation::WORDS],
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
    Protected = 5,
}

impl Receipt {
    pub fn new() -> Self {
        Self::with_preparation(None)
    }
    pub(crate) fn with_preparation(
        initial_preparation: Option<super::preparation::Preparation>,
    ) -> Self {
        Self::with_preparation_and_approval(initial_preparation, None, None)
    }
    /// Create one pending media receipt.
    /// Takes retained preparation and an optional reviewed override; returns a fresh receipt with no application acknowledgment.
    fn with_preparation_and_approval(
        initial_preparation: Option<super::preparation::Preparation>,
        deck_approval: Option<super::performance::DeckApproval>,
        deck_generation: Option<u64>,
    ) -> Self {
        let wall_origin = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
            .min(u64::MAX as u128) as u64;
        Self(Arc::new(Inner {
            deck_approval,
            deck_generation,
            history_key: super::history_measurement::parts::fresh_key(),
            state: AtomicU8::new(State::Pending as u8),
            history_pins: AtomicU64::new(0),
            grid_lock_sequence: AtomicU64::new(0),
            grid_lock: std::array::from_fn(|_| AtomicU64::new(0)),
            initial_preparation,
            preparation_sequence: AtomicU64::new(0),
            preparation: std::array::from_fn(|_| AtomicU64::new(0)),
            last_play: AtomicU64::new(0),
            wall_origin,
            clock_origin: std::time::Instant::now(),
        }))
    }
    /// Publish reviewed grid protection without callback locks.
    /// Takes locked state and its canonical saved grid; atomically replaces one bounded receipt target.
    pub(crate) fn set_grid_protection(&self, locked: bool, grid: Option<super::beatgrid::Grid>) {
        if self.grid_protection() == Some((locked, if locked { grid } else { None })) {
            return;
        }
        let words = super::beatgrid::Grid::encode(if locked { grid } else { None });
        self.0.grid_lock_sequence.fetch_add(1, Ordering::AcqRel);
        self.0.grid_lock[0].store(locked as u64, Ordering::Relaxed);
        for (slot, word) in self.0.grid_lock[1..].iter().zip(words) {
            slot.store(word, Ordering::Relaxed);
        }
        self.0.grid_lock_sequence.fetch_add(1, Ordering::Release);
    }
    /// Read coherent grid protection without waiting.
    /// Takes this receipt; returns its bounded lock/grid snapshot or none during concurrent publication.
    pub(super) fn grid_protection(&self) -> Option<(bool, Option<super::beatgrid::Grid>)> {
        let sequence = self.0.grid_lock_sequence.load(Ordering::Acquire);
        if sequence & 1 != 0 {
            return None;
        }
        let words: [u64; 4] = std::array::from_fn(|i| self.0.grid_lock[i].load(Ordering::Relaxed));
        std::sync::atomic::fence(Ordering::Acquire);
        if sequence != self.0.grid_lock_sequence.load(Ordering::Relaxed) {
            return None;
        }
        let grid = super::beatgrid::Grid::decode([words[1], words[2], words[3]])?;
        Some((words[0] != 0, grid))
    }
    /// Refuse grid edits while locked or while protection changes.
    /// Takes this receipt; returns whether a manual edit must wait for an explicit unlock.
    pub(crate) fn grid_is_locked(&self) -> bool {
        self.grid_protection().is_none_or(|(locked, _)| locked)
    }
    /// Bind a fresh load receipt to a deliberate deck review.
    /// Takes this unused receipt and approval; returns a new pending receipt retaining its preparation.
    pub(crate) fn with_deck_approval(self, approval: super::performance::DeckApproval) -> Self {
        let next = Self::with_preparation_and_approval(
            self.initial_preparation(),
            Some(approval),
            self.0.deck_generation,
        );
        if let Some((locked, grid)) = self.grid_protection() {
            next.set_grid_protection(locked, grid);
        }
        next
    }
    /// Bind prepared media to the deck it was requested for.
    /// Takes its captured media generation; returns a pending receipt which cannot replace a subsequently unloaded or changed deck.
    pub(crate) fn with_deck_generation(self, generation: u64) -> Self {
        let next = Self::with_preparation_and_approval(
            self.initial_preparation(),
            self.0.deck_approval.clone(),
            Some(generation),
        );
        if let Some((locked, grid)) = self.grid_protection() {
            next.set_grid_protection(locked, grid);
        }
        next
    }
    /// Read the media generation captured before worker preparation.
    /// Takes this receipt; returns its optional immutable generation.
    pub(crate) fn deck_generation(&self) -> Option<u64> {
        self.0.deck_generation
    }
    /// Inspect a reviewed deck override.
    /// Takes this receipt; returns its immutable approval when one was explicitly supplied.
    pub(crate) fn deck_approval(&self) -> Option<&super::performance::DeckApproval> {
        self.0.deck_approval.as_ref()
    }
    pub(super) fn initial_preparation(&self) -> Option<super::preparation::Preparation> {
        self.0.initial_preparation
    }
    /// Single renderer writer; a GUI poll makes one bounded attempt, never spins.
    pub(super) fn record_preparation(&self, preparation: super::preparation::Preparation) {
        let words = preparation.words();
        self.0.preparation_sequence.fetch_add(1, Ordering::AcqRel);
        for (slot, word) in self.0.preparation.iter().zip(words) {
            slot.store(word, Ordering::Relaxed);
        }
        self.0.preparation_sequence.fetch_add(1, Ordering::Release);
    }
    pub(crate) fn preparation(&self) -> Option<(u64, super::preparation::Preparation)> {
        let sequence = self.0.preparation_sequence.load(Ordering::Acquire);
        if sequence == 0 || sequence & 1 != 0 {
            return None;
        }
        let words = std::array::from_fn(|i| self.0.preparation[i].load(Ordering::Relaxed));
        std::sync::atomic::fence(Ordering::Acquire);
        (sequence == self.0.preparation_sequence.load(Ordering::Relaxed))
            .then(|| (sequence, super::preparation::Preparation::from_words(words)))
    }
    /// Ephemeral in-process snapshot key only. A GUI watch retains its receipt,
    /// so a different request cannot reuse this address while the key is used.
    /// Never serialized, persisted, or used as a content identity.
    pub(crate) fn snapshot_key(&self) -> usize {
        Arc::as_ptr(&self.0) as usize
    }
    pub(crate) fn history_key(&self) -> u64 {
        self.0.history_key
    }
    pub fn same_request(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
    pub fn state(&self) -> State {
        match self.0.state.load(Ordering::Acquire) {
            0 => State::Pending,
            1 => State::Current,
            2 => State::Unavailable,
            4 => State::Applying,
            5 => State::Protected,
            _ => State::Superseded,
        }
    }
    pub(crate) fn retained_by_history(&self) -> bool {
        self.0.history_pins.load(Ordering::Acquire) != 0
    }
    pub(super) fn pin_history(&self) {
        self.0.history_pins.fetch_add(1, Ordering::Release);
    }
    pub(super) fn unpin_history(&self) {
        self.0.history_pins.fetch_sub(1, Ordering::Release);
    }
    /// Only the renderer can restore an identity already owned by an inverse
    /// media patch. Pending/rejected decode requests cannot use this transition.
    pub(super) fn restore_from_history(&self) {
        let _ = self.0.state.compare_exchange(
            State::Superseded as u8,
            State::Current as u8,
            Ordering::AcqRel,
            Ordering::Acquire,
        );
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
    Unload,
    Builtin(u8),
    Decoded {
        token: LoadToken,
        audio: Arc<Sample>,
    },
}
