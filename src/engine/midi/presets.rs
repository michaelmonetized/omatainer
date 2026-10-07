//! Portable named overrides target a port only after explicit review.
use super::{
    learn::{Config, Endpoint, Mapping},
    Binding, MidiMap, UnmappedNotes,
};
use serde::{Deserialize, Serialize};

pub(crate) const VERSION: u32 = 5;
pub(crate) const MAX_PRESETS: usize = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Layer {
    FactoryOverlay,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Preset {
    pub version: u32,
    pub name: String,
    pub device_hint: String,
    pub revision_hint: String,
    pub layer: Layer,
    pub bindings: Vec<Binding>,
}

/// Validate a visible portable label.
/// Takes text and its byte limit; returns an error for blank, padded or control text.
fn visible(text: &str, max: usize) -> Result<(), String> {
    if text.is_empty()
        || text.trim() != text
        || text.len() > max
        || text.chars().any(char::is_control)
    {
        return Err(format!(
            "Preset labels need 1–{max} visible bytes without surrounding spaces"
        ));
    }
    Ok(())
}
impl Preset {
    /// Capture only the selected port's override layer.
    /// Takes a name, descriptive revision, exact source port and active config; returns a portable validated definition.
    pub(crate) fn capture(
        name: String,
        revision_hint: String,
        port: &Endpoint,
        config: &Config,
    ) -> Result<Self, String> {
        config.validate()?;
        let preset = Self {
            version: VERSION,
            name,
            device_hint: port.name.clone(),
            revision_hint,
            layer: Layer::FactoryOverlay,
            bindings: config
                .mappings
                .iter()
                .filter(|row| &row.endpoint == port)
                .map(|row| row.binding)
                .collect(),
        };
        preset.validate()?;
        Ok(preset)
    }
    /// Decode without applying a mapping or retaining a backend identity.
    /// Takes bounded JSON bytes; returns a strict versioned definition or a complete refusal.
    pub(crate) fn decode(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() > 65_536 {
            return Err("MIDI preset exceeds 64 KiB".into());
        }
        let raw: serde_json::Value = serde_json::from_slice(bytes).map_err(|error| error.to_string())?;
        if raw.get("version").and_then(|v| v.as_u64()) == Some(1)
            && raw.get("bindings").and_then(|v| v.as_array()).is_some_and(|bindings| bindings.iter().any(super::controls::new_fields)) {
            return Err("Encoder controls require MIDI preset version 2".into());
        }
        let preset: Self = serde_json::from_slice(bytes).map_err(|error| error.to_string())?;
        preset.validate()?;
        Ok(preset)
    }
    /// Validate supported actions and distinct wire addresses.
    /// Takes this definition; returns an error without changing any active input.
    pub(crate) fn validate(&self) -> Result<(), String> {
        if !matches!(self.version, 1 | 2 | 3 | 4 | VERSION) {
            return Err("Unsupported MIDI preset version; current mappings retained".into());
        }
        visible(&self.name, 80)?;
        visible(&self.device_hint, 256)?;
        if !self.revision_hint.is_empty() {
            visible(&self.revision_hint, 80)?;
        }
        if self.bindings.len() > super::learn::MAX_MAPPINGS {
            return Err("Keep at most 256 assignments per preset".into());
        }
        for binding in &self.bindings {
            if self.version < 5 && super::learn::deck_pad(binding.action) { return Err("Deck pad modes require MIDI preset version 5".into()); }
            if self.version < 4 && super::learn::saved_loop(binding.action) { return Err("Saved loop assignments require MIDI preset version 4".into()); }
            if self.version < 3 && super::learn::navigation(binding.action) { return Err("Song navigation requires MIDI preset version 3".into()); }
            if self.version == 1 && super::controls::modern(*binding) { return Err("Encoder controls require MIDI preset version 2".into()); }
            super::learn::validate_binding(binding)?;
        }
        MidiMap {
            name: self.name.clone(),
            matchers: vec![],
            bindings: self.bindings.clone(),
            unmapped_notes: UnmappedNotes::Ignore,
        }
        .validate()
        .map_err(|error| error.to_string())?;
        if serde_json::to_vec_pretty(self)
            .map_err(|error| error.to_string())?
            .len()
            > 65_536
        {
            return Err("MIDI preset exceeds 64 KiB".into());
        }
        Ok(())
    }
    /// Replace one exact port's overrides while preserving all other ports.
    /// Takes a deliberately selected destination and current config; returns the complete validated candidate.
    pub(crate) fn target(&self, port: &Endpoint, config: &Config) -> Result<Config, String> {
        self.validate()?;
        let mut next = defaults(port, config);
        next.mappings
            .extend(self.bindings.iter().map(|binding| Mapping {
                endpoint: port.clone(),
                binding: *binding,
            }));
        next.validate()?;
        Ok(next)
    }
}
/// Remove only a selected port's learned overrides.
/// Takes an exact port and complete config; returns a candidate that resumes its existing factory behavior.
pub(crate) fn defaults(port: &Endpoint, config: &Config) -> Config {
    Config {
        mappings: config
            .mappings
            .iter()
            .filter(|row| &row.endpoint != port)
            .cloned()
            .collect(),
    }
}
/// Validate a complete saved preset bank.
/// Takes definitions; rejects duplicate names and bounded-storage overflow before persistence.
pub(crate) fn validate_bank(bank: &[Preset]) -> Result<(), String> {
    if bank.len() > MAX_PRESETS
        || bank
            .iter()
            .map(|preset| preset.bindings.len())
            .sum::<usize>()
            > 2048
    {
        return Err("Keep at most 32 presets and 2048 saved preset assignments per profile".into());
    }
    for (index, preset) in bank.iter().enumerate() {
        preset.validate()?;
        if bank[..index].iter().any(|other| other.name == preset.name) {
            return Err("Choose distinct MIDI preset names".into());
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests;
