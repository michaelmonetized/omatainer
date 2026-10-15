use super::*;

#[derive(Clone, Debug)]
pub(super) struct Header {
    pub data_offset: u64,
    pub declared: u64,
    pub riff: u64,
    pub wave_mask: Option<u32>,
    fact: Option<(u64, u32)>,
}

impl Header {
    /// Read a bounded native recording header.
    /// Takes an open regular RIFF file and the reviewed sample format; returns exact data/fact offsets, refusing foreign layouts and incomplete headers.
    pub fn read(
        file: &mut File,
        channels: u16,
        rate: u32,
        encoding: Encoding,
    ) -> Result<Self, String> {
        file.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
        let mut riff = [0; 12];
        file.read_exact(&mut riff).map_err(|e| e.to_string())?;
        let refuse = "WAV does not match this recording's native format; no repair was made";
        if &riff[..4] != b"RIFF" || &riff[8..] != b"WAVE" {
            return Err(refuse.into());
        }
        let block = channels * encoding.bytes() as u16;
        let tag = if encoding == Encoding::Float32 {
            3_u16
        } else {
            1
        };
        let length = file.metadata().map_err(|e| e.to_string())?.len();
        let mut offset = 12_u64;
        let mut format = false;
        let mut extended = false;
        let mut tagged = false;
        let mut mask = None;
        let mut fact = None;
        loop {
            if offset
                .checked_add(8)
                .is_none_or(|end| end > length || end > 65536)
            {
                return Err(refuse.into());
            }
            file.seek(SeekFrom::Start(offset))
                .map_err(|e| e.to_string())?;
            let mut chunk = [0; 8];
            file.read_exact(&mut chunk).map_err(|e| e.to_string())?;
            let count = u64::from(u32::from_le_bytes(chunk[4..].try_into().unwrap()));
            if &chunk[..4] == b"data" {
                if !format || (offset != 36 && !tagged) || (extended && !tagged) {
                    return Err(refuse.into());
                }
                return Ok(Self {
                    data_offset: offset + 8,
                    declared: count,
                    riff: u64::from(u32::from_le_bytes(riff[4..8].try_into().unwrap())),
                    wave_mask: mask,
                    fact,
                });
            }
            let end = offset
                .checked_add(8)
                .and_then(|n| n.checked_add(count + (count & 1)))
                .filter(|&n| n <= length && n <= 65536)
                .ok_or(refuse)?;
            let mut bytes = vec![0; count as usize];
            file.read_exact(&mut bytes).map_err(|e| e.to_string())?;
            match &chunk[..4] {
                b"fmt " => {
                    if format || offset != 12 || ![16, 40].contains(&count) {
                        return Err(refuse.into());
                    }
                    extended = count == 40;
                    let actual_tag = u16::from_le_bytes(bytes[..2].try_into().unwrap());
                    if actual_tag != if extended { 0xfffe } else { tag }
                        || bytes[2..4] != channels.to_le_bytes()
                        || bytes[4..8] != rate.to_le_bytes()
                        || bytes[8..12] != (rate * u32::from(block)).to_le_bytes()
                        || bytes[12..14] != block.to_le_bytes()
                        || bytes[14..16] != encoding.bits().to_le_bytes()
                    {
                        return Err(refuse.into());
                    }
                    if extended {
                        if bytes[16..18] != 22_u16.to_le_bytes()
                            || bytes[18..20] != encoding.bits().to_le_bytes()
                            || bytes[24..28] != u32::from(tag).to_le_bytes()
                            || bytes[28..40]
                                != [0, 0, 0x10, 0, 0x80, 0, 0, 0xaa, 0, 0x38, 0x9b, 0x71]
                        {
                            return Err(refuse.into());
                        }
                        let bits = u32::from_le_bytes(bytes[20..24].try_into().unwrap());
                        if bits != 0 && bits.count_ones() != u32::from(channels) {
                            return Err(refuse.into());
                        }
                        mask = (bits != 0).then_some(bits);
                    } else {
                        mask = match channels {
                            1 => Some(4),
                            2 => Some(3),
                            _ => None,
                        };
                    }
                    format = true;
                }
                b"fact"
                    if format && encoding == Encoding::Float32 && count == 4 && fact.is_none() =>
                {
                    fact = Some((offset + 8, u32::from_le_bytes(bytes.try_into().unwrap())));
                }
                b"LIST" if format && bytes.starts_with(b"INFO") => {}
                b"iXML" if format && !tagged => {
                    let text = std::str::from_utf8(&bytes).map_err(|_| refuse)?;
                    if !text.starts_with("<?xml") || !text.contains("<OMATAINER_DELIVERY>") {
                        return Err(refuse.into());
                    }
                    tagged = true;
                }
                _ => return Err(refuse.into()),
            }
            offset = end;
        }
    }

    /// Check whether complete sample counts match the retained header.
    /// Takes the recovered frames and bytes; returns whether fact and RIFF/data sizes require repair.
    pub fn needs_repair(&self, frames: u64, bytes: u64, length: u64) -> bool {
        length != self.data_offset + bytes + (bytes & 1)
            || self.riff != self.data_offset - 8 + bytes + (bytes & 1)
            || self.declared != bytes
            || self.fact.is_some_and(|(_, n)| u64::from(n) != frames)
    }

    /// Finalize the reviewed prefix without replacing metadata or speaker declarations.
    /// Takes the locked file, complete frames and bytes; writes only the final pad and exact RIFF/data/fact counts, then syncs.
    pub fn repair(&self, file: &mut File, frames: u64, bytes: u32) -> std::io::Result<()> {
        let padded = u64::from(bytes) + u64::from(bytes & 1);
        file.set_len(self.data_offset + padded)?;
        if bytes & 1 != 0 {
            file.seek(SeekFrom::Start(self.data_offset + u64::from(bytes)))?;
            file.write_all(&[0])?;
        }
        file.seek(SeekFrom::Start(4))?;
        file.write_all(&((self.data_offset - 8 + padded) as u32).to_le_bytes())?;
        file.seek(SeekFrom::Start(self.data_offset - 4))?;
        file.write_all(&bytes.to_le_bytes())?;
        if let Some((offset, _)) = self.fact {
            file.seek(SeekFrom::Start(offset))?;
            file.write_all(&(frames as u32).to_le_bytes())?;
        }
        file.sync_all()
    }
}
