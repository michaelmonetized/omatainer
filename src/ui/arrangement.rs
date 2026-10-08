use super::*;
use crate::engine::{
    arrangement::{Instance, Model, Source},
    midi_edit::{Ack, Outcome},
    session::{Axis, Reference},
};
use egui::Align2;
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(test)]
mod tests;
mod worker;
mod fades;
use worker::{Event, Job, Preview, Worker};

#[derive(Default)]
pub(super) struct Editor {
    open: bool,
    worker: Option<Worker>,
    active: Option<Arc<AtomicBool>>,
    pending: Option<Ack>,
    preview: Option<Arc<Preview>>,
    model: Model,
    selected: Option<u64>,
    choice: usize,
    source_choice: Option<u64>,
    fade_partner: Option<u64>,
    fade_link_partner: Option<u64>,
    crossfade_length: f64,
    crossfade_curve: f32,
    target: Option<Reference>,
    start: f64,
    width: f64,
    snap: f64,
    range: [f64; 2],
    drag: Option<(u64, Instance)>,
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
        if let Some(c) = &self.active {
            c.store(true, Ordering::Release);
        }
        if let Some(a) = &self.pending {
            a.cancel();
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
        let result = (|| -> Result<(), String> {
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
                .map_err(|_| "Song worker is busy or disconnected")?;
            self.active = Some(cancel);
            self.message = "Preparing song on the worker…".into();
            Ok(())
        })();
        if let Err(error) = result {
            self.message = error;
        }
    }
    fn poll(&mut self, engine: &Engine) {
        if let Some(worker) = &self.worker {
            if let Ok(event) = worker.events.try_recv() {
                let cancelled = self
                    .active
                    .take()
                    .is_some_and(|c| c.load(Ordering::Acquire));
                match event {
                    Event::Preview(preview) if !cancelled => {
                        self.model = preview
                            .captured
                            .state
                            .arrangement
                            .as_deref()
                            .cloned()
                            .unwrap_or_default();
                        self.target = preview.captured.state.session.as_ref().and_then(|l| {
                            l.reference(Axis::Track, preview.captured.state.selected_track)
                        });
                        self.source_choice = self.model.sources.first().map(|s| s.id);
                        self.preview = Some(preview);
                        self.message="Place reviewed Session sources on the song, then apply with transport and recording stopped.".into();
                    }
                    Event::Preview(_) => self.message = "Song inspection cancelled".into(),
                    Event::Queued(ack) => {
                        if cancelled {
                            ack.cancel();
                        }
                        self.pending = Some(ack);
                    }
                    Event::Failed(error) => self.message = error,
                }
            }
        }
        if let Some(ack) = &self.pending {
            let state = ack.state();
            if state != Outcome::Pending {
                self.pending = None;
                self.message = match state {
                    Outcome::Applied => {
                        "Song applied. Undo restores all placements and the playback source."
                    }
                    Outcome::Cancelled => "Song edit cancelled.",
                    _ => "Song edit rejected; stop playback, refresh and review again.",
                }
                .into();
                if state == Outcome::Applied {
                    self.start(engine, Job::Inspect);
                }
            }
        }
    }
    fn add(&mut self, source: u64, track: Reference, beat: f64) -> Result<(), String> {
        if self.model.instances.len() >= crate::engine::arrangement::MAX_INSTANCES {
            return Err("Song has reached its instance limit".into());
        }
        let source = self
            .model
            .sources
            .iter()
            .find(|s| s.id == source)
            .ok_or("Choose a song source")?;
        let duration = source
            .clip
            .audio_region
            .and_then(|r| {
                self.preview
                    .as_ref()
                    .and_then(|p| source.clip.audio.and_then(|i| p.captured.media.get(i)))
                    .and_then(|a| r.prepare(a).ok())
            })
            .map_or_else(
                || {
                    source
                        .clip
                        .region
                        .map_or(f64::from(source.clip.bars) * 4.0, |r| r.end - r.start)
                },
                |p| p.duration_beats,
            );
        let fades=(source.clip.kind == crate::engine::ClipKind::Audio).then(||crate::engine::audio_clip::Fades { automatic:true,..source.clip.audio_region.map_or_else(Default::default,|r|r.fades) });
        let source = source.id;
        let id = self.model.identity()?;
        self.model.instances.push(Instance {
            id,
            source,
            track,
            start: beat,
            offset: 0.0,
            duration,
            repeating: false,
            gain: 1.0,
            fades,
            fade_link: 0,
            crossfade: None,
        });
        self.selected = Some(id);
        Ok(())
    }
    fn snap(&self, beat: f64) -> f64 {
        if self.snap > 0.0 {
            (beat / self.snap).round() * self.snap
        } else {
            beat
        }
        .clamp(0.0, 262144.0)
    }
}
fn number(ui: &mut Ui, label: &str, value: &mut f64, min: f64, max: f64, step: f64) {
    ui.horizontal(|ui| {
        ui.label(label);
        let response = ui.add(egui::DragValue::new(value).range(min..=max).speed(step));
        ui.ctx().accesskit_node_builder(response.id, |n| {
            n.set_label(label);
            n.set_numeric_value(*value);
            n.set_min_numeric_value(min);
            n.set_max_numeric_value(max);
            n.add_action(egui::accesskit::Action::SetValue);
        });
        if response.enabled() {
            ui.input(|input| {
                for action in
                    input.accesskit_action_requests(response.id, egui::accesskit::Action::SetValue)
                {
                    if let Some(egui::accesskit::ActionData::NumericValue(v)) = action.data {
                        if v.is_finite() {
                            *value = v.clamp(min, max);
                        }
                    }
                }
            });
        }
    });
}
impl App {
    pub(super) fn open_arrangement(&mut self) {
        self.arrangement.open = true;
        if self.arrangement.width == 0.0 {
            self.arrangement.width = 64.0;
            self.arrangement.snap = 1.0;
            self.arrangement.range = [0.0, 64.0];
        }
        self.arrangement.start(&self.engine, Job::Inspect);
    }
    pub(super) fn poll_arrangement(&mut self) {
        self.arrangement.poll(&self.engine);
        self.song_navigation.poll();
    }
    pub(super) fn arrangement_ui(&mut self, ctx: &egui::Context) {
        if !self.arrangement.open {
            return;
        }
        let mut editor = std::mem::take(&mut self.arrangement);
        let mut open = true;
        egui::Window::new("Arrangement timeline")
            .open(&mut open)
            .default_width(1100.0)
            .vscroll(true)
            .max_height(self.theme.window_height(ctx))
            .show(ctx, |ui| {
                keyboard::block_for_dialog(ctx);
                self.song_navigation_ui(ui,editor.start,editor.width);
                ui.label(&editor.message);
                if editor.busy() {
                    ui.spinner();
                    if ui.button("Cancel song operation").clicked() {
                        editor.cancel();
                    }
                }
                ui.add_enabled_ui(!editor.busy(), |ui| {
                    ui.horizontal(|ui| {
                        if ui.button("Refresh song").clicked() {
                            editor.start(&self.engine, Job::Inspect);
                        }
                        ui.label(if self.snap.arrangement_enabled {
                            "Playing source: Arrangement"
                        } else {
                            "Playing source: Session and decks"
                        });
                    });
                    let Some(preview) = editor.preview.clone() else {
                        return;
                    };
                    let Some(layout) = &preview.captured.state.session else {
                        return;
                    };
                    ui.horizontal(|ui| {
                        ui.checkbox(&mut editor.model.enabled, "Use Arrangement playback");
                        if ui
                            .button(if self.snap.playing {
                                "Pause song"
                            } else {
                                "Play song"
                            })
                            .clicked()
                        {
                            self.send(Command::TogglePlay);
                        }
                        if ui.button("Rewind song").clicked() {
                            self.send(Command::SongSeek(0.0));
                        }
                        if ui
                            .add_enabled(
                                !self.snap.playing
                                    && !self.snap.recording
                                    && !self.snap.decks.iter().any(|d| d.playing),
                                egui::Button::new("Apply song"),
                            )
                            .clicked()
                        {
                            let model = editor.model.clone();
                            let preview = preview.clone();
                            editor.start(&self.engine, |work| Job::Apply {
                                preview,
                                model,
                                work,
                            });
                        }
                    });
                    ui.horizontal(|ui| {
                        number(
                            ui,
                            "First visible beat",
                            &mut editor.start,
                            0.0,
                            262143.0,
                            1.0,
                        );
                        number(ui, "Visible beats", &mut editor.width, 0.25, 262144.0, 1.0);
                        let snap_selector = egui::ComboBox::from_label("Song snap")
                            .selected_text(format!("Snap {} beats", editor.snap))
                            .show_ui(ui, |ui| {
                                for n in [0.0, 0.25, 0.5, 1.0, 4.0, 16.0] {
                                    ui.selectable_value(
                                        &mut editor.snap,
                                        n,
                                        if n == 0.0 {
                                            "Snap off".into()
                                        } else {
                                            format!("{n} beats")
                                        },
                                    );
                                }
                            });
                        ui.ctx()
                            .accesskit_node_builder(snap_selector.response.id, |n| {
                                n.set_label("Song snap")
                            });
                        if ui.button("Follow playhead").clicked() {
                            editor.start = (self.snap.beat - editor.width * 0.25).max(0.0);
                        }
                    });
                    ui.horizontal(|ui| {
                        number(ui, "Range start", &mut editor.range[0], 0.0, 262143.0, 1.0);
                        number(ui, "Range end", &mut editor.range[1], 0.0, 262144.0, 1.0);
                        if ui.button("Show selected range").clicked()
                            && editor.range[1] > editor.range[0]
                        {
                            editor.start = editor.range[0];
                            editor.width = editor.range[1] - editor.range[0];
                        }
                        if ui.button("Seek range start").clicked() {
                            self.send(Command::SongSeek(editor.range[0]));
                        }
                    });
                    overview(ui, &mut editor, &self.theme, self.snap.beat);
                    ui.horizontal_wrapped(|ui| {
                        let source_selector = egui::ComboBox::from_label("Session source")
                            .selected_text(
                                preview
                                    .choices
                                    .get(editor.choice)
                                    .map_or("Choose Session clip", |c| c.2.as_str()),
                            )
                            .show_ui(ui, |ui| {
                                for (index, choice) in preview.choices.iter().enumerate() {
                                    ui.selectable_value(&mut editor.choice, index, &choice.2);
                                }
                            });
                        ui.ctx()
                            .accesskit_node_builder(source_selector.response.id, |n| {
                                n.set_label("Session source")
                            });
                        if ui.button("Retain Session source").clicked() {
                            let result = (|| -> Result<(), String> {
                                if editor.model.sources.len()
                                    >= crate::engine::arrangement::MAX_SOURCES
                                {
                                    return Err("Song source limit reached".into());
                                }
                                let (track, scene, _) = preview
                                    .choices
                                    .get(editor.choice)
                                    .ok_or("Choose a nonempty Session clip")?;
                                let clip =
                                    preview.captured.state.tracks[*track].clips[*scene].clone();
                                let id = editor.model.identity()?;
                                editor.model.sources.push(Source {
                                    id,
                                    clip,
                                    audio_clock: None,
                                });
                                editor.source_choice = Some(id);
                                Ok(())
                            })();
                            if let Err(error) = result {
                                editor.message = error;
                            }
                        }
                        let shared_selector = egui::ComboBox::from_label("Shared source")
                            .selected_text(
                                editor
                                    .model
                                    .sources
                                    .iter()
                                    .find(|s| Some(s.id) == editor.source_choice)
                                    .map_or("Choose shared source", |s| s.clip.name.as_str()),
                            )
                            .show_ui(ui, |ui| {
                                for source in &editor.model.sources {
                                    ui.selectable_value(
                                        &mut editor.source_choice,
                                        Some(source.id),
                                        format!("{} · source {}", source.clip.name, source.id),
                                    );
                                }
                            });
                        ui.ctx()
                            .accesskit_node_builder(shared_selector.response.id, |n| {
                                n.set_label("Shared source")
                            });
                        let track_selector = egui::ComboBox::from_label("Destination track")
                            .selected_text(
                                editor
                                    .target
                                    .and_then(|r| layout.tracks.iter().find(|t| t.id == r.id))
                                    .map_or("Choose track", |t| t.name.as_str()),
                            )
                            .show_ui(ui, |ui| {
                                for &slot in &layout.track_order {
                                    let slot = slot as usize;
                                    ui.selectable_value(
                                        &mut editor.target,
                                        layout.reference(Axis::Track, slot),
                                        &layout.tracks[slot].name,
                                    );
                                }
                            });
                        ui.ctx()
                            .accesskit_node_builder(track_selector.response.id, |n| {
                                n.set_label("Destination track")
                            });
                        if ui.button("Place source at range start").clicked() {
                            if let (Some(source), Some(track)) =
                                (editor.source_choice, editor.target)
                            {
                                let start = editor.snap(editor.range[0]);
                                if let Err(error) = editor.add(source, track, start) {
                                    editor.message = error;
                                }
                            }
                        }
                    });
                    waveform::legend(ui, &self.theme);
                    egui::ScrollArea::vertical()
                        .max_height(440.0)
                        .show(ui, |ui| {
                            for &slot in &layout.track_order {
                                let slot = slot as usize;
                                let Some(track) = layout.reference(Axis::Track, slot) else {
                                    continue;
                                };
                                ui.label(&layout.tracks[slot].name);
                                timeline_row(
                                    ui,
                                    &mut editor,
                                    track,
                                    &preview,
                                    &self.theme,
                                    self.snap.beat,
                                );
                            }
                        });
                    let instance_label = |instance: &Instance| {
                        let name = editor
                            .model
                            .sources
                            .iter()
                            .find(|s| s.id == instance.source)
                            .map_or("Missing source", |s| s.clip.name.as_str());
                        let track = layout
                            .resolve(Axis::Track, instance.track.id)
                            .filter(|slot| layout.resolves(Axis::Track, *slot, instance.track))
                            .map_or("Missing track", |slot| layout.tracks[slot].name.as_str());
                        format!(
                            "{} · {} · beat {} · {}",
                            instance.id, name, instance.start, track
                        )
                    };
                    let instance_selector = egui::ComboBox::from_label("Song instance")
                        .selected_text(
                            editor
                                .selected
                                .and_then(|id| editor.model.instances.iter().find(|i| i.id == id))
                                .map_or_else(|| "Choose instance".into(), instance_label),
                        )
                        .show_ui(ui, |ui| {
                            for instance in &editor.model.instances {
                                ui.selectable_value(
                                    &mut editor.selected,
                                    Some(instance.id),
                                    instance_label(instance),
                                );
                            }
                        });
                    ui.ctx()
                        .accesskit_node_builder(instance_selector.response.id, |n| {
                            n.set_label("Song instance")
                        });
                    if let Some(index) = editor
                        .selected
                        .and_then(|id| editor.model.instances.iter().position(|i| i.id == id))
                    {
                        let mut instance = editor.model.instances[index];
                        ui.label(format!(
                            "Selected instance {} · shared source {}",
                            instance.id, instance.source
                        ));
                        ui.horizontal_wrapped(|ui| {
                            number(
                                ui,
                                "Instance start beat",
                                &mut instance.start,
                                0.0,
                                262143.0,
                                0.25,
                            );
                            number(
                                ui,
                                "Source offset beats",
                                &mut instance.offset,
                                0.0,
                                262143.0,
                                0.25,
                            );
                            number(
                                ui,
                                "Instance duration beats",
                                &mut instance.duration,
                                0.000001,
                                262144.0,
                                0.25,
                            );
                            ui.checkbox(&mut instance.repeating, "Repeat source");
                            ui.add(
                                egui::Slider::new(&mut instance.gain, 0.0..=1.5)
                                    .text("Instance gain"),
                            );
                        });
                        if ui
                            .add_enabled(
                                editor.target.is_some(),
                                egui::Button::new("Move instance to destination track"),
                            )
                            .clicked()
                        {
                            instance.track = editor.target.unwrap();
                        }
                        if instance != editor.model.instances[index] {
                            if instance.fade_link != 0 || instance.crossfade.is_some() || editor.model.instances.iter().any(|i| i.crossfade == Some(instance.id)) {
                                if let Err(error)=editor.model.edit_fades(instance.id,instance,&preview.captured.media) { editor.message=error; }
                            } else { editor.model.instances[index]=instance; }
                        }
                        fades::controls(ui,&mut editor,&preview,instance.id);
                        ui.horizontal(|ui| {
                            if ui.button("Copy instance").clicked() {
                                if editor.model.instances.len()
                                    >= crate::engine::arrangement::MAX_INSTANCES
                                {
                                    editor.message = "Song has reached its instance limit".into();
                                } else if let Ok(id) = editor.model.identity() {
                                    let mut copy = instance;
                                    copy.id = id;
                                    copy.fade_link = 0;
                                    copy.crossfade = None;
                                    copy.start = editor.snap(instance.start + instance.duration);
                                    editor.model.instances.push(copy);
                                    editor.selected = Some(id);
                                }
                            }
                            if ui.button("Delete instance").clicked() {
                                editor.model.remove_instance(instance.id);
                                editor.selected = None;
                            }
                            if ui.button("Discard unused sources").clicked() {
                                editor.model.sources.retain(|s| {
                                    editor.model.instances.iter().any(|i| i.source == s.id)
                                });
                            }
                        });
                    }
                });
            });
        if !open {
            editor.cancel();
        }
        editor.open = open;
        if !open { self.song_navigation.cancel(); }
        self.arrangement = editor;
    }
}
fn overview(ui: &mut Ui, editor: &mut Editor, theme: &Theme, playhead: f64) {
    let end = editor
        .model
        .instances
        .iter()
        .map(|i| i.start + i.duration)
        .fold(8192.0_f64, f64::max)
        .max(editor.start + editor.width);
    let (rect, response) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), 34.0),
        Sense::click_and_drag(),
    );
    let p = ui.painter();
    p.rect_filled(rect, 2.0, theme.bg);
    for i in &editor.model.instances {
        let x = rect.left() + rect.width() * (i.start / end) as f32;
        let w = (rect.width() * (i.duration / end) as f32).max(1.0);
        p.rect_filled(
            Rect::from_min_size(
                Pos2::new(x, rect.top() + 3.0),
                Vec2::new(w, rect.height() - 6.0),
            ),
            1.0,
            theme.fg_dim,
        );
    }
    let view = Rect::from_x_y_ranges(
        (rect.left() + rect.width() * (editor.start / end) as f32)
            ..=(rect.left() + rect.width() * ((editor.start + editor.width) / end) as f32),
        rect.y_range(),
    );
    p.rect_stroke(
        view,
        1.0,
        Stroke::new(2.0_f32, theme.fg),
        egui::StrokeKind::Inside,
    );
    let x = rect.left() + rect.width() * (playhead / end) as f32;
    p.line_segment(
        [Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())],
        Stroke::new(1.0_f32, Color32::WHITE),
    );
    if response.clicked() || response.dragged() {
        if let Some(point) = response.interact_pointer_pos() {
            editor.start = (((point.x - rect.left()) / rect.width()) as f64 * end
                - editor.width / 2.0)
                .clamp(0.0, (end - editor.width).max(0.0));
        }
    }
    response.on_hover_text("Song overview: click or drag to navigate. Exact visible and selected ranges are available above.");
}
fn timeline_row(
    ui: &mut Ui,
    editor: &mut Editor,
    track: Reference,
    preview: &Preview,
    theme: &Theme,
    playhead: f64,
) {
    let (rect, row) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), 76.0),
        Sense::click_and_drag(),
    );
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 2.0, theme.bg);
    if !ui.is_rect_visible(rect) {
        return;
    }
    let (view_start, view_width) = (editor.start, editor.width);
    let x = |beat: f64| rect.left() + ((beat - view_start) / view_width) as f32 * rect.width();
    let unit = (editor.width / 32.0).max(1.0).log2().ceil().exp2();
    let first = (editor.start / unit).floor() as u64;
    for n in first..first + 34 {
        let beat = n as f64 * unit;
        let px = x(beat);
        painter.line_segment(
            [Pos2::new(px, rect.top()), Pos2::new(px, rect.bottom())],
            Stroke::new(0.5_f32, theme.fg_dim),
        );
        painter.text(
            Pos2::new(px + 2.0, rect.top()),
            Align2::LEFT_TOP,
            format!("{beat:.0}"),
            FontId::monospace(9.0),
            theme.fg_dim,
        );
    }
    let mut selected = None;
    let mut drag_update = None;
    for instance in editor.model.instances.iter().copied().filter(|i| {
        i.track == track
            && i.start + i.duration >= editor.start
            && i.start <= editor.start + editor.width
    }) {
        let block = Rect::from_min_max(
            Pos2::new(x(instance.start), rect.top() + 14.0),
            Pos2::new(
                x(instance.start + instance.duration).max(x(instance.start) + 4.0),
                rect.bottom() - 2.0,
            ),
        );
        let response = ui.interact(
            block.intersect(rect),
            ui.id().with(("song-instance", instance.id)),
            Sense::click_and_drag(),
        );
        let source = editor
            .model
            .sources
            .iter()
            .find(|s| s.id == instance.source)
            .unwrap();
        painter.rect_filled(block, 2.0, theme.bg);
        painter.rect_stroke(
            block,
            2.0,
            Stroke::new(
                if editor.selected == Some(instance.id) {
                    2.0_f32
                } else {
                    1.0_f32
                },
                theme.fg_dim,
            ),
            egui::StrokeKind::Inside,
        );
        if let Some(audio) = source
            .clip
            .audio
            .and_then(|i| preview.captured.media.get(i))
        {
            let mut snap = crate::engine::DeckSnap::default();
            snap.spectrum = audio.spectrum.clone();
            snap.source_sample_rate = audio.sr;
            let region = source.clip.audio_region.and_then(|r| r.prepare(audio).ok());
            let length = f64::from(source.clip.bars) * 4.0;
            let area = block.intersect(rect).shrink(2.0);
            waveform::paint_spectrum(&painter, theme, area, &snap, false, |fraction| {
                let song = editor.start
                    + (f64::from(area.left() - rect.left()) + fraction * f64::from(area.width()))
                        / f64::from(rect.width())
                        * editor.width;
                let beat = instance.offset + song - instance.start;
                Some(
                    region.map_or_else(
                        || beat.rem_euclid(length) / length * audio.frames() as f64,
                        |r| {
                            r.position(beat, instance.repeating)
                                .unwrap_or(r.region.end as f64)
                        },
                    ) / f64::from(audio.sr),
                )
            });
            if let Some(fades)=instance.fades {
                super::audio_fades::paint(&painter,block.shrink(2.0),fades,instance.duration,f64::from(preview.captured.state.bpm)/60.0,false);
            } else if let Some(region) = region {
                super::audio_fades::paint_gain(&painter, block.shrink(2.0), false, |fraction| {
                    region.fade_gain(instance.offset + fraction * instance.duration, instance.repeating, f64::from(preview.captured.state.bpm) / 60.0)
                });
            }
        } else {
            midi_preview(
                &source.clip,
                instance,
                [editor.start, editor.start + editor.width],
                |pitch, a, b| {
                    let y =
                        block.bottom() - 5.0 - (f32::from(pitch) / 127.0) * (block.height() - 22.0);
                    painter.line_segment(
                        [Pos2::new(x(a), y), Pos2::new(x(b), y)],
                        Stroke::new(2.0_f32, theme.fg_dim),
                    );
                },
            );
        }
        painter.text(
            block.left_top() + Vec2::new(4.0, 3.0),
            Align2::LEFT_TOP,
            &source.clip.name,
            FontId::proportional(11.0),
            theme.fg,
        );
        response.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::WidgetType::Button,
                true,
                format!(
                    "Song instance {}: {} at beat {}",
                    instance.id, source.clip.name, instance.start
                ),
            )
        });
        if response.clicked() {
            selected = Some(instance.id);
        }
        if response.drag_started() {
            editor.drag = Some((instance.id, instance));
            selected = Some(instance.id);
        }
        if response.dragged() {
            if let Some((id, original)) = editor.drag.filter(|(id, _)| *id == instance.id) {
                let delta = f64::from(response.drag_delta().x / rect.width()) * editor.width;
                let mut next = original;
                let trimming = ui.input(|i| i.modifiers.shift);
                if trimming {
                    next.duration = editor.snap(original.duration + delta).max(0.000001);
                } else {
                    next.start = editor.snap(original.start + delta);
                }
                drag_update = Some((id, next));
            }
        }
        if response.drag_stopped() {
            editor.drag = None;
        }
    }
    if let Some(id) = selected {
        editor.selected = Some(id);
    }
    if let Some((id, next)) = drag_update {
        if next.fade_link != 0 || next.crossfade.is_some() || editor.model.instances.iter().any(|i| i.crossfade == Some(next.id)) {
            if let Err(error)=editor.model.edit_fades(id,next,&preview.captured.media) { editor.message=error; }
        } else if let Some(i)=editor.model.instances.iter_mut().find(|i|i.id==id) { *i=next; }
    }
    let px = x(playhead);
    if rect.x_range().contains(px) {
        painter.line_segment(
            [Pos2::new(px, rect.top()), Pos2::new(px, rect.bottom())],
            Stroke::new(2.0_f32, Color32::WHITE),
        );
    }
    if row.double_clicked() {
        if let (Some(source), Some(point)) = (editor.source_choice, row.interact_pointer_pos()) {
            let beat = editor.snap(
                editor.start + f64::from((point.x - rect.left()) / rect.width()) * editor.width,
            );
            if let Err(error) = editor.add(source, track, beat) {
                editor.message = error;
            }
        }
    }
}

/// Draw bounded note occurrences at their visible song positions.
/// Takes a retained source, placement, visible beat range and drawing callback; emits at most 4096 clipped note spans, including held offsets and inner-loop repeats.
fn midi_preview(
    clip: &crate::engine::project::SavedClip,
    instance: Instance,
    view: [f64; 2],
    mut emit: impl FnMut(u8, f64, f64),
) {
    let region = clip
        .region
        .unwrap_or_else(|| crate::engine::midi_edit::Region::full(clip.bars));
    let repeating = clip
        .region
        .map_or(instance.repeating, |r| r.repeating(instance.repeating));
    let intro = if repeating {
        region.loop_end - region.start
    } else {
        region.end - region.start
    };
    let low = view[0].max(instance.start);
    let high = view[1].min(instance.start + instance.duration);
    if high <= low {
        return;
    }
    let offset = instance.offset + low - instance.start;
    let limit = instance.offset + high - instance.start;
    let mut left = 4096;
    let mut draw = |pitch, a: f64, b: f64| {
        let start = (instance.start + a - instance.offset).max(low);
        let end = (instance.start + b - instance.offset).min(high);
        if end > start && left > 0 {
            emit(pitch, start, end);
            left -= 1;
        }
        left > 0
    };
    for note in clip
        .notes
        .iter()
        .filter(|n| !n.muted && n.vel > 0 && n.source_duration() > 0.0)
    {
        let position = if clip.region.is_some() {
            note.source_start()
        } else {
            note.source_start().rem_euclid(region.end - region.start)
        };
        let end = if clip.region.is_some() {
            (position + note.source_duration()).min(region.start + intro)
        } else {
            position + note.source_duration()
        };
        if position < region.start + intro
            && end > region.start
            && !draw(
                note.pitch,
                position.max(region.start) - region.start,
                end - region.start,
            )
        {
            return;
        }
        if repeating && (region.loop_start..region.loop_end).contains(&position) {
            let duration = if clip.region.is_some() {
                (position + note.source_duration()).min(region.loop_end) - position
            } else {
                note.source_duration()
            };
            let first = intro + position - region.loop_start;
            let earliest = ((offset - first - duration) / region.period())
                .ceil()
                .max(0.0) as u64;
            let last = ((limit - first) / region.period()).ceil().max(0.0) as u64;
            for cycle in earliest..last {
                let a = first + cycle as f64 * region.period();
                if !draw(note.pitch, a, a + duration) {
                    return;
                }
            }
        }
    }
}
