use super::*;
use crate::engine::{performance::WorkPermit, project::Handle};
use crossbeam_channel::{bounded, Receiver, Sender};

#[derive(Clone)]
pub(super) struct Scope {
    pub revision: u64,
    pub namespace: [u64; 2],
}
#[derive(Clone)]
pub(super) struct Review {
    pub scope: Scope,
    pub inventory: Arc<data::Inventory>,
}
pub(super) enum Kind {
    Inspect,
    Search { review: Review, roots: Vec<PathBuf> },
    Verify { review: Review, choices: Vec<(data::Asset, data::Candidate)> },
}
pub(super) struct Job {
    pub kind: Kind,
    pub origins: Vec<data::Origin>,
    pub work: WorkPermit,
}
pub(super) enum ResultData {
    Inspected(Review),
    Searched(Review, data::Search),
    Verified(Scope, Vec<data::Origin>),
}
pub(super) struct Event {
    pub baseline: Vec<data::Origin>,
    pub result: Result<ResultData, String>,
}
pub(super) struct Worker { pub jobs: Sender<Job>, pub events: Receiver<Event> }
impl Worker {
    /// Start a bounded dependency worker for the actual project owner.
    /// `handle` captures coherent state; the returned channels admit one job.
    pub fn start(handle: Handle) -> Result<Self, String> {
        let (jobs, incoming) = bounded::<Job>(1);
        let (completed, events) = bounded(1);
        std::thread::Builder::new().name("project-dependencies".into()).spawn(move || {
            while let Ok(Job { kind, origins, work }) = incoming.recv() {
                let cancel = work.cancel();
                let result = (|| {
                    if work.cancelled() { return Err("Dependency operation cancelled".into()); }
                    let captured = handle.capture(&cancel).map_err(|e| e.to_string())?;
                    let scope = Scope { revision: captured.revision, namespace: captured.state.session.as_ref().ok_or("Project identity unavailable")?.namespace };
                    let expected = match &kind { Kind::Inspect => None, Kind::Search { review, .. } | Kind::Verify { review, .. } => Some(&review.scope) };
                    if expected.is_some_and(|expected| expected.revision != scope.revision || expected.namespace != scope.namespace) {
                        return Err("Project changed since inspection. Check dependencies again; current work was preserved.".into());
                    }
                    match kind {
                        Kind::Inspect => Ok(ResultData::Inspected(Review { scope, inventory: Arc::new(data::inspect(&captured.state, &captured.media, &origins, &cancel)?) })),
                        Kind::Search { review, roots } => Ok(ResultData::Searched(review.clone(), data::search(&review.inventory.assets, &roots, &cancel)?)),
                        Kind::Verify { choices, .. } => {
                            let verified = data::verify_choices(&choices, &cancel)?;
                            if handle.revision() != scope.revision { return Err("Project changed during source verification; no aliases were changed".into()); }
                            Ok(ResultData::Verified(scope, verified))
                        }
                    }
                })();
                if work.cancelled() { let _ = handle.retire_cancelled_capture(&cancel); }
                if completed.send(Event { baseline: origins, result }).is_err() { break; }
            }
        }).map_err(|e| e.to_string())?;
        Ok(Self { jobs, events })
    }
}
