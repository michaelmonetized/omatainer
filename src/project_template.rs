//! Reusable native templates keep desired hardware names without substitutions.
use crate::engine::midi::routing::Routing;
use crate::preferences::{Audio, MidiInputs, Profile};
use serde::{Deserialize, Serialize};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

pub(crate) const VERSION: u32 = 1;

#[derive(Clone, Copy)]
pub(crate) struct Target {
    pub namespace: [u64; 2],
    pub id: crate::engine::session::Id,
    pub slot: usize,
}
impl Target {
    /// Bind a hardware review to one active native track identity.
    /// `layout` and `slot` supply the current namespace and track; returns no inactive target.
    pub fn capture(layout: &crate::engine::session::Layout, slot: usize) -> Option<Self> {
        let track = layout
            .tracks
            .get(slot)?
            .active
            .then_some(&layout.tracks[slot])?;
        Some(Self {
            namespace: layout.namespace,
            id: track.id,
            slot,
        })
    }
    /// Resolve a retained target only in its unchanged active track slot.
    /// Returns no target after replacement, deletion or slot reuse.
    pub fn resolve(&self, layout: &crate::engine::session::Layout) -> Option<usize> {
        (layout.namespace == self.namespace
            && layout
                .tracks
                .get(self.slot)
                .is_some_and(|track| track.active && track.id == self.id))
        .then_some(self.slot)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Startup {
    #[default]
    Demo,
    Empty,
    Template {
        path: PathBuf,
    },
}
impl Startup {
    /// Check a persisted startup selection without opening files or devices.
    /// Returns an error for an invalid template path; availability is checked at load.
    pub fn validate(&self) -> Result<(), String> {
        if let Self::Template { path } = self {
            absolute(path)?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Kind {
    Project,
    Track { bus: String },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Hardware {
    pub audio: Audio,
    pub midi_inputs: MidiInputs,
    pub routing: Routing,
}
impl Hardware {
    /// Retain the profile's exact requested audio and MIDI endpoint identities.
    /// `profile` is the applied preference record; no backend connection is changed.
    pub fn capture(profile: &Profile) -> Self {
        Self {
            audio: profile.audio.clone(),
            midi_inputs: profile.midi_inputs.clone(),
            routing: profile.midi_routing.clone(),
        }
    }
    /// Validate saved hardware with the existing preference bounds.
    /// Returns an error without resolving, substituting or connecting devices.
    pub fn validate(&self) -> Result<(), String> {
        let mut profile = Profile::defaults(Path::new("/"));
        self.apply(&mut profile);
        profile.validate()
    }
    /// Put the desired hardware into a draft profile for explicit review.
    /// `profile` receives only audio, MIDI input policy and track routing.
    pub fn apply(&self, profile: &mut Profile) {
        profile.audio = self.audio.clone();
        profile.midi_inputs = self.midi_inputs.clone();
        profile.midi_routing = self.routing.clone();
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Metadata {
    pub schema: u32,
    pub name: String,
    pub kind: Kind,
    pub hardware: Hardware,
}
impl Metadata {
    /// Check a named template and its retained hardware before use or publication.
    /// Returns an error for unsupported schema, names, bus aliases or preferences.
    pub fn validate(&self) -> Result<(), String> {
        if self.schema != VERSION
            || self.name.trim() != self.name
            || self.name.is_empty()
            || self.name.chars().count() > 80
            || self.name.chars().any(char::is_control)
        {
            return Err(
                "Template names require 1–80 visible characters and a supported schema".into(),
            );
        }
        if let Kind::Track { bus } = &self.kind {
            if bus.is_empty()
                || bus.len() > crate::engine::session::MAX_NAME_BYTES
                || bus.contains('\0')
            {
                return Err("Invalid template scene-bus alias".into());
            }
            if self
                .hardware
                .routing
                .routes
                .iter()
                .any(|route| route.track != 0)
            {
                return Err("Track templates can contain only their own MIDI route".into());
            }
        }
        self.hardware.validate()
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Template<T> {
    pub metadata: Metadata,
    pub document: T,
}

/// Require an absolute new-file name without opening its parent or target.
/// `path` may be unavailable; returns an error for oversized or unsafe path data.
pub(crate) fn absolute(path: &Path) -> Result<(), String> {
    if !path.is_absolute()
        || path.file_name().is_none()
        || path.as_os_str().len() > 4096
        || path.as_os_str().as_bytes().contains(&0)
    {
        return Err("Choose an absolute template path with a file name".into());
    }
    Ok(())
}
