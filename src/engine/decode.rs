//! File decoding is transactional: detected damage never produces a loadable
//! partial Sample. Unknown/estimated lengths remain explicitly unverified.

use super::dsp::{detect_bpm, peaks_3band, Sample};
use std::fmt;
use std::path::Path;
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{
    CodecParameters, DecoderOptions, CODEC_TYPE_MP1, CODEC_TYPE_MP2, CODEC_TYPE_MP3,
};
use symphonia::core::errors::Error;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
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
        }
    }
}

impl DecodeDiagnostics {
    pub fn warning(&self) -> Option<&'static str> {
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
    let mut diagnostics = DecodeDiagnostics::default();
    check_cancel(&cancelled, DecodeStage::Open, &diagnostics)?;
    let file = std::fs::File::open(path).map_err(|error| {
        failure(
            DecodeFailureKind::Io,
            DecodeStage::Open,
            &diagnostics,
            error.to_string(),
        )
    })?;
    let stream = MediaSourceStream::new(Box::new(file), Default::default());
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
        if packet.ts > expected_start {
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
        diagnostics.decoded_packets += 1;
        packet_end = packet_end.max(packet.ts.saturating_add(packet.dur));
        data.extend_from_slice(samples.samples());
    }
    check_cancel(&cancelled, DecodeStage::Finish, &diagnostics)?;
    let spec = spec.filter(|_| !data.is_empty()).ok_or_else(|| {
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
    let peaks = peaks_3band(&data, ch, 2048);
    check_cancel(&cancelled, DecodeStage::Analysis, &diagnostics)?;
    let bpm = detect_bpm(&data, ch, spec.rate);
    check_cancel(&cancelled, DecodeStage::Analysis, &diagnostics)?;
    Ok(DecodedAudio {
        diagnostics,
        sample: Sample {
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
