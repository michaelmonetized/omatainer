use crate::engine::{
    deck_controls::{Button, Control},
    surface_controls::Input,
    Command, CommandPort,
};

#[derive(Default)]
pub(super) struct Decoder {
    fx: [[Option<u8>; 6]; 2],
    volume: Option<u8>,
}
impl Decoder {
    /// Decode the manufacturer's independent deck, FX and pad channels.
    /// Takes a complete message, source and command port; returns whether the address belongs to SP1.
    pub fn input(&mut self, message: [u8; 3], source: u64, cmd: &CommandPort) -> bool {
        let [status, address, value] = message;
        let channel = status & 15;
        let note = matches!(status & 0xf0, 0x80 | 0x90);
        let on = status & 0xf0 == 0x90 && value > 0;
        let cc = status & 0xf0 == 0xb0;
        let delta = if value < 64 {
            i16::from(value)
        } else {
            i16::from(value) - 128
        };
        if (7..=10).contains(&channel) && note {
            let deck = (channel - 7) % 2;
            let pad = address % 8;
            let mode = address / 16;
            let shifted = address & 8 != 0;
            let Some(mode) = crate::engine::deck_pads::Mode::from_index(mode) else { return false; };
            let key = crate::engine::deck_pads::sp1_key(channel, address);
            let command = if on { Command::DeckPadPress(crate::engine::deck_pads::Press { source, key, deck, id: pad + 1, mode: Some(mode), pressure: f32::from(value) / 127.0, shifted }) } else { Command::DeckPadRelease(crate::engine::deck_pads::Release { source, key }) };
            let _ = cmd.send(command);
            return true;
        }
        if (7..=10).contains(&channel) && cc && (0x70..=0x77).contains(&address) {
            let _ = cmd.send(Command::Surface(Input::SamplerPressure {
                source,
                pad: (channel - 7) % 2 * 8 + address - 0x70,
                value: f32::from(value) / 127.0,
            }));
            return true;
        }
        if channel < 4 {
            let deck = channel % 2;
            if cc && matches!(address, 0x17 | 0x37) {
                for _ in 0..delta.unsigned_abs().min(30) {
                    let _ = cmd.send(Command::DeckControl {
                        source,
                        deck,
                        control: if address == 0x17 {
                            Control::LoopScale { double: delta > 0 }
                        } else {
                            Control::LoopShift { forward: delta > 0 }
                        },
                    });
                }
                return true;
            }
            if note {
                let control = match address {
                    0x15 => Control::Hold {
                        button: Button::Bleep,
                        on,
                    },
                    0x38 => Control::Hold {
                        button: Button::Reverse,
                        on,
                    },
                    0x40 => Control::Slip,
                    0x63 => Control::Keylock,
                    0x55 => Control::LoopToggle,
                    0x56 => Control::Reloop,
                    0x1b => Control::PadMode { mode: 0 },
                    0x1e => Control::PadMode { mode: 1 },
                    0x20 => Control::PadMode { mode: 2 },
                    0x22 => Control::PadMode { mode: 3 },
                    0x69 => Control::PadMode { mode: 4 },
                    0x6b => Control::PadMode { mode: 5 },
                    0x6d => Control::PadMode { mode: 6 },
                    0x6f => Control::PadMode { mode: 7 },
                    0x24..=0x2b => Control::Parameter {
                        mode: address - 0x24,
                        up: false,
                        shifted: false,
                    },
                    0x2c..=0x33 => Control::Parameter {
                        mode: address - 0x2c,
                        up: true,
                        shifted: false,
                    },
                    1..=8 => Control::Parameter {
                        mode: address - 1,
                        up: false,
                        shifted: true,
                    },
                    0x7a..=0x7f => Control::Parameter {
                        mode: address - 0x79,
                        up: true,
                        shifted: true,
                    },
                    9 => Control::Parameter {
                        mode: 0,
                        up: true,
                        shifted: true,
                    },
                    0 => Control::Parameter {
                        mode: 7,
                        up: true,
                        shifted: true,
                    },
                    0x5c => Control::SyncOff,
                    0x72 | 0x73 => {
                        if on {
                            let _ = cmd.send(Command::SelectDeck(usize::from(deck)));
                        }
                        return true;
                    }
                    _ => return false,
                };
                if matches!(control, Control::Hold { .. }) || on {
                    let _ = cmd.send(Command::DeckControl {
                        source,
                        deck,
                        control,
                    });
                }
                return true;
            }
        }
        if (4..=5).contains(&channel) {
            let bank = usize::from(channel - 4);
            if cc {
                let msb = match address {
                    2 | 4 | 6 => Some(usize::from(address / 2 - 1)),
                    0x12 | 0x14 | 0x16 => Some(usize::from((address - 0x10) / 2 + 2)),
                    _ => None,
                };
                if let Some(slot) = msb {
                    self.fx[bank][slot] = Some(value);
                    return true;
                }
                let lsb = match address {
                    0x22 | 0x24 | 0x26 => Some(usize::from((address - 0x20) / 2 - 1)),
                    0x32 | 0x34 | 0x36 => Some(usize::from((address - 0x30) / 2 + 2)),
                    _ => None,
                };
                if let Some(slot) = lsb {
                    if let Some(msb) = self.fx[bank][slot].take() {
                        let _ = cmd.send(Command::Surface(Input::FxValue {
                            bank: bank as u8,
                            slot: (slot % 3) as u8,
                            parameter: slot >= 3,
                            value: f32::from((u16::from(msb) << 7) | u16::from(value)) / 16383.0,
                        }));
                    }
                    return true;
                }
                if matches!(address, 0 | 0x10) {
                    let _ = cmd.send(Command::Surface(Input::FxButton {
                        bank: bank as u8,
                        control: address,
                        delta,
                    }));
                    return true;
                }
            }
            if note && matches!(address, 0x40 | 0x43 | 0x47..=0x49 | 0x4a | 0x63..=0x66) {
                if on {
                    let command = if matches!(address, 0x4a | 0x66) {
                        Command::DeckControl {
                            source,
                            deck: bank as u8,
                            control: Control::Tap,
                        }
                    } else {
                        Command::Surface(Input::FxButton {
                            bank: bank as u8,
                            control: address,
                            delta: 0,
                        })
                    };
                    let _ = cmd.send(command);
                }
                return true;
            }
        }
        if channel == 6 {
            if cc && matches!(address, 3 | 0x23 | 0x69) {
                if address == 3 {
                    self.volume = Some(value);
                } else if address == 0x69 {
                    let _ = cmd.send(Command::Surface(Input::SamplerVolume(
                        f32::from(value) / 127.0,
                    )));
                } else if let Some(msb) = self.volume.take() {
                    let _ = cmd.send(Command::Surface(Input::SamplerVolume(
                        f32::from((u16::from(msb) << 7) | u16::from(value)) / 16383.0,
                    )));
                }
                return true;
            }
            if note
                && matches!(
                    address,
                    0x41 | 0x42 | 0x66 | 0x4c..=0x53 | 0x58 | 0x59 | 0x60 | 0x61
                )
            {
                if on {
                    let command = match address {
                        0x4c..=0x53 => Command::Surface(Input::FxButton {
                            bank: u8::from(address >= 0x50),
                            control: address,
                            delta: 0,
                        }),
                        0x58 | 0x59 | 0x60 | 0x61 => Command::DeckControl {
                            source,
                            deck: address & 1,
                            control: Control::TrackStart,
                        },
                        0x41 => Command::BrowsePanel(1),
                        0x42 => Command::BrowsePanel(2),
                        _ => Command::CrateReturn,
                    };
                    let _ = cmd.send(command);
                }
                return true;
            }
        }
        false
    }
}
