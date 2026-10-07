use super::{DeckSnap, RtEngine, DECKS};
use crate::musical_key::Key;

pub(crate) const MAX_SEMITONES: i8 = 6;
const FACTORS: [f64; 13] = [
    0.7071067811865476,
    0.7491535384383408,
    0.7937005259840998,
    0.8408964152537145,
    0.8908987181403393,
    0.9438743126816935,
    1.0,
    1.0594630943592953,
    1.122462048309373,
    1.189207115002721,
    1.2599210498948732,
    1.3348398541700344,
    1.4142135623730951,
];

/// Resolve the supported equal-tempered pitch factor.
/// Takes a semitone offset; returns its fixed factor, or unity for invalid state.
pub(crate) fn factor(semitones: i8) -> f64 {
    usize::try_from(i16::from(semitones) + i16::from(MAX_SEMITONES))
        .ok()
        .and_then(|index| FACTORS.get(index))
        .copied()
        .unwrap_or(1.0)
}

/// Find the closest exact transposition into a chosen key.
/// Takes valid source and target keys; returns an offset within six semitones or a visible refusal for unknown keys or a different mode.
pub(crate) fn matching(source: Key, target: Key) -> Result<i8, &'static str> {
    if !source.valid() || !target.valid() {
        return Err("Choose a known source and target key");
    }
    if source.minor != target.minor {
        return Err("Transposing preserves major or minor; choose a target in the same mode");
    }
    let distance = (i16::from(target.tonic) - i16::from(source.tonic)).rem_euclid(12);
    Ok((if distance > 6 {
        distance - 12
    } else {
        distance
    }) as i8)
}

/// Name a transposed source while preserving its mode.
/// Takes its key and supported offset; returns the resulting pitch class or an invalid-state refusal.
pub(crate) fn shifted(source: Key, semitones: i8) -> Option<Key> {
    (source.valid() && (-MAX_SEMITONES..=MAX_SEMITONES).contains(&semitones)).then(|| Key {
        tonic: (i16::from(source.tonic) + i16::from(semitones)).rem_euclid(12) as u8,
        minor: source.minor,
    })
}

/// Report the declared forward tempo interval for a requested shift.
/// Takes requested lock and semitones; returns the supported source-seconds-per-output-second bounds.
pub(crate) fn tempo_range(locked: bool, semitones: i8) -> (f32, f32) {
    let factor = factor(semitones) as f32;
    if locked {
        (
            (super::keylock::MIN_RATIO * factor).max(0.5),
            (super::keylock::MAX_RATIO * factor).min(1.5),
        )
    } else {
        (0.5, 1.5)
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Request {
    pub(super) deck: u8,
    pub(super) media: u64,
    pub(super) semitones: i8,
    pub(super) enable_lock: bool,
}
impl Request {
    /// Capture an independent shift for the currently applied source.
    /// Takes deck, renderer snapshot, offset and explicit match-lock intent; returns a fixed request or a visible source/range refusal.
    pub(crate) fn new(
        deck: u8,
        source: &DeckSnap,
        semitones: i8,
        enable_lock: bool,
    ) -> Result<Self, &'static str> {
        let request = Self {
            deck,
            media: source.media_key,
            semitones,
            enable_lock,
        };
        if !request.valid() || source.frames <= 0.0 {
            return Err("Load a track and choose a shift within six semitones");
        }
        Ok(request)
    }
    /// Validate producer admission without touching live media.
    /// Takes this request; returns whether its deck, source identity and offset are bounded.
    pub(super) fn valid(self) -> bool {
        usize::from(self.deck) < DECKS
            && self.media != 0
            && (-MAX_SEMITONES..=MAX_SEMITONES).contains(&self.semitones)
    }
    /// Refuse an edit after a source has changed.
    /// Takes this request and renderer; returns whether the captured source is still applied.
    pub(super) fn current(self, rt: &RtEngine) -> bool {
        self.valid()
            && rt
                .decks
                .get(usize::from(self.deck))
                .is_some_and(|deck| deck.audio.is_some() && deck.history_key == self.media)
    }
}

#[cfg(test)]
mod tests;

/// Omit original-key state from older compatible project data.
/// Takes its offset; returns whether it is exactly zero.
pub(crate) fn is_zero(offset: &i8) -> bool {
    *offset == 0
}
