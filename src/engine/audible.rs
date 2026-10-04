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
    positions: [AtomicU64; DECKS],
    keys: [AtomicU64; DECKS],
}
struct Shared {
    origin: Instant,
    frames: Box<[Frame]>,
    sequence: AtomicU64,
    known: AtomicBool,
    start: AtomicU64,
    end: AtomicU64,
    valid_from: AtomicU64,
    playback_ns: AtomicU64,
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
                start: AtomicU64::new(0),
                end: AtomicU64::new(0),
                valid_from: AtomicU64::new(0),
                playback_ns: AtomicU64::new(0),
                rate: AtomicU64::new(0),
            })),
            cursor: 0,
            start: 0,
            valid_from: 0,
            rate: 0,
            playback_ns: None,
            active: false,
        }
    }
    /// Share the prepared output history.
    /// Takes this writer; returns a reader with the same monotonic clock and storage.
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
        self.invalidate();
    }

    /// Start one output block.
    /// Takes output rate and the first frame's predicted playback time; returns no value.
    pub fn begin(&mut self, rate: u32, playback_ns: Option<u64>) {
        self.handle.0.sequence.fetch_add(1, Ordering::AcqRel);
        let frames = self.cursor - self.start;
        let duration = (u128::from(frames) * 1_000_000_000 / u128::from(self.rate.max(1))) as u64;
        let discontinuous = self
            .playback_ns
            .zip(playback_ns)
            .is_some_and(|(previous, next)| {
                previous
                    .checked_add(duration)
                    .is_none_or(|expected| expected.abs_diff(next) > duration.max(1))
            });
        if self.rate != rate || self.playback_ns.is_none() || discontinuous {
            self.valid_from = self.cursor;
        }
        self.rate = rate;
        self.start = self.cursor;
        self.playback_ns = playback_ns;
        self.active = true;
    }
    /// Retain the source positions used for one rendered output frame.
    /// Takes live deck states after rendering; returns without work outside an output block.
    pub fn push(&mut self, decks: &[DeckRt; DECKS]) {
        if !self.active {
            return;
        }
        let frame = &self.handle.0.frames[self.cursor as usize % CAPACITY];
        for (index, deck) in decks.iter().enumerate() {
            frame.positions[index].store(deck.pos.to_bits(), Ordering::Relaxed);
            frame.keys[index].store(deck.history_key, Ordering::Relaxed);
        }
        self.cursor += 1;
    }
    /// Publish one completed block.
    /// Takes this writer; returns after making its complete timing and position range visible.
    pub fn finish(&mut self) {
        let shared = &self.handle.0;
        shared.start.store(self.start, Ordering::Relaxed);
        shared.end.store(self.cursor, Ordering::Relaxed);
        shared.valid_from.store(self.valid_from, Ordering::Relaxed);
        shared.rate.store(u64::from(self.rate), Ordering::Relaxed);
        shared
            .playback_ns
            .store(self.playback_ns.unwrap_or(0), Ordering::Relaxed);
        shared
            .known
            .store(self.playback_ns.is_some(), Ordering::Relaxed);
        self.active = false;
        shared.sequence.fetch_add(1, Ordering::Release);
    }
    /// Mark output scheduling unavailable.
    /// Takes this writer; returns with readers refusing the retained estimate.
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
        let start = shared.start.load(Ordering::Relaxed);
        let end = shared.end.load(Ordering::Relaxed);
        let playback = shared.playback_ns.load(Ordering::Relaxed);
        let frame = if now_ns >= playback {
            start.checked_add(
                ((u128::from(now_ns - playback) * u128::from(rate)) / 1_000_000_000)
                    .try_into()
                    .ok()?,
            )?
        } else {
            start.checked_sub(
                (u128::from(playback - now_ns) * u128::from(rate))
                    .div_ceil(1_000_000_000)
                    .try_into()
                    .ok()?,
            )?
        };
        if frame < shared.valid_from.load(Ordering::Relaxed)
            || frame >= end
            || end - frame > CAPACITY as u64
        {
            return None;
        }
        let stored = &shared.frames[frame as usize % CAPACITY];
        let positions = std::array::from_fn(|index| Position {
            source_frame: f64::from_bits(stored.positions[index].load(Ordering::Relaxed)),
            media_key: stored.keys[index].load(Ordering::Relaxed),
            output_frame: frame,
        });
        std::sync::atomic::fence(Ordering::Acquire);
        (sequence == shared.sequence.load(Ordering::Relaxed)
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
    fn queued_frames_remain_readable_while_unknown_busy_expired_and_changed_rate_timing_is_refused()
    {
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
        assert!(handle.positions_at(1_050_000_000).is_none());
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
}
