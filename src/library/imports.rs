use super::*;
use std::sync::Arc;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Details {
    pub title: String,
    pub artist: String,
    pub bpm: Option<f32>,
    pub key: String,
    pub preparation: Option<Arc<Preparation>>,
    pub annotations: Option<annotations::Annotations>,
    pub warnings: Vec<String>,
}
impl Details {
    /// Validate imported musical metadata. Takes the retained fields; rejects unsupported numeric ranges or oversized source notes.
    pub fn validate(&self) -> Result<(), String> {
        if self
            .bpm
            .is_some_and(|v| !v.is_finite() || !(20.0..=400.0).contains(&v))
            || !crate::media_tags::text_valid(&self.key)
            || !crate::media_tags::text_valid(&self.title)
            || !crate::media_tags::text_valid(&self.artist)
            || self.preparation.as_ref().is_some_and(|p| !p.valid())
            || self.warnings.len() > 64
            || self
                .warnings
                .iter()
                .any(|s| !crate::media_tags::text_valid(s))
        {
            return Err("Invalid imported tempo, preparation or bounded metadata".into());
        }
        if let Some(fields) = &self.annotations {
            fields.validate()?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Reference {
    pub reference: String,
    pub track: Option<TrackId>,
    pub details: Arc<Details>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Collection {
    pub source: LibSource,
    pub digest: [u8; 32],
    pub format: String,
    pub key: String,
    pub crate_id: crates::CrateId,
    pub references: Vec<Reference>,
}

/// Validate durable source provenance and original reference order.
/// Takes imported collections and known tracks; refuses duplicate identities, malformed sources or more than 100000 retained references.
pub(crate) fn validate(records: &[Collection], tracks: &[Track]) -> Result<(), String> {
    if records.len() > crates::MAX_CRATES {
        return Err("Import provenance exceeds 4096 collections".into());
    }
    let known: HashSet<_> = tracks.iter().map(|t| &t.id).collect();
    let mut keys = HashSet::new();
    let mut references = 0;
    let mut wire_bytes = 0usize;
    let mut sizes = HashMap::new();
    for record in records {
        validate_source(&record.source)?;
        wire_bytes = wire_bytes.saturating_add(
            serde_json::to_vec(&record.source)
                .map_err(|e| e.to_string())?
                .len()
                + record.key.len().saturating_mul(6)
                + record.format.len().saturating_mul(6)
                + 512,
        );
        if wire_bytes > 32 * 1024 * 1024 {
            return Err(
                "Imported source provenance exceeds its 32 MiB serialization reserve".into(),
            );
        }
        if !matches!(
            &record.source,
            LibSource::File(_) | LibSource::Removable { .. }
        ) || record.key.len() > 8192
            || record.key.is_empty()
            || record.key.chars().any(char::is_control)
            || record.format.is_empty()
            || !crate::media_tags::text_valid(&record.format)
            || !keys.insert((&record.source, record.digest, &record.key))
            || record.crate_id.0.len() != 32
            || !record
                .crate_id
                .0
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        {
            return Err("Invalid or repeated imported collection identity".into());
        }
        references += record.references.len();
        if references > 100_000 {
            return Err("Import provenance exceeds 100000 original references".into());
        }
        for reference in &record.references {
            if !crate::media_tags::text_valid(&reference.reference)
                || reference
                    .track
                    .as_ref()
                    .is_some_and(|id| !known.contains(id))
            {
                return Err("Invalid imported track reference".into());
            }
            reference.details.validate()?;
            let size = if let Some(size) = sizes.get(&Arc::as_ptr(&reference.details)) {
                *size
            } else {
                let size = serde_json::to_vec(&reference.details)
                    .map_err(|e| e.to_string())?
                    .len();
                sizes.insert(Arc::as_ptr(&reference.details), size);
                size
            };
            wire_bytes =
                wire_bytes.saturating_add(size + reference.reference.len().saturating_mul(6) + 256);
            if wire_bytes > 32 * 1024 * 1024 {
                return Err(
                    "Imported source provenance exceeds its 32 MiB serialization reserve".into(),
                );
            }
        }
    }
    Ok(())
}
