//! Project workflow and dirty baselines; blocking work stays in `worker`.
use super::*;
use crate::engine::project::{Applied, CloseGuard, Handle};
use crate::project_file::Overwrite;
use crossbeam_channel::{bounded, Sender};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
mod worker;
use worker::{Event, Job, Worker};

pub(crate) const FACTORY_MAPPING_SCHEMA: u32 = 1;

#[cfg(test)]
impl App {
    pub(super) fn project_pending_for_test(&self) -> bool {
        self.project.busy() || self.project.awaiting_snapshot.is_some()
    }
    pub(super) fn project_path_text_for_test(&self) -> Option<String> {
        match &self.project.dialog { Some(Dialog::Path { text, .. }) => Some(text.clone()), _ => None }
    }
    pub(super) fn project_result_for_test(&self) -> (Option<PathBuf>, Option<String>) {
        (self.project.current_path.clone(), self.project.message.clone())
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Document {
    pub engine: crate::engine::project::State,
    pub view: UiState,
    pub mapping_schema: u32,
}
impl Document {
    pub(super) fn validate(&self) -> Result<(), String> {
        if self.mapping_schema != FACTORY_MAPPING_SCHEMA {
            return Err(format!(
                "Unsupported controller mapping schema {}; this build supports {}",
                self.mapping_schema, FACTORY_MAPPING_SCHEMA
            ));
        }
        if self.engine.version < 9 && !self.view.media_origins.is_empty() { return Err("Legacy projects cannot contain dependency source aliases".into()); }
        self.view.validate()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SavedIdentity {
    pub source: LibSource,
    pub fingerprint: Option<FileFingerprint>,
}
#[derive(Clone)]
pub(super) struct WatchIdentity {
    pub receipt: Receipt,
    pub identity: SavedIdentity,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct UiState {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub media_origins: Vec<crate::project_dependencies::Origin>,
    pub library_filter: String,
    #[serde(default)]
    pub selected_crate: Option<crate::library::crates::CrateId>,
    pub library_selection: Option<LibSource>,
    pub library_offset: f32,
    pub keys_open: bool,
    pub midi_open: bool,
    pub diagnostics_open: bool,
    #[serde(default)]
    pub history_open: bool,
    #[serde(default)]
    pub deck_time: [DeckTimeSettings; DECKS],
    // Verified identities accompany captured receipts. Embedded path strings
    // alone cannot credit a same-path replacement in the live library.
    pub(super) deck_identities: [Option<SavedIdentity>; DECKS],
}
impl UiState {
    pub(super) fn validate(&self) -> Result<(), String> {
        crate::project_dependencies::validate_origins(&self.media_origins)?;
        if self.selected_crate.as_ref().is_some_and(|id| id.0.len() != 32 || !id.0.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))) {
            return Err("Invalid project view: malformed crate identity".into());
        }
        if self.library_filter.len() > 4096
            || !self.library_offset.is_finite()
            || self.library_offset < 0.0
            || self
                .deck_time
                .iter()
                .any(|settings| settings.warning_lead_seconds > deck_time::MAX_WARNING_LEAD_SECONDS)
        {
            return Err(
                "Invalid project view: filter, scroll position or deck warning lead is outside supported bounds"
                    .into(),
            );
        }
        let valid_source = |source: &LibSource| match source {
            LibSource::Builtin(_) => true,
            LibSource::File(path) => !path.as_os_str().is_empty() && path.as_os_str().len() <= 4096,
            source => crate::library::validate_source(source).is_ok(),
        };
        if self
            .library_selection
            .as_ref()
            .is_some_and(|source| !valid_source(source))
            || self.deck_identities.iter().flatten().any(|identity| {
                !valid_source(&identity.source)
                    || matches!(identity.source, LibSource::File(_))
                        && identity.fingerprint.is_none()
            })
        {
            return Err("Invalid project view: media identity is incomplete".into());
        }
        Ok(())
    }
    fn editable_eq(&self, other: &Self) -> bool {
        self.media_origins == other.media_origins
            && self.selected_crate == other.selected_crate
            && self.library_filter == other.library_filter
            && self.library_selection == other.library_selection
            && self.library_offset == other.library_offset
            && self.keys_open == other.keys_open
            && self.midi_open == other.midi_open
            && self.diagnostics_open == other.diagnostics_open
            && self.history_open == other.history_open
            && self.deck_time == other.deck_time
    }
}

#[derive(Clone)]
struct Baseline {
    revision: u64,
    checkpoint: crate::engine::undo::Checkpoint,
    view: UiState,
    edits: u64,
}
impl Baseline {
    fn matches(&self, other: &Self) -> bool {
        self.revision == other.revision && self.edits == other.edits && self.saved_matches(other)
    }
    fn saved_matches(&self, other: &Self) -> bool {
        self.checkpoint == other.checkpoint && self.view.editable_eq(&other.view)
    }
}
#[derive(Clone)]
enum Action {
    New,
    OpenDialog,
    Open(PathBuf),
    Recover(crate::recovery::Candidate),
    Template(super::templates::worker::Record, Option<usize>),
    Close,
}
#[derive(Clone, Copy, PartialEq)]
enum SaveKind {
    Save,
    As,
    Copy,
}
enum PathKind {
    Open(Baseline),
    Save(SaveKind, Option<Action>),
}
enum Dialog {
    Unsaved(Action),
    Path {
        kind: PathKind,
        text: String,
        replace: bool,
    },
}
enum Operation {
    Save {
        kind: SaveKind,
        after: Option<Action>,
    },
    Prepare {
        before: Baseline,
        commit: Sender<u64>,
        committing: bool,
    },
    CloseCheck {
        before: Baseline,
        discard: bool,
    },
}
struct Active {
    operation: Operation,
    cancel: Arc<AtomicBool>,
}

pub(super) struct Projects {
    worker: Option<Worker>,
    current_path: Option<PathBuf>,
    recent: Vec<PathBuf>,
    recent_warning: Option<String>,
    clean: Option<Baseline>,
    active: Option<Active>,
    dialog: Option<Dialog>,
    message: Option<String>,
    allow_close: bool,
    closing_library: bool,
    close_guard: Option<CloseGuard>,
    awaiting_snapshot: Option<u64>,
    title: String,
    pending_selection: Option<(LibSource, usize, String)>,
    pub(super) local_edits: u64,
    restarting: bool,
}
impl Drop for Projects {
    fn drop(&mut self) {
        if let Some(active) = &self.active {
            active.cancel.store(true, Ordering::Release);
        }
        // Dropping channels ends the worker after its current cancellable
        // boundary. Never join filesystem/audio waits on the GUI thread.
    }
}
impl Projects {
    pub fn new(handle: Handle, output_sr: u32) -> Self {
        #[cfg(not(test))]
        let recent_file = Some(crate::theme::config_dir().join("recent-projects.json"));
        #[cfg(test)]
        let recent_file = None;
        Self::with_recent(handle,output_sr,recent_file)
    }
    pub(super) fn with_recent(handle:Handle,output_sr:u32,recent_file:Option<PathBuf>)->Self {
        let (worker, message) = match Worker::start(handle, output_sr, recent_file) {
            Ok(worker) => (Some(worker), None),
            Err(error) => (None, Some(format!("Project worker unavailable: {error}"))),
        };
        Self {
            worker,
            current_path: None,
            recent: Vec::new(),
            recent_warning: None,
            clean: None,
            active: None,
            dialog: None,
            message,
            allow_close: false,
            closing_library: false,
            close_guard: None,
            awaiting_snapshot: None,
            title: String::new(),
            pending_selection: None,
            local_edits: 0,
            restarting: false,
        }
    }
    pub fn committing(&self) -> bool {
        self.closing_library
            || self.awaiting_snapshot.is_some()
            || self.active.as_ref().is_some_and(|active| {
                matches!(
                    active.operation,
                    Operation::Prepare {
                        committing: true,
                        ..
                    } | Operation::CloseCheck { .. }
                )
            })
    }
    fn busy(&self) -> bool {
        self.active.is_some() || self.closing_library && !self.allow_close
    }
    pub fn dialog_is_closed(&self) -> bool {
        self.dialog.is_none()
    }
}

impl App {
    fn begin_coordinated_library_close(&mut self, ctx: &egui::Context) {
        self.project.closing_library = true;
        self.request_library_close(ctx);
    }
    pub(super) fn project_admission_sealed(&self) -> bool {
        self.project.close_guard.is_some()
    }
    pub(super) fn allow_project_close(&mut self, ctx: &egui::Context) {
        self.begin_recovery_close(ctx);
    }
    pub(super) fn request_normal_restart(&mut self) {
        if !self.engine.safe_mode() || self.project.busy() || self.project.committing() || self.project.dialog.is_some() {return;}
        self.project.restarting=true;
        self.request_project_action(Action::Close);
    }
    pub(super) fn finish_project_close(&mut self, ctx: &egui::Context) {
        self.begin_session_history_close(ctx);
    }
    pub(super) fn finish_history_exit(&mut self, ctx: &egui::Context) {
        self.project.allow_close = true;
        self.support.restart.store(self.project.restarting,Ordering::Release);
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }
    pub(super) fn cancel_project_close(&mut self) {
        self.project.restarting=false;
        self.support.restart.store(false,Ordering::Release);
        self.project.closing_library = false;
        self.project.close_guard = None;
        self.project.allow_close = false;
        self.cancel_library_close();
        self.cancel_recovery_close();
        self.cancel_session_history_close();
    }
    pub(super) fn project_view(&mut self) -> UiState {
        let selection = self.selected_library_item().map(|item| item.source.clone());
        let selection = if let Some((source, index, filter)) = &self.project.pending_selection {
            if self.lib_sel == *index && self.lib_filter == *filter {
                Some(source.clone())
            } else {
                self.project.pending_selection = None;
                selection
            }
        } else {
            selection
        };
        UiState {
            media_origins: self.dependencies.origins.clone(),
            library_filter: self.lib_filter.clone(),
            selected_crate: self.library_crates.selected.clone(),
            library_selection: selection,
            library_offset: self
                .library_view
                .pending_offset
                .unwrap_or(self.library_view.offset)
                .max(0.0),
            keys_open: self.keys_open,
            midi_open: self.midi_open,
            diagnostics_open: self.diagnostics.open,
            history_open: self.undo_history.open,
            deck_time: self.deck_time,
            deck_identities: Default::default(),
        }
    }
    fn project_baseline(&mut self) -> Baseline {
        Baseline {
            revision: self.engine.project.revision(),
            checkpoint: self.engine.undo.checkpoint(),
            view: self.project_view(),
            edits: self.project.local_edits,
        }
    }
    pub(super) fn project_dirty(&mut self) -> bool {
        let now = self.project_baseline();
        self.project
            .clean
            .as_ref()
            .is_none_or(|clean| !clean.saved_matches(&now))
            || self.engine.cmd.ui_request_stats().pending > 0
            || self
                .loads
                .iter()
                .flatten()
                .any(|load| matches!(load.phase, Phase::Loading | Phase::Queued))
    }
    pub(super) fn initialize_project_baseline(&mut self) {
        self.project.clean = Some(self.project_baseline());
    }

    pub(super) fn initialize_project_panels(&mut self, keys_open: bool, midi_open: bool) {
        self.keys_open = keys_open;
        self.midi_open = midi_open;
        // Startup panel defaults are the initial view, not a user edit. Preserve
        // the engine checkpoint so earlier accepted edits remain dirty.
        if let Some(clean) = &mut self.project.clean {
            clean.view.keys_open = keys_open;
            clean.view.midi_open = midi_open;
        }
    }

    fn begin_project_job(&mut self, operation: Operation, job: Job, cancel: Arc<AtomicBool>) {
        let recovery_fence = self.suspend_recovery();
        let result = self
            .project
            .worker
            .as_ref()
            .ok_or_else(|| "Project worker unavailable".to_owned())
            .and_then(|worker| worker.submit(job, recovery_fence));
        match result {
            Ok(()) => {
                self.project.message = None;
                self.project.active = Some(Active { operation, cancel });
            }
            Err(error) => self.project.message = Some(error),
        }
    }
    pub(super) fn project_performance_message(&mut self, message: String) { self.project.message = Some(message); }
    /// Refuse replacement while editors retain unapplied project-specific work.
    /// Returns whether the user must apply or explicitly discard that work first.
    fn guard_project_drafts(&mut self) -> bool {
        let dependencies = self.dependencies.guard_replacement();
        let timing = self.timing.guard_replacement();
        let blocked = dependencies || timing;
        if blocked { self.project.message = Some("Project replacement cancelled while editors retain unapplied work. Apply it or explicitly discard it, then choose the project action again.".into()); }
        blocked
    }
    /// Open a published imported session through the existing unsaved-work guard.
    /// `path` is its native session; installation stays stopped and cancellable.
    pub(super) fn open_imported_project(&mut self, path: PathBuf) { self.request_project_action(Action::Open(path)); }
    /// Use a reviewed template without assigning its source as the live save path.
    /// Project copies use the unsaved guard; track copies retain current music.
    pub(super) fn use_native_template(&mut self, record: super::templates::worker::Record) {
        let target = matches!(record.metadata.kind, crate::project_template::Kind::Track { .. }).then_some(self.snap.selected_track);
        if target.is_some() {
            if !self.project.busy() { self.begin_project_action(Action::Template(record, target)); }
        } else { self.request_project_action(Action::Template(record, None)); }
    }
    fn request_project_action(&mut self, action: Action) {
        if self.reject_protected_project() { return; }
        if !matches!(action, Action::Close) && self.guard_project_drafts() { return; }
        if self.project.busy() {
            return;
        }
        if self.project_dirty()
            || matches!(action, Action::Close)
                && (!self.engine.cmd.is_connected() || self.project.worker.is_none())
        {
            self.project.dialog = Some(Dialog::Unsaved(action));
        } else {
            self.begin_project_action(action);
        }
    }
    fn begin_project_action(&mut self, action: Action) {
        if self.reject_protected_project() { return; }
        if !matches!(action, Action::Close) && self.guard_project_drafts() { return; }
        match action {
            Action::OpenDialog => {
                self.project.dialog = Some(Dialog::Path {
                    kind: PathKind::Open(self.project_baseline()),
                    text: self
                        .project
                        .current_path
                        .as_ref()
                        .map(|path| path.display().to_string())
                        .unwrap_or_default(),
                    replace: false,
                });
            }
            Action::New | Action::Open(_) | Action::Recover(_) | Action::Template(_, _) => {
                let before = self.project_baseline();
                let (path, recovery, template) = match action {
                    Action::Open(path) => (Some(path), None, None),
                    Action::Recover(candidate) => (None, Some(candidate), None),
                    Action::Template(record, target) => (None, None, Some(worker::TemplateUse { record, target,
                        view: self.project_view(), identities: self.project_watch_identities(), current_path: self.project.current_path.clone() })),
                    _ => (None, None, None),
                };
                let work = match self.engine.cmd.performance().optional_work() {
                    Ok(work) => work,
                    Err(error) => { self.project.message = Some(error.to_string()); return; }
                };
                let cancel = work.cancel();
                let (commit, receiver) = bounded(1);
                self.begin_project_job(
                    Operation::Prepare {
                        before,
                        commit,
                        committing: false,
                    },
                    Job::Prepare {
                        _work: work,
                        path,
                        recovery,
                        template,
                        cancel: cancel.clone(),
                        commit: receiver,
                    },
                    cancel,
                );
            }
            Action::Close => self.begin_project_close(false),
        }
    }
    fn begin_project_close(&mut self, discard: bool) {
        if self.reject_protected_project() { return; }
        let before = self.project_baseline();
        let cancel = Arc::new(AtomicBool::new(false));
        self.begin_project_job(
            Operation::CloseCheck { before, discard },
            Job::CheckClose {
                cancel: cancel.clone(),
                discard,
            },
            cancel,
        );
    }
    fn request_project_save(&mut self, kind: SaveKind, after: Option<Action>) {
        if self.project.busy() {
            return;
        }
        if kind == SaveKind::Save {
            if let Some(path) = self.project.current_path.clone() {
                self.begin_project_save(kind, after, path, true);
                return;
            }
        }
        self.project.dialog = Some(Dialog::Path {
            kind: PathKind::Save(kind, after),
            text: self
                .project
                .current_path
                .as_ref()
                .map(|path| path.display().to_string())
                .unwrap_or_else(|| "Untitled.omat".into()),
            replace: false,
        });
    }
    fn begin_project_save(
        &mut self,
        kind: SaveKind,
        after: Option<Action>,
        path: PathBuf,
        replace: bool,
    ) {
        let view = self.project_view();
        let identities = self.project_watch_identities();
        let cancel = Arc::new(AtomicBool::new(false));
        self.begin_project_job(
            Operation::Save { kind, after },
            Job::Save {
                path,
                overwrite: if replace {
                    Overwrite::Replace
                } else {
                    Overwrite::Never
                },
                view,
                identities,
                cancel: cancel.clone(),
            },
            cancel,
        );
    }

    pub(super) fn poll_projects(&mut self, ctx: &egui::Context) {
        loop {
            let event = match self
                .project
                .worker
                .as_ref()
                .map(|worker| worker.events.try_recv())
            {
                Some(Ok(event)) => event,
                Some(Err(crossbeam_channel::TryRecvError::Disconnected)) => {
                    if let Some(active) = self.project.active.take() {
                        active.cancel.store(true, Ordering::Release);
                    }
                    self.project.worker = None;
                    self.project.message = Some("Project worker disconnected. The current session remains available; restart to retry project file operations.".into());
                    break;
                }
                _ => break,
            };
            match event {
                Event::Recent(paths, warning) => {
                    self.project.recent = paths;
                    self.project.recent_warning = warning;
                }
                Event::Ready => {
                    if self
                        .project
                        .active
                        .as_ref()
                        .is_none_or(|active| active.cancel.load(Ordering::Acquire)
                            || !matches!(active.operation, Operation::Prepare { .. }))
                    {
                        continue;
                    }
                    if self.guard_project_drafts() {
                        if let Some(active) = &self.project.active { active.cancel.store(true, Ordering::Release); }
                        continue;
                    }
                    // Finish older controller GUI requests before invalidating
                    // their file jobs. The mailbox is bounded to 16 requests.
                    self.poll_ui_requests();
                    self.poll_ui_requests();
                    let now = self.project_baseline();
                    let safe = self.project.active.as_ref().is_some_and(|active| {
                        match &active.operation {
                            Operation::Prepare { before, .. } => before.matches(&now),
                            _ => false,
                        }
                    });
                    if !safe {
                        if let Some(active) = &self.project.active {
                            active.cancel.store(true, Ordering::Release);
                        }
                        self.project.message = Some("Project was not opened: the current session changed while preparing it. Save or discard those edits and try again.".into());
                        continue;
                    }
                    for deck in 0..DECKS {
                        if let Some(loader) = &self.loader {
                            let _ = loader.invalidate(deck as u8);
                        }
                        if let Some(receipt) = self.loads[deck]
                            .as_ref()
                            .and_then(|load| load.receipt.as_ref())
                        {
                            receipt.cancel_pending();
                        }
                    }
                    if let Some(active) = &mut self.project.active {
                        if let Operation::Prepare {
                            commit, committing, ..
                        } = &mut active.operation
                        {
                            if commit.try_send(now.revision).is_ok() {
                                *committing = true;
                            } else {
                                active.cancel.store(true, Ordering::Release);
                            }
                        }
                    }
                }
                Event::Saved {
                    path,
                    revision,
                    checkpoint,
                    view,
                    warning,
                } => {
                    let Some(active) = self.project.active.take() else {
                        continue;
                    };
                    let Operation::Save { kind, after } = active.operation else {
                        continue;
                    };
                    if kind != SaveKind::Copy {
                        self.project.current_path = Some(path.clone());
                        self.project.clean = Some(Baseline {
                            revision,
                            checkpoint,
                            view,
                            edits: self.project.local_edits,
                        });
                    }
                    self.project.message = Some(match warning {
                        Some(warning) => {
                            format!("Saved {}. Durability warning: {warning}", path.display())
                        }
                        None => format!(
                            "Saved {}{}",
                            path.display(),
                            if kind == SaveKind::Copy {
                                " (copy; current project unchanged)"
                            } else {
                                ""
                            }
                        ),
                    });
                    if let Some(after) = after {
                        if self.project_dirty() {
                            self.project.message = Some(
                                "Saved the captured state; newer edits are still unsaved.".into(),
                            );
                            self.project.dialog = Some(Dialog::Unsaved(after));
                        } else if matches!(after, Action::Close) {
                            self.begin_project_action(Action::Close);
                        } else {
                            self.begin_project_action(after);
                        }
                    }
                }
                Event::Applied {
                    path,
                    view,
                    applied,
                    recovered,
                    report,
                    template,
                } => {
                    self.project.active = None;
                    self.install_project_view(ctx, view, applied);
                    self.project.current_path = path;
                    if let Some((metadata, target)) = template {
                        self.templates.loaded(metadata, target);
                        self.project.clean = None;
                        self.project.message = Some(if target.is_some() {
                            "Track configuration applied, stopped. Clips and other tracks were retained. Review saved hardware references in Templates before activating them.".into()
                        } else {
                            "Template instantiated as an unsaved stopped project. Save as chooses a new destination; the template remains intact. Review hardware references in Templates.".into()
                        });
                    } else if recovered {
                        self.project.current_path = None;
                        self.project.clean = None;
                        let notices = self.restored_recovery_report(report);
                        self.project.message = Some(format!("Recovered as an unsaved untitled copy, stopped. Use Save as to choose a destination; the original explicit project was not overwritten. {notices} recovery notices are available in Recovery."));
                    } else {
                        self.project.message = Some("Project ready, stopped. Space resumes remembered session clips; deck play buttons resume saved deck positions.".into());
                    }
                }
                Event::CheckedClose(revision, guard) => {
                    let Some(active) = self.project.active.take() else {
                        continue;
                    };
                    let Operation::CloseCheck { before, discard } = active.operation else {
                        continue;
                    };
                    if active.cancel.load(Ordering::Acquire) {
                        self.project.restarting=false;
                        self.project.message =
                            Some("Close cancelled; current session preserved.".into());
                        continue;
                    }
                    let now = self.project_baseline();
                    if discard
                        || before.matches(&now)
                            && revision == Some(now.revision)
                            && !self.project_dirty()
                    {
                        self.project.close_guard = Some(guard);
                        self.begin_coordinated_library_close(ctx);
                    } else {
                        self.project.dialog = Some(Dialog::Unsaved(Action::Close));
                    }
                }
                Event::CloseChanged => {
                    if self
                        .project
                        .active
                        .take()
                        .is_some_and(|active| active.cancel.load(Ordering::Acquire))
                    {
                        self.project.restarting=false;
                        self.project.message =
                            Some("Close cancelled; current session preserved.".into());
                        continue;
                    }
                    self.project.message = Some(
                        "The project changed before closing; review the unsaved changes.".into(),
                    );
                    self.project.dialog = Some(Dialog::Unsaved(Action::Close));
                }
                Event::CloseUnavailable(error) => {
                    let Some(active) = self.project.active.take() else {
                        continue;
                    };
                    let Operation::CloseCheck { discard, .. } = active.operation else {
                        continue;
                    };
                    if active.cancel.load(Ordering::Acquire) {
                        self.project.restarting=false;
                        self.project.message =
                            Some("Close cancelled; current session preserved.".into());
                    } else if discard {
                        // No renderer mutation committed: explicit Discard also
                        // authorizes losing accepted work when audio has stalled.
                        self.project.message = Some(format!(
                            "Closing without saving; audio renderer unavailable: {error}"
                        ));
                        self.begin_coordinated_library_close(ctx);
                    } else {
                        self.project.message = Some(format!("Could not verify a clean close: {error}. Save or explicitly discard before closing."));
                        self.project.dialog = Some(Dialog::Unsaved(Action::Close));
                    }
                }
                Event::Failed(error) => {
                    if !self.project.active.as_ref().is_some_and(|active|matches!(active.operation,Operation::CloseCheck{..})) {self.project.restarting=false;}
                    let code=if self.project.active.as_ref().is_some_and(|active|matches!(active.operation,Operation::Save{..})){crate::support::Code::ProjectWriteFailed}else{crate::support::Code::ProjectReadFailed};
                    self.engine.cmd.support_event(code,Some(crate::support::FailureClass::Unknown));
                    if self.project.active.take().is_some_and(|active| {
                        matches!(active.operation, Operation::CloseCheck { .. })
                            && !active.cancel.load(Ordering::Acquire)
                    }) {
                        self.project.dialog = Some(Dialog::Unsaved(Action::Close));
                    }
                    self.project.message = Some(format!("Project operation failed: {error}"));
                }
                Event::Cancelled => {
                    self.project.restarting=false;
                    self.project.active = None;
                    if self.project.message.is_none() {
                        self.project.message = Some("Project operation cancelled; current session and destination were preserved.".into());
                    }
                }
            }
        }
        if let Some((source, _, filter)) = self.project.pending_selection.clone() {
            if self.lib_filter == filter {
                self.refresh_library_view();
                if let Some(index) = self
                    .library_view
                    .indices
                    .iter()
                    .position(|&i| self.library[i].source == source)
                {
                    self.lib_sel = index;
                    self.project.pending_selection = None;
                    self.refresh_library_view();
                }
            }
        }
        if ctx.input(|input| input.viewport().close_requested()) && !self.project.allow_close {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.piano_roll.stop_for_close(&self.engine);
            self.midi_files.cancel();
            self.timing.cancel();
            self.dependencies.cancel();
            self.portability.cancel();
            self.templates.cancel();
            self.sampler_editor.stop_for_close(&self.engine);
            if self.templates.busy() {
                self.templates.open = true;
                self.project.message = Some("Close cancelled while template work settles. Wait for its publication or cancellation result before closing.".into());
            } else if self.portability.busy() {
                self.portability.open = true;
                self.project.message = Some("Close cancelled while portable project work settles. Wait for its publication or cancellation result before closing.".into());
            } else if self.dependencies.blocks_close() {
                self.dependencies.open = true;
                self.project.message = Some("Close cancelled while source review retains unapplied choices. Apply them or explicitly discard them before closing.".into());
            } else if self.timing.blocks_close() {
                self.timing.open = true;
                self.project.message = Some("Close cancelled while the timing editor retains unapplied work. Apply it or explicitly discard it before closing.".into());
            } else if self.midi_files.busy() {
                self.project.message = Some("Close cancelled while MIDI file work settles. Its pending import was cancelled if the renderer had not claimed it; completed changes remain in History.".into());
            } else if self.settings.busy() {
                self.settings.open = true;
                self.settings.message = "Close cancelled while a preferences operation is pending. Wait for its result or cancel it, then close again.".into();
            } else if self.piano_roll.blocks_close() {
                // The editor exposes its own explicit discard / cancel controls.
            } else if self.sampler_editor.blocks_close() {
                self.sampler_editor.close_pending();
            } else if !self.project.busy() && self.project.dialog.is_none() {
                self.request_project_action(Action::Close);
            }
        }
    }

    pub(crate) fn initialize_startup_session(&mut self, ctx: &egui::Context, view: UiState, metadata: Option<crate::project_template::Metadata>) {
        let applied = Applied { revision: self.engine.project.revision(), checkpoint: self.engine.undo.checkpoint(),
            playback_receipts: self.engine.initial_playback.clone() };
        self.install_project_view(ctx, view, applied);
        self.project.current_path = None;
        if let Some(metadata) = metadata { self.project.clean = None; self.templates.loaded(metadata, None); }
        self.project.message = Some("Startup session is stopped and unsaved. Save as a new project; review retained template hardware explicitly.".into());
    }

    fn install_project_view(&mut self, ctx: &egui::Context, view: UiState, applied: Applied) {
        self.project.awaiting_snapshot = Some(applied.revision);
        self.poll_play_history();
        self.loads = std::array::from_fn(|_| None);
        self.dependencies.install_origins(view.media_origins.clone());
        self.timing.reset_project();
        self.portability.reset_review();
        self.templates.hardware = None;
        self.lib_filter = view.library_filter.clone();
        self.choose_named_crate(view.selected_crate.clone());
        self.keys_open = view.keys_open;
        self.midi_open = view.midi_open;
        self.diagnostics.open = view.diagnostics_open;
        self.undo_history.open = view.history_open;
        self.deck_time = view.deck_time;
        self.clip_gain_edit = None;
        // Retire producer gate reservations before forgetting the old GUI
        // input owners; later key-up may otherwise see an already-cleared bit.
        for (pad, held) in self.pad_held.into_iter().enumerate() {
            if held { self.submit(Command::SamplerPad { pad: pad as u8, on: false }); }
        }
        self.pad_held = [false; 16];
        self.pad_inputs = [0; 16];
        accessibility::cancel_editor(ctx);

        self.refresh_library_view();
        self.project.pending_selection = None;
        if let Some(source) = view.library_selection.clone() {
            if let Some(index) = self
                .library_view
                .indices
                .iter()
                .position(|&i| self.library[i].source == source)
            {
                self.lib_sel = index;
            } else {
                self.project.pending_selection =
                    Some((source, self.lib_sel, self.lib_filter.clone()));
            }
        }
        self.library_view.pending_offset = Some(view.library_offset);
        self.refresh_library_view();
        self.cue_editor.project_receipts = applied.playback_receipts.clone();
        self.cue_editor.editor = None;
        for (identity, receipt) in view
            .deck_identities
            .into_iter()
            .zip(applied.playback_receipts)
        {
            if let (Some(identity), Some(receipt)) = (identity, receipt) {
                self.watch_playback(identity.source, identity.fingerprint, receipt);
            }
        }
        self.project.clean = Some(Baseline {
            revision: applied.revision,
            checkpoint: applied.checkpoint,
            view: self.project_view(),
            edits: self.project.local_edits,
        });
        self.publish_library_selection();
    }

    pub(super) fn confirm_project_snapshot(&mut self) {
        if self.project.awaiting_snapshot.is_some() && !self.engine.cmd.is_connected() {
            self.project.awaiting_snapshot = None;
            self.project.message = Some("Audio engine disconnected before confirming the project display. Project capture is unavailable; close offers an explicit discard option.".into());
        }
        if self
            .project
            .awaiting_snapshot
            .is_some_and(|revision| self.snap.project_revision >= revision)
        {
            self.project.awaiting_snapshot = None;
            self.deck_selection = deck_selection::Selection::new(self.snap.selected_deck_request);
        }
    }

    #[cfg(test)]
    pub(super) fn project_message_for_recovery_test(&self) -> Option<&str> { self.project.message.as_deref() }
    #[cfg(test)]
    pub(super) fn pause_project_prepare_for_recovery_test(&self) -> (crossbeam_channel::Receiver<()>, Sender<()>) {
        self.project.worker.as_ref().unwrap().pause_next(worker::Stage::Prepared)
    }
    pub(super) fn recovery_project_path(&self) -> Option<PathBuf> {
        self.project.current_path.clone()
    }
    pub(super) fn recovery_project_busy(&self) -> bool {
        self.project.busy() || self.project.committing() || !self.project.dialog_is_closed()
    }
    pub(super) fn request_recovery_restore(&mut self, candidate: crate::recovery::Candidate) {
        self.request_project_action(Action::Recover(candidate));
    }

    /// Read-only observations for the guided native project workflow.
    pub(super) fn project_help_state(&mut self) -> (Option<PathBuf>, bool, bool) {
        (self.project.current_path.clone(), self.project_dirty(), self.project.busy() || self.project.committing())
    }

    pub(super) fn project_toolbar(&mut self, ctx: &egui::Context) {
        let dirty = self.project_dirty();
        let name = self
            .project
            .current_path
            .as_ref()
            .and_then(|path| path.file_name())
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Untitled".into());
        let title = format!("{}{name} — omatainer", if dirty { "* " } else { "" });
        if title != self.project.title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.project.title = title;
        }
        let mut action = None;
        let mut save = None;
        egui::TopBottomPanel::top("project-toolbar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.add_enabled_ui(
                    !self.project.busy()
                        && !self.project.committing()
                        && self.project.dialog.is_none(),
                    |ui| {
                        self.undo_menu(ui);
                        let menu = ui.menu_button("Project", |ui| {
                            if ui.button("Support and crash reports…").help(ui,HelpControl::SupportOpen).clicked(){self.support.open=true;ui.close();}
                            let recovery = ui.button("Autosave and recovery…");
                            help::annotate(ui, &recovery, help::Control::RecoveryOpen);
                            if recovery.clicked() { self.recovery.open = true; ui.close(); }
                            let midi = ui.button("Import MIDI file…").help(ui, HelpControl::MidiFileImport);
                            if midi.clicked() { self.open_midi_files(false); ui.close(); }
                            let midi = ui.button("Export MIDI file…").help(ui, HelpControl::MidiFileExport);
                            if midi.clicked() { self.open_midi_files(true); ui.close(); }
                            if ui.button("Project dependencies…").help(ui, HelpControl::DependenciesOpen).clicked() { self.dependencies.open = true; ui.close(); }
                            if ui.button("Portable project…").help(ui, HelpControl::PortableOpen).clicked() { self.portability.open = true; ui.close(); }
                            if ui.button("Project and track templates…").help(ui, HelpControl::TemplateOpen).clicked() { self.templates.open = true; ui.close(); }
                            if ui.button("Tempo and meter…").help(ui, HelpControl::TimingOpen).clicked() { self.open_timing(); ui.close(); }
                            let response = ui.button("New project");
                            help::annotate(ui, &response, help::Control::ProjectNew);
                            if response.clicked() {
                                action = Some(Action::New);
                                ui.close();
                            }
                            let response = ui.button("Open project…");
                            help::annotate(ui, &response, help::Control::ProjectOpen);
                            if response.clicked() {
                                action = Some(Action::OpenDialog);
                                ui.close();
                            }
                            let recent = ui.menu_button("Open recent", |ui| {
                                if let Some(warning) = &self.project.recent_warning {
                                    ui.label(warning);
                                }
                                if self.project.recent.is_empty() {
                                    ui.label("No recent projects");
                                }
                                for path in &self.project.recent {
                                    let response = ui.button(path.display().to_string());
                                    help::annotate(ui, &response, help::Control::ProjectRecent);
                                    if response.clicked() {
                                        action = Some(Action::Open(path.clone()));
                                        ui.close();
                                    }
                                }
                            });
                            help::annotate(ui, &recent.response, help::Control::ProjectRecent);
                            ui.separator();
                            for (label, kind) in [
                                ("Save project", SaveKind::Save),
                                ("Save project as…", SaveKind::As),
                                ("Save project copy…", SaveKind::Copy),
                            ] {
                                let response = ui.button(label);
                                help::annotate(ui, &response, match kind {
                                    SaveKind::Save => help::Control::ProjectSave,
                                    SaveKind::As => help::Control::ProjectSaveAs,
                                    SaveKind::Copy => help::Control::ProjectSaveCopy,
                                });
                                if response.clicked() {
                                    save = Some(kind);
                                    ui.close();
                                }
                            }
                        });
                        help::annotate(ui, &menu.response, help::Control::ProjectMenu);
                    },
                );
                ui.label(format!(
                    "{name}{}",
                    if dirty {
                        " · unsaved"
                    } else if self.project.current_path.is_some() {
                        " · saved"
                    } else {
                        " · not saved to a file"
                    }
                ));
                if ui.button(self.session_history_toolbar_text()).help(ui, HelpControl::HistoryOpen).clicked() { self.session_history.open = true; }
                if self.snap.file_conductor {
                    ui.label(format!("Musical timeline · {}/{} · bar {} beat {:.2}", self.snap.meter_numerator, self.snap.meter_denominator, self.snap.bar, self.snap.beat_in_bar + 1.0));
                }
                if self.snap.count_in_remaining > 0.0 { ui.label(format!("Count-in · {:.1} s", self.snap.count_in_remaining)); }
                let recovery_text = self.recovery_toolbar_text();
                if ui.button(recovery_text).help(ui, HelpControl::RecoveryOpen).clicked() { self.recovery.open = true; }
                if self.project.awaiting_snapshot.is_some() {
                    ui.label("Waiting for project display…");
                }
                if let Some(active) = &self.project.active {
                    ui.label(match active.operation {
                        Operation::Save { .. } => "Saving…",
                        Operation::Prepare {
                            committing: true, ..
                        } => "Applying project…",
                        Operation::Prepare { .. } => "Preparing project…",
                        Operation::CloseCheck { .. } => "Checking pending edits…",
                    });
                    let cancel = ui.button("Cancel project operation");
                    help::annotate(ui, &cancel, help::Control::ProjectCancel);
                    if cancel.clicked() || cancel.is_pointer_button_down_on() {
                        active.cancel.store(true, Ordering::Release);
                    }
                }
            });
            if let Some(message) = &self.project.message {
                ui.horizontal_wrapped(|ui| {
                    ui.label(message);
                });
            }
        });
        if let Some(action) = action {
            self.request_project_action(action);
        }
        if let Some(kind) = save {
            self.request_project_save(kind, None);
        }
        self.project_dialogs(ctx);
    }

    fn project_dialogs(&mut self, ctx: &egui::Context) {
        let Some(mut dialog) = self.project.dialog.take() else {
            return;
        };
        keyboard::block_for_dialog(ctx);
        let mut retain = true;
        let mut action = None;
        let mut save = None;
        let mut recheck = None;
        let mut started_save = false;
        let response = egui::Modal::new(egui::Id::new("project-dialog")).show(ctx, |ui| {
            match &mut dialog {
                Dialog::Unsaved(next) => {
                    ui.heading("Unsaved project changes");
                    ui.label("Save the current project before continuing?");
                    ui.horizontal(|ui| {
                        let response = ui.button("Save changes");
                        help::annotate(ui, &response, help::Control::ProjectSave);
                        if response.clicked() { save = Some((SaveKind::Save, Some(next.clone()))); retain = false; }
                        let response = ui.button("Discard changes");
                        help::annotate(ui, &response, help::Control::ProjectDiscard);
                        if response.clicked() { action = Some(next.clone()); retain = false; }
                        let response = ui.button("Cancel");
                        help::annotate(ui, &response, help::Control::ProjectCancel);
                        if response.clicked() { retain = false; }
                    });
                }
                Dialog::Path { kind, text, replace } => {
                    let opening = matches!(kind, PathKind::Open(_));
                    ui.heading(if opening { "Open native project" } else { "Save native project" });
                    ui.label("Project path (.omat). Relative paths use the application working directory.");
                    let path_field = ui.add(egui::TextEdit::singleline(text).desired_width(440.0).hint_text("/path/to/session.omat"));
                    path_field.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, "Project file path"));
                    help::annotate(ui, &path_field, help::Control::ProjectPath);
                    if !opening {
                        let response = ui.checkbox(replace, "Replace an existing file at this path");
                        help::annotate(ui, &response, help::Control::ProjectReplace);
                    }
                    ui.horizontal(|ui| {
                        let response = ui.add_enabled(!text.trim().is_empty(), egui::Button::new(if opening { "Open" } else { "Save" }));
                        help::annotate(ui, &response, if opening { help::Control::ProjectOpen } else {
                            match kind { PathKind::Save(SaveKind::As, _) => help::Control::ProjectSaveAs,
                                PathKind::Save(SaveKind::Copy, _) => help::Control::ProjectSaveCopy, _ => help::Control::ProjectSave }
                        });
                        if response.clicked() {
                            let path = PathBuf::from(text.trim());
                            match kind { PathKind::Open(before) => {
                                if before.matches(&self.project_baseline()) { action = Some(Action::Open(path)); }
                                else { recheck = Some(Action::Open(path)); }
                            }, PathKind::Save(kind, after) => {
                                started_save = true;
                                self.begin_project_save(*kind, after.clone(), path, *replace);
                            } }
                            retain = false;
                        }
                        let response = ui.button("Cancel");
                        help::annotate(ui, &response, help::Control::ProjectCancel);
                        if response.clicked() { retain = false; }
                    });
                }
            }
        });
        if response.should_close() {
            retain = false;
        }
        if !retain && !started_save && action.is_none() && save.is_none() && recheck.is_none() {
            self.project.restarting=false;
            self.support.restart.store(false,Ordering::Release);
        }
        if retain {
            self.project.dialog = Some(dialog);
        }
        if let Some((kind, after)) = save {
            self.request_project_save(kind, after);
        }
        if let Some(action) = recheck {
            self.request_project_action(action);
        }
        if let Some(action) = action {
            if matches!(action, Action::Close) {
                // Explicit discard authorizes closing even with unsaved work.
                if self.engine.cmd.is_connected() && self.project.worker.is_some() {
                    self.begin_project_close(true);
                } else {
                    // Explicit Discard can close after a renderer/worker failure
                    // without claiming that pending creative work was saved.
                    self.begin_coordinated_library_close(ctx);
                }
            } else {
                self.begin_project_action(action);
            }
        }
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod contextual_help_tests {
    use super::*;
    use crate::ui::help::Control;
    use egui::accesskit;

    fn frame(ctx: &egui::Context, app: &mut App) -> Vec<(accesskit::NodeId, accesskit::Node)> {
        let mut output = None;
        for _ in 0..3 {
            output = Some(ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1600.0, 1200.0))),
                    ..Default::default()
                },
                |ctx| app.update_frame(ctx),
            ));
        }
        output
            .unwrap()
            .platform_output
            .accesskit_update
            .unwrap()
            .nodes
    }
    fn described(
        nodes: &[(accesskit::NodeId, accesskit::Node)],
        control: Control,
    ) -> &accesskit::Node {
        nodes
            .iter()
            .map(|(_, node)| node)
            .find(|node| {
                node.description()
                    .is_some_and(|description| description.contains(control.definition().purpose))
            })
            .unwrap_or_else(|| panic!("Missing actual dialog help for {control:?}"))
    }
    fn setup() -> (test_support::Fixture, egui::Context) {
        let mut fixture = test_support::Fixture::new(64);
        fixture.rt.publish_for_test();
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        (fixture, ctx)
    }

    #[test]
    fn project_path_and_unsaved_decisions_expose_specific_help_even_when_save_disabled() {
        let (mut fixture, ctx) = setup();
        fixture.app.project.dialog = Some(Dialog::Path {
            kind: PathKind::Save(SaveKind::Copy, None),
            text: String::new(),
            replace: false,
        });
        let before = fixture.app.engine.project.revision();
        let nodes = frame(&ctx, &mut fixture.app);
        assert_eq!(
            described(&nodes, Control::ProjectPath).role(),
            accesskit::Role::TextInput
        );
        assert_eq!(
            described(&nodes, Control::ProjectReplace).role(),
            accesskit::Role::CheckBox
        );
        assert!(described(&nodes, Control::ProjectSaveCopy).is_disabled());
        described(&nodes, Control::ProjectCancel);
        fixture.app.project.dialog = Some(Dialog::Unsaved(Action::New));
        let nodes = frame(&ctx, &mut fixture.app);
        for control in [
            Control::ProjectSave,
            Control::ProjectDiscard,
            Control::ProjectCancel,
        ] {
            described(&nodes, control);
        }
        assert_eq!(
            fixture.app.engine.project.revision(),
            before,
            "help must not edit a document"
        );
    }

    #[test]
    fn history_diagnostics_and_library_controls_expose_reference_in_actual_app() {
        let (mut fixture, ctx) = setup();
        fixture.app.undo_history.open = true;
        let nodes = frame(&ctx, &mut fixture.app);
        assert!(described(&nodes, Control::Undo).is_disabled());
        assert!(described(&nodes, Control::Redo).is_disabled());
        fixture.app.undo_history.open = false;
        fixture.app.diagnostics.open = true;
        let nodes = frame(&ctx, &mut fixture.app);
        for control in [
            Control::DiagnosticCapture,
            Control::DiagnosticStop,
            Control::DiagnosticCancel,
            Control::DiagnosticPath,
            Control::DiagnosticExport,
            Control::DiagnosticReopen,
            Control::DiagnosticFileCancel,
        ] {
            described(&nodes, control);
        }
        assert!(described(&nodes, Control::DiagnosticExport).is_disabled());
        fixture.app.diagnostics.open = false;
        fixture.app.library_import_open = true;
        let nodes = frame(&ctx, &mut fixture.app);
        for control in [
            Control::LibraryImportPath,
            Control::LibraryImport,
            Control::LibraryRetry,
        ] {
            described(&nodes, control);
        }
        assert_eq!(
            described(&nodes, Control::LibraryImportPath).label(),
            Some("DJ library import path")
        );
    }

    #[test]
    fn load_help_preserves_original_deck_specific_accessible_names() {
        let (mut fixture, ctx) = setup();
        fixture.app.loads[1] = Some(load_status::LoadState::new(
            Some(Selection {
                source: LibSource::Builtin(crate::engine::media_source::BuiltinStem::Drums),
                title: "Test source".into(),
            }),
            load_status::Phase::Failed("Fixture failure".into()),
        ));
        let nodes = frame(&ctx, &mut fixture.app);
        assert_eq!(
            described(&nodes, Control::LoadRetry).label(),
            Some("Deck B: Retry media load")
        );
        assert_eq!(
            described(&nodes, Control::LoadDismiss).label(),
            Some("Deck B: Dismiss load status")
        );
    }
}

#[cfg(test)]
mod performance_gate_tests;
