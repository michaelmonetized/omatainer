//! Four preallocated source/deck lanes, including retiring master-effect tails.
use super::{parts::Parts, tail, Episode, SlotBounds, LANES, MAX_RATE, MIN_RATE};
use crate::engine::{master_fx::MasterSlot, FxKind};

pub(in crate::engine) const MAX_STORAGE: usize = 96 * 1024 * 1024;

struct Lane {
    episode: Option<Episode>,
    slots: [MasterSlot; 3],
    bounds: [SlotBounds; 3],
    pristine: bool,
}
impl Lane {
    fn new(rate: u32) -> Result<Self, &'static str> {
        let slot = || MasterSlot::try_new(rate as f32).map_err(|_| "history effect storage unavailable");
        Ok(Self { episode: None, slots: [slot()?, slot()?, slot()?],
            bounds: [SlotBounds::default(); 3], pristine: true })
    }
    fn clear(&mut self) {
        for slot in &mut self.slots {
            for kind in [FxKind::Echo, FxKind::Reverb, FxKind::Filter] { slot.reset(kind); }
        }
        self.bounds = [SlotBounds::default(); 3];
        self.episode = None;
        self.pristine = true;
    }
}

#[derive(Clone, Copy, Debug)]
pub(in crate::engine) struct Contribution {
    pub episodes: [Option<Episode>; LANES],
    pub values: [[f32; 2]; LANES],
    pub error: [[f64; 2]; LANES],
    pub incomplete: bool,
}

pub(in crate::engine) struct Tracker {
    lanes: Vec<Lane>,
    kinds: [FxKind; 3],
    wet: [f32; 3],
    generation: u64,
    pub incomplete: bool,
}

impl Tracker {
    /// Called only while preparing an unowned renderer. Allocation failure
    /// disables measurement; it must never prevent audio from rendering.
    pub fn new(rate: u32) -> Result<Self, &'static str> {
        if !(MIN_RATE..=MAX_RATE).contains(&rate) { return Err("unsupported history sample rate"); }
        // Each comb rounding costs at most one extra sample in this bound.
        let per_channel = 2 * rate as usize
            + (14_460_u64 * u64::from(rate)).div_ceil(48_000) as usize + 8;
        let bound = per_channel.checked_mul(2 * 3 * LANES * 4)
            .and_then(|n| n.checked_add(std::mem::size_of::<Lane>() * LANES))
            .ok_or("history storage calculation overflow")?;
        if bound > MAX_STORAGE { return Err("history storage budget exceeded"); }
        let mut lanes = Vec::new();
        lanes.try_reserve_exact(LANES).map_err(|_| "history lane storage unavailable")?;
        for _ in 0..LANES { lanes.push(Lane::new(rate)?); }
        let value = Self { lanes, kinds: [FxKind::Echo, FxKind::Reverb, FxKind::Filter],
            wet: [0.0; 3], generation: 0, incomplete: false };
        if value.storage_bytes() > MAX_STORAGE { return Err("history allocated storage exceeds budget"); }
        Ok(value)
    }
    pub fn storage_bytes(&self) -> usize {
        self.lanes.capacity() * std::mem::size_of::<Lane>()
            + self.lanes.iter().flat_map(|l| &l.slots).map(MasterSlot::storage_bytes).sum::<usize>()
    }
    pub fn configure(&mut self, kinds: [FxKind; 3], wet: [f32; 3], samples_per_beat: f64) {
        for lane in &mut self.lanes {
            for i in 0..3 {
                if kinds[i] != self.kinds[i] {
                    lane.slots[i].reset(kinds[i]); lane.bounds[i].reset(kinds[i]);
                }
                lane.slots[i].configure(wet[i], samples_per_beat);
            }
        }
        self.kinds = kinds;
        self.wet = wet;
    }
    /// An actual master DSP reset or project replacement also clears observer
    /// histories. This does not reset the session clock or its saved evidence.
    pub fn reset(&mut self) { for lane in &mut self.lanes { lane.clear(); } }

    pub fn process(&mut self, parts: [Parts; 2], current: [u64; 2],
        actual_decks: [[f32; 2]; 2], gain: [f32; 2], pfl: [bool; 2], cue_mix: f32,
    ) -> Contribution {
        self.incomplete |= parts.iter().any(|p| p.incomplete);
        // Retire only with a future-output certificate. A quiet current sample
        // cannot retire a delay that may be exposed by a later wet/tempo edit.
        for lane in &mut self.lanes {
            if let Some(ep) = lane.episode {
                let deck = usize::from(ep.deck);
                let still_direct = current[deck] == ep.load
                    || parts[deck].values.iter().any(|p| p.key == ep.load);
                if !still_direct && tail::certified_negligible(&lane.bounds, self.kinds) { lane.clear(); }
            }
        }
        for (deck, part) in parts.iter().enumerate() {
            for p in part.values.into_iter().filter(|p| p.key != 0) {
                if self.lanes.iter().any(|lane| lane.episode.is_some_and(|ep| ep.load == p.key && usize::from(ep.deck) == deck)) { continue; }
                if let Some(lane) = self.lanes.iter_mut().find(|lane| lane.episode.is_none()) {
                    if let Some(next) = self.generation.checked_add(1) {
                        self.generation = next;
                        lane.episode = Some(Episode { load: p.key, generation: next, deck: deck as u8 });
                    } else { self.incomplete = true; }
                } else { self.incomplete = true; }
            }
        }
        let mut result = Contribution { episodes: [None; LANES], values: [[0.0; 2]; LANES],
            error: [[0.0; 2]; LANES], incomplete: self.incomplete };
        for (index, lane) in self.lanes.iter_mut().enumerate() {
            let Some(ep) = lane.episode else { continue; };
            result.episodes[index] = Some(ep);
            let deck = usize::from(ep.deck);
            let part = parts[deck].values.iter().find(|p| p.key == ep.load);
            let direct = part.map_or([0.0; 2], |p| p.value);
            let residual = if part.is_some() { parts[deck].residual(actual_decks[deck]) } else { [0.0; 2] };
            let mut value = direct.map(|v| (v * f64::from(gain[deck])) as f32);
            let mut error = std::array::from_fn(|c| {
                residual[c] * f64::from(gain[deck]).abs()
                    + (f64::from(value[c]) - direct[c] * f64::from(gain[deck])).abs()
            });
            if value != [0.0; 2] || error != [0.0; 2] { lane.pristine = false; }
            if !lane.pristine {
                for i in 0..3 {
                    value = lane.slots[i].process_bounded(value, self.kinds[i], self.wet[i], error, &mut lane.bounds[i]);
                    error = lane.bounds[i].output_error;
                }
            }
            for c in 0..2 {
                let cue = if pfl[deck] { direct[c] as f32 } else { 0.0 };
                let cue_error = if pfl[deck] { residual[c] + (f64::from(cue) - direct[c]).abs() } else { 0.0 };
                result.values[index][c] = value[c] * (1.0 - cue_mix) + cue * cue_mix;
                result.error[index][c] = super::error::mix(error[c], cue_error, value[c], cue, cue_mix);
            }
        }
        result
    }
}
