//! Source-qualified background analysis values. Waveform payloads live in a
//! separate bounded cache; catalog records contain only verified references.
use serde::{Deserialize, Serialize};

pub(crate) const ALGORITHM: u32 = 1;
pub(crate) const MAX_WAVEFORM_BINS: usize = 2048;
pub(crate) const MAX_WAVEFORM_BYTES: u32 = 256 * 1024;
pub(crate) const MAX_PCM_BYTES: u64 = 1024 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Fields {
    pub bpm: bool,
    pub duration: bool,
    pub waveform: bool,
}
impl Fields {
    pub const ALL: Self = Self { bpm: true, duration: true, waveform: true };
    pub fn valid(self) -> bool { self.bpm || self.duration || self.waveform }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Measured<T> {
    pub algorithm: u32,
    pub at_unix_ms: u64,
    pub value: T,
}
impl<T> Measured<T> {
    fn new(value: T, at_unix_ms: u64) -> Self {
        Self { algorithm: ALGORITHM, at_unix_ms, value }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WaveformRef {
    pub sha256: [u8; 32],
    pub bytes: u32,
    pub frames: u64,
    pub sample_rate: u32,
    pub channels: u16,
    pub bins: u16,
}
impl WaveformRef {
    pub fn valid(&self) -> bool {
        self.bytes > 0 && self.bytes <= MAX_WAVEFORM_BYTES
            && self.frames > 0
            && (1..=crate::project_file::MAX_SAMPLE_RATE).contains(&self.sample_rate)
            && (1..=crate::project_file::MAX_CHANNELS).contains(&self.channels)
            && (1..=MAX_WAVEFORM_BINS).contains(&usize::from(self.bins))
            && self.frames.checked_mul(u64::from(self.channels)).and_then(|v| v.checked_mul(4))
                .is_some_and(|bytes| bytes <= MAX_PCM_BYTES)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Record {
    // Some(Measured { value: None }) means analysis completed without a usable
    // tempo estimate. None means this field has not been analyzed.
    pub bpm: Option<Measured<Option<f32>>>,
    pub duration: Option<Measured<f64>>,
    pub waveform: Option<Measured<WaveformRef>>,
}
impl Record {
    pub fn valid(&self) -> bool {
        (self.bpm.is_some() || self.duration.is_some() || self.waveform.is_some())
            && self.bpm.as_ref().is_none_or(|m| m.algorithm == ALGORITHM && valid_bpm(m.value))
            && self.duration.as_ref().is_none_or(|m| m.algorithm == ALGORITHM && valid_duration(m.value))
            && self.waveform.as_ref().is_none_or(|m| m.algorithm == ALGORITHM && m.value.valid())
    }
    pub fn contains(&self, fields: Fields) -> bool {
        fields.valid() && self.valid()
            && (!fields.bpm || self.bpm.is_some())
            && (!fields.duration || self.duration.is_some())
            && (!fields.waveform || self.waveform.is_some())
    }
    pub fn merged(&self, patch: &Patch) -> Result<Self, String> {
        patch.validate()?;
        let mut next = self.clone();
        if patch.fields.bpm { next.bpm = Some(Measured::new(patch.bpm, patch.at_unix_ms)); }
        if patch.fields.duration { next.duration = Some(Measured::new(patch.duration, patch.at_unix_ms)); }
        if patch.fields.waveform { next.waveform = Some(Measured::new(patch.waveform.clone().unwrap(), patch.at_unix_ms)); }
        if !next.valid() { return Err("invalid combined track analysis record".into()); }
        Ok(next)
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Patch {
    /// A fresh same-descriptor decode/hash proof, never a serialized assertion.
    pub reference: crate::sampler_bank::SourceRef,
    pub fields: Fields,
    pub at_unix_ms: u64,
    pub bpm: Option<f32>,
    pub duration: f64,
    pub waveform: Option<WaveformRef>,
}
impl Patch {
    pub fn validate(&self) -> Result<(), String> {
        self.reference.validate()?;
        if self.reference.content_hash.is_none() || !self.fields.valid()
            || !valid_bpm(self.bpm) || !valid_duration(self.duration)
            || self.fields.waveform != self.waveform.is_some()
            || self.waveform.as_ref().is_some_and(|wave| !wave.valid()
                || ((wave.frames as f64 / f64::from(wave.sample_rate)) - self.duration).abs() > 1e-6)
        {
            return Err("invalid or unqualified track analysis result".into());
        }
        Ok(())
    }
}
fn valid_bpm(value: Option<f32>) -> bool {
    value.is_none_or(|value| value.is_finite() && (70.0..=180.0).contains(&value))
}
fn valid_duration(value: f64) -> bool { value.is_finite() && value > 0.0 && value <= 1.0e10 }
