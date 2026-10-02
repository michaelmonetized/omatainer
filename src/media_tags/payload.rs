//! Versioned identity for audio packets and playback-relevant codec parameters.
//! This is distinct from a complete-file SHA: changing tag bytes must not
//! manufacture a same-file proof. Callers still guard the captured file/path.
use super::*;
use sha2::{Digest, Sha256};
use std::sync::Arc;
use symphonia::core::{
    audio::SampleBuffer,
    codecs::*,
    errors::Error,
    formats::FormatOptions,
    io::{MediaSource, MediaSourceStream},
    meta::{Limit, MetadataOptions},
    probe::Hint,
};

const MAX_PACKETS: u64 = 5_000_000;
const MAX_PACKET_BYTES: usize = 16 * 1024 * 1024;

fn pcm_frame_bytes(parameters: &CodecParameters) -> Option<u64> {
    let sample_bytes = match parameters.codec {
        CODEC_TYPE_PCM_S8 | CODEC_TYPE_PCM_U8 | CODEC_TYPE_PCM_ALAW | CODEC_TYPE_PCM_MULAW => 1,
        CODEC_TYPE_PCM_S16LE | CODEC_TYPE_PCM_S16BE | CODEC_TYPE_PCM_U16LE
        | CODEC_TYPE_PCM_U16BE => 2,
        CODEC_TYPE_PCM_S24LE | CODEC_TYPE_PCM_S24BE | CODEC_TYPE_PCM_U24LE
        | CODEC_TYPE_PCM_U24BE => 3,
        CODEC_TYPE_PCM_S32LE | CODEC_TYPE_PCM_S32BE | CODEC_TYPE_PCM_U32LE
        | CODEC_TYPE_PCM_U32BE | CODEC_TYPE_PCM_F32LE | CODEC_TYPE_PCM_F32BE => 4,
        CODEC_TYPE_PCM_F64LE | CODEC_TYPE_PCM_F64BE => 8,
        _ => return None,
    };
    Some(parameters.channels?.count() as u64 * sample_bytes)
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Identity {
    /// Algorithm 1 binds Symphonia 0.5.5 codec parameters and ordered packets.
    pub algorithm: u32,
    pub digest: [u8; 32],
    pub packets: u64,
    pub bytes: u64,
}
impl Identity {
    pub fn valid(&self) -> bool {
        self.algorithm == 1
            && (1..=MAX_PACKETS).contains(&self.packets)
            && (1..=MAX_SOURCE).contains(&self.bytes)
    }
}
struct Input {
    file: File,
    cancel: Arc<AtomicBool>,
    length: u64,
    reads: u64,
    operations: u64,
}
impl Input {
    fn active(&mut self) -> io::Result<()> {
        active(&self.cancel).map_err(io::Error::other)?;
        self.operations = self
            .operations
            .checked_sub(1)
            .ok_or_else(|| io::Error::other("audio verification operation limit"))?;
        Ok(())
    }
}
impl Read for Input {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.active()?;
        if buffer.is_empty() {
            return Ok(0);
        }
        let remaining = self.length.saturating_sub(self.file.stream_position()?);
        if remaining == 0 {
            return Ok(0);
        }
        if self.reads == 0 {
            return Err(io::Error::other("audio verification read budget exceeded"));
        }
        let limit = buffer
            .len()
            .min(self.reads.min(remaining).min(usize::MAX as u64) as usize);
        let n = self.file.read(&mut buffer[..limit])?;
        self.reads -= n as u64;
        Ok(n)
    }
}
impl Seek for Input {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        self.active()?;
        let n = match position {
            SeekFrom::Start(n) => n as i128,
            SeekFrom::Current(n) => self.file.stream_position()? as i128 + n as i128,
            SeekFrom::End(n) => self.length as i128 + n as i128,
        };
        if n < 0 || n > self.length as i128 {
            return Err(io::Error::other(
                "audio verification seek outside captured file",
            ));
        }
        self.file.seek(SeekFrom::Start(n as u64))
    }
}
impl MediaSource for Input {
    fn is_seekable(&self) -> bool {
        true
    }
    fn byte_len(&self) -> Option<u64> {
        Some(self.length)
    }
}
pub(crate) fn measure(mut file: File, cancel: Arc<AtomicBool>) -> Result<Identity, String> {
    active(&cancel)?;
    let metadata = file.metadata().map_err(|e| e.to_string())?;
    if !metadata.is_file() || metadata.len() > MAX_SOURCE {
        return Err("audio verification requires a regular file within 8 GiB".into());
    }
    let fingerprint = FileFingerprint::from_metadata(&metadata);
    // A separate descriptor handle observes the same inode after demuxing.
    let guard = file.try_clone().map_err(|e| e.to_string())?;
    file.rewind().map_err(|e| e.to_string())?;
    let input = Input {
        file,
        cancel: cancel.clone(),
        length: metadata.len(),
        reads: metadata.len().saturating_add(TAG_READ_BYTES),
        operations: MAX_PACKETS.saturating_mul(4),
    };
    let mut format = symphonia::default::get_probe()
        .format(
            &Hint::new(),
            MediaSourceStream::new(Box::new(input), Default::default()),
            &FormatOptions {
                enable_gapless: true,
                ..Default::default()
            },
            &MetadataOptions {
                limit_metadata_bytes: Limit::Maximum(4096),
                limit_visual_bytes: Limit::Maximum(0),
            },
        )
        .map_err(|e| e.to_string())?
        .format;
    if format.tracks().len() != 1 {
        return Err("tag rewrite verification requires exactly one audio stream".into());
    }
    let track = format
        .default_track()
        .ok_or("audio stream unavailable for tag verification")?;
    let id = track.id;
    let p = &track.codec_params;
    // Match the production decoder's distinction: MPEG lengths may come from
    // a bitrate estimate, whereas the other supported container lengths are
    // declarations that must be complete before the identity can be trusted.
    let estimated_length = matches!(p.codec, CODEC_TYPE_MP1 | CODEC_TYPE_MP2 | CODEC_TYPE_MP3);
    let pcm_frame_bytes = pcm_frame_bytes(p);
    let mut decoder = symphonia::default::get_codecs()
        .make(p, &DecoderOptions { verify: true })
        .map_err(|e| format!("audio verification decoder unavailable: {e}"))?;
    let mut hash = Sha256::new();
    hash.update(b"omatainer:symphonia-0.5.5:audio-packets:v1\0");
    // n_frames can be an estimate based on container size. The actual ordered
    // packet durations/count and final bytes below bind the audio independently.
    let parameters = format!(
        "{:?}",
        (
            p.codec,
            p.sample_rate,
            p.time_base,
            p.start_ts,
            p.sample_format,
            p.bits_per_sample,
            p.bits_per_coded_sample,
            p.channels,
            p.channel_layout,
            p.delay,
            p.padding,
            p.frames_per_block
        )
    );
    hash.update((parameters.len() as u64).to_le_bytes());
    hash.update(parameters.as_bytes());
    if !estimated_length {
        hash.update([u8::from(p.n_frames.is_some())]);
        hash.update(p.n_frames.unwrap_or(0).to_le_bytes());
    }
    let extra = p.extra_data.as_deref().unwrap_or(&[]);
    if extra.len() > MAX_ITEM_ALLOCATION {
        return Err("audio codec header exceeds verification limit".into());
    }
    hash.update((extra.len() as u64).to_le_bytes());
    hash.update(extra);
    let mut packets = 0u64;
    let mut bytes = 0u64;
    let mut packet_end = p.start_ts;
    let mut decoded_frames = 0u64;
    let mut decoded_spec = None;
    loop {
        active(&cancel)?;
        let packet = match format.next_packet() {
            Ok(packet) => packet,
            Err(Error::IoError(error)) if error.kind() == io::ErrorKind::UnexpectedEof => break,
            Err(error) => return Err(format!("audio packet verification failed: {error}")),
        };
        if packet.data.is_empty() {
            return Err("truncated stream yielded an empty audio packet".into());
        }
        if let Some(frame_bytes) = pcm_frame_bytes {
            if packet.dur.checked_mul(frame_bytes) != Some(packet.data.len() as u64) {
                return Err(
                    "truncated PCM packet does not contain its declared audio frames".into(),
                );
            }
        }
        if packet.track_id() != id || packet.data.len() > MAX_PACKET_BYTES {
            return Err("audio stream changed or packet exceeds 16 MiB".into());
        }
        if packet.ts > packet_end {
            return Err("missing audio between packet timestamps".into());
        }
        let decoded = decoder
            .decode(&packet)
            .map_err(|e| format!("audio verification decode failed: {e}"))?;
        active(&cancel)?;
        let spec = *decoded.spec();
        let channels = spec.channels.count() as u64;
        if spec.rate == 0 || channels == 0 || decoded_spec.is_some_and(|old| old != spec) {
            return Err("invalid or changed audio format during verification".into());
        }
        if (decoded.capacity() as u64)
            .saturating_mul(channels)
            .saturating_mul(4)
            > MAX_PACKET_BYTES as u64
        {
            return Err("decoded verification packet exceeds 16 MiB".into());
        }
        decoded_spec = Some(spec);
        decoded_frames = decoded_frames
            .checked_add(decoded.frames() as u64)
            .ok_or("decoded audio frame count overflow")?;
        // One bounded scratch packet is discarded each iteration; the worker
        // never allocates or retains a complete decoded track for this proof.
        let mut samples = SampleBuffer::<f32>::new(decoded.capacity() as u64, spec);
        samples.copy_interleaved_ref(decoded);
        if samples.samples().iter().any(|sample| !sample.is_finite()) {
            return Err("non-finite decoded audio during verification".into());
        }
        packets += 1;
        bytes = bytes
            .checked_add(packet.data.len() as u64)
            .ok_or("audio byte count overflow")?;
        if packets > MAX_PACKETS || bytes > MAX_SOURCE {
            return Err("audio verification exceeds packet/byte limits".into());
        }
        hash.update(packet.ts.to_le_bytes());
        hash.update(packet.dur.to_le_bytes());
        hash.update(packet.trim_start.to_le_bytes());
        hash.update(packet.trim_end.to_le_bytes());
        hash.update((packet.data.len() as u64).to_le_bytes());
        hash.update(&packet.data);
        packet_end = packet_end.max(packet.ts.saturating_add(packet.dur));
    }
    active(&cancel)?;
    if !estimated_length {
        let parameters = &format
            .tracks()
            .iter()
            .find(|track| track.id == id)
            .ok_or("audio stream disappeared during verification")?
            .codec_params;
        if parameters
            .n_frames
            .is_some_and(|frames| packet_end < parameters.start_ts.saturating_add(frames))
        {
            return Err("audio verification ended before the declared audio duration".into());
        }
        if let (Some(frames), Some(spec)) = (parameters.n_frames, decoded_spec) {
            let expected = match parameters.time_base {
                Some(base) if base.denom != 0 && base.numer != 0 => {
                    u128::from(frames) * u128::from(base.numer) * u128::from(spec.rate)
                        / u128::from(base.denom)
                }
                Some(_) => return Err("invalid audio time base".into()),
                None => u128::from(frames),
            };
            if u128::from(decoded_frames) < expected {
                return Err("decoded audio ended before the declared audio duration".into());
            }
        }
    }
    if decoded_frames == 0 || decoder.finalize().verify_ok == Some(false) {
        return Err("decoded audio is empty or fails its embedded checksum".into());
    }
    if FileFingerprint::from_metadata(&guard.metadata().map_err(|e| e.to_string())?) != fingerprint
    {
        return Err("media changed during audio verification".into());
    }
    hash.update(packets.to_le_bytes());
    hash.update(bytes.to_le_bytes());
    let identity = Identity {
        algorithm: 1,
        digest: hash.finalize().into(),
        packets,
        bytes,
    };
    if !identity.valid() {
        return Err("no bounded audio packets found".into());
    }
    Ok(identity)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn appended_bytes_remain_outside_the_captured_audio_source() {
        use std::io::Write;
        let path = std::env::temp_dir().join(format!(
            "omat-payload-capture-{}",
            crate::sampler_bank::BankId::new().unwrap()
        ));
        let mut created = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap();
        std::fs::remove_file(&path).unwrap();
        created.write_all(b"capture").unwrap();
        let mut input = Input {
            file: created.try_clone().unwrap(),
            cancel: Arc::new(AtomicBool::new(false)),
            length: 7,
            reads: 100,
            operations: 100,
        };
        created.write_all(b"appended").unwrap();
        input.seek(SeekFrom::Start(5)).unwrap();
        let mut bytes = [0; 32];
        assert_eq!(input.read(&mut bytes).unwrap(), 2);
        assert_eq!(&bytes[..2], b"re");
        assert_eq!(input.read(&mut bytes).unwrap(), 0);
    }
}
