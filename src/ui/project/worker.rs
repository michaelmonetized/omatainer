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

pub(super) struct TemplateUse {
    pub record: super::super::templates::worker::Record,
    pub identities: Vec<WatchIdentity>,
    pub target: Option<usize>,
    pub view: UiState,
    pub current_path: Option<PathBuf>,
}

pub(super) enum Job {
    Save {
        path: PathBuf,
        overwrite: Overwrite,
        view: UiState,
        identities: Vec<WatchIdentity>,
        cancel: Arc<AtomicBool>,
    },
    Prepare {
        _work: crate::engine::performance::WorkPermit,
        path: Option<PathBuf>,
        recovery: Option<crate::recovery::Candidate>,
        template: Option<TemplateUse>,
        version: Option<super::super::project_versions::worker::Record>,
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
        recovered: bool,
        version_name: Option<String>,
        report: Vec<String>,
        template: Option<(crate::project_template::Metadata, Option<crate::project_template::Target>)>,
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
    jobs: Sender<(Job, Option<Arc<AtomicBool>>)>,
    pub events: Receiver<Event>,
}
impl Worker {
    pub fn start(
        handle: Handle,
        _output_sr: u32,
        recent_file: Option<PathBuf>,
    ) -> std::io::Result<Self> {
        #[cfg(test)]
        let hooks = Hooks::default();
        #[cfg(test)]
        let thread_hooks = hooks.clone();
        let (jobs, requests) = bounded::<(Job, Option<Arc<AtomicBool>>)>(1);
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
                while let Ok((job, recovery_fence)) = requests.recv() {
                    let result = match retire_recovery_capture(&handle, recovery_fence.as_deref(), job.cancel()) {
                        Err(error) => match &job { Job::CheckClose { .. } => close_error(error), _ => engine_error(error) },
                        Ok(()) => perform(
                        job,
                        &handle,
                        handle.sample_rate(),
                        &events,
                        #[cfg(test)]
                        &thread_hooks,
                    ) };
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
    pub fn submit(&self, job: Job, recovery_fence: Option<Arc<AtomicBool>>) -> Result<(), String> {
        self.jobs
            .try_send((job, recovery_fence))
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
            _work,
            path,
            recovery,
            template,
            version,
            cancel,
            commit,
        } => {
            let mut path = match path.map(resolve).transpose() {
                Ok(path) => path,
                Err(event) => return event,
            };
            let recovered = recovery.is_some();
            let version_name = version.as_ref().map(|v| v.entry.name.clone());
            let mut report = Vec::new();
            let mut template_metadata = None;
            let (prepared, view) = if let Some(use_template) = template {
                let source = match super::super::templates::worker::reviewed(&use_template.record, &cancel) {
                    Ok(source) => source, Err(error) => return Event::Failed(error),
                };
                let metadata = source.state.metadata;
                let (state, media, mut view) = if let Some(target) = use_template.target {
                    let crate::project_template::Kind::Track { bus } = &metadata.kind else { return Event::Failed("Use a track configuration template for this action".into()); };
                    let captured = match handle.capture(&cancel) { Ok(captured) => captured, Err(error) => return engine_error(error) };
                    let (state, media) = match captured.state.apply_track_configuration(captured.media,
                        &source.state.document.engine, &source.media, target, bus) {
                        Ok(result) => result, Err(error) => return Event::Failed(error),
                    };
                    let mut view = use_template.view;
                    view.deck_identities = std::array::from_fn(|deck| captured.playback_receipts[deck].as_ref()
                        .and_then(|receipt| use_template.identities.iter().find(|identity| identity.receipt.same_request(receipt)).map(|identity| identity.identity.clone())));
                    for origin in source.state.document.view.media_origins {
                        if !view.media_origins.iter().any(|current| current.key == origin.key) { view.media_origins.push(origin); }
                    }
                    path = use_template.current_path;
                    (state, media, view)
                } else {
                    if metadata.kind != crate::project_template::Kind::Project { return Event::Failed("A project template is required".into()); }
                    let mut state = source.state.document.engine;
                    let layout = state.session.as_mut().unwrap();
                    layout.namespace = crate::engine::midi_edit::NoteId::new().words();
                    layout.generation = match layout.generation.checked_add(1) { Some(generation) => generation, None => return Event::Failed("Template session generation exhausted".into()) };
                    path = None;
                    (state, source.media, source.state.document.view)
                };
                if use_template.target.is_none() { view.deck_identities = std::array::from_fn(|_| None); }
                let mut document = Document { engine: state, view, mapping_schema: FACTORY_MAPPING_SCHEMA };
                if let Err(error) = super::super::templates::worker::retain_origins(&mut document, &media, &cancel) { return Event::Failed(error); }
                if media.len() > project_file::DEFAULT_MEDIA_LIMIT || media.iter().map(|sample| sample.data.len() as u64 * 4).sum::<u64>() > Limits::default().max_pcm_bytes {
                    return Event::Failed("Template result exceeds native project media limits; no changes were applied".into());
                }
                let target = use_template.target.and_then(|target| document.engine.session.as_ref().and_then(|layout| crate::project_template::Target::capture(layout, target)));
                let prepared = match Prepared::from_state(document.engine, media, output_sr) { Ok(prepared) => prepared, Err(error) => return engine_error(error) };
                template_metadata = Some((metadata, target));
                (prepared, document.view)
            } else if let Some(record) = version {
                let bundle = match super::super::project_versions::worker::reviewed(&record, &cancel) { Ok(bundle) => bundle, Err(error) => return Event::Failed(error) };
                let prepared = match Prepared::from_state(bundle.state.engine, bundle.media, output_sr) { Ok(prepared) => prepared, Err(error) => return engine_error(error) };
                (prepared, bundle.state.view)
            } else if let Some(candidate) = recovery {
                let recovered = match crate::recovery::recover::<Document>(&candidate, &cancel) {
                    Ok(recovered) => recovered,
                    Err(error) => return Event::Failed(format!("Recovery refused; current session preserved: {error}")),
                };
                report = recovered.report;
                let bundle = recovered.bundle;
                if let Err(error) = bundle.state.validate() { return Event::Failed(error); }
                let prepared = match Prepared::from_state(bundle.state.engine, bundle.media, output_sr) {
                    Ok(prepared) => prepared,
                    Err(error) => return engine_error(error),
                };
                (prepared, bundle.state.view)
            } else if let Some(path) = &path {
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
                    recovered,
                    version_name,
                    report,
                    template: template_metadata,
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

impl Job {
    fn cancel(&self) -> &AtomicBool { match self { Self::Save { cancel, .. } | Self::Prepare { cancel, .. } | Self::CheckClose { cancel, .. } => cancel } }
}
// Only the cancelled recovery capture is waited on. An unrelated live project
// transaction retains its normal Busy result. No callback/GUI thread waits.
fn retire_recovery_capture(handle: &Handle, fence: Option<&AtomicBool>, cancel: &AtomicBool) -> Result<(), crate::engine::project::Error> {
    let Some(fence) = fence else { return Ok(()); };
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    while fence.load(Ordering::SeqCst) {
        if cancel.load(Ordering::Acquire) { return Err(crate::engine::project::Error::Cancelled); }
        if std::time::Instant::now() >= deadline { return Err(crate::engine::project::Error::Unavailable); }
        std::thread::sleep(Duration::from_millis(2));
    }
    handle.retire_cancelled_capture(cancel)
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
