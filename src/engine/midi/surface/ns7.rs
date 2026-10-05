use crate::engine::{
    deck_controls::{Button, Control},
    Command, CommandPort,
};
use std::time::Instant;

#[derive(Default)]
pub(super) struct Decoder {
    pub panel: u8,
}
impl Decoder {
    /// Decode the original NS7's performance controls.
    /// Takes one complete channel message, its owner and command port; returns whether this decoder owns it.
    pub fn input(
        &mut self,
        message: [u8; 3],
        source: u64,
        cmd: &CommandPort,
        _at: Instant,
    ) -> bool {
        let [status, address, value] = message;
        let on = status == 0x90 && value > 0;
        if status == 0xb0 {
            for (deck, strip, start, stop) in [(0, 0x45, 0x46, 0x47), (1, 0x4d, 0x4e, 0x4f)] {
                let value = f32::from(value) / 127.0;
                let control = if address == strip {
                    Control::Strip { value }
                } else if address == start {
                    Control::StartTime { value }
                } else if address == stop {
                    Control::StopTime { value }
                } else {
                    continue;
                };
                let _ = cmd.send(Command::DeckControl {
                    source,
                    deck,
                    control,
                });
                return true;
            }
            if address == 0x44 {
                let steps = if value < 64 {
                    i16::from(value)
                } else {
                    i16::from(value) - 128
                };
                if steps != 0 {
                    let _ = cmd.send(if self.panel == 1 {
                        Command::BrowseCrates(f32::from(steps))
                    } else {
                        Command::Browse(f32::from(steps))
                    });
                }
                return true;
            }
        }
        if !matches!(status, 0x90 | 0x80) {
            return false;
        }
        match address {
            1 => {
                let _ = cmd.send(Command::MeterMaster(on));
                return true;
            }
            2 | 3 => {
                let _ = cmd.send(Command::FaderStart {
                    deck: address - 2,
                    on,
                });
                return true;
            }
            4 => {
                let _ = cmd.send(Command::XfaderReverse(on));
                return true;
            }
            6 | 7 | 8 | 9 | 10 => {
                if on {
                    self.panel = match address {
                        8 => 1,
                        9 => 2,
                        10 => 0,
                        6 => (self.panel + 2) % 3,
                        _ => (self.panel + 1) % 3,
                    };
                    let _ = cmd.send(Command::BrowsePanel(self.panel));
                }
                return true;
            }
            0x0d => {
                if on {
                    let _ = cmd.send(Command::PrepareSelected { all: false });
                }
                return true;
            }
            _ => {}
        }
        for deck in 0..2 {
            let offset = deck * 0x21;
            let Some(note) = address.checked_sub(offset) else {
                continue;
            };
            let held = match note {
                0x10 => Some(Button::Cue),
                0x12 => Some(Button::Delete),
                0x13..=0x17 => Some(Button::HotCue(note - 0x13)),
                0x18 => Some(Button::BendDown),
                0x19 => Some(Button::BendUp),
                0x1c => Some(Button::Bleep),
                0x1d => Some(Button::Reverse),
                _ => None,
            };
            if let Some(button) = held {
                let _ = cmd.send(Command::DeckControl {
                    source,
                    deck,
                    control: Control::Hold { button, on },
                });
                return true;
            }
            let control = match note {
                0x1a => Control::PitchRange,
                0x1b => Control::Keylock,
                0x1e => Control::Tap,
                0x22 => Control::LoopScale { double: false },
                0x23 => Control::LoopScale { double: true },
                0x24 => Control::LoopToggle,
                0x25 => Control::LoopShift { forward: false },
                0x26 => Control::LoopShift { forward: true },
                0x27 => Control::LoopMode,
                0x28..=0x2b => Control::LoopButton { index: note - 0x28 },
                0x1f | 0x20 => {
                    if on {
                        let _ = cmd.send(Command::DeckTrack {
                            deck,
                            forward: note == 0x20,
                        });
                    }
                    return true;
                }
                _ => continue,
            };
            if on {
                let _ = cmd.send(Command::DeckControl {
                    source,
                    deck,
                    control,
                });
            }
            return true;
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{Engine, Sample};
    use std::sync::Arc;

    #[test]
    fn ns7_controls_wire_decoder_reaches_production_playback_for_both_decks() {
        let (engine, mut rt) = Engine::headless_for_test(48000, 256);
        for deck in 0..2 {
            rt.apply(Command::DeckAudio {
                deck,
                audio: Arc::new(Sample {
                    name: "NS7 controls".into(),
                    sr: 48000,
                    ch: 2,
                    data: vec![0.25; 96000],
                    peaks: Vec::new().into(),
                    bpm: 120.0,
                    path: String::new(),
                }),
            });
        }
        let mut decoder = Decoder::default();
        for deck in 0..2 {
            let offset = deck * 0x21;
            for (note, button) in [
                (0x1c, Button::Bleep),
                (0x1d, Button::Reverse),
                (0x18, Button::BendDown),
                (0x19, Button::BendUp),
            ] {
                assert!(decoder.input([0x90, note + offset, 127], 42, &engine.cmd, Instant::now()));
                rt.process(&mut [0.0; 256]);
                assert!(rt.decks[usize::from(deck)].controls.held(button));
                assert!(decoder.input([0x90, note + offset, 0], 42, &engine.cmd, Instant::now()));
                rt.process(&mut [0.0; 256]);
                assert!(!rt.decks[usize::from(deck)].controls.held(button));
                decoder.input([0x90, note + offset, 127], 42, &engine.cmd, Instant::now());
                decoder.input([0x80, note + offset, 64], 42, &engine.cmd, Instant::now());
                rt.process(&mut [0.0; 256]);
                assert!(!rt.decks[usize::from(deck)].controls.held(button));
            }
            decoder.input([0x90, 0x1b + offset, 127], 42, &engine.cmd, Instant::now());
            decoder.input([0x80, 0x1b + offset, 0], 42, &engine.cmd, Instant::now());
            rt.process(&mut [0.0; 256]);
            assert!(rt.decks[usize::from(deck)].keylock);
            for (press, expected) in [(0x1a, 1), (0x1a, 2), (0x1a, 0)] {
                decoder.input([0x90, press + offset, 127], 42, &engine.cmd, Instant::now());
                rt.process(&mut [0.0; 256]);
                assert_eq!(rt.decks[usize::from(deck)].pitch_range, expected);
            }
            for (cc, value) in [
                ([0x45, 0x4d][usize::from(deck)], 64),
                ([0x46, 0x4e][usize::from(deck)], 32),
                ([0x47, 0x4f][usize::from(deck)], 16),
            ] {
                decoder.input([0xb0, cc, value], 42, &engine.cmd, Instant::now());
            }
            rt.process(&mut [0.0; 256]);
            let d = &rt.decks[usize::from(deck)];
            assert!((d.pos / 47999.0 - 64.0 / 127.0).abs() < 0.0001);
            assert_eq!(d.controls.status().start_seconds, 32.0 / 127.0 * 4.0);
            assert_eq!(d.controls.status().stop_seconds, 16.0 / 127.0 * 4.0);
        }
        for note in [1, 2, 3, 4] {
            decoder.input([0x90, note, 127], 42, &engine.cmd, Instant::now());
        }
        rt.process(&mut [0.0; 256]);
        assert!(rt.meter_master && rt.xfader_reverse && rt.fader_start.iter().all(|on| *on));
        for note in [1, 2, 3, 4] {
            decoder.input([0x80, note, 0], 42, &engine.cmd, Instant::now());
        }
        rt.process(&mut [0.0; 256]);
        assert!(!rt.meter_master && !rt.xfader_reverse && rt.fader_start.iter().all(|on| !*on));
        assert!(!decoder.input([0x91, 0x1b, 127], 42, &engine.cmd, Instant::now()));
    }
}
