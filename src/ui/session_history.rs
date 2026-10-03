//! Performance sessions are renderer-confirmed and independent of project Undo.
use super::*;
use crate::performance_history::{Source, State, worker::{Job, Worker}};
use std::collections::VecDeque;

struct Identity {
    key: u64, source: LibSource, fingerprint: Option<FileFingerprint>, metadata: crate::library::Metadata,
}
#[derive(Default)]
pub(super) struct Panel {
    pub open: bool,
    pub worker: Option<Worker>,
    identities: VecDeque<Identity>,
    title: String, artist: String, export_path: String, notice: String,
    closing: bool, close_job: Option<u64>, keep_pending: bool, close_epoch: u64,
}
impl Panel {
    pub fn queue_identity(&mut self, key: u64, source: LibSource, fingerprint: Option<FileFingerprint>, metadata: crate::library::Metadata) {
        if self.identities.len() >= 8192 {
            self.notice = "History identity queue is full; additional tracks may remain unresolved.".into(); return;
        }
        self.identities.push_back(Identity { key, source, fingerprint, metadata });
    }
    fn submit(&mut self, job: Job) -> Option<u64> {
        match self.worker.as_mut().ok_or_else(|| "History worker unavailable".to_owned()).and_then(|w| w.submit(job)) {
            Ok(id) => { self.notice.clear(); Some(id) },
            Err(error) => { self.notice = error; None },
        }
    }
}
fn default_root() -> Result<PathBuf, String> {
    std::env::var_os("XDG_STATE_HOME").map(PathBuf::from).filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(PathBuf::from).filter(|p| p.is_absolute()).map(|p| p.join(".local/state")))
        .map(|p| p.join("omatainer/performance-history"))
        .ok_or_else(|| "History needs an absolute XDG_STATE_HOME or HOME directory".into())
}
fn timestamp(ns: u64) -> String { crate::localization::timestamp(ns) }
fn state_name(state: State) -> &'static str { match state { State::Active => tr!("Recording"), State::Ended => tr!("Ended"), State::Unclean => tr!("Interrupted") } }
fn label(value: &str, fallback: &str) -> String {
    let mut result = String::new();
    for c in value.trim().chars().filter(|c| !c.is_control()) {
        if result.len() + c.len_utf8() > crate::performance_history::MAX_LABEL { break; }
        result.push(c);
    }
    if result.is_empty() { fallback.into() } else { result }
}
fn source(identity: &Identity, catalog: &crate::library::Catalog) -> Option<Source> {
    let (id, version) = if let Some(track) = catalog.track_for_version(&identity.source, identity.fingerprint) {
        (track.id.0.clone(), u32::try_from(track.versions.iter().position(|v| v.fingerprint == identity.fingerprint)?).ok()?)
    } else if let LibSource::Builtin(stem) = identity.source { (format!("{:032x}", u32::from(stem.index()) + 1), 0) }
    else { return None; };
    // An automatically supplied file location is never copied into the export.
    let title = match &identity.source {
        LibSource::File(path) if identity.metadata.title == path.to_string_lossy() => path.file_name().and_then(|s| s.to_str()).unwrap_or("Track"),
        _ => &identity.metadata.title,
    };
    Some(Source::Catalog { track_id: id, version, title: label(title, "Track"), artist: label(&identity.metadata.artist, "") })
}
impl App {
    #[cfg(test)]
    pub(super) fn session_history_evidence(&self) -> serde_json::Value {
        let panel = &self.session_history;
        serde_json::json!({"open":panel.open,"notice":panel.notice,
            "pending":panel.worker.as_ref().is_some_and(Worker::pending),
            "ready":panel.worker.as_ref().is_some_and(|w|w.view().ready),
            "durable":panel.worker.as_ref().is_some_and(|w|w.view().durable),
            "active":panel.worker.as_ref().and_then(|w|w.view().active.as_ref()),
            "selected":panel.worker.as_ref().and_then(|w|w.view().selected.as_ref())})
    }
    pub(super) fn start_default_session_history(&mut self) {
        match default_root() {
            Ok(root) => self.start_session_history(root),
            Err(error) => { self.session_history.notice = error; self.session_history.open = true; },
        }
    }
    pub(super) fn start_session_history(&mut self, root: PathBuf) {
        match Worker::start(root, self.engine.performance_history.clone(), self.engine.cmd.performance().clone()) {
            Ok(worker) => self.session_history.worker = Some(worker),
            Err(error) => { self.session_history.notice = error; self.session_history.open = true; },
        }
    }
    pub(super) fn poll_session_history(&mut self) {
        let panel = &mut self.session_history;
        let Some(worker) = &mut panel.worker else { return; };
        worker.poll();
        if !worker.alive() { panel.notice = "History worker disconnected; recording and saving cannot be confirmed.".into(); return; }
        for _ in 0..panel.identities.len().min(32) {
            let identity = panel.identities.pop_front().unwrap();
            if source(&identity, &self.library_metadata.catalog).is_none_or(|source| worker.register(identity.key, source).is_err()) {
                panel.identities.push_back(identity);
            }
        }
        if panel.keep_pending && !worker.pending() {
            match worker.submit(Job::KeepWorking) {
                Ok(_) => panel.keep_pending = false,
                Err(error) => panel.notice = error,
            }
        }
    }
    pub(super) fn session_history_toolbar_text(&self) -> &'static str {
        if self.session_history.worker.as_ref().is_some_and(|w| w.view().active.is_some()) { "History · recording" } else { "History" }
    }
    pub(super) fn session_history_ui(&mut self, ctx: &egui::Context) {
        if !self.session_history.open { return; }
        let sealed = self.project_admission_sealed();
        let safe = self.engine.safe_mode();
        let output = self.engine.cmd.audio_metrics();
        let panel = &mut self.session_history;
        let mut open = panel.open;
        let mut action = None;
        let mut close_panel = false;
        egui::Window::new(tr!("Performance history")).id(egui::Id::new("Performance history")).open(&mut open).default_width(720.0).show(ctx, |ui| {
            ui.label(tr!("Tracks contributing to digital main output, measured in 10 ms windows above −90 dBFS. Cue blend is part of main output in this build. Hardware delivery and listening are not measured."));
            ui.label({ let __omatainer_args = (&(output.deadline_overruns),&(output.backend_errors),&(output.device_lost),); crate::localization::format("Output diagnostics since launch: {} late callbacks · {} backend errors · {} device losses. Backend dropped-buffer count unavailable.", &[format!("{}", __omatainer_args.0), format!("{}", __omatainer_args.1), format!("{}", __omatainer_args.2)]) });
            if !panel.notice.is_empty() { ui.label(&panel.notice); }
            if ui.button(tr!("Close history")).help(ui, HelpControl::HistoryOpen).clicked() { close_panel = true; }
            let Some(worker) = &panel.worker else { ui.label(tr!("Performance history unavailable.")); return; };
            let view = worker.view();
            ui.label(&view.message);
            let idle = !worker.pending() && worker.alive() && !panel.keep_pending;
            ui.horizontal(|ui| {
                if ui.add_enabled(idle && action.is_none() && view.ready && view.active.is_none() && !sealed && !safe, egui::Button::new(tr!("Start session"))).help(ui, HelpControl::HistoryStart).clicked() { action = Some(Job::Start); }
                ui.push_id(("end-history", &view.active), |ui| {
                    if ui.add_enabled(idle && action.is_none() && view.active.is_some(), egui::Button::new(tr!("End session"))).help(ui, HelpControl::HistoryEnd).clicked() { action = Some(Job::End); }
                });
                if ui.add_enabled(idle && action.is_none(), egui::Button::new(tr!("Retry history save"))).help(ui, HelpControl::HistoryRetry).clicked() { action = Some(Job::Retry); }
            });
            if worker.pending() { ui.label(tr!("History action pending…")); }
            ui.label(if view.durable { tr!("All published history changes saved.") } else { tr!("History save pending or unavailable; recent history may be lost on interruption.") });
            if let Some(time) = view.durable_at_ns { ui.label({ let __omatainer_args = (&(timestamp(time)),); crate::localization::format("Last history write confirmed at {}", &[format!("{}", __omatainer_args.0)]) }); }
            let sessions = egui::ScrollArea::vertical().id_salt("history-sessions").max_height(100.0).show_rows(ui, 24.0, view.sessions.len(), |ui, rows| {
                for row in rows {
                    let session = &view.sessions[row];
                    ui.push_id(&session.id, |ui| {
                        let selected = view.selected.as_ref().is_some_and(|s| s.id == session.id);
                        if ui.add_enabled(idle && action.is_none(), egui::Button::new({ let __omatainer_args = (&(state_name(session.state)),&(timestamp(session.started_ns)),&(session.entries),&(&session.id[..8]),); crate::localization::format("{} · {} · {} tracks · {}", &[format!("{}", __omatainer_args.0), format!("{}", __omatainer_args.1), format!("{}", __omatainer_args.2), format!("{}", __omatainer_args.3)]) }).selected(selected)).help(ui, HelpControl::HistorySelect).clicked() { action = Some(Job::Select(session.id.clone())); }
                    });
                }
            });
            accessibility::scrollbars(ui, "History sessions", &sessions);
            let Some(session) = &view.selected else { ui.label(tr!("Start a session to record a performance.")); return; };
            ui.push_id(&session.id, |ui| {
            ui.separator();
            ui.label({ let __omatainer_args = (&(state_name(session.state)),&(timestamp(session.started_ns)),&(session.ended_ns.map(timestamp).unwrap_or_else(|| "recording".into())),); crate::localization::format("{} · started {} · ended {}", &[format!("{}", __omatainer_args.0), format!("{}", __omatainer_args.1), format!("{}", __omatainer_args.2)]) });
            ui.label({ let __omatainer_args = (&(timestamp(session.last_confirmed_ns)),); crate::localization::format("Output confirmed through {}", &[format!("{}", __omatainer_args.0)]) });
            if session.incomplete { ui.label({ let __omatainer_args = (&(session.dropped_observation_frames),); crate::localization::format("Incomplete measurement · {} dropped per-track observation frames. Durations below cover only classified output.", &[format!("{}", __omatainer_args.0)]) }); }
            let entries = egui::ScrollArea::vertical().id_salt(("history-entries", &session.id)).max_height(220.0).show_rows(ui, 72.0, session.entries.len(), |ui, rows| {
                for row in rows {
                    let entry = &session.entries[row];
                    ui.push_id((&session.id, session.edit_revision, entry.id), |ui| { ui.allocate_ui(Vec2::new(ui.available_width(), 72.0), |ui| {
                        let origin = match entry.source { Source::External { .. } => "External assertion", Source::Unresolved => "Unresolved source", Source::Catalog { .. } => "Catalog track" };
                        let deck = entry.deck.map(|d| format!("deck {}", char::from(b'A' + d))).unwrap_or_else(|| "no measured deck".into());
                        ui.add(egui::Label::new({ let __omatainer_args = (&(entry.source.title()),&(entry.source.artist()),&(origin),&(deck),); crate::localization::format("{} · {} · {} · {}", &[format!("{}", __omatainer_args.0), format!("{}", __omatainer_args.1), format!("{}", __omatainer_args.2), format!("{}", __omatainer_args.3)]) }).truncate());
                        let uncertain: f64 = entry.rates.iter().map(|r| (r.ambiguous as f64 + r.invalid as f64) / f64::from(r.rate)).sum();
                        ui.add(egui::Label::new({ let __omatainer_args = (&(entry.measured_seconds()),&(uncertain),&(if entry.played() { "Played" } else { "Unplayed" }),&(if entry.played_override.is_some() { " (manual)" } else { " (automatic)" }),); crate::localization::format("Measured {:.3} s · uncertain {:.3} s · {}{}", &[format!("{:.3}", __omatainer_args.0), format!("{:.3}", __omatainer_args.1), format!("{}", __omatainer_args.2), format!("{}", __omatainer_args.3)]) }).truncate());
                        ui.horizontal(|ui| {
                            for (text, played) in [("Mark played", Some(true)), ("Mark unplayed", Some(false)), ("Use measured status", None)] {
                                let response = ui.add_enabled(idle && action.is_none() && !sealed, egui::Button::new(text)).help(ui, HelpControl::HistoryMark);
                                response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, idle && action.is_none() && !sealed,
                                    format!("{text} · {} · {deck} · entry {}", entry.source.title(), entry.id)));
                                if response.clicked() { action = Some(Job::Mark { session: session.id.clone(), revision: session.edit_revision, entry: entry.id, played }); }
                            }
                        });
                    }); });
                }
            });
            accessibility::scrollbars(ui, "History entries", &entries);
            ui.push_id(session.edit_revision, |ui| {
            ui.horizontal(|ui| {
                ui.label(tr!("Title")); let title = ui.text_edit_singleline(&mut panel.title).help(ui, HelpControl::HistoryExternal);
                title.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, "External track title"));
                ui.label(tr!("Artist")); let artist = ui.text_edit_singleline(&mut panel.artist).help(ui, HelpControl::HistoryExternal);
                artist.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, "External track artist"));
            });
            if ui.add_enabled(idle && action.is_none() && !sealed && !panel.title.trim().is_empty(), egui::Button::new(tr!("Add external track"))).help(ui, HelpControl::HistoryExternal).clicked() { action = Some(Job::External { session: session.id.clone(), revision: session.edit_revision, title: panel.title.trim().into(), artist: panel.artist.trim().into() }); }
            ui.horizontal(|ui| {
                ui.label(tr!("New JSON file")); let path = ui.text_edit_singleline(&mut panel.export_path).help(ui, HelpControl::HistoryExport);
                path.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, "History export destination"));
                if ui.add_enabled(idle && action.is_none() && !panel.export_path.trim().is_empty(), egui::Button::new(tr!("Export session"))).help(ui, HelpControl::HistoryExport).clicked() { action = Some(Job::Export { session: session.id.clone(), path: PathBuf::from(panel.export_path.trim()) }); }
            });
            ui.label(tr!("Manual marks and external tracks are assertions; they never alter measured duration. Export contains titles, artists and opaque catalog IDs; review these labels before sharing."));
            }); });
        });
        panel.open = open && !close_panel;
        if let Some(action) = action { panel.submit(action); }
    }
    pub(super) fn begin_session_history_close(&mut self, ctx: &egui::Context) {
        if self.session_history.worker.is_none() { self.finish_history_exit(ctx); return; }
        self.session_history.close_epoch = self.session_history.close_epoch.saturating_add(1);
        self.session_history.closing = true;
        self.session_history.close_job = None;
    }
    pub(super) fn cancel_session_history_close(&mut self) {
        if !self.session_history.closing { return; }
        self.session_history.closing = false;
        self.session_history.close_job = None;
        self.session_history.keep_pending = true;
    }
    pub(super) fn session_history_close_ui(&mut self, ctx: &egui::Context) {
        if !self.session_history.closing { return; }
        keyboard::block_for_dialog(ctx);
        let panel = &mut self.session_history;
        if panel.close_job.is_none() && panel.worker.as_ref().is_some_and(|w| !w.pending() && w.alive()) && !panel.keep_pending {
            panel.close_job = panel.submit(Job::Close);
        }
        let ready = panel.worker.as_ref().is_some_and(|w| w.alive() && !w.pending() && w.view().active.is_none()
            && (w.view().durable || !w.view().ready)
            && w.view().receipt.as_ref().is_some_and(|r| Some(r.id) == panel.close_job && r.applied));
        let mut keep = false; let mut override_close = false; let mut retry = false;
        egui::Window::new(tr!("Finishing performance history before exit")).id(egui::Id::new("Finishing performance history before exit")).collapsible(false).show(ctx, |ui| { ui.push_id(panel.close_epoch, |ui| {
            ui.label(tr!("Waiting for the actual session end and its history save. An interrupted session retains only the last saved prefix."));
            if let Some(worker) = &panel.worker { ui.label(&worker.view().message); }
            if !panel.notice.is_empty() { ui.label(&panel.notice); }
            if ui.add_enabled(panel.worker.as_ref().is_some_and(|w| !w.pending()), egui::Button::new(tr!("Retry history close"))).help(ui, HelpControl::HistoryRetry).clicked() { retry = true; }
            let response = ui.button(tr!("Keep working")).help(ui, HelpControl::HistoryKeep);
            keep = response.clicked() || response.is_pointer_button_down_on();
            override_close = ui.button(tr!("Close without confirmed history save")).help(ui, HelpControl::HistoryClose).clicked();
        }); });
        if keep { self.cancel_project_close(); }
        else if ready || override_close { self.session_history.closing = false; self.finish_history_exit(ctx); }
        else if retry { self.session_history.close_job = None; }
        ctx.request_repaint_after(std::time::Duration::from_millis(25));
    }
}

#[cfg(test)]
mod tests;
