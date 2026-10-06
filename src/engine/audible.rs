//! Bounded source positions for output frames whose backend playback time is known.
use super::{DeckRt, DECKS};
use std::{
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
    time::Instant,
};

const CAPACITY: usize = 65536;

#[derive(Default)]
struct Frame {
    index: AtomicU64,
    positions: [AtomicU64; DECKS],
    keys: [AtomicU64; DECKS],
    playback_ns: AtomicU64,
}
struct Shared {
    origin: Instant,
    frames: Box<[Frame]>,
    sequence: AtomicU64,
    known: AtomicBool,
    end: AtomicU64,
    valid_from: AtomicU64,
    rate: AtomicU64,
}
#[derive(Clone)]
pub(crate) struct Handle(Arc<Shared>);
#[derive(Clone, Copy, Debug)]
pub(crate) struct Position {
    pub source_frame: f64,
    pub media_key: u64,
    pub output_frame: u64,
}
pub(super) struct Writer {
    handle: Handle,
    cursor: u64,
    start: u64,
    valid_from: u64,
    rate: u32,
    playback_ns: Option<u64>,
    active: bool,
    last_ns: Option<u64>,
}
impl Writer {
    /// Prepare output position storage.
    /// Takes no inputs; returns one writer and fixed storage shared with UI readers.
    pub fn new() -> Self {
        Self {
            handle: Handle(Arc::new(Shared {
                origin: Instant::now(),
                frames: (0..CAPACITY).map(|_| Frame::default()).collect(),
                sequence: AtomicU64::new(0),
                known: AtomicBool::new(false),
                end: AtomicU64::new(0),
                valid_from: AtomicU64::new(0),
                rate: AtomicU64::new(0),
            })),
            cursor: 0,
            start: 0,
            valid_from: 0,
            rate: 0,
            playback_ns: None,
            active: false,
            last_ns: None,
        }
    }
    pub fn handle(&self) -> Handle {
        self.handle.clone()
    }
    pub fn now_ns(&self) -> u64 {
        self.handle.now_ns()
    }
    /// Retire timing from a stopped output stream.
    /// Takes this writer; returns with old queued frames unavailable to a replacement stream.
    pub fn restart(&mut self) {
        self.valid_from = self.cursor;
        self.rate = 0;
        self.playback_ns = None;
        self.last_ns = None;
        self.invalidate();
    }

    /// Start one output block.
    /// Takes output rate and the first frame's predicted playback time; returns no value.
    pub fn begin(&mut self, rate: u32, playback_ns: Option<u64>) {
        let overlapping = playback_ns
            .zip(self.last_ns)
            .is_some_and(|(next, last)| next <= last);
        if self.rate != rate || self.playback_ns.is_none() || overlapping {
            self.valid_from = self.cursor;
        }
        self.rate = rate;
        self.start = self.cursor;
        self.playback_ns = playback_ns.filter(|_| rate > 0);
        self.active = self.playback_ns.is_some();
    }
    /// Retain the source positions used for one rendered output frame.
    /// Takes live deck states after rendering; returns without work outside an output block.
    pub fn push(&mut self, decks: &[DeckRt; DECKS]) {
        if !self.active {
            return;
        }
        let frame = &self.handle.0.frames[self.cursor as usize % CAPACITY];
        frame.index.swap(0, Ordering::AcqRel);
        let timestamp = self.playback_ns.and_then(|start| {
            start.checked_add(
                (self.cursor - self.start).checked_mul(1_000_000_000)?
                    / u64::from(self.rate.max(1)),
            )
        });
        frame
            .playback_ns
            .store(timestamp.unwrap_or(0), Ordering::Relaxed);
        self.last_ns = timestamp;
        if timestamp.is_none() {
            self.playback_ns = None;
        }
        for (index, deck) in decks.iter().enumerate() {
            frame.positions[index].store(deck.pos.to_bits(), Ordering::Relaxed);
            let known_source = deck.transition_remaining == 0
                && deck.keylock_mode() != super::keylock::Mode::Locked;
            frame.keys[index].store(
                if known_source { deck.history_key } else { 0 },
                Ordering::Relaxed,
            );
        }
        frame.index.store(self.cursor + 1, Ordering::Release);
        self.cursor += 1;
    }
    /// Publish one completed block.
    /// Takes this writer; returns after making its complete timing and position range visible.
    pub fn finish(&mut self) {
        let shared = &self.handle.0;
        shared.sequence.fetch_add(1, Ordering::AcqRel);
        shared.end.store(self.cursor, Ordering::Relaxed);
        shared.valid_from.store(self.valid_from, Ordering::Relaxed);
        shared.rate.store(u64::from(self.rate), Ordering::Relaxed);
        shared
            .known
            .store(self.playback_ns.is_some(), Ordering::Relaxed);
        self.active = false;
        shared.sequence.fetch_add(1, Ordering::Release);
    }
    pub fn invalidate(&self) {
        self.handle.0.known.store(false, Ordering::Release);
    }
}
impl Handle {
    /// Read the clock shared by the renderer and UI.
    /// Takes this handle; returns monotonic nanoseconds since its setup.
    pub fn now_ns(&self) -> u64 {
        self.0.origin.elapsed().as_nanos().min(u128::from(u64::MAX)) as u64
    }

    /// Resolve the output frame predicted to play at a chosen clock time.
    /// Takes monotonic nanoseconds; returns coherent positions or unavailable timing, stale data or an overwritten frame.
    pub fn positions_at(&self, now_ns: u64) -> Option<[Position; DECKS]> {
        let shared = &self.0;
        let sequence = shared.sequence.load(Ordering::Acquire);
        if sequence == 0 || sequence & 1 != 0 || !shared.known.load(Ordering::Acquire) {
            return None;
        }
        let rate = shared.rate.load(Ordering::Relaxed);
        if rate == 0 {
            return None;
        }
        let end = shared.end.load(Ordering::Relaxed);
        let first = shared
            .valid_from
            .load(Ordering::Relaxed)
            .max(end.saturating_sub(CAPACITY as u64));
        let timestamp = |index: u64| {
            let frame = &shared.frames[index as usize % CAPACITY];
            if frame.index.load(Ordering::Acquire) != index + 1 {
                return None;
            }
            let ns = frame.playback_ns.load(Ordering::Relaxed);
            std::sync::atomic::fence(Ordering::Acquire);
            (frame.index.load(Ordering::Relaxed) == index + 1).then_some(ns)
        };
        let mut low = first;
        let mut high = end;
        while low < high {
            let middle = low + (high - low) / 2;
            if timestamp(middle)? <= now_ns {
                low = middle + 1;
            } else {
                high = middle;
            }
        }
        let frame = low.checked_sub(1).filter(|frame| *frame >= first)?;
        if now_ns.checked_sub(timestamp(frame)?)? >= 1_000_000_000u64.div_ceil(rate) {
            return None;
        }
        let stored = &shared.frames[frame as usize % CAPACITY];
        if stored.index.load(Ordering::Acquire) != frame + 1 {
            return None;
        }
        let positions = std::array::from_fn(|index| Position {
            source_frame: f64::from_bits(stored.positions[index].load(Ordering::Relaxed)),
            media_key: stored.keys[index].load(Ordering::Relaxed),
            output_frame: frame,
        });
        std::sync::atomic::fence(Ordering::Acquire);
        (stored.index.load(Ordering::Relaxed) == frame + 1
            && sequence == shared.sequence.load(Ordering::Relaxed)
            && shared.known.load(Ordering::Acquire))
        .then_some(positions)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::test_alloc;

    #[test]
    fn output_positions_follow_exact_rendered_motion_through_loops_reverse_and_media_changes() {
        let mut writer = Writer::new();
        let handle = writer.handle();
        let mut decks = [DeckRt::new(48000.0), DeckRt::new(48000.0)];
        decks[0].history_key = 9;
        decks[1].history_key = 10;
        assert_eq!(
            test_alloc::measure(|| {
                writer.begin(1000, Some(1_000_000_000));
                for frame in 0..100 {
                    decks[0].pos = (frame % 20) as f64;
                    decks[1].pos = 200.0 - frame as f64;
                    if frame == 50 {
                        decks[0].history_key = 11;
                    }
                    writer.push(&decks);
                }
                writer.finish();
            }),
            test_alloc::Counts::default()
        );
        for frame in 0..100 {
            let positions = handle
                .positions_at(1_000_000_000 + frame * 1_000_000)
                .unwrap();
            assert_eq!(positions[0].source_frame, (frame % 20) as f64);
            assert_eq!(positions[1].source_frame, 200.0 - frame as f64);
            assert_eq!(positions[0].media_key, if frame < 50 { 9 } else { 11 });
            assert_eq!(positions[0].output_frame, frame);
        }
        assert!(handle.positions_at(999_999_999).is_none());
        assert!(handle.positions_at(1_100_000_000).is_none());
    }

    #[test]
    fn queued_frames_remain_readable_during_later_blocks_and_refuse_unknown_expired_or_changed_rate_timing(
    ) {
        let mut writer = Writer::new();
        let handle = writer.handle();
        let mut decks = [DeckRt::new(48000.0), DeckRt::new(48000.0)];
        writer.begin(1000, Some(1_000_000_000));
        for frame in 0..100 {
            decks[0].pos = frame as f64;
            writer.push(&decks);
        }
        writer.finish();
        writer.begin(1000, Some(1_100_000_000));
        assert_eq!(
            handle.positions_at(1_050_000_000).unwrap()[0].source_frame,
            50.0
        );
        for frame in 100..200 {
            decks[0].pos = frame as f64;
            writer.push(&decks);
        }
        writer.finish();
        assert_eq!(
            handle.positions_at(1_050_000_000).unwrap()[0].source_frame,
            50.0
        );
        writer.begin(2000, Some(1_200_000_000));
        writer.push(&decks);
        writer.finish();
        assert!(handle.positions_at(1_199_999_999).is_none());
        assert!(handle.positions_at(1_200_500_000).is_none());
        writer.begin(2000, None);
        writer.push(&decks);
        writer.finish();
        assert!(handle.positions_at(1_200_000_000).is_none());
    }

    #[test]
    fn overwritten_history_cannot_be_mistaken_for_an_older_output_frame() {
        let mut writer = Writer::new();
        let handle = writer.handle();
        let mut decks = [DeckRt::new(48000.0), DeckRt::new(48000.0)];
        writer.begin(1000, Some(1_000_000_000));
        for frame in 0..CAPACITY + 10 {
            decks[0].pos = frame as f64;
            writer.push(&decks);
        }
        writer.finish();
        assert!(handle.positions_at(1_009_000_000).is_none());
        assert_eq!(
            handle.positions_at(1_010_000_000).unwrap()[0].source_frame,
            10.0
        );
        writer.invalidate();
        assert!(handle.positions_at(1_010_000_000).is_none());
    }

    #[test]
    fn each_block_retains_its_own_timestamps_and_refuses_gaps_or_overlapping_history() {
        let mut writer = Writer::new();
        let handle = writer.handle();
        let mut decks = [DeckRt::new(48000.0), DeckRt::new(48000.0)];
        for (start, first) in [(1_000_000_000, 0), (1_100_500_000, 100)] {
            writer.begin(1000, Some(start));
            for frame in first..first + 100 {
                decks[0].pos = frame as f64;
                writer.push(&decks);
            }
            writer.finish();
        }
        assert_eq!(
            handle.positions_at(1_050_000_000).unwrap()[0].source_frame,
            50.0
        );
        assert!(handle.positions_at(1_100_250_000).is_none());
        assert_eq!(
            handle.positions_at(1_100_500_000).unwrap()[0].source_frame,
            100.0
        );
        writer.begin(1000, Some(1_199_000_000));
        writer.push(&decks);
        writer.finish();
        assert!(handle.positions_at(1_050_000_000).is_none());
        writer.restart();
        assert!(handle.positions_at(1_199_000_000).is_none());
    }

    #[test]
    fn processed_source_mixtures_never_claim_one_audible_source_position() {
        let mut writer = Writer::new();
        let handle = writer.handle();
        let mut decks = [DeckRt::new(48000.0), DeckRt::new(48000.0)];
        decks[0].history_key = 73;
        decks[0].transition_remaining = 4;
        writer.begin(1000, Some(1_000_000_000));
        writer.push(&decks);
        writer.finish();
        assert_eq!(handle.positions_at(1_000_000_000).unwrap()[0].media_key, 0);
        decks[0].transition_remaining = 0;
        writer.begin(1000, Some(1_001_000_000));
        writer.push(&decks);
        writer.finish();
        assert_eq!(handle.positions_at(1_001_000_000).unwrap()[0].media_key, 73);
        decks[0].audio = Some(Arc::new(super::super::Sample { spectrum: None,
            name: "grain mixture".into(),
            path: String::new(),
            sr: 48000,
            ch: 2,
            bpm: 120.0,
            data: vec![0.1; 32],
            peaks: vec![].into(),
        }));
        decks[0].playing = true;
        decks[0].keylock = true;
        decks[0].rate = 0.75;
        writer.begin(1000, Some(1_002_000_000));
        writer.push(&decks);
        writer.finish();
        assert_eq!(handle.positions_at(1_002_000_000).unwrap()[0].media_key, 0);
        decks[0].touching = true;
        writer.begin(1000, Some(1_003_000_000));
        writer.push(&decks);
        writer.finish();
        assert_eq!(handle.positions_at(1_003_000_000).unwrap()[0].media_key, 73);
    }

    #[test]
    fn overwritten_frames_are_refused_before_the_writing_block_is_published() {
        let mut writer = Writer::new();
        let handle = writer.handle();
        let decks = [DeckRt::new(48000.0), DeckRt::new(48000.0)];
        writer.begin(1000, Some(1_000_000_000));
        writer.push(&decks);
        writer.finish();
        writer.begin(1000, Some(1_001_000_000));
        assert!(handle.positions_at(1_000_000_000).is_some());
        for _ in 0..CAPACITY {
            writer.push(&decks);
        }
        assert!(handle.positions_at(1_000_000_000).is_none());
        writer.finish();
    }

    #[test]
    fn concurrent_readers_never_mix_media_identity_and_source_position() {
        let mut writer = Writer::new();
        let handle = writer.handle();
        let progress = Arc::new(AtomicU64::new(0));
        let published = progress.clone();
        let worker = std::thread::spawn(move || {
            let mut decks = [DeckRt::new(48000.0), DeckRt::new(48000.0)];
            for block in 0..1024_u64 {
                writer.begin(1000, Some(1_000_000_000 + block * 128 * 1_000_000));
                for frame in block * 128..(block + 1) * 128 {
                    decks[0].pos = frame as f64;
                    decks[0].history_key = frame + 1;
                    writer.push(&decks);
                }
                writer.finish();
                published.store((block + 1) * 128, Ordering::Release);
            }
        });
        let check = || {
            let frame = progress.load(Ordering::Acquire).saturating_sub(33);
            if let Some(positions) = handle.positions_at(1_000_000_000 + frame * 1_000_000) {
                assert_eq!(positions[0].source_frame, frame as f64);
                assert_eq!(positions[0].media_key, frame + 1);
            }
        };
        while !worker.is_finished() {
            check();
        }
        worker.join().unwrap();
        check();
        assert!(handle
            .positions_at(1_000_000_000 + (1024 * 128 - 33) * 1_000_000)
            .is_some());
    }
}
