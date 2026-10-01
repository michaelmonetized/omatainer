//! Media-relative preparation. Seconds refer to decoded source frames, never
//! output rate or current playback pitch. Loading restores markers, not Play.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Preparation {
    pub cue: f64,
    pub hotcues: [Option<f64>; super::HOTCUES],
    pub loop_region: Option<Loop>,
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Loop {
    pub start: f64,
    pub length: f64,
    pub enabled: bool,
}
impl Preparation {
    pub fn valid(self) -> bool {
        let position = |v: f64| v.is_finite() && (0.0..=1.0e10).contains(&v);
        position(self.cue)
            && self.hotcues.into_iter().flatten().all(position)
            && self
                .loop_region
                .is_none_or(|r| position(r.start) && position(r.length) && r.length > 0.0)
    }
    pub(super) fn words(self) -> [u64; 12] {
        let mut words = [0; 12];
        words[0] = self.cue.to_bits();
        for (word, cue) in words[1..9].iter_mut().zip(self.hotcues) {
            *word = cue.unwrap_or(-1.0).to_bits();
        }
        words[9] = self.loop_region.map_or(-1.0, |r| r.start).to_bits();
        words[10] = self.loop_region.map_or(0.0, |r| r.length).to_bits();
        words[11] = self.loop_region.is_some_and(|r| r.enabled) as u64;
        words
    }
    pub(super) fn from_words(words: [u64; 12]) -> Self {
        let value = |i| f64::from_bits(words[i]);
        Self {
            cue: value(0),
            hotcues: std::array::from_fn(|i| (value(i + 1) >= 0.0).then(|| value(i + 1))),
            loop_region: (value(9) >= 0.0).then(|| Loop {
                start: value(9),
                length: value(10),
                enabled: words[11] != 0,
            }),
        }
    }
}

impl super::DeckRt {
    pub(super) fn preparation(&self) -> Option<Preparation> {
        let audio = self.audio.as_ref()?;
        let sr = audio.sr as f64;
        if sr <= 0.0 {
            return None;
        }
        let end = audio.frames() as f64;
        let pos = |p: f64| p.clamp(0.0, end) / sr;
        Some(Preparation {
            cue: pos(self.cue_pos),
            hotcues: std::array::from_fn(|i| self.hotcues[i].set.then(|| pos(self.hotcues[i].pos))),
            loop_region: (self.loop_len > 0.0 && self.loop_start < end).then(|| Loop {
                start: pos(self.loop_start),
                length: self.loop_len.min(end - self.loop_start.max(0.0)) / sr,
                enabled: self.loop_on,
            }),
        })
    }
    pub(super) fn publish_preparation(&self) {
        if let (Some(receipt), Some(preparation)) = (&self.load_receipt, self.preparation()) {
            receipt.record_preparation(preparation);
        }
    }
    pub(super) fn restore_preparation(&mut self, preparation: Preparation) {
        if !preparation.valid() {
            return;
        }
        let Some(audio) = &self.audio else { return };
        let frames = audio.frames() as f64;
        let sr = audio.sr as f64;
        self.cue_pos = (preparation.cue * sr).clamp(0.0, frames);
        for (cue, saved) in self.hotcues.iter_mut().zip(preparation.hotcues) {
            cue.set = saved.is_some_and(|p| p * sr <= frames);
            cue.pos = saved.unwrap_or(0.0) * sr;
        }
        if let Some(region) = preparation
            .loop_region
            .filter(|r| (r.start + r.length) * sr <= frames)
        {
            self.loop_start = region.start * sr;
            self.loop_len = region.length * sr;
            self.loop_on = region.enabled;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{load_receipt::Receipt, test_alloc, Command, Engine};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    #[test]
    fn bounded_receipt_publication_is_coherent_and_renderer_commands_allocate_nothing() {
        let (engine, mut rt) = Engine::headless_for_test(48_000, 64);
        let receipt = engine.initial_playback[0].clone();
        let counts = test_alloc::measure(|| {
            for i in 0..128 {
                rt.apply(Command::DeckSeek {
                    deck: 0,
                    frac: i as f32 / 128.0,
                });
                rt.apply(Command::DeckHotCue {
                    deck: 0,
                    pad: (i % 8) as u8,
                    del: i % 2 == 0,
                });
                rt.apply(Command::DeckLoop {
                    deck: 0,
                    beats: 0.5,
                });
            }
        });
        assert_eq!(counts, test_alloc::Counts::default());
        assert!(receipt.preparation().unwrap().1.valid());
        let shared = Receipt::new();
        let writing = shared.clone();
        let done = Arc::new(AtomicBool::new(false));
        let finished = done.clone();
        let worker = std::thread::spawn(move || {
            for i in 1..50_000 {
                writing.record_preparation(Preparation {
                    cue: i as f64,
                    hotcues: [Some(i as f64); 8],
                    loop_region: Some(Loop {
                        start: i as f64,
                        length: i as f64,
                        enabled: true,
                    }),
                });
            }
            finished.store(true, Ordering::Release);
        });
        while !done.load(Ordering::Acquire) {
            if let Some((revision, value)) = shared.preparation() {
                assert_eq!(revision & 1, 0);
                assert!(value.hotcues.into_iter().all(|cue| cue == Some(value.cue)));
                assert_eq!(value.loop_region.unwrap().length, value.cue);
            }
        }
        worker.join().unwrap();
        assert_eq!(shared.preparation().unwrap().1.cue, 49_999.0);
    }
}
