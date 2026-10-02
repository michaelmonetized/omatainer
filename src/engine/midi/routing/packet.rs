//! Complete bounded MIDI1 packets. This parser runs on the input worker, not
//! the native callback. Running status is local to one complete callback packet.
pub const MAX_BYTES: usize = 256;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Packet {
    len: u16,
    bytes: [u8; MAX_BYTES],
}
impl Packet {
    pub fn new(bytes: &[u8]) -> Option<Self> {
        let (&status, rest) = bytes.split_first()?;
        let valid = match status {
            0x80..=0xef => {
                let needed = if matches!(status & 0xf0, 0xc0 | 0xd0) {
                    2
                } else {
                    3
                };
                bytes.len() == needed && rest.iter().all(|&b| b < 128)
            }
            0xf0 => {
                (3..=MAX_BYTES).contains(&bytes.len())
                    && bytes.last() == Some(&0xf7)
                    && (bytes[1] != 0 || bytes.len() >= 5)
                    && bytes[1..bytes.len() - 1].iter().all(|&b| b < 128)
            }
            _ => false,
        };
        if !valid {
            return None;
        }
        let mut result = Self {
            len: bytes.len() as u16,
            bytes: [0; MAX_BYTES],
        };
        result.bytes[..bytes.len()].copy_from_slice(bytes);
        Some(result)
    }
    pub fn bytes(&self) -> &[u8] {
        &self.bytes[..usize::from(self.len)]
    }
    pub fn channel(&self) -> Option<u8> {
        (self.bytes[0] < 0xf0).then_some(self.bytes[0] & 15)
    }
    pub fn with_channel(mut self, channel: Option<u8>) -> Self {
        if let Some(ch) = channel.filter(|&ch| ch <= 15 && self.bytes[0] < 0xf0) {
            self.bytes[0] = self.bytes[0] & 0xf0 | ch;
        }
        self
    }
    pub fn note(&self) -> Option<(u8, u8, bool)> {
        match self.bytes[0] & 0xf0 {
            0x90 => Some((self.bytes[1], self.bytes[2], self.bytes[2] != 0)),
            0x80 => Some((self.bytes[1], self.bytes[2], false)),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Frame {
    Musical(Packet),
    Realtime(u8),
    Malformed,
}
pub struct Frames<'a> {
    bytes: &'a [u8],
    cursor: usize,
    status: u8,
    frame: [u8; MAX_BYTES],
    len: usize,
    needed: usize,
    sysex: bool,
}
pub fn frames(bytes: &[u8]) -> Frames<'_> {
    Frames {
        bytes,
        cursor: 0,
        status: 0,
        frame: [0; MAX_BYTES],
        len: 0,
        needed: 0,
        sysex: false,
    }
}
impl Iterator for Frames<'_> {
    type Item = Frame;
    fn next(&mut self) -> Option<Frame> {
        while self.cursor < self.bytes.len() {
            let b = self.bytes[self.cursor];
            self.cursor += 1;
            if b >= 0xf8 {
                return Some(Frame::Realtime(b));
            }
            if self.sysex {
                if b == 0xf7 && self.len < MAX_BYTES {
                    self.frame[self.len] = b;
                    self.len += 1;
                    self.sysex = false;
                    self.status = 0;
                    let p = Packet::new(&self.frame[..self.len]);
                    self.len = 0;
                    return Some(p.map_or(Frame::Malformed, Frame::Musical));
                }
                if b >= 128 || self.len >= MAX_BYTES - 1 {
                    self.sysex = false;
                    self.len = 0;
                    self.status = 0;
                    // A new channel status is processed on the next call.
                    if b >= 0x80 && b < 0xf0 {
                        self.cursor -= 1;
                    }
                    return Some(Frame::Malformed);
                }
                self.frame[self.len] = b;
                self.len += 1;
                continue;
            }
            if b >= 0x80 {
                let incomplete = self.len > 0;
                self.len = 0;
                match b {
                    0x80..=0xef => {
                        self.status = b;
                        self.frame[0] = b;
                        self.len = 1;
                        self.needed = if matches!(b & 0xf0, 0xc0 | 0xd0) {
                            2
                        } else {
                            3
                        };
                    }
                    0xf0 => {
                        self.status = 0;
                        self.sysex = true;
                        self.frame[0] = b;
                        self.len = 1;
                    }
                    _ => {
                        self.status = 0;
                        self.needed = 0;
                    }
                }
                if incomplete {
                    return Some(Frame::Malformed);
                }
                continue;
            }
            if self.status == 0 {
                continue;
            }
            if self.len == 0 {
                self.frame[0] = self.status;
                self.len = 1;
            }
            self.frame[self.len] = b;
            self.len += 1;
            if self.len == self.needed {
                let packet = Packet::new(&self.frame[..self.len]).unwrap();
                self.len = 0;
                return Some(Frame::Musical(packet));
            }
        }
        if self.sysex || self.len > 0 {
            self.sysex = false;
            self.len = 0;
            self.status = 0;
            return Some(Frame::Malformed);
        }
        None
    }
}
