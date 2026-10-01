//! Local watched-root bookmarks. Catalog imports never activate foreign roots.
//! Configured paths belong to user profiles; resolved mountpoints are transient.
use super::*;

pub(crate) const MAX_BINDINGS: usize = 32 * 64;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Binding {
    pub profile: String,
    pub configured: PathBuf,
    pub source: LibSource,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Book {
    revision: u64,
    bindings: Vec<Binding>,
}
#[derive(Clone, Debug)]
pub(crate) struct Adoption {
    pub old: LibSource,
    pub expected: FileFingerprint,
    pub source: LibSource,
}
#[derive(Clone, Debug)]
pub(crate) struct Batch {
    pub expected: u64,
    pub profile: String,
    pub configured: Vec<PathBuf>,
    pub observed: Vec<Binding>,
}
fn profile(name: &str) -> bool {
    !name.is_empty()
        && name.trim() == name
        && name.chars().count() <= 80
        && !name.chars().any(char::is_control)
}
fn configured(path: &Path) -> bool {
    validate_source(&LibSource::File(path.into())).is_ok()
}
impl Catalog {
    /// Enroll a captured existing local pathname in its observed volume
    /// namespace. TrackId/crate membership and archived versions survive. Only
    /// an identical fingerprint can reuse a version; changed bytes are upserted
    /// as a fresh unprepared version by the caller. No title/hash guessing.
    pub fn adopt_volume(&mut self, adoption: &Adoption) -> Result<(), String> {
        self.adopt_volumes(std::slice::from_ref(adoption))
    }
    pub fn adopt_volumes(&mut self, adoptions: &[Adoption]) -> Result<(), String> {
        if adoptions.is_empty() {
            return Ok(());
        }
        if adoptions.len() > MAX_TRACKS {
            return Err("volume enrollment exceeds track capacity".into());
        }
        let mut candidate = self.clone();
        let mut seen = HashSet::new();
        for adoption in adoptions {
            if !seen.insert(&adoption.old)
                || !matches!(adoption.old, LibSource::File(_))
                || !matches!(adoption.source, LibSource::Removable { .. })
            {
                return Err("invalid or conflicting volume namespace enrollment".into());
            }
            validate_source(&adoption.source)?;
            crate::media_location::validate_root_source(&adoption.source)
                .map_err(|e| e.to_string())?;
            let Some(i) = candidate.index.get(&adoption.old).copied() else {
                if candidate
                    .track_for_version(&adoption.old, Some(adoption.expected))
                    .is_some_and(|track| track.source == adoption.source)
                {
                    continue;
                }
                return Err("enrolled pathname is no longer in the captured catalog".into());
            };
            let track = &candidate.tracks[i];
            if track.versions[track.current].fingerprint != Some(adoption.expected) {
                return Err("pathname version changed before volume enrollment; rescan".into());
            }
            if candidate.index.contains_key(&adoption.source) {
                return Err(
                    "volume source already has a different catalog identity; enrollment refused"
                        .into(),
                );
            }
            let track = &mut candidate.tracks[i];
            for version in &track.versions {
                if let Some(fingerprint) = version.fingerprint {
                    let previous = PreviousLocation {
                        source: track.source.clone(),
                        fingerprint,
                    };
                    if !track.previous_locations.contains(&previous) {
                        track.previous_locations.push(previous.clone());
                    }
                    if candidate
                        .relocations
                        .insert(previous, i)
                        .is_some_and(|old| old != i)
                    {
                        return Err("volume enrollment conflicts with a previous location".into());
                    }
                }
            }
            candidate.index.remove(&track.source);
            candidate.index.insert(adoption.source.clone(), i);
            track.source = adoption.source.clone();
        }
        candidate.validate()?;
        *self = candidate;
        Ok(())
    }
}
impl Book {
    pub fn revision(&self) -> u64 {
        self.revision
    }
    pub fn binding(&self, profile: &str, configured: &Path) -> Option<&Binding> {
        self.bindings
            .iter()
            .find(|b| b.profile == profile && b.configured == configured)
    }
    /// The metadata owner can reprocess an already committed immutable scan
    /// while rebasing essential cue updates. This recognizes only an exact
    /// no-op; it never permits a stale batch to add, remove or rebind a root.
    pub fn matches(&self, batch: &Batch) -> bool {
        if !profile(&batch.profile) || batch.configured.len() > 64 || batch.observed.len() > 64 {
            return false;
        }
        let inputs: HashSet<_> = batch.configured.iter().collect();
        let observed: HashSet<_> = batch.observed.iter().map(|b| &b.configured).collect();
        inputs.len() == batch.configured.len()
            && observed.len() == batch.observed.len()
            && inputs.iter().all(|p| configured(p))
            && self
                .bindings
                .iter()
                .filter(|b| b.profile == batch.profile)
                .all(|b| inputs.contains(&b.configured))
            && batch.observed.iter().all(|b| {
                b.profile == batch.profile
                    && inputs.contains(&b.configured)
                    && self.binding(&b.profile, &b.configured) == Some(b)
            })
    }
    pub(super) fn validate(&self) -> Result<(), String> {
        if self.bindings.len() > MAX_BINDINGS {
            return Err("watched-root bookmark capacity exceeded".into());
        }
        let mut keys = HashSet::new();
        let mut profiles: HashMap<&str, usize> = HashMap::new();
        for binding in &self.bindings {
            if !profile(&binding.profile)
                || !configured(&binding.configured)
                || crate::media_location::validate_root_source(&binding.source).is_err()
                || matches!(&binding.source, LibSource::File(path) if validate_source(&LibSource::File(path.clone())).is_err())
                || !keys.insert((&binding.profile, &binding.configured))
            {
                return Err("invalid or duplicate watched-root bookmark".into());
            }
            *profiles.entry(&binding.profile).or_default() += 1;
        }
        if profiles.len() > 32 || profiles.values().any(|count| *count > 64) {
            return Err("watched roots exceed 32 profiles or 64 roots per profile".into());
        }
        Ok(())
    }
    /// The scan includes existing bindings even when a volume is offline.
    /// Removing a configured root only removes its local bookmark, never media.
    /// This pure candidate operation runs within the catalog owner's save.
    pub fn apply(&mut self, batch: &Batch) -> Result<bool, String> {
        if batch.expected != self.revision {
            return Err(
                "watched-root bookmarks changed; rescan against the current catalog".into(),
            );
        }
        if !profile(&batch.profile) || batch.configured.len() > 64 || batch.observed.len() > 64 {
            return Err("invalid watched-root batch".into());
        }
        let inputs: HashSet<_> = batch.configured.iter().collect();
        if inputs.len() != batch.configured.len() || inputs.iter().any(|path| !configured(path)) {
            return Err("invalid or duplicate configured watched root".into());
        }
        let mut seen = HashSet::new();
        for binding in &batch.observed {
            if binding.profile != batch.profile
                || !inputs.contains(&binding.configured)
                || !seen.insert(&binding.configured)
            {
                return Err(
                    "watched-root observations do not match the captured profile inputs".into(),
                );
            }
            if let Some(old) = self.binding(&batch.profile, &binding.configured) {
                if old.source != binding.source
                    && !matches!(
                        (&old.source, &binding.source),
                        (LibSource::File(_), LibSource::Removable { .. })
                    )
                {
                    return Err(
                        "a known watched root cannot silently adopt a different volume".into(),
                    );
                }
            }
        }
        let mut candidate = self.clone();
        candidate.bindings.retain(|binding| {
            binding.profile != batch.profile || inputs.contains(&binding.configured)
        });
        for binding in &batch.observed {
            if let Some(old) = candidate
                .bindings
                .iter_mut()
                .find(|old| old.profile == binding.profile && old.configured == binding.configured)
            {
                *old = binding.clone();
            } else {
                candidate.bindings.push(binding.clone());
            }
        }
        candidate.bindings.sort_by(|a, b| {
            a.profile
                .cmp(&b.profile)
                .then_with(|| a.configured.cmp(&b.configured))
        });
        candidate.validate()?;
        let mut before = self.bindings.clone();
        before.sort_by(|a, b| {
            a.profile
                .cmp(&b.profile)
                .then_with(|| a.configured.cmp(&b.configured))
        });
        if candidate.bindings == before {
            return Ok(false);
        }
        candidate.revision = self
            .revision
            .checked_add(1)
            .ok_or("watched-root revision exhausted")?;
        *self = candidate;
        Ok(true)
    }
}

#[cfg(test)]
mod tests;
