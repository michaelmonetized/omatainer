use super::{cbind, nbind, rbind, Action, MidiMap, RelativeEncoding, RelativeSpec, UnmappedNotes};
use crate::engine::{Command, CommandPort};

pub(super) fn pioneer_sp1() -> MidiMap {
    let mut bindings = Vec::new();
    for deck in 0..4 {
        let target = deck % 2;
        bindings.push(nbind(deck, 0x58, Action::DeckSync, target, 0));
        bindings.push(nbind(deck, 0x55, Action::DeckLoop4, target, 0));
        bindings.push(nbind(6, 0x46 + deck, Action::DeckLoad, target, 0));
        for pad in 0..8 {
            bindings.push(nbind(7 + deck, pad, Action::DeckHotCue, target, pad));
        }
        bindings.push(nbind(7 + deck, 0x60, Action::DeckLoopIn, target, 0));
        bindings.push(nbind(7 + deck, 0x61, Action::DeckLoopOut, target, 0));
    }
    bindings.push(nbind(6, 0x65, Action::CrateReturn, 0, 0));
    bindings.push(nbind(6, 0x67, Action::Prepare, 0, 0));
    bindings.push(nbind(6, 0x68, Action::PrepareCrate, 0, 0));
    let browse = RelativeSpec {
        encoding: RelativeEncoding::TwosComplement,
        scale: 1.0,
    };
    bindings.push(rbind(6, 0x40, Action::Browse, 0, 0, browse));
    bindings.push(rbind(6, 0x64, Action::BrowseCrates, 0, 0, browse));
    for channel in 4..6 {
        for slot in 0..3 {
            bindings.push(nbind(channel, 0x47 + slot, Action::FxSelect, 0, slot));
        }
    }
    MidiMap {
        name: "Pioneer DDJ-SP1".into(),
        matchers: vec!["ddj-sp1".into(), "ddj sp1".into()],
        bindings,
        unmapped_notes: UnmappedNotes::Ignore,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ns7_trim_cannot_open_a_closed_fader_or_change_the_other_deck() {
        let map = numark_ns7();
        let mut decoder = Decoder::default();
        let (cmd, received) = CommandPort::channel(32);
        for message in [
            [0xb0, 0x08, 0],
            [0xb0, 0x0c, 127],
            [0xb0, 0x0d, 127],
            [0xb0, 0x11, 64],
            [0xb0, 0x08, 127],
        ] {
            decoder.input(&map, &message, &cmd);
        }
        let values = received
            .try_iter()
            .map(|command| match command {
                Command::DeckGain { deck, value } => (deck, value),
                _ => panic!("unexpected command"),
            })
            .collect::<Vec<_>>();
        assert_eq!(values, [(0, 0.0), (0, 0.0), (1, 1.0), (1, 1.0), (0, 1.5)]);
        decoder.reset();
        decoder.input(&map, &[0xb0, 0x08, 127], &cmd);
        assert!(matches!(
            received.try_iter().next(),
            Some(Command::DeckGain {
                deck: 0,
                value: 1.0
            })
        ));
    }

    #[test]
    fn ns7_wheels_wrap_both_directions_and_reset_without_moving() {
        let map = numark_ns7();
        let mut decoder = Decoder::default();
        let (cmd, received) = CommandPort::channel(32);
        for message in [
            [0xb0, 0, 126],
            [0xb0, 0, 127],
            [0xb0, 0, 0],
            [0xb0, 0, 127],
            [0xb0, 2, 2],
            [0xb0, 2, 1],
        ] {
            assert!(decoder.input(&map, &message, &cmd));
        }
        let commands = received.try_iter().collect::<Vec<_>>();
        assert!(
            matches!(commands.as_slice(), [Command::DeckJog {deck:0,delta:a}, Command::DeckJog {deck:0,delta:b}, Command::DeckJog {deck:0,delta:c}, Command::DeckJog {deck:1,delta:d}] if *a==0.35 && *b==0.35 && *c == -0.35 && *d == -0.35)
        );
        decoder.reset();
        decoder.input(&map, &[0xb0, 0, 20], &cmd);
        assert!(received.try_iter().next().is_none());
    }

    #[test]
    fn sp1_knobs_require_a_complete_pair_and_shifted_pads_delete_the_right_deck() {
        let map = pioneer_sp1();
        let mut decoder = Decoder::default();
        let (cmd, received) = CommandPort::channel(32);
        decoder.input(&map, &[0xb4, 0x22, 127], &cmd);
        decoder.input(&map, &[0xb5, 4, 64], &cmd);
        assert!(received.try_iter().next().is_none());
        decoder.input(&map, &[0xb5, 0x24, 1], &cmd);
        assert!(
            matches!(received.try_iter().next(), Some(Command::FxWet {slot:1,value}) if value == 8193.0/16383.0)
        );
        decoder.input(&map, &[0x9a, 15, 127], &cmd);
        assert!(matches!(
            received.try_iter().next(),
            Some(Command::DeckHotCue {
                deck: 1,
                pad: 7,
                del: true
            })
        ));
        decoder.input(&map, &[0xb4, 2, 127], &cmd);
        decoder.reset();
        decoder.input(&map, &[0xb4, 0x22, 127], &cmd);
        assert!(received.try_iter().next().is_none());
    }
}

pub(super) fn numark_ns7() -> MidiMap {
    let mut bindings = Vec::new();
    for deck in 0..2 {
        let offset = deck * 0x21;
        bindings.push(nbind(0, 0x11 + offset, Action::DeckPlay, deck, 0));
        bindings.push(nbind(0, 0x10 + offset, Action::DeckCue, deck, 0));
        bindings.push(nbind(0, 0x0f + offset, Action::DeckSync, deck, 0));
        bindings.push(nbind(0, 0x21 + offset, Action::DeckVinyl, deck, 0));
        bindings.push(nbind(0, 0x0c + deck * 2, Action::DeckLoad, deck, 0));
        bindings.push(nbind(0, 0x28 + offset, Action::DeckLoopIn, deck, 0));
        bindings.push(nbind(
            0,
            if deck == 0 { 0x29 } else { 0x50 },
            Action::DeckLoopOut,
            deck,
            0,
        ));
        for pad in 0..5 {
            bindings.push(nbind(0, 0x13 + offset + pad, Action::DeckHotCue, deck, pad));
        }
        bindings.push(cbind(0, 4 + deck, Action::DeckPitch, deck, 0));
        let mixer = deck * 5;
        bindings.push(cbind(0, 0x0c + mixer, Action::DeckGain, deck, 0));
        bindings.push(cbind(0, 0x08 + mixer, Action::DeckGain, deck, 0));
        bindings.push(cbind(0, 0x0b + mixer, Action::DeckEqHi, deck, 0));
        bindings.push(cbind(0, 0x0a + mixer, Action::DeckEqMid, deck, 0));
        bindings.push(cbind(0, 0x09 + mixer, Action::DeckEqLow, deck, 0));
    }
    bindings.push(cbind(0, 7, Action::Xfader, 0, 0));
    bindings.push(cbind(0, 0x12, Action::CueMix, 0, 0));
    bindings.push(cbind(0, 0x40, Action::Master, 0, 0));
    MidiMap {
        name: "Numark NS7 (original)".into(),
        matchers: vec!["ns7".into()],
        bindings,
        unmapped_notes: UnmappedNotes::Ignore,
    }
}

pub(super) struct Decoder {
    jog: [Option<u8>; 2],
    fx: [[Option<u8>; 3]; 2],
    fader: [f32; 2],
    trim: [f32; 2],
}

impl Default for Decoder {
    fn default() -> Self {
        Self {
            jog: [None; 2],
            fx: [[None; 3]; 2],
            fader: [1.0; 2],
            trim: [1.0; 2],
        }
    }
}

impl Decoder {
    pub(super) fn reset(&mut self) {
        *self = Self::default();
    }

    /// Decode controller state into engine commands.
    /// Takes the factory profile, a complete wire message and command port; returns whether the message was consumed.
    pub(super) fn input(&mut self, map: &MidiMap, message: &[u8; 3], cmd: &CommandPort) -> bool {
        let [status, control, value] = *message;
        if map.name == "Numark NS7 (original)"
            && status == 0xb0
            && matches!(control, 0x08 | 0x0d | 0x0c | 0x11)
        {
            let deck = usize::from(matches!(control, 0x0d | 0x11));
            if matches!(control, 0x08 | 0x0d) {
                self.fader[deck] = f32::from(value) / 127.0;
            } else {
                self.trim[deck] = (f32::from(value) / 64.0).min(1.5);
            }
            let _ = cmd.send(Command::DeckGain {
                deck: deck as u8,
                value: self.trim[deck] * self.fader[deck],
            });
            return true;
        }
        if map.name == "Numark NS7 (original)" && status == 0xb0 && matches!(control, 0 | 2) {
            let deck = usize::from(control / 2);
            if let Some(previous) = self.jog[deck].replace(value) {
                let delta = (i16::from(value) - i16::from(previous) + 64).rem_euclid(128) - 64;
                if delta != 0 {
                    let _ = cmd.send(Command::DeckJog {
                        deck: deck as u8,
                        delta: delta as f32 * 0.35,
                    });
                }
            }
            return true;
        }
        if map.name == "Pioneer DDJ-SP1" {
            if (0x97..=0x9a).contains(&status) && (8..16).contains(&control) {
                if value > 0 {
                    let _ = cmd.send(Command::DeckHotCue {
                        deck: (status - 0x97) % 2,
                        pad: control - 8,
                        del: true,
                    });
                }
                return true;
            }
            if matches!(status, 0xb4 | 0xb5) {
                let bank = usize::from(status - 0xb4);
                if matches!(control, 2 | 4 | 6) {
                    self.fx[bank][usize::from(control / 2 - 1)] = Some(value);
                    return true;
                }
                if matches!(control, 0x22 | 0x24 | 0x26) {
                    let slot = usize::from((control - 0x20) / 2 - 1);
                    if let Some(msb) = self.fx[bank][slot].take() {
                        let value = f32::from((u16::from(msb) << 7) | u16::from(value)) / 16383.0;
                        let _ = cmd.send(Command::FxWet {
                            slot: slot as u8,
                            value,
                        });
                    }
                    return true;
                }
            }
        }
        false
    }
}
