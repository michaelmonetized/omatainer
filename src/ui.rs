use crate::engine::fx::FxId;
use crate::engine::{Command, Engine, Snapshot, SamplerInstrument, DECKS, SCENES, TRACKS};
use crate::engine::media_source::{BuiltinStem, LibSource, Selection};
use crate::theme::Theme;
use eframe::egui::{
    self, Align, Color32, FontId, Key, PointerButton, Pos2, Rect, RichText, Sense, Stroke, Ui, Vec2,
};
use std::cell::Cell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use crate::engine::media_load::Loader;
use std::time::{Instant, SystemTime};
mod library_scan;
mod library_metadata;
mod library_analysis;
mod library_tags;
mod library_annotations;
mod library_crates;
mod library_store;
pub(crate) mod bpm;
use bpm::{Bpm, Origin};
use crate::engine::media_source::FileFingerprint;
mod fx_controls;
mod library_view;
mod key_hints;
mod clip_gain;
use clip_gain::ClipGainEdit;
use library_view::{LibraryView, Cells};
mod load_status;
mod play_history;
mod session_history;
mod session_editor;
mod cue_editor;
mod grid_editor;
mod piano_roll;
mod midi_files;
mod timing;
mod dependencies;
mod portability;
mod midi_routing;
mod play_time;
mod project;
mod templates;
mod project_import;
mod project_versions;
pub(crate) use templates::startup_session;
mod undo;
mod audio_status;
mod diagnostics;
mod support;
pub(crate) mod deck_time;
use deck_time::{DeckTimeSettings, Readout, TimeMode};
#[cfg(test)]
mod deck_time_tests;
mod licenses;
mod accessibility;
mod preferences;
mod automation;
mod music_provider;
mod video;
mod performance;
mod deck_load_lock;
mod background_jobs;
mod audio_settings;
mod audio_routing;
mod recovery_settings;
mod recovery;
mod master_fx_status;
mod keylock_status;
use load_status::{LoadState, Phase};
use crate::engine::load_receipt::{Media, Receipt};
#[cfg(test)]
mod load_status_tests;
mod keyboard;
mod shortcuts;
mod command_palette;
mod touch;
mod workspace;
mod help;
use help::{Control as HelpControl, ContextHelp as _};
pub(crate) fn validate_shortcuts(profile: &crate::preferences::Profile) -> Result<(), String> { shortcuts::validate(profile) }
mod deck_selection;
use library_scan::LibraryScan;

#[cfg(test)]
mod test_support;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod controller_load_tests;
#[cfg(test)]
mod controller_browse_tests;
#[cfg(test)]
mod compose_tests;
#[cfg(test)]
mod library_view_tests;
#[cfg(test)]
mod keyboard_tests;
#[cfg(test)]
mod sampler_identity_tests;
#[cfg(test)]
mod duration_tests;
#[cfg(test)]
mod midi_connection_tests;
#[cfg(test)]
mod sampler_pad_tests;
mod sampler_editor;
#[cfg(test)]
mod theme_reload_tests;
mod theme_requests;
#[cfg(test)]
mod font_selection_tests;

pub struct App {
    support: support::Panel,
    recovery: recovery::Recovery,
    session_history: session_history::Panel,
    settings: preferences::Settings,
    automation_panel: automation::Panel,
    music_provider: music_provider::Panel,
    video: video::Panel,
    automation_network: crate::automation::osc::Manager,
    performance_panel: performance::Panel,
    deck_load_panel: deck_load_lock::Panel,
    background_jobs: background_jobs::Panel,
    audio_settings: audio_settings::Panel,
    diagnostics: diagnostics::Diagnostics,
    licenses: licenses::Licenses,
    engine: Engine,
    project: project::Projects,
    templates: templates::Templates,
    project_import: project_import::Panel,
    project_versions: project_versions::Panel,
    undo_history: undo::History,
    theme: Theme,
    deck_selection: deck_selection::Selection,

    theme_reload: Option<crate::theme::reload::Loader>,
    theme_requests: Option<crate::theme::requests::Endpoint>,
    theme_request: Option<theme_requests::Pending>,
    theme_fonts: Option<Arc<egui::FontDefinitions>>,
    library: Arc<Vec<LibItem>>,
    library_view: LibraryView,
    library_scan: LibraryScan,
    library_metadata: library_metadata::Metadata,
    library_analysis: library_analysis::Panel,
    library_tags: library_tags::Panel,
    library_annotations: library_annotations::Panel,
    library_crates: library_crates::Crates,
    library_import_open: bool,
    library_initialized: bool,
    library_close: library_store::Close,
    library_import_path: String,
    library_media_paths: String,
    library_media_revision: u64,
    // History can change while a worker holds the immutable crate baseline.
    // Keep those small edits separate from the full library allocation.
    last_played: play_history::History,
    playback_watches: Vec<play_history::Watch>,
    cue_editor: cue_editor::Cues,
    grid_editor: Option<grid_editor::Editor>,
    session_editor: session_editor::Editor,
    audio_routing: audio_routing::Panel,
    piano_roll: piano_roll::Editor,
    midi_files: midi_files::Editor,
    timing: timing::Editor,
    dependencies: dependencies::Dependencies,
    portability: portability::Portability,
    sampler_editor: sampler_editor::Editor,
    published_selection: Option<Arc<Selection>>,
    published_indices: std::sync::Weak<Vec<usize>>,
    lib_filter: String,
    lib_sel: usize,
    keys_open: bool,
    command_palette: command_palette::Palette,
    touch_input: touch::Input,
    workspace: workspace::State,
    help: help::Help,
    midi_open: bool,
    status: String,
    loads: [Option<LoadState>; DECKS],
    submission_error: Cell<Option<crate::engine::SubmissionError>>,
    seen_submission_failures: u64,
    loader: Option<Loader>,
    snap: Snapshot,
    last_play_idx: usize,
    pad_held: [bool; 16],
    pad_inputs: [u8; 16],
    shortcut_focus: keyboard::ShortcutFocus,
    clip_gain_edit: Option<ClipGainEdit>,
    deck_time: [DeckTimeSettings; DECKS],
}

#[derive(Clone)]
struct LibItem {
    title: String,
    artist: String,
    bpm: Bpm,
    fingerprint: Option<FileFingerprint>,
    key: String,
    length: Option<f64>,
    last_play: Option<SystemTime>,
    source: LibSource,
}

/// Keep input helpers separate across native windows while retaining root identities.
/// Takes a viewport and helper name; returns the stable context-data key for that window.
fn viewport_key(viewport:egui::ViewportId,name:&str)->egui::Id {
    if viewport==egui::ViewportId::ROOT {egui::Id::new(name)} else {egui::Id::new((viewport,name))}
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, engine: Engine) -> Self {
        let theme = Theme::default();
        cc.egui_ctx.set_fonts(crate::theme::reload::fallback_fonts());
        theme.apply(&cc.egui_ctx);
        let loader = Loader::start_with_performance(engine.cmd.performance().clone());
        let failure = loader.as_ref().err().map(|error| format!("load failed: decoder unavailable: {error}"));
        let mut app = Self::with_loader(engine, theme, loader.ok());
        if let Some(failure) = failure { app.status = failure; }
        if !app.engine.safe_mode() {
        match crate::theme::reload::Loader::start_with_performance(app.theme.clone(), app.engine.cmd.performance().clone()) {
            Ok(loader) => app.theme_reload = Some(loader),
            Err(error) => eprintln!("omatainer: theme reload worker unavailable: {error}"),
        }
        app.start_library_store(crate::library::default_path());
        app.library_scan.enable_watching();
        }
        app.start_default_recovery();
        app.start_default_session_history();
        app

    }

    fn with_loader(
        engine: Engine,
        theme: Theme,
        loader: Option<Loader>,
    ) -> Self {
        let project = if engine.safe_mode() { project::Projects::with_recent(engine.project.clone(),engine.sr(),None) }
            else { project::Projects::new(engine.project.clone(), engine.sr()) };
        let snap = engine.snapshot();
        let playback_watches = play_history::initial_watches(&engine);
        let theme_requests = engine.cmd.theme_requests().attach();
        let mut app = Self {
            support: support::Panel::default(),
            recovery: recovery::Recovery::default(),
            session_history: session_history::Panel::default(),
            audio_settings: audio_settings::Panel::new(engine.audio_handle()),
            audio_routing: audio_routing::Panel::default(),
            settings: preferences::Settings::default(),
            automation_panel: automation::Panel::default(),
            music_provider: music_provider::Panel::default(),
            video: video::Panel::default(),
            automation_network: crate::automation::osc::Manager::new(engine.cmd.clone(),engine.snap.clone()),
            performance_panel: performance::Panel::default(),
            deck_load_panel: deck_load_lock::Panel::default(),
            background_jobs: background_jobs::Panel::default(),
            diagnostics: diagnostics::Diagnostics::default(),
            licenses: licenses::Licenses::default(),
            engine,
            project,
            templates: templates::Templates::default(),
            project_import: project_import::Panel::default(),
            project_versions: project_versions::Panel::default(),
            undo_history: undo::History::default(),
            theme,
            deck_selection: deck_selection::Selection::new(snap.selected_deck_request),

            theme_reload: None,
            theme_requests,
            theme_request: None,
            theme_fonts: None,
            library: Arc::new(builtin_crate_items()),
            library_view: LibraryView::default(),
            library_scan: LibraryScan::default(),
            library_metadata: library_metadata::Metadata::default(),
            library_analysis: library_analysis::Panel::default(),
            library_tags: library_tags::Panel::default(),
            library_annotations: library_annotations::Panel::default(),
            library_crates: library_crates::Crates::default(),
            library_import_open: false,
            library_initialized: false,
            library_close: library_store::Close::default(),
            library_import_path: String::new(),
            library_media_paths: String::new(),
            library_media_revision: 0,
            last_played: play_history::History::default(),
            playback_watches,
            cue_editor: cue_editor::Cues::default(),
            grid_editor: None,
            session_editor: session_editor::Editor::default(),
            piano_roll: piano_roll::Editor::default(),
            midi_files: midi_files::Editor::default(),
            timing: timing::Editor::default(),
            dependencies: dependencies::Dependencies::default(),
            portability: portability::Portability::default(),
            sampler_editor: sampler_editor::Editor::default(),
            published_selection: None,
            published_indices: std::sync::Weak::new(),
            lib_filter: String::new(),
            lib_sel: 0,
            keys_open: false,
            command_palette: command_palette::Palette::default(),
            touch_input: touch::Input::default(),
            workspace: workspace::State::default(),
            help: help::Help::default(),
            midi_open: false,
            status: "Q quant · pads compose · ctrl-gain = fx".into(),
            loads: std::array::from_fn(|_| None),
            submission_error: Cell::new(None),
            seen_submission_failures: 0,
            loader,
            snap,
            last_play_idx: 0,
            pad_held: [false; 16],
            pad_inputs: [0; 16],
            shortcut_focus: keyboard::ShortcutFocus::default(),
            clip_gain_edit: None,
            deck_time: [DeckTimeSettings::default(); DECKS],
        };
        app.library_scan.set_performance(app.engine.cmd.performance().clone());
        app.library_metadata.set_performance(app.engine.cmd.performance().clone());
        app.publish_library_selection();
        app.initialize_project_baseline();
        app
    }

    fn scan_library(&mut self) {
        if self.engine.cmd.performance().protected() || self.project.committing() {
            self.settings.rescan=true;self.status="Library scan waits for Studio and the current project operation".into();return;
        }
        if !self.library_metadata.ready() || self.library_metadata.active() {
            self.settings.rescan=true;self.status="Waiting for the current catalog before scanning".into();return;
        }
        self.library_scan.start_watched(self.settings.profile().library_roots.clone(),self.library.clone(),
            self.settings.applied.active.clone(),self.library_metadata.catalog.clone());
    }

    fn poll_library_scan(&mut self) {
        if let Some(publication) = self.library_scan.poll() {
            self.library_metadata.stage_scan(publication, &self.library);
        }
        self.poll_library_metadata();
        if !self.library_scan.active() && !self.library_metadata.active() && self.library_metadata.ready()
            && !self.project.committing() && !self.engine.cmd.performance().protected() && self.library_scan.take_watch_hint() {
            self.scan_library();
        }
    }

    fn send(&self, c: Command) {
        self.submit(c);
    }

    fn submit(&self, c: Command) -> bool {
        if !matches!(c, Command::PerformanceMode(_) | Command::SafetyStop(_) | Command::RecoverPerformance) && !self.performance_allows(&c) { return false; }
        // Replacing or unloading a deck invalidates even a completion that has
        // already entered the audio command queue. The renderer rechecks it.
        if let Command::DeckUnload { deck } | Command::LoadBuiltin { deck, .. } | Command::DeckAudio { deck, .. } = &c {
            if let Some(load) = self.loads.get(*deck as usize).and_then(Option::as_ref) {
                if let Some(receipt) = &load.receipt { receipt.cancel_pending(); }
            }
            if self.loader.as_ref().is_some_and(|loader| loader.invalidate(*deck).is_err()) {
                return false;
            }
        }
        // A painted control owns the snapshot's object identity. Reading the
        // latest producer registry here could silently target a replacement.
        let c = if let Some(layout) = &self.snap.session {
            match crate::engine::session::Scoped::qualify_layout(c, layout) {
                Ok(command) => command,
                Err(_) => {self.submission_error.set(Some(crate::engine::SubmissionError::InvalidTarget));return false;}
            }
        } else {c};
        match self.engine.send(self.undo_history.wrap(c)) {
            Ok(_) => true,
            Err(error) => {
                self.submission_error.set(Some(error));
                false
            }
        }
    }

    fn load_sel(&mut self, deck: u8) {
        let picked = self.selected_library_item().map(|item| Selection {
            title: item.title.clone(), source: item.source.clone(),
        });
        self.load_source(deck, picked.as_ref());
    }

    fn load_source(&mut self, deck: u8, picked: Option<&Selection>) {
        self.load_source_approved(deck, picked, None);
    }
    /// Prepare one captured library choice.
    /// Takes its target, selected source and optional review; preserves the current deck until renderer application.
    fn load_source_approved(&mut self, deck: u8, picked: Option<&Selection>, approval: Option<crate::engine::performance::DeckApproval>) {
        if !self.deck_load_allows(deck, approval.as_ref()) { return; }
        self.project.local_edits = self.project.local_edits.wrapping_add(1);
        if deck as usize >= DECKS {
            self.status = "load failed: invalid deck".into();
            return;
        }
        if let Some(picked) = picked {
            match &picked.source {
                LibSource::Builtin(stem) => {
                    self.supersede_load(deck);
                    let mut state = LoadState::new(Some(picked.clone()), Phase::Queued);
                    if let Some(error) = self.loader.as_ref().and_then(|loader| loader.invalidate(deck).err()) {
                        state.phase = Phase::Failed(error);
                    } else {
                        let mut receipt = self.library_receipt(&picked.source, None).with_deck_generation(self.engine.cmd.performance().deck_load_word(deck as usize));
                        if let Some(approval) = approval { receipt = receipt.with_deck_approval(approval); }
                        if !self.submit(Command::DeckLoadRequested { deck, media: Media::Builtin(stem.index()), receipt: receipt.clone() }) {
                            state.phase = Phase::Failed("Load was not accepted; media was not loaded".into());
                        } else {
                            self.watch_playback(picked.source.clone(), None, receipt.clone());
                            state.receipt = Some(receipt);
                        }
                    }
                    self.set_load_state(deck, state);
                }
                LibSource::File(_) | LibSource::Removable { .. } => self.load_reference_approved(deck,picked.source.clone(),&picked.title,approval),
                LibSource::Provider { .. } => {
                    self.supersede_load(deck);
                    if let Some(loader) = &self.loader { let _ = loader.invalidate(deck); }
                    self.set_load_state(deck, LoadState::new(Some(picked.clone()), Phase::Failed(
                        "This provider library namespace is unavailable locally".into())));
                }
            }
        } else {
            self.supersede_load(deck);
            if let Some(loader) = &self.loader { let _ = loader.invalidate(deck); }
            self.set_load_state(deck, LoadState::new(None, Phase::Failed("no library item selected".into())));
        }
    }

    fn supersede_load(&self, deck: u8) {
        if let Some(load) = self.loads.get(deck as usize).and_then(Option::as_ref) {
            if let Some(receipt) = &load.receipt { receipt.cancel_pending(); }
        }
    }

    fn load_file(&mut self, deck: u8, path: PathBuf, name: &str) {
        self.load_reference(deck,LibSource::File(path),name);
    }
    fn load_reference(&mut self, deck:u8,source:LibSource,name:&str) {
        self.load_reference_approved(deck, source, name, None);
    }
    /// Start one source-bound asynchronous decode.
    /// Takes its deck, source, displayed name and optional review; retains approval with the pending job and leaves loaded audio intact.
    fn load_reference_approved(&mut self, deck:u8,source:LibSource,name:&str,approval:Option<crate::engine::performance::DeckApproval>) {
        if !self.deck_load_allows(deck, approval.as_ref()) { return; }
        self.project.local_edits = self.project.local_edits.wrapping_add(1);
        if deck as usize >= DECKS { self.status = "load failed: invalid deck".into(); return; }
        self.supersede_load(deck);
        let selection = Selection { title: name.into(), source:source.clone() };
        let mut state = LoadState::new(Some(selection), Phase::Loading);
        state.approval = approval;
        state.deck_generation=Some(self.engine.cmd.performance().deck_load_word(deck as usize));
        match self.loader.as_ref().ok_or_else(|| "decoder is unavailable".to_string())
            .and_then(|loader| loader.request_source(deck, source)) {
            Ok(token) => state.token = Some(token),
            Err(error) => state.phase = Phase::Failed(error),
        }
        self.set_load_state(deck, state);
    }

    fn poll_ui_requests(&mut self) {
        use crate::engine::ui_requests::Request;
        for request in self.engine.ui_requests.take_requests().into_iter().flatten() {
            match request {
                Request::Load(request) => self.load_source(request.deck, Some(&request.selection)),
                Request::Browse(request) => {
                    if request.epoch != self.engine.ui_requests.epoch() { continue; }
                    self.refresh_library_view();
                    let matches = |&i: &usize| self.library[i].source == request.selection.source;
                    let Some(index) = self.library_view.indices.get(request.index).filter(|i| matches(i))
                        .map(|_| request.index).or_else(|| self.library_view.indices.iter().position(matches))
                        else { continue };
                    self.lib_sel = index;
                    let top = index as f32 * self.library_view.stride.max(18.0);
                    let bottom = top + self.library_view.stride.max(18.0);
                    if top < self.library_view.offset {
                        self.library_view.pending_offset = Some(top);
                    } else if bottom > self.library_view.offset + self.library_view.height {
                        self.library_view.pending_offset = Some((bottom - self.library_view.height).max(0.0));
                    }
                    // This is an acknowledgement, not a new manual selection.
                    // Do not reset the worker cursor if later queued browse
                    // events have already advanced it further this same burst.
                    self.published_selection = Some(request.selection);
                    self.refresh_library_view();
                }
            }
        }
    }

    fn publish_library_selection(&mut self) {
        self.refresh_library_view();
        let selected = self.library_view.indices.get(self.lib_sel).map(|&i| &self.library[i]);
        if self.published_indices.as_ptr() == Arc::as_ptr(&self.library_view.indices)
            && self.published_selection.as_ref().map(|item| (&item.source, &item.title))
                == selected.map(|item| (&item.source, &item.title)) {
            return;
        }
        let selection = selected.map(|item| Arc::new(Selection {
            source: item.source.clone(), title: item.title.clone(),
        }));
        self.engine.ui_requests.publish_view(Arc::new(library_view::PublishedView {
            library: Arc::downgrade(&self.library), indices: Arc::downgrade(&self.library_view.indices),
        }), self.lib_sel);
        self.published_indices = Arc::downgrade(&self.library_view.indices);
        self.published_selection = selection;
    }

    fn poll_loads(&mut self) {
        self.poll_load_receipts();
        let Some(loader) = &self.loader else { return };
        let ready = loader.take_ready();
        for completion in ready.into_iter().flatten() {
            if !completion.token.is_current() { continue; }
            let deck = completion.token.deck;
            let Some(mut state) = self.loads[deck as usize].take() else { continue };
            if state.token.as_ref().map(|token| token.id) != Some(completion.token.id) {
                self.loads[deck as usize] = Some(state);
                continue;
            }
            match completion.result {
                Ok(mut report) => {
                    let analysis = Bpm::new(report.sample.bpm, Origin::Heuristic);
                    if let Some(selection) = &mut state.selection {
                        let saved = self.library_metadata.catalog.version(&selection.source, completion.fingerprint)
                            .and_then(|version| version.tags.as_ref()).and_then(|tags| tags.overrides.title.as_ref());
                        let embedded = completion.tags.as_ref().and_then(|tags| tags.as_ref().ok())
                            .and_then(|tags| tags.fields.title.as_ref()).map(|field| &field.value);
                        if let Some(title) = saved.or(embedded) {
                            selection.title.clone_from(title);
                            report.sample.name.clone_from(title);
                        }
                    }
                    let source = state.selection.as_ref().map(|selection| &selection.source);
                    let mut bpm = self.library.iter().find(|item| Some(&item.source) == source
                        && item.fingerprint.is_some() && item.fingerprint == completion.fingerprint)
                        .map(|item| item.bpm.reconcile(analysis)).unwrap_or(analysis);
                    if bpm.origin != Origin::User {
                        if let Some(value) = completion.tags.as_ref().and_then(|tags| tags.as_ref().ok())
                            .and_then(|tags| tags.fields.bpm.as_ref()).and_then(|field| field.value.trim().parse::<f32>().ok()) {
                            let embedded = Bpm::new(value, Origin::EmbeddedTag);
                            if embedded.value().is_some() { bpm = embedded; }
                        }
                    }
                    report.sample.bpm = bpm.value().unwrap_or(0.0);
                    state.bpm = Some(bpm);
                    state.metadata = completion.fingerprint.zip(source.cloned()).map(|(fingerprint, source)|
                        library_metadata::Patch { source, fingerprint, bpm: analysis, duration: decoded_duration(&report.sample), tags: completion.tags.clone() });
                    state.warning = report.diagnostics.warning();
                    let history_source = source.cloned();
                    let captured_metadata = history_source.as_ref().map(|source| {
                        let mut metadata = self.capture_metadata(source, completion.fingerprint);
                        metadata.bpm = bpm;
                        metadata.duration = decoded_duration(&report.sample);
                        if let Some(selection) = &state.selection { metadata.title = selection.title.clone(); }
                        metadata
                    });
                    let measured=history_source.as_ref().zip(completion.fingerprint).zip(completion.content_hash);
                    let mut receipt=if let Some(((source,fp),hash))=measured {
                        Receipt::with_preparation(self.library_metadata.catalog.preparation_for_content(source,fp,hash))
                    } else {history_source.as_ref().map(|source|self.library_receipt(source,completion.fingerprint)).unwrap_or_else(Receipt::new)};
                    if let Some(approval) = state.approval.take() { receipt = receipt.with_deck_approval(approval); }
                    if let Some(generation)=state.deck_generation {receipt=receipt.with_deck_generation(generation);}
                    if let Some(((source,fingerprint),hash))=measured {
                        if let Some(track)=self.library_metadata.catalog.track(source) {
                            let proof=crate::sampler_bank::SourceRef {track:track.id.clone(),source:source.clone(),fingerprint,content_hash:Some(hash)};
                            if let Some(metadata)=captured_metadata.as_ref() {
                                self.library_metadata.capture(library_store::Capture {source:source.clone(),fingerprint:Some(fingerprint),metadata:metadata.clone(),preparation:None,played:None});
                            }
                            if let Err(error)=self.library_metadata.qualify_sampler(proof) {self.status=format!("Media verified; catalog content proof needs attention: {error}");}
                        }
                    }
                    state.phase = if self.submit(Command::DeckLoadRequested {
                        deck, media: Media::Decoded { token: completion.token, audio: Arc::new(report.sample) },
                        receipt: receipt.clone(),
                    }) {
                        if let Some(source) = history_source {
                            self.watch_playback(source, completion.fingerprint, receipt.clone());
                            if let Some(metadata) = captured_metadata { self.watch_metadata(&receipt, metadata); }
                        }
                        state.receipt = Some(receipt);
                        Phase::Queued
                    } else { Phase::Failed("Load was not accepted; media was not loaded".into()) };
                }
                Err(error) => state.phase = Phase::Failed(error.to_string()),
            }
            self.set_load_state(deck, state);
        }
    }

    fn set_pad_input(&mut self, pad: usize, source: u8, on: bool) {
        self.set_pad_input_pressure(pad,source,on,None);
    }

    /// Share an admitted pad gate while preserving each local input owner's release.
    /// Takes pad, source bit, gate and optional pressure; returns no value and applies pressure only at attack.
    fn set_pad_input_pressure(&mut self, pad: usize, source: u8, on: bool, pressure: Option<f32>) {
        let before = self.pad_inputs[pad];
        let after = if on { before | source } else { before & !source };
        if before == after { return; }
        if before == 0 && after != 0 {
            let command=pressure.map_or(Command::SamplerPad {pad:pad as u8,on:true},|pressure|Command::SamplerPadPressure {pad:pad as u8,pressure});
            if !self.submit(command) {return;}
        }
        self.pad_inputs[pad] = after;
        self.pad_held[pad] = after != 0;
        if before != 0 && after == 0 { self.send(Command::SamplerPad { pad: pad as u8, on: false }); }
    }

    fn pad_gate(&mut self, ui: &Ui, p: usize, enabled: bool, r: &egui::Response) {
        if p >= self.pad_held.len() { return; }
        let mouse=touch::register(ui,r,touch::Target::Pad(p as u8),r.rect,enabled);
        let window_focus=r.ctx.input(|input| input.focused);
        if mouse.released || !mouse.down || !window_focus || !r.enabled() { self.set_pad_input(p, 1, false); }
        let pressed_here = mouse.pressed && window_focus && r.enabled() && mouse.starts_here(r);
        if enabled && pressed_here {
            self.set_pad_input(p, 1, true);
            if !mouse.down { self.set_pad_input(p, 1, false); }
        }
        // Pointer, keyboard and AT holds share one admitted pad gate, with
        // distinct local ownership so releasing one input cannot cut another.
        let focused = r.has_focus() && r.enabled() && window_focus;
        if !focused { self.set_pad_input(p, 2, false); self.set_pad_input(p, 8, false); }
        if !r.enabled() || !window_focus { self.set_pad_input(p, 4, false); }
        let keys = r.ctx.input(|input| input.events.iter().filter_map(|event| match event {
            egui::Event::Key { key: key @ (Key::Space | Key::Enter), pressed, repeat: false, modifiers, .. }
                if !pressed || !modifiers.ctrl && !modifiers.alt && !modifiers.command => Some((*key, *pressed)),
            _ => None,
        }).collect::<Vec<_>>());
        for (key, pressed) in keys {
            let source = if key == Key::Space { 2 } else { 8 };
            if !pressed { self.set_pad_input(p, source, false); }
            else if focused && enabled { self.set_pad_input(p, source, true); }
        }
        let action = accessibility::actions(ui, r, &["Press pad", "Release pad"]);
        let clicked = r.ctx.input(|input| input.num_accesskit_action_requests(r.id, egui::accesskit::Action::Click) > 0);
        if enabled && r.enabled() && window_focus && (action == Some(0) || clicked && self.pad_inputs[p] & 4 == 0) {
            self.set_pad_input(p, 4, true);
        } else if action == Some(1) || clicked {
            self.set_pad_input(p, 4, false);
        }
        let identity = crate::engine::sampler_pad::PadIdentity::new(p as u8);
        let name = if self.snap.sampler_inst.synth().is_some() {
            format!("Pad {}: {} MIDI note {}", p + 1, identity.piano_label(), identity.midi_note(self.snap.sampler_oct))
        } else { format!("Sample pad {}", p + 1) };
        if self.pad_held[p] {
            active_mark(ui.painter(), r.rect, self.theme.fg);
            ui.painter().rect_stroke(r.rect.shrink(3.0), 2.0, st(2.0,self.theme.fg), egui::StrokeKind::Inside);
        }
        accessibility::button(ui, r, &name, Some(self.pad_held[p]));
        r.ctx.accesskit_node_builder(r.id, |node| {
            node.set_description(if enabled { "Hold Space or Enter to play; release or move focus to stop. Assistive click toggles a hold; Shift+F10 offers Press and Release." } else { "No piano accidental at this pad; Release pad still releases an existing hold." });
        });
    }

    #[cfg(test)]
    fn filtered(&self) -> Vec<&LibItem> {
        let q = crate::localization::search_key(&self.lib_filter);
        self.library
            .iter()
            .filter(|i| {
                q.is_empty()
                    || crate::localization::search_key(&i.title).contains(&q)
                    || crate::localization::search_key(&i.artist).contains(&q)
            })
            .collect()
    }
}

fn builtin_crate_items() -> Vec<LibItem> {
    vec![
        LibItem {
            title: "Drums (session)".into(),
            artist: "omatainer".into(),
            bpm: Bpm::new(124.0, Origin::Builtin),
            fingerprint: None,
            key: "C".into(),
            length: Some(16.0 * 60.0 / 124.0),
            last_play: None,
            source: LibSource::Builtin(BuiltinStem::Drums),
        },
        LibItem {
            title: "Harmony (session)".into(),
            artist: "omatainer".into(),
            bpm: Bpm::new(124.0, Origin::Builtin),
            fingerprint: None,
            key: "C".into(),
            length: Some(16.0 * 60.0 / 124.0),
            last_play: None,
            source: LibSource::Builtin(BuiltinStem::Harmony),
        },
    ]
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.update_frame(ctx);
    }
}

impl App {
    fn poll_theme(&mut self, ctx: &egui::Context) {
        if self.poll_theme_requests(ctx) { return; }
        let Some((update, _commit)) = self.theme_reload.as_ref()
            .and_then(|loader| loader.poll_candidate())
            .and_then(|candidate| candidate.claim().ok()) else { return };
        self.settings.theme_update = Some(update);
        if self.settings.profile().appearance.follow_theme { self.apply_appearance(ctx); }
        ctx.request_repaint();
    }

    fn update_frame(&mut self, ctx: &egui::Context) {
        let _locale = crate::localization::scope(self.settings.profile().appearance.locale);
        let ui_started = Instant::now();
        #[cfg(test)]
        std::thread::sleep(self.diagnostics.ui_delay);
        self.shortcut_focus.begin_frame(ctx);
        self.touch_input.begin(ctx);
        accessibility::begin_frame(ctx);
        self.undo_history.begin_frame(ctx, &self.engine.undo);
        if !self.project.committing() {
            self.poll_ui_requests();
            self.poll_library_scan();
            ctx.request_repaint_after(std::time::Duration::from_millis(500));
        } else { keyboard::block_for_dialog(ctx); }
        let submissions = self.engine.cmd.stats();
        if submissions.rejected > self.seen_submission_failures {
            self.seen_submission_failures = submissions.rejected;
            self.submission_error.set(submissions.last_error);
        }
        self.poll_recovery();
        self.poll_preferences(ctx);
        self.poll_audio_settings(ctx);
        self.poll_theme(ctx);
        if !self.project.committing() { self.poll_loads(); }
        self.snap = self.engine.snapshot();
        self.poll_music_provider();
        self.poll_video(ctx);
        self.confirm_project_snapshot();
        self.poll_undo();
        self.poll_piano_roll();
        self.poll_midi_files();
        self.timing.poll(&self.engine);
        self.poll_dependencies();
        self.poll_portability();
        self.poll_templates();
        self.project_import.poll();
        self.project_versions.poll();
        self.poll_sampler_editor();
        self.poll_library_analysis();
        self.poll_library_tags();
        self.poll_named_crates();
        self.poll_session_history();
        let animating = self.snap.playing || self.snap.decks.iter().any(|d| d.playing);
        if let Some(p) = ctx.input(|i| {
            (!self.project.committing() && self.project.dialog_is_closed()).then(|| i.raw.dropped_files.iter().find_map(|f| f.path.clone())).flatten()
        }) {
            let x = ctx.input(|i| i.pointer.latest_pos().map(|p| p.x)).unwrap_or(0.0);
            let deck = if x > ctx.screen_rect().center().x { 1u8 } else { 0 };
            let name = p.file_stem().and_then(|name| name.to_str()).unwrap_or("track").to_string();
            self.load_file(deck, p, &name);
        }

        self.support_ui(ctx);
        // Collapsing four tall rows preserves the performance viewport when text is large.
        let compact = ctx.screen_rect().height() < self.theme.text_size(12.0) * 16.0 + 160.0
            || ctx.screen_rect().width() < self.theme.text_size(12.0) * 26.0;
        if compact {
            egui::TopBottomPanel::top("compact-controls").show(ctx, |ui| {
                ui.horizontal_wrapped(|ui| {
                    self.performance_controls(ctx, Some(ui));
                    self.project_controls(ctx, Some(ui));
                    self.setup_controls(ctx, Some(ui));
                    self.master_controls(ctx, Some(ui));
                });
            });
        } else {
            self.performance_ui(ctx);
            self.project_toolbar(ctx);
        }
        self.library_close_ui(ctx);
        self.library_store_ui(ctx);
        self.cue_editor_ui(ctx);
        self.grid_editor_ui(ctx);
        self.session_editor_ui(ctx);
        self.audio_routing_ui(ctx);
        self.piano_roll_ui(ctx);
        self.midi_files_ui(ctx);
        self.timing_ui(ctx);
        self.dependencies_ui(ctx);
        self.portability_ui(ctx);
        self.templates_ui(ctx);
        self.project_import_ui(ctx);
        self.project_versions_ui(ctx);
        self.sampler_editor_ui(ctx);
        self.library_analysis_ui(ctx);
        self.library_tags_ui(ctx);
        self.library_annotations_ui(ctx);
        self.named_crates_ui(ctx);
        self.session_history_ui(ctx);
        self.load_status(ctx);
        if !compact { self.audio_status(ctx); self.master_fx_status(ctx); }
        self.workspace_ui(ctx);
        self.background_jobs_ui(ctx);

        if let Some(error) = self.submission_error.get() {
            keyboard::block_for_dialog(ctx);
            egui::Window::new(tr!("Action was not accepted")).id(egui::Id::new("Action was not accepted"))
                .collapsible(false)
                .resizable(false)
                .show(ctx, |ui| {
                    ui.label(error.to_string());
                    if ui.button(tr!("Dismiss")).help(ui, HelpControl::AdmissionDismiss).clicked() {
                        self.submission_error.set(None);
                    }
                });
        }
        self.help_panel(ctx);
        if self.midi_open {
            egui::Window::new(tr!("midi")).id(egui::Id::new("midi")).vscroll(true).max_height(self.theme.window_height(ctx)).show(ctx, |ui| {
                let busy = self.engine.midi.connections_busy();
                if ui.add_enabled(!busy && self.engine.midi.connections_available(), egui::Button::new(tr!("Retry / rescan MIDI"))).help(ui, HelpControl::MidiRetry).clicked() {
                    match self.engine.midi.retry_connections() {
                        crate::engine::midi::Retry::Queued | crate::engine::midi::Retry::AlreadyRunning => {
                            if self.engine.midi.routing_status().is_some(){self.settings.routing_pending=true;}
                        }
                        crate::engine::midi::Retry::Performance(error) => self.submission_error.set(Some(crate::engine::SubmissionError::Performance(error))),
                        crate::engine::midi::Retry::Unavailable => {},
                    }
                }
                if busy {
                    ui.label(tr!("Checking MIDI connections…"));
                    ctx.request_repaint_after(std::time::Duration::from_millis(50));
                } else if !self.engine.midi.connections_available() {
                    ui.label(tr!("MIDI connection worker unavailable; restart to retry."));
                }
                ui.label(tr!("Keyboard and mouse remain available. Retry checks current ports."));
                let input = self.engine.midi.input_stats();
                ui.label({ let __omatainer_args = (&(input.received),&(input.queued),&(input.dispatched),); crate::localization::format("Input: {} received · {} queued · {} handled", &[format!("{}", __omatainer_args.0), format!("{}", __omatainer_args.1), format!("{}", __omatainer_args.2)]) });
                ui.label({ let __omatainer_args = (&(input.coalesced),&(input.dropped),&(input.resets),); crate::localization::format("Overload: {} coalesced · {} discarded · {} source resets", &[format!("{}", __omatainer_args.0), format!("{}", __omatainer_args.1), format!("{}", __omatainer_args.2)]) });
                ui.label({ let __omatainer_args = (&(input.oversized),&(input.disconnected),); crate::localization::format("{} oversized · {} disconnected", &[format!("{}", __omatainer_args.0), format!("{}", __omatainer_args.1)]) });
                for d in &self.snap.midi {
                    ui.label(d);
                }
                self.midi_routing_status_ui(ui,ctx);
            });
        }
        self.automation_ui(ctx);
        self.music_provider_ui(ctx);
        self.video_ui(ctx);
        self.preferences_ui(ctx);
        self.audio_settings_ui(ctx);
        self.recovery_ui(ctx);
        self.diagnostics_panel(ctx);
        self.licenses_panel(ctx);
        self.clip_gain_editor(ctx);
        self.undo_panel(ctx);
        accessibility::numeric_editor(ctx);
        self.command_palette_ui(ctx);
        self.touch_input_ui(ctx);
        // Text fields and dialogs get this frame's keys before global actions.
        self.handle_keys(ctx);
        self.finish_touch_input(ctx);
        self.undo_history.end_frame(ctx);
        accessibility::finish_frame(ctx);
        // Resolve terminal project outcomes after this frame's Cancel/input.
        self.poll_projects(ctx);
        self.recovery_close_ui(ctx);
        self.session_history_close_ui(ctx);
        self.sync_recovery();
        if animating {
            ctx.request_repaint();
        } else {
            ctx.request_repaint_after(std::time::Duration::from_millis(80));
        }
        self.publish_library_selection();
        self.guard_project_import_view();
        self.diagnostics.ui_update_ns = Some(ui_started.elapsed().as_nanos().min(u64::MAX as u128) as u64);
        self.observe_support();
    }
}

impl App {
    fn handle_keys(&mut self, ctx: &egui::Context) {
        let viewport=ctx.viewport_id();
        // A bound function-key Help action is safe in text/dialog contexts.
        // Letter and punctuation bindings keep the ordinary typing protection.
        let help = ctx.input_mut(|input| {
            let modifiers = input.events.iter().find_map(|event| match event {
                egui::Event::Key { key: Key::F1, pressed: true, repeat, modifiers, .. }
                    if shortcuts::lookup_with(self.settings.profile(), Key::F1, *modifiers, *repeat) == Some(shortcuts::Action::Help) => Some(*modifiers),
                _ => None,
            });
            modifiers.is_some_and(|modifiers| input.consume_key(modifiers, Key::F1))
        });
        if help { self.dispatch_shortcut(shortcuts::Action::Help); }
        if !self.shortcut_focus.globals_allowed(ctx) {
            return;
        }
        ctx.input(|i| {
            for ev in &i.events {
                if let egui::Event::Key { key, pressed: true, repeat, modifiers: mods, .. } = ev {
                    if Some(*key) == command_palette::chord(self.settings.profile()) && mods.ctrl && mods.shift && !mods.alt && !mods.mac_cmd && !repeat {
                        self.command_palette.open(viewport);
                        return;
                    }
                    if *key == Key::Comma && mods.ctrl && !mods.alt && !mods.shift && !repeat { self.settings.open = true; }
                    if let Some(action) = shortcuts::lookup_with(self.settings.profile(), *key, *mods, *repeat) {
                        self.dispatch_shortcut(action);
                    }
                }
            }
        });
        if self.command_palette.open { keyboard::block_for_dialog(ctx); }
    }

    fn scratch_row(&mut self, ui: &mut Ui, t: &Theme) {
        self.deck_selection.begin_pointer_frame();
        ui.spacing_mut().item_spacing = Vec2::splat(4.0);
        let h = ui.available_height();
        let w = ui.available_width();
        let gap = 4.0;
        let fader_h = t.target_size(28.0);
        let (sq, wave_h, side_w, mid_w) = scratch_metrics(w, h, fader_h, gap, t.target_size(22.0).max(if t.font_size <= 18.0 { t.text_size(11.0) * 2.0 + 8.0 } else { 0.0 }));
        ui.with_layout(egui::Layout::left_to_right(Align::Min), |ui| {
            ui.spacing_mut().item_spacing = Vec2::splat(gap);
            ui.allocate_ui(Vec2::new(side_w, h), |ui| {
                ui.spacing_mut().item_spacing = Vec2::splat(gap);
                self.deck_side(ui, t, 0, wave_h, fader_h, sq);
            });
            ui.allocate_ui(Vec2::new(mid_w, h), |ui| {
                ui.with_layout(egui::Layout::top_down(Align::Min), |ui| {
                    ui.spacing_mut().item_spacing = Vec2::splat(gap);
                    let wave_w = ((mid_w - gap) / 2.0).max(56.0);
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing = Vec2::splat(gap);
                        for d in 0..DECKS {
                            let snap = self.snap.decks.get(d).cloned().unwrap_or_default();
                            let wave = ui.scope(|ui| {
                                accessibility::scope(ui, &format!("Deck {}", (b'A' + d as u8) as char), |ui| {
                                    vertical_wave(ui, t, &snap, t.track_color(d), wave_w, wave_h, |frac| {
                                        self.send(Command::DeckSeek { deck: d as u8, frac });
                                    });
                                });
                            });
                            self.select_deck_from_pointer(ui, wave.response.rect, d);
                        }
                    });
                    let mut x = self.snap.xfader;
                    if xfader(ui, t, mid_w, fader_h, &mut x) {
                        self.send(Command::Xfader(x));
                    }
                });
            });
            ui.allocate_ui(Vec2::new(side_w, h), |ui| {
                ui.spacing_mut().item_spacing = Vec2::splat(gap);
                self.deck_side(ui, t, 1, wave_h, fader_h, sq);
            });
        });
    }

    fn deck_side(&mut self, ui: &mut Ui, t: &Theme, d: usize, wave_h: f32, fader_h: f32, sq: f32) {
        let snap = self.snap.decks.get(d).cloned().unwrap_or_default();
        let col = t.track_color(d);
        let h = wave_h + fader_h + 4.0;
        let panel = accessibility::scope(ui, &format!("Deck {}", (b'A' + d as u8) as char), |ui| ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing = Vec2::splat(4.0);
            ui.set_min_height(h);
            if d == 0 {
                self.speed_col(ui, t, d, &snap, h, sq);
                self.cue_eq_col(ui, t, d, &snap, col, wave_h, sq);
                self.platter_col(ui, t, d, &snap, col, wave_h);
                self.btn_stack(ui, t, d, &snap, sq);
            } else {
                self.btn_stack(ui, t, d, &snap, sq);
                self.platter_col(ui, t, d, &snap, col, wave_h);
                self.cue_eq_col(ui, t, d, &snap, col, wave_h, sq);
                self.speed_col(ui, t, d, &snap, h, sq);
            }
        }));
        self.select_deck_from_pointer(ui, panel.response.rect, d);
        if self.snap.selected_deck == d {
            ui.painter().rect_stroke(panel.response.rect.expand(2.0), 3.0,
                st(1.5, t.accent), egui::StrokeKind::Inside);
        }
    }

    fn speed_col(&mut self, ui: &mut Ui, t: &Theme, d: usize, snap: &crate::engine::DeckSnap, h: f32, sq: f32) {
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing = Vec2::splat(4.0);
            ui.set_width(sq);
            ui.set_min_height(h);
            let (mark, accent, status) = keylock_status::presentation(snap, t);
            if sq_btn(ui, t, mark, snap.keylock, accent, sq)
                .help_detail(ui, HelpControl::PitchLock, &status).clicked() {
                self.send(Command::DeckKeylock { deck: d as u8 });
            }
            let fader_h = (h - sq * 2.0 - 8.0).max(t.target_size(48.0)).min((ui.clip_rect().height() - 8.0).max(t.target_size(24.0)));
            let span = [8.0, 16.0, 50.0][snap.pitch_range.min(2) as usize];
            if let Some(v) = fader(ui, t, snap.pitch, span, snap.meter, t.accent, sq, fader_h, d as u8) {
                self.send(Command::DeckPitch { deck: d as u8, value: v });
            }
            let lab = ["8", "16", "50"][snap.pitch_range.min(2) as usize];
            let range = sq_btn(ui, t, lab, false, t.orange, sq);
            accessibility::button(ui, &range, &format!("Pitch range ±{span}%"), None);
            help::annotate(ui, &range, HelpControl::PitchRange);
            if range.clicked() {
                self.send(Command::DeckPitchRange { deck: d as u8 });
            }
        });
    }

    fn cue_eq_col(&mut self, ui: &mut Ui, t: &Theme, d: usize, snap: &crate::engine::DeckSnap, col: Color32, wave_h: f32, sq: f32) {
        let cell = sq;
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing = Vec2::splat(3.0);
            ui.set_width((cell + 4.0) * 4.0 + 9.0);
            ui.set_min_height(wave_h);
            for row in 0..2 {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing = Vec2::splat(3.0);
                    for c in 0..4 {
                        let i = row * 4 + c;
                        let on = snap.hotcues.get(i).copied().unwrap_or(false);
                        let name = snap.cue_styles[i].name.as_str();
                        let text = if name.is_empty() { format!("{}", i + 1) }
                            else { format!("{}\n{}", i + 1, cue_editor::short_name(name, 5)) };
                        let r = sq_btn(ui, t, &text, on, cue_editor::color(t, snap.cue_styles[i], i), cell);
                        let label = cue_editor::label(i, snap.cue_styles[i]);
                        accessibility::button(ui, &r, &label, Some(on));
                        accessibility::status(ui, &r, &cue_editor::description(snap, i));
                        let alternative = accessibility::actions(ui, &r, &["Set or jump to cue", "Delete cue", "Edit cue names and colors"]);
                        help::annotate(ui, &r, HelpControl::HotCue);
                        if alternative == Some(2) { self.open_cue_editor(d); }
                        else if r.clicked() || alternative.is_some() {
                            self.send(Command::DeckHotCue {
                                deck: d as u8,
                                pad: i as u8,
                                del: alternative.map(|action| action == 1).unwrap_or_else(|| ui.input(|i| i.modifiers.shift)),
                            });
                        }
                    }
                });
            }
            ui.horizontal_wrapped(|ui| {
                if ui.small_button(tr!("cues…")).help(ui, HelpControl::CueEditor).clicked() { self.open_cue_editor(d); }
                let grid = ui.small_button(tr!("grid…"));
                accessibility::button(ui, &grid, "Beatgrid editor", None);
                help::annotate(ui, &grid, HelpControl::GridEditor);
                if grid.clicked() { self.open_grid_editor(d); }
            });
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing = Vec2::splat(3.0);
                for (band, lab) in [(0u8, "b"), (1, "m"), (2, "t"), (3, "g")] {
                    let cut = snap.eq_cut.get(band as usize).copied().unwrap_or(false);
                    let solo = snap.eq_solo == band as i8;
                    let v = if band < 3 {
                        eq_to_knob(snap.eq[band as usize])
                    } else {
                        (snap.gain / 1.2).clamp(0.0, 1.0)
                    };
                    let c = if cut { t.red } else if solo { t.yellow } else { col };
                    let resp = rotary(ui, t, lab, v, c, cell + 4.0, cut, solo, if band < 3 { HelpControl::DeckEq } else { HelpControl::DeckGain });
                    if resp.changed {
                        if band < 3 {
                            self.send(Command::DeckEq { deck: d as u8, band, value: resp.value });
                        } else {
                            self.send(Command::DeckGain { deck: d as u8, value: resp.value * 1.2 });
                        }
                    }
                    if resp.clicked {
                        self.send(Command::DeckEqCut { deck: d as u8, band });
                    }
                    if resp.secondary {
                        self.send(Command::DeckEqSolo { deck: d as u8, band });
                    }
                }
            });
        });
    }

    fn platter_col(&mut self, ui: &mut Ui, t: &Theme, d: usize, snap: &crate::engine::DeckSnap, col: Color32, wave_h: f32) {
        ui.vertical(|ui| {
            ui.set_width(wave_h);
            let readout = Readout::from_snapshot(snap, self.deck_time[d]);
            let hit = platter(ui, t, snap, &readout, col, wave_h, |delta, touch| {
                self.send(Command::DeckTouch { deck: d as u8, on: touch });
                self.send(Command::DeckJog { deck: d as u8, delta });
            });
            if hit.shift_click {
                self.status = if self.submit(Command::DeckUnload { deck: d as u8 }) {
                    format!("queued unload → {}", (b'A' + d as u8) as char)
                } else { "Unload was not accepted".into() };
            } else if hit.right_click {
                self.send(Command::DeckCue { deck: d as u8 });
            } else if hit.click {
                self.send(Command::DeckPlay { deck: d as u8 });
            }
            // This row uses the same existing 28 px footer as the wave fader;
            // it does not shrink the platter or add height to the scratch area.
            ui.push_id(("deck-time", d), |ui| {
                let button = ui.button(RichText::new(self.deck_time[d].button_label()).size(t.text_size(11.0))).help(ui, HelpControl::DeckTime);
                egui::Popup::menu(&button)
                    .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                    .show(|ui| {
                    ui.label({ let __omatainer_args = (&((b'A' + d as u8) as char),); crate::localization::format("Deck {} time", &[format!("{}", __omatainer_args.0)]) });
                    ui.selectable_value(&mut self.deck_time[d].mode, TimeMode::Elapsed, tr!("Elapsed · source time")).help(ui, HelpControl::DeckTime);
                    ui.selectable_value(&mut self.deck_time[d].mode, TimeMode::Remaining, tr!("Remaining · wall estimate")).help(ui, HelpControl::DeckTime);
                    ui.separator();
                    ui.label(tr!("Warn before file end (seconds)"));
                    ui.add(egui::DragValue::new(&mut self.deck_time[d].warning_lead_seconds)
                        .range(0..=deck_time::MAX_WARNING_LEAD_SECONDS).suffix(" s")).help(ui, HelpControl::RunoutWarning);
                    ui.small("0 = off · repeating loops suppress alerts");
                });
            });
        });
    }

    fn btn_stack(&mut self, ui: &mut Ui, t: &Theme, d: usize, snap: &crate::engine::DeckSnap, sq: f32) {
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing = Vec2::splat(4.0);
            ui.set_width(sq);
            let q = self.snap.quantize;
            if sq_btn(ui, t, "Q", q, t.yellow, sq).help(ui, HelpControl::Quantize).clicked() {
                self.send(Command::ToggleQuant);
            }
            let io = sq_btn(ui, t, "I/O", snap.loop_on, t.accent, sq).help(ui, HelpControl::LoopBounds);
            let alternative = accessibility::actions(ui, &io, &["Loop in", "Loop out"]);
            if io.clicked() || alternative == Some(0) {
                self.send(Command::DeckLoopIn { deck: d as u8 });
            }
            if io.secondary_clicked() || alternative == Some(1) {
                self.send(Command::DeckLoopOut { deck: d as u8 });
            }
            if sq_btn(ui, t, "×2", false, t.cyan, sq).help(ui, HelpControl::LoopDouble).clicked() {
                self.send(Command::DeckLoopDouble { deck: d as u8 });
            }
            if sq_btn(ui, t, "½", false, t.cyan, sq).help(ui, HelpControl::LoopHalf).clicked() {
                self.send(Command::DeckLoopHalf { deck: d as u8 });
            }
            if sq_btn(ui, t, "↻", snap.loop_on, t.magenta, sq).help(ui, HelpControl::Reloop).clicked() {
                self.send(Command::DeckReloop { deck: d as u8 });
            }
            if sq_btn(ui, t, "⇄", snap.sync, t.green, sq).help(ui, HelpControl::Match).clicked() {
                self.send(Command::DeckMatch);
            }
        });
    }

    fn sampler_row(&mut self, ui: &mut Ui, t: &Theme) {
        ui.horizontal_wrapped(|ui| {
            let edit = ui.button(tr!("Edit banks"));
            accessibility::button(ui, &edit, "Edit sampler banks", None);
            help::annotate(ui, &edit, HelpControl::SamplerEdit);
            if edit.clicked() { self.open_sampler_editor(); }
            let midi = ui.button(tr!("Piano roll"));
            accessibility::button(ui, &midi, "Edit selected MIDI clip", None);
            help::annotate(ui, &midi, HelpControl::PianoRoll);
            if midi.clicked() { self.open_piano_roll(); }
            if let Some(target) = self.snap.compose_target {
                let name = self.snap.tracks.get(target.track).map(|tr| tr.name.as_str()).unwrap_or("track");
                ui.label(RichText::new({ let __omatainer_args = (&(name),&(target.scene + 1),); crate::localization::format("Compose armed: {} / scene {}", &[format!("{}", __omatainer_args.0), format!("{}", __omatainer_args.1)]) }).color(t.yellow));
                if ui.button(tr!("Disarm compose")).help(ui, HelpControl::ComposeDisarm).clicked() { self.send(Command::ComposeDisarm); }
            } else {
                ui.label(tr!("Compose disarmed"));
                if ui.button(tr!("Arm selected cell")).help(ui, HelpControl::ComposeArm).clicked() {
                    self.send(Command::ComposeArm { track: self.snap.selected_track, scene: self.snap.selected_scene });
                }
            }
            if self.snap.recording { ui.label(RichText::new(tr!("Recording pads")).color(t.red)); }
        });
        let h = ui.available_height();
        let w = ui.available_width();
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing = Vec2::splat(6.0);
            ui.vertical(|ui| {
                ui.set_width(108.0);
                ui.set_min_height(h);
                let bank = egui::ComboBox::from_id_salt("bank")
                    .selected_text(
                        self.snap
                            .sampler_banks
                            .get(self.snap.sampler_bank)
                            .cloned()
                            .unwrap_or_else(|| "bank".into()),
                    )
                    .show_ui(ui, |ui| {
                        for (i, n) in self.snap.sampler_banks.iter().enumerate() {
                            if ui.selectable_label(i == self.snap.sampler_bank, n).help(ui, HelpControl::SamplerBank).clicked() {
                                self.send(Command::SamplerBank(i));
                            }
                        }
                    });
                help::annotate(ui, &bank.response, HelpControl::SamplerBank);
                bank.response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::ComboBox, true, "Sampler bank"));
                ui.ctx().accesskit_node_builder(bank.response.id, |node| node.set_value(self.snap.sampler_banks.get(self.snap.sampler_bank).map(String::as_str).unwrap_or("No bank")));
                let instrument_label = if self.snap.sampler_unavailable { "Unavailable instrument" } else { self.snap.sampler_inst.label() };
                let instrument = egui::ComboBox::from_id_salt("inst")
                    .selected_text(instrument_label)
                    .show_ui(ui, |ui| {
                        for instrument in SamplerInstrument::ALL {
                            if ui.selectable_label(self.snap.sampler_inst == instrument, instrument.label())
                                .help_detail(ui, HelpControl::SamplerInstrument, instrument.description()).clicked() {
                                self.send(Command::SamplerInst(instrument));
                            }
                        }
                    });
                help::annotate(ui, &instrument.response, HelpControl::SamplerInstrument);
                instrument.response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::ComboBox, true, "Sampler instrument"));
                ui.ctx().accesskit_node_builder(instrument.response.id, |node| node.set_value(instrument_label));
                ui.horizontal(|ui| {
                    if sq_btn(ui, t, "^", false, t.accent, t.target_size(26.0)).help(ui, HelpControl::SamplerOctave).clicked() {
                        self.send(Command::SamplerOct(1));
                    }
                    ui.label(RichText::new({ let __omatainer_args = (&(self.snap.sampler_oct),); crate::localization::format("C{}", &[format!("{}", __omatainer_args.0)]) }).size(t.text_size(11.0)).color(t.fg));
                    if sq_btn(ui, t, "v", false, t.accent, t.target_size(26.0)).help(ui, HelpControl::SamplerOctave).clicked() {
                        self.send(Command::SamplerOct(-1));
                    }
                });
            });
            let pad_w = ((w - (108.0_f32).max(t.text_size(12.0) * 14.0) - 14.0) / 8.0).max(t.text_size(12.0) * 4.0);
            let pad_h = ((h - 4.0) / 2.0).max(t.target_size(32.0));
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing = Vec2::splat(4.0);
                let piano = self.snap.sampler_inst.synth().is_some();
                for row in [8u8, 0] {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing = Vec2::splat(4.0);
                        for column in 0..8u8 {
                            let pad = crate::engine::sampler_pad::PadIdentity::new(row + column);
                            let label = if piano { pad.piano_label() } else { pad.sample_label() };
                            let empty = label.is_empty();
                            let color = t.track_color(column as usize + if row == 0 { 8 } else { 0 });
                            let r = ui.push_id(("sampler-pad", pad.index()), |ui| {
                                pad_btn(ui, t, label, empty, color, Vec2::new(pad_w, pad_h))
                            }).inner;
                            help::rich_tooltip(&r, || {
                            let identity = if piano && empty {
                                format!("Pad {} · no piano accidental", pad.number())
                            } else if piano {
                                format!("{} · pad {} · MIDI note {}", label, pad.number(), pad.midi_note(self.snap.sampler_oct))
                            } else {
                                format!("Sample pad {} · bank slot {}", pad.number(), pad.index())
                            };
                            vec![identity, help::tooltip_text(HelpControl::SamplerPad)]
                            });
                            self.pad_gate(ui, pad.index(), !empty, &r);
                            help::describe(ui, &r, HelpControl::SamplerPad);
                        }
                    });
                }
            });
        });
    }

    fn crate_row(&mut self, ui: &mut Ui, t: &Theme) {
        self.crate_row_at(ui, t, SystemTime::now());
    }

    fn crate_row_at(&mut self, ui: &mut Ui, t: &Theme, now: SystemTime) {
        ui.vertical(|ui| {
            ui.horizontal_wrapped(|ui| {
                self.named_crate_selector(ui);
                let search = ui.add(egui::TextEdit::singleline(&mut self.lib_filter).id_salt("crate-search").hint_text(tr!("search")).desired_width(180.0));
                search.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, "Search crate"));
                help::annotate(ui, &search, HelpControl::CrateSearch);
                if ui.add_enabled(!self.library_scan.active() && self.library_metadata.ready() && !self.library_metadata.active(), egui::Button::new(tr!("scan"))).help(ui, HelpControl::CrateScan).clicked() {
                    self.scan_library();
                }
                if self.library_scan.active() && ui.button(tr!("cancel scan")).help(ui, HelpControl::CrateCancel).clicked() {
                    self.library_scan.cancel();
                }
                if ui.button(tr!("analyze…")).help(ui, HelpControl::LibraryAnalysis).clicked() { self.library_analysis.open = true; }
                if ui.button(tr!("annotations…")).help(ui, HelpControl::TrackAnnotations).clicked() { self.library_annotations.open = true; }
                if ui.button(tr!("tags…")).help(ui, HelpControl::TagEditor).clicked() { self.library_tags.open = true; }
                self.deck_selectors(ui);
                if ui.button(tr!("library…")).help(ui, HelpControl::Library).clicked() { self.library_import_open = true; }
                if ui.button(tr!("relocate…")).help(ui, HelpControl::CueRelocate).clicked() { self.open_cue_relocation(); }
                let load_a = ui.button(tr!("→ A"));
                accessibility::button(ui, &load_a, "Load selected crate item to deck A", None);
                help::annotate(ui, &load_a, HelpControl::DeckLoad);
                if load_a.clicked() {
                    self.load_sel(0);
                }
                let load_b = ui.button(tr!("→ B"));
                accessibility::button(ui, &load_b, "Load selected crate item to deck B", None);
                help::annotate(ui, &load_b, HelpControl::DeckLoad);
                if load_b.clicked() {
                    self.load_sel(1);
                }
                ui.label(RichText::new(if self.library_crates.selected.as_ref().and_then(|id|self.library_metadata.catalog.crates.node(id)).is_some_and(|node|node.annotation_rule.is_some()) { tr!("automatic annotation rule") } else if self.library_crates.selected.is_some() { tr!("manual crate order") } else { tr!("↓ bpm up   ↑ bpm down   same bpm → key → name") }).size(t.text_size(10.0)).color(t.muted));
                ui.label(RichText::new(tr!("metadata: inspect tags…")).size(t.text_size(10.0)).color(t.muted)).on_hover_text(key_hints::HELP);
                let progress = self.library_scan.label();
                ui.add(egui::Label::new(RichText::new(&progress).size(t.text_size(10.0)).color(t.fg_dim)).truncate())
                    .on_hover_text(progress);
            });
            ui.label(RichText::new(self.library_metadata.label()).size(t.text_size(10.0)).color(t.fg_dim));
            let selected_source=self.selected_library_item().map(|item|item.source.clone());
            if let Some(state)=selected_source.as_ref().and_then(|source|self.library_scan.summary.as_ref().and_then(|s|s.availability.get(source))) {
                ui.label(crate::localization::format("Last scan: {state}. Library records are retained.", &[format!("{}", state)]));
            }
            let header = ["song", "bpm · source", "key", "length", "last play", "artist", "rating", "color", "group", "tags", "notes"];
            let col_w = [280.0, 112.0, 48.0, 64.0, 140.0, 180.0, 64.0, 88.0, 128.0, 160.0, 240.0].map(|width| width * (t.text_size(11.0) / 11.0).max(1.0));
            ui.horizontal(|ui| {
                for (h, w) in header.iter().zip(col_w.iter()) {
                    ui.add_sized(Vec2::new(*w, t.target_size(16.0)), egui::Label::new(RichText::new(*h).size(t.text_size(10.0)).color(t.fg_dim)));
                }
            });
            self.refresh_library_view();
            if !self.library_view.annotation_error.is_empty() { ui.colored_label(ui.visuals().warn_fg_color, &self.library_view.annotation_error); }
            if self.library_view.unavailable != 0 {
                ui.label({ let __omatainer_args = (&(self.library_view.unavailable),); crate::localization::format("{} saved members unavailable in this published view; inspect Named crates for their identities.", &[format!("{}", __omatainer_args.0)]) });
            }
            let focus = ui.make_persistent_id("crate-navigation");
            self.crate_navigation(ui, focus);
            let row_height = t.target_size(18.0);
            let stride = row_height + ui.spacing().item_spacing.y;
            #[cfg(test)] { self.library_view.stats.rendered = 0; self.library_view.stats.formatted = 0; }
            let mut scroll = egui::ScrollArea::both().id_salt("crate-rows").animated(!t.reduced_motion).auto_shrink([false, false]);
            if let Some(offset) = self.library_view.pending_offset.take() {
                scroll = scroll.vertical_scroll_offset(offset);
            }
            let output = scroll.show_rows(ui, row_height, self.library_view.indices.len(), |ui, rows| {
                self.library_view.cells.retain(|index, _| rows.contains(index));
                for i in rows {
                    let item = &self.library[self.library_view.indices[i]];
                    let played_at = self.item_last_play(item);
                    let cells = self.library_view.cells.entry(i).or_insert_with(|| {
                        #[cfg(test)] { self.library_view.stats.formatted += 1; }
                        let mut cells = Cells::new(item, played_at, now);
                        if let Some(track) = self.library_metadata.catalog.track(&item.source) { cells.annotations = track.annotations.columns(); }
                        cells
                    });
                    if cells.refresh_play(played_at, now) {
                        #[cfg(test)] { self.library_view.stats.formatted += 1; }
                    }
                    if let Some(deadline) = cells.played_refresh_at {
                        // Eframe adds this duration to a native Instant. Cap
                        // distant future dates so malformed clocks cannot overflow it.
                        let delay = deadline.duration_since(now).unwrap_or_default()
                            .min(std::time::Duration::from_secs(86400));
                        ui.ctx().request_repaint_after(delay);
                    }
                    #[cfg(test)] { self.library_view.stats.rendered += 1; }
                    let sel = i == self.lib_sel;
                    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width().max(col_w.iter().sum()), t.target_size(18.0)), Sense::hover());
                    let resp = ui.interact(rect, ui.id().with(("crate-source-row", &item.source)), Sense::click());
                    if sel { ui.painter().rect_filled(rect, 2.0, t.tint(t.accent, 0.18)); }
                    if sel { active_mark(ui.painter(), rect, t.fg); }
                    let mut x = rect.left();
                    for (txt, w) in [&item.title, &cells.bpm, &item.key, &cells.length, &cells.played, &item.artist, &cells.annotations[0], &cells.annotations[1], &cells.annotations[2], &cells.annotations[3], &cells.annotations[4]].iter().zip(col_w) {
                        ui.painter().text(Pos2::new(x + 4.0, rect.center().y), egui::Align2::LEFT_CENTER,
                            *txt, FontId::proportional(t.text_size(11.0)), if sel { t.accent } else { t.fg });
                        x += w;
                    }
                    let identity = self.library_metadata.catalog.track(&item.source).map(|track| track.id.0.as_str()).unwrap_or("not saved yet");
                    accessibility::button(ui, &resp, &format!("Crate row {}: {}, artist {}, BPM {}, key {}, length {}, {}", i + 1, item.title, item.artist, cells.bpm, item.key, cells.length, cells.played_tooltip), Some(sel));
                    if let Some(track) = self.library_metadata.catalog.track(&item.source) { accessibility::status(ui, &resp, &track.annotations.description()); }
                    let row_action = accessibility::actions(ui, &resp, &["Select", "Load to deck A", "Load to deck B", "Load to selected deck"]);
                    help::describe(ui, &resp, HelpControl::CrateRow);
                    help::rich_tooltip(&resp, || vec![
                        format!("BPM: {}", item.bpm.label()),
                        self.library_metadata.catalog.version(&item.source, item.fingerprint).and_then(|v| v.tags.as_ref())
                            .map_or_else(|| "Title/artist/key: filename or catalog fallback; embedded tags not yet inspected".into(), |tags| tags.describe()),
                        cells.played_tooltip.clone(),
                        self.library_metadata.catalog.track(&item.source).map(|track|track.annotations.description()).unwrap_or_default(),
                        format!("Track ID: {identity}"), format!("Location: {:?}", item.source),
                        help::tooltip_text(HelpControl::CrateRow),
                    ]);
                    if resp.clicked() || resp.double_clicked() || row_action.is_some() {
                        self.lib_sel = i;
                        ui.memory_mut(|memory| memory.request_focus(focus));
                    }
                    if resp.double_clicked() || row_action == Some(3) { self.load_sel(self.load_target() as u8); }
                    else if let Some(deck) = row_action.and_then(|action| [None, Some(0), Some(1), None][action]) { self.load_sel(deck); }
                }
            });
            accessibility::scrollbars(ui, "Crate", &output);
            let navigation = ui.interact(output.inner_rect, focus, Sense::focusable_noninteractive());
            let count = self.library_view.indices.len();
            if let Some(row) = accessibility::numeric(ui, &navigation, "Crate selection", (self.lib_sel + 1).min(count) as f32, if count == 0 { 0.0 } else { 1.0 }, count as f32, 1.0, " row") {
                self.lib_sel = (row.round() as usize).saturating_sub(1).min(count.saturating_sub(1));
                self.library_view.pending_offset = Some(self.lib_sel as f32 * stride);
            }
            let selected = self.library_view.indices.get(self.lib_sel).map(|&i| &self.library[i]);
            ui.ctx().accesskit_node_builder(focus, |node| {
                node.set_value(selected.map(|item| format!("{} of {count}: {}, {}", self.lib_sel + 1, item.title, item.artist)).unwrap_or_else(|| "Empty crate".into()));
                node.set_description("All filtered rows are available: Up/Down, Page Up/Down, Home/End; F2 enters a row number. Shift+F10 loads the selected row. Search narrows the crate.");
            });
            let action = accessibility::actions(ui, &navigation, &["Load to deck A", "Load to deck B", "Load to selected deck"]);
            help::describe(ui, &navigation, HelpControl::CrateRow);
            if let Some(action) = action { self.load_sel(if action == 2 { self.load_target() as u8 } else { action as u8 }); }
            self.remember_crate_viewport(output.state.offset.y, output.inner_rect.height(), stride);
        });
    }

    fn sequencer_row(&mut self, ui: &mut Ui, t: &Theme) {
        self.session_toolbar(ui);
        let Some(layout) = self.snap.session.clone() else { return; };
        let avail = ui.available_size();
        let gap = 4.0;
        let head_h = t.target_size(26.0);
        let gain_h = t.target_size(32.0) + t.text_size(9.0) + 16.0;
        let scene_w = (100.0_f32).max(t.text_size(11.0) * 9.0);
        let cols = layout.track_order.len() as f32;
        let rows = layout.scene_order.len() as f32;
        let col_w = if cols <= 8.0 { ((avail.x - scene_w - gap * (cols + 1.0)) / cols).max(t.text_size(11.0) * 7.0) } else { (100.0_f32).max(t.text_size(11.0) * 7.0) };
        let row_h = t.target_size(26.0);
        let pack_w = scene_w + gap + cols * col_w + (cols - 1.0) * gap;
        let pack_h = head_h + gap + rows * row_h + (rows - 1.0) * gap + gap + gain_h;
        ui.spacing_mut().item_spacing = Vec2::splat(gap);
        let reveal = self.session_editor.reveal.take();
        let mut grid_gained_focus = false;
        let mut paint = |ui: &mut Ui, viewport: Rect| {
                let origin = ui.min_rect().min;
                ui.set_min_size(Vec2::new(pack_w, pack_h));
                let column_stride = col_w + gap;
                let row_stride = row_h + gap;
                let columns = session_editor::visible_range(viewport.min.x - scene_w - gap, viewport.max.x - scene_w - gap, column_stride, layout.track_order.len());
                let scenes = session_editor::visible_range(viewport.min.y - head_h - gap, viewport.max.y - head_h - gap, row_stride, layout.scene_order.len());
                if let Some((track, scene)) = reveal {
                    if let (Some(x),Some(y)) = (layout.track_order.iter().position(|s|usize::from(*s)==track),layout.scene_order.iter().position(|s|usize::from(*s)==scene)) {
                        ui.scroll_to_rect(Rect::from_min_size(origin + Vec2::new(scene_w + gap + x as f32 * column_stride, head_h + gap + y as f32 * row_stride), Vec2::new(col_w,row_h)), Some(Align::Center));
                    }
                }
                ui.with_layout(egui::Layout::top_down(Align::Min), |ui| {
                    ui.spacing_mut().item_spacing = Vec2::splat(gap);
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing = Vec2::splat(gap);
                        let _ = ui.allocate_exact_size(Vec2::new(scene_w, head_h), Sense::hover());
                        if columns.start > 0 { ui.add_space(columns.start as f32 * column_stride - gap); }
                        for display_track in columns.clone() {
                            let tr = usize::from(layout.track_order[display_track]);
                            let name = self.snap.tracks.get(tr).map(|x| x.name.as_str()).unwrap_or("tr");
                            let mute = self.snap.tracks.get(tr).map(|x| x.mute).unwrap_or(false);
                            let solo = self.snap.tracks.get(tr).map(|x| x.solo).unwrap_or(false);
                            let (rect, _) = ui.allocate_exact_size(Vec2::new(col_w, head_h), Sense::hover());
                            let resp = ui.interact(rect, egui::Id::new(("session-track",layout.namespace,layout.tracks[tr].id.0)), Sense::click());
                            let fill = if mute {
                                t.tint(t.red, 0.35)
                            } else if solo {
                                t.tint(t.yellow, 0.35)
                            } else {
                                t.bg_dark
                            };
                            ui.painter().rect_filled(rect, 4.0, fill);
                            ui.painter().rect_stroke(rect, 4.0, st(1.0, t.marker(layout.tracks[tr].color.map(|c| Color32::from_rgb(c[0],c[1],c[2])).unwrap_or_else(||t.track_color(tr)), fill)), egui::StrokeKind::Inside);
                            ui.painter_at(rect).text(rect.center(), egui::Align2::CENTER_CENTER, format!("{}{name}", if mute { "M " } else if solo { "S " } else { "" }), FontId::proportional(t.text_size(11.0)), t.fg);
                            grid_gained_focus |= resp.gained_focus();
                            accessibility::button(ui, &resp, &format!("Track {} {}: Mute", display_track + 1, name), Some(mute));
                            accessibility::status(ui, &resp, &format!("Mute {}; solo {}", if mute { "on" } else { "off" }, if solo { "on" } else { "off" }));
                            let action = accessibility::actions(ui, &resp, &["Toggle mute", "Toggle solo", "Open track effects", "Select track", "Edit track"]);
                            help::annotate(ui, &resp, HelpControl::Track);
                            if action == Some(4) {self.send(Command::Select {track:tr,scene:self.snap.selected_scene}); self.session_editor.open=true; self.session_editor.select_axis(crate::engine::session::Axis::Track);}
                            if resp.clicked() || action == Some(0) {
                                self.send(Command::Mute { track: tr as u8 });
                            }
                            if action == Some(2) { self.send(Command::OpenFxTrack(tr as u8)); }
                            if action == Some(3) { self.send(Command::Select { track: tr, scene: self.snap.selected_scene }); }
                            if resp.secondary_clicked() || action == Some(1) {
                                self.send(Command::Solo { track: tr as u8 });
                            }
                        }
                    });
                    if scenes.start > 0 { ui.add_space(scenes.start as f32 * row_stride - gap); }
                    for display_scene in scenes.clone() {
                        let sc = usize::from(layout.scene_order[display_scene]);
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing = Vec2::splat(gap);
                            let on = self.snap.playing && self.snap.tracks.iter().any(|tr| tr.playing_scene == sc as i16 && !tr.clip_pending);
                            let queued = self.snap.tracks.iter().any(|tr| tr.playing_scene == sc as i16 && tr.clip_pending);
                            let (hr, _) = ui.allocate_exact_size(Vec2::new(scene_w, row_h), Sense::hover());
                            let hresp = ui.interact(hr, egui::Id::new(("session-scene",layout.namespace,layout.scenes[sc].id.0)), Sense::click());
                            let scene_color=layout.scenes[sc].color.map(|c|Color32::from_rgb(c[0],c[1],c[2])).unwrap_or(t.accent);
                            ui.painter().rect_filled(hr,4.0,if on {t.tint(scene_color, 0.45)} else if layout.scenes[sc].color.is_some() {t.tint(scene_color, 0.22)} else {t.bg_dark});
                            ui.painter().rect_stroke(hr, 4.0, st(1.0, if on || queued || layout.scenes[sc].color.is_some() { scene_color } else { t.marker(t.muted, t.bg_dark) }), egui::StrokeKind::Inside);
                            ui.painter().with_clip_rect(hr).text(hr.center(), egui::Align2::CENTER_CENTER, &format!("{} {} {}", if queued { "Q" } else if on { ">" } else { "[]" }, display_scene+1,layout.scenes[sc].name), FontId::proportional(t.text_size(11.0)), t.fg);
                            grid_gained_focus |= hresp.gained_focus();
                            accessibility::button(ui, &hresp, &format!("Scene {}: Toggle playback", display_scene + 1), Some(on));
                            accessibility::status(ui,&hresp,&layout.scenes[sc].name);
                            let action = accessibility::actions(ui, &hresp, &["Toggle scene", "Add scene", "Open scene effects", "Restart scene", "Edit scene"]);
                            help::annotate(ui, &hresp, HelpControl::Scene);
                            if action == Some(4) { self.send(Command::Select {track:self.snap.selected_track,scene:sc}); self.session_editor.open=true; self.session_editor.select_axis(crate::engine::session::Axis::Scene); }
                            if hresp.clicked() || matches!(action, Some(0..=2)) {
                                if action == Some(1) || action.is_none() && ui.input(|i| i.modifiers.shift) {
                                    self.send(Command::AddScene { scene: sc as u16 });
                                } else if action == Some(2) || action.is_none() && ui.input(|i| i.modifiers.ctrl) {
                                    self.send(Command::OpenFxScene(sc as u16));
                                } else {
                                    self.send(Command::ToggleScene { scene: sc as u16 });
                                }
                            }
                            if hresp.secondary_clicked() || action == Some(3) {
                                self.send(Command::RestartScene { scene: sc as u16 });
                            }
                            if columns.start > 0 { ui.add_space(columns.start as f32 * column_stride - gap); }
                        for display_track in columns.clone() {
                            let tr = usize::from(layout.track_order[display_track]);
                                let clip = self.snap.tracks.get(tr).and_then(|x| x.clips.get(sc));
                                let filled = clip.map(|c| c.kind != 0).unwrap_or(false);
                                let queued = self.snap.tracks.get(tr).is_some_and(|x| x.playing_scene == sc as i16 && x.clip_pending);
                                let playing = self.snap.playing && self.snap.tracks.get(tr).is_some_and(|x| x.playing_scene == sc as i16 && !x.clip_pending);
                                let looping = self.snap.tracks.get(tr).map(|x| x.clip_looping).unwrap_or(false);
                                let color = layout.tracks[tr].color.map(|c| Color32::from_rgb(c[0],c[1],c[2])).unwrap_or_else(||t.track_color(tr));
                                let (rect, _) = ui.allocate_exact_size(Vec2::new(col_w, row_h), Sense::hover());
                                let resp = ui.interact(rect, egui::Id::new(("session-clip",layout.namespace,layout.tracks[tr].id.0,layout.scenes[sc].id.0)), Sense::click());
                                let fill = if playing {
                                    t.tint(color, 0.55)
                                } else if filled {
                                    t.tint(color, 0.22)
                                } else {
                                    t.bg_dark
                                };
                                ui.painter().rect_filled(rect, 4.0, fill);
                                ui.painter().rect_stroke(
                                    rect,
                                    4.0,
                                    st(if queued || (looping && playing) { 2.0 } else { 1.0 }, t.marker(color, fill)),
                                    egui::StrokeKind::Inside,
                                );
                                if filled {
                                    let name = clip.map(|c| c.name.as_str()).unwrap_or("");
                                    let label = format!("{} {name}", if queued { "Q" } else if playing { ">" } else { "[]" });
                                    ui.painter_at(rect).text(
                                        rect.center(),
                                        egui::Align2::CENTER_CENTER,
                                        &label,
                                        FontId::proportional(t.text_size(11.0)),
                                        t.fg,
                                    );
                                    if playing {
                                        let w = rect.width() * self.snap.tracks.get(tr).map(|x| x.clip_progress).unwrap_or(0.0);
                                        ui.painter().rect_filled(Rect::from_min_size(rect.min, Vec2::new(w, 3.0)), 0.0, t.fg);
                                    }
                                }
                                grid_gained_focus |= resp.gained_focus();
                                accessibility::button(ui, &resp, &format!("Clip track {} scene {}: {}", display_track + 1, display_scene + 1, clip.map(|c| c.name.as_str()).filter(|name| !name.is_empty()).unwrap_or("Empty")), Some(playing));
                                accessibility::status(ui, &resp, &format!("{}; {}", if queued { "Queued" } else if playing { "Playing" } else { "Stopped" }, if looping { "Looping" } else { "One shot" }));
                                let labels: &[&str] = if filled { &["Launch once", "Launch loop", "Arm compose", "Edit clip gain"] } else { &["Launch once", "Launch loop", "Arm compose"] };
                                let action = accessibility::actions(ui, &resp, labels);
                                help::annotate(ui, &resp, HelpControl::Clip);
                                if resp.clicked() || matches!(action, Some(0 | 2 | 3)) {
                                    if action == Some(3) || action.is_none() && ui.input(|i| i.modifiers.alt) {
                                        if filled {
                                            self.clip_gain_edit = Some(ClipGainEdit {
                                                track: tr as u8, scene: sc as u16,
                                                value: clip.map(|c| c.gain).unwrap_or(1.0),
                                                target: layout.reference(crate::engine::session::Axis::Track,tr).zip(layout.reference(crate::engine::session::Axis::Scene,sc)),
                                            });
                                        }
                                    } else if action == Some(2) || action.is_none() && ui.input(|i| i.modifiers.shift) {
                                        self.send(Command::ComposeArm { track: tr, scene: sc });
                                    } else {
                                        self.send(Command::FireClip { track: tr as u8, scene: sc as u16, looping: false });
                                    }
                                }
                                if resp.secondary_clicked() || action == Some(1) {
                                    self.send(Command::FireClip { track: tr as u8, scene: sc as u16, looping: true });
                                }
                            }
                        });
                    }
                    if scenes.end < layout.scene_order.len() { ui.add_space((layout.scene_order.len() - scenes.end) as f32 * row_stride - gap); }
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing = Vec2::splat(gap);
                        let _ = ui.allocate_exact_size(Vec2::new(scene_w, gain_h), Sense::hover());
                        if columns.start > 0 { ui.add_space(columns.start as f32 * column_stride - gap); }
                        for display_track in columns.clone() {
                            let tr = usize::from(layout.track_order[display_track]);
                            let g = self.snap.tracks.get(tr).map(|x| x.gain).unwrap_or(0.8);
                            let mute = self.snap.tracks.get(tr).map(|x| x.mute).unwrap_or(false);
                            let solo = self.snap.tracks.get(tr).map(|x| x.solo).unwrap_or(false);
                            let col = layout.tracks[tr].color.map(|c| Color32::from_rgb(c[0],c[1],c[2])).unwrap_or_else(||t.track_color(tr));
                            let c = if mute { t.red } else if solo { t.yellow } else { col };
                            let (cell, _) = ui.allocate_exact_size(Vec2::new(col_w, gain_h), Sense::hover());
                            let knob = (col_w.min(gain_h) - 2.0).clamp(28.0, 44.0);
                            let resp = rotary_in(ui, t, "gain", (g / 1.2).clamp(0.0, 1.0), c, knob, cell, tr, mute, solo);
                            grid_gained_focus |= resp.gained_focus;
                            if resp.changed {
                                self.send(Command::TrackGain { track: tr as u8, value: resp.value * 1.2 });
                            }
                            if resp.clicked || resp.effects {
                                if resp.effects {
                                    self.send(Command::OpenFxTrack(tr as u8));
                                } else {
                                    self.send(Command::Mute { track: tr as u8 });
                                }
                            }
                            if resp.secondary {
                                self.send(Command::Solo { track: tr as u8 });
                            }
                        }
                    });
                });
            };
        if cols > 8.0 || rows > 8.0 || pack_w > avail.x || pack_h > avail.y {
            let area = egui::ScrollArea::both().id_salt("session-grid").auto_shrink([false, false])
                .max_width(ui.clip_rect().width().min(avail.x)).max_height(avail.y.max(100.0)).animated(false).show_viewport(ui, &mut paint);
            if grid_gained_focus || reveal.is_some() { ui.scroll_to_rect_animation(area.inner_rect, Some(Align::Center),egui::style::ScrollAnimation::none()); }
            accessibility::scrollbars(ui,"Session grid",&area);
        } else {
            paint(ui, Rect::from_min_size(Pos2::ZERO, Vec2::new(pack_w,pack_h)));
        }
    }

    fn fx_row(&mut self, ui: &mut Ui, t: &Theme) {
        ui.horizontal(|ui| {
            let label = if self.snap.fx_view >= crate::engine::session::SCENE_FX_BASE {
                format!("scene {} fx", self.snap.fx_view - crate::engine::session::SCENE_FX_BASE + 1)
            } else {
                format!(
                    "{} fx",
                    self.snap.tracks.get(self.snap.fx_view as usize).map(|x| x.name.as_str()).unwrap_or("track")
                )
            };
            ui.label(RichText::new(label).color(t.accent).strong());
            if pill(ui, t, "back", false, t.fg).help(ui, HelpControl::FxClose).clicked() {
                self.send(Command::CloseFx);
            }
        });
        ui.horizontal_wrapped(|ui| {
            for (i, id) in FxId::all().iter().enumerate() {
                if self.snap.fx_view >= crate::engine::session::SCENE_FX_BASE && !id.supports_scene() {
                    continue;
                }
                if pill(ui, t, id.name(), false, t.cyan).help(ui, HelpControl::FxAdd).clicked() {
                    self.send(Command::FxAdd(i as u8));
                }
            }
        });
        egui::ScrollArea::vertical().show(ui, |ui| {
            for (i, (name, on, mix, p)) in self.snap.fx_slots.clone().into_iter().enumerate() {
                self.fx_slot_controls(ui, t, i, &name, on, mix, p);
            }
            if self.snap.fx_slots.is_empty() {
                ui.label(RichText::new(tr!("add a device — chain runs top to bottom")).color(t.muted));
            }
        });
    }
}

fn split_artist_title(stem: &str) -> (String, String) {
    if let Some((a, b)) = stem.split_once(" - ") {
        (a.trim().into(), b.trim().into())
    } else {
        (String::new(), stem.to_string())
    }
}

fn parse_tags(stem: &str) -> (f32, String) {
    let mut bpm = 0.0f32;
    for tok in stem.split(|c: char| !c.is_ascii_alphanumeric()) {
        if let Ok(n) = tok.parse::<f32>() {
            if (60.0..200.0).contains(&n) {
                bpm = n;
            }
        }
    }
    (bpm, key_hints::from_filename(stem).unwrap_or_else(|| "—".into()))
}

fn sort_crate(items: &mut [LibItem]) {
    items.sort_by(|a, b| {
        let ba = a.bpm.value().unwrap_or(f32::INFINITY);
        let bb = b.bpm.value().unwrap_or(f32::INFINITY);
        ba.partial_cmp(&bb)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.key.cmp(&b.key))
            .then_with(|| crate::localization::search_key(&a.title).cmp(&crate::localization::search_key(&b.title)))
    });
}

/// Duration of the successfully decoded playable buffer. An unavailable
/// declared source length does not make this measured frame count unknown.
fn decoded_duration(sample: &crate::engine::dsp::Sample) -> Option<f64> {
    if sample.sr == 0 || sample.ch == 0 || sample.data.len() % sample.ch as usize != 0 {
        return None;
    }
    Some(sample.frames() as f64 / sample.sr as f64)
}

fn fmt_len(seconds: Option<f64>) -> String {
    match seconds.filter(|s| s.is_finite() && *s >= 0.0) {
        Some(seconds) => {
            let seconds = seconds as u64;
            format!("{}:{:02}", seconds / 60, seconds % 60)
        }
        None => "unknown".into(),
    }
}


/// Draw a full toolbar, or its scrollable menu in a compact control bar.
/// `ctx`, optional parent, stable panel ID, caption and contents define the toolbar; returns no value.
fn toolbar(ctx: &egui::Context, parent: Option<&mut Ui>, id: &'static str, title: &str, draw: impl FnOnce(&mut Ui)) {
    if let Some(ui) = parent {
        let menu = ui.menu_button(title, |ui| {
            egui::ScrollArea::both().id_salt(id).auto_shrink([false,true])
                .max_width((ctx.screen_rect().width()-32.0).max(80.0))
                .max_height((ctx.screen_rect().height()*0.7).max(80.0)).show(ui, draw);
        });
        help::annotate(ui, &menu.response, HelpControl::DisplayLayout);
    } else { egui::TopBottomPanel::top(id).resizable(false).show(ctx, draw); }
}
/// Compute coherent deck geometry for readable controls.
/// `w`, `h`, fader height, gap and minimum square are logical units; returns square, waveform, side and center sizes.
fn scratch_metrics(w: f32, h: f32, fader_h: f32, gap: f32, minimum_square: f32) -> (f32, f32, f32, f32) {
    let minimum_wave = (minimum_square * 7.0).max(120.0);
    let mut wave_h = (h - fader_h - gap).max(minimum_wave);
    let mut sq = 26.0;
    let mut side_w = 0.0;
    let mut mid_w = 0.0;
    for _ in 0..12 {
        sq = (wave_h / 7.0).clamp(minimum_square, minimum_square.max(34.0));
        let cue_w = 4.0 * (sq + 4.0) + 9.0;
        side_w = sq + gap + cue_w + gap + wave_h + gap + sq;
        mid_w = w - 2.0 * side_w - 2.0 * gap;
        if mid_w >= 120.0 {
            break;
        }
        wave_h *= 0.92;
        if wave_h < minimum_wave {
            wave_h = minimum_wave;
            sq = (wave_h / 7.0).clamp(minimum_square, minimum_square.max(34.0));
            let cue_w = 4.0 * (sq + 4.0) + 9.0;
            side_w = sq + gap + cue_w + gap + wave_h + gap + sq;
            mid_w = (w - 2.0 * side_w - 2.0 * gap).max(96.0);
            break;
        }
    }
    (sq, wave_h, side_w, mid_w.max(96.0))
}

/// Mark an enabled button with a check independent of its hue.
/// `painter`, control rectangle and foreground define the mark; returns no value.
fn active_mark(painter: &egui::Painter, rect: Rect, color: Color32) {
    let p = rect.right_top() + Vec2::new(-9.0, 5.0);
    painter.line_segment([p + Vec2::new(-4.0, 2.0), p + Vec2::new(-2.0, 4.0)], st(1.5, color));
    painter.line_segment([p + Vec2::new(-2.0, 4.0), p + Vec2::new(3.0, -1.0)], st(1.5, color));
}
fn st(width: f32, color: Color32) -> Stroke {
    Stroke { width, color }
}

fn pill(ui: &mut Ui, t: &Theme, text: &str, on: bool, accent: Color32) -> egui::Response {
    let fill = if on { t.tint(accent, 0.35) } else { t.bg_light };
    let stroke = if on { accent } else { t.marker(t.muted, t.bg_dark) };
    let galley = ui.painter().layout_no_wrap(text.to_owned(), FontId::proportional(t.text_size(11.0)), if on { t.fg_bright } else { t.fg });
    let size = Vec2::new((galley.size().x + 28.0).max(t.target_size(28.0)), t.target_size(20.0));
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    ui.painter().rect_filled(rect, 4.0, fill);
    ui.painter().rect_stroke(rect, 4.0, st(1.0, stroke), egui::StrokeKind::Inside);
    ui.painter().galley(Pos2::new(rect.center().x - galley.size().x * 0.5, rect.center().y - galley.size().y * 0.5), galley, t.fg);
    if on { active_mark(ui.painter(), rect, t.fg); }
    accessibility::button(ui, &resp, text, Some(on));
    resp
}

fn sq_btn(ui: &mut Ui, t: &Theme, text: &str, on: bool, accent: Color32, size: f32) -> egui::Response {
    let size = t.target_size(size);
    let fill = if on { t.tint(accent, 0.4) } else { t.bg_light };
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), Sense::click());
    ui.painter().rect_filled(rect, 4.0, fill);
    ui.painter().rect_stroke(rect, 4.0, st(1.0, if on { accent } else { t.marker(t.muted, t.bg_dark) }), egui::StrokeKind::Inside);
    // The cue number remains visible at large text sizes; its full name stays in help and accessibility.
    let visible = if t.font_size > 18.0 { text.lines().next().unwrap_or(text) } else { text };
    ui.painter_at(rect).text(rect.center(), egui::Align2::CENTER_CENTER, visible, FontId::proportional(t.text_size(11.0)), t.fg);
    if on { active_mark(ui.painter(), rect, t.fg); }
    let name = match text { "L" | "L!" | "L~" => "Pitch lock", "Q" => "Quantize", "I/O" => "Loop in", "×2" => "Double loop", "½" => "Halve loop", "↻" => "Reloop", "⇄" => "Match decks", "^" => "Octave up", "v" => "Octave down", text => text };
    accessibility::button(ui, &resp, name, Some(on));
    resp
}

fn pad_btn(ui: &mut Ui, t: &Theme, text: &str, empty: bool, col: Color32, size: Vec2) -> egui::Response {
    let size = Vec2::new(t.target_size(size.x), t.target_size(size.y));
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click_and_drag());
    ui.painter().rect_filled(rect, 4.0, if empty { t.bg_darker } else { t.tint(col, 0.28) });
    ui.painter().rect_stroke(rect, 4.0, st(1.0, t.marker(col, t.bg_darker)), egui::StrokeKind::Inside);
    if !empty {
        ui.painter_at(rect).text(rect.center(), egui::Align2::CENTER_CENTER, text, FontId::proportional(t.text_size(12.0)), t.fg);
    }
    resp
}

struct RotaryResp {
    gained_focus: bool,
    value: f32,
    changed: bool,
    clicked: bool,
    secondary: bool,
    effects: bool,
}

fn rotary_in(
    ui: &mut Ui,
    t: &Theme,
    label: &str,
    value: f32,
    col: Color32,
    size: f32,
    cell: Rect,
    id: usize,
    mute: bool,
    solo: bool,
) -> RotaryResp {
    let size = size.min(cell.height() - t.text_size(9.0) - 5.0);
    let cx = cell.center().x;
    let cy = cell.top() + size * 0.5;
    let knob = Rect::from_center_size(Pos2::new(cx, cy), Vec2::splat(size));
    let resp = ui.interact(knob, ui.id().with(("rotary", id)), Sense::click_and_drag());
    let c = knob.center();
    let r = size * 0.38;
    ui.painter().circle_filled(c, r, t.bg_darker);
    ui.painter().circle_stroke(c, r, st(1.5, col));
    let ang = -2.2 + value.clamp(0.0, 1.0) * 4.4;
    let dir = Vec2::angled(ang);
    ui.painter().line_segment([c, c + dir * (r - 3.0)], st(2.0, col));
    ui.painter().text(
        Pos2::new(cx, knob.bottom() + 1.0),
        egui::Align2::CENTER_TOP,
        label,
        FontId::proportional(t.text_size(9.0)),
        t.fg_dim,
    );
    let mut out = RotaryResp {
        gained_focus: resp.gained_focus(),
        value,
        changed: false,
        clicked: resp.clicked(),
        secondary: resp.secondary_clicked(),
        effects: false,
    };
    if resp.dragged() {
        out.value = (value - resp.drag_delta().y * 0.01).clamp(0.0, 1.0);
        out.changed = true;
    }
    if let Some(next) = accessibility::numeric(ui, &resp, &format!("Track {}: Gain", id + 1), value * 120.0, 0.0, 120.0, 1.0, "%") {
        out.value = next / 120.0; out.changed = true;
    }
    accessibility::status(ui, &resp, &format!("Mute {}; solo {}", if mute { "on" } else { "off" }, if solo { "on" } else { "off" }));
    let action = accessibility::actions(ui, &resp, &["Toggle mute", "Toggle solo", "Open track effects"]);
    help::annotate(ui, &resp, HelpControl::TrackGain);
    out.clicked |= action == Some(0);
    out.secondary |= action == Some(1);
    out.effects = action == Some(2) || action.is_none() && resp.clicked() && ui.input(|i| i.modifiers.ctrl);
    out
}

fn rotary(ui: &mut Ui, t: &Theme, label: &str, value: f32, col: Color32, size: f32, cut: bool, solo: bool, control: HelpControl) -> RotaryResp {
    ui.vertical(|ui| {
        ui.set_width(size);
        let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), Sense::click_and_drag());
        let c = rect.center();
        let r = size * 0.38;
        ui.painter().circle_filled(c, r, t.bg_darker);
        ui.painter().circle_stroke(c, r, st(1.5, col));
        let ang = -2.2 + value.clamp(0.0, 1.0) * 4.4;
        let dir = Vec2::angled(ang);
        ui.painter().line_segment([c, c + dir * (r - 3.0)], st(2.0, col));
        ui.label(RichText::new({ let __omatainer_args = (&(if cut { " M" } else if solo { " S" } else { "" }),); crate::localization::format("{label}{}", &[format!("{}", label), format!("{}", __omatainer_args.0)]) }).size(t.text_size(9.0)).color(t.fg_dim));
        let mut out = RotaryResp {
        gained_focus: resp.gained_focus(),
            value,
            changed: false,
            clicked: resp.clicked(),
            secondary: resp.secondary_clicked(),
        effects: false,
        };
        if resp.dragged() {
            out.value = (value - resp.drag_delta().y * 0.01).clamp(0.0, 1.0);
            out.changed = true;
        }
        let full_label = match label { "b" => "Bass EQ", "m" => "Mid EQ", "t" => "Treble EQ", _ => "Gain" };
        let gain = if label == "g" { value * 1.2 } else if value <= 0.5 { (value * 2.0).powf(1.4) } else { 1.0 + (value - 0.5) * 4.8 };
        let maximum = if label == "g" { 120.0 } else { 340.0 };
        if let Some(next) = accessibility::numeric(ui, &resp, full_label, gain * 100.0, 0.0, maximum, 1.0, "%") {
            out.value = if label == "g" { next / 120.0 } else { eq_to_knob(next / 100.0) }; out.changed = true;
        }
        accessibility::status(ui, &resp, &format!("Cut {}; solo {}", if cut { "on" } else { "off" }, if solo { "on" } else { "off" }));
        let action = accessibility::actions(ui, &resp, &["Toggle cut", "Toggle solo"]);
        help::annotate(ui, &resp, control);
        out.clicked |= action == Some(0); out.secondary |= action == Some(1);
        out
    })
    .inner
}

struct PlatterHit {
    click: bool,
    right_click: bool,
    shift_click: bool,
}

fn platter(
    ui: &mut Ui,
    t: &Theme,
    snap: &crate::engine::DeckSnap,
    readout: &Readout,
    col: Color32,
    size: f32,
    mut on_jog: impl FnMut(f32, bool),
) -> PlatterHit {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), Sense::click_and_drag());
    let c = rect.center();
    let r = size * 0.47;
    let p = ui.painter();
    p.circle_filled(c, r, t.bg_darker);
    p.circle_stroke(c, r, st(if readout.warning { 3.0 } else { 2.0 },
        if readout.warning { t.red } else { t.marker(col, t.bg_darker) }));
    for i in 6..16 {
        p.circle_stroke(c, r * i as f32 / 18.0, st(0.5, t.muted.gamma_multiply(0.35)));
    }
    p.circle_filled(c, r * 0.38, t.tint(col, 0.28));
    let bpm = platter_bpm(snap);
    if !readout.status.is_empty() {
        p.text(c + Vec2::new(0.0, -r * 0.68), egui::Align2::CENTER_CENTER,
            readout.status, FontId::proportional(t.text_size(10.0)), if readout.warning { t.red } else { t.fg });
    }
    p.text(
        c + Vec2::new(0.0, -t.text_size(18.0) * 0.65),
        egui::Align2::CENTER_CENTER,
        format!("{bpm:.1}"),
        FontId::proportional(t.text_size((size * 0.11).clamp(12.0, 18.0))),
        t.fg_bright,
    );
    p.text(
        c + Vec2::new(0.0, t.text_size(14.0) * 0.65),
        egui::Align2::CENTER_CENTER,
        &readout.text,
        FontId::monospace(t.text_size((size * 0.08).clamp(10.0, 14.0))),
        if readout.warning { t.red } else { t.accent },
    );
    let angle = if !t.reduced_motion && snap.frames > 1.0 {
        (snap.pos / snap.frames) as f32 * std::f32::consts::TAU * 18.0
    } else {
        0.0
    };
    let dir = Vec2::angled(angle);
    p.line_segment([c + dir * 10.0, c + dir * (r - 4.0)], st(2.0, t.accent));
    if snap.playing {
        p.circle_filled(c + dir * (r - 7.0), 3.0, t.green);
    }
    let dragging = resp.dragged_by(PointerButton::Primary) && resp.drag_delta().length() > 1.5;
    if dragging {
        if let Some(pos) = resp.interact_pointer_pos() {
            let v = pos - c;
            if v.length() > 4.0 {
                let tangent = Vec2::new(-v.y, v.x).normalized();
                on_jog(resp.drag_delta().dot(tangent) / r, true);
            }
        }
    } else if resp.drag_stopped() {
        on_jog(0.0, false);
    }
    accessibility::button(ui, &resp, "Platter play or pause", Some(snap.playing));
    accessibility::status(ui, &resp, &format!("{}; {:.1} playing BPM; {}. {}. Left/Right arrows jog", snap.title, bpm, readout.status, readout.tooltip));
    let action = accessibility::actions(ui, &resp, &["Play or pause", "Cue", "Unload", "Jog backward", "Jog forward"]);
    help::describe(ui, &resp, HelpControl::Platter);
    if matches!(action, Some(3 | 4)) { on_jog(if action == Some(3) { -0.05 } else { 0.05 }, true); on_jog(0.0, false); }
    if resp.has_focus() {
        ui.memory_mut(|memory| memory.set_focus_lock_filter(resp.id, egui::EventFilter { horizontal_arrows: true, ..Default::default() }));
        let delta = ui.input_mut(|input| {
            let delta = input.num_presses(Key::ArrowRight) as f32 - input.num_presses(Key::ArrowLeft) as f32;
            input.consume_key(egui::Modifiers::NONE, Key::ArrowRight);
            input.consume_key(egui::Modifiers::NONE, Key::ArrowLeft);
            delta
        });
        if delta != 0.0 { on_jog(delta * 0.05, true); on_jog(0.0, false); }
    }
    let shift = ui.input(|i| i.modifiers.shift);
    help::rich_tooltip(&resp, || vec![readout.tooltip.clone(), help::tooltip_text(HelpControl::Platter)]);
    PlatterHit {
        click: (resp.clicked() && !shift) || action == Some(0),
        right_click: resp.secondary_clicked() || action == Some(1),
        shift_click: (resp.clicked() && shift) || action == Some(2),
    }
}

fn vertical_wave(
    ui: &mut Ui,
    t: &Theme,
    snap: &crate::engine::DeckSnap,
    col: Color32,
    w: f32,
    h: f32,
    mut on_seek: impl FnMut(f32),
) {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, h), Sense::click_and_drag());
    let p = ui.painter_at(rect);
    p.rect_filled(rect, 4.0, t.bg_darker);
    let position = if snap.frames > 0.0 { ((snap.pos / snap.frames) as f32 * snap.duration).clamp(0.0, snap.duration) } else { 0.0 };
    if let Some(seconds) = accessibility::numeric(ui, &resp, "Waveform position", position, 0.0, snap.duration.max(0.0), 0.1, " s") {
        if snap.duration > 0.0 { on_seek(seconds / snap.duration); }
    }
    help::annotate(ui, &resp, HelpControl::Seek);
    if snap.peaks.is_empty() || snap.frames < 1.0 || snap.duration <= 0.01 {
        p.text(rect.center(), egui::Align2::CENTER_CENTER, "wave", FontId::proportional(t.text_size(10.0)), t.muted);
        return;
    }
    let pos_s = (snap.pos / snap.frames) as f32 * snap.duration;
    let half = 3.5;
    let start_s = (pos_s - half).max(0.0);
    let end_s = (start_s + half * 2.0).min(snap.duration);
    let n = snap.peaks.len() as f32;
    let a = ((start_s / snap.duration) * n) as usize;
    let b = (((end_s / snap.duration) * n) as usize).max(a + 1).min(snap.peaks.len());
    let mid = rect.center().x;
    let hw = rect.width() * 0.46;
    let span = (b - a).max(1);
    let step = (span as f32 / rect.height().max(1.0)).ceil().max(1.0) as usize;
    let mut k = 0usize;
    let mut i = a;
    while i < b {
        let y = rect.top() + k as f32 * step as f32 / span as f32 * rect.height();
        let pk = snap.peaks[i];
        p.line_segment([Pos2::new(mid, y), Pos2::new(mid - pk[0] * hw, y)], st(1.0, t.waveform(t.red, 0.9)));
        p.line_segment([Pos2::new(mid, y), Pos2::new(mid + pk[1] * hw, y)], st(1.0, t.waveform(t.green, 0.85)));
        p.line_segment([Pos2::new(mid - pk[2] * hw * 0.35, y), Pos2::new(mid + pk[2] * hw * 0.35, y)], st(1.0, t.waveform(t.marker(col, t.bg_darker), 0.5)));
        i += step;
        k += 1;
    }
    grid_editor::paint_vertical_grid(&p, rect, t, snap.grid, start_s as f64, end_s as f64);
    for (i, position) in snap.hotcue_positions.iter().enumerate() {
        let Some(seconds) = position.map(|p| (p / snap.frames) as f32 * snap.duration) else { continue };
        if !(start_s..=end_s).contains(&seconds) { continue; }
        let y = rect.top() + (seconds - start_s) / (end_s - start_s).max(0.001) * rect.height();
        let color = cue_editor::color(t, snap.cue_styles[i], i);
        p.line_segment([Pos2::new(rect.left(), y), Pos2::new(rect.right(), y)], st(1.0, color));
        let name = cue_editor::short_name(snap.cue_styles[i].name.as_str(), 8);
        p.text(Pos2::new(rect.left() + 2.0, y), egui::Align2::LEFT_BOTTOM,
            format!("{} {}", i + 1, name), FontId::proportional(t.text_size(9.0)), t.fg);
    }
    accessibility::status(ui, &resp, &snap.hotcue_positions.iter().enumerate()
        .filter(|(_, pos)| pos.is_some()).map(|(i, _)| cue_editor::description(snap, i)).collect::<Vec<_>>().join("; "));
    let play_y = rect.top() + ((pos_s - start_s) / (end_s - start_s).max(0.001)) * rect.height();
    p.line_segment([Pos2::new(rect.left(), play_y), Pos2::new(rect.right(), play_y)], st(1.6, t.accent));
    if resp.clicked() || resp.dragged() {
        if let Some(pos) = resp.interact_pointer_pos() {
            let u = ((pos.y - rect.top()) / rect.height()).clamp(0.0, 1.0);
            let tsec = start_s + u * (end_s - start_s);
            on_seek((tsec / snap.duration).clamp(0.0, 1.0));
        }
    }
}

fn fader(ui: &mut Ui, t: &Theme, value: f32, span: f32, meter: f32, col: Color32, width: f32, height: f32, deck: u8) -> Option<f32> {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, height), Sense::click_and_drag());
    let p = ui.painter();
    let track = Rect::from_center_size(rect.center(), Vec2::new(7.0, rect.height() - 8.0));
    let mouse=touch::register(ui,&resp,touch::Target::Pitch(deck),track,true);
    p.rect_filled(track, 3.0, t.bg_darker);
    let mh = track.height() * meter.clamp(0.0, 1.0);
    p.rect_filled(Rect::from_min_max(Pos2::new(track.right() + 2.0, track.bottom() - mh), Pos2::new(track.right() + 5.0, track.bottom())), 1.0, t.trace(t.green, t.level_contrast));
    let y = track.bottom() - value.clamp(0.0, 1.0) * track.height();
    p.rect_filled(Rect::from_center_size(Pos2::new(rect.center().x, y), Vec2::new(16.0, 7.0)), 2.0, col);
    let alternate = accessibility::numeric(ui, &resp, "Pitch", (value * 2.0 - 1.0) * span, -span, span, 0.1, "%")
        .map(|percent| (percent / span + 1.0) * 0.5);
    accessibility::status(ui, &resp, &format!("Signal activity {:.0}% (smoothed, not a peak or clipping meter)", meter.clamp(0.0, 1.0) * 100.0));
    help::annotate(ui, &resp, HelpControl::Pitch);
    if let Some(pos) = mouse.position(&resp) {
        return Some((1.0 - (pos.y - track.top()) / track.height()).clamp(0.0, 1.0));
    }
    alternate
}

fn xfader(ui: &mut Ui, t: &Theme, width: f32, height: f32, value: &mut f32) -> bool {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, height), Sense::click_and_drag());
    let mouse=touch::register(ui,&resp,touch::Target::Crossfader,Rect::from_min_max(rect.min+Vec2::new(16.0,0.0),rect.max-Vec2::new(16.0,0.0)),true);
    let p = ui.painter();
    p.rect_filled(rect, 4.0, t.bg_darker);
    p.text(rect.left_center() + Vec2::new(6.0, 0.0), egui::Align2::LEFT_CENTER, "A", FontId::proportional(t.text_size(10.0)), t.track_color(0));
    p.text(rect.right_center() - Vec2::new(6.0, 0.0), egui::Align2::RIGHT_CENTER, "B", FontId::proportional(t.text_size(10.0)), t.track_color(1));
    let x = rect.left() + 16.0 + value.clamp(0.0, 1.0) * (rect.width() - 32.0);
    p.rect_filled(Rect::from_center_size(Pos2::new(x, rect.center().y), Vec2::new(12.0, 14.0)), 2.0, t.accent);
    let alternate = accessibility::numeric(ui, &resp, "Crossfader", *value * 100.0, 0.0, 100.0, 1.0, "% B");
    help::annotate(ui, &resp, HelpControl::Crossfader);
    if let Some(pos) = mouse.position(&resp) {
            *value = ((pos.x - rect.left() - 16.0) / (rect.width() - 32.0)).clamp(0.0, 1.0);
        return true;
    }
    if let Some(next) = alternate {
        *value = next / 100.0; return true;
    }
    false
}

fn platter_bpm(snap: &crate::engine::DeckSnap) -> f32 {
    let span = match snap.pitch_range {
        1 => 0.16,
        2 => 0.50,
        _ => 0.08,
    };
    let rate = 1.0 + (snap.pitch - 0.5) * 2.0 * span;
    snap.bpm.max(0.0) * rate
}

fn eq_to_knob(g: f32) -> f32 {
    if g <= 1.0 {
        (g.max(0.0)).powf(1.0 / 1.4) * 0.5
    } else {
        0.5 + (g - 1.0) / 2.4 * 0.5
    }
}

#[cfg(test)]
mod waveform_tests;

#[cfg(test)]
mod atspi_tests;

#[cfg(test)]
mod display_tests;
