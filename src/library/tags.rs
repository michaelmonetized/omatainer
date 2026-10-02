//! Catalog-only tag provenance and reviewed edits. File parsing/writes happen
//! on filesystem workers; these methods recheck the exact catalog association.
use super::*;
use crate::media_tags::{self, payload::Identity, Fields, Observation};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Patch {
    /// None keeps a field; Some("") explicitly clears it.
    pub title: Option<String>,
    pub artist: Option<String>,
    pub bpm: Option<String>,
    pub key: Option<String>,
}
impl Patch {
    pub fn validate(&self) -> Result<(), String> {
        if [&self.title, &self.artist, &self.bpm, &self.key]
            .into_iter()
            .flatten()
            .any(|value| !media_tags::text_valid(value))
        {
            return Err(
                "tag text must contain at most 4096 UTF-8 bytes and no control characters".into(),
            );
        }
        if self
            .bpm
            .as_ref()
            .is_some_and(|value| !value.is_empty() && parse_bpm(value).is_none())
        {
            return Err("tempo must be empty or a finite number greater than 1 BPM".into());
        }
        Ok(())
    }
    pub fn is_empty(&self) -> bool {
        self.title.is_none() && self.artist.is_none() && self.bpm.is_none() && self.key.is_none()
    }
    fn merge(&mut self, patch: &Self) {
        for (target, value) in [
            (&mut self.title, &patch.title),
            (&mut self.artist, &patch.artist),
            (&mut self.bpm, &patch.bpm),
            (&mut self.key, &patch.key),
        ] {
            if value.is_some() {
                *target = value.clone();
            }
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Review {
    pub id: TrackId,
    pub source: LibSource,
    pub fingerprint: FileFingerprint,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct VerifiedRewrite {
    pub old_hash: [u8; 32],
    pub new_hash: [u8; 32],
    pub new_fingerprint: FileFingerprint,
    pub original_audio: Identity,
    pub staged_audio: Identity,
}
impl VerifiedRewrite {
    pub fn validate(&self) -> Result<(), String> {
        if !self.original_audio.valid() || self.original_audio != self.staged_audio {
            return Err("tag rewrite lacks matching complete audio payload identities".into());
        }
        Ok(())
    }
}

/// Values captured before the first observation, retaining explicit filename
/// hints separately from embedded tags. Analysis remains in Version::analysis.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Fallback {
    pub title: String,
    pub artist: String,
    pub bpm: Bpm,
    pub key: String,
}
impl Fallback {
    fn from_metadata(metadata: &Metadata) -> Self {
        Self {
            title: metadata.title.clone(),
            artist: metadata.artist.clone(),
            bpm: metadata.bpm,
            key: metadata.key.clone(),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TagMetadata {
    pub observed: Fields,
    pub overrides: Patch,
    pub fallback: Fallback,
    /// Last loader heuristic when no durable analysis record exists yet.
    pub automatic_bpm: Bpm,
    pub format: String,
    pub notices: Vec<String>,
    pub rewrite_eligible: bool,
}
impl TagMetadata {
    fn empty(metadata: &Metadata) -> Self {
        Self {
            observed: Fields::default(),
            overrides: Patch::default(),
            fallback: Fallback::from_metadata(metadata),
            automatic_bpm: if metadata.bpm.origin == Origin::Heuristic {
                metadata.bpm
            } else {
                Bpm::UNKNOWN
            },
            format: "uninspected".into(),
            notices: Vec::new(),
            rewrite_eligible: false,
        }
    }
    pub fn valid(&self) -> bool {
        self.overrides.validate().is_ok()
            && self.fallback.bpm.valid()
            && self.automatic_bpm.valid()
            && matches!(
                self.automatic_bpm.origin,
                Origin::Unknown | Origin::Heuristic
            )
            && [
                &self.fallback.title,
                &self.fallback.artist,
                &self.fallback.key,
            ]
            .into_iter()
            .all(|text| text.len() <= media_tags::MAX_TEXT_BYTES)
            && [
                &self.observed.title,
                &self.observed.artist,
                &self.observed.bpm,
                &self.observed.key,
            ]
            .into_iter()
            .flatten()
            .all(|field| !field.value.is_empty() && media_tags::text_valid(&field.value))
            && !self.format.is_empty()
            && self.format.len() <= 64
            && media_tags::text_valid(&self.format)
            && self.notices.len() <= 32
            && self
                .notices
                .iter()
                .all(|notice| media_tags::text_valid(notice))
    }
    fn set_observation(&mut self, observation: &Observation) -> Result<(), String> {
        self.observed = observation.fields.clone();
        self.format = format!("{:?}", observation.format);
        self.notices = observation.notices.clone();
        if self
            .observed
            .bpm
            .as_ref()
            .is_some_and(|field| parse_bpm(&field.value).is_none())
            && self.notices.len() < 32
        {
            self.notices.push("Embedded BPM is not a finite number greater than 1; automatic or filename fallback is shown".into());
        }
        self.rewrite_eligible = observation.supports_write();
        if !self.valid() {
            return Err("invalid or oversized embedded tag observation".into());
        }
        Ok(())
    }
    pub fn title_source(&self) -> &'static str {
        provenance(&self.overrides.title, &self.observed.title)
    }
    pub fn artist_source(&self) -> &'static str {
        provenance(&self.overrides.artist, &self.observed.artist)
    }
    pub fn key_source(&self) -> &'static str {
        provenance(&self.overrides.key, &self.observed.key)
    }
    pub fn describe(&self) -> String {
        format!(
            "{} · title: {} · artist: {} · key: {}",
            self.format,
            self.title_source(),
            self.artist_source(),
            self.key_source()
        )
    }
}
fn provenance(user: &Option<String>, embedded: &Option<media_tags::Field>) -> &'static str {
    match (user, embedded) {
        (Some(value), _) if value.is_empty() => "user sidecar · cleared",
        (Some(_), _) => "user sidecar",
        (None, Some(field)) => field.source.label(),
        (None, None) => "filename/catalog fallback",
    }
}
fn parse_bpm(text: &str) -> Option<f32> {
    text.parse::<f32>()
        .ok()
        .filter(|value| value.is_finite() && *value > 1.0)
}
fn text(user: &Option<String>, embedded: &Option<media_tags::Field>, fallback: &str) -> String {
    user.as_deref()
        .or_else(|| embedded.as_ref().map(|field| field.value.as_str()))
        .unwrap_or(fallback)
        .to_owned()
}
pub(super) fn reconcile(version: &mut Version) {
    if let Some(tags) = &mut version.tags {
        if version.metadata.bpm.origin == Origin::Heuristic {
            tags.automatic_bpm = version.metadata.bpm;
        }
    }
    let Some(tags) = &version.tags else {
        return;
    };
    version.metadata.title = text(
        &tags.overrides.title,
        &tags.observed.title,
        &tags.fallback.title,
    );
    version.metadata.artist = text(
        &tags.overrides.artist,
        &tags.observed.artist,
        &tags.fallback.artist,
    );
    version.metadata.key = text(&tags.overrides.key, &tags.observed.key, &tags.fallback.key);
    version.metadata.bpm = if let Some(value) = &tags.overrides.bpm {
        parse_bpm(value).map_or(Bpm::USER_CLEARED, |value| Bpm::new(value, Origin::User))
    } else if version.metadata.bpm.origin == Origin::User {
        // The existing BPM correction path predates the per-field editor.
        version.metadata.bpm
    } else if tags.fallback.bpm.origin == Origin::User {
        tags.fallback.bpm
    } else if let Some(value) = tags
        .observed
        .bpm
        .as_ref()
        .and_then(|field| parse_bpm(&field.value))
    {
        Bpm::new(value, Origin::EmbeddedTag)
    } else if let Some(value) = version
        .analysis
        .as_ref()
        .and_then(|record| record.bpm.as_ref())
    {
        value
            .value
            .map_or(Bpm::UNKNOWN, |value| Bpm::new(value, Origin::Heuristic))
    } else if tags.automatic_bpm.origin == Origin::Heuristic {
        tags.automatic_bpm
    } else {
        tags.fallback.bpm
    };
}

pub(super) fn equivalent_audio(a: &Version, b: &Version) -> bool {
    a.content_hash.is_some()
        && b.content_hash.is_some()
        && (a.content_hash == b.content_hash
            || a.audio_identity.is_some() && a.audio_identity == b.audio_identity)
}

impl Catalog {
    fn reviewed_tag_index(&self, review: &Review) -> Result<usize, String> {
        let index = *self
            .index
            .get(&review.source)
            .ok_or("reviewed tag source is no longer current")?;
        let track = &self.tracks[index];
        if track.id != review.id
            || track.versions[track.current].fingerprint != Some(review.fingerprint)
        {
            return Err("track changed since tag review; review its current version again".into());
        }
        if !matches!(
            review.source,
            LibSource::File(_) | LibSource::Removable { .. }
        ) {
            return Err("embedded tags require a local media source".into());
        }
        Ok(index)
    }
    /// Observations of an archived loaded version cannot change current media.
    pub(crate) fn observe_tags(
        &mut self,
        review: &Review,
        observation: &Observation,
    ) -> Result<(), String> {
        self.observe_tags_inner(review, observation, None)
    }
    pub(crate) fn observe_tags_with_fallback(
        &mut self,
        review: &Review,
        observation: &Observation,
        fallback: &Metadata,
    ) -> Result<(), String> {
        self.observe_tags_inner(review, observation, Some(fallback))
    }
    pub(crate) fn observe_loader_bpm(&mut self, review: &Review, bpm: Bpm) -> Result<(), String> {
        if !bpm.valid() || !matches!(bpm.origin, Origin::Unknown | Origin::Heuristic) {
            return Err("invalid loader tempo observation".into());
        }
        let index = self.reviewed_tag_index(review)?;
        let track = &mut self.tracks[index];
        let version = &mut track.versions[track.current];
        let tags = version
            .tags
            .as_mut()
            .ok_or("loader tempo needs an explicit tag observation")?;
        tags.automatic_bpm = bpm;
        if version.metadata.bpm.origin == Origin::Heuristic {
            version.metadata.bpm = bpm;
        }
        reconcile(version);
        Ok(())
    }
    fn observe_tags_inner(
        &mut self,
        review: &Review,
        observation: &Observation,
        fallback: Option<&Metadata>,
    ) -> Result<(), String> {
        let index = self.reviewed_tag_index(review)?;
        if observation.fingerprint != review.fingerprint {
            return Err("tag observation does not match the reviewed file".into());
        }
        let track = &mut self.tracks[index];
        let version = &mut track.versions[track.current];
        let mut tags = version
            .tags
            .clone()
            .unwrap_or_else(|| TagMetadata::empty(&version.metadata));
        if let Some(fallback) = fallback {
            tags.fallback = Fallback::from_metadata(fallback);
        }
        tags.set_observation(observation)?;
        version.tags = Some(tags);
        reconcile(version);
        Ok(())
    }
    pub(crate) fn observe_tag_failure(
        &mut self,
        review: &Review,
        error: &str,
    ) -> Result<(), String> {
        self.observe_tag_failure_inner(review, error, None)
    }
    pub(crate) fn observe_tag_failure_with_fallback(
        &mut self,
        review: &Review,
        error: &str,
        fallback: &Metadata,
    ) -> Result<(), String> {
        self.observe_tag_failure_inner(review, error, Some(fallback))
    }
    fn observe_tag_failure_inner(
        &mut self,
        review: &Review,
        error: &str,
        fallback: Option<&Metadata>,
    ) -> Result<(), String> {
        let index = self.reviewed_tag_index(review)?;
        let track = &mut self.tracks[index];
        let version = &mut track.versions[track.current];
        let mut tags = version
            .tags
            .clone()
            .unwrap_or_else(|| TagMetadata::empty(&version.metadata));
        if let Some(fallback) = fallback {
            tags.fallback = Fallback::from_metadata(fallback);
        }
        // A failed refresh cannot erase already observed fields for these exact
        // bytes. It disables rewriting and makes incomplete inspection visible.
        let mut notice = "Tag inspection failed: ".to_owned();
        for character in error.chars().map(|c| if c.is_control() { ' ' } else { c }) {
            if notice.len() + character.len_utf8() > media_tags::MAX_TEXT_BYTES {
                break;
            }
            notice.push(character);
        }
        tags.notices = vec![notice];
        tags.rewrite_eligible = false;
        if !tags.valid() {
            return Err("invalid tag fallback metadata".into());
        }
        version.tags = Some(tags);
        reconcile(version);
        Ok(())
    }
    pub(crate) fn apply_tag_sidecar(
        &mut self,
        review: &Review,
        patch: &Patch,
    ) -> Result<(), String> {
        patch.validate()?;
        if patch.is_empty() {
            return Err("tag edit did not select any fields".into());
        }
        let index = self.reviewed_tag_index(review)?;
        let track = &mut self.tracks[index];
        let version = &mut track.versions[track.current];
        let mut tags = version
            .tags
            .clone()
            .unwrap_or_else(|| TagMetadata::empty(&version.metadata));
        tags.overrides.merge(patch);
        if !tags.valid() {
            return Err("invalid tag sidecar metadata".into());
        }
        version.tags = Some(tags);
        reconcile(version);
        Ok(())
    }
    /// Caller supplies hashes/payload identities measured by the guarded writer,
    /// never values imported from an untrusted project or matching by filename.
    /// Old receipts keep their old full-file hash; audio equivalence is explicit.
    pub(crate) fn accept_tag_rewrite(
        &mut self,
        review: &Review,
        proof: &VerifiedRewrite,
        observation: &Observation,
        patch: &Patch,
    ) -> Result<(), String> {
        proof.validate()?;
        patch.validate()?;
        if observation.fingerprint != proof.new_fingerprint
            || proof.new_fingerprint == review.fingerprint
        {
            return Err("tag rewrite observation does not identify a new file version".into());
        }
        let index = *self
            .index
            .get(&review.source)
            .ok_or("rewritten tag source is no longer current")?;
        let track = &self.tracks[index];
        if track.id != review.id {
            return Err("tag rewrite belongs to a different track identity".into());
        }
        let old_index = track
            .versions
            .iter()
            .position(|version| version.fingerprint == Some(review.fingerprint))
            .ok_or("tag rewrite original version is missing")?;
        let old = &track.versions[old_index];
        if old.content_hash.is_some_and(|hash| hash != proof.old_hash)
            || old
                .audio_identity
                .as_ref()
                .is_some_and(|identity| identity != &proof.original_audio)
        {
            return Err("tag rewrite conflicts with saved original content proof".into());
        }
        let current = &track.versions[track.current];
        // Recovery is idempotent and never replays old edits over newer sidecars.
        if current.fingerprint == Some(proof.new_fingerprint)
            && current.content_hash == Some(proof.new_hash)
            && current.audio_identity.as_ref() == Some(&proof.staged_audio)
            && old.content_hash == Some(proof.old_hash)
            && old.audio_identity.as_ref() == Some(&proof.original_audio)
        {
            return Ok(());
        }
        self.reviewed_tag_index(review)?;
        if track
            .versions
            .iter()
            .any(|version| version.fingerprint == Some(proof.new_fingerprint))
        {
            return Err(
                "tag rewrite destination version already exists with different proof".into(),
            );
        }
        if self
            .tracks
            .iter()
            .map(|track| track.versions.len())
            .sum::<usize>()
            >= MAX_VERSIONS
        {
            return Err("library version limit reached".into());
        }
        let mut next = old.clone();
        next.fingerprint = Some(proof.new_fingerprint);
        next.content_hash = Some(proof.new_hash);
        next.audio_identity = Some(proof.staged_audio.clone());
        let mut tags = next
            .tags
            .clone()
            .unwrap_or_else(|| TagMetadata::empty(&next.metadata));
        tags.set_observation(observation)?;
        tags.overrides.merge(patch);
        next.tags = Some(tags);
        reconcile(&mut next);
        let track = &mut self.tracks[index];
        track.versions[old_index].content_hash = Some(proof.old_hash);
        track.versions[old_index].audio_identity = Some(proof.original_audio.clone());
        track.versions.push(next);
        track.current = track.versions.len() - 1;
        Ok(())
    }
}

/// New fields are forbidden even when null in pre-tag schemas. Current Track
/// uses defaults for migrations, so serde alone cannot enforce this boundary.
pub(super) fn reject_legacy_fields(header: &serde_json::Value) -> Result<(), String> {
    let tracks = header.get("tracks").and_then(serde_json::Value::as_array);
    for track in tracks.into_iter().flatten() {
        let versions: Vec<_> = track
            .get("versions")
            .and_then(serde_json::Value::as_array)
            .map(|versions| versions.iter().collect())
            .unwrap_or_else(|| vec![track]);
        for version in versions {
            if version.get("tags").is_some() || version.get("audio_identity").is_some() {
                return Err(
                    "tag metadata and audio identity are unsupported in legacy library schemas"
                        .into(),
                );
            }
            if let Some(bpm) = version
                .get("metadata")
                .and_then(|metadata| metadata.get("bpm"))
            {
                if bpm.get("origin").and_then(serde_json::Value::as_str) == Some("EmbeddedTag")
                    || bpm.get("origin").and_then(serde_json::Value::as_str) == Some("User")
                        && bpm.get("value").is_some_and(serde_json::Value::is_null)
                {
                    return Err("tag BPM provenance and explicit clearing are unsupported in legacy library schemas".into());
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
