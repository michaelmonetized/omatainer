//! The sole catalog writer acknowledges a tag transaction only after durable
//! catalog persistence. Installed media recovery is essential even if optional
//! work was cancelled after the filesystem exchange won its commit claim.
use crate::{
    engine::{media_source::FileFingerprint, performance::WorkPermit},
    library::{tags::Review, Store},
    media_location::{Access, Location, Snapshot},
    media_tags::write::{self, Applied, Record},
    ui::library_scan::tag_jobs::Reply,
};
use std::{
    fs::{File, OpenOptions},
    os::unix::fs::OpenOptionsExt,
    sync::Arc,
};

pub(in crate::ui) struct Save {
    pub id: u64,
    pub result: Arc<Reply>,
    pub recovery_index: Option<usize>,
    pub work: Arc<WorkPermit>,
}
pub(in crate::ui) struct Receipt {
    pub id: u64,
    pub committed: bool,
    pub durable: bool,
    pub outcome: Result<String, String>,
    pub cleanup: Option<Record>,
}
impl Receipt {
    pub fn refused(id: u64, error: String) -> Self {
        Self {
            id,
            committed: false,
            durable: false,
            outcome: Err(error),
            cleanup: None,
        }
    }
}
struct Guard {
    location: Location,
    access: Access,
    file: File,
    fingerprint: FileFingerprint,
}
impl Guard {
    fn open(review: &Review, fingerprint: FileFingerprint) -> Result<Self, String> {
        let snapshot = Snapshot::discover().map_err(|error| error.to_string())?;
        let location = snapshot
            .resolve(&review.source)
            .map_err(|error| error.to_string())?;
        let access = snapshot
            .access(&location.path)
            .map_err(|error| error.to_string())?;
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(&location.path)
            .map_err(|error| error.to_string())?;
        let guard = Self {
            location,
            access,
            file,
            fingerprint,
        };
        guard.check()?;
        Ok(guard)
    }
    fn check(&self) -> Result<(), String> {
        let snapshot = Snapshot::discover().map_err(|error| error.to_string())?;
        self.access
            .check(&snapshot, &self.location.path)
            .map_err(|error| error.to_string())?;
        self.location
            .recheck_with(&snapshot)
            .map_err(|error| error.to_string())?;
        self.location
            .verify_file(&self.file, self.fingerprint)
            .map_err(|error| error.to_string())
    }
}
fn installed<'a>(request: &'a Save) -> Result<Option<&'a Applied>, String> {
    match (request.result.as_ref(), request.recovery_index) {
        (Reply::Applied(applied), None) => Ok(Some(applied)),
        (Reply::Sidecar { .. }, None) => Ok(None),
        (Reply::Recover(recovered), Some(index)) => match recovered.get(index) {
            Some(write::Recovery::Applied(applied)) => Ok(Some(applied)),
            _ => Err("Tag recovery selection is not an installed media transaction".into()),
        },
        _ => Err("Tag persistence receipt does not match the requested operation".into()),
    }
}
fn proof_matches_record(applied: &Applied) -> Result<(), String> {
    let proof = &applied.proof;
    let record = &applied.record;
    proof.validate()?;
    if proof.old_hash != record.old_hash
        || proof.new_hash != record.new_hash
        || proof.original_audio != record.original_audio
        || proof.staged_audio != record.staged_audio
        || proof.new_fingerprint != applied.observation.fingerprint
    {
        return Err("Installed tag proof does not match its durable recovery record".into());
    }
    Ok(())
}

pub(super) fn save(store: &mut Store, request: Save) -> Receipt {
    save_using(store, request, |_| {}, Store::save)
}

/// Foreground loads are admitted by the renderer before their patches arrive.
/// Observe only the exact current version, preserving user sidecars and keeping
/// the raw filename fallback separate from the decoded row's effective values.
pub(super) fn observe_loaded<'a>(
    store: &mut Store,
    patches: impl Iterator<Item = &'a super::Patch>,
) -> Result<(), String> {
    let mut changed = false;
    for patch in patches {
        let Some(observation) = &patch.tags else {
            continue;
        };
        let Some(track) = store.catalog.track(&patch.source) else {
            continue;
        };
        if track.versions[track.current].fingerprint != Some(patch.fingerprint) {
            continue;
        }
        let target = Review {
            id: track.id.clone(),
            source: patch.source.clone(),
            fingerprint: patch.fingerprint,
        };
        let before = track.versions[track.current].tags.clone();
        let Some(filename) = patch.filename_item() else {
            continue;
        };
        match observation {
            Ok(observation) => store.catalog.observe_tags_with_fallback(
                &target,
                observation,
                &filename.stored_metadata(),
            )?,
            Err(error) => store.catalog.observe_tag_failure_with_fallback(
                &target,
                error,
                &filename.stored_metadata(),
            )?,
        }
        if matches!(
            patch.bpm.origin,
            crate::ui::bpm::Origin::Unknown | crate::ui::bpm::Origin::Heuristic
        ) {
            store.catalog.observe_loader_bpm(&target, patch.bpm)?;
        }
        changed |= store
            .catalog
            .version(&patch.source, Some(patch.fingerprint))
            .and_then(|version| version.tags.as_ref())
            != before.as_ref();
    }
    if changed {
        store.save()?;
    }
    Ok(())
}
fn save_using(
    store: &mut Store,
    request: Save,
    mut checkpoint: impl FnMut(u8),
    persist: impl FnOnce(&mut Store) -> Result<(), String>,
) -> Receipt {
    let mut receipt = Receipt::refused(request.id, "Tag persistence did not complete".into());
    let mut media_installed = false;
    let result = (|| -> Result<String, String> {
        let applied = installed(&request)?;
        media_installed = applied.is_some();
        if !media_installed && request.work.cancelled() {
            return Err("Tag sidecar cancelled before publication".into());
        }
        let (review, expected) = if let Some(applied) = applied {
            proof_matches_record(applied)?;
            (&applied.record.review, applied.proof.new_fingerprint)
        } else if let Reply::Sidecar { target, .. } = request.result.as_ref() {
            (target, target.fingerprint)
        } else {
            unreachable!()
        };
        let guard = Guard::open(review, expected)?;
        let mut candidate = store.catalog.clone();
        let message = if let Some(applied) = applied {
            candidate.accept_tag_rewrite(
                review,
                &applied.proof,
                &applied.observation,
                &applied.record.patch,
            )?;
            let mut message = "Embedded tags and library metadata saved".to_owned();
            if !applied.notices.is_empty() {
                message.push_str(". ");
                message.push_str(&applied.notices.join(" "));
            }
            message
        } else if let Reply::Sidecar {
            patch,
            observation,
            notice,
            ..
        } = request.result.as_ref()
        {
            match observation {
                Ok(observation) => candidate.observe_tags(review, observation)?,
                Err(error) => candidate.observe_tag_failure(review, error)?,
            }
            candidate.apply_tag_sidecar(review, patch)?;
            format!("Library sidecar saved. {notice}")
        } else {
            unreachable!()
        };
        checkpoint(0);
        // A completed exchange cannot be treated as a cancelled optional write.
        // Sidecars still need the short exclusive optional-publication claim.
        let claim = if media_installed {
            None
        } else {
            Some(request.work.commit().map_err(|error| error.to_string())?)
        };
        guard.check()?;
        checkpoint(1);
        let baseline = std::mem::replace(&mut store.catalog, candidate);
        let result = persist(store);
        receipt.committed = result.is_ok() || store.last_save_replaced();
        receipt.durable = result.is_ok();
        if !receipt.committed {
            store.catalog = baseline;
        }
        if receipt.durable {
            receipt.cleanup = applied.map(|applied| applied.record.clone());
        }
        drop(claim);
        checkpoint(2);
        result?;
        Ok(message)
    })();
    receipt.outcome = result.map_err(|error| {
        if media_installed && !receipt.committed {
            format!(
                "Media tags installed; library update pending. Recovery journal retained: {error}"
            )
        } else if receipt.committed && !receipt.durable {
            if media_installed {
                format!("Tag library update committed; durability unconfirmed. Recovery retained: {error}")
            } else {
                format!("Tag sidecar committed; durability unconfirmed: {error}")
            }
        } else {
            error
        }
    });
    receipt
}

#[cfg(test)]
mod tests;
