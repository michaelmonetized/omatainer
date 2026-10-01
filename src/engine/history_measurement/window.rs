use super::{Classification, Episode, ACTIVITY_FLOOR, LANES, MAX_RATE, MIN_RATE};

/// Already mapped main channels (mono occupies only element zero). The digital
/// values are decoded from the actual destination sample buffer, not predicted
/// from deck play/level flags. `without_*` is the same-frame counterfactual.
#[derive(Clone, Copy)]
pub(in crate::engine) struct Frame {
    pub analog: [f64; 2],
    pub digital: [f64; 2],
    pub without_analog: [[f64; 2]; LANES],
    pub without_digital: [[f64; 2]; LANES],
    /// Conservative source decomposition/numerical error per analog channel.
    pub error: [[f64; 2]; LANES],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::engine) struct Observation {
    pub episode: Episode,
    pub first_frame: u64,
    pub frames: u32,
    pub sample_rate: u32,
    pub classification: Classification,
}

#[derive(Clone, Copy, Default)]
struct Energy {
    actual_analog: [f64; 2],
    actual_digital: [f64; 2],
    removal_low: [f64; 2],
    removal_high: [f64; 2],
    removal_digital: [f64; 2],
    invalid: bool,
}

impl Energy {
    fn add(&mut self, frame: &Frame, lane: usize) {
        for channel in 0..2 {
            let actual = frame.analog[channel];
            let digital = frame.digital[channel];
            let without = frame.without_analog[lane][channel];
            let without_digital = frame.without_digital[lane][channel];
            let error = frame.error[lane][channel];
            if ![actual, digital, without, without_digital, error]
                .into_iter().all(f64::is_finite) || error < 0.0
            {
                self.invalid = true;
                continue;
            }
            let removal = (actual - without).abs();
            self.actual_analog[channel] += actual * actual;
            self.actual_digital[channel] += digital * digital;
            self.removal_low[channel] += (removal - error).max(0.0).powi(2);
            self.removal_high[channel] += (removal + error).powi(2);
            self.removal_digital[channel] += (digital - without_digital).powi(2);
        }
    }

    fn classify(&self, frames: u32) -> Classification {
        if self.invalid || [self.actual_analog, self.actual_digital,
            self.removal_low, self.removal_high, self.removal_digital]
            .into_iter().flatten().any(|value| !value.is_finite())
        {
            return Classification::Nonfinite;
        }
        let minimum = ACTIVITY_FLOOR * ACTIVITY_FLOOR * frames as f64;
        let mut ambiguous = false;
        for channel in 0..2 {
            // Requiring analog activity excludes a quantization-boundary flip
            // caused by an arbitrarily small, certified-negligible tail.
            if self.actual_analog[channel] > minimum
                && self.actual_digital[channel] > minimum
                && self.removal_digital[channel] > minimum
            {
                if self.removal_low[channel] > minimum {
                    return Classification::Active;
                }
                ambiguous |= self.removal_high[channel] >= minimum;
            }
        }
        if ambiguous { Classification::Ambiguous } else { Classification::BelowFloor }
    }
}

/// Fixed storage and sample-clock windows. Call finish before changing rate,
/// episode ownership or session state; the final partial window is not padded.
pub(in crate::engine) struct Windows {
    rate: u32,
    width: u32,
    first: u64,
    frames: u32,
    episodes: [Option<Episode>; LANES],
    energy: [Energy; LANES],
    exhausted: bool,
}

impl Windows {
    pub fn new(rate: u32, first: u64, episodes: [Option<Episode>; LANES]) -> Option<Self> {
        if !(MIN_RATE..=MAX_RATE).contains(&rate) { return None; }
        Some(Self { rate, width: rate.div_ceil(100), first, frames: 0,
            episodes, energy: [Energy::default(); LANES], exhausted: false })
    }
    pub fn push(&mut self, frame: &Frame) -> Option<[Option<Observation>; LANES]> {
        for lane in 0..LANES {
            if self.episodes[lane].is_some() { self.energy[lane].add(frame, lane); }
        }
        self.frames += 1;
        (self.frames == self.width).then(|| self.finish()).flatten()
    }
    pub fn finish(&mut self) -> Option<[Option<Observation>; LANES]> {
        if self.frames == 0 { return None; }
        let result = std::array::from_fn(|lane| self.episodes[lane].map(|episode| Observation {
            episode, first_frame: self.first, frames: self.frames, sample_rate: self.rate,
            classification: if self.exhausted || self.first.checked_add(self.frames as u64).is_none() {
                Classification::ClockOverflow
            } else { self.energy[lane].classify(self.frames) },
        }));
        match self.first.checked_add(self.frames as u64) {
            Some(next) => self.first = next,
            None => self.exhausted = true,
        }
        self.frames = 0;
        self.energy = [Energy::default(); LANES];
        Some(result)
    }
}
