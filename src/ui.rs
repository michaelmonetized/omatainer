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
mod bpm;
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
mod play_time;
mod project;
mod audio_status;
mod diagnostics;
mod master_fx_status;
use load_status::{LoadState, Phase};
use crate::engine::load_receipt::{Media, Receipt};
#[cfg(test)]
mod load_status_tests;
mod keyboard;
mod shortcuts;
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
#[cfg(test)]
mod theme_reload_tests;
#[cfg(test)]
mod font_selection_tests;

pub struct App {
    diagnostics: diagnostics::Diagnostics,
    engine: Engine,
    project: project::Projects,
    theme: Theme,
    deck_selection: deck_selection::Selection,

    theme_reload: Option<crate::theme::reload::Loader>,
    theme_fonts: Option<Arc<egui::FontDefinitions>>,
    library: Arc<Vec<LibItem>>,
    library_view: LibraryView,
    library_scan: LibraryScan,
    library_metadata: library_metadata::Metadata,
    // History can change while a worker holds the immutable crate baseline.
    // Keep those small edits separate from the full library allocation.
    last_played: play_history::History,
    playback_watches: Vec<play_history::Watch>,
    published_selection: Option<Arc<Selection>>,
    published_indices: std::sync::Weak<Vec<usize>>,
    lib_filter: String,
    lib_sel: usize,
    keys_open: bool,
    midi_open: bool,
    status: String,
    loads: [Option<LoadState>; DECKS],
    submission_error: Cell<Option<crate::engine::SubmissionError>>,
    seen_submission_failures: u64,
    loader: Option<Loader>,
    snap: Snapshot,
    last_play_idx: usize,
    pad_held: [bool; 16],
    shortcut_focus: keyboard::ShortcutFocus,
    clip_gain_edit: Option<ClipGainEdit>,
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

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, engine: Engine) -> Self {
        let theme = Theme::default();
        cc.egui_ctx.set_fonts(crate::theme::reload::fallback_fonts());
        theme.apply(&cc.egui_ctx);
        let loader = Loader::start();
        let failure = loader.as_ref().err().map(|error| format!("load failed: decoder unavailable: {error}"));
        let mut app = Self::with_loader(engine, theme, loader.ok());
        if let Some(failure) = failure { app.status = failure; }
        match crate::theme::reload::Loader::start(app.theme.clone()) {
            Ok(loader) => app.theme_reload = Some(loader),
            Err(error) => eprintln!("omatainer: theme reload worker unavailable: {error}"),
        }
        app.scan_library();
        app
    }

    fn with_loader(
        engine: Engine,
        theme: Theme,
        loader: Option<Loader>,
    ) -> Self {
        let project = project::Projects::new(engine.project.clone(), engine.sr());
        let snap = engine.snapshot();
        let playback_watches = play_history::initial_watches(&engine);
        let mut app = Self {
            diagnostics: diagnostics::Diagnostics::default(),
            engine,
            project,
            theme,
            deck_selection: deck_selection::Selection::new(snap.selected_deck_request),

            theme_reload: None,
            theme_fonts: None,
            library: Arc::new(builtin_crate_items()),
            library_view: LibraryView::default(),
            library_scan: LibraryScan::default(),
            library_metadata: library_metadata::Metadata::default(),
            last_played: play_history::History::default(),
            playback_watches,
            published_selection: None,
            published_indices: std::sync::Weak::new(),
            lib_filter: String::new(),
            lib_sel: 0,
            keys_open: false,
            midi_open: false,
            status: "Q quant · pads compose · ctrl-gain = fx".into(),
            loads: std::array::from_fn(|_| None),
            submission_error: Cell::new(None),
            seen_submission_failures: 0,
            loader,
            snap,
            last_play_idx: 0,
            pad_held: [false; 16],
            shortcut_focus: keyboard::ShortcutFocus::default(),
            clip_gain_edit: None,
        };
        app.publish_library_selection();
        app.initialize_project_baseline();
        app
    }

    fn scan_library(&mut self) {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
        self.library_scan.start(
            vec![PathBuf::from(&home).join("Music"), PathBuf::from(&home).join("music")],
            self.library.clone(),
        );
    }

    fn poll_library_scan(&mut self) {
        if let Some(publication) = self.library_scan.poll() {
            self.library_metadata.stage_scan(publication, &self.library);
        }
        self.poll_library_metadata();
    }

    fn send(&self, c: Command) {
        self.submit(c);
    }

    fn submit(&self, c: Command) -> bool {
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
        match self.engine.send(c) {
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
                        let receipt = Receipt::new();
                        if !self.submit(Command::DeckLoadRequested { deck, media: Media::Builtin(stem.index()), receipt: receipt.clone() }) {
                            state.phase = Phase::Failed("Load was not accepted; media was not loaded".into());
                        } else {
                            self.watch_playback(picked.source.clone(), None, receipt.clone());
                            state.receipt = Some(receipt);
                        }
                    }
                    self.set_load_state(deck, state);
                }
                LibSource::File(path) => self.load_file(deck, path.clone(), &picked.title),
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
        self.project.local_edits = self.project.local_edits.wrapping_add(1);
        if deck as usize >= DECKS { self.status = "load failed: invalid deck".into(); return; }
        self.supersede_load(deck);
        let selection = Selection { title: name.into(), source: LibSource::File(path.clone()) };
        let mut state = LoadState::new(Some(selection), Phase::Loading);
        match self.loader.as_ref().ok_or_else(|| "decoder is unavailable".to_string())
            .and_then(|loader| loader.request(deck, path)) {
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
                    let source = state.selection.as_ref().map(|selection| &selection.source);
                    let bpm = self.library.iter().find(|item| Some(&item.source) == source
                        && item.fingerprint.is_some() && item.fingerprint == completion.fingerprint)
                        .map(|item| item.bpm.reconcile(analysis)).unwrap_or(analysis);
                    report.sample.bpm = bpm.value().unwrap_or(0.0);
                    state.bpm = Some(bpm);
                    state.metadata = completion.fingerprint.zip(source.cloned()).map(|(fingerprint, source)|
                        library_metadata::Patch { source, fingerprint, bpm, duration: decoded_duration(&report.sample) });
                    state.warning = report.diagnostics.warning();
                    let history_source = source.cloned();
                    let receipt = Receipt::new();
                    state.phase = if self.submit(Command::DeckLoadRequested {
                        deck, media: Media::Decoded { token: completion.token, audio: Arc::new(report.sample) },
                        receipt: receipt.clone(),
                    }) {
                        if let Some(source) = history_source {
                            self.watch_playback(source, completion.fingerprint, receipt.clone());
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

    fn pad_gate(&mut self, p: usize, enabled: bool, r: &egui::Response) {
        if p >= self.pad_held.len() { return; }
        let (pressed, released, down, origin) = r.ctx.input(|input| (
            input.pointer.button_pressed(PointerButton::Primary),
            input.pointer.button_released(PointerButton::Primary),
            input.pointer.button_down(PointerButton::Primary),
            input.events.iter().rev().find_map(|event| match event {
                egui::Event::PointerButton { pos, button: PointerButton::Primary, pressed: true, .. } => Some(*pos),
                _ => None,
            }),
        ));
        // Retire the captured identity even when this cell became a piano gap.
        // A release followed by another press in this frame ends the old gate.
        if self.pad_held[p] && (released || !down) {
            self.pad_held[p] = false;
            self.send(Command::SamplerPad { pad: p as u8, on: false });
        }
        // Egui's final hover/down flags can be false after a complete tap or
        // release/press handoff. Hit-test the actual press, respecting clipping,
        // disabled UI and the top interactive layer instead of final hover.
        let pressed_here = pressed && r.enabled() && origin.is_some_and(|pos|
            r.interact_rect.contains(pos) && r.ctx.layer_id_at(pos) == Some(r.layer_id));
        if enabled && pressed_here && !self.pad_held[p] {
            // Admission owns a matching release reservation. Rejected presses
            // cannot create local held state or an unmatched release.
            self.pad_held[p] = self.submit(Command::SamplerPad { pad: p as u8, on: true });
            if self.pad_held[p] && !down {
                // A complete quick tap can also arrive in one GUI frame.
                self.pad_held[p] = false;
                self.send(Command::SamplerPad { pad: p as u8, on: false });
            }
        }
    }

    #[cfg(test)]
    fn filtered(&self) -> Vec<&LibItem> {
        let q = self.lib_filter.to_lowercase();
        self.library
            .iter()
            .filter(|i| {
                q.is_empty()
                    || i.title.to_lowercase().contains(&q)
                    || i.artist.to_lowercase().contains(&q)
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
        let Some(update) = self.theme_reload.as_ref().and_then(|loader| loader.poll()) else { return };
        if !self.theme_fonts.as_ref().is_some_and(|fonts| Arc::ptr_eq(fonts, &update.fonts)) {
            ctx.set_fonts((*update.fonts).clone());
            self.theme_fonts = Some(update.fonts.clone());
        }
        self.theme = update.theme.clone();
        self.theme.apply(ctx);
        ctx.request_repaint();
    }

    fn update_frame(&mut self, ctx: &egui::Context) {
        let ui_started = Instant::now();
        #[cfg(test)]
        std::thread::sleep(self.diagnostics.ui_delay);
        self.shortcut_focus.begin_frame(ctx);
        if !self.project.committing() {
            self.poll_ui_requests();
            self.poll_library_scan();
        } else { keyboard::block_for_dialog(ctx); }
        let submissions = self.engine.cmd.stats();
        if submissions.rejected > self.seen_submission_failures {
            self.seen_submission_failures = submissions.rejected;
            self.submission_error.set(submissions.last_error);
        }
        self.poll_theme(ctx);
        if !self.project.committing() { self.poll_loads(); }
        self.snap = self.engine.snapshot();
        self.confirm_project_snapshot();
        let animating = self.snap.playing || self.snap.decks.iter().any(|d| d.playing);
        if let Some(p) = ctx.input(|i| {
            (!self.project.committing() && self.project.dialog_is_closed()).then(|| i.raw.dropped_files.iter().find_map(|f| f.path.clone())).flatten()
        }) {
            let x = ctx.input(|i| i.pointer.latest_pos().map(|p| p.x)).unwrap_or(0.0);
            let deck = if x > ctx.screen_rect().center().x { 1u8 } else { 0 };
            let name = p.file_stem().and_then(|name| name.to_str()).unwrap_or("track").to_string();
            self.load_file(deck, p, &name);
        }

        self.project_toolbar(ctx);
        self.load_status(ctx);
        self.audio_status(ctx);
        self.master_fx_status(ctx);
        let t = self.theme.clone();
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(t.bg).inner_margin(6.0))
            .show(ctx, |ui| {
                if self.project.committing() || !self.project.dialog_is_closed() { ui.disable(); }
                let h = ui.available_height();
                let gap = 4.0;
                let samp_h = 118.0;
                let crate_h = 108.0;
                let seq_row = (t.font_size + 10.0).clamp(20.0, 26.0);
                let seq_h = 26.0 + 48.0 + seq_row * SCENES as f32 + gap * (SCENES as f32 + 2.0);
                let scratch_h = (h - samp_h - crate_h - seq_h - gap * 3.0).max(200.0);
                ui.allocate_ui(Vec2::new(ui.available_width(), scratch_h), |ui| {
                    self.scratch_row(ui, &t);
                });
                ui.add_space(gap);
                ui.allocate_ui(Vec2::new(ui.available_width(), samp_h), |ui| {
                    self.sampler_row(ui, &t);
                });
                ui.add_space(gap);
                ui.allocate_ui(Vec2::new(ui.available_width(), crate_h), |ui| {
                    self.crate_row(ui, &t);
                });
                ui.add_space(gap);
                if self.snap.fx_view >= 0 {
                    self.fx_row(ui, &t);
                } else {
                    ui.allocate_ui(Vec2::new(ui.available_width(), seq_h), |ui| {
                        self.sequencer_row(ui, &t);
                    });
                }
            });

        if let Some(error) = self.submission_error.get() {
            keyboard::block_for_dialog(ctx);
            egui::Window::new("Action was not accepted")
                .collapsible(false)
                .resizable(false)
                .show(ctx, |ui| {
                    ui.label(error.to_string());
                    if ui.button("Dismiss").clicked() {
                        self.submission_error.set(None);
                    }
                });
        }
        if self.keys_open {
            egui::Window::new("keys").show(ctx, |ui| {
                shortcuts::show_help(ui);
                ui.separator();
                ui.monospace(POINTER_HELP);
            });
        }
        if self.midi_open {
            egui::Window::new("midi").show(ctx, |ui| {
                let busy = self.engine.midi.connections_busy();
                if ui.add_enabled(!busy && self.engine.midi.connections_available(), egui::Button::new("Retry / rescan MIDI")).clicked() {
                    let _ = self.engine.midi.retry_connections();
                }
                if busy {
                    ui.label("Checking MIDI connections…");
                    ctx.request_repaint_after(std::time::Duration::from_millis(50));
                } else if !self.engine.midi.connections_available() {
                    ui.label("MIDI connection worker unavailable; restart to retry.");
                }
                ui.label("Keyboard and mouse remain available. Retry checks current ports.");
                let input = self.engine.midi.input_stats();
                ui.label(format!("Input: {} received · {} queued · {} handled", input.received, input.queued, input.dispatched));
                ui.label(format!("Overload: {} coalesced · {} discarded · {} source resets", input.coalesced, input.dropped, input.resets));
                ui.label(format!("{} oversized · {} disconnected", input.oversized, input.disconnected));
                for d in &self.snap.midi {
                    ui.label(d);
                }
            });
        }
        self.diagnostics_panel(ctx);
        self.clip_gain_editor(ctx);
        // Text fields and dialogs get this frame's keys before global actions.
        self.handle_keys(ctx);
        // Resolve terminal project outcomes after this frame's Cancel/input.
        self.poll_projects(ctx);
        if animating {
            ctx.request_repaint();
        } else {
            ctx.request_repaint_after(std::time::Duration::from_millis(80));
        }
        self.publish_library_selection();
        self.diagnostics.ui_update_ns = Some(ui_started.elapsed().as_nanos().min(u64::MAX as u128) as u64);
    }
}

impl App {
    fn handle_keys(&mut self, ctx: &egui::Context) {
        if !self.shortcut_focus.globals_allowed(ctx) {
            return;
        }
        ctx.input(|i| {
            for ev in &i.events {
                if let egui::Event::Key { key, pressed: true, repeat, modifiers: mods, .. } = ev {
                    if let Some(action) = shortcuts::lookup(*key, *mods, *repeat) {
                        self.dispatch_shortcut(action);
                    }
                }
            }
        });
    }

    fn scratch_row(&mut self, ui: &mut Ui, t: &Theme) {
        self.deck_selection.begin_pointer_frame();
        ui.spacing_mut().item_spacing = Vec2::splat(4.0);
        let h = ui.available_height();
        let w = ui.available_width();
        let gap = 4.0;
        let fader_h = 28.0;
        let (sq, wave_h, side_w, mid_w) = scratch_metrics(w, h, fader_h, gap);
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
                                vertical_wave(ui, t, &snap, t.track_color(d), wave_w, wave_h, |frac| {
                                    self.send(Command::DeckSeek { deck: d as u8, frac });
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
        let panel = ui.horizontal(|ui| {
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
        });
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
            if sq_btn(ui, t, "L", snap.keylock, t.cyan, sq).on_hover_text("pitch lock").clicked() {
                self.send(Command::DeckKeylock { deck: d as u8 });
            }
            let fader_h = (h - sq * 2.0 - 8.0).max(48.0);
            if let Some(v) = fader(ui, t, snap.pitch, 1.0, 0.0, t.accent, sq, fader_h) {
                self.send(Command::DeckPitch { deck: d as u8, value: v });
            }
            let lab = ["8", "16", "50"][snap.pitch_range.min(2) as usize];
            if sq_btn(ui, t, lab, false, t.orange, sq).on_hover_text("pitch range").clicked() {
                self.send(Command::DeckPitchRange { deck: d as u8 });
            }
        });
    }

    fn cue_eq_col(&mut self, ui: &mut Ui, t: &Theme, d: usize, snap: &crate::engine::DeckSnap, col: Color32, wave_h: f32, sq: f32) {
        let cell = sq;
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing = Vec2::splat(3.0);
            ui.set_width(cell * 4.0 + 9.0);
            ui.set_min_height(wave_h);
            for row in 0..2 {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing = Vec2::splat(3.0);
                    for c in 0..4 {
                        let i = row * 4 + c;
                        let on = snap.hotcues.get(i).copied().unwrap_or(false);
                        let r = sq_btn(ui, t, &format!("{}", i + 1), on, t.track_color(i), cell);
                        if r.clicked() {
                            self.send(Command::DeckHotCue {
                                deck: d as u8,
                                pad: i as u8,
                                del: ui.input(|i| i.modifiers.shift),
                            });
                        }
                    }
                });
            }
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
                    let resp = rotary(ui, t, lab, v, c, cell + 4.0);
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
            let hit = platter(ui, t, snap, col, wave_h, |delta, touch| {
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
        });
    }

    fn btn_stack(&mut self, ui: &mut Ui, t: &Theme, d: usize, snap: &crate::engine::DeckSnap, sq: f32) {
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing = Vec2::splat(4.0);
            ui.set_width(sq);
            let q = self.snap.quantize;
            if sq_btn(ui, t, "Q", q, t.yellow, sq).on_hover_text("quantize").clicked() {
                self.send(Command::ToggleQuant);
            }
            let io = sq_btn(ui, t, "I/O", snap.loop_on, t.accent, sq).on_hover_text("loop in · right-click out");
            if io.clicked() {
                self.send(Command::DeckLoopIn { deck: d as u8 });
            }
            if io.secondary_clicked() {
                self.send(Command::DeckLoopOut { deck: d as u8 });
            }
            if sq_btn(ui, t, "×2", false, t.cyan, sq).on_hover_text("double loop").clicked() {
                self.send(Command::DeckLoopDouble { deck: d as u8 });
            }
            if sq_btn(ui, t, "½", false, t.cyan, sq).on_hover_text("halve loop").clicked() {
                self.send(Command::DeckLoopHalf { deck: d as u8 });
            }
            if sq_btn(ui, t, "↻", snap.loop_on, t.magenta, sq).on_hover_text("reloop 4 bars").clicked() {
                self.send(Command::DeckReloop { deck: d as u8 });
            }
            if sq_btn(ui, t, "⇄", snap.sync, t.green, sq).on_hover_text("match").clicked() {
                self.send(Command::DeckMatch);
            }
        });
    }

    fn sampler_row(&mut self, ui: &mut Ui, t: &Theme) {
        ui.horizontal(|ui| {
            if let Some(target) = self.snap.compose_target {
                let name = self.snap.tracks.get(target.track).map(|tr| tr.name.as_str()).unwrap_or("track");
                ui.label(RichText::new(format!("Compose armed: {} / scene {}", name, target.scene + 1)).color(t.yellow));
                if ui.button("Disarm compose").clicked() { self.send(Command::ComposeDisarm); }
            } else {
                ui.label("Compose disarmed");
                if ui.button("Arm selected cell").on_hover_text("Pads write to this cell; shift-click a sequencer cell to choose another target").clicked() {
                    self.send(Command::ComposeArm { track: self.snap.selected_track, scene: self.snap.selected_scene });
                }
            }
            if self.snap.recording { ui.label(RichText::new("Recording pads").color(t.red)); }
        });
        let h = ui.available_height();
        let w = ui.available_width();
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing = Vec2::splat(6.0);
            ui.vertical(|ui| {
                ui.set_width(108.0);
                ui.set_min_height(h);
                egui::ComboBox::from_id_salt("bank")
                    .selected_text(
                        self.snap
                            .sampler_banks
                            .get(self.snap.sampler_bank)
                            .cloned()
                            .unwrap_or_else(|| "bank".into()),
                    )
                    .show_ui(ui, |ui| {
                        for (i, n) in self.snap.sampler_banks.iter().enumerate() {
                            if ui.selectable_label(i == self.snap.sampler_bank, n).clicked() {
                                self.send(Command::SamplerBank(i));
                            }
                        }
                    });
                egui::ComboBox::from_id_salt("inst")
                    .selected_text(self.snap.sampler_inst.label())
                    .show_ui(ui, |ui| {
                        for instrument in SamplerInstrument::ALL {
                            if ui.selectable_label(self.snap.sampler_inst == instrument, instrument.label())
                                .on_hover_text(instrument.description()).clicked() {
                                self.send(Command::SamplerInst(instrument));
                            }
                        }
                    });
                ui.horizontal(|ui| {
                    if sq_btn(ui, t, "^", false, t.accent, 26.0).clicked() {
                        self.send(Command::SamplerOct(1));
                    }
                    ui.label(RichText::new(format!("C{}", self.snap.sampler_oct)).size(11.0).color(t.fg));
                    if sq_btn(ui, t, "v", false, t.accent, 26.0).clicked() {
                        self.send(Command::SamplerOct(-1));
                    }
                });
            });
            let pad_w = ((w - 120.0 - 8.0) / 8.0).max(36.0);
            let pad_h = ((h - 4.0) / 2.0).max(32.0);
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
                            }).inner.on_hover_ui(|ui| {
                                if piano && empty {
                                    ui.label(format!("Pad {} · no piano accidental", pad.number()));
                                } else if piano {
                                    ui.label(format!("{} · pad {} · MIDI note {}", label, pad.number(), pad.midi_note(self.snap.sampler_oct)));
                                } else {
                                    ui.label(format!("Sample pad {} · bank slot {}", pad.number(), pad.index()));
                                }
                            });
                            self.pad_gate(pad.index(), !empty, &r);
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
            ui.horizontal(|ui| {
                ui.label(RichText::new("crate").size(11.0).color(t.fg_dim));
                ui.add(egui::TextEdit::singleline(&mut self.lib_filter).id_salt("crate-search").hint_text("search").desired_width(180.0));
                if ui.add_enabled(!self.library_scan.active(), egui::Button::new("scan")).clicked() {
                    self.scan_library();
                }
                if self.library_scan.active() && ui.button("cancel scan").clicked() {
                    self.library_scan.cancel();
                }
                self.deck_selectors(ui);
                if ui.button("→ A").clicked() {
                    self.load_sel(0);
                }
                if ui.button("→ B").clicked() {
                    self.load_sel(1);
                }
                ui.label(RichText::new("↓ bpm up   ↑ bpm down   same bpm → key → name").size(10.0).color(t.muted));
                ui.label(RichText::new("file keys: hints").size(10.0).color(t.muted)).on_hover_text(key_hints::HELP);
                let progress = self.library_scan.label();
                ui.add(egui::Label::new(RichText::new(&progress).size(10.0).color(t.fg_dim)).truncate())
                    .on_hover_text(progress);
            });
            let header = ["song", "bpm · source", "key", "length", "last play", "artist"];
            let col_w = [280.0, 112.0, 48.0, 64.0, 140.0, 180.0];
            ui.horizontal(|ui| {
                for (h, w) in header.iter().zip(col_w.iter()) {
                    ui.add_sized(Vec2::new(*w, 16.0), egui::Label::new(RichText::new(*h).size(10.0).color(t.fg_dim)));
                }
            });
            self.refresh_library_view();
            let focus = ui.make_persistent_id("crate-navigation");
            self.crate_navigation(ui, focus);
            let stride = 18.0 + ui.spacing().item_spacing.y;
            #[cfg(test)] { self.library_view.stats.rendered = 0; self.library_view.stats.formatted = 0; }
            let mut scroll = egui::ScrollArea::vertical().id_salt("crate-rows").auto_shrink([false, false]);
            if let Some(offset) = self.library_view.pending_offset.take() {
                scroll = scroll.vertical_scroll_offset(offset);
            }
            let output = scroll.show_rows(ui, 18.0, self.library_view.indices.len(), |ui, rows| {
                self.library_view.cells.retain(|index, _| rows.contains(index));
                for i in rows {
                    let item = &self.library[self.library_view.indices[i]];
                    let played_at = self.item_last_play(item);
                    let cells = self.library_view.cells.entry(i).or_insert_with(|| {
                        #[cfg(test)] { self.library_view.stats.formatted += 1; }
                        Cells::new(item, played_at, now)
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
                    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 18.0), Sense::click());
                    if sel { ui.painter().rect_filled(rect, 2.0, t.accent.gamma_multiply(0.18)); }
                    let mut x = rect.left();
                    for (txt, w) in [&item.title, &cells.bpm, &item.key, &cells.length, &cells.played, &item.artist].iter().zip(col_w) {
                        ui.painter().text(Pos2::new(x + 4.0, rect.center().y), egui::Align2::LEFT_CENTER,
                            *txt, FontId::proportional(11.0), if sel { t.accent } else { t.fg });
                        x += w;
                    }
                    let resp = resp.on_hover_ui(|ui| {
                        ui.label(format!("BPM: {}", item.bpm.label()));
                        ui.label(&cells.played_tooltip);
                    });
                    if resp.clicked() || resp.double_clicked() {
                        self.lib_sel = i;
                        ui.memory_mut(|memory| memory.request_focus(focus));
                    }
                    if resp.double_clicked() { self.load_sel(self.load_target() as u8); }
                }
            });
            ui.interact(output.inner_rect, focus, Sense::focusable_noninteractive());
            self.remember_crate_viewport(output.state.offset.y, output.inner_rect.height(), stride);
        });
    }

    fn sequencer_row(&mut self, ui: &mut Ui, t: &Theme) {
        let avail = ui.available_size();
        let gap = 4.0;
        let head_h = 26.0;
        let gain_h = 48.0;
        let scene_w = 28.0;
        let cols = TRACKS as f32;
        let rows = SCENES as f32;
        let col_w = ((avail.x - scene_w - gap * (cols + 1.0)) / cols).max(36.0);
        let row_h = (t.font_size + 10.0).clamp(20.0, 26.0);
        let pack_w = scene_w + gap + cols * col_w + (cols - 1.0) * gap;
        let pack_h = head_h + gap + rows * row_h + (rows - 1.0) * gap + gap + gain_h;
        ui.spacing_mut().item_spacing = Vec2::splat(gap);
        ui.with_layout(egui::Layout::left_to_right(Align::Min), |ui| {
            ui.add_space(((avail.x - pack_w) * 0.5).max(0.0));
            ui.allocate_ui(Vec2::new(pack_w, pack_h.min(avail.y)), |ui| {
                ui.with_layout(egui::Layout::top_down(Align::Min), |ui| {
                    ui.spacing_mut().item_spacing = Vec2::splat(gap);
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing = Vec2::splat(gap);
                        let _ = ui.allocate_exact_size(Vec2::new(scene_w, head_h), Sense::hover());
                        for tr in 0..TRACKS {
                            let name = self.snap.tracks.get(tr).map(|x| x.name.as_str()).unwrap_or("tr");
                            let mute = self.snap.tracks.get(tr).map(|x| x.mute).unwrap_or(false);
                            let solo = self.snap.tracks.get(tr).map(|x| x.solo).unwrap_or(false);
                            let (rect, resp) = ui.allocate_exact_size(Vec2::new(col_w, head_h), Sense::click());
                            let fill = if mute {
                                t.red.gamma_multiply(0.35)
                            } else if solo {
                                t.yellow.gamma_multiply(0.35)
                            } else {
                                t.bg_dark
                            };
                            ui.painter().rect_filled(rect, 4.0, fill);
                            ui.painter().rect_stroke(rect, 4.0, st(1.0, t.track_color(tr).gamma_multiply(0.7)), egui::StrokeKind::Inside);
                            let fs = (col_w * 0.12).clamp(10.0, 13.0);
                            ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, name, FontId::proportional(fs), t.track_color(tr));
                            if resp.clicked() {
                                self.send(Command::Mute { track: tr as u8 });
                            }
                            if resp.secondary_clicked() {
                                self.send(Command::Solo { track: tr as u8 });
                            }
                        }
                    });
                    for sc in 0..SCENES {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing = Vec2::splat(gap);
                            let on = self.snap.tracks.iter().any(|tr| tr.playing_scene == sc as i8 && !tr.clip_pending);
                            let queued = self.snap.tracks.iter().any(|tr| tr.playing_scene == sc as i8 && tr.clip_pending);
                            let (hr, hresp) = ui.allocate_exact_size(Vec2::new(scene_w, row_h), Sense::click());
                            ui.painter().rect_filled(hr, 4.0, if on { t.accent.gamma_multiply(0.45) } else { t.bg_dark });
                            ui.painter().rect_stroke(hr, 4.0, st(1.0, if on || queued { t.accent } else { t.muted.gamma_multiply(0.5) }), egui::StrokeKind::Inside);
                            ui.painter().text(hr.center(), egui::Align2::CENTER_CENTER, &format!("{}", sc + 1), FontId::proportional(11.0), t.fg);
                            if hresp.clicked() {
                                if ui.input(|i| i.modifiers.shift) {
                                    self.send(Command::AddScene { scene: sc as u8 });
                                } else if ui.input(|i| i.modifiers.ctrl) {
                                    self.send(Command::OpenFxScene(sc as u8));
                                } else {
                                    self.send(Command::ToggleScene { scene: sc as u8 });
                                }
                            }
                            if hresp.secondary_clicked() {
                                self.send(Command::RestartScene { scene: sc as u8 });
                            }
                            for tr in 0..TRACKS {
                                let clip = self.snap.tracks.get(tr).and_then(|x| x.clips.get(sc));
                                let filled = clip.map(|c| c.kind != 0).unwrap_or(false);
                                let queued = self.snap.tracks.get(tr).is_some_and(|x| x.playing_scene == sc as i8 && x.clip_pending);
                                let playing = self.snap.tracks.get(tr).is_some_and(|x| x.playing_scene == sc as i8 && !x.clip_pending);
                                let looping = self.snap.tracks.get(tr).map(|x| x.clip_looping).unwrap_or(false);
                                let color = t.track_color(tr);
                                let (rect, resp) = ui.allocate_exact_size(Vec2::new(col_w, row_h), Sense::click());
                                let fill = if playing {
                                    color.gamma_multiply(0.55)
                                } else if filled {
                                    color.gamma_multiply(0.22)
                                } else {
                                    t.bg_dark
                                };
                                ui.painter().rect_filled(rect, 4.0, fill);
                                ui.painter().rect_stroke(
                                    rect,
                                    4.0,
                                    st(if queued || (looping && playing) { 2.0 } else { 1.0 }, color.gamma_multiply(0.65)),
                                    egui::StrokeKind::Inside,
                                );
                                if filled {
                                    let fs = (row_h * 0.38).clamp(10.0, 13.0);
                                    let name = clip.map(|c| c.name.as_str()).unwrap_or("");
                                    let queued_name = queued.then(|| format!("{name} · queued"));
                                    let label = queued_name.as_deref().unwrap_or(name);
                                    ui.painter().text(
                                        rect.center(),
                                        egui::Align2::CENTER_CENTER,
                                        label,
                                        FontId::proportional(fs),
                                        t.fg,
                                    );
                                    if playing {
                                        let w = rect.width() * self.snap.tracks.get(tr).map(|x| x.clip_progress).unwrap_or(0.0);
                                        ui.painter().rect_filled(Rect::from_min_size(rect.min, Vec2::new(w, 3.0)), 0.0, t.fg);
                                    }
                                }
                                if resp.clicked() {
                                    if ui.input(|i| i.modifiers.alt) {
                                        if filled {
                                            self.clip_gain_edit = Some(ClipGainEdit {
                                                track: tr as u8, scene: sc as u8,
                                                value: clip.map(|c| c.gain).unwrap_or(1.0),
                                            });
                                        }
                                    } else if ui.input(|i| i.modifiers.shift) {
                                        self.send(Command::ComposeArm { track: tr, scene: sc });
                                    } else {
                                        self.send(Command::FireClip { track: tr as u8, scene: sc as u8, looping: false });
                                    }
                                }
                                if resp.secondary_clicked() {
                                    self.send(Command::FireClip { track: tr as u8, scene: sc as u8, looping: true });
                                }
                            }
                        });
                    }
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing = Vec2::splat(gap);
                        let _ = ui.allocate_exact_size(Vec2::new(scene_w, gain_h), Sense::hover());
                        for tr in 0..TRACKS {
                            let g = self.snap.tracks.get(tr).map(|x| x.gain).unwrap_or(0.8);
                            let mute = self.snap.tracks.get(tr).map(|x| x.mute).unwrap_or(false);
                            let solo = self.snap.tracks.get(tr).map(|x| x.solo).unwrap_or(false);
                            let col = t.track_color(tr);
                            let c = if mute { t.red } else if solo { t.yellow } else { col };
                            let (cell, _) = ui.allocate_exact_size(Vec2::new(col_w, gain_h), Sense::hover());
                            let knob = (col_w.min(gain_h) - 2.0).clamp(28.0, 44.0);
                            let resp = rotary_in(ui, t, "gain", (g / 1.2).clamp(0.0, 1.0), c, knob, cell, tr);
                            if resp.changed {
                                self.send(Command::TrackGain { track: tr as u8, value: resp.value * 1.2 });
                            }
                            if resp.clicked {
                                if ui.input(|i| i.modifiers.ctrl) {
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
            });
        });
    }

    fn fx_row(&mut self, ui: &mut Ui, t: &Theme) {
        ui.horizontal(|ui| {
            let label = if self.snap.fx_view >= 100 {
                format!("scene {} fx", self.snap.fx_view - 99)
            } else {
                format!(
                    "{} fx",
                    self.snap.tracks.get(self.snap.fx_view as usize).map(|x| x.name.as_str()).unwrap_or("track")
                )
            };
            ui.label(RichText::new(label).color(t.accent).strong());
            if pill(ui, t, "back", false, t.fg).clicked() {
                self.send(Command::CloseFx);
            }
        });
        ui.horizontal_wrapped(|ui| {
            for (i, id) in FxId::all().iter().enumerate() {
                if self.snap.fx_view >= 100 && !id.supports_scene() {
                    continue;
                }
                if pill(ui, t, id.name(), false, t.cyan).clicked() {
                    self.send(Command::FxAdd(i as u8));
                }
            }
        });
        egui::ScrollArea::vertical().show(ui, |ui| {
            for (i, (name, on, mix, p)) in self.snap.fx_slots.clone().into_iter().enumerate() {
                self.fx_slot_controls(ui, t, i, &name, on, mix, p);
            }
            if self.snap.fx_slots.is_empty() {
                ui.label(RichText::new("add a device — chain runs top to bottom").color(t.muted));
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
            .then_with(|| a.title.to_lowercase().cmp(&b.title.to_lowercase()))
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


fn scratch_metrics(w: f32, h: f32, fader_h: f32, gap: f32) -> (f32, f32, f32, f32) {
    let mut wave_h = (h - fader_h - gap).max(120.0);
    let mut sq = 26.0;
    let mut side_w = 0.0;
    let mut mid_w = 0.0;
    for _ in 0..12 {
        sq = (wave_h / 7.0).clamp(22.0, 34.0);
        let cue_w = 4.0 * sq + 9.0;
        side_w = sq + gap + cue_w + gap + wave_h + gap + sq;
        mid_w = w - 2.0 * side_w - 2.0 * gap;
        if mid_w >= 120.0 {
            break;
        }
        wave_h *= 0.92;
        if wave_h < 120.0 {
            wave_h = 120.0;
            sq = (wave_h / 7.0).clamp(22.0, 34.0);
            let cue_w = 4.0 * sq + 9.0;
            side_w = sq + gap + cue_w + gap + wave_h + gap + sq;
            mid_w = (w - 2.0 * side_w - 2.0 * gap).max(96.0);
            break;
        }
    }
    (sq, wave_h, side_w, mid_w.max(96.0))
}

fn st(width: f32, color: Color32) -> Stroke {
    Stroke { width, color }
}

fn pill(ui: &mut Ui, t: &Theme, text: &str, on: bool, accent: Color32) -> egui::Response {
    let fill = if on { accent.gamma_multiply(0.35) } else { t.bg_light };
    let stroke = if on { accent } else { t.muted.gamma_multiply(0.5) };
    let galley = ui.painter().layout_no_wrap(text.to_owned(), FontId::proportional(11.0), if on { t.fg_bright } else { t.fg });
    let size = Vec2::new((galley.size().x + 12.0).max(28.0), 20.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    ui.painter().rect_filled(rect, 4.0, fill);
    ui.painter().rect_stroke(rect, 4.0, st(1.0, stroke), egui::StrokeKind::Inside);
    ui.painter().galley(Pos2::new(rect.center().x - galley.size().x * 0.5, rect.center().y - galley.size().y * 0.5), galley, t.fg);
    resp
}

fn sq_btn(ui: &mut Ui, t: &Theme, text: &str, on: bool, accent: Color32, size: f32) -> egui::Response {
    let fill = if on { accent.gamma_multiply(0.4) } else { t.bg_light };
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), Sense::click());
    ui.painter().rect_filled(rect, 4.0, fill);
    ui.painter().rect_stroke(rect, 4.0, st(1.0, if on { accent } else { t.muted.gamma_multiply(0.5) }), egui::StrokeKind::Inside);
    ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, text, FontId::proportional(11.0), t.fg);
    resp
}

fn pad_btn(ui: &mut Ui, t: &Theme, text: &str, empty: bool, col: Color32, size: Vec2) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click_and_drag());
    ui.painter().rect_filled(rect, 4.0, if empty { t.bg_darker } else { col.gamma_multiply(0.28) });
    ui.painter().rect_stroke(rect, 4.0, st(1.0, col.gamma_multiply(0.6)), egui::StrokeKind::Inside);
    if !empty {
        ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, text, FontId::proportional(12.0), t.fg);
    }
    resp
}

struct RotaryResp {
    value: f32,
    changed: bool,
    clicked: bool,
    secondary: bool,
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
) -> RotaryResp {
    let cx = cell.center().x;
    let cy = cell.center().y - 6.0;
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
        FontId::proportional(9.0),
        t.fg_dim,
    );
    let mut out = RotaryResp {
        value,
        changed: false,
        clicked: resp.clicked(),
        secondary: resp.secondary_clicked(),
    };
    if resp.dragged() {
        out.value = (value - resp.drag_delta().y * 0.01).clamp(0.0, 1.0);
        out.changed = true;
    }
    out
}

fn rotary(ui: &mut Ui, t: &Theme, label: &str, value: f32, col: Color32, size: f32) -> RotaryResp {
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
        ui.label(RichText::new(label).size(9.0).color(t.fg_dim));
        let mut out = RotaryResp {
            value,
            changed: false,
            clicked: resp.clicked(),
            secondary: resp.secondary_clicked(),
        };
        if resp.dragged() {
            out.value = (value - resp.drag_delta().y * 0.01).clamp(0.0, 1.0);
            out.changed = true;
        }
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
    col: Color32,
    size: f32,
    mut on_jog: impl FnMut(f32, bool),
) -> PlatterHit {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), Sense::click_and_drag());
    let c = rect.center();
    let r = size * 0.47;
    let p = ui.painter();
    p.circle_filled(c, r, t.bg_darker);
    p.circle_stroke(c, r, st(2.0, col.gamma_multiply(0.85)));
    for i in 6..16 {
        p.circle_stroke(c, r * i as f32 / 18.0, st(0.5, t.muted.gamma_multiply(0.35)));
    }
    p.circle_filled(c, r * 0.38, col.gamma_multiply(0.28));
    let bpm = platter_bpm(snap);
    let remain = platter_remain(snap);
    p.text(
        c + Vec2::new(0.0, -8.0),
        egui::Align2::CENTER_CENTER,
        format!("{bpm:.1}"),
        FontId::proportional((size * 0.11).clamp(12.0, 18.0)),
        t.fg_bright,
    );
    p.text(
        c + Vec2::new(0.0, 10.0),
        egui::Align2::CENTER_CENTER,
        remain,
        FontId::monospace((size * 0.08).clamp(10.0, 14.0)),
        t.accent,
    );
    let angle = if snap.frames > 1.0 {
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
    let shift = ui.input(|i| i.modifiers.shift);
    PlatterHit {
        click: resp.clicked() && !shift,
        right_click: resp.secondary_clicked(),
        shift_click: resp.clicked() && shift,
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
    if snap.peaks.is_empty() || snap.frames < 1.0 || snap.duration <= 0.01 {
        p.text(rect.center(), egui::Align2::CENTER_CENTER, "wave", FontId::proportional(10.0), t.muted);
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
        p.line_segment([Pos2::new(mid, y), Pos2::new(mid - pk[0] * hw, y)], st(1.0, t.red.gamma_multiply(0.9)));
        p.line_segment([Pos2::new(mid, y), Pos2::new(mid + pk[1] * hw, y)], st(1.0, t.green.gamma_multiply(0.85)));
        p.line_segment([Pos2::new(mid - pk[2] * hw * 0.35, y), Pos2::new(mid + pk[2] * hw * 0.35, y)], st(1.0, col.gamma_multiply(0.5)));
        i += step;
        k += 1;
    }
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

fn fader(ui: &mut Ui, t: &Theme, value: f32, max: f32, meter: f32, col: Color32, width: f32, height: f32) -> Option<f32> {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, height), Sense::click_and_drag());
    let p = ui.painter();
    let track = Rect::from_center_size(rect.center(), Vec2::new(7.0, rect.height() - 8.0));
    p.rect_filled(track, 3.0, t.bg_darker);
    let mh = track.height() * meter.clamp(0.0, 1.0);
    p.rect_filled(Rect::from_min_max(Pos2::new(track.right() + 2.0, track.bottom() - mh), Pos2::new(track.right() + 5.0, track.bottom())), 1.0, t.green);
    let y = track.bottom() - (value / max.max(0.001)).clamp(0.0, 1.0) * track.height();
    p.rect_filled(Rect::from_center_size(Pos2::new(rect.center().x, y), Vec2::new(16.0, 7.0)), 2.0, col);
    if resp.clicked() || resp.dragged() {
        if let Some(pos) = resp.interact_pointer_pos() {
            return Some((1.0 - (pos.y - track.top()) / track.height()).clamp(0.0, 1.0) * max);
        }
    }
    None
}

fn xfader(ui: &mut Ui, t: &Theme, width: f32, height: f32, value: &mut f32) -> bool {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, height), Sense::click_and_drag());
    let p = ui.painter();
    p.rect_filled(rect, 4.0, t.bg_darker);
    p.text(rect.left_center() + Vec2::new(6.0, 0.0), egui::Align2::LEFT_CENTER, "A", FontId::proportional(10.0), t.track_color(0));
    p.text(rect.right_center() - Vec2::new(6.0, 0.0), egui::Align2::RIGHT_CENTER, "B", FontId::proportional(10.0), t.track_color(1));
    let x = rect.left() + 16.0 + value.clamp(0.0, 1.0) * (rect.width() - 32.0);
    p.rect_filled(Rect::from_center_size(Pos2::new(x, rect.center().y), Vec2::new(12.0, 14.0)), 2.0, t.accent);
    if resp.clicked() || resp.dragged() {
        if let Some(pos) = resp.interact_pointer_pos() {
            *value = ((pos.x - rect.left() - 16.0) / (rect.width() - 32.0)).clamp(0.0, 1.0);
            return true;
        }
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

fn platter_remain(snap: &crate::engine::DeckSnap) -> String {
    if snap.frames < 1.0 || snap.duration <= 0.01 {
        return "—:——".into();
    }
    let left = snap.duration * (1.0 - (snap.pos / snap.frames) as f32).clamp(0.0, 1.0);
    let span = match snap.pitch_range {
        1 => 0.16,
        2 => 0.50,
        _ => 0.08,
    };
    let rate = (1.0 + (snap.pitch - 0.5) * 2.0 * span).max(0.05);
    let s = left / rate;
    let m = (s as u32) / 60;
    let sec = s % 60.0;
    format!("-{m}:{sec:04.1}")
}

fn eq_to_knob(g: f32) -> f32 {
    if g <= 1.0 {
        (g.max(0.0)).powf(1.0 / 1.4) * 0.5
    } else {
        0.5 + (g - 1.0) / 2.4 * 0.5
    }
}

const POINTER_HELP: &str = "\
platter click = play   right-click = cue   shift-click = unload
platter shows playing BPM + time remaining
buttons: Q quantize · I/O loop in / right-click out · ×2 ½ ↻ reloop ⇄ match
cues 1-8   bass/mid/treb/gain click=cut  rclick=solo
pitch L lock + 8/16/50 range
pads: hold to play the selected sample or instrument
crate: ↓ bpm up. load →A / →B
seq: scene head launch/stop, rclick restart, shift add
track head mute / rclick solo
cell click once / rclick loop / shift arm compose; Stop disarms
gain rotary click mute / rclick solo / ctrl fx chain
alt-click clip = gain for new notes/hits
";

#[cfg(test)]
mod waveform_tests;
