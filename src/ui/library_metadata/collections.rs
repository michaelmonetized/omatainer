//! One explicit collection operation and one terminal receipt on the existing
//! catalog owner. Cancellation never relabels a transaction after its claim.
use super::Metadata;
use crate::{
    engine::performance::{self, WorkPermit},
    library::{
        crates::{CrateId, Edit},
        Store, TrackId,
    },
};
use std::{
    fmt,
    io::Read,
    sync::{
        atomic::{AtomicU8, Ordering},
        Arc,
    },
};

const MAX_SELECTION: usize = 4096;
const PENDING: u8 = 0;
const CANCELLED: u8 = 1;
const CLAIMED: u8 = 2;
const FINISHED: u8 = 3;

#[derive(Clone, Debug)]
pub(in crate::ui) enum Action {
    /// Fresh IDs are generated only on the catalog worker, never from a name.
    Create {
        name: String,
        parent: Option<CrateId>,
        before: Option<CrateId>,
    },
    Edit(Edit<TrackId>),
    Annotate { ids: Vec<TrackId>, patch: crate::library::annotations::Patch },
    Protect { targets: Vec<crate::library::protection::Target>, patch: crate::library::protection::Patch },
    Read,
}
impl Action {
    fn validate(&self) -> Result<(), Admission> {
        // Only bounded shape checks at UI admission. Duplicate membership and
        // whole-forest validation/allocation remain on the metadata worker.
        let invalid = |text: &str| Admission::Invalid(text.into());
        if let Self::Protect { targets,patch }=self {
            if targets.is_empty() || targets.len()>4096 || patch.is_empty() || targets.iter().any(|target|target.track.0.len()!=32 || !target.track.0.bytes().all(|b|b.is_ascii_digit() || (b'a'..=b'f').contains(&b))) {return Err(invalid("Review 1–4096 current track versions and at least one changed lock field"));}
            return Ok(());
        }
        if let Self::Annotate { ids, patch } = self {
            if ids.is_empty() || ids.len() > 100000 || patch.is_empty() || ids.iter().any(|id| id.0.len() != 32 || !id.0.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))) { return Err(invalid("Select 1–100000 stable tracks and at least one changed annotation field")); }
            patch.apply(&crate::library::annotations::Annotations::default()).map_err(|e|invalid(&e))?;
            return Ok(());
        }
        if let Self::Edit(Edit::SetAnnotationRule { rule: Some(rule), .. }) = self { rule.validate().map_err(|e|invalid(&e))?; }
        if let Self::Edit(Edit::SetSmartRule { rule: Some(rule), .. }) = self { rule.validate().map_err(|e|invalid(&e))?; }

        let name = match self {
            Self::Create { name, .. } | Self::Edit(Edit::Rename { name, .. }) => Some(name),
            Self::Edit(Edit::Create { .. }) => {
                return Err(invalid(
                    "Use Create so the catalog owner generates a fresh crate identity",
                ))
            }
            _ => None,
        };
        if name.is_some_and(|name| {
            name.is_empty()
                || name.len() > crate::library::crates::MAX_NAME_BYTES
                || name.trim() != name
                || name.chars().any(char::is_control)
        }) {
            return Err(invalid(
                "Crate names must be trimmed, nonempty, control-free and at most 256 UTF-8 bytes",
            ));
        }
        let members = match self {
            Self::Edit(
                Edit::AddMembers { members, .. }
                | Edit::RemoveMembers { members, .. }
                | Edit::MoveMembers { members, .. },
            ) => Some(members),
            _ => None,
        };
        if members.is_some_and(|members| members.len() > MAX_SELECTION) {
            return Err(invalid(
                "One edit accepts at most 4096 selected tracks; no membership was changed",
            ));
        }
        let valid = |text: &str| {
            text.len() == 32
                && text
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        };
        let crate_id = |id: &CrateId| valid(&id.0);
        let anchor = |id: &Option<CrateId>| id.as_ref().is_none_or(crate_id);
        let shape = match self {
            Self::Create { parent, before, .. } => anchor(parent) && anchor(before),
            Self::Edit(Edit::Rename { id, .. } | Edit::DeleteSubtree { id } | Edit::SetAnnotationRule { id, .. } | Edit::SetSmartRule { id, .. }) => crate_id(id),
            Self::Edit(Edit::MoveCrate { id, parent, before }) => {
                crate_id(id) && anchor(parent) && anchor(before)
            }
            Self::Edit(Edit::AddMembers { id, before, .. }) => {
                crate_id(id) && before.as_ref().is_none_or(|id| valid(&id.0))
            }
            Self::Edit(Edit::RemoveMembers { id, .. }) => crate_id(id),
            Self::Edit(Edit::MoveMembers {
                source,
                destination,
                before,
                ..
            }) => {
                crate_id(source)
                    && crate_id(destination)
                    && before.as_ref().is_none_or(|id| valid(&id.0))
            }
            Self::Read => true,
            Self::Annotate { .. } | Self::Protect { .. } => unreachable!(),
            Self::Edit(Edit::Create { .. }) => false,
        };
        if !shape || members.is_some_and(|members| members.iter().any(|id| !valid(&id.0))) {
            return Err(invalid(
                "Crate and track identities must be 32 lowercase hexadecimal characters",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub(in crate::ui) struct Token {
    pub id: u64,
    state: Arc<AtomicU8>,
}
impl Token {
    pub fn cancel(&self) -> bool {
        self.state
            .compare_exchange(PENDING, CANCELLED, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }
    fn claim(&self) -> Result<(), Failure> {
        self.state
            .compare_exchange(PENDING, CLAIMED, Ordering::AcqRel, Ordering::Acquire)
            .map(|_| ())
            .map_err(|_| Failure::Cancelled)
    }
    fn cancelled(&self) -> bool {
        self.state.load(Ordering::Acquire) == CANCELLED
    }
    fn finished(&self) {
        self.state.store(FINISHED, Ordering::Release);
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::ui) enum Admission {
    Unavailable,
    Busy,
    Closing,
    Performance(performance::Error),
    Invalid(String),
    Exhausted,
}
impl fmt::Display for Admission {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unavailable => f.write_str("Persistent crate writer is unavailable"),
            Self::Busy => {
                f.write_str("A crate request is pending; consume its result before another edit")
            }
            Self::Closing => {
                f.write_str("Library close is pending; choose Keep working before editing crates")
            }
            Self::Performance(error) => error.fmt(f),
            Self::Invalid(error) => error.fmt(f),
            Self::Exhausted => {
                f.write_str("Crate request identity exhausted; restart before editing")
            }
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::ui) enum Failure {
    Cancelled,
    Performance(performance::Error),
    Invalid(String),
    Storage(String),
    Unavailable,
}
impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cancelled => f.write_str("Crate request cancelled before publication"),
            Self::Performance(error) => error.fmt(f),
            Self::Invalid(error) | Self::Storage(error) => error.fmt(f),
            Self::Unavailable => {
                f.write_str("Persistent crate writer is unavailable; request was not applied")
            }
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::ui) enum Outcome {
    Read,
    Durable {
        changed: bool,
    },
    CommittedUnconfirmed(String),
    Rejected(Failure),
    /// A worker disappeared after its irreversible claim. Inspect the actual
    /// reopened catalog before retrying; no rejected/committed guess is made.
    Unknown(String),
}
#[derive(Clone, Debug)]
pub(in crate::ui) struct Receipt {
    pub id: u64,
    pub revision: u64,
    pub created: Option<CrateId>,
    pub outcome: Outcome,
}
impl Receipt {
    pub(super) fn unavailable(token: &Token, revision: u64) -> Self {
        let outcome = if matches!(token.state.load(Ordering::Acquire), CLAIMED | FINISHED) {
            Outcome::Unknown("Crate writer stopped after the publication claim; reopen the library to confirm the stored outcome".into())
        } else {
            Outcome::Rejected(Failure::Unavailable)
        };
        token.finished();
        Self {
            id: token.id,
            revision,
            created: None,
            outcome,
        }
    }
    pub(super) fn refused(request: Request, revision: u64, failure: Failure) -> Self {
        request.token.finished();
        Self {
            id: request.token.id,
            revision,
            created: None,
            outcome: Outcome::Rejected(failure),
        }
    }
}
pub(super) struct Request {
    token: Token,
    expected: u64,
    pub(super) action: Action,
    work: Option<WorkPermit>,
}

impl Metadata {
    pub fn set_collections_closing(&mut self, closing: bool) {
        self.collections_closing = closing;
    }
    pub fn edit_crates(&mut self, expected: u64, action: Action) -> Result<Token, Admission> {
        if self.worker_closed || self.storage.is_none() {
            return Err(Admission::Unavailable);
        }
        if self.collections_closing {
            return Err(Admission::Closing);
        }
        if self.collection_active.is_some() {
            return Err(Admission::Busy);
        }
        action.validate()?;
        let work = if matches!(action, Action::Read) {
            None
        } else {
            Some(
                self.performance
                    .optional_work()
                    .map_err(Admission::Performance)?,
            )
        };
        let id = self
            .next_collection
            .checked_add(1)
            .ok_or(Admission::Exhausted)?;
        let token = Token {
            id,
            state: Arc::new(AtomicU8::new(PENDING)),
        };
        self.next_collection = id;
        self.collection_active = Some(token.clone());
        self.collection = Some(Request {
            token: token.clone(),
            expected,
            action,
            work,
        });
        self.clear_error = true;
        self.revision = self.revision.wrapping_add(1);
        self.dirty = true;
        Ok(token)
    }
    pub fn take_collection_result(&mut self) -> Option<Receipt> {
        let result = self.collection_result.take();
        if result.is_some() {
            self.collection_active = None;
        }
        result
    }
}

pub(super) fn apply(store: &mut Store, request: Request) -> Receipt {
    apply_using(store, request, |_| {}, Store::save)
}
fn apply_using(
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
    let mut created = None;
    let mut outcome = None;
    let attempt = (|| {
        let check = || {
            if token.cancelled() {
                return Err(Failure::Cancelled);
            }
            if work.as_ref().is_some_and(WorkPermit::cancelled) {
                return Err(Failure::Performance(performance::Error::Protected));
            }
            Ok(())
        };
        check()?;
        if matches!(action, Action::Read) {
            token.claim()?;
            outcome = Some(Outcome::Read);
            return Ok(());
        }
        let mut candidate = store.catalog.clone();
        let changed = if let Action::Protect {targets,patch}=&action {
            if expected!=candidate.crates.revision() {return Err(Failure::Invalid("Crate selection changed; review preparation locks again".into()));}
            candidate.protect(targets,*patch).map_err(Failure::Invalid)?
        } else if let Action::Annotate { ids, patch } = &action {
            if expected != candidate.crates.revision() { return Err(Failure::Invalid("Crate selection changed; review the batch again".into())); }
            candidate.annotate(ids, patch).map_err(Failure::Invalid)?
        } else {
        let edit = match action {
            Action::Create {
                name,
                parent,
                before,
            } => {
                let mut bytes = [0u8; 16];
                std::fs::File::open("/dev/urandom")
                    .and_then(|mut file| file.read_exact(&mut bytes))
                    .map_err(|error| Failure::Storage(error.to_string()))?;
                let id = CrateId(bytes.iter().map(|byte| format!("{byte:02x}")).collect());
                created = Some(id.clone());
                Edit::Create {
                    id,
                    name,
                    parent,
                    before,
                }
            }
            Action::Edit(edit) => edit,
            Action::Read | Action::Annotate { .. } | Action::Protect { .. } => unreachable!(),
        };
        candidate.edit_crates(expected, &edit).map_err(Failure::Invalid)?
        };
        checkpoint(0);
        check()?;
        let guard = work
            .as_ref()
            .unwrap()
            .commit()
            .map_err(Failure::Performance)?;
        token.claim()?;
        checkpoint(1);
        let baseline = std::mem::replace(&mut store.catalog, candidate);
        let saved = save(store);
        if saved.is_ok() {
            outcome = Some(Outcome::Durable { changed });
        } else if store.last_save_replaced() {
            outcome = Some(Outcome::CommittedUnconfirmed(saved.unwrap_err()));
        } else {
            store.catalog = baseline;
            return Err(Failure::Storage(saved.unwrap_err()));
        }
        checkpoint(2);
        drop(guard);
        Ok(())
    })();
    if let Err(error) = attempt {
        created = None;
        outcome = Some(Outcome::Rejected(error));
    }
    token.finished();
    Receipt {
        id: token.id,
        revision: store.catalog.crates.revision(),
        created,
        outcome: outcome.unwrap(),
    }
}

#[cfg(test)]
mod tests;
