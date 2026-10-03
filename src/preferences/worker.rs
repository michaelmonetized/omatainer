//! Settings I/O and backend discovery never run inside App::update.
use super::{storage, Preferences};
use crate::engine::audio::config::{self, Inventory, Plan};
use crossbeam_channel::{bounded, Receiver, Sender};
use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

#[derive(Clone)]
pub struct Startup {
    pub path: PathBuf,
    pub home: PathBuf,
    pub preferences: Preferences,
    pub revision: Option<storage::Revision>,
    pub blocked: bool,
    pub diagnostic: Option<String>,
}
impl Startup {
    pub fn read(path: PathBuf, home: PathBuf) -> Self {
        let mut state = Self {
            path,
            preferences: Preferences::defaults(&home),
            home,
            revision: None,
            blocked: false,
            diagnostic: None,
        };
        match std::fs::symlink_metadata(&state.path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            _ => {
                match storage::load(&state.path, &AtomicBool::new(false)) {
                    Ok(loaded) => {
                        state.preferences = loaded.preferences;
                        state.revision = loaded.revision;
                        state.diagnostic = loaded.migrated.then(|| format!("Preferences migrated in memory; Apply saves version {}.", super::VERSION));
                    }
                    Err(error) => {
                        state.blocked = true;
                        state.diagnostic = Some(format!("Preferences were preserved: {error}"));
                    }
                }
            }
        }
        state
    }
}

pub enum Job {
    Preview(Preferences),
    Save {
        preferences: Preferences,
        revision: Option<storage::Revision>,
    },
    Import(PathBuf),
    Export {
        path: PathBuf,
        preferences: Preferences,
    },
    ImportShortcuts(PathBuf),
    ExportShortcuts { path: PathBuf, profile: super::Profile },
    Reload,
    Reset(Preferences),
}
#[derive(Debug)]
pub enum Event {
    Preview {
        preferences: Preferences,
        inventory: Result<Inventory, String>,
        plan: Result<Plan, String>,
        paths: Vec<String>,
    },
    Saved {
        preferences: Preferences,
        saved: storage::Saved,
    },
    Imported(Preferences, bool),
    Exported(storage::Saved),
    ShortcutsImported(super::shortcuts::Bundle),
    ShortcutsExported(storage::Saved),
    Reloaded(storage::Loaded),
    Failed(String),
    Cancelled,
}
pub struct Worker {
    jobs: Sender<(Job, Arc<AtomicBool>, crate::engine::performance::WorkPermit)>,
    performance: crate::engine::performance::Handle,
    results: Receiver<Event>,
    cancel: Option<Arc<AtomicBool>>,
    #[cfg(test)]
    pub delay: Arc<std::sync::atomic::AtomicU64>,
}
impl Worker {
    pub fn start(path: PathBuf) -> std::io::Result<Self> {
        Self::with_discovery(path, config::discover)
    }
    pub fn with_discovery(
        path: PathBuf,
        discover: impl Fn() -> Result<Inventory, String> + Send + 'static,
    ) -> std::io::Result<Self> {
        let (jobs, input) = bounded::<(Job, Arc<AtomicBool>, crate::engine::performance::WorkPermit)>(1);
        let (output, results) = bounded(1);
        #[cfg(test)]
        let delay = Arc::new(std::sync::atomic::AtomicU64::new(0));
        #[cfg(test)]
        let delayed = delay.clone();
        std::thread::Builder::new()
            .name("omatainer-settings".into())
            .spawn(move || {
                while let Ok((job, cancel, permit)) = input.recv() {
                    #[cfg(test)]
                    std::thread::sleep(std::time::Duration::from_millis(
                        delayed.load(Ordering::Acquire),
                    ));
                    let event = if cancel.load(Ordering::Acquire) {
                        Event::Cancelled
                    } else {
                        execute(&path, job, &cancel, &discover, &permit)
                    };
                    if output.send(event).is_err() {
                        break;
                    }
                }
            })?;
        Ok(Self {
            performance: crate::engine::performance::Handle::default(),
            jobs,
            results,
            cancel: None,
            #[cfg(test)]
            delay,
        })
    }
    pub fn busy(&self) -> bool {
        self.cancel.is_some()
    }
    pub fn set_performance(&mut self, performance: crate::engine::performance::Handle) { self.performance = performance; }
    pub fn request(&mut self, job: Job) -> Result<(), String> {
        if self.busy() {
            return Err("A preferences operation is already pending".into());
        }
        let permit = self.performance.optional_work().map_err(|error| error.to_string())?;
        let cancel = permit.cancel();
        self.jobs
            .try_send((job, cancel.clone(), permit))
            .map_err(|_| "Preferences worker unavailable".to_string())?;
        self.cancel = Some(cancel);
        Ok(())
    }
    pub fn cancel(&self) {
        if let Some(cancel) = &self.cancel {
            cancel.store(true, Ordering::Release);
        }
    }
    pub fn poll(&mut self) -> Option<Event> {
        match self.results.try_recv() {
            Ok(event) => {
                let cancelled = self.cancel.take().is_some_and(|cancel| cancel.load(Ordering::Acquire));
                Some(if cancelled && matches!(event, Event::ShortcutsImported(_)) { Event::Cancelled } else { event })
            }
            Err(crossbeam_channel::TryRecvError::Disconnected) if self.busy() => {
                self.cancel = None;
                Some(Event::Failed(
                    "Preferences worker disconnected; current settings retained".into(),
                ))
            }
            _ => None,
        }
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.cancel();
    }
}
fn failure(error: storage::Error) -> Event {
    if matches!(error, storage::Error::Cancelled) {
        Event::Cancelled
    } else {
        Event::Failed(error.to_string())
    }
}
fn validate_startup(preferences: &Preferences, cancel: &AtomicBool) -> Result<(), String> {
    preferences.validate()?;
    if let crate::project_template::Startup::Template { .. } = &preferences.current().unwrap().startup.session {
        crate::ui::startup_session(&preferences.current().unwrap().startup.session, cancel)?;
    }
    Ok(())
}
fn execute(
    path: &std::path::Path,
    job: Job,
    cancel: &AtomicBool,
    discover: &impl Fn() -> Result<Inventory, String>,
    permit: &crate::engine::performance::WorkPermit,
) -> Event {
    match job {
        Job::Preview(preferences) => {
            if let Err(error) = validate_startup(&preferences, cancel) {
                return Event::Failed(error);
            }
            let inventory = discover();
            if cancel.load(Ordering::Acquire) {
                return Event::Cancelled;
            }
            let current = preferences.current().expect("validated active profile");
            let plan = inventory
                .as_ref()
                .map_err(Clone::clone)
                .and_then(|inventory| config::plan(&current.audio, inventory));
            let mut paths = Vec::new();
            for path in &current.library_roots {
                if cancel.load(Ordering::Acquire) {
                    return Event::Cancelled;
                }
                paths.push(match std::fs::metadata(path) {
                    Ok(meta) if meta.is_dir() => format!("{}: available", path.display()),
                    Ok(_) => format!("{}: not a directory; scan will report it", path.display()),
                    Err(error) => format!("{}: unavailable ({error})", path.display()),
                });
            }
            if cancel.load(Ordering::Acquire) {
                Event::Cancelled
            } else {
                Event::Preview {
                    preferences,
                    inventory,
                    plan,
                    paths,
                }
            }
        }
        Job::Save {
            preferences,
            revision,
        } => {
            if let Err(error) = validate_startup(&preferences, cancel) { return if cancel.load(Ordering::Acquire) { Event::Cancelled } else { Event::Failed(error) }; }
            match storage::save_protected(
            path,
            &preferences,
            storage::Overwrite::Exact(revision),
            cancel,
            permit,
        ) {
            Ok(saved) => Event::Saved { preferences, saved },
            Err(error) => failure(error),
        } },
        Job::Reset(preferences) => {
            let _commit = match permit.commit() { Ok(guard) => guard, Err(error) => return Event::Failed(error.to_string()) };
            match storage::backup_and_reset(path, &preferences, cancel) {
            Ok(saved) => Event::Saved { preferences, saved },
            Err(error) => failure(error),
        } },
        Job::Import(path) => match storage::load(&path, cancel) {
            Ok(loaded) => Event::Imported(loaded.preferences, loaded.migrated),
            Err(error) => failure(error),
        },
        Job::Export { path, preferences } => {
            match storage::save_protected(&path, &preferences, storage::Overwrite::New, cancel, permit) {
                Ok(saved) => Event::Exported(saved),
                Err(error) => failure(error),
            }
        }
        Job::ImportShortcuts(path) => match storage::read_raw(&path, cancel) {
            Ok((bytes, _)) => match super::shortcuts::Bundle::decode(&bytes) {
                Ok(bundle) => Event::ShortcutsImported(bundle),
                Err(error) => Event::Failed(error),
            },
            Err(error) => failure(error),
        },
        Job::ExportShortcuts { path, profile } => {
            let bundle = super::shortcuts::Bundle::from_profile(&profile);
            if let Err(error) = bundle.apply(&mut profile.clone()) { return Event::Failed(error); }
            match serde_json::to_vec_pretty(&bundle) {
                Ok(bytes) => match storage::publish_new(&path, &bytes, cancel, permit) {
                    Ok(saved) => Event::ShortcutsExported(saved),
                    Err(error) => failure(error),
                },
                Err(error) => Event::Failed(error.to_string()),
            }
        },
        Job::Reload => match storage::load(path, cancel) {
            Ok(loaded) => Event::Reloaded(loaded),
            Err(error) => failure(error),
        },
    }
}
