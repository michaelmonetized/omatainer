use super::*;
use crate::video::{self as data, Clip, Locator};
use std::{collections::VecDeque, time::Duration};
mod worker;
use worker::{Job, Kind, Reply};
#[derive(Default)]
pub(super) struct Panel {
    pub open: bool,
    pub clip: Option<Clip>,
    draft: Option<Clip>,
    path: String,
    output: String,
    locator: String,
    job: Option<Job>,
    pending: Option<Kind>,
    preview_paused: bool,
    future: VecDeque<data::decoder::Frame>,
    texture: Option<(u64, egui::TextureHandle)>,
    validated: bool,
    needs_probe: bool,
    message: String,
}
impl Panel {
    /// Restore project picture metadata without reading media on the frame.
    /// Takes saved clip; cancels old work and schedules verification when its viewer opens.
    pub fn install(&mut self, clip: Option<Clip>) {
        if let Some(job) = &mut self.job {
            job.cancel();
        }
        self.pending = None;
        self.future.clear();
        self.texture = None;
        self.validated = false;
        self.needs_probe = clip.is_some();
        self.path = clip.as_ref().map_or(String::new(), |clip| {
            clip.path.to_string_lossy().into_owned()
        });
        self.draft = clip.clone();
        self.clip = clip;
        self.message.clear();
    }
    fn stop(&mut self) {
        if let Some(job) = &mut self.job {
            job.cancel();
        }
        self.future.clear();
    }
}
impl App {
    /// Admit a picture job after rechecking current project and protection fences.
    /// Takes an operation; retains one active worker and reports admission failures visibly.
    fn start_video(&mut self, kind: Kind) {
        if self.project.committing() {
            return;
        }
        if let Some(job) = &mut self.video.job {
            if job.decode_start.is_some() {
                job.cancel();
                self.video.future.clear();
                self.video.texture = None;
                self.video.pending = Some(kind);
            }
            return;
        }
        match self
            .engine
            .cmd
            .performance()
            .optional_work()
            .map_err(|e| e.to_string())
            .and_then(|permit| Job::start(kind, self.engine.project.clone(), permit))
        {
            Ok(job) => {
                self.video.job = Some(job);
                if self
                    .video
                    .job
                    .as_ref()
                    .is_some_and(|job| job.decode_start.is_none())
                {
                    self.video.message = "Video job running; Cancel is available".into();
                }
            }
            Err(error) => self.video.message = error,
        }
    }
    /// Follow the audio-owned picture position through bounded worker queues.
    /// Takes frame context; presents only due frames and restarts obsolete decode positions without UI or audio I/O.
    pub(super) fn poll_video(&mut self, ctx: &egui::Context) {
        let protected = self.engine.cmd.performance().protected() || self.project.committing();
        let visible = self.video.open || self.video.clip.as_ref().is_some_and(|clip| clip.detached);
        let due = self
            .video
            .clip
            .as_ref()
            .and_then(|clip| clip.source_frame(self.snap.timeline_seconds));
        let panel = &mut self.video;
        if protected {
            panel.stop();
        }
        if let Some(job) = &mut panel.job {
            if let Some(start) = job.decode_start {
                let latest = panel.texture.as_ref().map_or(start, |t| t.0);
                if !visible || due.is_none_or(|n| n < latest || n > latest + 4) {
                    job.cancel();
                    panel.future.clear();
                    panel.texture = None;
                }
                while !job.intentionally_cancelled && panel.future.len() < 3 {
                    if let Ok(frame) = job.frames.try_recv() {
                        panel.future.push_back(frame);
                    } else {
                        break;
                    }
                }
            }
            let ignored = job.intentionally_cancelled;
            let source_operation = job.source_operation;
            if let Some(result) = job.poll() {
                panel.job = None;
                match result {
                    Ok(Reply::Imported(clip)) => {
                        panel.path = clip.path.to_string_lossy().into_owned();
                        panel.draft = Some(clip.clone());
                        panel.clip = Some(clip);
                        panel.validated = true;
                        panel.preview_paused = false;
                        panel.needs_probe = false;
                        panel.texture = None;
                        panel.future.clear();
                        panel.message =
                            "Picture verified; preview follows the audio transport".into();
                    }
                    Ok(Reply::Rendered(path, outcome)) => {
                        panel.message = match outcome {
                            crate::project_file::SaveOutcome::Durable => format!(
                                "Score WAV, aligned picture and alignment metadata saved to {}",
                                path.display()
                            ),
                            crate::project_file::SaveOutcome::CommittedButDirectorySyncFailed(
                                error,
                            ) => format!(
                                "Render published to {}; directory durability unconfirmed: {error}",
                                path.display()
                            ),
                        }
                    }
                    Ok(Reply::Decoded) => {}
                    Err(error) if !ignored => {
                        panel.message = error;
                        if source_operation {
                            panel.validated = false;
                        }
                        panel.future.clear();
                        panel.texture = None;
                    }
                    Err(_) => {}
                }
            }
        }
        if let Some(due) = due {
            let mut display = None;
            while panel.future.front().is_some_and(|f| f.index <= due) {
                display = panel.future.pop_front();
            }
            if let Some(frame) = display {
                let image = egui::ColorImage::from_rgba_unmultiplied(
                    [frame.width, frame.height],
                    &frame.rgba,
                );
                if let Some((index, texture)) = &mut panel.texture {
                    texture.set(image, egui::TextureOptions::LINEAR);
                    *index = frame.index;
                } else {
                    panel.texture = Some((
                        frame.index,
                        ctx.load_texture("video-picture", image, egui::TextureOptions::LINEAR),
                    ));
                }
            }
        }
        if !protected && self.video.job.is_none() {
            if let Some(kind) = self.video.pending.take() {
                self.start_video(kind);
                return;
            }
        }
        if !protected && visible && self.video.job.is_none() {
            if self.video.needs_probe {
                self.video.needs_probe = false;
                if let Some(clip) = self.video.clip.clone() {
                    self.start_video(Kind::Import {
                        path: clip.path.clone(),
                        expected: Some(clip),
                    });
                }
            } else if self.video.validated
                && !self.video.preview_paused
                && self.video.future.is_empty()
            {
                if let Some(start) =
                    due.filter(|n| self.video.texture.as_ref().is_none_or(|t| t.0 != *n))
                {
                    if let Some(clip) = self.video.clip.clone() {
                        self.start_video(Kind::Decode { clip, start });
                    }
                }
            }
        }
        if visible {
            ctx.request_repaint_after(Duration::from_millis(16));
        }
    }
    /// Show picture import, frame editing, locators and an independent preview viewport.
    /// Takes the native context; stores only reviewed valid picture metadata and sends explicit transport seeks.
    pub(super) fn video_ui(&mut self, ctx: &egui::Context) {
        let mut operation = None;
        let mut seek = None;
        let mut remove = false;
        let available = !self.engine.cmd.performance().protected()
            && !self.project.committing()
            && self
                .video
                .job
                .as_ref()
                .is_none_or(|job| job.decode_start.is_some());
        let seconds = self.snap.timeline_seconds;
        let revision = self.snap.project_revision;
        if self.video.open {
            keyboard::block_for_dialog(ctx);
            let mut open = true;
            let panel = &mut self.video;
            egui::Window::new("Score to video").id(egui::Id::new("score-video-window")).open(&mut open).default_width(740.0).resizable(true).vscroll(true).show(ctx,|ui|{
                ui.label("One local picture clip. FFmpeg/ffprobe required. Source audio is excluded. Picture follows audio transport seconds; frame numbers are zero-based and trim end is exclusive.");
                video_text(ui,"Local video path",&mut panel.path);
                if ui.add_enabled(available,egui::Button::new("Import local video")).help(ui,HelpControl::VideoImport).clicked(){operation=Some(Kind::Import{path:PathBuf::from(&panel.path),expected:None});panel.stop();panel.texture=None;panel.future.clear();panel.validated=false;}
                if panel.job.is_some() && ui.button("Cancel video job").help(ui,HelpControl::VideoCancel).clicked(){panel.stop();panel.preview_paused=true;panel.pending=None;}
                ui.checkbox(&mut panel.preview_paused,"Pause picture decoding");
                if panel.preview_paused { if let Some(job)=&mut panel.job {if job.decode_start.is_some(){job.cancel();panel.future.clear();}}}
                ui.label(&panel.message);
                if let Some(clip)=&mut panel.clip {
                    let rate=clip.info.rate;let frame=rate.frame(seconds);let source=clip.source_frame(seconds);
                    ui.label(format!("{} · {}×{} · {}/{} fps · {} source frames · project {:.6}s",clip.info.codec,clip.info.width,clip.info.height,rate.numerator,rate.denominator,clip.info.frames,seconds));
                    ui.label(format!("Project frame {frame}; source timecode {}",source.map(|n|rate.timecode(n as i64+clip.timecode_offset,clip.drop_frame).unwrap()).unwrap_or_else(||"outside picture".into())));
                    if let Some(draft)=&mut panel.draft {
                        ui.add_enabled_ui(available,|ui|{
                            video_number(ui,"Picture trim in frame",&mut draft.trim_in,0..=clip.info.frames-1);
                            video_number(ui,"Picture trim end frame (exclusive)",&mut draft.trim_out,1..=clip.info.frames);
                            video_number(ui,"Picture placement project frame",&mut draft.placement,0..=data::MAX_FRAMES);
                            let label=ui.label("Timecode offset frames");let response=ui.add(egui::DragValue::new(&mut draft.timecode_offset).range(-10_000_000..=10_000_000)).labelled_by(label.id);ui.ctx().accesskit_node_builder(response.id,|node|node.set_label("Timecode offset frames"));
                            ui.checkbox(&mut draft.drop_frame,"Drop-frame timecode");
                            let label=ui.label("Preview latency offset (ms; negative delays picture)");
                            let response=ui.add(egui::DragValue::new(&mut draft.preview_offset_ms).range(-2000..=2000)).labelled_by(label.id);
                            ui.ctx().accesskit_node_builder(response.id,|node|node.set_label("Preview latency offset (ms; negative delays picture)"));
                            ui.label("Calibrate picture against your actual audio output. This saved preview adjustment leaves rendered timing unchanged.");
                            if ui.button("Apply picture trim and placement").help(ui,HelpControl::VideoPlacement).clicked(){
                                match draft.validate(){Ok(())=>{draft.locators=clip.locators.clone();draft.detached=clip.detached;*clip=draft.clone();if let Some(job)=&mut panel.job{job.cancel();}panel.future.clear();panel.texture=None;panel.message="Picture placement applied; save the native project to retain it".into();},Err(error)=>panel.message=error}
                            }
                        });
                    }
                    ui.add_enabled_ui(available,|ui|{
                        let mut target=frame.min(clip.end());
                        let response=ui.add(egui::Slider::new(&mut target,0..=clip.end()).text("Scrub project video frame"));
                        if response.changed(){seek=Some(rate.seconds(target));}
                        ui.horizontal(|ui|{if ui.button("Previous picture frame").clicked(){seek=Some(rate.seconds(frame.saturating_sub(1)));}if ui.button("Next picture frame").clicked(){seek=Some(rate.seconds((frame+1).min(clip.end())));}if ui.button("Seek picture start").clicked(){seek=Some(rate.seconds(clip.placement));}});
                        video_text(ui,"Picture locator name",&mut panel.locator);
                        if ui.button("Add picture locator").help(ui,HelpControl::VideoLocator).clicked(){
                            let locator=Locator{frame,name:panel.locator.clone()};let mut next=clip.clone();next.locators.push(locator);
                            match next.validate(){Ok(())=>{*clip=next;panel.locator.clear();},Err(error)=>panel.message=error}
                        }
                        let mut remove=None;
                        for (i,locator) in clip.locators.iter().enumerate(){ui.horizontal(|ui|{if ui.button(format!("Seek locator {} · {}",i+1,locator.name)).clicked(){seek=Some(rate.seconds(locator.frame));}if ui.button(format!("Remove locator {}",i+1)).clicked(){remove=Some(i);}});}
                        if let Some(i)=remove{clip.locators.remove(i);}
                    });
                    if ui.add_enabled(available,egui::Button::new("Remove picture reference")).help(ui,HelpControl::VideoPlacement).clicked(){remove=true;}
                    ui.checkbox(&mut clip.detached,"Detach picture preview").help(ui,HelpControl::VideoPreview);
                    paint(ui,panel.texture.as_ref(),source);
                    video_text(ui,"New score output folder",&mut panel.output);
                    ui.label("Render loops the selected Session scene from project zero, using fresh native mixer/effect state. It writes 48 kHz stereo float WAV, lossless FFV1/PCM picture and exact alignment metadata in a new folder. Existing output is preserved; WAV is bounded to 2 GiB. Live playback is unchanged.");
                    if ui.add_enabled(available && panel.validated,egui::Button::new("Render selected scene against picture")).help(ui,HelpControl::VideoRender).clicked(){operation=Some(Kind::Render{clip:clip.clone(),path:PathBuf::from(&panel.output),revision});}
                }
            });
            self.video.open = open;
        }
        if let Some(clip) = &mut self.video.clip {
            if clip.detached {
                let source = clip.source_frame(seconds);
                let mut detached = true;
                ctx.show_viewport_immediate(
                    egui::ViewportId::from_hash_of("omatainer-picture-preview"),
                    egui::ViewportBuilder::default()
                        .with_title("Omatainer picture preview")
                        .with_inner_size([800.0, 480.0])
                        .with_resizable(true),
                    |ctx, class| {
                        if class == egui::ViewportClass::Embedded {
                            egui::Window::new("Detached picture preview")
                                .open(&mut detached)
                                .resizable(true)
                                .show(ctx, |ui| paint(ui, self.video.texture.as_ref(), source));
                        } else {
                            egui::CentralPanel::default()
                                .show(ctx, |ui| paint(ui, self.video.texture.as_ref(), source));
                            if ctx.input(|i| i.viewport().close_requested()) {
                                detached = false;
                            }
                        }
                    },
                );
                clip.detached = detached;
            }
        }
        if remove {
            self.video.install(None);
        }
        if let Some(kind) = operation {
            self.start_video(kind);
        }
        if let Some(seconds) = seek {
            self.send(Command::TimelineSeek(seconds));
        }
    }
}
/// Present the exact due frame or an explicit black/waiting state.
/// Takes texture and required source frame; never labels a stale picture as current.
fn paint(ui: &mut Ui, texture: Option<&(u64, egui::TextureHandle)>, due: Option<u64>) {
    if let Some((index, texture)) = texture.filter(|(index, _)| Some(*index) == due) {
        ui.add(
            egui::Image::new(texture)
                .max_size(ui.available_size().min(Vec2::new(1280.0, 720.0)))
                .maintain_aspect_ratio(true),
        );
        ui.label(format!("Decoded source frame {index}"));
    } else {
        ui.label(due.map_or_else(
            || "Outside trimmed picture".into(),
            |frame| format!("Waiting for source frame {frame}"),
        ));
        let size = Vec2::new(ui.available_width().min(1280.0), 240.0);
        let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
        ui.painter().rect_filled(rect, 0.0, Color32::BLACK);
    }
}
/// Label a native picture text field for keyboard and assistive input.
/// Takes label and draft; updates bounded text only.
fn video_text(ui: &mut Ui, name: &str, value: &mut String) {
    let label = ui.label(name);
    let response = ui
        .add(egui::TextEdit::singleline(value).char_limit(4096))
        .labelled_by(label.id);
    ui.ctx()
        .accesskit_node_builder(response.id, |node| node.set_label(name));
}
/// Label a native frame-number control.
/// Takes label, integer draft and range; exposes the same value to assistive technology.
fn video_number(ui: &mut Ui, name: &str, value: &mut u64, range: std::ops::RangeInclusive<u64>) {
    let label = ui.label(name);
    let response = ui
        .add(egui::DragValue::new(value).range(range))
        .labelled_by(label.id);
    ui.ctx()
        .accesskit_node_builder(response.id, |node| node.set_label(name));
}
#[cfg(test)]
mod tests;
