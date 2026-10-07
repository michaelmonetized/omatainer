//! Real tag observations travel with the same immutable scan receipt. Parsing
//! and retirement stay on the filesystem owner; the sole catalog writer applies
//! provenance only when it accepts that scan's captured media version.
use super::*;
use crate::library::{tags::Review, Catalog, Metadata};
use crate::media_tags::Observation;

#[derive(Debug)]
pub(in crate::ui) struct Observed {
    pub source: LibSource,
    pub fingerprint: FileFingerprint,
    pub filename: Metadata,
    pub result: Result<Observation, String>,
}
impl Observed {
    pub fn reconcile(&self, catalog: &mut Catalog) -> Result<(), String> {
        let Some(track) = catalog.track(&self.source) else {
            return Err("Inspected track is missing from the scan transaction".into());
        };
        // A stale scan cannot change a newer current version's tags.
        if track.versions[track.current].fingerprint != Some(self.fingerprint) {
            return Ok(());
        }
        let target = Review {
            id: track.id.clone(),
            source: self.source.clone(),
            fingerprint: self.fingerprint,
        };
        match &self.result {
            Ok(observation) => {
                catalog.observe_tags_with_fallback(&target, observation, &self.filename)
            }
            Err(error) => catalog.observe_tag_failure_with_fallback(&target, error, &self.filename),
        }
    }
}

pub(super) fn observe(item: &mut LibItem, filename: Metadata, cancel: &AtomicBool) -> Observed {
    let fingerprint = item.fingerprint.expect("scanner captured a regular file");
    let result = crate::media_location::Location::resolve(&item.source)
        .map_err(|error| error.to_string())
        .and_then(|location| crate::media_tags::inspect(&location, fingerprint, cancel));
    if let Ok(observation) = &result {
        let fields = &observation.fields;
        if let Some(value) = &fields.title {
            item.title.clone_from(&value.value);
        }
        if let Some(value) = &fields.artist {
            item.artist.clone_from(&value.value);
        }
        if let Some(value) = &fields.key {
            item.key.clone_from(&value.value);
        }
        if !matches!(item.bpm.origin, super::super::bpm::Origin::User | super::super::bpm::Origin::Imported) {
            if let Some(value) = fields
                .bpm
                .as_ref()
                .and_then(|field| field.value.trim().parse::<f32>().ok())
            {
                let bpm = Bpm::new(value, super::super::bpm::Origin::EmbeddedTag);
                if bpm.value().is_some() {
                    item.bpm = bpm;
                }
            }
        }
    }
    Observed {
        source: item.source.clone(),
        fingerprint,
        filename,
        result,
    }
}
