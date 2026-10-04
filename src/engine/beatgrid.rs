//! Manual musical coordinates in source seconds. Ordered local tempo anchors
//! preserve a continuous beat count; decoded audio and absolute cues stay untouched.
use serde::{Deserialize, Serialize};

pub(crate) const MIN_BPM: f64 = 20.0;
pub(crate) const MAX_BPM: f64 = 400.0;
pub(crate) const MAX_ANCHORS: usize = 64;
pub(super) const WORDS: usize = 4 + 2 * MAX_ANCHORS;
const POSITION_LIMIT: f64 = 1.0e10;

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Anchor {
    pub seconds: f64,
    pub bpm: f64,
    #[serde(skip)]
    beat: f64,
}
#[derive(Clone, Copy, Debug, PartialEq)]
struct Anchors { values: [Anchor; MAX_ANCHORS], len: usize }
impl Default for Anchors {
    fn default() -> Self { Self { values: [Anchor::default(); MAX_ANCHORS], len: 0 } }
}
impl Serialize for Anchors {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.values[..self.len].serialize(serializer)
    }
}
impl<'de> Deserialize<'de> for Anchors {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Visitor;
        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = Anchors;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result { write!(f, "at most {MAX_ANCHORS} ordered tempo anchors") }
            fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut seq: A) -> Result<Anchors, A::Error> {
                let mut result = Anchors::default();
                while let Some(anchor) = seq.next_element()? {
                    if result.len == MAX_ANCHORS { return Err(serde::de::Error::custom("Too many track tempo anchors")); }
                    result.values[result.len] = anchor; result.len += 1;
                }
                Ok(result)
            }
        }
        deserializer.deserialize_seq(Visitor)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "Stored", into = "Stored")]
pub(crate) struct Grid {
    downbeat_seconds: f64,
    seconds_per_beat: f64,
    anchors: Anchors,
}
#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Stored {
    downbeat_seconds: f64,
    seconds_per_beat: f64,
    #[serde(default)]
    anchors: Anchors,
}
impl From<Grid> for Stored {
    fn from(grid: Grid) -> Self { Self { downbeat_seconds: grid.downbeat_seconds, seconds_per_beat: grid.seconds_per_beat, anchors: grid.anchors } }
}
impl TryFrom<Stored> for Grid {
    type Error = &'static str;
    fn try_from(mut value: Stored) -> Result<Self, Self::Error> {
        if !value.downbeat_seconds.is_finite() || value.downbeat_seconds.abs() > POSITION_LIMIT {
            return Err("Grid downbeat must be a finite source position within the supported range");
        }
        if !value.seconds_per_beat.is_finite() || !(60.0 / MAX_BPM..=60.0 / MIN_BPM).contains(&value.seconds_per_beat) {
            return Err("Grid tempo must be between 20 and 400 BPM");
        }
        let mut previous = value.downbeat_seconds; let mut beat = 0.0; let mut period = value.seconds_per_beat;
        for anchor in &mut value.anchors.values[..value.anchors.len] {
            if !anchor.seconds.is_finite() || anchor.seconds <= previous || anchor.seconds.abs() > POSITION_LIMIT || !anchor.bpm.is_finite() || !(MIN_BPM..=MAX_BPM).contains(&anchor.bpm) {
                return Err("Tempo anchors require increasing source seconds after the downbeat and 20–400 BPM");
            }
            let next = beat + (anchor.seconds - previous) / period;
            if !next.is_finite() || next <= beat { return Err("Tempo anchors are too close to retain distinct continuous beat boundaries"); }
            beat = next;
            anchor.beat = beat; previous = anchor.seconds; period = 60.0 / anchor.bpm;
        }
        Ok(Self { downbeat_seconds: value.downbeat_seconds, seconds_per_beat: value.seconds_per_beat, anchors: value.anchors })
    }
}
impl Grid {
    /// Create a source-time grid.
    /// Takes a downbeat in seconds and initial tempo; returns validated musical coordinates without changing audio.
    pub fn new(downbeat_seconds: f64, bpm: f64) -> Result<Self, &'static str> {
        if !bpm.is_finite() || !(MIN_BPM..=MAX_BPM).contains(&bpm) { return Err("Grid tempo must be between 20 and 400 BPM"); }
        Stored { downbeat_seconds, seconds_per_beat: 60.0 / bpm, anchors: Anchors::default() }.try_into()
    }
    pub fn downbeat(&self) -> f64 { self.downbeat_seconds }
    pub fn period(&self) -> f64 { self.seconds_per_beat }
    pub fn bpm(&self) -> f64 { 60.0 / self.seconds_per_beat }
    pub fn anchors(&self) -> &[Anchor] { &self.anchors.values[..self.anchors.len] }
    fn segment_seconds(&self, seconds: f64) -> (f64, f64, f64) {
        let index = self.anchors().partition_point(|anchor| anchor.seconds <= seconds);
        if index == 0 { (self.downbeat_seconds, 0.0, self.seconds_per_beat) }
        else { let anchor = self.anchors.values[index - 1]; (anchor.seconds, anchor.beat, 60.0 / anchor.bpm) }
    }
    /// Read tempo at a source position.
    /// Takes source seconds; returns the local segment tempo, including pickup extrapolation before the downbeat.
    pub fn bpm_at(&self, seconds: f64) -> f64 { 60.0 / self.segment_seconds(seconds).2 }
    /// Convert source time to continuous musical beats.
    /// Takes source seconds; returns the local beat coordinate or refusal of a nonfinite/out-of-range position.
    pub fn beat_at(&self, seconds: f64) -> Option<f64> {
        if !seconds.is_finite() || seconds.abs() > POSITION_LIMIT { return None; }
        let (origin, beat, period) = self.segment_seconds(seconds);
        let beat = beat + (seconds - origin) / period; beat.is_finite().then_some(beat)
    }
    /// Convert continuous musical beats to source time.
    /// Takes a beat coordinate; returns source seconds across tempo boundaries or refusal of an excessive position.
    pub fn seconds_at(&self, beat: f64) -> Option<f64> {
        if !beat.is_finite() { return None; }
        let index = self.anchors().partition_point(|anchor| anchor.beat <= beat);
        let (origin, start, period) = if index == 0 { (self.downbeat_seconds, 0.0, self.seconds_per_beat) }
            else { let anchor = self.anchors.values[index - 1]; (anchor.seconds, anchor.beat, 60.0 / anchor.bpm) };
        let seconds = origin + (beat - start) * period;
        (seconds.is_finite() && seconds.abs() <= POSITION_LIMIT).then_some(seconds)
    }
    pub fn nearest(&self, seconds: f64) -> Option<f64> { self.seconds_at(self.beat_at(seconds)?.round()) }
    /// Advance through local source intervals without rounding a large absolute beat count.
    /// Takes source seconds and nonnegative beats; returns the new source position across all crossed anchors.
    pub fn advance(&self, mut seconds: f64, mut beats: f64) -> Option<f64> {
        if !seconds.is_finite() || seconds.abs() > POSITION_LIMIT || !beats.is_finite() || beats < 0.0 { return None; }
        let mut index = self.anchors().partition_point(|anchor| anchor.seconds <= seconds);
        let mut period = if index == 0 { self.seconds_per_beat } else { 60.0 / self.anchors.values[index - 1].bpm };
        while let Some(anchor) = self.anchors().get(index) {
            let span = (anchor.seconds - seconds) / period;
            if beats < span { break; }
            beats -= span;
            seconds = anchor.seconds;
            period = 60.0 / anchor.bpm;
            index += 1;
        }
        seconds += beats * period;
        (seconds.is_finite() && seconds.abs() <= POSITION_LIMIT).then_some(seconds)
    }
    /// Measure beats over local source intervals.
    /// Takes increasing finite source positions; returns their musical span without subtracting large absolute beat counts.
    pub fn beats_between(&self, mut start: f64, end: f64) -> Option<f64> {
        if !start.is_finite() || !end.is_finite() || start.abs() > POSITION_LIMIT || end.abs() > POSITION_LIMIT || end < start { return None; }
        let mut index = self.anchors().partition_point(|anchor| anchor.seconds <= start);
        let mut period = if index == 0 { self.seconds_per_beat } else { 60.0 / self.anchors.values[index - 1].bpm };
        let mut beats = 0.0;
        while let Some(anchor) = self.anchors().get(index).filter(|anchor| anchor.seconds < end) {
            beats += (anchor.seconds - start) / period;
            start = anchor.seconds;
            period = 60.0 / anchor.bpm;
            index += 1;
        }
        Some(beats + (end - start) / period)
    }
    /// Insert or update an exact local tempo anchor.
    /// Takes source seconds and BPM; returns a bounded ordered map with recomputed continuous beat boundaries.
    pub fn set_anchor(self, seconds: f64, bpm: f64) -> Result<Self, &'static str> {
        let mut stored: Stored = self.into();
        let index = self.anchors().partition_point(|anchor| anchor.seconds < seconds);
        if index == stored.anchors.len || stored.anchors.values[index].seconds != seconds {
            if stored.anchors.len == MAX_ANCHORS { return Err("Use at most 64 track tempo anchors"); }
            stored.anchors.values.copy_within(index..stored.anchors.len, index + 1); stored.anchors.len += 1;
        }
        stored.anchors.values[index] = Anchor { seconds, bpm, beat: 0.0 }; stored.try_into()
    }
    /// Delete one local tempo change.
    /// Takes its ordered index; returns a continuous map whose prior segment extends through the removed boundary.
    pub fn remove_anchor(self, index: usize) -> Result<Self, &'static str> {
        let mut stored: Stored = self.into();
        if index >= stored.anchors.len { return Err("Tempo anchor is no longer available"); }
        stored.anchors.values.copy_within(index + 1..stored.anchors.len, index); stored.anchors.len -= 1;
        stored.anchors.values[stored.anchors.len] = Anchor::default(); stored.try_into()
    }
    /// Shift the complete musical grid in source time.
    /// Takes a seconds offset; returns shifted downbeat and anchor positions while keeping all local tempi.
    pub fn slip(self, seconds: f64) -> Result<Self, &'static str> {
        let mut stored: Stored = self.into(); stored.downbeat_seconds += seconds;
        for anchor in &mut stored.anchors.values[..stored.anchors.len] { anchor.seconds += seconds; }
        stored.try_into()
    }
    /// Scale every local tempo about the musical downbeat.
    /// Takes a new initial BPM; returns a map retaining source anchor positions and relative tempo ratios.
    pub fn stretch(self, bpm: f64) -> Result<Self, &'static str> {
        if !bpm.is_finite() || !(MIN_BPM..=MAX_BPM).contains(&bpm) { return Err("Grid tempo must be between 20 and 400 BPM"); }
        let mut stored: Stored = self.into(); let ratio = bpm / self.bpm(); stored.seconds_per_beat = 60.0 / bpm;
        for anchor in &mut stored.anchors.values[..stored.anchors.len] { anchor.bpm *= ratio; }
        stored.try_into()
    }
    pub fn half_tempo(self) -> Result<Self, &'static str> { self.stretch(self.bpm() * 0.5) }
    pub fn double_tempo(self) -> Result<Self, &'static str> { self.stretch(self.bpm() * 2.0) }
    pub(super) fn encode(grid: Option<Self>) -> [u64; WORDS] {
        let mut words = [0; WORDS];
        if let Some(grid) = grid {
            words[..4].copy_from_slice(&[1, grid.downbeat_seconds.to_bits(), grid.seconds_per_beat.to_bits(), grid.anchors.len as u64]);
            for (anchor, words) in grid.anchors().iter().zip(words[4..].chunks_exact_mut(2)) { words.copy_from_slice(&[anchor.seconds.to_bits(), anchor.bpm.to_bits()]); }
        }
        words
    }
    pub(super) fn decode(words: impl AsRef<[u64]>) -> Option<Option<Self>> {
        let words = words.as_ref();
        if words.len() != WORDS { return None; }
        if words.iter().all(|word| *word == 0) { return Some(None); }
        if words.len() != WORDS || words[0] != 1 || words[3] > MAX_ANCHORS as u64 { return None; }
        let mut anchors = Anchors { len: words[3] as usize, ..Default::default() };
        for (anchor, words) in anchors.values[..anchors.len].iter_mut().zip(words[4..].chunks_exact(2)) { *anchor = Anchor { seconds: f64::from_bits(words[0]), bpm: f64::from_bits(words[1]), beat: 0.0 }; }
        if words[4 + anchors.len * 2..].iter().any(|word| *word != 0) { return None; }
        Stored { downbeat_seconds: f64::from_bits(words[1]), seconds_per_beat: f64::from_bits(words[2]), anchors }.try_into().ok().map(Some)
    }
}

/// One request's renderer acknowledgement, separate from the media receipt's
/// latest preparation. A later grid edit/Undo cannot erase this completion.
#[derive(Clone, Debug)]
pub(crate) struct GridEditAck(std::sync::Arc<std::sync::atomic::AtomicU8>);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum GridEditState { Pending, Applied, Rejected }
impl GridEditAck {
    pub fn new() -> Self { Self(std::sync::Arc::new(std::sync::atomic::AtomicU8::new(0))) }
    pub fn state(&self) -> GridEditState {
        match self.0.load(std::sync::atomic::Ordering::Acquire) {
            1 => GridEditState::Applied,
            2 => GridEditState::Rejected,
            _ => GridEditState::Pending,
        }
    }
    pub(super) fn applied(&self) { self.0.store(1, std::sync::atomic::Ordering::Release); }
    fn reject_pending(&self) {
        let _ = self.0.compare_exchange(0, 2, std::sync::atomic::Ordering::AcqRel, std::sync::atomic::Ordering::Acquire);
    }
}
pub(super) fn reject_retired(mut command: &super::Command) {
    loop {
        match command {
            super::Command::DeckGrid { ack, .. } => { ack.reject_pending(); return; }
            super::Command::Gesture { command: inner, .. } => command = inner,
            _ => return,
        }
    }
}

#[cfg(test)]
mod variable_tests;

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
        self.grid.as_ref().map_or_else(|| self.audio.as_ref().map_or(self.bpm, |audio| audio.bpm), |grid| grid.bpm_at(self.pos / self.audio.as_ref().map_or(48000.0, |audio| audio.sr as f64)) as f32)
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
        if let (Some(grid), Some(audio)) = (&self.grid, &self.audio) {
            return grid.nearest(position / audio.sr as f64).map_or(position, |seconds| seconds * audio.sr as f64).clamp(0.0, audio.frames() as f64);
        }
        let (origin, period) = self.grid_geometry(sr, bpm);
        let snapped = origin + ((position - origin) / period).round() * period;
        if self.grid.is_some() {
            snapped.clamp(0.0, self.audio.as_ref().map_or(f64::MAX, |audio| audio.frames() as f64))
        } else { snapped }
    }
    /// Convert a deck source frame to its musical coordinate.
    /// Takes a frame and fallback clock; returns local mapped beats, preserving the legacy uniform fallback.
    pub(super) fn grid_beat(&self, position: f64, sr: f32, bpm: f32) -> f64 {
        if let (Some(grid), Some(audio)) = (&self.grid, &self.audio) { return grid.beat_at(position / audio.sr as f64).unwrap_or(0.0); }
        let (origin, period) = self.grid_geometry(sr, bpm); (position - origin) / period
    }
    /// Convert a musical coordinate to a deck source frame.
    /// Takes beats and fallback clock; returns the local mapped frame, including tempo boundaries.
    pub(super) fn grid_position(&self, beat: f64, sr: f32, bpm: f32) -> f64 {
        if let (Some(grid), Some(audio)) = (&self.grid, &self.audio) { return grid.seconds_at(beat).unwrap_or(0.0) * audio.sr as f64; }
        let (origin, period) = self.grid_geometry(sr, bpm); origin + beat * period
    }
    /// Measure a musical loop across local tempo changes.
    /// Takes a source start, beat duration and fallback clock; returns the corresponding source-frame span.
    pub(super) fn grid_span(&self, start: f64, beats: f64, sr: f32, bpm: f32) -> f64 {
        if let (Some(grid), Some(audio)) = (&self.grid, &self.audio) { grid.advance(start / audio.sr as f64, beats).map_or(0.0, |seconds| seconds * audio.sr as f64 - start) }
        else { beats * self.grid_geometry(sr, bpm).1 }
    }
    /// Measure a source-frame range in local beats.
    /// Takes increasing source frames and a fallback clock; returns the musical span without subtracting absolute coordinates.
    pub(super) fn grid_beats_between(&self, start: f64, end: f64, sr: f32, bpm: f32) -> f64 {
        if let (Some(grid), Some(audio)) = (&self.grid, &self.audio) {
            grid.beats_between(start / audio.sr as f64, end / audio.sr as f64).unwrap_or(0.0)
        } else { (end - start) / self.grid_geometry(sr, bpm).1 }
    }
}

#[cfg(test)]
mod renderer_tests {
    use super::*;
    use crate::engine::{load_receipt::{Media, Receipt}, test_alloc, Command, Engine};

    #[test]
    fn grid_edit_reset_undo_and_reload_preserve_absolute_cues_without_callback_heap_traffic() {
        let (engine, mut rt) = Engine::headless_for_test(48_000, 128);
        let receipt = engine.initial_playback[0].clone().unwrap();
        rt.decks[0].pos = 12_000.0;
        rt.apply(Command::DeckHotCue { deck: 0, pad: 0, del: false });
        rt.clear_undo_for_test();
        let source_bpm = rt.decks[0].audio.as_ref().unwrap().bpm;
        let grid = Grid::new(1.25, 127.5).unwrap();
        let command = Command::DeckGrid { ack: GridEditAck::new(), deck: 0, grid: Some(grid), receipt: receipt.clone() };
        assert_eq!(test_alloc::measure(|| rt.apply(command)), test_alloc::Counts::default());
        assert_eq!(rt.decks[0].grid, Some(grid));
        assert_eq!(receipt.preparation().unwrap().1.grid, Some(grid));
        rt.publish_for_test();
        assert_eq!(rt.snap.lock().decks[0].bpm, 127.5);
        assert_eq!(rt.snap.lock().decks[0].source_bpm, source_bpm);
        assert_eq!(rt.decks[0].hotcues[0].pos, 12_000.0);
        assert_eq!(receipt.preparation().unwrap().1.hotcues[0], Some(0.25));
        assert_eq!(test_alloc::measure(|| rt.apply(Command::Undo)), test_alloc::Counts::default());
        assert_eq!(rt.decks[0].grid, None);
        assert_eq!(receipt.preparation().unwrap().1.grid, None);
        rt.apply(Command::Redo);
        let loaded = Receipt::with_preparation(Some(receipt.preparation().unwrap().1));
        rt.apply(Command::DeckLoadRequested { deck: 1, media: Media::Builtin(0), receipt: loaded.clone() });
        assert_eq!(rt.decks[1].grid, Some(grid));
        assert_eq!(rt.decks[1].hotcues[0].pos, 12_000.0);
        assert!(!rt.decks[1].playing);
        rt.apply(Command::DeckGrid { ack: GridEditAck::new(), deck: 0, grid: None, receipt: receipt.clone() });
        assert_eq!(rt.decks[0].grid, None);
        assert_eq!(rt.decks[0].hotcues[0].pos, 12_000.0);
        // A lagging snapshot may still show the old effective manual tempo,
        // while its immutable source hint remains correct for a new draft.
        assert_eq!(rt.snap.lock().decks[0].bpm, 127.5);
        assert_eq!(rt.snap.lock().decks[0].source_bpm, source_bpm);
        rt.apply(Command::Undo);
        assert_eq!(rt.decks[0].grid, Some(grid));
        let before = engine.undo.checkpoint();
        let same = Command::DeckGrid { ack: GridEditAck::new(), deck: 0, grid: Some(grid), receipt: receipt.clone() };
        assert_eq!(test_alloc::measure(|| rt.apply(same)), test_alloc::Counts::default());
        assert_eq!(engine.undo.checkpoint(), before);
        let commands = [
            Command::DeckGrid { ack: GridEditAck::new(), deck: 255, grid: None, receipt: receipt.clone() },
            Command::DeckGrid { ack: GridEditAck::new(), deck: 0, grid: None, receipt: Receipt::new() },
            Command::DeckGrid { ack: GridEditAck::new(), deck: 0, grid: None, receipt: loaded },
        ];
        assert_eq!(test_alloc::measure(|| { for command in commands { rt.apply(command); } }), test_alloc::Counts::default());
        assert_eq!(engine.undo.checkpoint(), before);
        assert_eq!(rt.decks[0].grid, Some(grid));
        rt.apply(Command::DeckUnload { deck: 0 });
        let before = engine.undo.checkpoint();
        rt.apply(Command::DeckGrid { ack: GridEditAck::new(), deck: 0, grid: Some(grid), receipt });
        assert_eq!(engine.undo.checkpoint(), before);
        assert_eq!(rt.decks[0].grid, None);
    }

    #[test]
    fn request_acknowledgements_survive_later_edits_and_undo_and_reject_retired_work() {
        let (engine, mut rt) = Engine::headless_for_test(48_000, 128);
        let receipt = engine.initial_playback[0].clone().unwrap();
        let b = Grid::new(0.5, 120.0).unwrap();
        let c = Grid::new(1.5, 100.0).unwrap();
        let first = GridEditAck::new();
        let later = GridEditAck::new();
        let commands = [
            Command::DeckGrid { deck: 0, grid: Some(b), receipt: receipt.clone(), ack: first.clone() },
            Command::DeckGrid { deck: 0, grid: Some(c), receipt: receipt.clone(), ack: later.clone() },
        ];
        assert_eq!(first.state(), GridEditState::Pending);
        assert_eq!(test_alloc::measure(|| { for command in commands { rt.apply(command); } }), test_alloc::Counts::default());
        assert_eq!(first.state(), GridEditState::Applied);
        assert_eq!(later.state(), GridEditState::Applied);
        assert_eq!(receipt.preparation().unwrap().1.grid, Some(c));
        rt.apply(Command::Undo);
        assert_eq!(later.state(), GridEditState::Applied);
        assert_eq!(receipt.preparation().unwrap().1.grid, Some(b));
        let unchanged = GridEditAck::new();
        let before = engine.undo.checkpoint();
        rt.apply(Command::DeckGrid { deck: 0, grid: Some(b), receipt: receipt.clone(), ack: unchanged.clone() });
        assert_eq!(unchanged.state(), GridEditState::Applied);
        assert_eq!(engine.undo.checkpoint(), before);
        let invalid = GridEditAck::new();
        let rejected = Command::DeckGrid { deck: 0, grid: Some(c), receipt: Receipt::new(), ack: invalid.clone() };
        assert_eq!(test_alloc::measure(|| rt.apply(rejected)), test_alloc::Counts::default());
        assert_eq!(invalid.state(), GridEditState::Rejected);
        let cancelled = GridEditAck::new();
        let dropped = Command::Gesture { id: 45, command: Box::new(Command::DeckGrid {
            deck: 0, grid: Some(c), receipt, ack: cancelled.clone(),
        }) };
        assert_eq!(test_alloc::measure(|| rt.undo.retire_command(dropped)), test_alloc::Counts::default());
        assert_eq!(cancelled.state(), GridEditState::Rejected);
        // Even if the GUI has closed, unique command ownership retires off callback.
        let unique = Command::DeckGrid { deck: 255, grid: None, receipt: Receipt::new(), ack: GridEditAck::new() };
        assert_eq!(test_alloc::measure(|| rt.apply(unique)), test_alloc::Counts::default());
    }

    #[test]
    fn match_and_quantized_loops_follow_each_manual_downbeat_and_tempo() {
        let (engine, mut rt) = Engine::headless_for_test(48_000, 128);
        let a = Grid::new(1.25, 120.0).unwrap();
        let b = Grid::new(0.375, 90.0).unwrap();
        for (deck, grid) in [(0, a), (1, b)] {
            rt.apply(Command::DeckGrid { ack: GridEditAck::new(), deck, grid: Some(grid), receipt: engine.initial_playback[deck as usize].clone().unwrap() });
        }
        rt.decks[0].pos = a.seconds_at(3.25).unwrap() * 48_000.0;
        rt.decks[1].pos = b.seconds_at(5.8).unwrap() * 48_000.0;
        rt.decks[0].playing = true;
        rt.decks[1].playing = true;
        rt.xfader = 0.25;
        let favorite = rt.decks[0].pos;
        assert_eq!(test_alloc::measure(|| rt.apply(Command::DeckMatch)), test_alloc::Counts::default());
        assert_eq!(rt.decks[0].pos, favorite);
        assert!((b.beat_at(rt.decks[1].pos / 48_000.0).unwrap() - 5.25).abs() < 1e-10);
        assert_eq!(rt.decks[1].sync_bpm, 120.0);
        rt.process_interleaved(&mut [0.0; 2], 2);
        assert!((rt.decks[1].target_rate - 120.0 / 90.0).abs() < 1e-6);
        rt.quantize = true;
        rt.decks[0].pos = a.seconds_at(2.2).unwrap() * 48_000.0;
        rt.apply(Command::DeckLoopIn { deck: 0 });
        assert!((rt.decks[0].loop_start - a.seconds_at(2.0).unwrap() * 48_000.0).abs() < 1e-8);
        rt.decks[0].pos = a.seconds_at(6.4).unwrap() * 48_000.0;
        rt.apply(Command::DeckLoopOut { deck: 0 });
        assert!((rt.decks[0].loop_len - 4.0 * a.period() * 48_000.0).abs() < 1e-8);
        rt.decks[0].loop_on = false;
        rt.decks[0].pos = a.seconds_at(3.2).unwrap() * 48_000.0;
        rt.apply(Command::DeckLoop { deck: 0, beats: 8.0 });
        assert!((rt.decks[0].loop_start - a.seconds_at(3.0).unwrap() * 48_000.0).abs() < 1e-8);
        assert!((rt.decks[0].loop_len - 8.0 * a.period() * 48_000.0).abs() < 1e-8);
        rt.decks[0].pos = a.seconds_at(-0.6).unwrap() * 48_000.0;
        rt.apply(Command::DeckReloop { deck: 0 });
        assert!((a.beat_at(rt.decks[0].loop_start / 48_000.0).unwrap() + 1.0).abs() < 1e-10);
        let before = rt.decks[0].loop_start;
        rt.quantize = false;
        rt.decks[0].pos = before + 123.0;
        rt.apply(Command::DeckLoopIn { deck: 0 });
        assert_eq!(rt.decks[0].loop_start, before + 123.0);
    }
}
