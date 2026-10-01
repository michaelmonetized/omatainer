//! Blocking engine handshakes and all project/config file I/O live here.
//! A prepared replacement stays on this worker until the GUI authorizes commit;
//! cancellation never hands a large DSP/media allocation back to the GUI.
use super::{Document, UiState, WatchIdentity, FACTORY_MAPPING_SCHEMA};
use crate::engine::project::{Applied, CloseGuard, Handle, Prepared};
use crate::project_file::{self, Bundle, Limits, Overwrite, SaveOutcome};
use crossbeam_channel::{bounded, Receiver, Sender};
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::Duration;

pub(super) enum Job {
    Save {
        path: PathBuf,
        overwrite: Overwrite,
        view: UiState,
        identities: Vec<WatchIdentity>,
        cancel: Arc<AtomicBool>,
    },
    Prepare {
        path: Option<PathBuf>,
        cancel: Arc<AtomicBool>,
        commit: Receiver<u64>,
    },
    CheckClose {
        cancel: Arc<AtomicBool>,
        discard: bool,
    },
}

pub(super) enum Event {
    Recent(Vec<PathBuf>, Option<String>),
    Ready,
    Saved {
        path: PathBuf,
        revision: u64,
        checkpoint: crate::engine::undo::Checkpoint,
        view: UiState,
        warning: Option<String>,
    },
    Applied {
        path: Option<PathBuf>,
        view: UiState,
        applied: Applied,
    },
    CheckedClose(Option<u64>, CloseGuard),
    CloseChanged,
    CloseUnavailable(String),
    Failed(String),
    Cancelled,
}

#[cfg(test)]
#[derive(Clone, Copy, PartialEq)]
pub(super) enum Stage {
    Captured,
    Prepared,
    CloseCaptured,
    BeforeInstall,
}
#[cfg(test)]
type Hooks = Arc<parking_lot::Mutex<Option<(Stage, Sender<()>, Receiver<()>)>>>;
#[cfg(test)]
fn pause_at(hooks: &Hooks, stage: Stage) {
    let gate = {
        let mut gate = hooks.lock();
        if gate.as_ref().is_some_and(|gate| gate.0 == stage) {
            gate.take()
        } else {
            None
        }
    };
    if let Some((_, entered, resume)) = gate {
        let _ = entered.send(());
        let _ = resume.recv();
    }
}

pub(super) struct Worker {
    #[cfg(test)]
    hooks: Hooks,
    jobs: Sender<Job>,
    pub events: Receiver<Event>,
}
impl Worker {
    pub fn start(
        handle: Handle,
        output_sr: u32,
        recent_file: Option<PathBuf>,
    ) -> std::io::Result<Self> {
        #[cfg(test)]
        let hooks = Hooks::default();
        #[cfg(test)]
        let thread_hooks = hooks.clone();
        let (jobs, requests) = bounded(1);
        let (events, results) = bounded(4);
        std::thread::Builder::new()
            .name("omatainer-project-io".into())
            .spawn(move || {
                let (mut recent, warning) =
                    match recent_file.as_deref().map(read_recent).transpose() {
                        Ok(recent) => (recent.unwrap_or_default(), None),
                        Err(error) => (Vec::new(), Some(error)),
                    };
                // A corrupt/newer/unreadable cache may contain recoverable
                // user state. Keep this session's list in memory until a future
                // startup can read it; primary project I/O remains available.
                let recent_read_warning = warning;
                if events
                    .send(Event::Recent(recent.clone(), recent_read_warning.clone()))
                    .is_err()
                {
                    return;
                }
                while let Ok(job) = requests.recv() {
                    let result = perform(
                        job,
                        &handle,
                        output_sr,
                        &events,
                        #[cfg(test)]
                        &thread_hooks,
                    );
                    let successful_path = match &result {
                        Event::Saved { path, .. }
                        | Event::Applied {
                            path: Some(path), ..
                        } => Some(path),
                        _ => None,
                    };
                    if let Some(path) = successful_path {
                        recent.retain(|candidate| candidate != path);
                        recent.insert(0, path.clone());
                        recent.truncate(12);
                        // Recent-path cache is auxiliary: a failure cannot undo or
                        // misreport a committed project file or installed session.
                        let warning = recent_read_warning.clone().or_else(|| {
                            recent_file
                                .as_ref()
                                .and_then(|file| write_recent(file, &recent).err())
                                .map(|error| {
                                    format!("Recent project history could not be saved: {error}")
                                })
                        });
                        if events.send(Event::Recent(recent.clone(), warning)).is_err() {
                            return;
                        }
                    }
                    if events.send(result).is_err() {
                        return;
                    }
                }
            })?;
        Ok(Self {
            #[cfg(test)]
            hooks,
            jobs,
            events: results,
        })
    }
    #[cfg(test)]
    pub fn pause_next(&self, stage: Stage) -> (Receiver<()>, Sender<()>) {
        let (entered, observed) = bounded(1);
        let (resume, waiting) = bounded(1);
        *self.hooks.lock() = Some((stage, entered, waiting));
        (observed, resume)
    }
    #[cfg(test)]
    pub fn disconnect_results(&mut self) {
        let (sender, receiver) = bounded(1);
        drop(sender);
        self.events = receiver;
    }
    pub fn submit(&self, job: Job) -> Result<(), String> {
        self.jobs
            .try_send(job)
            .map_err(|error| format!("Project worker did not accept the operation: {error}"))
    }
}

fn perform(
    job: Job,
    handle: &Handle,
    output_sr: u32,
    events: &Sender<Event>,
    #[cfg(test)] hooks: &Hooks,
) -> Event {
    let resolve = |path: PathBuf| {
        std::path::absolute(path)
            .map_err(|error| Event::Failed(format!("Cannot resolve project path: {error}")))
    };
    match job {
        Job::Save {
            path,
            overwrite,
            mut view,
            identities,
            cancel,
        } => {
            let path = match resolve(path) {
                Ok(path) => path,
                Err(event) => return event,
            };
            let captured = match handle.capture(&cancel) {
                Ok(captured) => captured,
                Err(error) => return engine_error(error),
            };
            view.deck_identities = std::array::from_fn(|deck| {
                captured.playback_receipts[deck]
                    .as_ref()
                    .and_then(|receipt| {
                        identities
                            .iter()
                            .find(|identity| identity.receipt.same_request(receipt))
                            .map(|identity| identity.identity.clone())
                    })
            });
            #[cfg(test)]
            pause_at(hooks, Stage::Captured);
            let revision = captured.revision;
            let checkpoint = captured.checkpoint;
            let state = Document {
                engine: captured.state,
                view: view.clone(),
                mapping_schema: FACTORY_MAPPING_SCHEMA,
            };
            if let Err(error) = state.validate() {
                return Event::Failed(error);
            }
            let bundle = Bundle {
                state,
                media: captured.media,
            };
            match project_file::save(&path, &bundle, overwrite, &Limits::default(), &cancel) {
                Ok(outcome) => Event::Saved {
                    path,
                    revision,
                    checkpoint,
                    view,
                    warning: match outcome {
                        SaveOutcome::Durable => None,
                        SaveOutcome::CommittedButDirectorySyncFailed(warning) => Some(warning),
                    },
                },
                Err(project_file::Error::Cancelled) => Event::Cancelled,
                Err(error) => Event::Failed(error.to_string()),
            }
        }
        Job::Prepare {
            path,
            cancel,
            commit,
        } => {
            let path = match path.map(resolve).transpose() {
                Ok(path) => path,
                Err(event) => return event,
            };
            let (prepared, view) = if let Some(path) = &path {
                let bundle = match project_file::load::<Document>(path, &Limits::default(), &cancel)
                {
                    Ok(bundle) => bundle,
                    Err(project_file::Error::Cancelled) => return Event::Cancelled,
                    Err(error) => return Event::Failed(error.to_string()),
                };
                if let Err(error) = bundle.state.validate() {
                    return Event::Failed(error);
                }
                let prepared =
                    match Prepared::from_state(bundle.state.engine, bundle.media, output_sr) {
                        Ok(prepared) => prepared,
                        Err(error) => return engine_error(error),
                    };
                (prepared, bundle.state.view)
            } else {
                let prepared = match Prepared::empty(output_sr) {
                    Ok(prepared) => prepared,
                    Err(error) => return engine_error(error),
                };
                (prepared, UiState::default())
            };
            #[cfg(test)]
            pause_at(hooks, Stage::Prepared);
            if cancel.load(Ordering::Acquire) {
                return Event::Cancelled;
            }
            if events.send(Event::Ready).is_err() {
                return Event::Cancelled;
            }
            let expected_revision = loop {
                if cancel.load(Ordering::Acquire) {
                    return Event::Cancelled;
                }
                match commit.recv_timeout(Duration::from_millis(20)) {
                    Ok(revision) => break revision,
                    Err(crossbeam_channel::RecvTimeoutError::Timeout) => continue,
                    Err(crossbeam_channel::RecvTimeoutError::Disconnected) => {
                        return Event::Cancelled
                    }
                }
            };
            #[cfg(test)]
            pause_at(hooks, Stage::BeforeInstall);
            match handle.install(prepared, expected_revision, &cancel) {
                Ok(applied) => Event::Applied {
                    path,
                    view,
                    applied,
                },
                Err(error) => engine_error(error),
            }
        }
        Job::CheckClose { cancel, discard } => {
            let revision = if discard {
                None
            } else {
                let captured = match handle.capture(&cancel) {
                    Ok(captured) => captured,
                    Err(error) => return close_error(error),
                };
                Some(captured.revision)
            };
            #[cfg(test)]
            pause_at(hooks, Stage::CloseCaptured);
            match handle.seal_for_close(revision, &cancel) {
                Ok(guard) => Event::CheckedClose(revision, guard),
                Err(crate::engine::project::Error::Conflict) => Event::CloseChanged,
                Err(error) => close_error(error),
            }
        }
    }
}

fn close_error(error: crate::engine::project::Error) -> Event {
    match error {
        crate::engine::project::Error::Busy | crate::engine::project::Error::Unavailable => {
            Event::CloseUnavailable(error.to_string())
        }
        error => engine_error(error),
    }
}

fn engine_error(error: crate::engine::project::Error) -> Event {
    match error {
        crate::engine::project::Error::Cancelled => Event::Cancelled,
        error => Event::Failed(error.to_string()),
    }
}

fn read_recent(path: &Path) -> Result<Vec<PathBuf>, String> {
    use std::io::Read;
    use std::os::unix::fs::OpenOptionsExt;
    let file = match std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
        .open(path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(format!("Recent project history could not be read: {error}")),
    };
    let metadata = file
        .metadata()
        .map_err(|error| format!("Recent project history metadata failed: {error}"))?;
    if !metadata.is_file() {
        return Err("Recent project history is not a regular file".into());
    }
    if metadata.len() > 65_536 {
        return Err("Recent project history exceeds its size limit".into());
    }
    let mut bytes = Vec::new();
    file.take(65_537)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("Recent project history could not be read: {error}"))?;
    if bytes.len() > 65_536 {
        return Err("Recent project history exceeds its size limit".into());
    }
    let paths = serde_json::from_slice::<Vec<PathBuf>>(&bytes)
        .map_err(|error| format!("Recent project history is invalid: {error}"))?;
    Ok(paths
        .into_iter()
        .filter(|path| path.is_absolute() && path.as_os_str().len() <= 4096)
        .take(12)
        .collect())
}
fn write_recent(path: &Path, recent: &[PathBuf]) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let parent = path
        .parent()
        .ok_or_else(|| std::io::Error::other("Recent path has no parent"))?;
    std::fs::create_dir_all(parent)?;
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if !metadata.file_type().is_file() => {
            return Err(std::io::Error::other(
                "Recent project history is not a regular file",
            ))
        }
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => return Err(error),
        _ => {}
    }
    let temporary = parent.join(format!(".recent-projects-{}.tmp", std::process::id()));
    let mut created = false;
    let result = (|| {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)?;
        created = true;
        let bytes = serde_json::to_vec(recent).map_err(std::io::Error::other)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        std::fs::rename(&temporary, path)?;
        std::fs::File::open(parent)?.sync_all()
    })();
    if created && result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}
