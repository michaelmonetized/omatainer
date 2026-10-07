//! Eight media-relative loop slots with stable identities and independent display order.
use super::cue_metadata::{Style, STYLE_WORDS};
use serde::{Deserialize, Serialize};

pub(crate) const SLOTS: usize = 8;
const SLOT_WORDS: usize = 3 + STYLE_WORDS;
pub(super) const WORDS: usize = 2 + SLOTS * SLOT_WORDS;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Slot {
    pub start: f64,
    pub length: f64,
    pub style: Style,
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Bank {
    pub slots: [Option<Slot>; SLOTS],
    pub order: [u8; SLOTS],
    pub selected: u8,
}
impl Default for Bank {
    fn default() -> Self {
        Self {
            slots: [None; SLOTS],
            order: [1, 2, 3, 4, 5, 6, 7, 8],
            selected: 1,
        }
    }
}
impl Bank {
    /// Validate every source region and stable slot reference.
    /// Takes this fixed bank; returns false for nonfinite bounds, invalid IDs or duplicate display entries.
    pub(crate) fn valid(self) -> bool {
        let mut mask = 0u16;
        (1..=8).contains(&self.selected)
            && self.order.into_iter().all(|id| {
                if !(1..=8).contains(&id) || mask & (1 << id) != 0 {
                    return false;
                }
                mask |= 1 << id;
                true
            })
            && self.slots.into_iter().flatten().all(|slot| {
                slot.start.is_finite()
                    && slot.length.is_finite()
                    && slot.start >= 0.0
                    && slot.length > 0.0
                    && slot.start + slot.length <= 1.0e10
            })
    }
    /// Omit unchanged defaults from metadata.
    /// Takes this bank; returns true only when no region, order or selected slot differs from its default.
    pub(crate) fn is_default(&self) -> bool {
        *self == Self::default()
    }
    /// Encode a coherent fixed-size preparation receipt.
    /// Takes this bank; returns bounded words without callback allocation.
    pub(super) fn words(self) -> [u64; WORDS] {
        let mut words = [0; WORDS];
        words[0] = u64::from_le_bytes(self.order);
        words[1] = u64::from(self.selected);
        for (dest, slot) in words[2..].chunks_exact_mut(SLOT_WORDS).zip(self.slots) {
            if let Some(slot) = slot {
                dest[0] = 1;
                dest[1] = slot.start.to_bits();
                dest[2] = slot.length.to_bits();
                dest[3..].copy_from_slice(&slot.style.words());
            }
        }
        words
    }
    /// Decode one complete atomic receipt.
    /// Takes exact words; returns a fully validated bank or refuses the whole bank without partial recovery.
    pub(super) fn from_words(words: [u64; WORDS]) -> Option<Self> {
        let mut bank = Self {
            order: words[0].to_le_bytes(),
            selected: u8::try_from(words[1]).ok()?,
            ..Default::default()
        };
        for (dest, slot) in bank
            .slots
            .iter_mut()
            .zip(words[2..].chunks_exact(SLOT_WORDS))
        {
            *dest = match slot[0] {
                0 if slot.iter().all(|word| *word == 0) => None,
                1 => Some(Slot {
                    start: f64::from_bits(slot[1]),
                    length: f64::from_bits(slot[2]),
                    style: Style::from_words(slot[3..].try_into().ok()?)?,
                }),
                _ => return None,
            };
        }
        bank.valid().then_some(bank)
    }
}

#[cfg(test)]
mod tests;
