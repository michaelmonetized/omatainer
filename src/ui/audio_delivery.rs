use super::*;
use crate::audio_delivery::{self as data, Export, Format, Source};
use crate::engine::audio::routing::model::Direction;
use crate::engine::audio::routing::record::delivery as live;
use crossbeam_channel::{bounded, Receiver};
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(test)]
mod tests;

#[derive(Clone, Debug)]
struct RecordSource {
    id: u64,
    name: String,
    channels: Option<Vec<u16>>,
    width: u16,
}
#[derive(Clone, Debug)]
struct Plan {
    revision: u64,
    namespace: [u64; 2],
    generation: u64,
    outputs: Vec<(u64, String)>,
    routed: bool,
    scenes: Vec<(u16, String)>,
    records: Vec<RecordSource>,
    rate: u32,
}
enum Operation {
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
        path: PathBuf,
    },
}
enum Reply {
    Inspected(Plan),
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
}
impl Job {
    fn start(operation: Operation, engine: &Engine) -> Result<Self, String> {
        let recording = matches!(&operation, Operation::Record { .. });
        let permit = if matches!(&operation, Operation::Export { .. }) {
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
        let (tx, result) = bounded(1);
        let thread=std::thread::Builder::new().name("omatainer-audio-delivery".into()).spawn(move||{
            let result=(||{
                let _running=if let Some(ticket)=&ticket{Some(ticket.enter(||stop.load(Ordering::Acquire))?)}else{crate::background::recording_priority()?;None};
                match &operation { Operation::ReviewRecovery(path)=>return data::recovery::review(path,&stop).map(Reply::Recovery), Operation::Recover(plan)=>return data::recovery::recover(plan,&stop).map(Reply::Recovered), _=>{} }
                let epoch=recorder.epoch();let captured=handle.capture(&stop).map_err(|e|e.to_string())?;
                match operation{
                    Operation::ReviewRecovery(_)|Operation::Recover(_)=>unreachable!(),
                    Operation::Inspect=>{
                        let layout=captured.state.session.as_ref().ok_or("Session identity unavailable")?;let model=captured.state.routing.as_ref();
                        let outputs=model.map_or(Vec::new(),|m|m.ports.iter().filter(|p|p.direction==Direction::Output&&(1..=2).contains(&p.channels.len())&&m.monitor_output!=Some(p.id)).map(|p|(p.id,p.alias.clone())).collect());
                        let records=model.map_or_else(||vec![RecordSource{id:live::STANDARD_OUTPUT,name:"Standard master output".into(),channels:Some(vec![0,1]),width:2}],|m|m.ports.iter().filter(|p|matches!(p.direction,Direction::Output|Direction::Record)&&p.channels.len()<=26&&m.monitor_output!=Some(p.id)).map(|p|RecordSource{id:p.id,name:format!("{} · {}",p.alias,if p.direction==Direction::Output{"final output"}else{"raw routed source"}),channels:(p.direction==Direction::Output).then(||p.channels.clone()),width:p.channels.len() as u16}).collect());
                        let scenes=layout.scene_order.iter().filter_map(|&slot|layout.scenes.get(usize::from(slot)).filter(|s|s.active).map(|s|(slot,s.name.clone()))).collect();
                        Ok(Reply::Inspected(Plan{revision:captured.revision,namespace:layout.namespace,generation:layout.generation,outputs,routed:model.is_some(),scenes,records,rate:handle.sample_rate()}))
                    },
                    Operation::Export{plan,request,path}=>{
                        let layout=captured.state.session.as_ref().ok_or("Session identity unavailable")?;if captured.revision!=plan.revision||layout.namespace!=plan.namespace||layout.generation!=plan.generation{return Err("Project changed since export review. Refresh the source before rendering".into());}
                        data::run(captured,&request,&path,permit.as_ref().unwrap(),&ticket.as_ref().unwrap().reporter()).map(Reply::Exported)
                    },
                    Operation::Record{plan,source,format,dither,seconds,path}=>{
                        let layout=captured.state.session.as_ref().ok_or("Session identity unavailable")?;if layout.namespace!=plan.namespace||layout.generation!=plan.generation||handle.sample_rate()!=plan.rate{return Err("Recording source changed since review; refresh before recording".into());}
                        if let Some(model)=&captured.state.routing{let direction=if source.channels.is_some(){Direction::Output}else{Direction::Record};let port=model.port(source.id,direction).ok_or("Reviewed recording alias is unavailable")?;if port.channels.len()!=usize::from(source.width)||source.channels.as_ref().is_some_and(|c|c!=&port.channels)||model.monitor_output==Some(source.id){return Err("Recording channel plan changed since review".into());}}else if source.id!=live::STANDARD_OUTPUT{return Err("Reviewed routing source was removed".into());}
                        let request=live::Request{alias:source.id,output_channels:source.channels,options:data::Options{format,rate:plan.rate,channels:source.width,dither,normalize:false},seconds,epoch};
                        recorder.write_delivery(&request,&path,&stop).map(Reply::Recorded)
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
                Ok(Reply::Inspected(plan)) => {
                    if self.audio_delivery.request.output_alias.is_none() && plan.outputs.len() == 1
                    {
                        self.audio_delivery.request.output_alias = Some(plan.outputs[0].0);
                    }
                    if !plan
                        .scenes
                        .iter()
                        .any(|(slot, _)| *slot == self.audio_delivery.request.scene)
                    {
                        self.audio_delivery.request.scene =
                            plan.scenes.first().map_or(0, |(slot, _)| *slot);
                    }
                    if self.audio_delivery.record_source.is_none() {
                        self.audio_delivery.record_source = plan.records.first().map(|s| s.id);
                    }
                    self.audio_delivery.plan = Some(plan);
                    self.audio_delivery.message="Source reviewed. Export uses a coherent snapshot and fresh DSP; live playback stays unchanged.".into();
                }
                Ok(Reply::Exported(o)) => {
                    self.audio_delivery.message = format!(
                        "Saved {} frames · {} Hz · {} channels · peak {:.3} · gain {:.6} to {}",
                        o.frames,
                        o.rate,
                        o.channels,
                        o.peak,
                        o.gain,
                        o.folder.display()
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
                ui.horizontal(|ui|{ui.selectable_value(&mut panel.request.source,Source::Arrangement,"Arrangement song");ui.selectable_value(&mut panel.request.source,Source::Session,tr!("Current session launches"));ui.selectable_value(&mut panel.request.source,Source::Scene,tr!("Selected scene"));});
                if panel.request.source==Source::Scene{egui::ComboBox::from_id_salt("audio-export-scene").selected_text(plan.scenes.iter().find(|(slot,_)|*slot==panel.request.scene).map_or("Choose a scene",|(_,name)|name.as_str())).show_ui(ui,|ui|{for(slot,name)in &plan.scenes{ui.selectable_value(&mut panel.request.scene,*slot,name);}});}
                ui.checkbox(&mut panel.request.decks,tr!("Include loaded decks from their saved cursors"));
                if plan.routed{egui::ComboBox::from_id_salt("audio-export-alias").selected_text(plan.outputs.iter().find(|(id,_)|Some(*id)==panel.request.output_alias).map_or("Choose program output",|(_,name)|name.as_str())).show_ui(ui,|ui|{for(id,name)in &plan.outputs{ui.selectable_value(&mut panel.request.output_alias,Some(*id),name);}});}else{panel.request.output_alias=None;ui.label(tr!("Standard stereo master output"));}
                ui.horizontal(|ui|{ui.label(tr!("Range start (seconds)"));ui.add(egui::DragValue::new(&mut panel.request.start).range(0.0..=28800.0).speed(0.1));ui.label(tr!("Range end (seconds)"));ui.add(egui::DragValue::new(&mut panel.request.end).range(0.0..=28800.0).speed(0.1));});
                ui.horizontal(|ui|{ui.label(tr!("Repeat range"));ui.add(egui::DragValue::new(&mut panel.request.repeats).range(1..=64));ui.label(tr!("Final tail (seconds)"));ui.add(egui::DragValue::new(&mut panel.request.tail).range(0.0..=120.0).speed(0.1));});
                egui::ComboBox::from_id_salt("audio-export-format").selected_text(panel.request.options.format.title()).show_ui(ui,|ui|{for format in Format::ALL{ui.selectable_value(&mut panel.request.options.format,format,format.title());}});
                ui.horizontal(|ui|{ui.label(tr!("Sample rate (Hz)"));ui.add(egui::DragValue::new(&mut panel.request.options.rate).range(8000..=192000).speed(100.0));ui.selectable_value(&mut panel.request.options.channels,1,tr!("Mono"));ui.selectable_value(&mut panel.request.options.channels,2,tr!("Stereo"));});
                if !panel.request.options.format.dither(){panel.request.options.dither=false;}ui.add_enabled(panel.request.options.format.dither(),egui::Checkbox::new(&mut panel.request.options.dither,tr!("Triangular dither for integer quantization")));ui.checkbox(&mut panel.request.options.normalize,tr!("Normalize peak to −1 dBFS"));
                ui.label(tr!("Ranges start at scene zero or the captured session cursor. The session uses its currently launched clips. Effects build from the source start; each repeat copies the same rendered range. Tail keeps the decay after sources stop. Metronome and headphone cue are excluded. FLAC/MP3 require FFmpeg."));
                ui.label(tr!("New export folder"));ui.add(egui::TextEdit::singleline(&mut panel.path).hint_text("/home/you/Music/Exports/new-mix").desired_width(f32::INFINITY));
                match panel.request.frames(){Ok((_,_,_,frames))=>{ui.label(format!("{} frames · {:.3} seconds",frames,frames as f64/f64::from(panel.request.options.rate)));if ui.button(tr!("Render master audio")).help(ui,HelpControl::AudioExport).clicked(){export=Some(Operation::Export{plan:plan.clone(),request:panel.request.clone(),path:PathBuf::from(&panel.path)});}},Err(e)=>{ui.label(e);}}
            }});
            ui.separator();
            ui.heading(tr!("Record the performance"));
            ui.add_enabled_ui(!busy,|ui|{if let Some(plan)=&panel.plan{
                if panel.record_seconds==0{panel.record_seconds=3600;}
                egui::ComboBox::from_id_salt("performance-record-source").selected_text(plan.records.iter().find(|s|Some(s.id)==panel.record_source).map_or("Choose a recording source",|s|s.name.as_str())).show_ui(ui,|ui|{for source in &plan.records{ui.selectable_value(&mut panel.record_source,Some(source.id),&source.name);}});
                egui::ComboBox::from_id_salt("performance-record-format").selected_text(panel.record_format.title()).show_ui(ui,|ui|{for format in Format::ALL{ui.selectable_value(&mut panel.record_format,format,format.title());}});
                if !panel.record_format.dither(){panel.record_dither=false;}ui.add_enabled(panel.record_format.dither(),egui::Checkbox::new(&mut panel.record_dither,tr!("Dither the recording to integer PCM")));
                ui.horizontal(|ui|{ui.label(tr!("Maximum recording seconds"));ui.add(egui::DragValue::new(&mut panel.record_seconds).range(1..=43200));});
                ui.label(tr!("New recording folder"));ui.add(egui::TextEdit::singleline(&mut panel.record_path).hint_text("/home/you/Music/Recordings/new-set").desired_width(f32::INFINITY));
                ui.label(tr!("Final output includes the limiter, output conversion and recovery fades. Raw routed sources include their chosen return and tap. WAV segments split automatically at 256 MiB. Stop preserves valid audio; FLAC/MP3 encoding follows capture and keeps the original WAV files. Recording is available during Performance Mode."));
                if let Some(source)=plan.records.iter().find(|s|Some(s.id)==panel.record_source){ui.label(format!("{} Hz · {} channels in saved order",plan.rate,source.width));if ui.button(tr!("Start performance recording")).help(ui,HelpControl::PerformanceRecording).clicked(){export=Some(Operation::Record{plan:plan.clone(),source:source.clone(),format:panel.record_format,dither:panel.record_dither,seconds:panel.record_seconds,path:PathBuf::from(&panel.record_path)});}}
            }});
            ui.separator();ui.heading(tr!("Recover an interrupted recording"));
            ui.add_enabled_ui(!busy,|ui|{ui.label(tr!("Existing recording folder"));ui.add(egui::TextEdit::singleline(&mut panel.recovery_path).hint_text("/home/you/Music/Recordings/interrupted-set").desired_width(f32::INFINITY));if ui.button(tr!("Review recording recovery")).clicked(){export=Some(Operation::ReviewRecovery(PathBuf::from(&panel.recovery_path)));}if let Some(plan)=&panel.recovery{ui.label(format!("{} frames · {} Hz · {} channels · {} files need repair",plan.frames,plan.rate,plan.channels,plan.repairs));if ui.button(tr!("Recover reviewed recording")).clicked(){export=Some(Operation::Recover(plan.clone()));}}});
            if let Some(job)=&panel.job{if job.recording{ui.label(format!("Recorded source frames: {} · peak {:.3}",self.engine.routing.recorder.frames(),self.engine.routing.recorder.delivery_peak()));}if let Some((done,total))=job.progress.values(){if let Some(total)=total.filter(|t|*t>0){ui.add(egui::ProgressBar::new(done as f32/total as f32).text(format!("{} / {} frames",done,total)));}else{ui.label(tr!("Waiting for worker or encoding"));}}if ui.button(if job.recording{tr!("Stop performance recording")}else{tr!("Cancel audio work")}).clicked(){job.cancel.store(true,Ordering::Release);}}
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
