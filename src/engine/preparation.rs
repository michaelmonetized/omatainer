//! Media-relative preparation. Seconds refer to decoded source frames, never
//! output rate or current playback pitch. Loading restores markers, not Play.
use serde::{Deserialize, Serialize};
use super::cue_metadata::{Style, STYLE_WORDS};
const GRID_OFFSET: usize = 12 + super::HOTCUES * STYLE_WORDS;
pub(super) const WORDS: usize = GRID_OFFSET + super::beatgrid::WORDS;

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Preparation {
    pub cue: f64,
    #[serde(default)]
    pub grid: Option<super::beatgrid::Grid>,
    pub hotcues: [Option<f64>; super::HOTCUES],
    #[serde(default)]
    pub hotcue_styles: [Style; super::HOTCUES],
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
    pub(super) fn words(self) -> [u64; WORDS] {
        let mut words = [0; WORDS];
        words[0] = self.cue.to_bits();
        for (word, cue) in words[1..9].iter_mut().zip(self.hotcues) {
            *word = cue.unwrap_or(-1.0).to_bits();
        }
        words[9] = self.loop_region.map_or(-1.0, |r| r.start).to_bits();
        words[10] = self.loop_region.map_or(0.0, |r| r.length).to_bits();
        words[11] = self.loop_region.is_some_and(|r| r.enabled) as u64;
        for (dest, style) in words[12..].chunks_exact_mut(STYLE_WORDS).zip(self.hotcue_styles) {
            dest.copy_from_slice(&style.words());
        }
        words[GRID_OFFSET..].copy_from_slice(&super::beatgrid::Grid::encode(self.grid));
        words
    }
    pub(super) fn from_words(words: [u64; WORDS]) -> Self {
        let value = |i| f64::from_bits(words[i]);
        Self {
            cue: value(0),
            grid: super::beatgrid::Grid::decode(words[GRID_OFFSET..].try_into().unwrap()).flatten(),
            hotcues: std::array::from_fn(|i| (value(i + 1) >= 0.0).then(|| value(i + 1))),
            hotcue_styles: std::array::from_fn(|i| Style::from_words(words[12+i*STYLE_WORDS..12+(i+1)*STYLE_WORDS].try_into().unwrap()).unwrap_or_default()),
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
            grid: self.grid,
            hotcue_styles: self.cue_styles,
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
        self.cue_styles = preparation.hotcue_styles;
        self.grid = preparation.grid;
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
        let (engine, mut rt) = Engine::headless_for_test(48_000, 80);
        let receipt = engine.initial_playback[0].clone().unwrap();
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
                    grid: Some(super::super::beatgrid::Grid::new(i as f64, 120.0).unwrap()),
                    hotcues: [Some(i as f64); 8],
                    hotcue_styles: [Style { name: super::super::cue_metadata::Name::new(&format!("{i}")).unwrap(), color: Some([(i % 256) as u8, 23, 42]) }; super::super::HOTCUES],
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
                assert_eq!(value.grid.unwrap().downbeat(), value.cue);
                for style in value.hotcue_styles {
                    assert_eq!(style.name.as_str(), format!("{}", value.cue as u64));
                    assert_eq!(style.color, Some([(value.cue as u64 % 256) as u8, 23, 42]));
                }
            }
        }
        worker.join().unwrap();
        assert_eq!(shared.preparation().unwrap().1.cue, 49_999.0);
    }
}

#[cfg(test)]
mod cue_style_tests {
    use super::*;
    use crate::engine::{cue_metadata::Name, load_receipt::Receipt, test_alloc, Command, Engine};

    #[test]
    fn all_eight_cue_styles_roundtrip_receipts_history_and_delete_without_renderer_heap_work() {
        let (engine, mut rt) = Engine::headless_for_test(48_000, 144);
        let receipt = engine.initial_playback[0].clone().unwrap();
        for pad in 0..8 {
            rt.decks[0].pos = (pad + 1) as f64 * 128.0;
            rt.apply(Command::DeckHotCue { deck: 0, pad, del: false });
        }
        rt.clear_undo_for_test();
        let styles: [Style; 8] = std::array::from_fn(|i| Style {
            name: Name::new(&format!("Cue {} • 演奏", i + 1)).unwrap(),
            color: Some([i as u8 * 31, 220, 128]),
        });
        let commands: [Command; 8] = std::array::from_fn(|i| Command::DeckCueStyle {
            deck: 0, pad: i as u8, style: styles[i], receipt: receipt.clone(),
        });
        let counts = test_alloc::measure(|| {
            for command in commands { rt.apply(command); }
        });
        assert_eq!(counts, test_alloc::Counts::default());
        assert_eq!(rt.decks[0].cue_styles, styles);
        assert_eq!(receipt.preparation().unwrap().1.hotcue_styles, styles);
        let positions = receipt.preparation().unwrap().1.hotcues;
        let counts = test_alloc::measure(|| {
            for _ in 0..8 { rt.apply(Command::Undo); }
        });
        assert_eq!(counts, test_alloc::Counts::default());
        assert_eq!(rt.decks[0].cue_styles, [Style::default(); 8]);
        assert_eq!(receipt.preparation().unwrap().1.hotcue_styles, [Style::default(); 8]);
        assert_eq!(receipt.preparation().unwrap().1.hotcues, positions);
        for _ in 0..8 { rt.apply(Command::Redo); }
        assert_eq!(rt.decks[0].cue_styles, styles);
        rt.apply(Command::DeckHotCue { deck: 0, pad: 3, del: true });
        assert_eq!(rt.decks[0].cue_styles[3], Style::default());
        assert!(!rt.decks[0].hotcues[3].set);
        rt.apply(Command::Undo);
        assert_eq!(rt.decks[0].cue_styles[3], styles[3]);
        assert!(rt.decks[0].hotcues[3].set);
        let saved = receipt.preparation().unwrap().1;
        let reload = Receipt::with_preparation(Some(saved));
        rt.apply(Command::DeckLoadRequested {
            deck: 1, media: crate::engine::load_receipt::Media::Builtin(0), receipt: reload.clone(),
        });
        assert_eq!(rt.decks[1].cue_styles, styles);
        assert_eq!(reload.preparation().unwrap().1, saved);
        assert!(!rt.decks[1].playing);
    }

    #[test]
    fn cue_style_noop_invalid_empty_and_stale_identity_never_create_edits() {
        let (engine, mut rt) = Engine::headless_for_test(48_000, 144);
        let receipt = engine.initial_playback[0].clone().unwrap();
        rt.apply(Command::DeckHotCue { deck: 0, pad: 0, del: false });
        rt.clear_undo_for_test();
        let before = rt.undo.checkpoint();
        let style = Style { name: Name::new("Drop").unwrap(), color: Some([1,2,3]) };
        let commands = [
            Command::DeckCueStyle { deck: 0, pad: 0, style: Style::default(), receipt: receipt.clone() },
            Command::DeckCueStyle { deck: 255, pad: 0, style, receipt: receipt.clone() },
            Command::DeckCueStyle { deck: 0, pad: 255, style, receipt: receipt.clone() },
            Command::DeckCueStyle { deck: 0, pad: 7, style, receipt: receipt.clone() },
            // No other owner retains this receipt: rejection must retire it.
            Command::DeckCueStyle { deck: 0, pad: 0, style, receipt: Receipt::new() },
        ];
        let counts = test_alloc::measure(|| {
            for command in commands { rt.apply(command); }
        });
        assert_eq!(counts, test_alloc::Counts::default());
        assert_eq!(rt.undo.checkpoint(), before);
        assert_eq!(rt.decks[0].cue_styles, [Style::default(); 8]);
        let replacement = Receipt::new();
        rt.apply(Command::DeckLoadRequested { deck: 0,
            media: crate::engine::load_receipt::Media::Builtin(1), receipt: replacement });
        rt.apply(Command::DeckHotCue { deck: 0, pad: 0, del: false });
        let before = rt.undo.checkpoint();
        let stale = Command::DeckCueStyle { deck: 0, pad: 0, style, receipt };
        assert_eq!(test_alloc::measure(|| rt.apply(stale)), test_alloc::Counts::default());
        assert_eq!(rt.undo.checkpoint(), before);
        assert_eq!(rt.decks[0].cue_styles, [Style::default(); 8]);
    }
}

#[cfg(test)]
mod cue_editor_identity_tests {
    use crate::engine::{load_receipt::{Media, Receipt}, test_alloc, Command, Engine};
    #[test]
    fn queued_cue_point_editor_actions_cannot_target_replacement_media() {
        let (engine, mut rt) = Engine::headless_for_test(48_000, 48);
        let original = engine.initial_playback[0].clone().unwrap();
        rt.apply(Command::DeckCuePoint { deck: 0, pad: 3, del: false, receipt: original.clone() });
        assert!(rt.decks[0].hotcues[3].set);
        rt.apply(Command::Undo);
        assert!(!rt.decks[0].hotcues[3].set);
        rt.apply(Command::Redo);
        assert!(rt.decks[0].hotcues[3].set);
        let replacement = Receipt::new();
        rt.apply(Command::DeckLoadRequested { deck: 0, media: Media::Builtin(1), receipt: replacement.clone() });
        rt.apply(Command::DeckHotCue { deck: 0, pad: 2, del: false });
        let checkpoint = engine.undo.checkpoint();
        let commands = [
            Command::DeckCuePoint { deck: 0, pad: 3, del: false, receipt: original.clone() },
            Command::DeckCuePoint { deck: 0, pad: 2, del: true, receipt: original },
            Command::DeckCuePoint { deck: 255, pad: 2, del: true, receipt: replacement.clone() },
            Command::DeckCuePoint { deck: 0, pad: 255, del: true, receipt: replacement },
            Command::DeckCuePoint { deck: 0, pad: 2, del: true, receipt: Receipt::new() },
        ];
        assert_eq!(test_alloc::measure(|| { for command in commands { rt.apply(command); } }), test_alloc::Counts::default());
        assert_eq!(engine.undo.checkpoint(), checkpoint);
        assert!(!rt.decks[0].hotcues[3].set);
        assert!(rt.decks[0].hotcues[2].set);
    }
}
