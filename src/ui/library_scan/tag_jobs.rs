//! Tag review and media transactions share the filesystem scanner. Catalog
//! saves remain independently serviceable while a large source is inspected.
use super::*;
use crate::library::{
    tags::{Patch, Review},
    Catalog, Metadata,
};
use crate::media_tags::{
    write::{self, Record},
    Observation,
};

pub(in crate::ui) const MAX_REVIEW: usize = 4096;
pub(in crate::ui) struct Row {
    pub target: Option<Review>,
    pub metadata: Metadata,
    pub observation: Result<Observation, String>,
}
pub(in crate::ui) struct Preview {
    pub rows: Vec<Row>,
    // These captures can survive a GUI publication. The worker's preview pin
    // owns their eventual destruction even after the dialog closes.
    _catalog: Arc<Catalog>,
    _rows: Arc<Vec<LibItem>>,
    _indices: Arc<Vec<usize>>,
}
pub(in crate::ui) enum Task {
    Inspect {
        catalog: Arc<Catalog>,
        rows: Arc<Vec<LibItem>>,
        indices: Arc<Vec<usize>>,
        begin: usize,
        end: usize,
    },
    Apply {
        target: Review,
        patch: Patch,
        root: PathBuf,
        embedded: bool,
    },
    Recover {
        root: PathBuf,
    },
    Cleanup {
        record: Record,
        unchanged: bool,
    },
}
pub(in crate::ui) enum Reply {
    Preview(Preview),
    Applied(write::Applied),
    Sidecar {
        target: Review,
        patch: Patch,
        observation: Result<Observation, String>,
        notice: String,
    },
    Recover(Vec<write::Recovery>),
    Cleaned,
    Failed {
        message: String,
        record: Option<Record>,
    },
}
pub(super) struct Job {
    pub id: u64,
    pub task: Task,
}
pub(in crate::ui) struct Handle {
    pub id: u64,
    pub work: Arc<WorkPermit>,
}
impl Handle {
    pub fn cancel(&self) {
        self.work.cancel().store(true, Ordering::Release);
    }
    pub fn cancelled(&self) -> bool {
        self.work.cancelled()
    }
}

pub(super) fn run(task: Task, work: &Arc<WorkPermit>) -> Reply {
    let result = (|| -> Result<Reply, String> {
        match task {
            Task::Inspect {
                catalog,
                rows,
                indices,
                begin,
                end,
            } => {
                if begin >= end || end > indices.len() || end - begin > MAX_REVIEW {
                    return Err("Tag review requires 1–4,096 captured rows".into());
                }
                let mut reviewed = Vec::with_capacity(end - begin);
                for &index in &indices[begin..end] {
                    if work.cancelled() {
                        return Err("Tag review cancelled".into());
                    }
                    let item = rows.get(index).ok_or("Captured crate row is unavailable")?;
                    let target = catalog.track(&item.source).and_then(|track| {
                        let fingerprint = item.fingerprint?;
                        (track.versions[track.current].fingerprint == Some(fingerprint)
                            && matches!(
                                item.source,
                                LibSource::File(_) | LibSource::Removable { .. }
                            ))
                        .then(|| Review {
                            id: track.id.clone(),
                            source: item.source.clone(),
                            fingerprint,
                        })
                    });
                    let observation = target
                        .as_ref()
                        .ok_or_else(|| "Only saved current local media can be edited".to_string())
                        .and_then(|target| {
                            crate::media_location::Location::resolve(&target.source)
                                .map_err(|e| e.to_string())
                                .and_then(|location| {
                                    crate::media_tags::inspect(
                                        &location,
                                        target.fingerprint,
                                        &work.cancel(),
                                    )
                                })
                        });
                    reviewed.push(Row {
                        target,
                        metadata: item.stored_metadata(),
                        observation,
                    });
                }
                Ok(Reply::Preview(Preview {
                    rows: reviewed,
                    _catalog: catalog,
                    _rows: rows,
                    _indices: indices,
                }))
            }
            Task::Apply {
                target,
                patch,
                root,
                embedded,
            } => {
                patch.validate()?;
                if work.cancelled() {
                    return Err("Tag edit cancelled before file access".into());
                }
                let location = crate::media_location::Location::resolve(&target.source)
                    .map_err(|e| e.to_string())?;
                // Sidecars also require the reviewed file still to exist with
                // the same identity. They never authorize a different file.
                let snapshot =
                    crate::media_location::Snapshot::discover().map_err(|e| e.to_string())?;
                if snapshot
                    .inspect(&location)
                    .map(|m| FileFingerprint::from_metadata(&m))
                    .ok()
                    != Some(target.fingerprint)
                {
                    return Err("Media changed since review; inspect its current tags again".into());
                }
                let observation =
                    crate::media_tags::inspect(&location, target.fingerprint, &work.cancel());
                if work.cancelled() {
                    return Err("Tag edit cancelled before writing".into());
                }
                if embedded && observation.as_ref().is_ok_and(Observation::supports_write) {
                    match write::apply(&location, &target, &patch, &root, work) {
                        Ok(applied) => return Ok(Reply::Applied(applied)),
                        Err(error) if error.record.is_some() || !error.fallback_safe => {
                            return Ok(Reply::Failed {
                                message: error.message,
                                record: error.record,
                            })
                        }
                        Err(error) => {
                            if work.cancelled() {
                                return Err(error.message);
                            }
                            return Ok(Reply::Sidecar {
                                target,
                                patch,
                                observation,
                                notice: format!(
                                    "Embedded tags unchanged; saving sidecar: {}",
                                    error.message
                                ),
                            });
                        }
                    }
                }
                let notice = if embedded {
                    "Embedded writing unavailable; saving a library sidecar"
                } else {
                    "Saving a library sidecar; media bytes unchanged"
                }
                .into();
                Ok(Reply::Sidecar {
                    target,
                    patch,
                    observation,
                    notice,
                })
            }
            Task::Recover { root } => Ok(Reply::Recover(write::recover(&root))),
            Task::Cleanup { record, unchanged } => {
                if unchanged {
                    write::discard_staged(&record)?;
                } else {
                    write::finalize(&record)?;
                }
                Ok(Reply::Cleaned)
            }
        }
    })();
    result.unwrap_or_else(|message| Reply::Failed {
        message,
        record: None,
    })
}

impl LibraryScan {
    pub fn tag_task(
        &mut self,
        task: Task,
        existing_work: Option<Arc<WorkPermit>>,
    ) -> Result<Handle, String> {
        if self.active() {
            return Err("Wait for the current filesystem operation".into());
        }
        // Recovery and cleanup must remain retryable after a failed worker,
        // without requiring an unrelated scan or reusing old operation IDs.
        if self.requests.is_none() {
            let performance = self.performance.clone();
            let watching = self.watch_enabled;
            let next_search = self.next_search;
            *self = Self::default();
            self.performance = performance;
            self.watch_enabled = watching;
            self.next_search = next_search;
        }
        let work = match existing_work {
            Some(work) => work,
            None => Arc::new(
                self.performance
                    .optional_work()
                    .map_err(|e| e.to_string())?,
            ),
        };
        let id = self
            .next_search
            .checked_add(1)
            .ok_or("Filesystem operation identity exhausted; restart")?;
        let options = Options::default();
        let request = Request {
            tags: Some(Job { id, task }),
            replacement: None,
            performance: self.performance.clone(),
            watch_enabled: self.watch_enabled,
            watch: None,
            work: work.clone(),
            roots: Vec::new(),
            kind: Kind::Import,
            baseline: Arc::new(Vec::new()),
            cancel: work.cancel(),
            progress: Arc::new(Progress::default()),
            options,
        };
        self.requests
            .as_ref()
            .ok_or("Filesystem worker unavailable; retry scan to restart it")?
            .try_send(request)
            .map_err(|_| "Filesystem worker unavailable or still busy")?;
        self.next_search = id;
        self.tag_result = None;
        self.cancel = work.cancel();
        self.state = ScanState::Tags;
        Ok(Handle { id, work })
    }
    pub fn tag_result(&mut self, id: u64) -> Option<Arc<Reply>> {
        if self
            .tag_result
            .as_ref()
            .is_some_and(|(found, _)| *found == id)
        {
            self.tag_result.take().map(|(_, result)| result)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn result(scan: &mut LibraryScan, id: u64) -> Arc<Reply> {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            scan.poll();
            if let Some(result) = scan.tag_result(id) {
                return result;
            }
            assert!(
                Instant::now() < deadline,
                "filesystem receipt never arrived"
            );
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    #[test]
    fn failed_filesystem_owner_reports_uncertainty_and_restarts_for_explicit_recovery() {
        let root = std::env::temp_dir().join(format!(
            "omat-tags-worker-{}",
            crate::sampler_bank::BankId::new().unwrap()
        ));
        let mut scan = LibraryScan::default();
        scan.set_tag_hook(|_| panic!("injected filesystem owner failure"));
        let first = scan
            .tag_task(Task::Recover { root: root.clone() }, None)
            .unwrap();
        let failure = result(&mut scan, first.id);
        assert!(
            matches!(failure.as_ref(), Reply::Failed { message, .. } if message.contains("unconfirmed"))
        );
        assert!(scan.requests.is_none());
        let retry = scan.tag_task(Task::Recover { root }, None).unwrap();
        assert!(retry.id > first.id);
        let recovered = result(&mut scan, retry.id);
        assert!(matches!(recovered.as_ref(), Reply::Recover(rows) if rows.is_empty()));
        assert!(
            scan.summary.is_none(),
            "recovery unexpectedly scanned or replaced a library"
        );
    }
}
