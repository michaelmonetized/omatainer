use super::*;
use crate::{
    ableton::{Imported, Options},
    engine::performance::WorkPermit,
    project_file::{self, Bundle, Overwrite, SaveOutcome},
};
use crossbeam_channel::{bounded, Receiver};
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(test)]
mod tests;

#[derive(Default)]
pub(super) struct Panel {
    pub open: bool,
    path: String,
    destination: String,
    from: String,
    to: String,
    accepted: bool,
    reviewed_version: bool,
    device_index: Option<usize>,
    render_path: String,
    render_start: f64,
    render_body: f64,
    render_target: Option<i64>,
    render_master: bool,
    render_backup: Option<Imported>,
    native_review: bool,
    draft: Option<Imported>,
    published: Option<PathBuf>,
    events: Option<Receiver<Event>>,
    cancel: Option<Arc<AtomicBool>>,
    message: String,
}
enum Event {
    Reviewed(Imported),
    Rendered(Imported, Imported),
    Published(PathBuf, SaveOutcome),
    Failed(String),
}
impl Panel {
    pub(super) fn busy(&self) -> bool {
        self.events.is_some()
    }
    pub(super) fn cancel(&self) {
        if let Some(cancel) = &self.cancel {
            cancel.store(true, Ordering::Release);
        }
    }
    pub(super) fn poll(&mut self) {
        let Some(events) = &self.events else { return };
        let event = match events.try_recv() {
            Ok(event) => event,
            Err(crossbeam_channel::TryRecvError::Empty) => return,
            Err(_) => Event::Failed("Ableton migration worker ended without a result".into()),
        };
        let cancelled = self
            .cancel
            .as_ref()
            .is_some_and(|c| c.load(Ordering::Acquire));
        self.events = None;
        self.cancel = None;
        match event {
            Event::Reviewed(draft) if !cancelled => {
                if self.render_body <= 0. {
                    self.render_body = draft.state.arrangement.as_ref().map_or(4., |song| {
                        song.instances
                            .iter()
                            .map(|i| i.start + i.duration)
                            .fold(4., f64::max)
                    });
                }
                self.draft = Some(draft);
                self.accepted = false;
                self.message =
                    "Review every playback difference. The source Set remains intact.".into();
            }
            Event::Reviewed(_) => {
                self.message = "Ableton review cancelled; current session preserved.".into();
            }
            Event::Rendered(draft, original) if !cancelled => {
                self.draft = Some(draft);
                self.render_backup = Some(original);
                self.accepted = false;
                self.message = "Render attached. Check timing, tails and printed processing before publication; original MIDI and device state remain editable.".into();
            }
            Event::Rendered(_, _) => {
                self.message = "Render attachment cancelled; previous draft preserved.".into();
            }
            Event::Published(path, outcome) => {
                self.message=match outcome {SaveOutcome::Durable=>"Native project published. Open it when ready; unsaved-work protection still applies.".into(),SaveOutcome::CommittedButDirectorySyncFailed(error)=>format!("Native project published; directory durability unconfirmed: {error}. Do not repeat this publication.")};
                self.published = Some(path);
                self.accepted = false;
            }
            Event::Failed(error) => {
                self.message = error;
            }
        }
    }
    fn start(&mut self, engine: &Engine, publish: bool) {
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
            let (done, events) = bounded(1);
            let job = if publish {
                let draft = self.draft.as_ref().ok_or("Review a Live Set first")?;
                if !self.accepted {
                    return Err("Acknowledge the playback differences before publishing".into());
                }
                let path =
                    std::path::absolute(self.destination.trim()).map_err(|e| e.to_string())?;
                if path.extension().is_none_or(|e| e != "omatainer") {
                    return Err("Choose a new .omatainer destination".into());
                }
                let draft = draft.clone();
                Box::new(move |work: WorkPermit| -> Result<Event, String> {
                    let cancel = work.cancel();
                    crate::ableton::process::verify(&draft, &cancel)?;
                    let document = project::Document {
                        engine: draft.state,
                        view: Default::default(),
                        mapping_schema: project::FACTORY_MAPPING_SCHEMA,
                    };
                    let result = project_file::save(
                        &path,
                        &Bundle {
                            state: document,
                            media: draft.media,
                        },
                        Overwrite::Never,
                        &Default::default(),
                        &cancel,
                    )
                    .map_err(|e| e.to_string())?;
                    Ok(Event::Published(path, result))
                }) as Box<dyn FnOnce(WorkPermit) -> Result<Event, String> + Send>
            } else {
                if !self.native_review && self.from.trim().is_empty() != self.to.trim().is_empty() {
                    return Err("Supply both path-map fields or leave both empty".into());
                }
                let path = std::path::absolute(self.path.trim()).map_err(|e| e.to_string())?;
                if self.path.trim().is_empty() {
                    return Err("Choose an owned Live Set".into());
                }
                let options = Options {
                    remaps: if self.native_review || self.from.trim().is_empty() {
                        vec![]
                    } else {
                        vec![(
                            self.from.trim().into(),
                            std::path::absolute(self.to.trim()).map_err(|e| e.to_string())?,
                        )]
                    },
                };
                let native_review = self.native_review;
                Box::new(move |work: WorkPermit| {
                    crate::ableton::process::review(&path, &options, native_review, &work.cancel())
                        .map(Event::Reviewed)
                }) as Box<dyn FnOnce(WorkPermit) -> Result<Event, String> + Send>
            };
            let ticket = work.background(crate::background::Kind::Prepare,"Ableton migration".into(),crate::background::MEMORY_BYTES)?;
            std::thread::Builder::new()
                .name("ableton-migration".into())
                .spawn(move || {
                    let result =
                        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            let _running=ticket.enter(||work.cancelled())?;
                            job(work)
                        }))
                            .unwrap_or_else(|_| {
                                Err("Ableton migration worker failed; current session preserved"
                                    .into())
                            });
                    let _ = done.send(result.unwrap_or_else(Event::Failed));
                })
                .map_err(|e| e.to_string())?;
            self.events = Some(events);
            self.cancel = Some(cancel);
            self.published = None;
            self.accepted = false;
            if !publish {
                self.draft = None;
                self.render_backup = None;
            }
            self.message = if publish {
                "Publishing the reviewed native project…"
            } else {
                "Reading the Set and its media…"
            }
            .into();
            Ok(())
        })();
        if let Err(error) = result {
            self.message = error;
        }
    }
    fn relink(
        &mut self,
        engine: &Engine,
        binary: crate::plugin_host::BinaryIdentity,
        class: crate::plugin_host::Class,
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
            let draft = self.draft.clone().ok_or("Review a Live Set first")?;
            let device = self.device_index.ok_or("Choose a retained device first")?;
            let reviewed = self.reviewed_version;
            let cancel = work.cancel();
            let (done, events) = bounded(1);
            let ticket = work.background(crate::background::Kind::Prepare,"Ableton migration".into(),crate::background::MEMORY_BYTES)?;
            std::thread::Builder::new()
                .name("ableton-plugin-relink".into())
                .spawn(move || {
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        let _running=ticket.enter(||work.cancelled())?;
                        crate::ableton::process::edit(
                            &draft,
                            crate::ableton::process::Edit::Relink {
                                source: 0,
                                device,
                                binary,
                                class,
                                reviewed,
                            },
                            &work.cancel(),
                        )
                        .map(Event::Reviewed)
                    }))
                    .unwrap_or_else(|_| {
                        Err("Imported plugin relink worker failed; previous draft preserved".into())
                    });
                    let _ = done.send(result.unwrap_or_else(Event::Failed));
                })
                .map_err(|e| e.to_string())?;
            self.events = Some(events);
            self.cancel = Some(cancel);
            self.accepted = false;
            self.published = None;
            self.message = "Restoring the original plugin state in an isolated worker…".into();
            Ok(())
        })();
        if let Err(error) = result {
            self.message = error;
        }
    }

    fn render(&mut self, engine: &Engine, restore: bool) {
        if self.busy() {
            return;
        }
        let result = (|| -> Result<(), String> {
            if !restore && self.render_path.trim().is_empty() {
                return Err("Choose an authorized audio export first".into());
            }
            let work = engine
                .cmd
                .performance()
                .optional_work()
                .map_err(|e| e.to_string())?;
            let draft = self.draft.clone().ok_or("Review a Live Set first")?;
            let path = if restore {
                PathBuf::new()
            } else {
                std::path::absolute(self.render_path.trim()).map_err(|e| e.to_string())?
            };
            let options = crate::ableton::renders::Options {
                target: self.render_target,
                start: self.render_start,
                body_beats: self.render_body,
                includes_returns_master: self.render_master,
            };
            let cancel = work.cancel();
            let (done, events) = bounded(1);
            let ticket = work.background(crate::background::Kind::Prepare,"Ableton migration".into(),crate::background::MEMORY_BYTES)?;
            std::thread::Builder::new()
                .name("ableton-render-relink".into())
                .spawn(move || {
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        let _running=ticket.enter(||work.cancelled())?;
                        if restore {
                            return crate::ableton::process::edit(
                                &draft,
                                crate::ableton::process::Edit::Restore,
                                &work.cancel(),
                            )
                            .map(Event::Reviewed);
                        }
                        let original = draft.clone();
                        crate::ableton::process::edit(
                            &draft,
                            crate::ableton::process::Edit::Render { path, options },
                            &work.cancel(),
                        )
                        .map(|next| Event::Rendered(next, original))
                    }))
                    .unwrap_or_else(|_| {
                        Err("Render worker failed; previous draft preserved".into())
                    });
                    let _ = done.send(result.unwrap_or_else(Event::Failed));
                })
                .map_err(|e| e.to_string())?;
            self.events = Some(events);
            self.cancel = Some(cancel);
            self.accepted = false;
            self.published = None;
            self.message = "Reading the authorized render and checking its timing…".into();
            Ok(())
        })();
        if let Err(error) = result {
            self.message = error;
        }
    }
}
impl Drop for Panel {
    fn drop(&mut self) {
        self.cancel();
    }
}
impl App {
    pub(super) fn ableton_ui(&mut self, ctx: &egui::Context) {
        if !self.ableton.open {
            return;
        }
        let mut open = true;
        let enabled = !self.project.busy()
            && !self.project.committing()
            && !self.engine.cmd.performance().protected();
        let mut open_project = None;
        let mut relink = None;
        let mut render = false;
        let mut restore_render = false;
        let mut publish = false;
        egui::Window::new("Import Ableton Live Set").id(egui::Id::new("ableton-migration")).open(&mut open).default_width(680.).show(ctx,|ui|{
            let panel=&mut self.ableton;
            ui.add_enabled_ui(enabled&&!panel.busy(),|ui|{
                let label=ui.label("Owned Live 10/11 Set or saved migration project");
                let changed=ui.add(egui::TextEdit::singleline(&mut panel.path).char_limit(4096).desired_width(f32::INFINITY)).labelled_by(label.id).changed();
                let label=ui.label("Optional source path prefix");let changed=ui.add(egui::TextEdit::singleline(&mut panel.from).char_limit(4096)).labelled_by(label.id).changed()||changed;
                let label=ui.label("Replacement folder");let changed=ui.add(egui::TextEdit::singleline(&mut panel.to).char_limit(4096)).labelled_by(label.id).changed()||changed;
                let changed=ui.checkbox(&mut panel.native_review,"Read a saved Omatainer migration archive").changed()||changed;
                if changed{panel.draft=None;panel.render_backup=None;panel.published=None;panel.accepted=false;panel.device_index=None;panel.reviewed_version=false;}
                if ui.button("Review Live Set").clicked(){panel.start(&self.engine,false);}
            });
            if let Some(draft)=&panel.draft {
                let source=&draft.state.migration.as_ref().unwrap().sources[0];
                ui.label(format!("Saved format {} · {} tracks · {} scenes · {} devices · {} asset references",source.format,draft.state.tracks.len(),draft.state.scene_fx.len(),source.devices.len(),source.assets.len()));
                egui::ScrollArea::vertical().id_salt("ableton-review").max_height(280.).show(ui,|ui|{for difference in &source.differences {ui.label(format!("{} · {}: {}",difference.item,difference.feature,difference.detail));}});
                if draft.reviewed_native.is_some(){ui.label("The saved archive supplies original MIDI/device state. The original Live installation, Set and printed audio files may be offline; the selected native project is verified again before publication.");}
                ui.add_enabled_ui(enabled&&!panel.busy()&&source.renders.is_empty(),|ui|{
                    egui::ComboBox::from_id_salt("ableton-device").selected_text(panel.device_index.and_then(|i|source.devices.get(i)).map_or("Choose a retained device",|d|d.name.as_str())).show_ui(ui,|ui|{for (index,device) in source.devices.iter().enumerate(){if ui.selectable_value(&mut panel.device_index,Some(index),format!("{} · track {} · {}",device.name,device.track,device.path)).changed(){panel.reviewed_version=false;}}});
                    if let Some(device)=panel.device_index.and_then(|i|source.devices.get(i)) {
                        ui.label(format!("{} · class {} · source version {}",device.format.as_deref().unwrap_or("Ableton device"),device.class_id.as_deref().unwrap_or("unresolved"),device.version.as_deref().unwrap_or("not recorded")));
                        if let Some(resolution)=&device.resolution {ui.label(&resolution.fidelity);}
                        ui.checkbox(&mut panel.reviewed_version,"I reviewed installed-version and cross-platform state compatibility");
                        let mut matches=0;
                        for record in &self.plugins.catalog.records {
                            if record.failure.is_some()||self.plugins.catalog.blacklist.contains(&record.path){continue;}
                            let Some(binary)=&record.binary else{continue};
                            for class in &record.classes {
                                if device.format.as_deref()==Some("VST3") && device.class_id.as_ref().is_some_and(|id|id.eq_ignore_ascii_case(&class.info.uid)) {
                                    matches+=1;ui.label(format!("Installed {} · {} · {}",class.info.version,binary.arch,record.path.display()));
                                    if ui.add_enabled(panel.reviewed_version||device.version.as_deref()==Some(class.info.version.as_str()),egui::Button::new(format!("Restore {} {}",class.info.name,class.info.version))).clicked(){relink=Some((binary.clone(),class.clone()));}
                                }
                            }
                        }
                        if matches==0{ui.label("No qualified installed class matches this source identity. Scan the installed native VST3 folder, then return here. Other formats and native Ableton engines require an explicit replacement or aligned render.");}
                    }
                });
                ui.add_enabled_ui(enabled&&!panel.busy(),|ui|{
                    ui.separator();
                    ui.label("Authorized frozen/flattened audio or Live export");
                    if source.renders.is_empty() {
                        let label=ui.label("Render audio path");ui.add(egui::TextEdit::singleline(&mut panel.render_path).char_limit(4096).desired_width(f32::INFINITY)).labelled_by(label.id);
                        egui::ComboBox::from_id_salt("ableton-render-target").selected_text(panel.render_target.map_or("Complete mix".into(),|id|format!("Track {id}"))).show_ui(ui,|ui|{
                            if ui.selectable_value(&mut panel.render_target,None,"Complete mix").changed(){panel.render_master=false;}
                            for track in &source.tracks {if matches!(track.role.as_str(),"MidiTrack"|"AudioTrack") && ui.selectable_value(&mut panel.render_target,Some(track.source_id),format!("Track {} · {}",track.source_id,track.role)).changed(){panel.render_master=false;}}
                        });
                        ui.horizontal(|ui|{ui.label("Source start (quarter-note beats)");ui.add(egui::DragValue::new(&mut panel.render_start).range(0.0..=262144.0));});
                        ui.horizontal(|ui|{ui.label("Musical body (beats, before tail)");ui.add(egui::DragValue::new(&mut panel.render_body).range(0.000001..=262144.0));});
                        if panel.render_target.is_none(){ui.checkbox(&mut panel.render_master,"The complete mix includes return and master processing/gain");}
                        ui.label("Track exports must include track FX and mixer gain, excluding returns/master. Complete mixes retain printed returns; include master processing only when declared above. Use the same source start for every export and include the full tail.");
                        if ui.button("Attach aligned render").clicked(){render=true;}
                    } else {
                        for receipt in &source.renders {ui.label(format!("Rendered at beat {} · body {} beats · tail {:.6} s · printed master {}",receipt.start,receipt.body_beats,receipt.tail_seconds,receipt.includes_returns_master));}
                        if panel.render_backup.is_some() && ui.button("Restore pre-render draft").clicked(){restore_render=true;}
                        if panel.render_backup.is_none() && source.renders.iter().all(|r|r.restorable) && ui.button("Restore saved editable source").clicked(){restore_render=true;}
                    }
                    ui.checkbox(&mut panel.accepted,"I reviewed these playback differences and unresolved dependencies");
                    let label=ui.label("New native project path");ui.add(egui::TextEdit::singleline(&mut panel.destination).char_limit(4096).desired_width(f32::INFINITY)).labelled_by(label.id);
                    if ui.add_enabled(panel.accepted&&panel.published.is_none(),egui::Button::new("Publish native project")).clicked(){publish=true;}
                });
            }
            if let Some(path)=&panel.published {ui.label(path.display().to_string());if ui.add_enabled(enabled,egui::Button::new("Open imported project")).clicked(){open_project=Some(path.clone());}}
            if panel.busy()&&ui.button("Cancel migration").clicked(){panel.cancel();}
            ui.label(&panel.message);
        });
        if !open {
            self.ableton.cancel();
        }
        self.ableton.open = open;
        if let Some(path) = open_project {
            self.open_imported_project(path);
        }
        if let Some((binary, class)) = relink {
            self.ableton.relink(&self.engine, binary, class);
        }
        if render {
            self.ableton.render(&self.engine, false);
        }
        if publish {
            self.ableton.start(&self.engine, true);
        }
        if restore_render {
            if let Some(original) = self.ableton.render_backup.take() {
                self.ableton.draft = Some(original);
                self.ableton.accepted = false;
                self.ableton.published = None;
                self.ableton.message =
                    "Pre-render draft restored; current session preserved.".into();
            } else {
                self.ableton.render(&self.engine, true);
            }
        }
    }
}
