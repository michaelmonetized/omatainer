//! Two native atomic words cover all 128 track destinations. Routing publication
//! brackets their update with its generation; the callback never locks or spins.
use std::sync::atomic::{AtomicU64, Ordering};

pub(super) struct AtomicMask([AtomicU64; 2]);
impl AtomicMask {
    pub fn new(value: u128) -> Self {
        Self([
            AtomicU64::new(value as u64),
            AtomicU64::new((value >> 64) as u64),
        ])
    }
    pub fn load(&self, ordering: Ordering) -> u128 {
        u128::from(self.0[0].load(ordering)) | (u128::from(self.0[1].load(ordering)) << 64)
    }
    pub fn store(&self, value: u128, ordering: Ordering) {
        self.0[0].store(value as u64, ordering);
        self.0[1].store((value >> 64) as u64, ordering);
    }
    pub fn fetch_and(&self, value: u128, ordering: Ordering) {
        self.0[0].fetch_and(value as u64, ordering);
        self.0[1].fetch_and((value >> 64) as u64, ordering);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn all_track_bits_survive_native_word_boundaries() {
        let mask = AtomicMask::new(0);
        for track in 0..128 {
            mask.store(1u128 << track, Ordering::Release);
            assert_eq!(mask.load(Ordering::Acquire), 1u128 << track);
        }
        mask.store(u128::MAX, Ordering::Release);
        mask.fetch_and(!(1u128 << 64), Ordering::AcqRel);
        assert_eq!(mask.load(Ordering::Acquire), u128::MAX ^ (1u128 << 64));
        mask.fetch_and(!(1u128 << 127), Ordering::AcqRel);
        assert_eq!(
            mask.load(Ordering::Acquire),
            u128::MAX ^ (1u128 << 64) ^ (1u128 << 127)
        );
    }
}
