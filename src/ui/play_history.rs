//! Renderer-confirmed history is independent of crate selection and load labels.
use super::*;
use crate::engine::load_receipt::State;
use crate::engine::media_source::FileFingerprint;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) struct Identity {
    source: LibSource,
    fingerprint: Option<FileFingerprint>,
}
impl Identity {
    pub(super) fn new(source: LibSource, fingerprint: Option<FileFingerprint>) -> Option<Self> {
        // A pathname without verified file identity must not credit replacement
        // content at that path. Built-ins have stable typed in-session identity.
        if matches!(source, LibSource::File(_)) && fingerprint.is_none() {
            return None;
        }
        Some(Self {
            source,
            fingerprint,
        })
    }
    pub(super) fn matches(&self, item: &LibItem) -> bool {
        self.source == item.source && self.fingerprint == item.fingerprint
    }
    pub(super) fn matches_current(&self, item: &LibItem, catalog: &crate::library::Catalog) -> bool {
        self.matches(item) || catalog.equivalent_current(&self.source, self.fingerprint)
            .is_some_and(|(source, fingerprint)| *source == item.source && fingerprint == item.fingerprint)
    }
}

#[derive(Default)]
pub(super) struct History {
    by_source: HashMap<LibSource, HashMap<Option<FileFingerprint>, SystemTime>>,
    latest: Option<(SystemTime, Identity)>,
}
impl History {
    pub(super) fn record(&mut self, identity: &Identity, played: SystemTime) -> bool {
        self.by_source
            .entry(identity.source.clone())
            .or_default()
            .entry(identity.fingerprint)
            .and_modify(|last| *last = (*last).max(played))
            .or_insert(played);
        let newest = self.latest.as_ref().is_none_or(|(last, _)| played > *last);
        if newest {
            self.latest = Some((played, identity.clone()));
        }
        newest
    }
    pub(super) fn latest_identity(&self) -> Option<&Identity> {
        self.latest.as_ref().map(|(_, identity)| identity)
    }
    pub(super) fn get(&self, item: &LibItem) -> Option<SystemTime> {
        self.by_source
            .get(&item.source)?
            .get(&item.fingerprint)
            .copied()
    }
}

pub(super) struct Watch {
    identity: Identity,
    receipt: Receipt,
    observed: Option<SystemTime>,
    preparation_revision: u64,
    metadata: crate::library::Metadata,
}

pub(super) fn initial_watches(engine: &Engine) -> Vec<Watch> {
    [BuiltinStem::Drums, BuiltinStem::Harmony]
        .into_iter()
        .zip(&engine.initial_playback)
        .map(|(stem, receipt)| Watch {
            identity: Identity::new(LibSource::Builtin(stem), None).unwrap(),
            receipt: receipt.clone(),
            observed: None,
            preparation_revision: receipt.preparation().map_or(0, |(revision, _)| revision),
            metadata: builtin_crate_items()
                .into_iter()
                .find(|i| i.source == LibSource::Builtin(stem))
                .unwrap()
                .stored_metadata(),
        })
        .collect()
}

impl App {
    pub(super) fn project_watch_identities(&self) -> Vec<super::project::WatchIdentity> {
        self.playback_watches
            .iter()
            .map(|watch| super::project::WatchIdentity {
                receipt: watch.receipt.clone(),
                identity: super::project::SavedIdentity {
                    source: watch.identity.source.clone(),
                    fingerprint: watch.identity.fingerprint,
                },
            })
            .collect()
    }

    pub(super) fn watch_playback(
        &mut self,
        source: LibSource,
        fingerprint: Option<FileFingerprint>,
        receipt: Receipt,
    ) {
        let metadata = self.capture_metadata(&source, fingerprint);
        if let Some(identity) = Identity::new(source, fingerprint) {
            self.playback_watches.push(Watch {
                identity,
                receipt,
                observed: None,
                preparation_revision: 0,
                metadata,
            });
        }
    }

    pub(super) fn watch_metadata(&mut self, receipt: &Receipt, metadata: crate::library::Metadata) {
        if let Some(watch) = self
            .playback_watches
            .iter_mut()
            .rev()
            .find(|watch| watch.receipt.same_request(receipt))
        {
            watch.metadata = metadata;
        }
    }

    pub(super) fn poll_play_history(&mut self) {
        // Read disconnection first: if the renderer ended, this acquire makes
        // its final playback write visible before any watches are retired.
        let connected = self.engine.cmd.is_connected();
        let mut latest = None;
        self.playback_watches.retain_mut(|watch| {
            // A terminal state is published after the last possible playback
            // update. Read it first to avoid losing a last event at retirement.
            let state = watch.receipt.state();
            let played = watch.receipt.last_play();
            let preparation = watch.receipt.preparation();
            if preparation.is_some_and(|(revision, _)| revision != watch.preparation_revision)
                || played != watch.observed
            {
                let previous_revision = watch.preparation_revision;
                if let Some((revision, _)) = preparation {
                    watch.preparation_revision = revision;
                }
                self.library_metadata
                    .capture(super::library_store::Capture {
                        source: watch.identity.source.clone(),
                        fingerprint: watch.identity.fingerprint,
                        metadata: watch.metadata.clone(),
                        preparation: preparation
                            .filter(|(revision, _)| *revision != previous_revision)
                            .map(|(_, p)| p),
                        played,
                    });
            }
            if played != watch.observed {
                watch.observed = played;
                if let Some(played) = played {
                    if self.last_played.record(&watch.identity, played) {
                        latest = Some(watch.identity.clone());
                    }
                }
            }
            connected && (!matches!(state, State::Superseded | State::Unavailable | State::Protected)
                || watch.receipt.retained_by_history())
        });
        if let Some(identity) = latest {
            self.refresh_library_view();
            if let Some(index) = self
                .library_view
                .indices
                .iter()
                .position(|&index| identity.matches(&self.library[index]))
            {
                self.last_play_idx = index;
            }
        }
    }

    pub(super) fn item_last_play(&self, item: &LibItem) -> Option<SystemTime> {
        // Borrow during row rendering; constructing/cloning a pathname key for
        // every painted row would reintroduce avoidable work in the viewport.
        let recorded = self.last_played.get(item);
        match (recorded, item.last_play) {
            (Some(recorded), Some(existing)) => Some(recorded.max(existing)),
            (recorded, existing) => recorded.or(existing),
        }
    }
}

#[cfg(test)]
mod tests;

impl App {
    pub(super) fn cue_receipt(&self, key: usize) -> Option<Receipt> {
        self.playback_watches.iter().map(|w| &w.receipt)
            .chain(self.cue_editor.project_receipts.iter().flatten())
            .find(|receipt| key != 0 && receipt.snapshot_key() == key).cloned()
    }
    pub(super) fn cue_storage_status(&self, receipt: &Receipt) -> &'static str {
        let Some(watch) = self.playback_watches.iter().find(|w| w.receipt.same_request(receipt)) else {
            return "Session cues — save the project to keep them; no library association";
        };
        if self.library_metadata.storage.is_none() { return "Session cues — DJ library storage is unavailable"; }
        if !self.library_metadata.durable || self.library_metadata.storage_error.is_some() {
            return "Library save needs attention — cues remain in this session";
        }
        let saved = self.library_metadata.catalog.version(&watch.identity.source, watch.identity.fingerprint);
        if !self.library_metadata.active() && receipt.preparation().is_some_and(|(_, p)| saved.is_some_and(|v| v.preparation == p)) {
            if matches!(watch.identity.source, LibSource::File(_)) && saved.is_some_and(|v| v.content_hash.is_none()) {
                "Saved in DJ library; move verification pending — keep the original file available"
            } else { "Saved in DJ library" }
        } else { "Saving cues to DJ library…" }
    }
}
