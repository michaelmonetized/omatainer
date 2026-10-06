use super::*;
use crate::engine::{
    clip_management::{
        edit::{Action, Slot},
        Properties,
    },
    midi_edit::{Ack, Outcome},
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
    source: Option<Slot>,
    destination: Option<Slot>,
    name: String,
    properties: Properties,
    bars: f32,
    path: String,
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
                .map_err(|_| "Clip worker is busy or disconnected")?;
            self.active = Some(cancel);
            self.message = "Preparing clips…".into();
            Ok(())
        })();
        if let Err(e) = result {
            self.message = e;
        }
    }
    fn select(&mut self, slot: Slot) {
        self.source = Some(slot);
        if let Some(clip) = self.preview.as_ref().and_then(|p| p.clip(slot)) {
            self.name = clip.name.clone();
            self.properties = clip.properties;
            self.bars = clip.bars;
        }
    }
    fn refresh(&mut self, engine: &Engine) {
        self.start(engine, Job::Inspect);
    }
    fn apply(&mut self, engine: &Engine, action: Action) {
        if let Some(preview) = self.preview.clone() {
            self.start(engine, |work| Job::Apply {
                preview,
                action,
                work,
            });
        }
    }
    fn poll(&mut self, engine: &Engine) {
        if let Some(event) = self.worker.as_ref().and_then(|w| w.events.try_recv().ok()) {
            let cancelled = self
                .active
                .take()
                .is_some_and(|c| c.load(Ordering::Acquire));
            match event {
                Event::Preview(preview) if !cancelled => {
                    let initial = Slot::at(
                        preview.captured.state.session.as_ref().unwrap(),
                        preview.captured.state.selected_track,
                        preview.captured.state.selected_scene,
                    )
                    .ok();
                    let slot = self
                        .source
                        .filter(|s| preview.clip(*s).is_some())
                        .or(initial);
                    self.destination = self
                        .destination
                        .filter(|s| preview.clip(*s).is_some())
                        .or_else(|| {
                            preview
                                .slots
                                .iter()
                                .find(|(s, _)| {
                                    Some(*s) != slot
                                        && preview.clip(*s).is_some_and(|c| {
                                            c.kind == crate::engine::ClipKind::Empty
                                        })
                                })
                                .map(|(s, _)| *s)
                        });
                    self.preview = Some(preview);
                    if let Some(slot) = slot {
                        self.select(slot);
                    }
                    self.message="Choose a clip and an empty destination. Stop affected clips and recording before edits; audition uses the destination track’s instrument and mixer.".into();
                }
                Event::Preview(_) => self.message = "Clip inspection cancelled".into(),
                Event::Queued(ack) => {
                    if cancelled {
                        ack.cancel();
                    }
                    self.pending = Some(ack);
                }
                Event::Saved(message) => self.message = message,
                Event::Failed(error) => self.message = error,
            }
        }
        if let Some(ack) = &self.pending {
            let state = ack.state();
            if state != Outcome::Pending {
                self.pending = None;
                self.message = match state {
                    Outcome::Applied => {
                        "Clip operation applied. Undo restores complete content and settings."
                    }
                    Outcome::Cancelled => "Clip operation cancelled.",
                    _ => "Clip edit rejected. Stop affected clips, refresh and review again.",
                }
                .into();
                if state == Outcome::Applied {
                    self.refresh(engine);
                }
            }
        }
    }
    fn audition(&mut self, engine: &Engine, slot: Slot) {
        if let Some(preview) = &self.preview {
            if preview.captured.checkpoint != engine.undo.checkpoint() {
                self.message = "Project changed; refresh clips before audition.".into();
                return;
            }
            let Some((track, scene)) = preview.indices(slot) else {
                return;
            };
            let command = Command::SessionControl(crate::engine::session::Scoped {
                track: Some((track, slot.track)),
                scene: Some((scene, slot.scene)),
                command: Box::new(Command::FireClip {
                    track: track as u8,
                    scene: scene as u16,
                    looping: false,
                }),
            });
            if let Err(e) = engine.cmd.send(command) {
                self.message = e.to_string();
            }
        }
    }
    fn stop(&mut self, engine: &Engine, slot: Slot) {
        if let Some((track, _)) = self.preview.as_ref().and_then(|p| p.indices(slot)) {
            let command = Command::SessionControl(crate::engine::session::Scoped {
                track: Some((track, slot.track)),
                scene: None,
                command: Box::new(Command::StopTrack { track: track as u8 }),
            });
            if let Err(e) = engine.cmd.send(command) {
                self.message = e.to_string();
            }
        }
    }
    fn save(&mut self, engine: &Engine) {
        if let (Some(preview), Some(source)) = (self.preview.clone(), self.source) {
            let path = PathBuf::from(self.path.trim());
            self.start(engine, |work| Job::Write {
                preview,
                source,
                path,
                work,
            });
        }
    }
    fn read(&mut self, engine: &Engine) {
        if let Some(preview) = self.preview.clone() {
            let path = PathBuf::from(self.path.trim());
            self.start(engine, |work| Job::Read {
                preview,
                path,
                work,
            });
        }
    }
}
fn choice(ui: &mut Ui, label: &str, value: &mut Option<Slot>, preview: &Preview) {
    ui.label(label);
    let response = egui::ComboBox::from_id_salt(label)
        .selected_text(
            value
                .and_then(|s| preview.slots.iter().find(|(slot, _)| *slot == s))
                .map_or("Choose slot", |(_, name)| name.as_str()),
        )
        .show_ui(ui, |ui| {
            let height = ui.text_style_height(&egui::TextStyle::Button)
                + ui.spacing().button_padding.y * 2.0;
            egui::ScrollArea::vertical()
                .id_salt((label, "clip-slots"))
                .max_height(256.0)
                .show_rows(ui, height, preview.slots.len(), |ui, range| {
                    for index in range {
                        let (slot, name) = &preview.slots[index];
                        if ui.selectable_value(value, Some(*slot), name).clicked() {
                            ui.close();
                        }
                    }
                });
        })
        .response;
    ui.ctx()
        .accesskit_node_builder(response.id, |n| n.set_label(label));
}
fn text(ui: &mut Ui, label: &str, value: &mut String) {
    ui.horizontal(|ui| {
        ui.label(label);
        let r = ui.text_edit_singleline(value);
        ui.ctx()
            .accesskit_node_builder(r.id, |n| n.set_label(label));
    });
}
impl App {
    pub(super) fn open_clip_manager(&mut self, track: usize, scene: usize) {
        self.clip_manager.open = true;
        if let Some(slot) = self
            .snap
            .session
            .as_ref()
            .and_then(|l| Slot::at(l, track, scene).ok())
        {
            self.clip_manager.source = Some(slot);
        }
        self.clip_manager.refresh(&self.engine);
    }
    pub(super) fn poll_clip_manager(&mut self) {
        self.clip_manager.poll(&self.engine);
    }
    pub(super) fn clip_manager_ui(&mut self, ctx: &egui::Context) {
        if !self.clip_manager.open {
            return;
        }
        let mut editor = std::mem::take(&mut self.clip_manager);
        let mut open = true;
        let mut audio = None;
        egui::Window::new("Session clips").open(&mut open).default_width(860.0).vscroll(true).max_height(self.theme.window_height(ctx)).show(ctx,|ui|{
            keyboard::block_for_dialog(ctx);ui.label(&editor.message);
            if editor.busy(){ui.spinner();if ui.button("Cancel clip operation").clicked(){editor.cancel();}}
            ui.push_id("session-clip-manager-controls",|ui|ui.add_enabled_ui(!editor.busy(),|ui|{
                if ui.button("Refresh clips").clicked(){editor.refresh(&self.engine);}
                let Some(preview)=editor.preview.clone()else{return;};let old=editor.source;
                choice(ui,"Source slot",&mut editor.source,&preview);
                if old!=editor.source{if let Some(s)=editor.source{editor.select(s);}}
                choice(ui,"Destination slot",&mut editor.destination,&preview);
                let Some(source)=editor.source else{return;};let Some(clip)=preview.clip(source)else{return;};
                let empty=clip.kind==crate::engine::ClipKind::Empty;
                let destination=editor.destination.filter(|s|*s!=source&&preview.clip(*s).is_some_and(|c|c.kind==crate::engine::ClipKind::Empty));
                text(ui,"Clip name",&mut editor.name);
                ui.horizontal(|ui|{
                    let r=ui.checkbox(&mut editor.properties.disabled,"Disable clip");accessibility::button(ui,&r,"Disable clip",Some(editor.properties.disabled));
                    let mut custom=editor.properties.color.is_some();if ui.checkbox(&mut custom,"Custom clip color").changed(){editor.properties.color=custom.then_some([255,128,32]);}
                    if let Some(color)=&mut editor.properties.color{ui.color_edit_button_srgb(color);}
                });
                if let Some(color)=&mut editor.properties.color{ui.horizontal(|ui|{for (channel,label) in ["Clip color red","Clip color green","Clip color blue"].into_iter().enumerate(){let mut value=f32::from(color[channel]);let r=ui.add(egui::Slider::new(&mut value,0.0..=255.0).text(label));if let Some(next)=accessibility::numeric(ui,&r,label,value,0.0,255.0,1.0,""){value=next;}color[channel]=value.round()as u8;}});}
                let r=ui.add(egui::Slider::new(&mut editor.bars,0.25..=256.0).text("New MIDI clip bars"));
                if let Some(v)=accessibility::numeric(ui,&r,"New MIDI clip bars",editor.bars,0.25,256.0,0.25," bars"){editor.bars=v;}
                let key=|key,modifiers|ctx.input_mut(|i|i.consume_key(modifiers,key));
                let apply=key(egui::Key::Enter,egui::Modifiers::CTRL);
                let duplicate=key(egui::Key::D,egui::Modifiers::CTRL);
                let moving=key(egui::Key::M,egui::Modifiers::CTRL);
                let delete=key(egui::Key::Backspace,egui::Modifiers::CTRL|egui::Modifiers::SHIFT);
                let create=key(egui::Key::N,egui::Modifiers::CTRL);
                ui.horizontal_wrapped(|ui|{
                    if (ui.add_enabled(!empty,egui::Button::new("Apply clip properties")).clicked()||apply)&&!empty{editor.apply(&self.engine,Action::Metadata{target:source,name:editor.name.clone(),properties:editor.properties});}
                    if (ui.add_enabled(empty,egui::Button::new("Create MIDI clip")).clicked()||create)&&empty{editor.apply(&self.engine,Action::CreateMidi{target:source,name:editor.name.clone(),bars:editor.bars,properties:editor.properties});}
                    if ui.button("Edit or import audio").clicked(){audio=preview.indices(source);}
                    if (ui.add_enabled(!empty&&destination.is_some(),egui::Button::new("Duplicate clip")).clicked()||duplicate)&&!empty{if let Some(destination)=destination{editor.apply(&self.engine,Action::Copy{source,destination});}}
                    if (ui.add_enabled(!empty&&destination.is_some(),egui::Button::new("Move clip")).clicked()||moving)&&!empty{if let Some(destination)=destination{editor.apply(&self.engine,Action::Move{source,destination});editor.source=Some(destination);}}
                    if (ui.add_enabled(!empty,egui::Button::new("Delete clip")).clicked()||delete)&&!empty{editor.apply(&self.engine,Action::Delete{target:source});}
                });
                ui.label("Ctrl+Enter: properties · Ctrl+N: MIDI clip · Ctrl+D: duplicate · Ctrl+M: move · Ctrl+Shift+Backspace: delete");
                ui.label("MIDI content plays the chosen track’s instrument and MIDI routes. Audio retains its source, gain, trim, reverse and loop settings on any destination track. Existing destinations are preserved.");
                ui.horizontal(|ui|{
                    if ui.add_enabled(!empty&&!clip.properties.disabled,egui::Button::new("Audition source")).clicked(){editor.audition(&self.engine,source);}
                    if ui.button("Stop source track").clicked(){editor.stop(&self.engine,source);}
                    if let Some(slot)=editor.destination{
                        if ui.add_enabled(preview.clip(slot).is_some_and(|c|c.kind!=crate::engine::ClipKind::Empty&&!c.properties.disabled),egui::Button::new("Audition destination")).clicked(){editor.audition(&self.engine,slot);}
                        if ui.button("Stop destination track").clicked(){editor.stop(&self.engine,slot);}
                    }
                });
                ui.separator();text(ui,"Clip preset path",&mut editor.path);
                let save=key(egui::Key::S,egui::Modifiers::CTRL|egui::Modifiers::SHIFT);
                let read=key(egui::Key::O,egui::Modifiers::CTRL);
                ui.horizontal(|ui|{
                    if (ui.add_enabled(!empty&&!editor.path.trim().is_empty(),egui::Button::new("Save clip preset")).clicked()||save)&&!empty&&!editor.path.trim().is_empty(){editor.save(&self.engine);}
                    if (ui.add_enabled(!editor.path.trim().is_empty(),egui::Button::new("Inspect clip preset")).clicked()||read)&&!editor.path.trim().is_empty(){editor.read(&self.engine);}
                    if ui.add_enabled(destination.is_some()&&preview.preset.is_some(),egui::Button::new("Insert preset in destination")).clicked(){if let(Some(target),Some(preset))=(destination,preview.preset.as_ref()){editor.apply(&self.engine,Action::Insert{target,preset:(**preset).clone()});}}
                });
                if let Some(preset)=&preview.preset{ui.label(format!("Reviewed preset: {} · {:?} · {} bars · {} notes · {} embedded audio source",preset.state.clip.name,preset.state.clip.kind,preset.state.clip.bars,preset.state.clip.notes.len(),preset.media.len()));}
                ui.label(".omatclip presets embed complete clip settings and up to 128 MiB of source audio. Save preserves existing files. Insert into an empty destination, then audition there. Track instruments, effects and routes come from the destination project. Ctrl+Shift+S: save · Ctrl+O: inspect.");
            }));
        });
        if !open {
            editor.cancel();
        }
        editor.open = open;
        self.clip_manager = editor;
        if let Some((track, scene)) = audio {
            self.open_audio_clip(track as u8, scene as u16);
        }
    }
}
