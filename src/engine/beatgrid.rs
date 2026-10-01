//! A manual constant-tempo musical coordinate system in source seconds. The
//! downbeat need not be file frame zero; pickups have negative beat coordinates.
//! Analysis BPM is a separate hint and cannot manufacture a verified downbeat.
use serde::{Deserialize, Serialize};

pub(crate) const MIN_BPM: f64 = 20.0;
pub(crate) const MAX_BPM: f64 = 400.0;
pub(super) const WORDS: usize = 3;
const POSITION_LIMIT: f64 = 1.0e10;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "Stored", into = "Stored")]
pub(crate) struct Grid {
    downbeat_seconds: f64,
    seconds_per_beat: f64,
}
#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Stored {
    downbeat_seconds: f64,
    seconds_per_beat: f64,
}
impl From<Grid> for Stored {
    fn from(grid: Grid) -> Self {
        Self { downbeat_seconds: grid.downbeat_seconds, seconds_per_beat: grid.seconds_per_beat }
    }
}
impl TryFrom<Stored> for Grid {
    type Error = &'static str;
    fn try_from(value: Stored) -> Result<Self, Self::Error> {
        if !value.downbeat_seconds.is_finite() || value.downbeat_seconds.abs() > POSITION_LIMIT {
            return Err("Grid downbeat must be a finite source position within the supported range");
        }
        if !value.seconds_per_beat.is_finite()
            || !(60.0 / MAX_BPM..=60.0 / MIN_BPM).contains(&value.seconds_per_beat) {
            return Err("Grid tempo must be between 20 and 400 BPM");
        }
        Ok(Self { downbeat_seconds: value.downbeat_seconds, seconds_per_beat: value.seconds_per_beat })
    }
}
impl Grid {
    pub fn new(downbeat_seconds: f64, bpm: f64) -> Result<Self, &'static str> {
        if !bpm.is_finite() || !(MIN_BPM..=MAX_BPM).contains(&bpm) {
            return Err("Grid tempo must be between 20 and 400 BPM");
        }
        Stored { downbeat_seconds, seconds_per_beat: 60.0 / bpm }.try_into()
    }
    pub fn downbeat(self) -> f64 { self.downbeat_seconds }
    pub fn period(self) -> f64 { self.seconds_per_beat }
    pub fn bpm(self) -> f64 { 60.0 / self.seconds_per_beat }
    pub fn beat_at(self, seconds: f64) -> Option<f64> {
        let beat = (seconds - self.downbeat_seconds) / self.seconds_per_beat;
        (seconds.is_finite() && seconds.abs() <= POSITION_LIMIT && beat.is_finite()).then_some(beat)
    }
    pub fn seconds_at(self, beat: f64) -> Option<f64> {
        let seconds = self.downbeat_seconds + beat * self.seconds_per_beat;
        (beat.is_finite() && seconds.is_finite() && seconds.abs() <= POSITION_LIMIT).then_some(seconds)
    }
    pub fn nearest(self, seconds: f64) -> Option<f64> {
        self.seconds_at(self.beat_at(seconds)?.round())
    }
    pub fn slip(self, seconds: f64) -> Result<Self, &'static str> {
        Stored { downbeat_seconds: self.downbeat_seconds + seconds, ..self.into() }.try_into()
    }
    /// Stretch is anchored at the musical downbeat; existing absolute cue
    /// positions and PCM are deliberately not part of this coordinate edit.
    pub fn stretch(self, bpm: f64) -> Result<Self, &'static str> {
        Self::new(self.downbeat_seconds, bpm)
    }
    pub fn half_tempo(self) -> Result<Self, &'static str> { self.stretch(self.bpm() * 0.5) }
    pub fn double_tempo(self) -> Result<Self, &'static str> { self.stretch(self.bpm() * 2.0) }
    pub(super) fn encode(grid: Option<Self>) -> [u64; WORDS] {
        grid.map_or([0; WORDS], |grid| [1, grid.downbeat_seconds.to_bits(), grid.seconds_per_beat.to_bits()])
    }
    pub(super) fn decode(words: [u64; WORDS]) -> Option<Option<Self>> {
        if words == [0; WORDS] { return Some(None); }
        if words[0] != 1 { return None; }
        let grid = Stored { downbeat_seconds: f64::from_bits(words[1]), seconds_per_beat: f64::from_bits(words[2]) }.try_into().ok()?;
        Some(Some(grid))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::test_alloc;

    #[test]
    fn pickup_and_leading_silence_use_musical_coordinates_independent_of_sample_rate() {
        for (origin, bpm) in [(0.375, 120.0), (7.25, 97.5), (0.0, 400.0), (-0.25, 60.0)] {
            let grid = Grid::new(origin, bpm).unwrap();
            assert_eq!(grid.beat_at(origin), Some(0.0));
            assert_eq!(grid.seconds_at(0.0), Some(origin));
            for beat in [-8.0, -1.5, -1.0, 0.0, 0.5, 1.0, 127.25] {
                let seconds = grid.seconds_at(beat).unwrap();
                assert!((grid.beat_at(seconds).unwrap() - beat).abs() < 1.0e-10);
                for sr in [44_100.0, 48_000.0, 96_000.0] {
                    let source_frame = seconds * sr;
                    assert!((grid.beat_at(source_frame / sr).unwrap() - beat).abs() < 1.0e-10);
                }
            }
        }
        let pickup = Grid::new(1.25, 120.0).unwrap();
        assert_eq!(pickup.beat_at(0.0), Some(-2.5));
        assert_eq!(pickup.nearest(1.47), Some(1.25));
        assert_eq!(pickup.nearest(1.6), Some(1.75));
    }
    #[test]
    fn slip_and_stretch_are_distinct_and_half_double_retain_the_downbeat() {
        let grid = Grid::new(4.0, 120.0).unwrap();
        let slip = grid.slip(0.025).unwrap();
        assert_eq!(slip.bpm(), grid.bpm());
        assert!((slip.seconds_at(32.0).unwrap() - grid.seconds_at(32.0).unwrap() - 0.025).abs() < 1e-12);
        let stretched = grid.stretch(125.0).unwrap();
        assert_eq!(stretched.downbeat(), grid.downbeat());
        assert_ne!(stretched.seconds_at(32.0), grid.seconds_at(32.0));
        let half = grid.half_tempo().unwrap();
        assert_eq!(half.bpm(), 60.0);
        assert_eq!(half.downbeat(), grid.downbeat());
        assert_eq!(half.double_tempo().unwrap(), grid);
        let double = grid.double_tempo().unwrap();
        assert_eq!(double.bpm(), 240.0);
        assert_eq!(double.half_tempo().unwrap(), grid);
        assert!(Grid::new(0.0, MIN_BPM).unwrap().half_tempo().is_err());
        assert!(Grid::new(0.0, MAX_BPM).unwrap().double_tempo().is_err());
    }
    #[test]
    fn storage_rejects_unknown_fields_invalid_geometry_and_corrupt_atomic_tags() {
        let grid = Grid::new(2.5, 127.125).unwrap();
        let json = serde_json::to_value(grid).unwrap();
        assert_eq!(serde_json::from_value::<Grid>(json.clone()).unwrap(), grid);
        for (field, value) in [("seconds_per_beat", serde_json::json!(0.0)),
            ("seconds_per_beat", serde_json::json!(-1.0)),
            ("downbeat_seconds", serde_json::json!(1e11)),
            ("automatic", serde_json::json!(true))] {
            let mut invalid = json.clone(); invalid[field] = value;
            assert!(serde_json::from_value::<Grid>(invalid).is_err());
        }
        for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert!(Grid::new(bad, 120.0).is_err()); assert!(Grid::new(0.0, bad).is_err());
            assert!(grid.beat_at(bad).is_none()); assert!(grid.seconds_at(bad).is_none());
        }
        assert_eq!(Grid::decode(Grid::encode(Some(grid))), Some(Some(grid)));
        assert_eq!(Grid::decode(Grid::encode(None)), Some(None));
        assert!(Grid::decode([2, 0, 0]).is_none());
        assert!(Grid::decode([0, 1, 0]).is_none());
        assert!(Grid::decode([1, 0, f64::NAN.to_bits()]).is_none());
        assert_eq!(test_alloc::measure(|| {
            for _ in 0..1024 {
                std::hint::black_box(Grid::decode(Grid::encode(Some(grid))));
                std::hint::black_box(grid.nearest(16.75));
                std::hint::black_box(grid.slip(0.001).unwrap().stretch(125.0).unwrap());
            }
        }), test_alloc::Counts::default());
    }
}

impl super::DeckRt {
    pub(super) fn musical_bpm(&self) -> f32 {
        self.grid.map_or_else(|| self.audio.as_ref().map_or(self.bpm, |audio| audio.bpm), |grid| grid.bpm() as f32)
    }
    pub(super) fn grid_geometry(&self, fallback_sr: f32, fallback_bpm: f32) -> (f64, f64) {
        if let Some(grid) = self.grid {
            let sr = self.audio.as_ref().map_or(fallback_sr as f64, |audio| audio.sr as f64);
            (grid.downbeat() * sr, grid.period() * sr)
        } else {
            // Preserve the original arithmetic for unprepared tracks.
            (0.0, self.audio.as_ref().map(|audio| audio.sr as f64 * 60.0 / audio.bpm.max(1.0) as f64)
                .unwrap_or(fallback_sr as f64 * 60.0 / fallback_bpm as f64))
        }
    }
    pub(super) fn grid_snap(&self, position: f64, sr: f32, bpm: f32) -> f64 {
        let (origin, period) = self.grid_geometry(sr, bpm);
        let snapped = origin + ((position - origin) / period).round() * period;
        if self.grid.is_some() {
            snapped.clamp(0.0, self.audio.as_ref().map_or(f64::MAX, |audio| audio.frames() as f64))
        } else { snapped }
    }
}
