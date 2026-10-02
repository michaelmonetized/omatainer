use super::*;
use crate::track_analysis::Patch;

impl Catalog {
    /// Publish selected freshly measured fields to exactly the captured version.
    /// Working-deck state, manual grids/cues, user tempo and unrelated fields
    /// remain authoritative. All validation precedes any catalog mutation.
    pub(crate) fn apply_analysis(&mut self, patch: &Patch) -> Result<(), String> {
        patch.validate()?;
        let proof = &patch.reference;
        let index = self.version_track(&proof.source, Some(proof.fingerprint))
            .ok_or("track analysis no longer matches a catalog version")?;
        let track = &mut self.tracks[index];
        if track.id != proof.track { return Err("track analysis belongs to a different track identity".into()); }
        let version = track.versions.iter_mut().find(|v| v.fingerprint == Some(proof.fingerprint))
            .ok_or("track analysis version is missing")?;
        if version.content_hash.is_some() && version.content_hash != proof.content_hash {
            return Err("track analysis conflicts with the saved source digest".into());
        }
        let next = version.analysis.as_ref().cloned().unwrap_or_default().merged(patch)?;
        version.content_hash = proof.content_hash;
        version.analysis = Some(next);
        if patch.fields.bpm {
            version.metadata.bpm = version.metadata.bpm.reconcile(
                patch.bpm.map_or(Bpm::UNKNOWN, |value| Bpm::new(value, Origin::Heuristic)));
        }
        if patch.fields.duration { version.metadata.duration = Some(patch.duration); }
        tags::reconcile(version);
        Ok(())
    }
}

#[cfg(test)]
mod tests;
