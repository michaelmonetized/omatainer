use super::*;
use crate::engine::{
    audio_clip::{
        edit::{Document, Request},
        Region,
    },
    dsp::Sample,
    midi_edit::{Ack, Outcome},
    session::{Axis, Reference},
};
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(test)]
mod tests;
mod worker;
use worker::{Event, Job, Preview, Worker};

#[derive(Default)]
pub(super) struct Editor {
    open: bool,
    worker: Option<Worker>,
    active: Option<Arc<AtomicBool>>,
    pending: Option<Ack>,
    preview: Option<Arc<Preview>>,
    source: Option<usize>,
    region: Option<Region>,
    name: String,
    gain: f32,
    path: String,
    view_start: f64,
    view_width: f64,
    message: String,
}
impl Drop for Editor {
    fn drop(&mut self) {
        self.cancel();
    }
}
impl Editor {
    fn busy(&self) -> bool {
        self.active.is_some() || self.pending.is_some()
    }
    fn cancel(&mut self) {
        if let Some(cancel) = &self.active {
            cancel.store(true, Ordering::Release);
        }
        if let Some(ack) = &self.pending {
            ack.cancel();
        }
    }
    fn start(
        &mut self,
        engine: &Engine,
        job: impl FnOnce(crate::engine::performance::WorkPermit) -> Job,
    ) {
        if self.busy() {
            return;
        }
        let result = (|| {
            let work = engine
                .cmd
                .performance()
                .optional_work()
                .map_err(|e| e.to_string())?;
            let cancel = work.cancel();
            if self.worker.is_none() {
                self.worker = Some(Worker::start(engine.project.clone(), engine.cmd.clone())?);
            }
            self.worker
                .as_ref()
                .unwrap()
                .jobs
                .try_send(job(work))
                .map_err(|_| "Audio clip worker is busy or disconnected".to_owned())?;
            self.active = Some(cancel);
            self.message = "Preparing audio clip on the worker…".into();
            Ok::<_, String>(())
        })();
        if let Err(error) = result {
            self.message = error;
        }
    }
    fn select(&mut self, index: usize) {
        let Some(preview) = &self.preview else {
            return;
        };
        let Some(source) = preview.sources.get(index) else {
            return;
        };
        self.source = Some(index);
        self.region = Region::full(source, preview.tempo).ok();
        self.name = source.name.clone();
        self.view_start = 0.0;
        self.view_width = 1.0;
    }
    fn poll(&mut self, engine: &Engine) {
        if let Some(worker) = &self.worker {
            match worker.events.try_recv() {
                Ok(event) => {
                    let cancelled = self
                        .active
                        .take()
                        .is_some_and(|c| c.load(Ordering::Acquire));
                    match event {
                        Event::Preview(preview) if !cancelled => {
                            let source = preview.source.as_ref().and_then(|source| {
                                preview.sources.iter().position(|s| Arc::ptr_eq(s, source))
                            });
                            self.gain = preview.document.clip.gain;
                            self.name = preview.document.clip.name.clone();
                            self.region = preview.document.clip.audio_region.filter(|_| {
                                preview
                                    .source
                                    .as_ref()
                                    .zip(preview.document.source.as_ref())
                                    .is_some_and(|(a, b)| Arc::ptr_eq(a, b))
                            });
                            if self.region.is_none() {
                                self.region = preview
                                    .source
                                    .as_ref()
                                    .and_then(|s| Region::full(s, preview.tempo).ok());
                            }
                            if preview
                                .source
                                .as_ref()
                                .zip(preview.document.source.as_ref())
                                .is_none_or(|(a, b)| !Arc::ptr_eq(a, b))
                            {
                                self.name = preview
                                    .source
                                    .as_ref()
                                    .map_or_else(String::new, |s| s.name.clone());
                            }
                            self.source = source;
                            self.view_start = 0.0;
                            self.view_width = 1.0;
                            self.message = preview.warning.clone().unwrap_or_else(|| "Review source bounds and playback settings, then apply to the stopped clip.".into());
                            self.preview = Some(preview);
                        }
                        Event::Preview(_) => {
                            self.message = "Audio clip operation cancelled.".into()
                        }
                        Event::Queued(ack) => {
                            if cancelled {
                                ack.cancel();
                            }
                            self.pending = Some(ack);
                            self.message = "Waiting for the renderer to confirm the edit…".into();
                        }
                        Event::Failed(error) => self.message = error,
                    }
                }
                Err(crossbeam_channel::TryRecvError::Disconnected) => {
                    self.active = None;
                    self.message =
                        "Audio clip worker disconnected; reopen the application to retry.".into();
                }
                Err(crossbeam_channel::TryRecvError::Empty) => {}
            }
        }
        if let Some(ack) = &self.pending {
            let state = ack.state();
            if state != Outcome::Pending {
                self.pending = None;
                self.message = match state {
                    Outcome::Applied => "Audio clip applied. Undo restores its complete source and settings.",
                    Outcome::Rejected => "Audio edit rejected: the target, playback, project or history changed. Refresh and review it again.",
                    Outcome::Cancelled => "Audio edit cancelled before application; current work was preserved.",
                    Outcome::Pending => unreachable!(),
                }.into();
                if state == Outcome::Applied {
                    if let Some(preview) = &self.preview {
                        let d = &preview.document;
                        let (track, scene, target) =
                            (d.track, d.scene, (d.track_identity, d.scene_identity));
                        self.start(engine, |work| Job::Inspect {
                            track,
                            scene,
                            target,
                            work,
                        });
                    }
                }
            }
        }
    }
}
fn frame_control(ui: &mut Ui, label: &str, frame: &mut u64, frames: u64, rate: u32) {
    ui.horizontal(|ui| {
        ui.label(label);
        let response = ui.add(egui::DragValue::new(frame).range(0..=frames).speed(1.0));
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::DragValue, response.enabled(), label)
        });
        ui.ctx().accesskit_node_builder(response.id, |node| {
            node.set_label(label);
            node.set_numeric_value(*frame as f64);
            node.set_min_numeric_value(0.0);
            node.set_max_numeric_value(frames as f64);
            node.set_numeric_value_step(1.0);
            if response.enabled() {
                node.add_action(egui::accesskit::Action::SetValue);
            }
        });
        if response.enabled() {
            ui.input(|input| {
                for request in
                    input.accesskit_action_requests(response.id, egui::accesskit::Action::SetValue)
                {
                    if let Some(egui::accesskit::ActionData::NumericValue(value)) = request.data {
                        if value.is_finite() {
                            *frame = value.round().clamp(0.0, frames as f64) as u64;
                        }
                    }
                }
            });
        }
        ui.label(format!("{:.6} s", *frame as f64 / f64::from(rate)));
    });
}
fn scalar(ui: &mut Ui, label: &str, value: &mut f32, min: f32, max: f32, unit: &str) {
    let response = ui.add(
        egui::DragValue::new(value)
            .range(min..=max)
            .prefix(format!("{label} "))
            .suffix(unit),
    );
    if let Some(next) = accessibility::numeric(ui, &response, label, *value, min, max, 0.1, unit) {
        *value = next;
    }
}
impl App {
    pub(super) fn open_audio_clip(&mut self, track: u8, scene: u16) {
        self.audio_clips.open = true;
        if self.audio_clips.busy() {
            return;
        }
        let target = self.snap.session.as_ref().and_then(|l| {
            l.reference(Axis::Track, usize::from(track))
                .zip(l.reference(Axis::Scene, usize::from(scene)))
        });
        let Some(target) = target else {
            self.audio_clips.message = "Choose an active audio or empty clip.".into();
            return;
        };
        self.audio_clips.preview = None;
        self.audio_clips.source = None;
        self.audio_clips.region = None;
        self.audio_clips.start(&self.engine, |work| Job::Inspect {
            track,
            scene,
            target,
            work,
        });
    }
    pub(super) fn poll_audio_clips(&mut self) {
        self.audio_clips.poll(&self.engine);
    }
    pub(super) fn audio_clips_ui(&mut self, ctx: &egui::Context) {
        if !self.audio_clips.open {
            return;
        }
        let mut editor = std::mem::take(&mut self.audio_clips);
        let mut open = true;
        egui::Window::new(tr!("Audio clip")).open(&mut open).default_width(740.0).vscroll(true).max_height(self.theme.window_height(ctx)).show(ctx, |ui| {
            keyboard::block_for_dialog(ctx);
            ui.label(tr!("Edit a Session audio clip. Stop this clip before applying; other tracks may keep playing in Studio mode."));
            ui.label(&editor.message);
            if editor.busy() {
                ui.spinner();
                if ui.button(tr!("Cancel audio operation")).clicked() { editor.cancel(); }
            }
            ui.add_enabled_ui(!editor.busy(), |ui| {
                let Some(preview) = editor.preview.clone() else { return; };
                let document = &preview.document;
                ui.label(format!("Track {} · Scene {}", usize::from(document.track) + 1, usize::from(document.scene) + 1));
                ui.horizontal(|ui| {
                    if ui.button(tr!("Refresh audio clip")).clicked() {
                        let (track, scene, target) = (document.track, document.scene, (document.track_identity, document.scene_identity));
                        editor.start(&self.engine, |work| Job::Inspect { track, scene, target, work });
                    }
                    ui.label(tr!("Audio file path"));
                    let response = ui.text_edit_singleline(&mut editor.path);
                    ui.ctx().accesskit_node_builder(response.id, |n| n.set_label("Audio file path"));
                    if ui.add_enabled(!editor.path.trim().is_empty(), egui::Button::new(tr!("Inspect audio file"))).clicked() {
                        let path = PathBuf::from(editor.path.trim());
                        editor.start(&self.engine, |work| Job::Load { preview: preview.clone(), path, work });
                    }
                });
                ui.label(tr!("Imports decode at most 128 MiB of PCM. Existing sources are shared across clips without copying audio."));
                let selected = editor.source;
                egui::ComboBox::from_id_salt("audio-clip-source").selected_text(selected.and_then(|i| preview.sources.get(i)).map_or("Choose a source", |s| s.name.as_str())).show_ui(ui, |ui| {
                    for (index, source) in preview.sources.iter().enumerate() { ui.selectable_value(&mut editor.source, Some(index), format!("{} · {:.2} s", source.name, source.frames() as f64 / f64::from(source.sr))); }
                });
                if editor.source != selected { if let Some(index) = editor.source { editor.select(index); } }
                let Some(source) = editor.source.and_then(|i| preview.sources.get(i)) else { return; };
                let Some(mut region) = editor.region else { return; };
                ui.horizontal(|ui| { ui.label(tr!("Clip name")); let response = ui.text_edit_singleline(&mut editor.name); ui.ctx().accesskit_node_builder(response.id, |n| n.set_label("Audio clip name")); });
                if document.source.is_some() && document.clip.audio_region.is_none() { ui.label(tr!("This older clip uses whole-buffer stretching. Applying converts it to the explicit source tempo shown below.")); }
                let frames = source.frames() as u64;
                ui.horizontal(|ui| {
                    if ui.button(tr!("Zoom in audio")).clicked() { editor.view_width = (editor.view_width * 0.5).max(1.0 / frames as f64); }
                    if ui.button(tr!("Zoom out audio")).clicked() { editor.view_width = (editor.view_width * 2.0).min(1.0); }
                    if ui.button(tr!("Show complete source")).clicked() { editor.view_start = 0.0; editor.view_width = 1.0; }
                    if ui.add_enabled(region.start < region.end && region.end <= frames, egui::Button::new(tr!("Show trim"))).clicked() { editor.view_start = region.start as f64 / frames as f64; editor.view_width = (region.end - region.start) as f64 / frames as f64; }
                });
                editor.view_start = editor.view_start.clamp(0.0, 1.0 - editor.view_width);
                ui.add(egui::Slider::new(&mut editor.view_start, 0.0..=(1.0 - editor.view_width)).text(tr!("Source scroll")));
                let (rect, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 150.0), Sense::hover());
                let snap = crate::engine::DeckSnap { spectrum: source.spectrum.clone(), source_sample_rate: source.sr, ..Default::default() };
                let duration = frames as f64 / f64::from(source.sr);
                let painter = ui.painter_at(rect);
                painter.rect_filled(rect, 3.0, self.theme.bg_dark);
                if !super::waveform::paint_spectrum(&painter, &self.theme, rect, &snap, false, |f| Some((editor.view_start + f * editor.view_width) * duration)) { painter.text(rect.center(), egui::Align2::CENTER_CENTER, tr!("Source analysis unavailable"), FontId::proportional(12.0), self.theme.fg_dim); }
                for (frame, label, color) in [(region.start, "Start", Color32::WHITE), (region.end, "End", Color32::WHITE), (region.loop_start, "Loop in", Color32::YELLOW), (region.loop_end, "Loop out", Color32::YELLOW)] {
                    let f = (frame as f64 / frames as f64 - editor.view_start) / editor.view_width;
                    if (0.0..=1.0).contains(&f) { let x = rect.left() + rect.width() * f as f32; painter.line_segment([Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())], Stroke::new(1.0_f32, color)); painter.text(Pos2::new(x, rect.top()), egui::Align2::LEFT_TOP, label, FontId::proportional(11.0), color); }
                }
                response.on_hover_text(format!("Rainbow frequency waveform · {} Hz · {} channels · {} source frames", source.sr, source.ch, frames));
                super::waveform::legend(ui, &self.theme);
                frame_control(ui, "Source start frame", &mut region.start, frames, source.sr);
                frame_control(ui, "Source end frame (exclusive)", &mut region.end, frames, source.sr);
                ui.checkbox(&mut region.loop_enabled, tr!("Use inner loop on repeating launches"));
                frame_control(ui, "Loop start frame", &mut region.loop_start, frames, source.sr);
                frame_control(ui, "Loop end frame (exclusive)", &mut region.loop_end, frames, source.sr);
                ui.horizontal(|ui| {
                    ui.checkbox(&mut region.reverse, tr!("Reverse audio"));
                    scalar(ui, "Audio transpose", &mut region.transpose, -48.0, 48.0, " semitones");
                    scalar(ui, "Audio source tempo", &mut region.tempo, 30.0, 300.0, " BPM");
                    let response = ui.add(egui::Slider::new(&mut editor.gain, 0.0..=1.5).text(tr!("Audio clip gain")));
                    if let Some(next) = accessibility::numeric(ui, &response, "Audio clip gain", editor.gain, 0.0, 1.5, 0.01, "") { editor.gain = next; }
                });
                ui.label(tr!("Pitch resampling changes pitch and duration. Source tempo sets playback speed against the song clock."));
                let plan = region.prepare(source);
                match &plan { Ok(plan) => { ui.label(format!("Trim {:.6} s · musical length {:.6} beats", (region.end - region.start) as f64 / f64::from(source.sr), plan.duration_beats)); }, Err(error) => { ui.colored_label(Color32::LIGHT_RED, *error); } }
                editor.region = Some(region);
                if ui.add_enabled(plan.is_ok(), egui::Button::new(tr!("Apply audio clip"))).clicked() {
                    let (source, name, gain) = (source.clone(), editor.name.clone(), editor.gain);
                    editor.start(&self.engine, |work| Job::Apply { preview: preview.clone(), source, region, name, gain, work });
                }
                let monitor = self.snap.tracks.get(usize::from(document.track)).map(|t| t.input_monitor);
                if let Some(monitor) = monitor { ui.label(format!("Track input monitor: {monitor:?}")); }
            });
        });
        if !open {
            editor.cancel();
        }
        editor.open = open;
        self.audio_clips = editor;
    }
}
