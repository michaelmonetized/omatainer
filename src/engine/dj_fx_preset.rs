use crate::engine::{session::Reference, surface_controls::fx::{Placement, Timing}, FxKind};
use serde::{Deserialize, Serialize};
use std::{path::Path, sync::atomic::AtomicBool};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Name { bytes: [u8; 80], length: u8 }
impl Name {
    /// Prepare a bounded performance label.
    /// Takes visible UTF-8 text; returns fixed callback-safe bytes or an invalid-label error.
    pub fn new(value: &str) -> Result<Self, String> {
        if value.is_empty() || value.len() > 80 || value.trim() != value || value.chars().any(char::is_control) { return Err("Use a visible name of at most 80 bytes without leading or trailing spaces".into()); }
        let mut result = Self { bytes: [0; 80], length: value.len() as u8 }; result.bytes[..value.len()].copy_from_slice(value.as_bytes()); Ok(result)
    }
    /// Read the prepared label.
    /// Takes this fixed name; returns its validated UTF-8 text without allocation.
    pub fn as_str(&self) -> &str { std::str::from_utf8(&self.bytes[..usize::from(self.length)]).expect("Prepared UTF-8 label") }
}
impl Serialize for Name { fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> { serializer.serialize_str(self.as_str()) } }
impl<'de> Deserialize<'de> for Name { fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> { let text = String::deserialize(deserializer)?; Self::new(&text).map_err(serde::de::Error::custom) } }

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Settings {
    pub name: Name,
    pub kinds: [FxKind; 3],
    pub wet: [f32; 3],
    pub on: [bool; 3],
    pub parameter: [f32; 3],
    pub beats: i8,
    pub assigned: [bool; 2],
    pub sampler: Option<Reference>,
    pub master: bool,
    pub placement: Placement,
    pub timing: Timing,
    pub manual_ms: f32,
}
impl Settings {
    /// Check saved unit controls before publication or recall.
    /// Takes this fixed settings object; returns whether its numbers and stable route shape are supported.
    pub fn valid(&self) -> bool {
        self.wet.iter().chain(&self.parameter).all(|value| value.is_finite() && (0.0..=1.0).contains(value))
            && (-4..=3).contains(&self.beats) && self.manual_ms.is_finite() && (1.0..=1998.0).contains(&self.manual_ms)
            && self.sampler.is_none_or(|target| target.namespace != [0, 0] && target.id.0 != 0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Preset { pub version: u32, pub name: Name, pub units: [Settings; 2] }
impl Preset {
    /// Validate a portable layout before file I/O or admission.
    /// Takes this preset; returns success for version 1 and supported controls.
    pub fn validate(&self) -> Result<(), String> {
        if self.version != 1 || self.units.iter().any(|unit| !unit.valid()) { Err("Unsupported or invalid DJ effect preset; current effects are preserved".into()) } else { Ok(()) }
    }
}
fn limits() -> crate::project_file::Limits { crate::project_file::Limits { max_metadata_bytes: 64 * 1024, max_media: 0, max_pcm_bytes: 0 } }
/// Save a new portable layout without replacing a file.
/// Takes path, validated preset and cancellation; returns durable commit status or an I/O error.
pub(crate) fn save(path: &Path, preset: Preset, cancelled: &AtomicBool) -> Result<crate::project_file::SaveOutcome, String> {
    preset.validate()?;
    crate::project_file::save(path, &crate::project_file::Bundle { state: preset, media: vec![] }, crate::project_file::Overwrite::Never, &limits(), cancelled).map_err(|error| error.to_string())
}
/// Inspect a bounded portable layout outside audio ownership.
/// Takes path and cancellation; returns validated controls or a format/I/O error.
pub(crate) fn load(path: &Path, cancelled: &AtomicBool) -> Result<Preset, String> {
    let bundle = crate::project_file::load::<Preset>(path, &limits(), cancelled).map_err(|error| error.to_string())?;
    bundle.state.validate()?; Ok(bundle.state)
}

impl From<crate::engine::surface_controls::EffectBank> for Settings {
    fn from(bank: crate::engine::surface_controls::EffectBank) -> Self {
        Self { name: bank.name, kinds: bank.kinds, wet: bank.wet, on: bank.on, parameter: bank.parameter, beats: bank.beats, assigned: bank.assigned, sampler: bank.sampler, master: bank.master, placement: bank.placement, timing: bank.timing, manual_ms: bank.manual_ms }
    }
}
impl Settings {
    /// Prepare one fixed unit with no retained histories.
    /// Takes validated saved controls; returns the corresponding renderer bank with all tail flags reset.
    pub(super) fn bank(self) -> crate::engine::surface_controls::EffectBank {
        crate::engine::surface_controls::EffectBank { name: self.name, kinds: self.kinds, wet: self.wet, on: self.on, parameter: self.parameter, beats: self.beats, assigned: self.assigned, sampler: self.sampler, master: self.master, placement: self.placement, timing: self.timing, manual_ms: self.manual_ms, tails: [false; 4] }
    }
}
#[cfg(test)]
mod tests;
