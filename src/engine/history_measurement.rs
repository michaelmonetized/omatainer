//! Digital main-output activity, not a claim about speakers or human hearing.
//!
//! A measurement identity is one load/play episode on one deck. Catalog
//! identities and labels live off callback; aggregating episode intervals does
//! not reclassify the combined waveform of several loads of the same track.
pub(super) mod parts;
pub(super) mod error;
pub(super) mod capture;
pub(crate) mod control;
pub(super) mod tracker;
mod tail;
mod window;
pub(super) use tail::SlotBounds;
pub(super) use window::{Frame, Windows};
pub(crate) use window::Observation;

pub(super) const LANES: usize = 4;
pub(super) const MIN_RATE: u32 = 8_000;
pub(super) const MAX_RATE: u32 = 384_000;
/// Both analog removal and final converted signal must pass this RMS floor.
pub(super) const ACTIVITY_FLOOR: f64 = 0.000_031_622_776_601_683_795; // -90 dBFS
/// The future-output certificate includes all subsequent master gain/FX.
pub(super) const RETIRE_FLOOR: f64 = 0.000_003_162_277_660_168_379; // -110 dBFS

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Episode {
    pub load: u64,
    pub generation: u64,
    pub deck: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Classification {
    Active,
    BelowFloor,
    /// A numerical/error boundary is reported, never silently made exact.
    Ambiguous,
    Nonfinite,
    /// The sample clock cannot represent the complete interval.
    ClockOverflow,
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod error_tests;

#[cfg(test)]
mod tracker_tests;

#[cfg(test)]
mod capture_tests;

#[cfg(test)]
mod performance_tests;

#[cfg(test)]
mod control_tests;
