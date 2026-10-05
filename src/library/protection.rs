//! Track preparation locks, enforced by catalog publication and loaded media receipts.
use super::*;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Locks {
    pub grid: bool,
    pub bpm: bool,
    pub metadata: bool,
}
impl Locks {
    /// Omit an unlocked version from saved records.
    /// Takes these locks; returns true when every preparation field remains editable.
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// Keep locked metadata during automatic observation.
    /// Takes the previous and proposed values; restores only locked fields, preserving playback history.
    pub fn preserve(self, previous: &Metadata, next: &mut Metadata) {
        if self.bpm {
            next.bpm = previous.bpm;
        }
        if self.metadata {
            next.title.clone_from(&previous.title);
            next.artist.clone_from(&previous.artist);
            next.key.clone_from(&previous.key);
            next.duration = previous.duration;
        }
    }

    /// Preserve locked fields that a manual review did not select.
    /// Takes old/proposed metadata and the approved tag patch; permits only its explicit edits through existing locks.
    pub fn preserve_reviewed(
        self,
        previous: &Metadata,
        next: &mut Metadata,
        patch: &super::tags::Patch,
    ) {
        if self.bpm && patch.bpm.is_none() {
            next.bpm = previous.bpm;
        }
        if self.metadata {
            if patch.title.is_none() {
                next.title.clone_from(&previous.title);
            }
            if patch.artist.is_none() {
                next.artist.clone_from(&previous.artist);
            }
            if patch.key.is_none() {
                next.key.clone_from(&previous.key);
            }
            next.duration = previous.duration;
        }
    }

    /// Exclude protected fields before expensive analysis.
    /// Takes selected analysis fields; returns the still-permitted subset without adding any work.
    pub fn analysis(
        self,
        mut fields: crate::track_analysis::Fields,
    ) -> crate::track_analysis::Fields {
        fields.bpm &= !self.bpm;
        fields.duration &= !self.metadata;
        fields
    }

    /// Describe preparation protection for a visible row.
    /// Takes these locks; returns the protected field names or an unlocked status.
    pub fn description(self) -> String {
        let names = [
            (self.grid, "grid"),
            (self.bpm, "BPM"),
            (self.metadata, "metadata"),
        ];
        let fields: Vec<_> = names
            .into_iter()
            .filter_map(|(locked, name)| locked.then_some(name))
            .collect();
        if fields.is_empty() {
            "Preparation unlocked".into()
        } else {
            format!("Preparation locked: {}", fields.join(", "))
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Patch {
    pub grid: Option<bool>,
    pub bpm: Option<bool>,
    pub metadata: Option<bool>,
}
impl Patch {
    /// Detect a review with no selected fields.
    /// Takes this patch; returns true when no lock is changed.
    pub fn is_empty(self) -> bool {
        self.grid.is_none() && self.bpm.is_none() && self.metadata.is_none()
    }
    /// Apply explicit lock choices.
    /// Takes previous locks; returns a complete version-specific replacement.
    pub fn apply(self, previous: Locks) -> Locks {
        Locks {
            grid: self.grid.unwrap_or(previous.grid),
            bpm: self.bpm.unwrap_or(previous.bpm),
            metadata: self.metadata.unwrap_or(previous.metadata),
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Target {
    pub track: TrackId,
    pub source: LibSource,
    pub fingerprint: Option<FileFingerprint>,
    pub locks: Locks,
    pub grid: Option<crate::engine::beatgrid::Grid>,
    pub metadata: Metadata,
}
impl Target {
    /// Bind review to the exact visible media version.
    /// Takes a stable track and its current version; returns the metadata and locks the user reviewed.
    pub fn capture(track: &Track) -> Self {
        let version = &track.versions[track.current];
        Self {
            track: track.id.clone(),
            source: track.source.clone(),
            fingerprint: version.fingerprint,
            locks: track.locks,
            grid: version.preparation.grid,
            metadata: version.metadata.clone(),
        }
    }
}
impl Catalog {
    /// Publish one reviewed preparation protection batch.
    /// Takes unique version-qualified targets and selected lock choices; rejects changed targets before any mutation.
    pub fn protect(&mut self, targets: &[Target], patch: Patch) -> Result<bool, String> {
        if targets.is_empty() || targets.len() > 4096 || patch.is_empty() {
            return Err("Review 1–4096 saved versions and select at least one lock field".into());
        }
        let mut wanted: HashMap<_, _> = targets
            .iter()
            .map(|target| (&target.track, target))
            .collect();
        if wanted.len() != targets.len() {
            return Err("Duplicate preparation target; nothing changed".into());
        }
        let mut replacements = Vec::with_capacity(targets.len());
        for (index, track) in self.tracks.iter().enumerate() {
            let Some(target) = wanted.remove(&track.id) else {
                continue;
            };
            let version = &track.versions[track.current];
            let a = &version.metadata;
            let b = &target.metadata;
            if track.source != target.source
                || version.fingerprint != target.fingerprint
                || track.locks != target.locks
                || version.preparation.grid != target.grid
                || a.title != b.title
                || a.artist != b.artist
                || a.key != b.key
                || a.bpm != b.bpm
                || a.duration != b.duration
            {
                return Err(
                    "Prepared version changed after review; refresh before saving its locks".into(),
                );
            }
            replacements.push((index, patch.apply(track.locks)));
        }
        if !wanted.is_empty() {
            return Err("A preparation target no longer exists; nothing changed".into());
        }
        let mut changed = false;
        for (index, locks) in replacements {
            let track = &mut self.tracks[index];
            changed |= track.locks != locks;
            track.locks = locks;
        }
        Ok(changed)
    }
}

#[cfg(test)]
mod tests;
