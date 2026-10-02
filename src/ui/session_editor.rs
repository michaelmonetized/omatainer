//! Native session management and offscreen focus navigation.
use super::*;
use crate::engine::{
    midi_edit::{Ack, Outcome},
    session::{self, Action, Axis, Structure},
};
use std::sync::atomic::{AtomicBool, Ordering};
mod worker;
use worker::{Job, Worker};

pub(super) struct Editor {
    pub open: bool,
    axis: Axis,
    draft: Option<([u64; 2], session::Id, String, usize, Option<[u8; 3]>)>,
    worker: Option<Worker>,
    active: Option<Arc<AtomicBool>>,
    pending: Option<Ack>,
    message: String,
    error: Option<String>,
    pub reveal: Option<(usize, usize)>,
}
impl Default for Editor {
    fn default() -> Self {
        Self {
            open: false,
            axis: Axis::Track,
            draft: None,
            worker: None,
            active: None,
            pending: None,
            message: String::new(),
            error: None,
            reveal: None,
        }
    }
}
impl Drop for Editor {
    fn drop(&mut self) {
        self.cancel();
    }
}
impl Editor {
    pub(super) fn select_axis(&mut self, axis: Axis) {
        self.axis = axis;
        self.draft = None;
    }
    fn busy(&self) -> bool {
        self.active.is_some() || self.pending.is_some()
    }
    fn cancel(&self) {
        if let Some(cancel) = &self.active {
            cancel.store(true, Ordering::Release);
        }
        if let Some(ack) = &self.pending {
            ack.cancel();
        }
    }
    fn poll(&mut self) {
        if let Some(worker) = &self.worker {
            match worker.events.try_recv() {
                Ok(result) => {
                    let cancelled = self
                        .active
                        .take()
                        .is_some_and(|c| c.load(Ordering::Acquire));
                    match result {
                        Ok(ack) => {
                            if cancelled {
                                ack.cancel();
                            }
                            self.pending = Some(ack);
                        }
                        Err(error) => self.error = Some(error),
                    }
                }
                Err(crossbeam_channel::TryRecvError::Disconnected) => {
                    self.active = None;
                    self.error = Some(
                        "Session editor worker disconnected; reopen the application to retry"
                            .into(),
                    );
                }
                Err(_) => {}
            }
        }
        if let Some(ack) = &self.pending {
            match ack.state() {
                Outcome::Pending => {}
                Outcome::Applied => {
                    self.pending = None;
                    self.error = None;
                    self.draft = None;
                    self.message =
                        "Session edit applied. Undo and Redo are available in History.".into();
                }
                Outcome::Cancelled => {
                    self.pending = None;
                    self.message = "Session edit cancelled before application.".into();
                }
                Outcome::Rejected => {
                    self.pending = None;
                    self.error=Some("Session edit was not applied: the target, recording, protection or history state changed, or a resource limit was reached. Review History and retry.".into());
                }
            }
        }
    }
    fn start(&mut self, engine: &Engine, job: Job, cancel: Arc<AtomicBool>) {
        if self.busy() {
            return;
        }
        let result = (|| -> Result<(), String> {
            if self.worker.is_none() {
                self.worker = Some(Worker::start(engine.project.clone(), engine.cmd.clone())?);
            }
            self.worker
                .as_ref()
                .unwrap()
                .jobs
                .try_send(job)
                .map_err(|_| "Session editor is busy; retry shortly".to_string())?;
            Ok(())
        })();
        match result {
            Ok(()) => {
                self.active = Some(cancel);
                self.error = None;
                self.message = "Preparing session edit…".into();
            }
            Err(e) => self.error = Some(e),
        }
    }
    fn structure(&mut self, engine: &Engine, layout: &session::Layout, operation: Structure) {
        match engine.cmd.performance().optional_work() {
            Ok(work) => {
                let cancel = work.cancel();
                self.start(
                    engine,
                    Job::Structure {
                        namespace: layout.namespace,
                        generation: layout.generation,
                        operation,
                        work,
                    },
                    cancel,
                );
            }
            Err(e) => self.error = Some(e.to_string()),
        }
    }
    fn metadata(&mut self, engine: &Engine, layout: &session::Layout, action: Action) {
        let cancel = Arc::new(AtomicBool::new(false));
        self.start(
            engine,
            Job::Metadata {
                layout: layout.clone(),
                epoch: engine.undo.checkpoint().epoch,
                action,
                cancel: cancel.clone(),
            },
            cancel,
        );
    }
}
fn number(ui: &mut Ui, label: &str, value: &mut usize, count: usize) -> bool {
    let response = ui.add(
        egui::DragValue::new(value)
            .range(1..=count.max(1))
            .speed(1.0)
            .prefix(format!("{label} ")),
    );
    let mut changed = response.changed();
    if let Some(next) = accessibility::numeric(
        ui,
        &response,
        label,
        *value as f32,
        1.0,
        count.max(1) as f32,
        1.0,
        "",
    ) {
        *value = next.round() as usize;
        changed = true;
    }
    changed
}
impl App {
    pub(super) fn session_toolbar(&mut self, ui: &mut Ui) {
        let Some(layout) = self.snap.session.clone() else {
            return;
        };
        ui.horizontal_wrapped(|ui| {
            if ui
                .button("Edit session")
                .help(ui, HelpControl::SessionLayout)
                .clicked()
            {
                self.session_editor.open = true;
            }
            ui.label(format!(
                "{} tracks · {} scenes",
                layout.track_order.len(),
                layout.scene_order.len()
            ));
            let mut track = layout
                .track_order
                .iter()
                .position(|slot| usize::from(*slot) == self.snap.selected_track)
                .unwrap_or(0)
                + 1;
            let mut scene = layout
                .scene_order
                .iter()
                .position(|slot| usize::from(*slot) == self.snap.selected_scene)
                .unwrap_or(0)
                + 1;
            let changed = number(ui, "Go to track", &mut track, layout.track_order.len())
                | number(ui, "Go to scene", &mut scene, layout.scene_order.len());
            if changed {
                let target = (
                    usize::from(layout.track_order[track - 1]),
                    usize::from(layout.scene_order[scene - 1]),
                );
                self.send(Command::Select {
                    track: target.0,
                    scene: target.1,
                });
                self.session_editor.reveal = Some(target);
            }
            ui.add_enabled_ui(!self.session_editor.busy(), |ui| {
                if ui
                    .button("+ MIDI track")
                    .help(ui, HelpControl::SessionLayout)
                    .clicked()
                {
                    self.session_editor.structure(
                        &self.engine,
                        &layout,
                        Structure::Track {
                            name: format!("MIDI {}", layout.track_order.len() + 1),
                            audio: false,
                            position: layout.track_order.len(),
                        },
                    );
                }
                if ui
                    .button("+ Audio track")
                    .help(ui, HelpControl::SessionLayout)
                    .clicked()
                {
                    self.session_editor.structure(
                        &self.engine,
                        &layout,
                        Structure::Track {
                            name: format!("Audio {}", layout.track_order.len() + 1),
                            audio: true,
                            position: layout.track_order.len(),
                        },
                    );
                }
                if ui
                    .button("+ Scene")
                    .help(ui, HelpControl::SessionLayout)
                    .clicked()
                {
                    self.session_editor.structure(
                        &self.engine,
                        &layout,
                        Structure::Scene {
                            name: format!("Scene {}", layout.scene_order.len() + 1),
                            position: layout.scene_order.len(),
                        },
                    );
                }
            });
            if self.session_editor.busy() && ui.button("Cancel session edit").clicked() {
                self.session_editor.cancel();
            }
        });
        if let Some(error) = &self.session_editor.error {
            ui.colored_label(self.theme.red, error);
        } else if !self.session_editor.message.is_empty() {
            ui.label(&self.session_editor.message);
        }
    }
    pub(super) fn session_editor_ui(&mut self, ctx: &egui::Context) {
        self.session_editor.poll();
        if !self.session_editor.open {
            return;
        }
        let Some(layout) = self.snap.session.clone() else {
            return;
        };
        keyboard::block_for_dialog(ctx);
        let mut open = true;
        egui::Window::new("Session editor").id(egui::Id::new("session-editor")).open(&mut open).resizable(true).default_width(560.0).show(ctx, |ui| {
            ui.label("Stable identities keep playing clips, automation and controller targets attached when reordered. Limits: 128 tracks, 512 scenes, 65,536 notes, 8,192 notes per clip, 16 MiB MIDI lanes, 256 MiB effect buffers and bounded undo storage.");
            ui.horizontal(|ui| {ui.selectable_value(&mut self.session_editor.axis,Axis::Track,"Track");ui.selectable_value(&mut self.session_editor.axis,Axis::Scene,"Scene");});
            let axis=self.session_editor.axis;
            let (slot,position,count)=match axis {
                Axis::Track => (self.snap.selected_track,layout.track_order.iter().position(|slot|usize::from(*slot)==self.snap.selected_track).unwrap_or(0),layout.track_order.len()),
                Axis::Scene => (self.snap.selected_scene,layout.scene_order.iter().position(|slot|usize::from(*slot)==self.snap.selected_scene).unwrap_or(0),layout.scene_order.len()),
            };
            let item=match axis {Axis::Track=>&layout.tracks[slot],Axis::Scene=>&layout.scenes[slot]};
            let label=if axis==Axis::Track {"track"} else {"scene"};
            if self.session_editor.draft.as_ref().is_none_or(|(namespace,id,..)| *namespace!=layout.namespace || *id!=item.id) {
                self.session_editor.draft=Some((layout.namespace,item.id,item.name.clone(),position+1,item.color));
            }
            let (_,id,name,new_position,color)=self.session_editor.draft.as_mut().unwrap();
            let id=*id;
            ui.label(format!("Selected {label} {}: {}",position+1,item.name));
            let response=ui.add(egui::TextEdit::singleline(name).char_limit(session::MAX_NAME_BYTES).desired_width(380.0));
            response.widget_info(||egui::WidgetInfo::labeled(egui::WidgetType::TextEdit,response.enabled(),&format!("Session {label} name")));
            number(ui,"Session position",new_position,count);
            let mut custom=color.is_some(); if ui.checkbox(&mut custom,"Use custom color").changed() {*color=custom.then_some([80,160,240]);}
            if let Some(rgb)=color {ui.horizontal(|ui| {
                for (channel,label) in rgb.iter_mut().zip(["Session color red","Session color green","Session color blue"]) {
                    let response=ui.add(egui::DragValue::new(channel).range(0..=255).speed(1.0).prefix(format!("{label} ")));
                    if let Some(next)=accessibility::numeric(ui,&response,label,*channel as f32,0.0,255.0,1.0,"") {*channel=next.round() as u8;}
                }
            });}
            let name=name.clone(); let new_position=*new_position; let color=*color;
            ui.add_enabled_ui(!self.session_editor.busy(), |ui| {
                ui.horizontal_wrapped(|ui| {
                    if ui.button(format!("Rename {label}")).help(ui,HelpControl::SessionLayout).clicked() {self.session_editor.metadata(&self.engine,&layout,Action::Rename {axis,id,name:name.clone()});}
                    if ui.button(format!("Reorder {label}")).help(ui,HelpControl::SessionLayout).clicked() {self.session_editor.metadata(&self.engine,&layout,Action::Move {axis,id,position:new_position-1});}
                    if ui.button(format!("Color {label}")).help(ui,HelpControl::SessionLayout).clicked() {self.session_editor.metadata(&self.engine,&layout,Action::Color {axis,id,color});}
                    if ui.button(format!("Duplicate {label}")).help(ui,HelpControl::SessionLayout).clicked() {self.session_editor.structure(&self.engine,&layout,Structure::Duplicate {axis,id,name:format!("{name} copy"),position:position+1});}
                    if ui.button(format!("Delete {label}")).help(ui,HelpControl::SessionLayout).clicked() {self.session_editor.metadata(&self.engine,&layout,Action::Delete {axis,id});}
                });
            });
            if self.session_editor.busy() {if ui.button("Cancel session edit").clicked() {self.session_editor.cancel();} ctx.request_repaint_after(std::time::Duration::from_millis(16));}
            if let Some(error)=&self.session_editor.error {ui.colored_label(self.theme.red,error);} else {ui.label(&self.session_editor.message);}
            ui.label("Choose any offscreen track or scene using Go to track and Go to scene above the grid. Those numbers follow display order; MIDI routing uses stable track slots.");
        });
        if !open {
            self.session_editor.open = false;
            self.session_editor.cancel();
        }
    }
}

/// Bound paint and accessibility work to the viewport, regardless of set dimensions.
pub(super) fn visible_range(
    min: f32,
    max: f32,
    stride: f32,
    count: usize,
) -> std::ops::Range<usize> {
    let start = (min.max(0.0) / stride).floor() as usize;
    let end = (max.max(0.0) / stride).ceil() as usize;
    start.min(count)..end.max(start).min(count)
}

#[cfg(test)]
mod tests;
