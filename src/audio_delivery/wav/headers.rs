use super::Encoding;
use serde::{Deserialize, Serialize};

pub(super) const REPORT_BYTES: usize = 8192;
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Container {
    #[default]
    Wave,
    Bwf,
    Rf64,
    Aiff,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct Metadata {
    pub title: String,
    pub artist: String,
    pub isrc: String,
    pub description: String,
    pub originator: String,
    pub originator_reference: String,
    pub date: String,
    pub time: String,
    pub time_reference: u64,
}
impl Metadata {
    /// Validate delivery metadata without truncation.
    /// Args: authored tags. Returns an error for oversized text or malformed broadcast fields.
    pub fn validate(&self) -> Result<(), String> {
        for text in [
            &self.title,
            &self.artist,
            &self.isrc,
            &self.description,
            &self.originator,
            &self.originator_reference,
            &self.date,
            &self.time,
        ] {
            if text.len() > 1024 || text.chars().any(|c| c.is_control()) {
                return Err("Delivery metadata contains oversized or control text".into());
            }
        }
        if !self.isrc.is_empty()
            && (self.isrc.len() != 12 || !self.isrc.bytes().all(|b| b.is_ascii_alphanumeric()))
        {
            return Err("ISRC needs12 letters/digits without separators".into());
        }
        Ok(())
    }
}
#[derive(Clone, Debug)]
pub(crate) struct Settings {
    pub container: Container,
    pub wave_mask: Option<u32>,
    pub metadata: Metadata,
    pub seed: u64,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            container: Container::Wave,
            wave_mask: None,
            metadata: Default::default(),
            seed: 0x706572666f726d31,
        }
    }
}
fn chunk(out: &mut Vec<u8>, id: &[u8; 4], bytes: &[u8], big: bool) {
    out.extend_from_slice(id);
    out.extend_from_slice(&if big {
        (bytes.len() as u32).to_be_bytes()
    } else {
        (bytes.len() as u32).to_le_bytes()
    });
    out.extend_from_slice(bytes);
    if bytes.len() & 1 != 0 {
        out.push(0);
    }
}
fn fixed(out: &mut [u8], text: &str) -> Result<(), String> {
    if !text.is_ascii() || text.len() > out.len() {
        return Err(format!(
            "Broadcast metadata requires ASCII text fitting{} bytes",
            out.len()
        ));
    }
    out[..text.len()].copy_from_slice(text.as_bytes());
    Ok(())
}
fn xml(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}
/// Encode a fixed-size embedded delivery report.
/// Args: JSON report and container. Returns bounded XML for WAVE or JSON annotation for AIFF, padded without changing audio offsets.
pub(crate) fn report_bytes(
    value: &serde_json::Value,
    container: Container,
) -> Result<Vec<u8>, String> {
    let json = serde_json::to_string(value).map_err(|e| e.to_string())?;
    let text = if container == Container::Aiff {
        json
    } else {
        format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?><BWFXML><OMATAINER_DELIVERY>{}</OMATAINER_DELIVERY></BWFXML>",xml(&json))
    };
    if text.len() > REPORT_BYTES {
        return Err("Embedded delivery report exceeds8KiB; shorten metadata".into());
    }
    let mut bytes = vec![b' '; REPORT_BYTES];
    bytes[..text.len()].copy_from_slice(text.as_bytes());
    Ok(bytes)
}
impl Settings {
    /// Build a streamed container header for exact complete frames.
    /// Args: actual channels/rate/encoding/frame count. Returns header bytes with unpadded data size and bounded embedded metadata.
    pub(super) fn header(
        &self,
        channels: u16,
        rate: u32,
        encoding: Encoding,
        frames: u64,
    ) -> Result<Vec<u8>, String> {
        if !(1..=64).contains(&channels) || !(8000..=384000).contains(&rate) {
            return Err("Unsupported container channel count or rate".into());
        }
        self.metadata.validate()?;
        if self
            .wave_mask
            .is_some_and(|mask| mask.count_ones() != u32::from(channels))
        {
            return Err("Speaker mask count differs from actual channel count".into());
        }
        let data = frames
            .checked_mul(u64::from(channels) * encoding.bytes() as u64)
            .ok_or("Container size overflow")?;
        let tags = serde_json::to_value(&self.metadata).map_err(|e| e.to_string())?;
        let report = report_bytes(
            &serde_json::json!({"metadata":tags,"channels":channels,"wave_mask":self.wave_mask,"qc":null}),
            self.container,
        )?;
        if self.container == Container::Aiff {
            if encoding == Encoding::Float32
                || frames > u64::from(u32::MAX)
                || data > u64::from(u32::MAX) - 8
            {
                return Err("AIFF requires16/24-bit PCM within its32-bit frame/chunk sizes; use RF64 for larger files".into());
            }
            let mut out = b"FORM\0\0\0\0AIFF".to_vec();
            let mut comm = Vec::new();
            comm.extend_from_slice(&channels.to_be_bytes());
            comm.extend_from_slice(&(frames as u32).to_be_bytes());
            comm.extend_from_slice(&encoding.bits().to_be_bytes());
            let power = 31 - rate.leading_zeros();
            comm.extend_from_slice(&((16383 + power) as u16).to_be_bytes());
            comm.extend_from_slice(&(u64::from(rate) << (63 - power)).to_be_bytes());
            chunk(&mut out, b"COMM", &comm, true);
            if let Some(mask) = self.wave_mask.filter(|mask| *mask < 0x40000) {
                let mut layout = Vec::with_capacity(12);
                layout.extend_from_slice(&0x10000_u32.to_be_bytes());
                layout.extend_from_slice(&mask.to_be_bytes());
                layout.extend_from_slice(&0_u32.to_be_bytes());
                chunk(&mut out, b"CHAN", &layout, true);
            }
            if !self.metadata.title.is_empty() {
                chunk(&mut out, b"NAME", self.metadata.title.as_bytes(), true);
            }
            if !self.metadata.artist.is_empty() {
                chunk(&mut out, b"AUTH", self.metadata.artist.as_bytes(), true);
            }
            chunk(&mut out, b"ANNO", &report, true);
            out.extend_from_slice(b"SSND");
            out.extend_from_slice(&((data + 8) as u32).to_be_bytes());
            out.extend_from_slice(&[0; 8]);
            let size = (out.len() as u64 - 8)
                .checked_add(data + (data & 1))
                .ok_or("AIFF size overflow")?;
            if size > u64::from(u32::MAX) {
                return Err("AIFF FORM exceeds32-bit size; use RF64".into());
            }
            out[4..8].copy_from_slice(&(size as u32).to_be_bytes());
            return Ok(out);
        }
        let rf64 = self.container == Container::Rf64;
        let mut out = if rf64 {
            b"RF64\xff\xff\xff\xffWAVE".to_vec()
        } else {
            b"RIFF\0\0\0\0WAVE".to_vec()
        };
        if rf64 {
            let mut ds64 = Vec::new();
            ds64.extend_from_slice(&0_u64.to_le_bytes());
            ds64.extend_from_slice(&data.to_le_bytes());
            ds64.extend_from_slice(&frames.to_le_bytes());
            ds64.extend_from_slice(&0_u32.to_le_bytes());
            chunk(&mut out, b"ds64", &ds64, false);
        }
        let extended = channels > 2 || self.wave_mask.is_some();
        let tag = if encoding == Encoding::Float32 {
            3_u16
        } else {
            1
        };
        let block = channels * encoding.bytes() as u16;
        let mut fmt = Vec::new();
        fmt.extend_from_slice(&if extended { 0xfffe_u16 } else { tag }.to_le_bytes());
        fmt.extend_from_slice(&channels.to_le_bytes());
        fmt.extend_from_slice(&rate.to_le_bytes());
        fmt.extend_from_slice(&(rate * u32::from(block)).to_le_bytes());
        fmt.extend_from_slice(&block.to_le_bytes());
        fmt.extend_from_slice(&encoding.bits().to_le_bytes());
        if extended {
            fmt.extend_from_slice(&22_u16.to_le_bytes());
            fmt.extend_from_slice(&encoding.bits().to_le_bytes());
            fmt.extend_from_slice(&self.wave_mask.unwrap_or(0).to_le_bytes());
            fmt.extend_from_slice(&u32::from(tag).to_le_bytes());
            fmt.extend_from_slice(&[0, 0, 0x10, 0, 0x80, 0, 0, 0xaa, 0, 0x38, 0x9b, 0x71]);
        }
        chunk(&mut out, b"fmt ", &fmt, false);
        if encoding == Encoding::Float32 {
            chunk(
                &mut out,
                b"fact",
                &(frames.min(u64::from(u32::MAX)) as u32).to_le_bytes(),
                false,
            );
        }
        if matches!(self.container, Container::Bwf | Container::Rf64) {
            let m = &self.metadata;
            let mut bext = vec![0; 602];
            fixed(&mut bext[..256], &m.description)?;
            fixed(&mut bext[256..288], &m.originator)?;
            fixed(&mut bext[288..320], &m.originator_reference)?;
            if (!m.date.is_empty()
                && (m.date.len() != 10
                    || !m.date.bytes().enumerate().all(|(i, b)| {
                        if [4, 7].contains(&i) {
                            b == b'-'
                        } else {
                            b.is_ascii_digit()
                        }
                    })))
                || (!m.time.is_empty()
                    && (m.time.len() != 8
                        || !m.time.bytes().enumerate().all(|(i, b)| {
                            if [2, 5].contains(&i) {
                                b == b':'
                            } else {
                                b.is_ascii_digit()
                            }
                        })))
            {
                return Err("BWF date/time require YYYY-MM-DD and HH:MM:SS".into());
            }
            fixed(&mut bext[320..330], &m.date)?;
            fixed(&mut bext[330..338], &m.time)?;
            bext[338..346].copy_from_slice(&m.time_reference.to_le_bytes());
            bext[346..348].copy_from_slice(&1_u16.to_le_bytes());
            chunk(&mut out, b"bext", &bext, false);
        }
        let mut info = b"INFO".to_vec();
        for (id, text) in [
            (b"INAM", &self.metadata.title),
            (b"IART", &self.metadata.artist),
            (b"ICMT", &self.metadata.description),
        ] {
            if !text.is_empty() {
                let mut bytes = text.as_bytes().to_vec();
                bytes.push(0);
                chunk(&mut info, id, &bytes, false);
            }
        }
        if info.len() > 4 {
            chunk(&mut out, b"LIST", &info, false);
        }
        chunk(&mut out, b"iXML", &report, false);
        out.extend_from_slice(b"data");
        out.extend_from_slice(
            &if rf64 {
                u32::MAX
            } else {
                u32::try_from(data).map_err(|_| "RIFF data exceeds4GiB; choose RF64")?
            }
            .to_le_bytes(),
        );
        let size = (out.len() as u64 - 8)
            .checked_add(data + (data & 1))
            .ok_or("RIFF size overflow")?;
        if rf64 {
            out[20..28].copy_from_slice(&size.to_le_bytes());
        } else {
            out[4..8].copy_from_slice(
                &u32::try_from(size)
                    .map_err(|_| "RIFF exceeds4GiB; choose RF64")?
                    .to_le_bytes(),
            );
        }
        Ok(out)
    }
}
