use super::*;
use crate::audio_delivery::{self as data, Export, Format, Source};
use crate::engine::audio::routing::model::Direction;
use crate::engine::audio::routing::record::delivery as live;
use crossbeam_channel::{bounded, Receiver};
use std::sync::atomic::{AtomicBool, Ordering};
mod finishing;
mod batch;
mod source;
#[cfg(test)]
mod tests;

#[derive(Clone, Debug)]
struct RecordSource {
    id: u64,
    name: String,
    channels: Option<Vec<u16>>,
    width: u16,
    contract: Option<crate::engine::audio::routing::speakers::OutputContract>,
}
#[derive(Clone, Debug)]
struct OutputSource {
    id: u64,
    name: String,
    contract: crate::engine::audio::routing::speakers::OutputContract,
}
#[derive(Clone, Debug)]
struct Plan {
    revision: u64,
    namespace: [u64; 2],
    generation: u64,
    outputs: Vec<OutputSource>,
    routed: bool,
    scenes: Vec<(u16, String)>,
    scene_refs:Vec<(u16,crate::engine::session::Reference)>,
    records: Vec<RecordSource>,
    rate: u32,
    catalog: Vec<(crate::engine::audio::routing::selection::Target,String)>,
    batch: data::batch::Recipe,
}
enum Operation {
    Batch {plan:Plan,recipe:data::batch::Recipe,path:PathBuf},
    SaveRecipe(data::batch::Recipe,PathBuf),
    LoadRecipe(PathBuf),
    Inspect,
    ReviewRecovery(PathBuf),
    Recover(data::recovery::Plan),
    Record {
        plan: Plan,
        source: RecordSource,
        format: Format,
        dither: bool,
        seconds: u32,
        path: PathBuf,
    },
    Export {
        plan: Plan,
        request: Export,
        config: data::finishing::Config,
        path: PathBuf,
    },
}
enum Reply {
    Inspected(Plan),
    Batched(data::batch::BatchOutcome),
    RecipeLoaded(data::batch::Recipe),
    RecipeSaved(PathBuf),
    Recovery(data::recovery::Plan),
    Recovered(data::recovery::Plan),
    Exported(data::Outcome),
    Recorded(live::Outcome),
}
struct Job {
    cancel: Arc<AtomicBool>,
    result: Receiver<Result<Reply, String>>,
    thread: Option<std::thread::JoinHandle<()>>,
    progress: crate::background::Reporter,
    recording: bool,
    batch_progress: data::batch::Progress,
}
impl Job {
    fn start(operation: Operation, engine: &Engine) -> Result<Self, String> {
        let recording = matches!(&operation, Operation::Record { .. });
        let permit = if matches!(&operation, Operation::Export { .. }|Operation::Batch{..}) {
            Some(
                engine
                    .cmd
                    .performance()
                    .optional_work()
                    .map_err(|e| e.to_string())?,
            )
        } else {
            None
        };
        let ticket = if let Some(permit) = &permit {
            Some(permit.background(
                crate::background::Kind::Render,
                crate::background::identity(&("audio-export", engine.project.revision()))?,
                crate::background::MEMORY_BYTES,
            )?)
        } else {
            None
        };
        let progress = ticket
            .as_ref()
            .map_or_else(crate::background::Reporter::default, |t| t.reporter());
        let cancel = permit
            .as_ref()
            .map_or_else(|| Arc::new(AtomicBool::new(false)), |p| p.cancel());
        let stop = cancel.clone();
        let handle = engine.project.clone();
        let recorder = engine.routing.recorder.clone();
        let batch_progress=data::batch::Progress::default();let batch_status=batch_progress.clone();
        let (tx, result) = bounded(1);
        let thread=std::thread::Builder::new().name("omatainer-audio-delivery".into()).spawn(move||{
            let result=(||{
                let _running=if let Some(ticket)=&ticket{Some(ticket.enter(||stop.load(Ordering::Acquire))?)}else{crate::background::recording_priority()?;None};
                match &operation { Operation::SaveRecipe(recipe,path)=>{recipe.save(path)?;return Ok(Reply::RecipeSaved(path.clone()));},Operation::LoadRecipe(path)=>return data::batch::Recipe::load(path,&stop).map(Reply::RecipeLoaded), Operation::ReviewRecovery(path)=>return data::recovery::review(path,&stop).map(Reply::Recovery), Operation::Recover(plan)=>return data::recovery::recover(plan,&stop).map(Reply::Recovered), _=>{} }
                let epoch=recorder.epoch();let captured=handle.capture(&stop).map_err(|e|e.to_string())?;
                match operation{
                    Operation::ReviewRecovery(_)|Operation::Recover(_)|Operation::SaveRecipe(..)|Operation::LoadRecipe(_)=>unreachable!(),
                    Operation::Batch{plan,recipe,path}=>{let layout=captured.state.session.as_ref().ok_or("Session identity unavailable")?;if captured.revision!=plan.revision||layout.namespace!=plan.namespace||layout.generation!=plan.generation{return Err("Project changed since batch review; refresh source".into());}let review=recipe.review(&captured)?;data::batch::run(captured,&review,&path,permit.as_ref().unwrap(),&ticket.as_ref().unwrap().reporter(),&batch_status).map(Reply::Batched)},
                    Operation::Inspect=>{
                        let layout=captured.state.session.as_ref().ok_or("Session identity unavailable")?;let model=captured.state.routing.as_ref();
                        let (outputs,records)=source::inspect(&captured)?;
                        let scenes=layout.scene_order.iter().filter_map(|&slot|layout.scenes.get(usize::from(slot)).filter(|s|s.active).map(|s|(slot,s.name.clone()))).collect();
                        Ok(Reply::Inspected(Plan{revision:captured.revision,namespace:layout.namespace,generation:layout.generation,outputs,routed:model.is_some(),scenes,scene_refs:layout.scene_order.iter().filter_map(|&slot|layout.reference(crate::engine::session::Axis::Scene,usize::from(slot)).map(|r|(slot,r))).collect(),records,rate:handle.sample_rate(),catalog:data::batch::Recipe::catalog(&captured)?,batch:data::batch::Recipe::aligned(&captured,Export::default(),data::finishing::Config::default())?}))
                    },
                    Operation::Export{plan,request,config,path}=>{
                        let layout=captured.state.session.as_ref().ok_or("Session identity unavailable")?;if captured.revision!=plan.revision||layout.namespace!=plan.namespace||layout.generation!=plan.generation{return Err("Project changed since export review. Refresh the source before rendering".into());}
                        data::run_selected(captured,&request,None,&config,&path,permit.as_ref().unwrap(),&ticket.as_ref().unwrap().reporter()).map(Reply::Exported)
                    },
                    Operation::Record{plan,source,format,dither,seconds,path}=>{
                        let layout=captured.state.session.as_ref().ok_or("Session identity unavailable")?;if layout.namespace!=plan.namespace||layout.generation!=plan.generation||handle.sample_rate()!=plan.rate{return Err("Recording source changed since review; refresh before recording".into());}
                        let (_,current_sources)=source::inspect(&captured)?;
                        if current_sources.iter().find(|s|s.id==source.id&&s.channels.is_some()==source.channels.is_some()).is_none_or(|s|s.width!=source.width||s.channels!=source.channels||s.contract!=source.contract){return Err("Recording channel or speaker plan changed since review".into());}
                        let request=live::Request{alias:source.id,output_channels:source.channels,options:data::Options{format,rate:plan.rate,channels:source.width,dither,normalize:false},seconds,epoch};
                        recorder.write_delivery_with_contract(&request,&path,&stop,source.contract.as_ref()).map(Reply::Recorded)
                    }
                }
            })();if stop.load(Ordering::Acquire){let _=handle.retire_cancelled_capture(&stop);}let _=tx.try_send(result);
        }).map_err(|e|e.to_string())?;
        Ok(Self {
            cancel,
            result,
            thread: Some(thread),
            progress,
            recording,
            batch_progress,
        })
    }
    fn poll(&mut self) -> Option<Result<Reply, String>> {
        if !self.thread.as_ref()?.is_finished() {
            return None;
        }
        let _ = self.thread.take().unwrap().join();
        Some(
            self.result
                .try_recv()
                .unwrap_or_else(|_| Err("Audio delivery worker ended without a result".into())),
        )
    }
}
impl Drop for Job {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
        if self.thread.as_ref().is_some_and(|t| t.is_finished()) {
            let _ = self.thread.take().unwrap().join();
        }
    }
}
#[derive(Default)]
pub(super) struct Panel {
    pub open: bool,
    plan: Option<Plan>,
    request: Export,
    config: data::finishing::Config,
    path: String,
    job: Option<Job>,
    message: String,
    record_source: Option<u64>,
    record_format: Format,
    record_dither: bool,
    record_seconds: u32,
    record_path: String,
    recovery_path: String,
    recovery: Option<data::recovery::Plan>,
    batch:Option<data::batch::Recipe>,
    version_name:String,
    version_targets:Vec<crate::engine::audio::routing::selection::Target>,
    version_policy:crate::engine::audio::routing::selection::Policy,
    version_kind:data::batch::Kind,
    recipe_path:String,
    batch_normalize:bool,
    batch_dither:bool,
}
impl App {
    fn start_audio_delivery(&mut self, operation: Operation) {
        if self.audio_delivery.job.is_some() || self.project.committing() {
            return;
        }
        match Job::start(operation, &self.engine) {
            Ok(job) => {
                self.audio_delivery.job = Some(job);
                self.audio_delivery.message = "Audio delivery queued; Cancel is available".into();
            }
            Err(e) => self.audio_delivery.message = e,
        }
    }
    pub(super) fn poll_audio_delivery(&mut self, ctx: &egui::Context) {
        if self.audio_delivery.job.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(50));
        }
        let reply = self.audio_delivery.job.as_mut().and_then(Job::poll);
        if let Some(reply) = reply {
            self.audio_delivery.job = None;
            match reply {
                Ok(Reply::Batched(outcome))=>{self.audio_delivery.message=format!("Saved aligned batch: {} files · {} frames per pass in {}{}",outcome.files,outcome.frames,outcome.folder.display(),outcome.warning.map_or(String::new(),|w|format!(" · {w}")));},
                Ok(Reply::RecipeLoaded(recipe))=>{self.audio_delivery.batch_normalize=recipe.export.options.normalize;self.audio_delivery.batch_dither=recipe.export.options.dither;self.audio_delivery.request=recipe.export.clone();self.audio_delivery.config=recipe.config.clone();self.audio_delivery.batch=Some(recipe);self.audio_delivery.message="Recipe reopened. Saved contribution IDs will be checked against this reviewed project before rendering.".into();},
                Ok(Reply::RecipeSaved(path))=>{self.audio_delivery.message=format!("Saved batch recipe: {}",path.display());},
                Ok(Reply::Inspected(plan)) => {
                    let needs_batch=self.audio_delivery.batch.as_ref().is_none_or(|r|r.namespace!=plan.namespace);
                    if self.audio_delivery.request.output_alias.is_none() && plan.outputs.len() == 1
                    {
                        self.audio_delivery.request.output_alias = Some(plan.outputs[0].id);
                        self.audio_delivery.request.options.channels = plan.outputs[0].contract.channel_count;
                    }
                    if !plan
                        .scenes
                        .iter()
                        .any(|(slot, _)| *slot == self.audio_delivery.request.scene)
                    {
                        self.audio_delivery.request.scene =
                            plan.scenes.first().map_or(0, |(slot, _)| *slot);
                    }
                    if needs_batch {let mut recipe=plan.batch.clone();recipe.source_scene=plan.scene_refs.iter().find(|(slot,_)|*slot==self.audio_delivery.request.scene).map(|(_,r)|*r);self.audio_delivery.batch=Some(recipe);}
                    if self.audio_delivery.record_source.is_none() {
                        self.audio_delivery.record_source = plan.records.first().map(|s| s.id);
                    }
                    self.audio_delivery.plan = Some(plan);
                    self.audio_delivery.message="Source reviewed. Export uses a coherent snapshot and fresh DSP; live playback stays unchanged.".into();
                }
                Ok(Reply::Exported(o)) => {
                    self.audio_delivery.message = format!(
                        "Saved {} frames · {} Hz · {} channels · peak {:.3} · gain {:.6} to {}{}",
                        o.frames,
                        o.rate,
                        o.channels,
                        o.peak,
                        o.gain,
                        o.folder.display(),
                        o.warning.map_or(String::new(), |w| format!(" · {w}"))
                    )
                }
                Ok(Reply::Recorded(o)) => {
                    self.audio_delivery.message = format!(
                        "Recorded {} frames · {} Hz · {} channels · {} files in {}{}",
                        o.frames,
                        o.rate,
                        o.channels,
                        o.files.len(),
                        o.folder.display(),
                        o.warning.map_or(String::new(), |w| format!(" · {w}"))
                    )
                }
                Ok(Reply::Recovery(plan)) => {
                    self.audio_delivery.message = format!(
                        "Reviewed {} valid frames in {} · {} files need repair",
                        plan.frames,
                        plan.folder.display(),
                        plan.repairs
                    );
                    self.audio_delivery.recovery = Some(plan);
                }
                Ok(Reply::Recovered(plan)) => {
                    self.audio_delivery.message = format!(
                        "Recovered {} frames in {}",
                        plan.frames,
                        plan.folder.display()
                    );
                    self.audio_delivery.recovery = Some(plan);
                }
                Err(e) => self.audio_delivery.message = e,
            }
        }
    }
    pub(super) fn audio_delivery_ui(&mut self, ctx: &egui::Context) {
        if !self.audio_delivery.open {
            return;
        }
        let mut open = true;
        let mut inspect = false;
        let mut export = None;
        egui::Window::new(tr!("Audio export & recording")).open(&mut open).default_width(640.0).show(ctx,|ui|{
            let panel=&mut self.audio_delivery;let busy=panel.job.is_some();ui.label(tr!("Export a scene or the current session without changing playback. Choose a new output folder."));
            if ui.add_enabled(!busy,egui::Button::new(tr!("Review export source"))).help(ui,HelpControl::AudioExport).clicked(){inspect=true;}
            ui.add_enabled_ui(!busy,|ui|{if let Some(plan)=&panel.plan{
                let previous_source=panel.request.source;
                ui.horizontal(|ui|{ui.selectable_value(&mut panel.request.source,Source::Arrangement,"Arrangement song");ui.selectable_value(&mut panel.request.source,Source::Session,tr!("Current session launches"));ui.selectable_value(&mut panel.request.source,Source::Scene,tr!("Selected scene"));});
                if previous_source!=panel.request.source&&panel.request.source==Source::Scene {if let Some(recipe)=&mut panel.batch {recipe.source_scene=plan.scene_refs.iter().find(|(slot,_)|*slot==panel.request.scene).map(|(_,r)|*r);}}
                if panel.request.source==Source::Scene{egui::ComboBox::from_id_salt("audio-export-scene").selected_text(plan.scenes.iter().find(|(slot,_)|*slot==panel.request.scene).map_or("Choose a scene",|(_,name)|name.as_str())).show_ui(ui,|ui|{for(slot,name)in &plan.scenes{if ui.selectable_value(&mut panel.request.scene,*slot,name).clicked(){if let Some(recipe)=&mut panel.batch{recipe.source_scene=plan.scene_refs.iter().find(|(s,_)|s==slot).map(|(_,r)|*r);}}}});}
                ui.checkbox(&mut panel.request.decks,tr!("Include loaded decks from their saved cursors"));
                source::output(ui,&mut panel.request,plan);
                ui.horizontal(|ui|{ui.label(tr!("Range start (seconds)"));ui.add(egui::DragValue::new(&mut panel.request.start).range(0.0..=28800.0).speed(0.1));ui.label(tr!("Range end (seconds)"));ui.add(egui::DragValue::new(&mut panel.request.end).range(0.0..=28800.0).speed(0.1));});
                ui.horizontal(|ui|{ui.label(tr!("Repeat range"));ui.add(egui::DragValue::new(&mut panel.request.repeats).range(1..=64));ui.label(tr!("Final tail (seconds)"));ui.add(egui::DragValue::new(&mut panel.request.tail).range(0.0..=120.0).speed(0.1));});
                egui::ComboBox::from_id_salt("audio-export-format").selected_text(panel.request.options.format.title()).show_ui(ui,|ui|{for format in Format::ALL{ui.selectable_value(&mut panel.request.options.format,format,format.title());}});
                ui.horizontal(|ui|{let label=ui.label(tr!("Sample rate (Hz)"));ui.add(egui::DragValue::new(&mut panel.request.options.rate).range(8000..=192000).speed(100.0)).labelled_by(label.id);source::width(ui,&mut panel.request,plan);});
                if panel.request.options.channels<=2&&plan.outputs.iter().any(|s|Some(s.id)==panel.request.output_alias&&s.contract.channel_count>2){ui.label("Mono and stereo use the saved speaker fold and exclude dedicated LFE.");}
                if !panel.request.options.format.dither(){panel.request.options.dither=false;}ui.add_enabled(panel.request.options.format.dither(),egui::Checkbox::new(&mut panel.request.options.dither,tr!("Triangular dither for integer quantization")));ui.checkbox(&mut panel.request.options.normalize,tr!("Normalize sample peak to −1 dBFS"));
                ui.label(tr!("Ranges start at scene zero or the captured session cursor. The session uses its currently launched clips. Effects build from the source start; each repeat copies the same rendered range. Tail keeps the decay after sources stop. Metronome and headphone cue are excluded. FLAC/MP3 require FFmpeg."));
                finishing::controls(ui,&mut panel.config);
                ui.label(tr!("New export folder"));ui.add(egui::TextEdit::singleline(&mut panel.path).hint_text("/home/you/Music/Exports/new-mix").desired_width(f32::INFINITY));
                match panel.request.frames(){Ok((_,_,_,frames))=>{ui.label(format!("{} frames · {:.3} seconds",frames,frames as f64/f64::from(panel.request.options.rate)));if ui.button(tr!("Render master audio")).help(ui,HelpControl::AudioExport).clicked(){export=Some(Operation::Export{plan:plan.clone(),request:panel.request.clone(),config:panel.config.clone(),path:PathBuf::from(&panel.path)});}},Err(e)=>{ui.label(e);}}
                let reviewed=plan.clone();if let Some(operation)=batch::controls(ui,panel,&reviewed){export=Some(operation);}
            }});
            ui.separator();
            ui.heading(tr!("Record the performance"));
            ui.add_enabled_ui(!busy,|ui|{if let Some(plan)=&panel.plan{
                if panel.record_seconds==0{panel.record_seconds=3600;}
                egui::ComboBox::from_id_salt("performance-record-source").selected_text(plan.records.iter().find(|s|Some(s.id)==panel.record_source).map_or("Choose a recording source",|s|s.name.as_str())).show_ui(ui,|ui|{for source in &plan.records{ui.selectable_value(&mut panel.record_source,Some(source.id),&source.name);}});
                egui::ComboBox::from_id_salt("performance-record-format").selected_text(panel.record_format.title()).show_ui(ui,|ui|{for format in Format::RECORDING{ui.selectable_value(&mut panel.record_format,format,format.title());}});
                if !panel.record_format.dither(){panel.record_dither=false;}ui.add_enabled(panel.record_format.dither(),egui::Checkbox::new(&mut panel.record_dither,tr!("Dither the recording to integer PCM")));
                ui.horizontal(|ui|{ui.label(tr!("Maximum recording seconds"));ui.add(egui::DragValue::new(&mut panel.record_seconds).range(1..=43200));});
                ui.label(tr!("New recording folder"));ui.add(egui::TextEdit::singleline(&mut panel.record_path).hint_text("/home/you/Music/Recordings/new-set").desired_width(f32::INFINITY));
                ui.label(tr!("Final output includes the limiter, output conversion and recovery fades. Raw routed sources include their chosen return and tap. WAV segments split automatically at 256 MiB. Stop preserves valid audio; FLAC/MP3 encoding follows capture and keeps the original WAV files. Recording is available during Performance Mode."));
                if let Some(source)=plan.records.iter().find(|s|Some(s.id)==panel.record_source){ui.label(format!("{} Hz · {} channels in saved order",plan.rate,source.width));if ui.button(tr!("Start performance recording")).help(ui,HelpControl::PerformanceRecording).clicked(){export=Some(Operation::Record{plan:plan.clone(),source:source.clone(),format:panel.record_format,dither:panel.record_dither,seconds:panel.record_seconds,path:PathBuf::from(&panel.record_path)});}}
            }});
            ui.separator();ui.heading(tr!("Recover an interrupted recording"));
            ui.add_enabled_ui(!busy,|ui|{ui.label(tr!("Existing recording folder"));ui.add(egui::TextEdit::singleline(&mut panel.recovery_path).hint_text("/home/you/Music/Recordings/interrupted-set").desired_width(f32::INFINITY));if ui.button(tr!("Review recording recovery")).clicked(){export=Some(Operation::ReviewRecovery(PathBuf::from(&panel.recovery_path)));}if let Some(plan)=&panel.recovery{ui.label(format!("{} frames · {} Hz · {} channels · {} files need repair",plan.frames,plan.rate,plan.channels,plan.repairs));if ui.button(tr!("Recover reviewed recording")).clicked(){export=Some(Operation::Recover(plan.clone()));}}});
            if let Some(job)=&panel.job{let status=job.batch_progress.lock().unwrap().clone();if status.total>0{ui.label(format!("Pass {} / {} · {} · {}",status.item,status.total,status.name,status.phase));}if job.recording{ui.label(format!("Recorded source frames: {} · peak {:.3}",self.engine.routing.recorder.frames(),self.engine.routing.recorder.delivery_peak()));}if let Some((done,total))=job.progress.values(){if let Some(total)=total.filter(|t|*t>0){ui.add(egui::ProgressBar::new(done as f32/total as f32).text(format!("{} / {} frames",done,total)));}else{ui.label(tr!("Waiting for worker or encoding"));}}if ui.button(if job.recording{tr!("Stop performance recording")}else{tr!("Cancel audio work")}).clicked(){job.cancel.store(true,Ordering::Release);}}
            ui.label(&panel.message);
        });
        self.audio_delivery.open = open;
        if inspect {
            self.start_audio_delivery(Operation::Inspect);
        }
        if let Some(operation) = export {
            self.start_audio_delivery(operation);
        }
    }
}
