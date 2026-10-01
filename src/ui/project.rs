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

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Document {
    pub engine: crate::engine::project::State,
    pub view: UiState,
    pub mapping_schema: u32,
}
impl Document {
    fn validate(&self) -> Result<(), String> {
        if self.mapping_schema != FACTORY_MAPPING_SCHEMA {
            return Err(format!(
                "Unsupported controller mapping schema {}; this build supports {}",
                self.mapping_schema, FACTORY_MAPPING_SCHEMA
            ));
        }
        self.view.validate()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SavedIdentity {
    pub source: LibSource,
    pub fingerprint: Option<FileFingerprint>,
}
pub(super) struct WatchIdentity {
    pub receipt: Receipt,
    pub identity: SavedIdentity,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct UiState {
    pub library_filter: String,
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
    fn validate(&self) -> Result<(), String> {
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
        self.library_filter == other.library_filter
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
        self.project.allow_close = true;
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }
    pub(super) fn cancel_project_close(&mut self) {
        self.project.closing_library = false;
        self.project.close_guard = None;
        self.project.allow_close = false;
        self.cancel_library_close();
    }
    fn project_view(&mut self) -> UiState {
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
            library_filter: self.lib_filter.clone(),
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
    fn project_dirty(&mut self) -> bool {
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
        let result = self
            .project
            .worker
            .as_ref()
            .ok_or_else(|| "Project worker unavailable".to_owned())
            .and_then(|worker| worker.submit(job));
        match result {
            Ok(()) => {
                self.project.message = None;
                self.project.active = Some(Active { operation, cancel });
            }
            Err(error) => self.project.message = Some(error),
        }
    }
    fn request_project_action(&mut self, action: Action) {
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
            Action::New | Action::Open(_) => {
                let before = self.project_baseline();
                let path = match action {
                    Action::Open(path) => Some(path),
                    _ => None,
                };
                let cancel = Arc::new(AtomicBool::new(false));
                let (commit, receiver) = bounded(1);
                self.begin_project_job(
                    Operation::Prepare {
                        before,
                        commit,
                        committing: false,
                    },
                    Job::Prepare {
                        path,
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
                        .is_none_or(|active| active.cancel.load(Ordering::Acquire))
                    {
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
                } => {
                    self.project.active = None;
                    self.install_project_view(ctx, view, applied);
                    self.project.current_path = path;
                    self.project.message = Some("Project ready, stopped. Space resumes remembered session clips; deck play buttons resume saved deck positions.".into());
                }
                Event::CheckedClose(revision, guard) => {
                    let Some(active) = self.project.active.take() else {
                        continue;
                    };
                    let Operation::CloseCheck { before, discard } = active.operation else {
                        continue;
                    };
                    if active.cancel.load(Ordering::Acquire) {
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
                    if self.project.active.take().is_some_and(|active| {
                        matches!(active.operation, Operation::CloseCheck { .. })
                            && !active.cancel.load(Ordering::Acquire)
                    }) {
                        self.project.dialog = Some(Dialog::Unsaved(Action::Close));
                    }
                    self.project.message = Some(format!("Project operation failed: {error}"));
                }
                Event::Cancelled => {
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
            if self.settings.busy() {
                self.settings.open = true;
                self.settings.message = "Close cancelled while a preferences operation is pending. Wait for its result or cancel it, then close again.".into();
            } else if !self.project.busy() && self.project.dialog.is_none() {
                self.request_project_action(Action::Close);
            }
        }
    }

    fn install_project_view(&mut self, ctx: &egui::Context, view: UiState, applied: Applied) {
        self.project.awaiting_snapshot = Some(applied.revision);
        self.poll_play_history();
        self.loads = std::array::from_fn(|_| None);
        self.lib_filter = view.library_filter.clone();
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
                        ui.menu_button("Project", |ui| {
                            if ui.button("New project").clicked() {
                                action = Some(Action::New);
                                ui.close();
                            }
                            if ui.button("Open project…").clicked() {
                                action = Some(Action::OpenDialog);
                                ui.close();
                            }
                            ui.menu_button("Open recent", |ui| {
                                if let Some(warning) = &self.project.recent_warning {
                                    ui.label(warning);
                                }
                                if self.project.recent.is_empty() {
                                    ui.label("No recent projects");
                                }
                                for path in &self.project.recent {
                                    if ui.button(path.display().to_string()).clicked() {
                                        action = Some(Action::Open(path.clone()));
                                        ui.close();
                                    }
                                }
                            });
                            ui.separator();
                            for (label, kind) in [
                                ("Save project", SaveKind::Save),
                                ("Save project as…", SaveKind::As),
                                ("Save project copy…", SaveKind::Copy),
                            ] {
                                if ui.button(label).clicked() {
                                    save = Some(kind);
                                    ui.close();
                                }
                            }
                        });
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
        let response = egui::Modal::new(egui::Id::new("project-dialog")).show(ctx, |ui| {
            match &mut dialog {
                Dialog::Unsaved(next) => {
                    ui.heading("Unsaved project changes");
                    ui.label("Save the current project before continuing?");
                    ui.horizontal(|ui| {
                        if ui.button("Save changes").clicked() { save = Some((SaveKind::Save, Some(next.clone()))); retain = false; }
                        if ui.button("Discard changes").clicked() { action = Some(next.clone()); retain = false; }
                        if ui.button("Cancel").clicked() { retain = false; }
                    });
                }
                Dialog::Path { kind, text, replace } => {
                    let opening = matches!(kind, PathKind::Open(_));
                    ui.heading(if opening { "Open native project" } else { "Save native project" });
                    ui.label("Project path (.omat). Relative paths use the application working directory.");
                    let path_field = ui.add(egui::TextEdit::singleline(text).desired_width(440.0).hint_text("/path/to/session.omat"));
                    path_field.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, "Project file path"));
                    if !opening { ui.checkbox(replace, "Replace an existing file at this path"); }
                    ui.horizontal(|ui| {
                        if ui.add_enabled(!text.trim().is_empty(), egui::Button::new(if opening { "Open" } else { "Save" })).clicked() {
                            let path = PathBuf::from(text.trim());
                            match kind { PathKind::Open(before) => {
                                if before.matches(&self.project_baseline()) { action = Some(Action::Open(path)); }
                                else { recheck = Some(Action::Open(path)); }
                            }, PathKind::Save(kind, after) => {
                                self.begin_project_save(*kind, after.clone(), path, *replace);
                            } }
                            retain = false;
                        }
                        if ui.button("Cancel").clicked() { retain = false; }
                    });
                }
            }
        });
        if response.should_close() {
            retain = false;
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
