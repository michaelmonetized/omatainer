//! File decoding is transactional: detected damage never produces a loadable
//! partial Sample. Unknown/estimated lengths remain explicitly unverified.

use super::dsp::{detect_bpm, peaks_3band_with_cancel, Sample};
use std::fmt;
use std::path::Path;
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{
    CodecParameters, DecoderOptions, CODEC_TYPE_MP1, CODEC_TYPE_MP2, CODEC_TYPE_MP3,
};
use symphonia::core::errors::Error;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::{MediaSource, MediaSourceStream};
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecodeStage {
    Open,
    Probe,
    CreateDecoder,
    ReadPacket,
    DecodePacket,
    Finish,
    Analysis,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecodeFailureKind {
    Io,
    Unsupported,
    Fatal,
    /// The codec could continue with another packet, but doing so would load
    /// damaged/partial media. Our strict policy refuses the entire load.
    RecoverablePacketDamage,
    Incomplete,
    ResetRequired,
    FormatChanged,
    VerificationFailed,
    Empty,
    Cancelled,
    Capacity,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LengthEvidence {
    Declared,
    Estimated,
    Unavailable,
}

#[derive(Clone, Debug)]
pub struct DecodeDiagnostics {
    pub decoded_frames: u64,
    pub decoded_packets: u64,
    /// PCM frames, converted from container timebase when one is available.
    pub expected_frames: Option<u64>,
    pub length_evidence: LengthEvidence,
    pub verification: Option<bool>,
    pub reached_eof: bool,
    pub analysis_deferred: bool,
}

impl Default for DecodeDiagnostics {
    fn default() -> Self {
        Self {
            decoded_frames: 0,
            decoded_packets: 0,
            expected_frames: None,
            length_evidence: LengthEvidence::Unavailable,
            verification: None,
            reached_eof: false,
            analysis_deferred: false,
        }
    }
}

impl DecodeDiagnostics {
    pub fn warning(&self) -> Option<&'static str> {
        if self.analysis_deferred { return Some(if self.verification == Some(true) || self.length_evidence == LengthEvidence::Declared { "heuristic BPM analysis deferred by performance protection; manual or filename BPM is retained" } else { "heuristic BPM analysis deferred; length unverified and incomplete media cannot be ruled out" }); }
        if self.verification == Some(true) || self.length_evidence == LengthEvidence::Declared {
            None
        } else {
            Some("length unverified; incomplete media cannot be ruled out")
        }
    }
}

#[derive(Debug)]
pub struct DecodedAudio {
    pub sample: Sample,
    pub diagnostics: DecodeDiagnostics,
}

#[derive(Debug)]
pub struct DecodeFailure {
    pub kind: DecodeFailureKind,
    pub stage: DecodeStage,
    pub diagnostics: DecodeDiagnostics,
    pub detail: String,
}

impl fmt::Display for DecodeFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let reason = match self.kind {
            DecodeFailureKind::Io => "Could not read audio",
            DecodeFailureKind::Unsupported => "Unsupported audio",
            DecodeFailureKind::Fatal => "Invalid audio stream",
            DecodeFailureKind::RecoverablePacketDamage => "Damaged audio packet",
            DecodeFailureKind::Incomplete => "Incomplete audio",
            DecodeFailureKind::ResetRequired => "Unsupported stream reset",
            DecodeFailureKind::FormatChanged => "Audio format changed",
            DecodeFailureKind::VerificationFailed => "Audio checksum failed",
            DecodeFailureKind::Empty => "Empty audio",
            DecodeFailureKind::Cancelled => "Load cancelled",
            DecodeFailureKind::Capacity => "Audio exceeds its decode budget",
        };
        let activity = match self.stage {
            DecodeStage::Open => "opening the file",
            DecodeStage::Probe => "reading the format",
            DecodeStage::CreateDecoder => "preparing the decoder",
            DecodeStage::ReadPacket => "reading audio",
            DecodeStage::DecodePacket => "decoding audio",
            DecodeStage::Finish => "checking the complete stream",
            DecodeStage::Analysis => "analysing audio",
        };
        write!(
            f,
            "{reason} while {activity}: {} ({} decoded frames",
            self.detail, self.diagnostics.decoded_frames
        )?;
        if let Some(expected) = self.diagnostics.expected_frames {
            write!(f, ", expected {expected}")?;
        }
        write!(f, "; media was not loaded)")
    }
}

impl std::error::Error for DecodeFailure {}

fn failure(
    kind: DecodeFailureKind,
    stage: DecodeStage,
    diagnostics: &DecodeDiagnostics,
    detail: impl Into<String>,
) -> DecodeFailure {
    DecodeFailure {
        kind,
        stage,
        diagnostics: diagnostics.clone(),
        detail: detail.into(),
    }
}

fn codec_failure(
    error: Error,
    stage: DecodeStage,
    diagnostics: &DecodeDiagnostics,
) -> DecodeFailure {
    let kind = match &error {
        Error::ResetRequired => DecodeFailureKind::ResetRequired,
        Error::DecodeError(_) | Error::IoError(_) if stage == DecodeStage::DecodePacket => {
            DecodeFailureKind::RecoverablePacketDamage
        }
        Error::IoError(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
            DecodeFailureKind::Incomplete
        }
        Error::IoError(_) => DecodeFailureKind::Io,
        Error::Unsupported(_) => DecodeFailureKind::Unsupported,
        _ => DecodeFailureKind::Fatal,
    };
    let detail = if kind == DecodeFailureKind::ResetRequired {
        "stream reset/chained streams are unsupported; refusing the decoded prefix".into()
    } else {
        error.to_string()
    };
    failure(kind, stage, diagnostics, detail)
}

fn check_cancel(
    cancelled: &impl Fn() -> bool,
    stage: DecodeStage,
    diagnostics: &DecodeDiagnostics,
) -> Result<(), DecodeFailure> {
    if cancelled() {
        Err(failure(
            DecodeFailureKind::Cancelled,
            stage,
            diagnostics,
            "load superseded or cancelled",
        ))
    } else {
        Ok(())
    }
}

fn set_expected(diagnostics: &mut DecodeDiagnostics, params: &CodecParameters, sr: u32) {
    diagnostics.expected_frames = params.n_frames.and_then(|ticks| {
        let frames = match params.time_base {
            Some(tb) if tb.denom != 0 && tb.numer != 0 => {
                u128::from(ticks) * u128::from(tb.numer) * u128::from(sr) / u128::from(tb.denom)
            }
            Some(_) => return None,
            None => u128::from(ticks),
        };
        u64::try_from(frames).ok()
    });
    diagnostics.length_evidence = match diagnostics.expected_frames {
        None => LengthEvidence::Unavailable,
        // Symphonia's MPEG reader may estimate this from bitrate. The public
        // CodecParameters API does not identify whether Xing/VBRI supplied it.
        Some(_)
            if matches!(
                params.codec,
                CODEC_TYPE_MP1 | CODEC_TYPE_MP2 | CODEC_TYPE_MP3
            ) =>
        {
            LengthEvidence::Estimated
        }
        Some(_) => LengthEvidence::Declared,
    };
}

pub fn decode_audio(path: &Path) -> Result<DecodedAudio, DecodeFailure> {
    decode_audio_with_cancel(path, || false)
}

/// Cooperative cancellation checks do not interrupt a blocking filesystem read.
pub fn decode_audio_with_cancel(
    path: &Path,
    cancelled: impl Fn() -> bool,
) -> Result<DecodedAudio, DecodeFailure> {
    decode_with_analysis(path, cancelled, |data, ch, sr| Some(detect_bpm(data, ch, sr)))
}
pub(crate) fn decode_audio_for_show(path: &Path, cancelled: impl Fn() -> bool, performance: &super::performance::Handle) -> Result<DecodedAudio, DecodeFailure> {
    decode_with_analysis(path, cancelled, |data, ch, sr| {
        let permit = performance.optional_work().ok()?;
        let cancel = permit.cancel();
        super::dsp::detect_bpm_with_cancel(data, ch, sr, || cancel.load(std::sync::atomic::Ordering::Acquire))
    })
}
/// Foreground deck loading owns and verifies this descriptor on its media
/// worker. Preserve the existing optional BPM policy without reopening a path.
pub(crate) fn decode_deck_file(path:&Path,file:std::fs::File,cancelled:impl Fn()->bool,performance:&super::performance::Handle) -> Result<DecodedAudio,DecodeFailure> {
    decode_deck_file_progress(path,file,cancelled,performance,|_,_|{})
}
/// Decode a bounded foreground source with measured frame progress.
/// Takes its descriptor, cancellation/protection guards and progress sink; returns at most 512 MiB of decoded PCM without reopening the path.
pub(crate) fn decode_deck_file_progress(path:&Path,file:std::fs::File,cancelled:impl Fn()->bool,performance:&super::performance::Handle,progress:impl Fn(u64,Option<u64>)) -> Result<DecodedAudio,DecodeFailure> {
    decode_source(path,Some(Box::new(file)),Some(512*crate::background::MIB),cancelled,|data,ch,sr| {
        let permit=performance.optional_work().ok()?;let cancel=permit.cancel();
        super::dsp::detect_bpm_with_cancel(data,ch,sr,||cancel.load(std::sync::atomic::Ordering::Acquire))
    },true,true,progress)
}
fn decode_with_analysis(path: &Path, cancelled: impl Fn() -> bool, analyze: impl FnOnce(&[f32], u16, u32) -> Option<f32>) -> Result<DecodedAudio, DecodeFailure> {
    decode_source(path, None, None, cancelled, analyze, true, true, |_, _| {})
}

/// Sampler preparation already owns a regular, fingerprint/hash-qualified
/// descriptor and a PCM reservation. Decode that exact descriptor, never reopen
/// the pathname. The caller retains a duplicate descriptor for final identity
/// verification. This lane does not run unrelated heuristic BPM analysis.
pub(crate) fn decode_sampler_file(
    path: &Path,
    file: std::fs::File,
    pcm_bytes: u64,
    cancelled: impl Fn() -> bool,
) -> Result<DecodedAudio, DecodeFailure> {
    decode_source(path, Some(Box::new(file)), Some(pcm_bytes), cancelled, |_, _, _| None, true, true, |_, _| {})
}
/// Background analysis shares the strict decoder and exact open descriptor,
/// but computes only requested optional fields after decoding. No PCM escapes
/// the analysis worker's result boundary.
pub(crate) fn decode_analysis_file(path: &Path, file: std::fs::File,
    cancelled: impl Fn() -> bool, progress: impl Fn(u64, Option<u64>)) -> Result<DecodedAudio, DecodeFailure> {
    decode_source(path, Some(Box::new(file)), Some(crate::track_analysis::MAX_PCM_BYTES), cancelled,
        |_, _, _| None, false, true, progress)
}
/// Check a complete audio stream with bounded packet scratch and no retained PCM.
/// Takes an already verified descriptor and cancellation/progress callbacks; returns strict decode diagnostics, sample rate and channel count.
pub(crate) fn validate_audio_file(path: &Path, file: std::fs::File, cancelled: impl Fn() -> bool,
    progress: impl Fn(u64, Option<u64>)) -> Result<(DecodeDiagnostics, u32, u16), DecodeFailure> {
    let mut decoded = decode_source(path, Some(Box::new(file)), Some(64 * 1024 * 1024), cancelled,
        |_, _, _| None, false, false, progress)?;
    decoded.diagnostics.analysis_deferred = false;
    Ok((decoded.diagnostics, decoded.sample.sr, decoded.sample.ch))
}
fn decode_source(
    path: &Path,
    supplied: Option<Box<dyn MediaSource>>,
    pcm_limit: Option<u64>,
    cancelled: impl Fn() -> bool,
    analyze: impl FnOnce(&[f32], u16, u32) -> Option<f32>,
    waveform: bool,
    collect_pcm: bool,
    progress: impl Fn(u64, Option<u64>),
) -> Result<DecodedAudio, DecodeFailure> {
    let mut diagnostics = DecodeDiagnostics::default();
    check_cancel(&cancelled, DecodeStage::Open, &diagnostics)?;
    let file = supplied.map(Ok).unwrap_or_else(|| std::fs::File::open(path).map(|file| Box::new(file) as Box<dyn MediaSource>)).map_err(|error| {
        failure(
            DecodeFailureKind::Io,
            DecodeStage::Open,
            &diagnostics,
            error.to_string(),
        )
    })?;
    let stream = MediaSourceStream::new(file, Default::default());
    let mut hint = Hint::new();
    if let Some(extension) = path.extension().and_then(|s| s.to_str()) {
        hint.with_extension(extension);
    }
    check_cancel(&cancelled, DecodeStage::Probe, &diagnostics)?;
    let mut format = symphonia::default::get_probe()
        .format(
            &hint,
            stream,
            &FormatOptions {
                enable_gapless: true,
                ..Default::default()
            },
            &MetadataOptions::default(),
        )
        .map_err(|error| codec_failure(error, DecodeStage::Probe, &diagnostics))?
        .format;
    check_cancel(&cancelled, DecodeStage::CreateDecoder, &diagnostics)?;
    let track = format.default_track().ok_or_else(|| {
        failure(
            DecodeFailureKind::Unsupported,
            DecodeStage::Probe,
            &diagnostics,
            "no audio track",
        )
    })?;
    let id = track.id;
    let mut params = track.codec_params.clone();
    if let Some(sr) = params.sample_rate {
        set_expected(&mut diagnostics, &params, sr);
    }
    if let Some(limit) = pcm_limit {
        let ch = params.channels.map(|channels| channels.count() as u64);
        if params.sample_rate.is_some_and(|rate| rate == 0 || rate > crate::project_file::MAX_SAMPLE_RATE)
            || ch.is_some_and(|channels| channels == 0 || channels > u64::from(crate::project_file::MAX_CHANNELS))
            || ch.zip(diagnostics.expected_frames).is_some_and(|(channels, frames)| {
                collect_pcm && diagnostics.length_evidence == LengthEvidence::Declared
                    && frames.saturating_mul(channels).saturating_mul(4) > limit
            })
        {
            return Err(failure(DecodeFailureKind::Capacity, DecodeStage::CreateDecoder,
                &diagnostics, "declared audio exceeds the reserved sampler PCM/channel/rate limit"));
        }
    }
    let mut decoder = symphonia::default::get_codecs()
        .make(&params, &DecoderOptions { verify: true })
        .map_err(|error| codec_failure(error, DecodeStage::CreateDecoder, &diagnostics))?;
    let mut data = Vec::new();
    let mut spec = None;
    let mut packet_end = 0;
    loop {
        check_cancel(&cancelled, DecodeStage::ReadPacket, &diagnostics)?;
        let next = format.next_packet();
        check_cancel(&cancelled, DecodeStage::ReadPacket, &diagnostics)?;
        let packet = match next {
            Ok(packet) => packet,
            Err(Error::IoError(error)) if error.kind() == std::io::ErrorKind::UnexpectedEof => {
                diagnostics.reached_eof = true;
                break;
            }
            Err(error) => return Err(codec_failure(error, DecodeStage::ReadPacket, &diagnostics)),
        };
        if packet.track_id() != id {
            continue;
        }
        let expected_start = if diagnostics.decoded_packets == 0 {
            params.start_ts
        } else {
            packet_end
        };
        let fully_trimmed = packet.dur == 0 && (packet.trim_start > 0 || packet.trim_end > 0);
        if packet.ts > expected_start && !fully_trimmed {
            return Err(failure(
                DecodeFailureKind::Incomplete,
                DecodeStage::ReadPacket,
                &diagnostics,
                "missing audio between packet timestamps",
            ));
        }
        check_cancel(&cancelled, DecodeStage::DecodePacket, &diagnostics)?;
        let decoded = decoder.decode(&packet);
        check_cancel(&cancelled, DecodeStage::DecodePacket, &diagnostics)?;
        let buffer = decoded
            .map_err(|error| codec_failure(error, DecodeStage::DecodePacket, &diagnostics))?;
        if fully_trimmed && buffer.frames() != 0 {
            return Err(failure(DecodeFailureKind::Fatal, DecodeStage::DecodePacket,
                &diagnostics, "fully trimmed packet returned audible frames"));
        }
        let current = *buffer.spec();
        if current.rate == 0 || current.channels.count() == 0 {
            return Err(failure(
                DecodeFailureKind::Fatal,
                DecodeStage::DecodePacket,
                &diagnostics,
                "decoded audio has no sample rate or channels",
            ));
        }
        if spec.is_some_and(|previous| previous != current) {
            return Err(failure(
                DecodeFailureKind::FormatChanged,
                DecodeStage::DecodePacket,
                &diagnostics,
                "sample rate or channel layout changed; refusing mixed-format audio",
            ));
        }
        spec = Some(current);
        if let Some(limit) = pcm_limit {
            let channels = current.channels.count() as u64;
            let added = (buffer.frames() as u64).saturating_mul(channels);
            let required = (data.len() as u64).saturating_add(added).saturating_mul(4);
            // Bound the conversion scratch buffer too. Decoder internals are
            // supplied by Symphonia; our owned PCM never grows past its credit.
            if current.rate > crate::project_file::MAX_SAMPLE_RATE
                || channels > u64::from(crate::project_file::MAX_CHANNELS)
                || collect_pcm && required > limit
                || (buffer.capacity() as u64).saturating_mul(channels).saturating_mul(4) > limit
            {
                return Err(failure(DecodeFailureKind::Capacity, DecodeStage::DecodePacket,
                    &diagnostics, "decoded audio exceeds the reserved sampler PCM/channel/rate limit"));
            }
            let added = usize::try_from(added).map_err(|_| failure(DecodeFailureKind::Capacity,
                DecodeStage::DecodePacket, &diagnostics, "decoded packet length overflow"))?;
            let required_samples = data.len().saturating_add(added);
            if collect_pcm && required_samples > data.capacity() {
                let ceiling = usize::try_from(limit / 4).unwrap_or(usize::MAX);
                let target = required_samples.max(data.capacity().saturating_mul(2)).min(ceiling);
                data.try_reserve_exact(target - data.len()).map_err(|_| failure(DecodeFailureKind::Capacity,
                    DecodeStage::DecodePacket, &diagnostics, "could not reserve bounded sampler PCM"))?;
            }
            if data.capacity() as u64 * 4 > limit {
                return Err(failure(DecodeFailureKind::Capacity, DecodeStage::DecodePacket,
                    &diagnostics, "allocator capacity exceeds the reserved sampler PCM limit"));
            }
        }
        let mut samples = SampleBuffer::<f32>::new(buffer.capacity() as u64, current);
        samples.copy_interleaved_ref(buffer);
        if samples.samples().iter().any(|sample| !sample.is_finite()) {
            return Err(failure(
                DecodeFailureKind::RecoverablePacketDamage,
                DecodeStage::DecodePacket,
                &diagnostics,
                "non-finite PCM samples",
            ));
        }
        diagnostics.decoded_frames += (samples.samples().len() / current.channels.count()) as u64;
        progress(diagnostics.decoded_frames, diagnostics.expected_frames);
        diagnostics.decoded_packets += 1;
        if !fully_trimmed { packet_end = packet_end.max(packet.ts.saturating_add(packet.dur)); }
        if collect_pcm { data.extend_from_slice(samples.samples()); }
    }
    check_cancel(&cancelled, DecodeStage::Finish, &diagnostics)?;
    let spec = spec.filter(|_| diagnostics.decoded_frames > 0).ok_or_else(|| {
        failure(
            if diagnostics.expected_frames.is_some_and(|frames| frames > 0) {
                DecodeFailureKind::Incomplete
            } else {
                DecodeFailureKind::Empty
            },
            DecodeStage::Finish,
            &diagnostics,
            "no decodable audio frames",
        )
    })?;
    // Some readers discover duration while traversing the stream (not at probe).
    if let Some(track) = format.tracks().iter().find(|track| track.id == id) {
        params = track.codec_params.clone();
    }
    set_expected(&mut diagnostics, &params, spec.rate);
    if diagnostics.length_evidence == LengthEvidence::Declared {
        if let Some(expected) = params.n_frames {
            if packet_end < params.start_ts.saturating_add(expected)
                || diagnostics.decoded_frames < diagnostics.expected_frames.unwrap_or(0)
            {
                return Err(failure(
                    DecodeFailureKind::Incomplete,
                    DecodeStage::Finish,
                    &diagnostics,
                    "end of file before the declared audio duration",
                ));
            }
        }
    }
    diagnostics.verification = decoder.finalize().verify_ok;
    if diagnostics.verification == Some(false) {
        return Err(failure(
            DecodeFailureKind::VerificationFailed,
            DecodeStage::Finish,
            &diagnostics,
            "decoded audio checksum does not match the stream",
        ));
    }
    check_cancel(&cancelled, DecodeStage::Analysis, &diagnostics)?;
    let ch = spec.channels.count() as u16;
    let peaks = if waveform {
        peaks_3band_with_cancel(&data, ch, 2048, &cancelled).ok_or_else(||
            failure(DecodeFailureKind::Cancelled, DecodeStage::Analysis, &diagnostics, "decode cancelled"))?
    } else { Vec::new() };
    let spectrum = if waveform {
        Some(std::sync::Arc::new(super::waveform::Waveform::analyze(&data, ch, spec.rate, &cancelled)
            .ok_or_else(|| failure(DecodeFailureKind::Cancelled, DecodeStage::Analysis, &diagnostics, "waveform analysis cancelled"))?))
    } else { None };
    check_cancel(&cancelled, DecodeStage::Analysis, &diagnostics)?;
    let analysis = analyze(&data, ch, spec.rate);
    diagnostics.analysis_deferred = analysis.is_none();
    let bpm = analysis.unwrap_or(0.0);
    check_cancel(&cancelled, DecodeStage::Analysis, &diagnostics)?;
    Ok(DecodedAudio {
        diagnostics,
        sample: Sample { spectrum,
            name: path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("track")
                .into(),
            sr: spec.rate,
            ch,
            data,
            peaks: peaks.into(),
            bpm,
            path: path.to_string_lossy().into(),
        },
    })
}

#[cfg(test)]
mod tests;

/// Decode transient provider bytes without creating a local media identity.
/// Takes bounded MP3 bytes and cancellation; returns checked PCM with an empty local path.
pub(crate) fn decode_provider_bytes(bytes: Vec<u8>, cancelled: impl Fn() -> bool) -> Result<DecodedAudio, DecodeFailure> {
    let mut result = decode_source(Path::new("provider.mp3"), Some(Box::new(std::io::Cursor::new(bytes))),
        Some(128 * 1024 * 1024), cancelled, |_, _, _| None, false, true, |_, _| {})?;
    result.sample.path.clear();
    Ok(result)
}
