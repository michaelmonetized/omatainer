//! Immutable working-bank payloads, prepared outside the renderer.
use super::{BankId, Controls, Definition, Slot, Source, SLOTS};
use crate::engine::dsp::Sample;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Settings {
    pub name: String,
    /// Provenance only. It never identifies a working project instance.
    pub definition: Option<BankId>,
    pub slots: [Slot; SLOTS],
}
impl Settings {
    pub fn empty(name: String) -> Result<Self, String> {
        let value = Self {
            name,
            definition: None,
            slots: std::array::from_fn(|_| Slot::default()),
        };
        value.validate()?;
        Ok(value)
    }
    pub fn from_definition(definition: &Definition) -> Self {
        Self {
            name: definition.name.clone(),
            definition: Some(definition.id),
            slots: definition.slots.clone(),
        }
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.name.trim().is_empty()
            || self.name.len() > 4096
            || self.name.chars().any(char::is_control)
        {
            return Err(
                "working bank name must contain 1–4096 UTF-8 bytes without control characters"
                    .into(),
            );
        }
        for slot in &self.slots {
            slot.validate()?;
        }
        Ok(())
    }
    pub(super) fn storage_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + self.name.capacity()
            + self
                .slots
                .iter()
                .map(|slot| match &slot.source {
                    Some(Source::Library { reference }) => {
                        reference.track.0.capacity()
                            + match &reference.source {
                                crate::engine::media_source::LibSource::File(path) => {
                                    path.capacity()
                                }
                                crate::engine::media_source::LibSource::Removable {volume_id,relative_path}=>volume_id.capacity()+relative_path.capacity(),
                                _ => 0,
                            }
                    }
                    _ => 0,
                })
                .sum::<usize>()
    }
}

#[derive(Debug)]
pub(crate) struct Data {
    pub settings: Arc<Settings>,
    pub audio: [Option<Arc<Sample>>; SLOTS],
    /// A referenced source may be missing. Empty and embedded-only slots are
    /// distinguished by the reference and audio, never by a default sound.
    pub issues: [Option<String>; SLOTS],
    pub ranges: [Option<(f64, f64)>; SLOTS],
}
impl Data {
    pub fn prepare(
        settings: Arc<Settings>,
        audio: [Option<Arc<Sample>>; SLOTS],
        issues: [Option<String>; SLOTS],
    ) -> Result<Self, String> {
        settings.validate()?;
        let mut ranges = [None; SLOTS];
        for index in 0..SLOTS {
            if issues[index].as_ref().is_some_and(|s| s.len() > 2048) {
                return Err("sampler source diagnostic exceeds 2048 bytes".into());
            }
            if let Some(sample) = &audio[index] {
                if issues[index].is_some() {
                    return Err(
                        "a playable sampler slot cannot also report a missing source".into(),
                    );
                }
                validate_sample(sample)?;
                ranges[index] = Some(
                    settings.slots[index]
                        .controls
                        .frames(sample.sr, sample.frames())?,
                );
            } else if settings.slots[index].source.is_some() && issues[index].is_none() {
                return Err("a missing sampler source requires an explicit diagnostic".into());
            } else if settings.slots[index].source.is_none() && issues[index].is_some() {
                return Err("an empty sampler slot cannot report a missing source".into());
            }
        }
        Ok(Self {
            settings,
            audio,
            issues,
            ranges,
        })
    }
    pub fn empty(name: String) -> Result<Self, String> {
        Self::prepare(
            Arc::new(Settings::empty(name)?),
            std::array::from_fn(|_| None),
            std::array::from_fn(|_| None),
        )
    }
    pub fn definition(&self, id: BankId) -> Result<Definition, String> {
        for index in 0..SLOTS {
            if self.audio[index].is_some() && self.settings.slots[index].source.is_none() {
                return Err(format!("slot {} has project-embedded audio without a reusable source; assign a library source before saving this bank", index + 1));
            }
        }
        let definition = Definition {
            id,
            name: self.settings.name.clone(),
            slots: self.settings.slots.clone(),
        };
        definition.validate()?;
        Ok(definition)
    }
    pub(super) fn storage_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + self
                .issues
                .iter()
                .flatten()
                .map(String::capacity)
                .sum::<usize>()
    }
}

pub(crate) fn validate_sample(sample: &Sample) -> Result<(), String> {
    if !(1..=768_000).contains(&sample.sr)
        || !(1..=32).contains(&sample.ch)
        || sample.data.is_empty()
        || sample.data.len() % sample.ch as usize != 0
        || sample.data.iter().any(|x| !x.is_finite())
        || sample
            .peaks
            .iter()
            .flatten()
            .any(|x| !x.is_finite() || *x < 0.0)
        || !sample.bpm.is_finite()
        || sample.name.len() > 4096
        || sample.path.len() > 4096
    {
        return Err("invalid sampler PCM or analysis metadata".into());
    }
    Ok(())
}

/// An onset's immutable source-time range and gain. The active voice owns its
/// source independently of later slot edits, selection, Undo, or replacement.
#[derive(Clone, Debug)]
pub(crate) struct Voice {
    pub audio: Arc<Sample>,
    pub position: f64,
    pub start: f64,
    pub end: f64,
    pub gain: f32,
    pub rate: f64,
    pub track: usize,
}
impl Voice {
    pub fn new(
        audio: Arc<Sample>,
        range: (f64, f64),
        controls: Controls,
        rate: f64,
        track: usize,
    ) -> Self {
        Self {
            audio,
            position: range.0,
            start: range.0,
            end: range.1,
            gain: controls.gain,
            rate,
            track,
        }
    }
    pub fn tick(&mut self, output_rate: f64) -> Option<(f32, f32)> {
        if self.position >= self.end {
            return None;
        }
        if self.start == 0.0 && self.end == self.audio.frames() as f64 {
            // Preserve the existing factory/full-source endpoint convention.
            let (left, right) = self.audio.at(self.position);
            self.position += self.rate * self.audio.sr as f64 / output_rate;
            return Some((left * self.gain, right * self.gain));
        }
        // Clamp interpolation to the final frame inside the exclusive crop.
        // Never read a frame after the selected source endpoint.
        let first = self.position.floor() as usize;
        let last = ((self.end.ceil() as usize).saturating_sub(1)).min(self.audio.frames() - 1);
        let next = (first + 1).min(last);
        let fraction = (self.position - first as f64) as f32;
        let channels = self.audio.ch as usize;
        let at = |channel| {
            let a = self.audio.data[first * channels + channel];
            let b = self.audio.data[next * channels + channel];
            (a + (b - a) * fraction) * self.gain
        };
        let left = at(0);
        let right = if channels > 1 { at(1) } else { left };
        self.position += self.rate * self.audio.sr as f64 / output_rate;
        Some((left, right))
    }
}
