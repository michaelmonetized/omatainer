//! Explicit deck locks and reviewed media generations share the producer/renderer guard.
use super::{Command, Error, Handle, Ordering, RECOVERY, SAFETY_GENERATION};
use std::sync::Arc;

#[derive(Clone)]
pub(crate) struct Approval {
    handle: Handle,
    deck: usize,
    word: u64,
    safety: u64,
    generation: u64,
}
impl std::fmt::Debug for Approval {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeckApproval").field("deck", &self.deck).field("word", &self.word).finish()
    }
}
impl Approval {
    /// Confirm one reviewed deck source.
    /// Takes the owning guard and actual target; returns whether the reviewed media and safety generations remain current.
    pub(super) fn valid(&self, handle: &Handle, deck: usize) -> bool {
        Arc::ptr_eq(&self.handle.0, &handle.0)
            && self.deck == deck
            && handle.deck_load_word(deck) == self.word
            && self.safety_current(handle)
    }
    /// Claim one reviewed media boundary.
    /// Takes its owner and actual deck; atomically consumes the generation before media mutation and refuses concurrent lock/safety changes.
    pub(crate) fn claim(&self, handle: &Handle, deck: usize) -> bool {
        if !self.valid(handle, deck) { return false; }
        let Some(next) = self.word.checked_add(2) else { return false };
        handle.0.deck_load[deck].compare_exchange(self.word, next, Ordering::AcqRel, Ordering::Acquire).is_ok()
            && self.safety_current(handle)
    }
    /// Recheck the retained safety state.
    /// Takes the owner; returns whether protection generation and recovery still match the review.
    fn safety_current(&self, handle: &Handle) -> bool {
        let state = handle.0.admission.load(Ordering::Acquire);
        state & RECOVERY == 0
            && handle.safety_epoch() == self.safety
            && state / SAFETY_GENERATION == self.generation
    }
}
impl Handle {
    /// Claim a guarded media mutation.
    /// Takes its deck, current renderer activity and optional review; returns whether one atomic media boundary may proceed without callback waits.
    pub(crate) fn claim_deck_media(&self, deck: usize, activity: u8, approval: Option<&Approval>) -> bool {
        if let Some(approval) = approval { return approval.claim(self, deck); }
        let word = self.deck_load_word(deck);
        let Some(next) = word.checked_add(2) else { return false };
        let command = Command::DeckUnload { deck: deck as u8 };
        self.check(&command, Some(activity)).is_ok()
            && self.0.deck_load[deck].compare_exchange(word, next, Ordering::AcqRel, Ordering::Acquire).is_ok()
            && self.check(&command, Some(activity)).is_ok()
    }
    /// Read a deck's media and lock generation.
    /// Takes its index; returns its atomic generation word, or an invalid sentinel for an absent deck.
    pub(crate) fn deck_load_word(&self, deck: usize) -> u64 {
        self.0.deck_load.get(deck).map_or(u64::MAX, |word| word.load(Ordering::Acquire))
    }
    /// Set explicit protection for a deck.
    /// Takes its index and lock choice; atomically invalidates older reviews when the choice changes.
    pub(crate) fn set_deck_load_lock(&self, deck: usize, locked: bool) -> Result<(), Error> {
        let word = self.0.deck_load.get(deck).ok_or(Error::Changing)?;
        let mut old = word.load(Ordering::Acquire);
        loop {
            if old & 1 == u64::from(locked) { return Ok(()); }
            let next = old.checked_add(2).ok_or(Error::Changing)? & !1 | u64::from(locked);
            match word.compare_exchange_weak(old, next, Ordering::AcqRel, Ordering::Acquire) {
                Ok(_) => return Ok(()), Err(value) => old = value,
            }
        }
    }
    /// Approve the exact deck displayed in a deliberate review.
    /// Takes its index and snapshot generation; returns a source-bound approval or refuses changed state.
    pub(crate) fn approve_deck_load(&self, deck: usize, reviewed: u64) -> Result<Approval, Error> {
        if deck >= super::super::DECKS || self.deck_load_word(deck) != reviewed {
            return Err(Error::Changing);
        }
        let status = self.status();
        if status.recovery { return Err(Error::Recovery); }
        if status.changing { return Err(Error::Changing); }
        Ok(Approval { handle: self.clone(), deck, word: reviewed, safety: self.safety_epoch(),
            generation: self.0.admission.load(Ordering::Acquire) / SAFETY_GENERATION })
    }
    /// Retire approvals after a media boundary.
    /// Takes its deck index; advances the retained generation without changing the lock choice.
    pub(crate) fn deck_media_changed(&self, deck: usize) {
        self.0.deck_load[deck].fetch_add(2, Ordering::AcqRel);
    }
}
/// Inspect a source-bound command approval.
/// Takes an ordinary or scoped command; returns only the explicit receipt's retained review.
pub(super) fn approval(command: &Command) -> Option<&Approval> {
    match command {
        Command::DeckLoadRequested { receipt, .. } => receipt.deck_approval(),
        Command::SessionControl(scoped) => approval(&scoped.command),
        Command::Gesture { command, .. } => approval(command),
        _ => None,
    }
}
