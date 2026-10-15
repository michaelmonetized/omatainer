use super::process;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::AtomicBool,
};
mod api;
#[cfg(test)]
mod tests;
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct Tool {
    pub path: PathBuf,
    pub sha256: String,
    pub version: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct Report {
    pub standard: String,
    pub integrated_lufs: Option<f64>,
    pub short_term_max_lufs: Option<f64>,
    pub momentary_max_lufs: Option<f64>,
    pub lra_lu: Option<f64>,
    pub sample_peak_dbfs: Option<f64>,
    pub true_peak_dbtp: Option<f64>,
    pub full_scale_samples: u64,
    pub frames: u64,
    pub rate: u32,
    pub channels: u16,
    pub wave_mask: Option<u32>,
    pub loudness_supported: bool,
    pub measured_final_encoded_audio: bool,
    pub decoded_pcm_sha256: String,
    pub meter: Tool,
    pub decoder: Tool,
    pub probe: Tool,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum Policy {
    #[default]
    Report,
    Warn,
    Block,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct Targets {
    pub policy: Policy,
    pub integrated_lufs: Option<f64>,
    pub loudness_tolerance: f64,
    pub max_true_peak: f64,
    pub max_lra: Option<f64>,
}
impl Default for Targets {
    fn default() -> Self {
        Self {
            policy: Policy::Report,
            integrated_lufs: None,
            loudness_tolerance: 0.5,
            max_true_peak: -1.,
            max_lra: None,
        }
    }
}
impl Targets {
    /// Validate explicit QC targets.
    /// Args: requested target and warn/block policy. Returns an error for nonfinite or unsupported ranges, without changing audio gain.
    pub fn validate(&self) -> Result<(), String> {
        if self
            .integrated_lufs
            .is_some_and(|v| !v.is_finite() || !(-70.0..=0.).contains(&v))
            || !self.loudness_tolerance.is_finite()
            || !(0.1..=10.).contains(&self.loudness_tolerance)
            || !self.max_true_peak.is_finite()
            || !(-30.0..=6.).contains(&self.max_true_peak)
            || self
                .max_lra
                .is_some_and(|v| !v.is_finite() || !(0.0..=70.).contains(&v))
        {
            return Err("QC targets need finite supported LUFS/LRA/true-peak values".into());
        }
        Ok(())
    }
    /// Evaluate measured encoded audio without normalization.
    /// Args: final-file report. Returns observable target violations; block policy is enforced before atomic publication.
    pub fn violations(&self, r: &Report) -> Result<Vec<String>, String> {
        self.validate()?;
        let mut out = Vec::new();
        if let Some(target) = self.integrated_lufs {
            match r.integrated_lufs{Some(v)if(v-target).abs()<=self.loudness_tolerance=>{},Some(v)=>out.push(format!("Integrated{v:.2}LUFS differs from target{target:.2}±{:.2}",self.loudness_tolerance)),None=>out.push("Integrated target cannot be qualified: silence, short duration or unknown speaker layout".into())}
        }
        if r.true_peak_dbtp.is_some_and(|v| v > self.max_true_peak) {
            out.push(format!("True peak exceeds{:.2}dBTP", self.max_true_peak));
        }
        if let Some(max) = self.max_lra {
            match r.lra_lu {
                Some(v) if v <= max => {}
                Some(v) => out.push(format!("LRA{v:.2}LU exceeds{max:.2}")),
                None => out.push(
                    "LRA target cannot be qualified for short/silent/unknown-layout audio".into(),
                ),
            }
        }
        if r.full_scale_samples > 0 {
            out.push(format!(
                "{} samples reach or exceed digital full scale",
                r.full_scale_samples
            ));
        }
        Ok(out)
    }
}
fn db(v: f64) -> Option<f64> {
    (v > 0.).then(|| 20. * v.log10())
}
fn maximum(current: &mut Option<f64>, v: Option<f64>) {
    if let Some(v) = v {
        *current = Some(current.map_or(v, |old| old.max(v)));
    }
}
fn tool(name: &str, cancel: &AtomicBool) -> Result<Tool, String> {
    let path = process::executable(name)?;
    let output = process::capture(Command::new(&path).arg("-version"), cancel)?;
    Ok(Tool {
        sha256: process::sha256_cancelled(&path, cancel)?,
        path,
        version: String::from_utf8_lossy(&output).lines().take(1).collect(),
    })
}
/// Measure the actual final encoded file with a separate native meter.
/// Args: file, expected exact rate/width, canonical optional mask and cancellation. Returns finite JSON-safe EBU/BS1770 metrics from decoded output, including post-lossy peaks; no gain is applied.
pub(crate) fn measure(
    path: &Path,
    rate: u32,
    channels: u16,
    mask: Option<u32>,
    cancel: &AtomicBool,
) -> Result<Report, String> {
    if !(8000..=192000).contains(&rate) || !(1..=64).contains(&channels) {
        return Err("QC rate/channel count exceeds delivery admission".into());
    }
    let decoder = tool("ffmpeg", cancel)?;
    let probe = tool("ffprobe", cancel)?;
    let info = process::capture(
        Command::new(&probe.path)
            .args([
                "-v",
                "error",
                "-select_streams",
                "a:0",
                "-show_entries",
                "stream=channels,sample_rate",
                "-of",
                "json",
            ])
            .arg(path),
        cancel,
    )?;
    let info: serde_json::Value = serde_json::from_slice(&info).map_err(|e| e.to_string())?;
    if info["streams"][0]["channels"].as_u64() != Some(u64::from(channels))
        || info["streams"][0]["sample_rate"]
            .as_str()
            .and_then(|v| v.parse::<u32>().ok())
            != Some(rate)
    {
        return Err("Final encoded file changed the reviewed sample rate or channel count".into());
    }
    let (meter, supported) = api::Meter::new(rate, channels, mask, cancel)?;
    let mut pending = Vec::new();
    let mut pcm = Sha256::new();
    let mut frames = 0_u64;
    let mut full_scale = 0_u64;
    let mut short = None;
    let mut momentary = None;
    let width = usize::from(channels);
    let step = u64::from(rate) / 10;
    let mut boundary = step;
    process::stream(
        Command::new(&decoder.path)
            .args([
                "-v",
                "error",
                "-nostdin",
                "-threads",
                "1",
                "-protocol_whitelist",
                "file,pipe",
                "-format_whitelist",
                "wav,aiff,flac,mp3",
                "-i",
            ])
            .arg(path)
            .args([
                "-map",
                "0:a:0",
                "-vn",
                "-sn",
                "-dn",
                "-c:a",
                "pcm_f32le",
                "-f",
                "f32le",
                "pipe:1",
            ]),
        cancel,
        false,
        |bytes| {
            pending.extend_from_slice(bytes);
            let complete = pending.len() / (width * 4) * (width * 4);
            let samples: Vec<f32> = pending[..complete]
                .chunks_exact(4)
                .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
                .collect();
            if samples.iter().any(|v| !v.is_finite()) {
                return Err("Final encoded file decodes to nonfinite audio; no publication".into());
            }
            pcm.update(&pending[..complete]);
            full_scale += samples.iter().filter(|v| v.abs() >= 1.).count() as u64;
            let mut offset = 0;
            while offset < samples.len() {
                let n = (boundary - frames).min(((samples.len() - offset) / width) as u64) as usize;
                meter.add(&samples[offset..offset + n * width], width)?;
                frames += n as u64;
                offset += n * width;
                if frames == boundary {
                    if supported && frames >= u64::from(rate) * 3 {
                        maximum(&mut short, meter.metric(2)?);
                    }
                    if supported && frames >= u64::from(rate) * 4 / 10 {
                        maximum(&mut momentary, meter.metric(3)?);
                    }
                    boundary += step;
                }
            }
            pending.drain(..complete);
            Ok(())
        },
    )?;
    if !pending.is_empty() || frames == 0 {
        return Err("Final decoder ended without complete nonempty channel frames".into());
    }
    let integrated = if supported && frames >= u64::from(rate) * 4 / 10 {
        meter.metric(0)?
    } else {
        None
    };
    let lra = if supported && frames >= u64::from(rate) * 3 && integrated.is_some() {
        meter.metric(1)?
    } else {
        None
    };
    let mut sample = 0_f64;
    for c in 0..u32::from(channels) {
        sample = sample.max(meter.peak(c, false)?);
    }
    meter.add(&vec![0.; 64 * width], width)?;
    let mut peak = 0_f64;
    for c in 0..u32::from(channels) {
        peak = peak.max(meter.peak(c, true)?);
    }
    Ok(Report{standard:"ITU-R BS.1770-4 / EBU R128; LRA EBU Tech3342; libebur128 implementation-defined polyphase true peak (4x below96kHz,2x below192kHz,1x at192kHz),64-frame zero extension for peak tail only".into(),integrated_lufs:integrated,short_term_max_lufs:short,momentary_max_lufs:momentary,lra_lu:lra,sample_peak_dbfs:db(sample),true_peak_dbtp:db(peak),full_scale_samples:full_scale,frames,rate,channels,wave_mask:mask,loudness_supported:supported,measured_final_encoded_audio:true,decoded_pcm_sha256:format!("{:x}",pcm.finalize()),meter:meter_tool(&meter.api.version,cancel)?,decoder,probe})
}

fn meter_tool(version: &str, cancel: &AtomicBool) -> Result<Tool, String> {
    static IDENTITY: std::sync::OnceLock<Tool> = std::sync::OnceLock::new();
    super::active(cancel)?;
    if let Some(identity) = IDENTITY.get() {
        return Ok(identity.clone());
    }
    let identity = Tool {
        path: std::env::current_exe().map_err(|e| e.to_string())?,
        sha256: process::sha256_cancelled(Path::new("/proc/self/exe"), cancel)?,
        version: version.to_owned(),
    };
    let _ = IDENTITY.set(identity.clone());
    Ok(identity)
}
/// Review exact delivery runtime availability without opening devices.
/// Args: cancellation. Returns bundled meter source/ABI and actual decoder/probe identities, or a useful missing-tool error.
pub(crate) fn preflight(cancel: &AtomicBool) -> Result<serde_json::Value, String> {
    let api = api::Api::open()?;
    let (meter, valid) = api::Meter::new(48000, 2, Some(3), cancel)?;
    if !valid {
        return Err("Bundled stereo meter refused standard layout".into());
    }
    drop(meter);
    Ok(
        serde_json::json!({"meter":meter_tool(&api.version,cancel)?,"ffmpeg":tool("ffmpeg",cancel)?,"ffprobe":tool("ffprobe",cancel)?,"native_meter_bundled":true,"system_libebur128_required":false,"source_sha256":env!("EBUR128_SOURCE_SHA256")}),
    )
}
