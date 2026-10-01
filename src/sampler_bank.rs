//! Reusable sampler definitions. These are source references, not the PCM held
//! by a project. All identity creation and persistence run outside the renderer.
use crate::engine::media_source::{FileFingerprint, LibSource};
use crate::library::{Catalog, TrackId};
use serde::{Deserialize, Serialize};
use std::{collections::HashSet, fmt, io::Read, path::PathBuf};

mod store;
pub(crate) mod assets;
pub(crate) mod resident;
pub(crate) mod prepare;
pub(crate) mod manager;
pub(crate) use store::{default_path, Commit, Store};
#[cfg(test)]
mod tests;

pub(crate) const SLOTS: usize = 16;
pub(crate) const MAX_DEFINITIONS: usize = 64;
pub(crate) const MAX_BYTES: u64 = 4 * 1024 * 1024;
pub(crate) const MAX_NAME_BYTES: usize = 128;
const SCHEMA: u32 = 1;

/// A working bank instance and a reusable definition each get their own ID.
/// Import/copy must allocate a new working ID, retaining definition ID only as
/// provenance. Neither names nor current UI selection identify an edit target.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct BankId([u8; 16]);
impl BankId {
    pub fn new() -> Result<Self, String> {
        let mut bytes = [0; 16];
        std::fs::File::open("/dev/urandom")
            .and_then(|mut file| file.read_exact(&mut bytes))
            .map_err(|error| format!("sampler identity: {error}"))?;
        Self::from_bytes(bytes)
    }
    pub fn from_bytes(bytes: [u8; 16]) -> Result<Self, String> {
        if bytes == [0; 16] {
            return Err("zero sampler identity is reserved".into());
        }
        Ok(Self(bytes))
    }
    pub fn bytes(self) -> [u8; 16] {
        self.0
    }
}
impl fmt::Display for BankId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(formatter, "{byte:02x}")?;
        }
        Ok(())
    }
}
impl Serialize for BankId {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}
impl<'de> Deserialize<'de> for BankId {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        if text.len() != 32
            || !text
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(serde::de::Error::custom(
                "sampler identity must contain 32 lowercase hexadecimal digits",
            ));
        }
        let mut bytes = [0; 16];
        for (index, byte) in bytes.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&text[index * 2..index * 2 + 2], 16)
                .map_err(serde::de::Error::custom)?;
        }
        Self::from_bytes(bytes).map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Factory {
    Kit,
    Perc,
    Hits,
}
impl Factory {
    pub const ALL: [Self; 3] = [Self::Kit, Self::Perc, Self::Hits];
    pub fn index(self) -> usize {
        match self {
            Self::Kit => 0,
            Self::Perc => 1,
            Self::Hits => 2,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::Kit => "Kit",
            Self::Perc => "Perc",
            Self::Hits => "Hits",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SourceRef {
    pub track: TrackId,
    pub source: LibSource,
    pub fingerprint: FileFingerprint,
    pub content_hash: Option<[u8; 32]>,
}
impl SourceRef {
    pub fn validate(&self) -> Result<(), String> {
        if self.track.0.len() != 32
            || !self
                .track
                .0
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err("invalid sampler catalog track identity".into());
        }
        let LibSource::File(path) = &self.source else {
            return Err("sampler library sources must be local files".into());
        };
        use std::os::unix::ffi::OsStrExt;
        let bytes = path.as_os_str().as_bytes();
        if !path.is_absolute() || bytes.len() > 4096 || bytes.contains(&0) {
            return Err("invalid local sampler source path".into());
        }
        Ok(())
    }

    /// Catalog identity resolution performs no filesystem access. A changed
    /// path/fingerprint requires a previously verified same-byte digest, never
    /// a matching title or a TrackId whose current content has been replaced.
    pub fn resolve(&self, catalog: &Catalog) -> Result<Self, String> {
        self.validate()?;
        let track = catalog
            .tracks
            .iter()
            .find(|track| track.id == self.track)
            .ok_or("sampler source is not in the current library")?;
        let old = catalog
            .track_for_version(&self.source, Some(self.fingerprint))
            .filter(|old| old.id == self.track)
            .and_then(|old| {
                old.versions
                    .iter()
                    .find(|v| v.fingerprint == Some(self.fingerprint))
            })
            .ok_or("sampler source version is no longer attributable to this track")?;
        let current = track
            .versions
            .get(track.current)
            .ok_or("sampler source version is missing")?;
        let fingerprint = current
            .fingerprint
            .ok_or("sampler source has no local file identity")?;
        if track.source != self.source || fingerprint != self.fingerprint {
            let expected = self
                .content_hash
                .or(old.content_hash)
                .ok_or("sampler source moved without verified matching bytes")?;
            if old.content_hash != Some(expected) || current.content_hash != Some(expected) {
                return Err("sampler source content changed; original assignment preserved".into());
            }
        } else if self.content_hash.is_some()
            && current.content_hash.is_some()
            && self.content_hash != current.content_hash
        {
            return Err("sampler source digest conflicts with the library".into());
        }
        let result = Self {
            track: self.track.clone(),
            source: track.source.clone(),
            fingerprint,
            content_hash: self.content_hash.or(current.content_hash),
        };
        result.validate()?;
        Ok(result)
    }
    pub fn path(&self) -> Result<&std::path::Path, String> {
        match &self.source {
            LibSource::File(path) => Ok(path),
            _ => Err("sampler reference is not a local file".into()),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Source {
    Library { reference: SourceRef },
    Factory { bank: Factory, slot: u8 },
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Controls {
    pub gain: f32,
    pub start_seconds: f64,
    /// None means the source end, including when missing media cannot currently
    /// supply a duration. Some is an exclusive source-time endpoint.
    pub end_seconds: Option<f64>,
}
impl Default for Controls {
    fn default() -> Self {
        Self {
            gain: 1.0,
            start_seconds: 0.0,
            end_seconds: None,
        }
    }
}
impl Controls {
    pub fn validate(self) -> Result<(), String> {
        if !self.gain.is_finite()
            || !(0.0..=2.0).contains(&self.gain)
            || !self.start_seconds.is_finite()
            || !(0.0..=1.0e9).contains(&self.start_seconds)
            || self
                .end_seconds
                .is_some_and(|end| !end.is_finite() || end <= self.start_seconds || end > 1.0e9)
        {
            return Err(
                "sampler gain must be 0–2 and the source range finite, nonnegative and increasing"
                    .into(),
            );
        }
        Ok(())
    }
    /// Preparation resolves a range once against resident PCM. Endpoints are
    /// source frames, so output-rate changes cannot change the selected range.
    pub fn frames(self, sample_rate: u32, frames: usize) -> Result<(f64, f64), String> {
        self.validate()?;
        if sample_rate == 0 || frames == 0 {
            return Err("sampler source contains no playable frames".into());
        }
        let start = self.start_seconds * sample_rate as f64;
        let end = self
            .end_seconds
            .map_or(frames as f64, |end| end * sample_rate as f64);
        if !start.is_finite() || !end.is_finite() || start >= end || end > frames as f64 {
            return Err(
                "sampler range is outside this source; adjust the draft before applying".into(),
            );
        }
        Ok((start, end))
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Slot {
    pub source: Option<Source>,
    pub controls: Controls,
}
impl Slot {
    pub fn validate(&self) -> Result<(), String> {
        self.controls.validate()?;
        match &self.source {
            Some(Source::Library { reference }) => reference.validate()?,
            Some(Source::Factory { slot, .. }) if *slot as usize >= SLOTS => {
                return Err("factory sampler slot is outside 0–15".into())
            }
            _ => {}
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Definition {
    pub id: BankId,
    pub name: String,
    pub slots: [Slot; SLOTS],
}
impl Definition {
    pub fn empty(name: String) -> Result<Self, String> {
        let result = Self {
            id: BankId::new()?,
            name,
            slots: std::array::from_fn(|_| Slot::default()),
        };
        result.validate()?;
        Ok(result)
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.name.trim().is_empty()
            || self.name.len() > MAX_NAME_BYTES
            || self.name.chars().any(char::is_control)
        {
            return Err(
                "bank name must contain 1–128 UTF-8 bytes without control characters".into(),
            );
        }
        for slot in &self.slots {
            slot.validate()?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Collection {
    pub schema: u32,
    pub revision: u64,
    pub banks: Vec<Definition>,
}
impl Default for Collection {
    fn default() -> Self {
        Self {
            schema: SCHEMA,
            revision: 0,
            banks: vec![],
        }
    }
}
impl Collection {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema != SCHEMA {
            return Err(format!(
                "unsupported sampler bank schema {}; original file preserved",
                self.schema
            ));
        }
        if self.banks.len() > MAX_DEFINITIONS {
            return Err("reusable sampler collection exceeds 64 banks".into());
        }
        let mut seen = HashSet::new();
        for bank in &self.banks {
            if !seen.insert(bank.id) {
                return Err("duplicate reusable bank identity".into());
            }
            bank.validate()?;
        }
        Ok(())
    }
    /// Overwrite authorization is by exact definition ID, never by display name.
    pub fn put(&mut self, bank: Definition, replace: Option<BankId>) -> Result<(), String> {
        bank.validate()?;
        let revision = self
            .revision
            .checked_add(1)
            .ok_or("reusable bank revision exhausted")?;
        let existing = self.banks.iter().position(|old| old.id == bank.id);
        match (existing, replace) {
            (Some(index), Some(id)) if id == bank.id => self.banks[index] = bank,
            (None, None) if self.banks.len() < MAX_DEFINITIONS => self.banks.push(bank),
            (None, None) => return Err("reusable sampler bank limit reached".into()),
            _ => return Err(
                "reusable bank changed or replacement was not explicitly authorized by identity"
                    .into(),
            ),
        }
        self.revision = revision;
        Ok(())
    }
}
