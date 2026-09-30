//! A MIDI callback supplies complete packets. Do not carry truncated channel
//! frames into another callback or synthesize missing data. Realtime bytes may
//! occur anywhere, including between a channel status and its data bytes.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Message {
    Realtime(u8),
    Channel([u8; 3]),
}

pub(super) struct Messages<'a> {
    bytes: std::slice::Iter<'a, u8>,
    frame: [u8; 3],
    len: usize,
    needed: usize,
}

pub(super) fn messages(bytes: &[u8]) -> Messages<'_> {
    Messages {
        bytes: bytes.iter(),
        frame: [0; 3],
        len: 0,
        needed: 0,
    }
}

impl Iterator for Messages<'_> {
    type Item = Message;

    fn next(&mut self) -> Option<Self::Item> {
        for &byte in self.bytes.by_ref() {
            if byte >= 0xf8 {
                // Realtime never becomes data or changes channel framing.
                return Some(Message::Realtime(byte));
            }
            if byte >= 0x80 {
                // Any new non-realtime status discards an incomplete frame.
                self.len = 0;
                self.needed = match byte & 0xf0 {
                    0x80 | 0x90 | 0xa0 | 0xb0 | 0xe0 => 3,
                    0xc0 | 0xd0 => 2,
                    _ => 0, // System-common and SysEx have no mapped action.
                };
                if self.needed != 0 {
                    self.frame[0] = byte;
                    self.len = 1;
                }
                continue;
            }
            if self.len == 0 {
                continue; // Orphan data cannot manufacture a channel/status.
            }
            self.frame[self.len] = byte;
            self.len += 1;
            if self.len == self.needed {
                self.len = 0;
                // Program Change and Channel Pressure are complete at two
                // bytes, but currently unsupported by bindings/learn. Do not
                // invent a third byte for the existing three-byte interface.
                if self.needed == 3 {
                    return Some(Message::Channel(self.frame));
                }
            }
        }
        None
    }
}
