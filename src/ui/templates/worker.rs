//! Template capture, verification and publication stay off the GUI and audio.
use super::*;
use crate::{
    engine::performance::WorkPermit,
    engine::project::Handle,
    project_file::{self, Bundle, Limits, Overwrite, SaveOutcome},
};
use crossbeam_channel::{bounded, Receiver, Sender};

pub(in crate::ui) type Native = model::Template<project::Document>;

#[derive(Clone)]
pub(in crate::ui) struct Record {
    pub path: PathBuf,
    pub fingerprint: FileFingerprint,
    pub metadata: model::Metadata,
    pub manifest: Arc<data::Manifest>,
}
pub(super) enum Kind {
    Save {
        path: PathBuf,
        name: String,
        track: Option<usize>,
        hardware: model::Hardware,
        view: project::UiState,
    },
    Inspect(PathBuf),
    Duplicate {
        record: Record,
        path: PathBuf,
        name: String,
    },
    Backup {
        record: Record,
        path: PathBuf,
    },
    ReviewArchive(PathBuf),
    Import {
        archive: PathBuf,
        destination: PathBuf,
        reviewed: Arc<data::Manifest>,
    },
}
pub(super) enum ResultData {
    Inspected(Record),
    Saved(PathBuf, SaveOutcome),
    Imported(PathBuf, SaveOutcome),
    Archive(PathBuf, Arc<data::Manifest>),
}
pub(super) struct Job {
    pub kind: Kind,
    pub work: WorkPermit,
}
pub(super) struct Worker {
    pub jobs: Sender<Job>,
    pub events: Receiver<Result<ResultData, String>>,
    #[cfg(test)]
    hook: Arc<parking_lot::Mutex<Option<(Sender<()>, Receiver<()>)>>>,
}
impl Worker {
    #[cfg(test)]
    pub fn pause_next(&self) -> (Receiver<()>, Sender<()>) {
        let (entered, observed) = bounded(1);
        let (resume, waiting) = bounded(1);
        *self.hook.lock() = Some((entered, waiting));
        (observed, resume)
    }
    /// Start one bounded worker for template captures and local file operations.
    /// `handle` supplies native state; returned channels admit one pending job.
    pub fn start(handle: Handle) -> Result<Self, String> {
        let (jobs, incoming) = bounded::<Job>(1);
        let (completed, events) = bounded(1);
        #[cfg(test)]
        let hook = Arc::new(parking_lot::Mutex::new(None::<(Sender<()>, Receiver<()>)>));
        #[cfg(test)]
        let thread_hook = hook.clone();
        std::thread::Builder::new()
            .name("project-templates".into())
            .spawn(move || {
                while let Ok(Job { kind, work }) = incoming.recv() {
                    #[cfg(test)]
                    if let Some((entered, resume)) = thread_hook.lock().take() {
                        let _ = entered.send(());
                        let _ = resume.recv();
                    }
                    let cancel = work.cancel();
                    let result = perform(kind, &handle, &cancel);
                    if work.cancelled() {
                        let _ = handle.retire_cancelled_capture(&cancel);
                    }
                    if completed.send(result).is_err() {
                        break;
                    }
                }
            })
            .map_err(|error| error.to_string())?;
        Ok(Self {
            jobs,
            events,
            #[cfg(test)]
            hook,
        })
    }
}

/// Load and validate a bounded native template while retaining its file identity.
/// `path` is absolute; `cancel` interrupts reads. Returns owned state and media.
pub(in crate::ui) fn load(
    path: &Path,
    cancel: &AtomicBool,
) -> Result<(Bundle<Native>, FileFingerprint), String> {
    model::absolute(path)?;
    let before = FileFingerprint::read(path)
        .ok_or_else(|| "Template file identity is unavailable".to_string())?;
    let bundle = project_file::load::<Native>(path, &Limits::default(), cancel)
        .map_err(|error| error.to_string())?;
    validate(&bundle)?;
    let after = FileFingerprint::read(path)
        .ok_or_else(|| "Template file identity is unavailable".to_string())?;
    if before != after {
        return Err("Template changed while reading; inspect it again".into());
    }
    Ok((bundle, after))
}

pub(in crate::ui) fn validate(bundle: &Bundle<Native>) -> Result<(), String> {
    bundle.state.metadata.validate()?;
    bundle.state.document.validate()?;
    bundle.state.document.engine.validate(&bundle.media)?;
    if bundle.state.document.engine.version < 7 {
        return Err(
            "Templates require native session identities; reopen and save the older project first"
                .into(),
        );
    }
    if matches!(bundle.state.metadata.kind, model::Kind::Track { .. }) {
        let state = &bundle.state.document.engine;
        if state.tracks.len() != 1
            || state.scene_fx.len() != 1
            || state.tracks[0].launch.is_some()
            || state.conductor.is_some()
            || state.scene_fx.iter().any(|rack| !rack.is_empty())
            || state.decks.iter().any(|deck| deck.audio.is_some())
            || state
                .banks
                .iter()
                .any(|bank| bank.media.iter().any(Option::is_some))
            || state.builtin.iter().any(Option::is_some)
            || state.tracks[0].clips.iter().any(|clip| {
                clip.kind != crate::engine::ClipKind::Empty
                    || !clip.notes.is_empty()
                    || clip.audio.is_some()
                    || clip.lanes.is_some()
                    || clip.region.is_some()
            })
        {
            return Err("Track configuration template contains song content".into());
        }
    }
    Ok(())
}

fn dependency_manifest(
    bundle: &Bundle<Native>,
    cancel: &AtomicBool,
) -> Result<data::Manifest, String> {
    let document = &bundle.state.document;
    let inventory = dependencies::embedded(
        &document.engine,
        &bundle.media,
        &document.view.media_origins,
        cancel,
    )?;
    super::super::portability::worker::manifest(document, &inventory, cancel)
}
pub(in crate::ui) fn reviewed(
    record: &Record,
    cancel: &AtomicBool,
) -> Result<Bundle<Native>, String> {
    let (bundle, fingerprint) = load(&record.path, cancel)?;
    if fingerprint != record.fingerprint
        || bundle.state.metadata != record.metadata
        || dependency_manifest(&bundle, cancel)? != *record.manifest
    {
        return Err("Template changed since inspection; inspect it again".into());
    }
    Ok(bundle)
}

/// Keep aliases only for audio present in the candidate native document.
/// `document` and `media` are disposable worker-owned state; hashing is cancellable.
pub(in crate::ui) fn retain_origins(
    document: &mut project::Document,
    media: &[Arc<crate::engine::dsp::Sample>],
    cancel: &AtomicBool,
) -> Result<(), String> {
    let inventory = dependencies::embedded(&document.engine, media, &[], cancel)?;
    let keys: std::collections::HashSet<_> = inventory
        .assets
        .into_iter()
        .map(|asset| asset.key)
        .collect();
    document
        .view
        .media_origins
        .retain(|origin| keys.contains(&origin.key));
    document.validate()
}

fn perform(kind: Kind, handle: &Handle, cancel: &AtomicBool) -> Result<ResultData, String> {
    if cancel.load(Ordering::Acquire) {
        return Err("Template operation cancelled".into());
    }
    match kind {
        Kind::Save {
            path,
            name,
            track,
            mut hardware,
            mut view,
        } => {
            model::absolute(&path)?;
            let captured = handle.capture(cancel).map_err(|error| error.to_string())?;
            let (engine, media, kind) = if let Some(track) = track {
                let bus = captured
                    .state
                    .tracks
                    .get(track)
                    .and_then(|track| {
                        captured
                            .state
                            .session
                            .as_ref()
                            .and_then(|layout| layout.scenes.get(track.scene_bus))
                    })
                    .ok_or("Track scene-bus alias is unavailable")?
                    .name
                    .clone();
                let (engine, media) = captured.state.track_configuration(&captured.media, track)?;
                hardware
                    .routing
                    .routes
                    .retain(|route| usize::from(route.track) == track);
                for route in &mut hardware.routing.routes {
                    route.track = 0;
                }
                let origins = std::mem::take(&mut view.media_origins);
                view = project::UiState::default();
                view.media_origins = origins;
                (engine, media, model::Kind::Track { bus })
            } else {
                (captured.state, captured.media, model::Kind::Project)
            };
            view.deck_identities = std::array::from_fn(|_| None);
            let mut document = project::Document {
                engine,
                view,
                mapping_schema: project::FACTORY_MAPPING_SCHEMA,
            };
            retain_origins(&mut document, &media, cancel)?;
            let bundle = Bundle {
                state: model::Template {
                    metadata: model::Metadata {
                        schema: model::VERSION,
                        name,
                        kind,
                        hardware,
                    },
                    document,
                },
                media,
            };
            validate(&bundle)?;
            let outcome =
                project_file::save(&path, &bundle, Overwrite::Never, &Limits::default(), cancel)
                    .map_err(|error| error.to_string())?;
            Ok(ResultData::Saved(path, outcome))
        }
        Kind::Inspect(path) => {
            let (bundle, fingerprint) = load(&path, cancel)?;
            let manifest = Arc::new(dependency_manifest(&bundle, cancel)?);
            Ok(ResultData::Inspected(Record {
                path,
                fingerprint,
                metadata: bundle.state.metadata,
                manifest,
            }))
        }
        Kind::Duplicate { record, path, name } => {
            model::absolute(&path)?;
            let mut bundle = reviewed(&record, cancel)?;
            bundle.state.metadata.name = name;
            validate(&bundle)?;
            let outcome =
                project_file::save(&path, &bundle, Overwrite::Never, &Limits::default(), cancel)
                    .map_err(|error| error.to_string())?;
            Ok(ResultData::Saved(path, outcome))
        }
        Kind::Backup { record, path } => {
            let bundle = reviewed(&record, cancel)?;
            let parent = super::super::portability::worker::destination_parent(&path)?;
            let stage = data::Stage::new(parent)?;
            let outcome = data::export(&path, &stage, &bundle, (*record.manifest).clone(), cancel)?;
            Ok(ResultData::Saved(path, outcome))
        }
        Kind::ReviewArchive(path) => Ok(ResultData::Archive(
            path.clone(),
            Arc::new(data::preview(&path, cancel)?),
        )),
        Kind::Import {
            archive,
            destination,
            reviewed,
        } => {
            let parent = super::super::portability::worker::destination_parent(&destination)?;
            let (stage, manifest, mut bundle) = data::extract::<Native>(&archive, parent, cancel)?;
            if manifest != *reviewed {
                return Err("Archive changed since review; review it again".into());
            }
            validate(&bundle)?;
            super::super::portability::worker::rebind_document(
                &stage,
                &manifest,
                &mut bundle.state.document,
                &bundle.media,
                &destination,
                handle.sample_rate(),
                cancel,
            )?;
            project_file::save(
                &stage.path.join("session.omat"),
                &bundle,
                Overwrite::Replace,
                &Limits::default(),
                cancel,
            )
            .map_err(|error| error.to_string())?;
            std::fs::rename(
                stage.path.join("session.omat"),
                stage.path.join("template.omtemplate"),
            )
            .map_err(|error| error.to_string())?;
            let outcome = data::publish(stage, &destination, cancel)?;
            Ok(ResultData::Imported(
                destination.join("template.omtemplate"),
                outcome,
            ))
        }
    }
}
