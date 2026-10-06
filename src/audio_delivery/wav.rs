use std::fs::{File, OpenOptions};
use std::io::{Seek, SeekFrom, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Encoding {
    Float32,
    Pcm16,
    Pcm24,
}
impl Encoding {
    pub fn bytes(self) -> usize {
        match self {
            Self::Float32 => 4,
            Self::Pcm16 => 2,
            Self::Pcm24 => 3,
        }
    }
    pub fn bits(self) -> u16 {
        (self.bytes() * 8) as u16
    }
}

pub(crate) struct Writer {
    file: File,
    path: PathBuf,
    channels: u16,
    rate: u32,
    encoding: Encoding,
    dither: bool,
    random: u64,
    bytes: Vec<u8>,
    frames: u64,
}
impl Writer {
    /// Create a new bounded audio file.
    /// Takes its path, width, sample rate, encoding and integer dither choice; returns a writer that never replaces an existing file.
    pub fn new(
        path: &Path,
        channels: u16,
        rate: u32,
        encoding: Encoding,
        dither: bool,
    ) -> Result<Self, String> {
        if !(1..=26).contains(&channels)
            || !(8000..=384000).contains(&rate)
            || dither && encoding == Encoding::Float32
        {
            return Err("Unsupported WAV width, rate or floating-point dither".into());
        }
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)
            .map_err(|e| e.to_string())?;
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return Err("Audio file is already open".into());
        }
        let mut writer = Self {
            file,
            path: path.to_owned(),
            channels,
            rate,
            encoding,
            dither,
            random: 0x706572666f726d31,
            bytes: Vec::with_capacity(16384 * encoding.bytes()),
            frames: 0,
        };
        writer.header()?;
        Ok(writer)
    }
    pub fn frames(&self) -> u64 {
        self.frames
    }
    fn uniform(&mut self) -> f64 {
        self.random ^= self.random << 13;
        self.random ^= self.random >> 7;
        self.random ^= self.random << 17;
        (self.random >> 11) as f64 / (1_u64 << 53) as f64
    }
    /// Append complete channel frames.
    /// Takes finite interleaved samples and gain; writes with fixed bounded conversion storage and retains only whole frames after a write failure.
    pub fn append(&mut self, samples: &[f32], gain: f64) -> Result<(), String> {
        let width = usize::from(self.channels);
        if samples.len() % width != 0
            || !gain.is_finite()
            || gain < 0.0
            || samples
                .iter()
                .any(|v| !v.is_finite() || !(f64::from(*v) * gain).is_finite())
        {
            return Err(
                "Audio file needs complete finite frames and a finite positive gain".into(),
            );
        }
        let frames = self
            .frames
            .checked_add((samples.len() / width) as u64)
            .ok_or("WAV frame count overflow")?;
        if frames
            .checked_mul(width as u64 * self.encoding.bytes() as u64)
            .is_none_or(|n| n > u64::from(u32::MAX) - 36)
        {
            return Err("WAV size limit reached; split the recording".into());
        }
        for chunk in samples.chunks(4096 / width * width) {
            self.bytes.clear();
            for &sample in chunk {
                let value = f64::from(sample) * gain;
                match self.encoding {
                    Encoding::Float32 => {
                        let value = value as f32;
                        if !value.is_finite() {
                            return Err("Floating-point audio exceeds its finite range".into());
                        }
                        self.bytes.extend_from_slice(&value.to_le_bytes());
                    }
                    Encoding::Pcm16 | Encoding::Pcm24 => {
                        let maximum = (1_i32 << (self.encoding.bits() - 1)) - 1;
                        let noise = if self.dither {
                            self.uniform() - self.uniform()
                        } else {
                            0.0
                        };
                        let quantized = (value * f64::from(maximum + 1) + noise)
                            .round()
                            .clamp(-f64::from(maximum + 1), f64::from(maximum))
                            as i32;
                        let bytes = quantized.to_le_bytes();
                        self.bytes
                            .extend_from_slice(&bytes[..self.encoding.bytes()]);
                    }
                }
            }
            if let Err(error) = self.file.write_all(&self.bytes) {
                if let Ok(length) = self.file.metadata().map(|m| m.len()) {
                    self.frames =
                        length.saturating_sub(44) / (width * self.encoding.bytes()) as u64;
                }
                return Err(format!(
                    "Audio write failed after {} complete frames at {}: {error}",
                    self.frames,
                    self.path.display()
                ));
            }
            self.frames += (chunk.len() / width) as u64;
        }
        Ok(())
    }
    fn header(&mut self) -> Result<(), String> {
        let block = self.channels * self.encoding.bytes() as u16;
        let bytes = (self.frames * u64::from(block)) as u32;
        let mut header = [0_u8; 44];
        header[..4].copy_from_slice(b"RIFF");
        header[4..8].copy_from_slice(&(36 + bytes).to_le_bytes());
        header[8..16].copy_from_slice(b"WAVEfmt ");
        header[16..20].copy_from_slice(&16_u32.to_le_bytes());
        header[20..22].copy_from_slice(
            &(if self.encoding == Encoding::Float32 {
                3_u16
            } else {
                1_u16
            })
            .to_le_bytes(),
        );
        header[22..24].copy_from_slice(&self.channels.to_le_bytes());
        header[24..28].copy_from_slice(&self.rate.to_le_bytes());
        header[28..32].copy_from_slice(&(self.rate * u32::from(block)).to_le_bytes());
        header[32..34].copy_from_slice(&block.to_le_bytes());
        header[34..36].copy_from_slice(&self.encoding.bits().to_le_bytes());
        header[36..40].copy_from_slice(b"data");
        header[40..44].copy_from_slice(&bytes.to_le_bytes());
        self.file
            .seek(SeekFrom::Start(0))
            .and_then(|_| self.file.write_all(&header))
            .map_err(|e| e.to_string())?;
        Ok(())
    }
    /// Finalize the valid audio prefix.
    /// Takes this writer; truncates incomplete frame bytes, fixes the header and flushes the file, returning the exact frame count.
    pub fn finish(mut self) -> Result<u64, String> {
        self.file
            .set_len(44 + self.frames * u64::from(self.channels) * self.encoding.bytes() as u64)
            .map_err(|e| format!("Cannot trim audio prefix at {}: {e}", self.path.display()))?;
        self.header().map_err(|e| {
            format!(
                "Cannot finalize audio prefix at {}: {e}",
                self.path.display()
            )
        })?;
        self.file.sync_all().map_err(|e| {
            format!(
                "Cannot flush finalized audio at {}: {e}",
                self.path.display()
            )
        })?;
        Ok(self.frames)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn directory() -> PathBuf {
        let p = std::env::temp_dir().join(format!(
            "omatainer-wav-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&p).unwrap();
        p
    }
    #[test]
    fn native_formats_have_exact_headers_channels_duration_and_quantization() {
        let root = directory();
        for encoding in [Encoding::Float32, Encoding::Pcm16, Encoding::Pcm24] {
            let path = root.join(format!("{encoding:?}.wav"));
            let mut w = Writer::new(&path, 2, 96000, encoding, false).unwrap();
            w.append(&[-1.0, 0.5, 0.25, 1.0], 1.0).unwrap();
            assert_eq!(w.frames(), 2);
            assert_eq!(w.finish().unwrap(), 2);
            let b = std::fs::read(&path).unwrap();
            assert_eq!(&b[..4], b"RIFF");
            assert_eq!(
                u32::from_le_bytes(b[4..8].try_into().unwrap()) as usize,
                b.len() - 8
            );
            assert_eq!(u16::from_le_bytes(b[22..24].try_into().unwrap()), 2);
            assert_eq!(u32::from_le_bytes(b[24..28].try_into().unwrap()), 96000);
            assert_eq!(
                u16::from_le_bytes(b[34..36].try_into().unwrap()),
                encoding.bits()
            );
            assert_eq!(b.len(), 44 + 4 * encoding.bytes());
            match encoding {
                Encoding::Float32 => assert_eq!(
                    &b[44..],
                    &[-1.0_f32, 0.5, 0.25, 1.0].map(f32::to_le_bytes).concat()
                ),
                Encoding::Pcm16 => assert_eq!(
                    &b[44..],
                    &[-32768_i16, 16384, 8192, 32767]
                        .map(i16::to_le_bytes)
                        .concat()
                ),
                Encoding::Pcm24 => {
                    assert_eq!(&b[44..], &[0, 0, 128, 0, 0, 64, 0, 0, 32, 255, 255, 127])
                }
            }
        }
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn dither_is_repeatable_across_blocks_and_changes_integer_silence_only() {
        let root = directory();
        let samples = [0.0; 4096];
        let a = root.join("a.wav");
        let b = root.join("b.wav");
        let mut w = Writer::new(&a, 1, 44100, Encoding::Pcm16, true).unwrap();
        w.append(&samples, 1.0).unwrap();
        w.finish().unwrap();
        let mut w = Writer::new(&b, 1, 44100, Encoding::Pcm16, true).unwrap();
        for c in samples.chunks(137) {
            w.append(c, 1.0).unwrap()
        }
        w.finish().unwrap();
        let a = std::fs::read(a).unwrap();
        assert_eq!(a, std::fs::read(b).unwrap());
        assert!(a[44..].iter().any(|&v| v != 0));
        for sample in a[44..].chunks_exact(2) {
            assert!(i16::from_le_bytes(sample.try_into().unwrap()).abs() <= 1);
        }
        assert!(Writer::new(&root.join("float.wav"), 2, 44100, Encoding::Float32, true).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn invalid_frames_preserve_finalizable_prefix_and_existing_files_are_refused() {
        let root = directory();
        let path = root.join("record.wav");
        let mut w = Writer::new(&path, 2, 48000, Encoding::Float32, false).unwrap();
        w.append(&[0.1, 0.2], 1.0).unwrap();
        assert!(w.append(&[f32::NAN, 0.0], 1.0).is_err());
        assert!(w.append(&[1.0], 1.0).is_err());
        assert_eq!(w.finish().unwrap(), 1);
        let before = std::fs::read(&path).unwrap();
        assert!(Writer::new(&path, 2, 48000, Encoding::Float32, false).is_err());
        assert_eq!(before, std::fs::read(path).unwrap());
        std::fs::remove_dir_all(root).unwrap();
    }
}
