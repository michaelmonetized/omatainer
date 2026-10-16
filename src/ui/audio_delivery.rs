use super::*;
use crate::audio_delivery::{self as data, Export, Format, Source};
use crate::engine::audio::routing::model::Direction;
use crate::engine::audio::routing::record::delivery as live;
use crossbeam_channel::{bounded, Receiver};
use std::sync::atomic::{AtomicBool, Ordering};
mod finishing;
mod batch;
#[cfg(test)]
mod tests;
#[cfg(test)]
pub(super) mod keyboard_tests;

/// Name an existing delivery text field without changing its draft or parser.
/// Args: UI, property salt, visible purpose, draft and existing width choice. Returns the actual native response.
fn text(
    ui: &mut egui::Ui,
    salt: &str,
    name: &str,
    value: &mut String,
    full_width: bool,
) -> egui::Response {
    let id = keyboard_authoring::property(ui, salt);
    let label = ui.label(name);
    let width = if full_width {
        f32::INFINITY
    } else {
        ui.spacing().text_edit_width
    };
    let hint = match salt {
        "export-folder" => "/home/you/Music/Exports/new-mix",
        "record-folder" => "/home/you/Music/Recordings/new-set",
        "recovery-folder" => "/home/you/Music/Recordings/interrupted-set",
        _ => "",
    };
    let response = ui
        .add(
            egui::TextEdit::singleline(value)
                .id(id)
                .hint_text(hint)
                .desired_width(width),
        )
        .labelled_by(label.id);
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, response.enabled(), name)
    });
    accessibility::focus(ui, &response);
    response
}

/// Format the original numeric type without losing precision through f32 or f64 casts.
/// Args: original typed value, native value and native decimal bounds. Returns exact text, retaining native padding when it roundtrips.
fn number_text<N: egui::emath::Numeric + std::fmt::Display + std::str::FromStr>(
    original: N,
    value: f64,
    decimals: std::ops::RangeInclusive<usize>,
) -> String {
    let typed = if value == original.to_f64() {
        original
    } else {
        N::from_f64(value)
    };
    if !N::INTEGRAL {
        for precision in decimals {
            let text = format!("{value:.precision$}");
            if text.parse::<N>().ok() == Some(typed) {
                return text;
            }
        }
    }
    typed.to_string()
}

/// Decorate a delivery DragValue while retaining its native range, step and numeric type.
/// Args: UI, property salt, purpose with units, value and the original optional range/step overrides. Returns the actual native response.
fn number<N: egui::emath::Numeric + std::fmt::Display + std::str::FromStr>(
    ui: &mut egui::Ui,
    salt: &str,
    name: &str,
    value: &mut N,
    range: Option<std::ops::RangeInclusive<N>>,
    speed: Option<f64>,
) -> egui::Response {
    let id = keyboard_authoring::property(ui, salt);
    keyboard_authoring::native(ui, id, |ui| {
        let label = ui.label(name);
        let before = *value;
        let (text_input, edit_key, actions) = ui.input(|input| {
            (
                input.events.iter().any(|event| matches!(event, egui::Event::Text(_) | egui::Event::Paste(_) | egui::Event::Cut)),
                input.events.iter().any(|event| matches!(event,
                    egui::Event::Key { key, pressed: true, modifiers, .. }
                    if matches!(key, Key::ArrowUp | Key::ArrowDown | Key::Backspace | Key::Delete)
                        || (modifiers.command || modifiers.ctrl) && matches!(key, Key::Z | Key::Y | Key::X | Key::V | Key::K | Key::U | Key::W))),
                input.events.iter().filter_map(|event| match event {
                    egui::Event::AccessKitActionRequest(request) if matches!(request.action,
                        egui::accesskit::Action::SetValue | egui::accesskit::Action::Increment | egui::accesskit::Action::Decrement) => Some(request.target),
                    _ => None,
                }).collect::<Vec<_>>(),
            )
        });
        let attempted = std::cell::RefCell::new(None);
        let parse = |text: &str| {
            text.chars()
                .filter(|character| !character.is_whitespace())
                .map(|character| if character == '−' { '-' } else { character })
                .collect::<String>()
                .parse::<f64>()
                .ok()
                .filter(|value| value.is_finite())
        };
        let mut drag = egui::DragValue::new(value)
            .custom_formatter(|native, decimals| number_text(before, native, decimals))
            .custom_parser(|draft| {
                attempted.replace(Some(draft.to_owned()));
                parse(draft)
            });
        if let Some(range) = range {
            drag = drag.range(range);
        }
        if let Some(speed) = speed {
            drag = drag.speed(speed);
        }
        let response = ui.add(drag).labelled_by(label.id);
        let draft = id.with("invalid-native-draft");
        let cancelled = (response.has_focus() || response.lost_focus())
            && ui.input(|input| input.key_pressed(Key::Escape));
        ui.data_mut(|data| {
            if let Some(text) = attempted.into_inner() {
                if parse(&text).is_none() {
                    data.insert_temp(draft, text);
                } else {
                    data.remove::<String>(draft);
                }
            } else if response.changed() {
                data.remove::<String>(draft);
            }
            if cancelled {
                data.remove::<String>(draft);
            }
        });
        let invalid = ui.data(|data| data.get_temp::<String>(draft).is_some());
        ui.data_mut(|data| data.insert_temp(response.id.with("delivery-number-valid"), !invalid));
        let accepted = response.changed()
            && (response.dragged()
                || (response.has_focus() || response.lost_focus()) && (text_input || edit_key)
                || actions.contains(&response.id.value().into()));
        if invalid || cancelled || !response.enabled() || !accepted {
            *value = before;
        }
        response.widget_info(|| {
            let mut info = egui::WidgetInfo::drag_value(response.enabled(), value.to_f64());
            info.label = Some(name.into());
            info
        });
        accessibility::focus(ui, &response);
        if response.has_focus() {
            ui.memory_mut(|memory| {
                memory.set_focus_lock_filter(
                    response.id,
                    egui::EventFilter {
                        vertical_arrows: true,
                        horizontal_arrows: true,
                        ..Default::default()
                    },
                );
            });
        }
        if invalid {
            keyboard_authoring::error(ui, &response, "Enter a finite number");
        }
        response
    })
}

/// Read this row decorator's public-parser validation state.
/// Args: actual native number response. Returns whether its owned attempted draft is valid.
fn number_is_valid(response: &egui::Response) -> bool {
    response.ctx.data(|data| {
        data.get_temp::<bool>(response.id.with("delivery-number-valid"))
            .unwrap_or(true)
    })
}


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
                        let outputs=model.map_or(Vec::new(),|m|m.ports.iter().filter(|p|p.direction==Direction::Output&&(1..=2).contains(&p.channels.len())&&m.monitor_output!=Some(p.id)).map(|p|(p.id,p.alias.clone())).collect());
                        let records=model.map_or_else(||vec![RecordSource{id:live::STANDARD_OUTPUT,name:"Standard master output".into(),channels:Some(vec![0,1]),width:2}],|m|m.ports.iter().filter(|p|matches!(p.direction,Direction::Output|Direction::Record)&&p.channels.len()<=26&&m.monitor_output!=Some(p.id)).map(|p|RecordSource{id:p.id,name:format!("{} · {}",p.alias,if p.direction==Direction::Output{"final output"}else{"raw routed source"}),channels:(p.direction==Direction::Output).then(||p.channels.clone()),width:p.channels.len() as u16}).collect());
                        let scenes=layout.scene_order.iter().filter_map(|&slot|layout.scenes.get(usize::from(slot)).filter(|s|s.active).map(|s|(slot,s.name.clone()))).collect();
                        Ok(Reply::Inspected(Plan{revision:captured.revision,namespace:layout.namespace,generation:layout.generation,outputs,routed:model.is_some(),scenes,scene_refs:layout.scene_order.iter().filter_map(|&slot|layout.reference(crate::engine::session::Axis::Scene,usize::from(slot)).map(|r|(slot,r))).collect(),records,rate:handle.sample_rate(),catalog:data::batch::Recipe::catalog(&captured)?,batch:data::batch::Recipe::aligned(&captured,Export::default(),data::finishing::Config::default())?}))
                    },
                    Operation::Export{plan,request,config,path}=>{
                        let layout=captured.state.session.as_ref().ok_or("Session identity unavailable")?;if captured.revision!=plan.revision||layout.namespace!=plan.namespace||layout.generation!=plan.generation{return Err("Project changed since export review. Refresh the source before rendering".into());}
                        data::run_selected(captured,&request,None,&config,&path,permit.as_ref().unwrap(),&ticket.as_ref().unwrap().reporter()).map(Reply::Exported)
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
        let draft_key = egui::Id::new("audio-delivery-native-draft");
        if !self.audio_delivery.open {
            ctx.data_mut(|data| data.remove::<egui::Id>(draft_key));
            return;
        }
        static NEXT_DRAFT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let draft = ctx.data_mut(|data| {
            *data.get_temp_mut_or_insert_with(draft_key, || {
                egui::Id::new(NEXT_DRAFT.fetch_add(1, Ordering::Relaxed))
            })
        });
        let namespace = self
            .snap
            .session
            .as_ref()
            .map_or([0; 2], |session| session.namespace);
        let mut open = true;
        let mut inspect = false;
        let mut close = false;
        let mut export = None;
        egui::Window::new(tr!("Audio export & recording")).open(&mut open).default_width(640.0).show(ctx, |ui| {
            keyboard_authoring::target(ui, ("audio-delivery", namespace, draft), |ui| {
                let panel = &mut self.audio_delivery;
                let busy = panel.job.is_some();
                ui.label(tr!("Export a scene or the current session without changing playback. Choose a new output folder."));
                if keyboard_authoring::action(ui, &tr!("Review export source"), !busy).help(ui, HelpControl::AudioExport).clicked() { inspect = true; }
                ui.add_enabled_ui(!busy, |ui| {
                    if let Some(plan) = panel.plan.clone() {
                        keyboard_authoring::form(ui, keyboard_authoring::property(ui, "export-settings"), |ui| {
                            let previous_source = panel.request.source;
                            ui.horizontal(|ui| {
                                for (value, label) in [(Source::Arrangement, "Arrangement song"), (Source::Session, tr!("Current session launches")), (Source::Scene, tr!("Selected scene"))] {
                                    let response = keyboard_authoring::native(ui, keyboard_authoring::property(ui, ("source", value as u8)), |ui| ui.selectable_value(&mut panel.request.source, value, label));
                                    accessibility::focus(ui, &response);
                                }
                            });
                            if previous_source != panel.request.source && panel.request.source == Source::Scene {
                                if let Some(recipe) = &mut panel.batch { recipe.source_scene = plan.scene_refs.iter().find(|(slot, _)| *slot == panel.request.scene).map(|(_, reference)| *reference); }
                            }
                            if panel.request.source == Source::Scene {
                                let response = egui::ComboBox::new("audio-export-scene", "Export scene").selected_text(plan.scenes.iter().find(|(slot, _)| *slot == panel.request.scene).map_or("Choose a scene", |(_, name)| name.as_str())).show_ui(ui, |ui| {
                                    for (slot, name) in &plan.scenes {
                                        let saved = plan.scene_refs.iter().find(|(s, _)| s == slot).map(|(_, reference)| reference.id);
                                        let response = keyboard_authoring::native(ui, keyboard_authoring::property(ui, ("scene", saved)), |ui| ui.selectable_value(&mut panel.request.scene, *slot, name));
                                        if response.clicked() { if let Some(recipe) = &mut panel.batch { recipe.source_scene = plan.scene_refs.iter().find(|(s, _)| s == slot).map(|(_, reference)| *reference); } }
                                        accessibility::focus(ui, &response);
                                    }
                                }).response;
                                accessibility::focus(ui, &response);
                            }
                            let response = keyboard_authoring::native(ui, keyboard_authoring::property(ui, "include-decks"), |ui| ui.checkbox(&mut panel.request.decks, tr!("Include loaded decks from their saved cursors")));
                            accessibility::focus(ui, &response);
                            if plan.routed {
                                let response = egui::ComboBox::new("audio-export-alias", "Export program output").selected_text(plan.outputs.iter().find(|(id, _)| Some(*id) == panel.request.output_alias).map_or("Choose program output", |(_, name)| name.as_str())).show_ui(ui, |ui| {
                                    for (id, name) in &plan.outputs {
                                        let response = keyboard_authoring::native(ui, keyboard_authoring::property(ui, ("output", id)), |ui| ui.selectable_value(&mut panel.request.output_alias, Some(*id), name));
                                        accessibility::focus(ui, &response);
                                    }
                                }).response;
                                accessibility::focus(ui, &response);
                            } else { panel.request.output_alias = None; ui.label(tr!("Standard stereo master output")); }
                            let mut end_response = None;
                            ui.horizontal(|ui| {
                                number(ui, "range-start", &tr!("Range start (seconds)"), &mut panel.request.start, Some(0.0..=28800.0), Some(0.1));
                                end_response = Some(number(ui, "range-end", &tr!("Range end (seconds)"), &mut panel.request.end, Some(0.0..=28800.0), Some(0.1)));
                            });
                            ui.horizontal(|ui| {
                                number(ui, "repeats", &format!("{} (times)", tr!("Repeat range")), &mut panel.request.repeats, Some(1..=64), None);
                                number(ui, "tail", &tr!("Final tail (seconds)"), &mut panel.request.tail, Some(0.0..=120.0), Some(0.1));
                            });
                            let response = egui::ComboBox::new("audio-export-format", "Export format").selected_text(panel.request.options.format.title()).show_ui(ui, |ui| {
                                for format in Format::ALL {
                                    let response = keyboard_authoring::native(ui, keyboard_authoring::property(ui, ("format", format as u8)), |ui| ui.selectable_value(&mut panel.request.options.format, format, format.title()));
                                    accessibility::focus(ui, &response);
                                }
                            }).response;
                            accessibility::focus(ui, &response);
                            ui.horizontal(|ui| {
                                number(ui, "sample-rate", &tr!("Sample rate (Hz)"), &mut panel.request.options.rate, Some(8000..=192000), Some(100.0));
                                for (channels, name) in [(1, tr!("Mono")), (2, tr!("Stereo"))] {
                                    let response = keyboard_authoring::native(ui, keyboard_authoring::property(ui, ("channels", channels)), |ui| ui.selectable_value(&mut panel.request.options.channels, channels, name));
                                    accessibility::focus(ui, &response);
                                }
                            });
                            if !panel.request.options.format.dither() { panel.request.options.dither = false; }
                            let response = keyboard_authoring::native(ui, keyboard_authoring::property(ui, "dither"), |ui| ui.add_enabled(panel.request.options.format.dither(), egui::Checkbox::new(&mut panel.request.options.dither, tr!("Triangular dither for integer quantization"))));
                            accessibility::focus(ui, &response);
                            let response = keyboard_authoring::native(ui, keyboard_authoring::property(ui, "normalize"), |ui| ui.checkbox(&mut panel.request.options.normalize, tr!("Normalize sample peak to −1 dBFS")));
                            accessibility::focus(ui, &response);
                            ui.label(tr!("Ranges start at scene zero or the captured session cursor. The session uses its currently launched clips. Effects build from the source start; each repeat copies the same rendered range. Tail keeps the decay after sources stop. Metronome and headphone cue are excluded. FLAC/MP3 require FFmpeg."));
                            finishing::controls(ui, &mut panel.config, panel.request.options.format);
                            text(ui, "export-folder", &tr!("New export folder"), &mut panel.path, true);
                            match panel.request.frames() {
                                Ok((_, _, _, frames)) => {
                                    ui.label(format!("{} frames · {:.3} seconds", frames, frames as f64 / f64::from(panel.request.options.rate)));
                                    if keyboard_authoring::action(ui, &tr!("Render master audio"), !keyboard_authoring::invalid(ui) && panel.config.validate().is_ok()).help(ui, HelpControl::AudioExport).clicked() {
                                        export = Some(Operation::Export { plan: plan.clone(), request: panel.request.clone(), config: panel.config.clone(), path: PathBuf::from(&panel.path) });
                                    }
                                }
                                Err(error) => { if !keyboard_authoring::invalid(ui) { keyboard_authoring::error(ui, end_response.as_ref().unwrap(), &error); } }
                            }
                            let reviewed = plan.clone();
                            if let Some(operation) = batch::controls(ui, panel, &reviewed) { export = Some(operation); }
                        });
                    }
                });
                ui.separator();
                ui.heading(tr!("Record the performance"));
                ui.add_enabled_ui(!busy, |ui| {
                    if let Some(plan) = panel.plan.clone() {
                        keyboard_authoring::form(ui, keyboard_authoring::property(ui, "record-settings"), |ui| {
                            if panel.record_seconds == 0 { panel.record_seconds = 3600; }
                            let response = egui::ComboBox::new("performance-record-source", "Recording source").selected_text(plan.records.iter().find(|source| Some(source.id) == panel.record_source).map_or("Choose a recording source", |source| source.name.as_str())).show_ui(ui, |ui| {
                                for source in &plan.records {
                                    let response = keyboard_authoring::native(ui, keyboard_authoring::property(ui, ("record-source", source.id)), |ui| ui.selectable_value(&mut panel.record_source, Some(source.id), &source.name));
                                    accessibility::focus(ui, &response);
                                }
                            }).response;
                            accessibility::focus(ui, &response);
                            let response = egui::ComboBox::new("performance-record-format", "Recording format").selected_text(panel.record_format.title()).show_ui(ui, |ui| {
                                for format in Format::RECORDING {
                                    let response = keyboard_authoring::native(ui, keyboard_authoring::property(ui, ("record-format", format as u8)), |ui| ui.selectable_value(&mut panel.record_format, format, format.title()));
                                    accessibility::focus(ui, &response);
                                }
                            }).response;
                            accessibility::focus(ui, &response);
                            if !panel.record_format.dither() { panel.record_dither = false; }
                            let response = keyboard_authoring::native(ui, keyboard_authoring::property(ui, "record-dither"), |ui| ui.add_enabled(panel.record_format.dither(), egui::Checkbox::new(&mut panel.record_dither, tr!("Dither the recording to integer PCM"))));
                            accessibility::focus(ui, &response);
                            number(ui, "record-seconds", &tr!("Maximum recording seconds"), &mut panel.record_seconds, Some(1..=43200), None);
                            text(ui, "record-folder", &tr!("New recording folder"), &mut panel.record_path, true);
                            ui.label(tr!("Final output includes the limiter, output conversion and recovery fades. Raw routed sources include their chosen return and tap. WAV segments split automatically at 256 MiB. Stop preserves valid audio; FLAC/MP3 encoding follows capture and keeps the original WAV files. Recording is available during Performance Mode."));
                            if let Some(source) = plan.records.iter().find(|source| Some(source.id) == panel.record_source) {
                                ui.label(format!("{} Hz · {} channels in saved order", plan.rate, source.width));
                                if keyboard_authoring::action(ui, &tr!("Start performance recording"), !keyboard_authoring::invalid(ui)).help(ui, HelpControl::PerformanceRecording).clicked() {
                                    export = Some(Operation::Record { plan: plan.clone(), source: source.clone(), format: panel.record_format, dither: panel.record_dither, seconds: panel.record_seconds, path: PathBuf::from(&panel.record_path) });
                                }
                            }
                        });
                    }
                });
                ui.separator();
                ui.heading(tr!("Recover an interrupted recording"));
                ui.add_enabled_ui(!busy, |ui| {
                    text(ui, "recovery-folder", &tr!("Existing recording folder"), &mut panel.recovery_path, true);
                    if keyboard_authoring::action(ui, &tr!("Review recording recovery"), true).clicked() { export = Some(Operation::ReviewRecovery(PathBuf::from(&panel.recovery_path))); }
                    if let Some(plan) = &panel.recovery {
                        ui.label(format!("{} frames · {} Hz · {} channels · {} files need repair", plan.frames, plan.rate, plan.channels, plan.repairs));
                        if keyboard_authoring::action(ui, &tr!("Recover reviewed recording"), true).clicked() { export = Some(Operation::Recover(plan.clone())); }
                    }
                });
                if let Some(job) = &panel.job {
                    let status = job.batch_progress.lock().unwrap().clone();
                    if status.total > 0 { ui.label(format!("Pass {} / {} · {} · {}", status.item, status.total, status.name, status.phase)); }
                    if job.recording { ui.label(format!("Recorded source frames: {} · peak {:.3}", self.engine.routing.recorder.frames(), self.engine.routing.recorder.delivery_peak())); }
                    if let Some((done, total)) = job.progress.values() {
                        if let Some(total) = total.filter(|total| *total > 0) { ui.add(egui::ProgressBar::new(done as f32 / total as f32).text(format!("{} / {} frames", done, total))); }
                        else { ui.label(tr!("Waiting for worker or encoding")); }
                    }
                    if keyboard_authoring::action(ui, if job.recording { &tr!("Stop performance recording") } else { &tr!("Cancel audio work") }, true).clicked() { job.cancel.store(true, Ordering::Release); }
                }
                ui.label(&panel.message);
                if keyboard_authoring::action(ui, "Close audio export & recording", true).clicked() { close = true; }
            });
        });
        self.audio_delivery.open = open && !close;
        if !self.audio_delivery.open {
            ctx.data_mut(|data| data.remove::<egui::Id>(draft_key));
        }
        if inspect {
            self.start_audio_delivery(Operation::Inspect);
        }
        if let Some(operation) = export {
            self.start_audio_delivery(operation);
        }
    }
}
