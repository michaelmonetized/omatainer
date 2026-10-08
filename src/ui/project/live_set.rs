//! Native next-set review; preflight and graph retirement stay on one worker.
use super::*;
use crate::engine::{live_set::Control, project::Prepared};
use std::time::{Duration, Instant};
#[cfg(test)]
mod tests;

pub(super) struct Panel {
    pub open: bool,
    path: String,
    fade: f64,
    discard: bool,
    busy: bool,
    cancel: Option<Arc<AtomicBool>>,
    control: Option<Arc<Control>>,
    events: Option<crossbeam_channel::Receiver<LiveEvent>>,
    message: Option<String>,
}
enum LiveEvent {
    Ready(Arc<Control>, Duration),
    Applied(PathBuf, UiState, Applied),
    Finished(Result<(), String>, bool),
}
impl Default for Panel {
    fn default() -> Self { Self { open: false, path: String::new(), fade: 2.0, discard: false, busy: false,
        cancel: None, control: None, events: None, message: None } }
}
impl Drop for Panel {
    fn drop(&mut self) { self.cancel(); }
}
impl Panel {
    pub(super) fn cancel(&self) { if let Some(cancel) = &self.cancel { cancel.store(true, Ordering::Release); } }
    pub(super) fn busy(&self) -> bool { self.busy }
}

impl super::super::App {
    fn start_live_set(&mut self) {
        if self.project.live.busy || self.project.busy() || self.project.committing() || self.guard_project_drafts() { return; }
        let Some(namespace) = self.snap.session.as_ref().map(|session| session.namespace) else { return; };
        let work = match self.engine.cmd.performance().optional_work() {
            Ok(work) => work, Err(error) => { self.project.live.message = Some(error.to_string()); return; }
        };
        let path = match std::path::absolute(PathBuf::from(self.project.live.path.trim())) {
            Ok(path) if !self.project.live.path.trim().is_empty() => path,
            _ => { self.project.live.message = Some("Enter a native project path".into()); return; }
        };
        let cancel = work.cancel();
        let ticket = match work.background(crate::background::Kind::Prepare, format!("next-set:{}", path.display()), 1024 * crate::background::MIB) {
            Ok(ticket) => ticket, Err(error) => { self.project.live.message = Some(error); return; }
        };
        let owner = self.engine.project.clone();
        let live = owner.live_sets();
        let reservation = match live.reserve() {
            Ok(reservation) => reservation,
            Err(error) => { self.project.live.message = Some(error); return; }
        };
        let (sender, receiver) = bounded(4);
        let child_cancel = cancel.clone();
        let worker = std::thread::Builder::new().name("omatainer-next-set".into()).spawn(move || {
            let started = Instant::now();
            let _running = match ticket.enter(|| child_cancel.load(Ordering::Acquire)) {
                Ok(running) => running,
                Err(error) => { drop(reservation); let _ = sender.send(LiveEvent::Finished(Err(error), false)); return; }
            };
            let prepared = (|| -> Result<(Prepared, UiState), String> {
                let limits = crate::project_file::Limits { max_metadata_bytes: 32 * 1024 * 1024,
                    max_media: 256, max_pcm_bytes: 256 * crate::background::MIB };
                let bundle = crate::project_file::load::<Document>(&path, &limits, &child_cancel).map_err(|error| error.to_string())?;
                bundle.state.validate()?;
                let state = &bundle.state.engine;
                if state.routing.as_ref().is_some_and(|model| model.input.is_some() || model.ports.iter().any(|port|
                    port.direction == crate::engine::audio::routing::model::Direction::Input)) {
                    return Err("Sets with physical input routes must be opened while stopped; current performance and saved routes are retained".into());
                }
                if state.sampler_synth.offline.is_some() || state.tracks.iter().enumerate().any(|(slot, track)| state.track_processing_required(slot)
                    && (track.synth.offline.is_some() || track.fx.iter().any(|effect| effect.offline.is_some())))
                    || state.scene_fx.iter().flatten().any(|effect| effect.offline.is_some()) {
                    return Err("Next-set preflight refused an unavailable instrument or effect".into());
                }
                if state.banks.iter().any(|bank| bank.settings.as_ref().is_some_and(|settings|
                    settings.slots.iter().zip(bank.media).any(|(slot, media)| slot.source.is_some() && media.is_none()))) {
                    return Err("Next-set preflight refused missing sampler audio".into());
                }
                if child_cancel.load(Ordering::Acquire) { return Err("Next-set preload cancelled".into()); }
                let prepared = Prepared::from_state(bundle.state.engine, bundle.media, owner.sample_rate()).map_err(|error| error.to_string())?;
                Ok((prepared, bundle.state.view))
            })();
            let (prepared, mut view) = match prepared {
                Ok(prepared) => prepared,
                Err(error) => { drop(reservation); let _ = sender.send(LiveEvent::Finished(Err(error), false)); return; }
            };
            if child_cancel.load(Ordering::Acquire) { drop(reservation); let _ = sender.send(LiveEvent::Finished(Err("Next-set preload cancelled".into()), false)); return; }
            let control = match live.stage(reservation, prepared, namespace, child_cancel.clone()) {
                Ok(control) => control,
                Err(error) => { let _ = sender.send(LiveEvent::Finished(Err(error), false)); return; }
            };
            let mut notified = false;
            let mut applied = false;
            loop {
                if control.ready() && !notified {
                    notified = true;
                    if sender.send(LiveEvent::Ready(control.clone(), started.elapsed())).is_err() { child_cancel.store(true, Ordering::Release); }
                }
                if let Some(receipt) = live.applied() {
                    applied = true;
                    if sender.send(LiveEvent::Applied(path.clone(), std::mem::take(&mut view), receipt)).is_err() { child_cancel.store(true, Ordering::Release); }
                }
                if let Some(result) = live.retire() {
                    let _ = sender.send(LiveEvent::Finished(result.map_err(|error| error.to_string()), applied));
                    break;
                }
                if !notified && started.elapsed() > Duration::from_secs(10) { child_cancel.store(true, Ordering::Release); }
                std::thread::sleep(Duration::from_millis(1));
            }
            drop(ticket);
            drop(work);
        });
        match worker {
            Ok(_) => {
                self.project.live.busy = true;
                self.project.live.events = Some(receiver);
                self.project.live.cancel = Some(cancel);
                self.project.live.control = None;
                self.project.live.discard = false;
                self.project.live.message = Some("Preflighting and preloading the next set; current mix continues".into());
            }
            Err(error) => self.project.live.message = Some(format!("Next-set worker unavailable: {error}")),
        }
    }

    pub(super) fn poll_live_set(&mut self, ctx: &egui::Context) {
        loop {
            let event = match self.project.live.events.as_ref().map(|events| events.try_recv()) {
                Some(Ok(event)) => event,
                Some(Err(crossbeam_channel::TryRecvError::Disconnected)) if self.project.live.busy => {
                    self.project.live.cancel();
                    self.project.live.message = Some("Next-set worker disconnected; cancellation requested".into());
                    break;
                }
                _ => break,
            };
            match event {
                LiveEvent::Ready(control, elapsed) => {
                    self.project.live.control = Some(control);
                    self.project.live.message = Some(format!("Next set ready after {:.3} seconds. Preview continues from its current cue position; transition uses that position.", elapsed.as_secs_f64()));
                }
                LiveEvent::Applied(path, view, applied) => {
                    for deck in 0..DECKS { if let Some(loader) = &self.loader { let _ = loader.invalidate(deck as u8); } }
                    self.install_project_view(ctx, view, applied);
                    self.project.current_path = Some(path);
                    self.project.live.message = Some("Next set committed and playing; outgoing decks and effects continue through the fade".into());
                }
                LiveEvent::Finished(result, applied) => {
                    self.project.live.busy = false;
                    self.project.live.control = None;
                    self.project.live.cancel = None;
                    self.project.live.events = None;
                    self.project.live.message = Some(match result {
                        Ok(()) => "Transition finished; outgoing graph retired on the worker".into(),
                        Err(error) if applied => format!("Next set committed; safety ended its outgoing fade: {error}"),
                        Err(error) => format!("Next set not applied; current performance preserved: {error}"),
                    });
                    break;
                }
            }
        }
    }

    pub(super) fn live_set_panel(&mut self, ctx: &egui::Context) {
        if !self.project.live.open { return; }
        let dirty = self.project_dirty();
        let mut open = true;
        let mut preload = false;
        let mut transition = false;
        egui::Window::new(tr!("Next live set")).open(&mut open).resizable(true).show(ctx, |ui| {
            ui.label(tr!("Fades retain each set's output routes. Next-set cue uses free outputs 3/4 and an unambiguous stereo main route."));
            ui.label(tr!("Preload retains one next project, up to 256 MiB of embedded audio and 256 MiB of processors. Unavailable devices and missing sampler audio are refused."));
            ui.add_enabled_ui(!self.project.live.busy, |ui| {
                let path = ui.text_edit_singleline(&mut self.project.live.path);
                path.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, "Next live set path"));
                help::annotate(ui, &path, help::Control::LiveSetPath);
                preload = ui.button(tr!("Preload next set")).help(ui, help::Control::LiveSetPreload).clicked();
            });
            if let Some(control) = &self.project.live.control {
                let ready = control.ready();
                let mut preview = control.preview.load(Ordering::Acquire);
                let cue = control.cue_available.load(Ordering::Acquire);
                if ui.add_enabled(ready && cue, egui::Checkbox::new(&mut preview, tr!("Cue next set on outputs 3/4"))).help(ui, help::Control::LiveSetCue).changed() {
                    control.preview.store(preview, Ordering::Release);
                }
                if !cue { ui.label(tr!("Cue needs four output channels, free outputs 3/4 and one stereo main route. Transition retains the saved outputs.")); }
                let priming = control.priming_seconds(self.engine.project.sample_rate());
                if priming > 0.0 { ui.label(format!("Next set needs {priming:.4} seconds of processing history. Cue it first or choose a longer fade.")); }
                let fade = ui.add(egui::DragValue::new(&mut self.project.live.fade).range(0.01..=30.0).suffix(" s"));
                fade.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::DragValue, ready, "Live-set fade seconds"));
                help::annotate(ui, &fade, help::Control::LiveSetFade);
                if dirty { ui.checkbox(&mut self.project.live.discard, tr!("Discard current unsaved edits at transition")).help(ui, help::Control::LiveSetDiscard); }
                transition = ui.add_enabled(ready && (!dirty || self.project.live.discard), egui::Button::new(tr!("Transition to next set"))).help(ui, help::Control::LiveSetTransition).clicked();
            }
            if self.project.live.busy && ui.button(tr!("Cancel next set")).help(ui, help::Control::LiveSetCancel).clicked() { self.project.live.cancel(); }
            if let Some(message) = &self.project.live.message { ui.label(message); }
        });
        self.project.live.open = open;
        if preload { self.start_live_set(); }
        if transition && !self.project.busy() && !self.project.committing() && !self.guard_project_drafts() {
            self.poll_ui_requests();
            self.poll_ui_requests();
            if let Some(control) = &self.project.live.control {
                if let Err(error) = control.transition(&self.engine.project, self.engine.project.revision(), self.project.live.fade) { self.project.live.message = Some(error); }
            }
        }
    }
}
