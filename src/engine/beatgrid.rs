//! A bounded manual beat map in unchanged source seconds.
use serde::{Deserialize, Serialize};

pub(crate) const MIN_BPM: f64 = 20.0;
pub(crate) const MAX_BPM: f64 = 400.0;
pub(crate) const MAX_ANCHORS: usize = 32;
pub(super) const WORDS: usize = 4 + 2 * MAX_ANCHORS;
const POSITION_LIMIT: f64 = 1.0e10;

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Anchor {
    pub beat: f64,
    pub seconds: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Anchors {
    values: [Anchor; MAX_ANCHORS],
    count: u8,
}
impl Default for Anchors {
    fn default() -> Self { Self { values: [Anchor::default(); MAX_ANCHORS], count: 0 } }
}
impl Anchors {
    fn slice(&self) -> &[Anchor] { &self.values[..self.count as usize] }
    fn empty(&self) -> bool { self.count == 0 }
}
impl Serialize for Anchors {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.slice().serialize(serializer)
    }
}
impl<'de> Deserialize<'de> for Anchors {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Visitor;
        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = Anchors;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                write!(f, "at most {MAX_ANCHORS} ordered tempo anchors")
            }
            fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut sequence: A) -> Result<Anchors, A::Error> {
                let mut anchors = Anchors::default();
                while let Some(anchor) = sequence.next_element()? {
                    if anchors.count as usize == MAX_ANCHORS {
                        return Err(serde::de::Error::custom("Beatgrid exceeds 32 tempo anchors"));
                    }
                    anchors.values[anchors.count as usize] = anchor;
                    anchors.count += 1;
                }
                Ok(anchors)
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
    #[serde(default, skip_serializing_if = "Anchors::empty")]
    anchors: Anchors,
}
impl From<Grid> for Stored {
    fn from(grid: Grid) -> Self {
        Self { downbeat_seconds: grid.downbeat_seconds, seconds_per_beat: grid.seconds_per_beat, anchors: grid.anchors }
    }
}
impl TryFrom<Stored> for Grid {
    type Error = &'static str;
    fn try_from(value: Stored) -> Result<Self, Self::Error> {
        if !value.downbeat_seconds.is_finite() || value.downbeat_seconds.abs() > POSITION_LIMIT {
            return Err("Grid downbeat must be a finite source position within the supported range");
        }
        let valid_period = |period: f64| period.is_finite() && (60.0 / MAX_BPM..=60.0 / MIN_BPM).contains(&period);
        if !valid_period(value.seconds_per_beat) { return Err("Grid tempo must be between 20 and 400 BPM"); }
        let mut previous = Anchor { beat: 0.0, seconds: value.downbeat_seconds };
        for anchor in value.anchors.slice() {
            if !anchor.beat.is_finite() || !anchor.seconds.is_finite()
                || anchor.beat <= previous.beat || anchor.beat > POSITION_LIMIT
                || anchor.seconds <= previous.seconds || anchor.seconds.abs() > POSITION_LIMIT {
                return Err("Tempo anchors need strictly increasing beat and source positions after beat 0");
            }
            if !valid_period((anchor.seconds - previous.seconds) / (anchor.beat - previous.beat)) {
                return Err("Every tempo segment must stay between 20 and 400 BPM");
            }
            previous = *anchor;
        }
        let mut grid = Self { downbeat_seconds: value.downbeat_seconds, seconds_per_beat: value.seconds_per_beat, anchors: value.anchors };
        grid.seconds_per_beat = grid.period();
        Ok(grid)
    }
}
impl Grid {
    /// Create a constant manual grid.
    /// Takes source downbeat seconds and BPM; returns validated musical coordinates.
    pub fn new(downbeat_seconds: f64, bpm: f64) -> Result<Self, &'static str> {
        if !bpm.is_finite() || !(MIN_BPM..=MAX_BPM).contains(&bpm) {
            return Err("Grid tempo must be between 20 and 400 BPM");
        }
        Stored { downbeat_seconds, seconds_per_beat: 60.0 / bpm, anchors: Anchors::default() }.try_into()
    }
    pub fn downbeat(self) -> f64 { self.downbeat_seconds }
    pub fn anchors(&self) -> &[Anchor] { self.anchors.slice() }
    pub fn period(self) -> f64 { self.segment(0).2 }
    pub fn bpm(self) -> f64 { 60.0 / self.period() }

    /// Locate one continuous linear segment.
    /// Takes the count of anchors preceding a coordinate; returns its origin beat, source time and local beat period.
    fn segment(self, index: usize) -> (f64, f64, f64) {
        let anchors = self.anchors.slice();
        let origin = Anchor { beat: 0.0, seconds: self.downbeat_seconds };
        if anchors.is_empty() { return (0.0, origin.seconds, self.seconds_per_beat); }
        let left = index.checked_sub(1).map_or(origin, |i| anchors[i]);
        let period = if let Some(right) = anchors.get(index) {
            (right.seconds - left.seconds) / (right.beat - left.beat)
        } else {
            let previous = index.checked_sub(2).map_or(origin, |i| anchors[i]);
            (left.seconds - previous.seconds) / (left.beat - previous.beat)
        };
        (left.beat, left.seconds, period)
    }
    /// Read tempo at a source position.
    /// Takes source seconds; returns the following segment's BPM at an exact boundary.
    pub fn bpm_at(self, seconds: f64) -> Option<f64> {
        if !seconds.is_finite() || seconds.abs() > POSITION_LIMIT { return None; }
        Some(60.0 / self.segment(self.anchors.slice().partition_point(|a| a.seconds <= seconds)).2)
    }
    /// Convert source time to musical beats.
    /// Takes finite source seconds; returns a continuous beat position, including negative pickups.
    pub fn beat_at(self, seconds: f64) -> Option<f64> {
        if !seconds.is_finite() || seconds.abs() > POSITION_LIMIT { return None; }
        let (beat, origin, period) = self.segment(self.anchors.slice().partition_point(|a| a.seconds <= seconds));
        let result = beat + (seconds - origin) / period;
        result.is_finite().then_some(result)
    }
    /// Convert musical beats to source time.
    /// Takes finite beats; returns a source position using the same continuous segments.
    pub fn seconds_at(self, beat: f64) -> Option<f64> {
        if !beat.is_finite() { return None; }
        let (origin, seconds, period) = self.segment(self.anchors.slice().partition_point(|a| a.beat <= beat));
        let result = seconds + (beat - origin) * period;
        (result.is_finite() && result.abs() <= POSITION_LIMIT).then_some(result)
    }
    pub fn nearest(self, seconds: f64) -> Option<f64> { self.seconds_at(self.beat_at(seconds)?.round()) }

    /// Insert or replace a manual tempo anchor.
    /// Takes its beat and unchanged source time; returns a bounded continuous map or retains the caller's original on error.
    pub fn with_anchor(self, beat: f64, seconds: f64) -> Result<Self, &'static str> {
        let mut next = self;
        let index = next.anchors.slice().partition_point(|a| a.beat < beat);
        let replacing = next.anchors.slice().get(index).is_some_and(|a| a.beat == beat);
        if !replacing {
            let count = next.anchors.count as usize;
            if count == MAX_ANCHORS { return Err("Beatgrid exceeds 32 tempo anchors"); }
            next.anchors.values.copy_within(index..count, index + 1);
            next.anchors.count += 1;
        }
        next.anchors.values[index] = Anchor { beat, seconds };
        Stored::from(next).try_into()
    }
    /// Remove one captured anchor.
    /// Takes its index; returns a validated continuous map with the remaining anchors preserved.
    pub fn without_anchor(self, index: usize) -> Result<Self, &'static str> {
        let count = self.anchors.count as usize;
        if index >= count { return Err("This tempo anchor no longer exists"); }
        let mut next = self;
        next.seconds_per_beat = self.period();
        next.anchors.values.copy_within(index + 1..count, index);
        next.anchors.values[count - 1] = Anchor::default();
        next.anchors.count -= 1;
        Stored::from(next).try_into()
    }
    /// Move every beat without changing tempo.
    /// Takes a source-time offset; returns a map with all anchors shifted equally.
    pub fn slip(self, seconds: f64) -> Result<Self, &'static str> {
        let mut next = self;
        next.downbeat_seconds += seconds;
        for anchor in &mut next.anchors.values[..next.anchors.count as usize] { anchor.seconds += seconds; }
        Stored::from(next).try_into()
    }
    /// Scale all segments around the downbeat.
    /// Takes new initial BPM; returns a map preserving beat coordinates and relative tempo changes.
    pub fn stretch(self, bpm: f64) -> Result<Self, &'static str> {
        if !bpm.is_finite() || !(MIN_BPM..=MAX_BPM).contains(&bpm) { return Err("Grid tempo must be between 20 and 400 BPM"); }
        let mut next = self;
        let ratio = self.bpm() / bpm;
        next.seconds_per_beat = 60.0 / bpm;
        for anchor in &mut next.anchors.values[..next.anchors.count as usize] {
            anchor.seconds = self.downbeat_seconds + (anchor.seconds - self.downbeat_seconds) * ratio;
        }
        Stored::from(next).try_into()
    }
    pub fn half_tempo(self) -> Result<Self, &'static str> { self.stretch(self.bpm() * 0.5) }
    pub fn double_tempo(self) -> Result<Self, &'static str> { self.stretch(self.bpm() * 2.0) }
    /// Publish one fixed-size grid snapshot.
    /// Takes an optional map; returns allocation-free atomic words with zero unused anchor slots.
    pub(super) fn encode(grid: Option<Self>) -> [u64; WORDS] {
        let mut words = [0; WORDS];
        if let Some(grid) = grid {
            words[..4].copy_from_slice(&[1, grid.downbeat_seconds.to_bits(), grid.seconds_per_beat.to_bits(), grid.anchors.count as u64]);
            for (dest, anchor) in words[4..].chunks_exact_mut(2).zip(grid.anchors.slice()) {
                dest.copy_from_slice(&[anchor.beat.to_bits(), anchor.seconds.to_bits()]);
            }
        }
        words
    }
    /// Read a complete bounded atomic map.
    /// Takes fixed-size words; returns absent, valid or corrupt geometry without allocation.
    pub(super) fn decode(words: [u64; WORDS]) -> Option<Option<Self>> {
        if words == [0; WORDS] { return Some(None); }
        if words[0] != 1 || words[3] > MAX_ANCHORS as u64 { return None; }
        let count = words[3] as usize;
        if words[4 + 2 * count..].iter().any(|word| *word != 0) { return None; }
        let mut anchors = Anchors { count: count as u8, ..Anchors::default() };
        for (dest, source) in anchors.values.iter_mut().zip(words[4..].chunks_exact(2)).take(count) {
            *dest = Anchor { beat: f64::from_bits(source[0]), seconds: f64::from_bits(source[1]) };
        }
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
        for triple in [[2, 0, 0], [0, 1, 0], [1, 0, f64::NAN.to_bits()]] {
            let mut words = [0; WORDS]; words[..3].copy_from_slice(&triple);
            assert!(Grid::decode(words).is_none());
        }
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
    /// Read the current mapped source tempo.
    /// Takes this deck; returns local BPM without changing analyzed metadata.
    pub(super) fn musical_bpm(&self) -> f32 {
        self.grid.and_then(|grid| grid.bpm_at(self.pos / self.audio.as_ref().map_or(1.0, |audio| audio.sr as f64)))
            .map_or_else(|| self.audio.as_ref().map_or(self.bpm, |audio| audio.bpm), |bpm| bpm as f32)
    }
    /// Advance one synced sample through mapped tempo boundaries.
    /// Takes output sample rate; returns source-speed ratio for anchored grids or none for the existing unanchored smoothing path.
    pub(super) fn mapped_sync_rate(&self, output_rate: f32) -> Option<f32> {
        let grid = self.grid.filter(|grid| !grid.anchors().is_empty())?;
        let audio = self.audio.as_ref()?;
        let seconds = self.pos / f64::from(audio.sr);
        let beat = grid.beat_at(seconds)?;
        let next = grid.seconds_at(beat + f64::from(self.sync_bpm) / (60.0 * f64::from(output_rate)))?;
        let rate = ((next - seconds) * f64::from(output_rate)) as f32;
        (rate.is_finite() && rate > 0.0).then_some(rate)
    }
    /// Wrap a synced loop in musical coordinates.
    /// Takes the advanced source-frame position; returns its beat-preserving loop remainder for an anchored map.
    pub(super) fn mapped_loop_wrap(&self, position: f64) -> Option<f64> {
        let grid = self.grid.filter(|grid| !grid.anchors().is_empty())?;
        let source_rate = f64::from(self.audio.as_ref()?.sr);
        let start = grid.beat_at(self.loop_start / source_rate)?;
        let end = grid.beat_at((self.loop_start + self.loop_len) / source_rate)?;
        let span = end - start;
        if !span.is_finite() || span <= 0.0 { return None; }
        let beat = grid.beat_at(position / source_rate)?;
        Some(grid.seconds_at(start + (beat - start).rem_euclid(span))? * source_rate)
    }
    fn source_rate(&self, fallback: f32) -> f64 {
        self.audio.as_ref().map_or(fallback as f64, |audio| audio.sr as f64)
    }
    fn fallback_period(&self, sr: f32, bpm: f32) -> f64 {
        self.audio.as_ref().map(|audio| audio.sr as f64 * 60.0 / audio.bpm.max(1.0) as f64)
            .unwrap_or(sr as f64 * 60.0 / bpm as f64)
    }
    /// Convert source frames into musical coordinates.
    /// Takes frame position and unprepared-track fallback; returns the local beat position.
    pub(super) fn grid_beat_at(&self, position: f64, sr: f32, bpm: f32) -> f64 {
        self.grid.and_then(|grid| grid.beat_at(position / self.source_rate(sr)))
            .unwrap_or_else(|| position / self.fallback_period(sr, bpm))
    }
    /// Read phase while retaining the unprepared-track arithmetic.
    /// Takes source frame and fallback clock; returns positive phase within the local beat.
    pub(super) fn grid_phase(&self, position: f64, sr: f32, bpm: f32) -> f64 {
        if self.grid.is_some() { self.grid_beat_at(position, sr, bpm).rem_euclid(1.0) }
        else { let period = self.fallback_period(sr, bpm); position.rem_euclid(period) / period }
    }
    /// Convert musical coordinates into unchanged source frames.
    /// Takes beat position and unprepared-track fallback; returns the corresponding source frame.
    pub(super) fn grid_position_at(&self, beat: f64, sr: f32, bpm: f32) -> f64 {
        self.grid.and_then(|grid| grid.seconds_at(beat)).map_or_else(
            || beat * self.fallback_period(sr, bpm), |seconds| seconds * self.source_rate(sr))
    }
    /// Measure a musical loop through every crossed tempo boundary.
    /// Takes starting source frame, beat length and unprepared-track fallback; returns its source-frame length.
    pub(super) fn grid_span(&self, position: f64, beats: f64, sr: f32, bpm: f32) -> f64 {
        if self.grid.is_some() {
            self.grid_position_at(self.grid_beat_at(position, sr, bpm) + beats, sr, bpm) - position
        } else { beats * self.fallback_period(sr, bpm) }
    }
    /// Snap to a mapped beat without moving the original audio.
    /// Takes source frame and unprepared-track fallback; returns a bounded prepared position or the legacy unprepared snap.
    pub(super) fn grid_snap(&self, position: f64, sr: f32, bpm: f32) -> f64 {
        let snapped = self.grid_position_at(self.grid_beat_at(position, sr, bpm).round(), sr, bpm);
        if self.grid.is_some() {
            snapped.clamp(0.0, self.audio.as_ref().map_or(f64::MAX, |audio| audio.frames() as f64))
        } else { snapped }
    }
}

#[cfg(test)]
mod renderer_tests {
    use super::*;
    use crate::engine::{load_receipt::{Media, Receipt}, test_alloc, Command, Engine};

    #[test]
    fn catalog_grid_lock_blocks_queued_edits_and_undo_with_zero_callback_heap_work() {
        let (engine,mut rt)=Engine::headless_for_test(48000,128);let receipt=engine.initial_playback[0].clone().unwrap();
        let original=Grid::new(0.25,123.0).unwrap();let changed=Grid::new(2.0,150.0).unwrap();
        rt.apply(Command::DeckGrid {deck:0,grid:Some(original),receipt:receipt.clone(),ack:GridEditAck::new()});
        receipt.set_grid_protection(true,Some(original));
        let ack=GridEditAck::new();let command=Command::DeckGrid {deck:0,grid:Some(changed),receipt:receipt.clone(),ack:ack.clone()};
        assert_eq!(test_alloc::measure(||rt.apply(command)),test_alloc::Counts::default());assert_eq!(ack.state(),GridEditState::Rejected);assert_eq!(rt.decks[0].grid,Some(original));
        assert_eq!(test_alloc::measure(||rt.apply(Command::Undo)),test_alloc::Counts::default());assert_eq!(rt.decks[0].grid,Some(original));
        rt.decks[0].grid=Some(changed);assert_eq!(test_alloc::measure(||rt.process(&mut [0.0;256])),test_alloc::Counts::default());assert_eq!(rt.decks[0].grid,Some(original));
        receipt.set_grid_protection(false,None);rt.apply(Command::Undo);assert_eq!(rt.decks[0].grid,None);rt.apply(Command::Redo);assert_eq!(rt.decks[0].grid,Some(original));
    }

    #[test]
    fn grid_edit_reset_undo_and_reload_preserve_absolute_cues_without_callback_heap_traffic() {
        let (engine, mut rt) = Engine::headless_for_test(48_000, 144);
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
        let (engine, mut rt) = Engine::headless_for_test(48_000, 144);
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
        let (engine, mut rt) = Engine::headless_for_test(48_000, 144);
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

#[cfg(test)]
mod variable_tests;
