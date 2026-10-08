use super::*;
use crate::library::{
    file_management::{
        self as model,
        transfer::{
            self,
            journal::{self, Journal},
        },
    },
    Catalog,
};
use std::path::PathBuf;

#[derive(Clone, Debug)]
pub(in crate::ui) struct Selection {
    pub catalog: Arc<Catalog>,
    pub ids: Vec<TrackId>,
}
impl Selection {
    fn targets(&self, current: &Catalog) -> Result<Vec<model::Target>, String> {
        self.validate()?;
        let targets = self
            .ids
            .iter()
            .map(|id| {
                self.catalog
                    .tracks
                    .iter()
                    .find(|track| &track.id == id)
                    .map(model::Target::capture)
                    .ok_or_else(|| "Reviewed track is unavailable".to_string())
            })
            .collect::<Result<Vec<_>, _>>()?;
        model::checked_indices(current, &targets)?;
        Ok(targets)
    }
    fn validate(&self) -> Result<(), String> {
        if self.ids.is_empty()
            || self.ids.len() > model::MAX_SELECTION
            || self.ids.iter().any(|id| {
                id.0.len() != 32
                    || !id
                        .0
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            })
        {
            return Err("Review 1–128 saved library tracks; nothing is truncated".into());
        }
        Ok(())
    }
}
#[derive(Clone, Debug)]
pub(in crate::ui) enum Action {
    Inspect,
    Duplicates(Selection),
    Remove(Selection),
    Merge {
        selection: Selection,
        retain: TrackId,
        preparation: TrackId,
    },
    Transfer {
        selection: Selection,
        destination: PathBuf,
        move_files: bool,
    },
    Restore {
        id: String,
    },
    Keep {
        id: String,
    },
}
impl Action {
    pub(in crate::ui) fn validate(&self) -> Result<(), String> {
        match self {
            Self::Inspect => Ok(()),
            Self::Duplicates(selection) => {
                selection.validate()?;
                if selection.ids.len() != 1 {
                    return Err("Select one saved track to find likely duplicates".into());
                }
                Ok(())
            }
            Self::Remove(selection) => selection.validate(),
            Self::Merge {
                selection,
                retain,
                preparation,
            } => {
                selection.validate()?;
                if selection.ids.len() != 2
                    || !selection.ids.contains(retain)
                    || !selection.ids.contains(preparation)
                {
                    return Err("Review exactly two duplicates, the retained identity and the chosen preparation".into());
                }
                Ok(())
            }
            Self::Transfer {
                selection,
                destination,
                ..
            } => {
                selection.validate()?;
                if !destination.is_absolute()
                    || destination
                        .components()
                        .any(|c| matches!(c, std::path::Component::ParentDir))
                {
                    return Err("Enter an absolute existing destination folder".into());
                }
                Ok(())
            }
            Self::Restore { id } | Self::Keep { id } => {
                if id.len() == 32
                    && id
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                {
                    Ok(())
                } else {
                    Err("Inspect a current file recovery operation first".into())
                }
            }
        }
    }
}
#[derive(Clone, Debug, Default)]
pub(in crate::ui) struct Report {
    pub message: String,
    pub duplicates: Vec<model::Duplicate>,
    pub recovery: Option<journal::Recovery>,
    pub archives: Vec<journal::Recovery>,
}

/// Apply a reviewed file operation on the existing catalog owner.
/// Takes one admitted request, publication checkpoints and the real catalog save; returns one terminal receipt and retains recoverable media whenever publication is uncertain.
pub(super) fn apply_using(
    store: &mut Store,
    request: Request,
    mut checkpoint: impl FnMut(u8),
    save: impl FnOnce(&mut Store) -> Result<(), String>,
) -> Receipt {
    let Request {
        token,
        expected,
        action,
        work,
    } = request;
    let super::Action::Files(action) = action else {
        unreachable!()
    };
    let work = work.unwrap();
    let cancel = work.cancel();
    let mut report = Report::default();
    let mut staged = None;
    let mut move_journal = None;
    let mut published = false;
    let check = || {
        if token.cancelled() {
            Err(Failure::Cancelled)
        } else if work.cancelled() {
            Err(Failure::Performance(performance::Error::Protected))
        } else {
            Ok(())
        }
    };
    let result = (|| {
        check()?;
        let ticket = work
            .background(
                crate::background::Kind::Index,
                format!("library-files:{}", store.path().display()),
                256 * crate::background::MIB,
            )
            .map_err(Failure::Invalid)?;
        let running = ticket
            .enter(|| check().is_err())
            .map_err(Failure::Invalid)?;
        if matches!(&action, Action::Inspect) {
            report.recovery = journal::pending(store).map_err(Failure::Storage)?;
            report.archives = journal::archived(store).map_err(Failure::Storage)?;
            report.message = if report.recovery.is_some() {
                "Pending move recovery is ready for review.".into()
            } else {
                "No pending file move. Source audio is kept when removing library references."
                    .into()
            };
            check()?;
            token.claim()?;
            return Ok(Outcome::Read);
        }
        if let Action::Duplicates(selection) = &action {
            selection
                .targets(&store.catalog)
                .map_err(Failure::Invalid)?;
            report.duplicates = model::likely_duplicates(&store.catalog, &selection.ids[0])
                .map_err(Failure::Invalid)?;
            report.message=format!("{} likely duplicates. Merge independently verifies the complete bytes and preserves both prepared histories.",report.duplicates.len());
            check()?;
            token.claim()?;
            return Ok(Outcome::Read);
        }
        if let Action::Keep { id } = &action {
            let recovery = Journal::reviewed(store, id).map_err(Failure::Storage)?;
            check()?;
            let guard = work.commit().map_err(Failure::Performance)?;
            token.claim()?;
            published = true;
            recovery.keep(store, &cancel).map_err(Failure::Storage)?;
            report.message="Moved locations retained. The original bytes and full recovery record remain available under saved moves.".into();
            report.archives = journal::archived(store).map_err(Failure::Storage)?;
            drop(guard);
            ticket.progress(1, Some(1));
            drop(running);
            return Ok(Outcome::Durable { changed: false });
        }
        if let Action::Restore { id } = &action {
            let mut recovery = Journal::reviewed(store, id).map_err(Failure::Storage)?;
            check()?;
            let guard = work.commit().map_err(Failure::Performance)?;
            token.claim()?;
            let candidate = recovery
                .restore_prepare(store, &cancel)
                .map_err(Failure::Storage)?;
            let outcome = if let Some(candidate) = candidate {
                let baseline = std::mem::replace(&mut store.catalog, candidate);
                match save(store) {
                    Ok(()) => {
                        published = true;
                        match recovery.finish_restore(&store.catalog,&cancel){Ok(())=>report.message="Original locations restored. Only the transaction's verified copies were removed.".into(),Err(error)=>report.message=format!("Original locations saved; copy cleanup remains recoverable: {error}")};
                        Outcome::Durable { changed: true }
                    }
                    Err(error) if store.last_save_replaced() => {
                        published = true;
                        report.message="Restoration may have committed. Recovery files are retained; inspect before retrying.".into();
                        Outcome::CommittedUnconfirmed(error)
                    }
                    Err(error) => {
                        store.catalog = baseline;
                        return Err(Failure::Storage(format!("Restoration save failed; originals and moved copies are preserved with the recovery record: {error}")));
                    }
                }
            } else {
                report.message =
                    "Uncommitted or already restored move cleaned up after verification.".into();
                Outcome::Durable { changed: false }
            };
            report.recovery = journal::pending(store).map_err(Failure::Storage)?;
            report.archives = journal::archived(store).map_err(Failure::Storage)?;
            drop(guard);
            ticket.progress(1, Some(1));
            drop(running);
            return Ok(outcome);
        }
        if journal::pending(store).map_err(Failure::Storage)?.is_some() {
            return Err(Failure::Invalid(
                "Inspect and restore the pending move before another file edit".into(),
            ));
        }
        if expected != store.catalog.crates.revision() {
            return Err(Failure::Invalid(
                "Crate selection changed; capture the file review again".into(),
            ));
        }
        let candidate = match &action {
            Action::Remove(selection) => {
                let targets = selection
                    .targets(&store.catalog)
                    .map_err(Failure::Invalid)?;
                report.message = format!(
                    "{} library references removed. Source audio remains in place.",
                    targets.len()
                );
                store
                    .catalog
                    .remove_file_references(&targets, expected)
                    .map_err(Failure::Invalid)?
            }
            Action::Merge {
                selection,
                retain,
                preparation,
            } => {
                let targets = selection
                    .targets(&store.catalog)
                    .map_err(Failure::Invalid)?;
                report.message="Verified duplicate references merged; both prepared histories and both audio files remain.".into();
                store
                    .catalog
                    .merge_file_duplicates(&targets, expected, retain, preparation, &cancel)
                    .map_err(Failure::Invalid)?
            }
            Action::Transfer {
                selection,
                destination,
                move_files,
            } => {
                let targets = selection
                    .targets(&store.catalog)
                    .map_err(Failure::Invalid)?;
                staged = Some(
                    transfer::stage(&store.catalog, &targets, destination, &cancel)
                        .map_err(Failure::Storage)?,
                );
                if *move_files {
                    move_journal = Some(
                        Journal::prepare(store, staged.as_ref().unwrap())
                            .map_err(Failure::Storage)?,
                    );
                }
                let installed = staged
                    .as_mut()
                    .unwrap()
                    .install(&cancel)
                    .map_err(Failure::Storage)?;
                let candidate = store
                    .catalog
                    .adopt_file_copies(&installed, expected, &cancel)
                    .map_err(Failure::Invalid)?;
                if let Some(journal) = &mut move_journal {
                    journal
                        .bind_candidate(&candidate)
                        .map_err(Failure::Storage)?;
                }
                report.message = if *move_files {
                    "Moved locations saved; originals remain in recoverable source-folder storage."
                        .into()
                } else {
                    "Verified copies saved as the library locations; original audio remains in place.".into()
                };
                candidate
            }
            _ => unreachable!(),
        };
        checkpoint(0);
        check()?;
        let guard = work.commit().map_err(Failure::Performance)?;
        token.claim()?;
        checkpoint(1);
        let baseline = std::mem::replace(&mut store.catalog, candidate);
        let outcome = match save(store) {
            Ok(()) => {
                published = true;
                Outcome::Durable { changed: true }
            }
            Err(error) if store.last_save_replaced() => {
                published = true;
                Outcome::CommittedUnconfirmed(error)
            }
            Err(error) => {
                store.catalog = baseline;
                return Err(Failure::Storage(error));
            }
        };
        if let Some(staged) = &mut staged {
            staged.retain_installed().map_err(Failure::Storage)?;
        }
        if let Some(journal) = &move_journal {
            if matches!(&outcome, Outcome::Durable { .. }) {
                if let Err(error) = journal.retire_originals(&store.catalog, &cancel) {
                    report.message=format!("Moved locations saved; some originals still await recoverable retirement: {error}");
                }
            }
            report.recovery = journal::pending(store).map_err(Failure::Storage)?;
        }
        checkpoint(2);
        ticket.progress(1, Some(1));
        drop(guard);
        drop(running);
        Ok(outcome)
    })();
    let outcome=match result {
        Ok(outcome)=>outcome,
        Err(error) if published=>Outcome::Unknown(format!("Catalog publication completed or remains unconfirmed; retained recovery must be inspected: {error}")),
        Err(error)=>{
            let cleanup=staged.as_mut().map(transfer::Staged::rollback).transpose().and_then(|_|move_journal.as_ref().map(Journal::clear).transpose());
            match cleanup {Ok(_)=>Outcome::Rejected(error),Err(cleanup)=>Outcome::Rejected(Failure::Storage(format!("{error}; recovery retained: {cleanup}")))}
        },
    };
    token.finished();
    Receipt {
        id: token.id,
        revision: store.catalog.crates.revision(),
        created: None,
        review: None,
        files: Some(report),
        outcome,
    }
}

#[cfg(test)]
mod tests;
