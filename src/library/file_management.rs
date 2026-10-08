use super::*;
use std::sync::atomic::{AtomicBool, Ordering};

pub(crate) const MAX_SELECTION: usize = 128;
pub(crate) mod transfer;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Target {
    pub track: Track,
}
impl Target {
    /// Retain one exact catalog row for a file-management review.
    /// Takes the selected stable track; returns its versions and preparation without filesystem work.
    pub(crate) fn capture(track: &Track) -> Self {
        Self {
            track: track.clone(),
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Duplicate {
    pub first: TrackId,
    pub second: TrackId,
    pub reason: String,
    pub label: String,
}

/// Find likely duplicates without changing catalog references.
/// Takes one selected track and the catalog; returns at most 128 byte-hash or matching-name candidates for explicit review.
pub(crate) fn likely_duplicates(
    catalog: &Catalog,
    selected: &TrackId,
) -> Result<Vec<Duplicate>, String> {
    let first = catalog
        .tracks
        .iter()
        .find(|track| &track.id == selected)
        .ok_or("Selected track is no longer saved")?;
    let current = &first.versions[first.current];
    let basename = |source: &LibSource| -> Option<String> {
        match source {
            LibSource::File(path) => path.file_name(),
            LibSource::Removable { relative_path, .. } => relative_path.file_name(),
            _ => None,
        }
        .map(|name| name.to_string_lossy().to_lowercase())
    };
    let name = basename(&first.source);
    let title = current.metadata.title.trim().to_lowercase();
    let artist = current.metadata.artist.trim().to_lowercase();
    let mut found = Vec::new();
    for other in &catalog.tracks {
        if &other.id == selected {
            continue;
        }
        let version = &other.versions[other.current];
        let hash = current.content_hash.is_some() && current.content_hash == version.content_hash;
        let same_name = name.is_some() && name == basename(&other.source);
        let same_metadata = !title.is_empty()
            && !artist.is_empty()
            && title == version.metadata.title.trim().to_lowercase()
            && artist == version.metadata.artist.trim().to_lowercase();
        if hash || same_name || same_metadata {
            found.push(Duplicate {
                first: first.id.clone(),
                second: other.id.clone(),
                label: format!("{} · {:?}",version.metadata.title,other.source),
                reason: if hash {
                    "Saved byte hashes match; live bytes must still be verified"
                } else if same_name {
                    "Filenames match; matching audio has not been proved"
                } else {
                    "Artist and title match; matching audio has not been proved"
                }
                .into(),
            });
            if found.len() == MAX_SELECTION {
                break;
            }
        }
    }
    Ok(found)
}

pub(crate) fn checked_indices(catalog: &Catalog, targets: &[Target]) -> Result<Vec<usize>, String> {
    if targets.is_empty() || targets.len() > MAX_SELECTION {
        return Err("Review 1–128 saved tracks before changing files or references".into());
    }
    let mut unique = HashSet::new();
    targets
        .iter()
        .map(|target| {
            if !unique.insert(&target.track.id) {
                return Err("Repeated file-management target; nothing changed".into());
            }
            let index = catalog
                .tracks
                .iter()
                .position(|track| track.id == target.track.id)
                .ok_or("Reviewed track is no longer saved")?;
            if catalog.tracks[index] != target.track {
                return Err(
                    "Track metadata or preparation changed after review; review it again".into(),
                );
            }
            Ok(index)
        })
        .collect()
}

impl Catalog {
    /// Remove reviewed catalog references while retaining every source file.
    /// Takes exact selected rows and their crate revision; returns an atomic candidate with only those memberships removed.
    pub(crate) fn remove_file_references(
        &self,
        targets: &[Target],
        expected: u64,
    ) -> Result<Self, String> {
        let indices = checked_indices(self, targets)?;
        let ids: HashSet<_> = indices
            .iter()
            .map(|&index| self.tracks[index].id.clone())
            .collect();
        let before: HashSet<_> = self.tracks.iter().map(|track| track.id.clone()).collect();
        let map: HashMap<_, _> = ids.iter().cloned().map(|id| (id, None)).collect();
        let mut next = self.clone();
        next.tracks.retain(|track| !ids.contains(&track.id));
        let after: HashSet<_> = next.tracks.iter().map(|track| track.id.clone()).collect();
        next.crates
            .rewrite_members(
                expected,
                &map,
                |id| before.contains(id),
                |id| after.contains(id),
            )
            .map_err(|e| e.to_string())?;
        for imported in &mut next.imports {
            for reference in &mut imported.references {
                if reference.track.as_ref().is_some_and(|id| ids.contains(id)) {
                    reference.track = None;
                }
            }
        }
        next.validate()?;
        Ok(next)
    }

    /// Merge byte-verified duplicate references and preserve both tracks' prepared versions.
    /// Takes exactly two reviewed rows, explicit retained identity/preparation choice and cancellation; returns one candidate without deleting audio.
    pub(crate) fn merge_file_duplicates(
        &self,
        targets: &[Target],
        expected: u64,
        retain: &TrackId,
        preparation_from: &TrackId,
        cancel: &AtomicBool,
    ) -> Result<Self, String> {
        let indices = checked_indices(self, targets)?;
        if indices.len() != 2 {
            return Err("Choose two distinct duplicates and one retained identity".into());
        }
        let kept = indices
            .iter()
            .copied()
            .find(|&index| &self.tracks[index].id == retain)
            .ok_or("Retained track was not part of the review")?;
        let removed = indices
            .iter()
            .copied()
            .find(|&index| &self.tracks[index].id != retain)
            .unwrap();
        let prepared = indices
            .iter()
            .copied()
            .find(|&index| &self.tracks[index].id == preparation_from)
            .ok_or("Preparation choice was not part of the review")?;
        let originals = [&self.tracks[kept], &self.tracks[removed]];
        let mut proof = Vec::new();
        for track in originals {
            if cancel.load(Ordering::Acquire) {
                return Err(
                    "Duplicate verification cancelled; catalog and files are unchanged".into(),
                );
            }
            let version = &track.versions[track.current];
            let fingerprint = version
                .fingerprint
                .ok_or("Duplicate has no verified file version")?;
            let location = crate::media_location::Location::resolve(&track.source)
                .map_err(|e| e.to_string())?;
            location.recheck().map_err(|e| e.to_string())?;
            let hash = content::hash_file(&location.path, fingerprint, || {
                !cancel.load(Ordering::Acquire)
            })?;
            if version.content_hash.is_some_and(|saved| saved != hash) {
                return Err(
                    "Duplicate bytes disagree with saved content proof; rescan and review again"
                        .into(),
                );
            }
            proof.push((location, fingerprint, hash));
        }
        if proof[0].2 != proof[1].2 {
            return Err(
                "These tracks contain different bytes. No references or preparation were merged"
                    .into(),
            );
        }
        let mut next = self.clone();
        let other = next.tracks[removed].clone();
        let chosen = self.tracks[prepared].versions[self.tracks[prepared].current].clone();
        let track = &mut next.tracks[kept];
        let original_preparation = track.versions[track.current].preparation;
        if track.locks.grid && original_preparation.grid != chosen.preparation.grid {
            return Err("The retained grid is locked; choose its own preparation or deliberately unlock it first".into());
        }
        if track.locks.bpm && track.versions[track.current].metadata.bpm != chosen.metadata.bpm {
            return Err("The retained tempo is locked; choose its own preparation or deliberately unlock it first".into());
        }
        if track.locks.metadata && track.versions[track.current].metadata != chosen.metadata {
            return Err("Retained metadata is locked; choose its own prepared version or deliberately unlock it first".into());
        }
        let previous = PreviousLocation {
            source: other.source.clone(),
            fingerprint: proof[1].1,
        };
        if !track.previous_locations.contains(&previous) {
            track.previous_locations.push(previous);
        }
        for old in &other.previous_locations {
            if !track.previous_locations.contains(old) {
                track.previous_locations.push(old.clone());
            }
        }
        for version in &other.versions {
            if let Some(existing) = track
                .versions
                .iter()
                .find(|saved| saved.fingerprint == version.fingerprint)
            {
                if existing != version {
                    return Err("Duplicate version identities carry conflicting preparation; original tracks were preserved".into());
                }
            } else {
                track.versions.push(version.clone());
            }
        }
        for version in &mut track.versions {
            if proof
                .iter()
                .any(|(_, fingerprint, _)| version.fingerprint == Some(*fingerprint))
            {
                version.content_hash = Some(proof[0].2);
            }
        }
        if preparation_from != retain {
            let old = PreviousLocation {
                source: track.source.clone(),
                fingerprint: proof[0].1,
            };
            if !track.previous_locations.contains(&old) {
                track.previous_locations.push(old);
            }
            track.current = track
                .versions
                .iter()
                .position(|version| version.fingerprint == chosen.fingerprint)
                .ok_or("Chosen prepared version was not preserved")?;
            track.source = self.tracks[prepared].source.clone();
        }
        if !track.annotations.group.is_empty()
            && !other.annotations.group.is_empty()
            && track.annotations.group != other.annotations.group
        {
            return Err("Duplicate groups differ. Review annotations before merging; both original tracks were preserved".into());
        }
        if track.annotations.color.is_some()
            && other.annotations.color.is_some()
            && track.annotations.color != other.annotations.color
        {
            return Err("Duplicate colors differ. Review annotations before merging; both original tracks were preserved".into());
        }
        track.locks.grid |= other.locks.grid;
        track.locks.bpm |= other.locks.bpm;
        track.locks.metadata |= other.locks.metadata;
        for tag in &other.annotations.tags {
            if !track
                .annotations
                .tags
                .iter()
                .any(|saved| saved.eq_ignore_ascii_case(tag))
            {
                track.annotations.tags.push(tag.clone());
            }
        }
        if track.annotations.notes != other.annotations.notes && !other.annotations.notes.is_empty()
        {
            if !track.annotations.notes.is_empty() {
                track.annotations.notes.push_str("\n\n");
            }
            track.annotations.notes.push_str(&other.annotations.notes);
        }
        track.annotations.rating = track.annotations.rating.max(other.annotations.rating);
        if track.annotations.group.is_empty() {
            track.annotations.group = other.annotations.group.clone();
        }
        if track.annotations.color.is_none() {
            track.annotations.color = other.annotations.color;
        }
        let removed_id = other.id.clone();
        let before: HashSet<_> = self.tracks.iter().map(|track| track.id.clone()).collect();
        next.tracks.retain(|track| track.id != removed_id);
        let after: HashSet<_> = next.tracks.iter().map(|track| track.id.clone()).collect();
        let map = HashMap::from([(removed_id.clone(), Some(retain.clone()))]);
        next.crates
            .rewrite_members(
                expected,
                &map,
                |id| before.contains(id),
                |id| after.contains(id),
            )
            .map_err(|e| e.to_string())?;
        for imported in &mut next.imports {
            for reference in &mut imported.references {
                if reference.track.as_ref() == Some(&removed_id) {
                    reference.track = Some(retain.clone());
                }
            }
        }
        for (location, fingerprint, hash) in proof {
            location.recheck().map_err(|e| e.to_string())?;
            if content::hash_file(&location.path, fingerprint, || {
                !cancel.load(Ordering::Acquire)
            })? != hash
            {
                return Err("Duplicate bytes changed during review; originals preserved".into());
            }
        }
        if cancel.load(Ordering::Acquire) {
            return Err("Duplicate verification cancelled before publication".into());
        }
        next.validate()?;
        Ok(next)
    }
}

impl Catalog {
    /// Adopt independently verified copies under the same stable track identities.
    /// Takes exact reviewed outputs, the crate revision and cancellation; returns a complete candidate after existing relocation verifies bytes and preserves prepared versions.
    pub(crate) fn adopt_file_copies(
        &self,
        installed: &[transfer::Installed],
        expected: u64,
        cancel: &AtomicBool,
    ) -> Result<Self, String> {
        if expected != self.crates.revision() {
            return Err("Crate references changed after file review; review again".into());
        }
        let targets: Vec<_> = installed.iter().map(|copy| copy.target.clone()).collect();
        checked_indices(self, &targets)?;
        let mut next = self.clone();
        for copy in installed {
            if cancel.load(Ordering::Acquire) {
                return Err("Copy reassignment cancelled; the saved catalog is unchanged".into());
            }
            if FileFingerprint::read(&copy.destination) != Some(copy.fingerprint)
                || content::hash_file(&copy.destination, copy.fingerprint, || {
                    !cancel.load(Ordering::Acquire)
                })? != copy.hash
            {
                return Err("Copied destination changed before reassignment".into());
            }
            next.relocate_cancellable(
                &Relocate {
                    id: copy.target.track.id.clone(),
                    source: copy.target.track.source.clone(),
                    fingerprint: copy.target.track.versions[copy.target.track.current]
                        .fingerprint
                        .ok_or("Reviewed source proof is missing")?,
                    destination: copy.destination.clone(),
                },
                cancel,
            )?;
            let relocated = next
                .tracks
                .iter()
                .find(|track| track.id == copy.target.track.id)
                .ok_or("Relocated stable identity was lost")?;
            if relocated.versions[relocated.current].fingerprint != Some(copy.fingerprint)
                || relocated.versions[relocated.current].content_hash != Some(copy.hash)
            {
                return Err("Copied preparation identity did not match the installed bytes".into());
            }
        }
        next.validate()?;
        Ok(next)
    }
}

#[cfg(test)]
mod tests;
