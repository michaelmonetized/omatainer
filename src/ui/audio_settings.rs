//! Confirmed device operations and calibration run on a bounded worker lane.
use super::*;
use crate::engine::audio::{calibration, config, owner};
use crate::preferences::{Audio, AudioFormat};
use crossbeam_channel::{bounded, Receiver, Sender};
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Clone)]
struct Preview {
    profile: String,
    saved: Audio,
    inventory: config::Inventory,
    output: Result<config::Plan, String>,
    calibration: Result<calibration::Request, String>,
}
enum Job {
    Preview(String, Audio),
    Apply(Preview),
    Calibrate(Preview),
}
enum Event {
    Preview(Preview),
    Applied(Preview, Result<Arc<owner::Status>, String>),
    Calibrated(Result<Arc<owner::Status>, String>),
    Failed(String),
}
struct Worker {
    sender: Sender<(Job, Arc<AtomicBool>)>,
    receiver: Receiver<Event>,
    cancel: Option<Arc<AtomicBool>>,
}
impl Worker {
    fn start(
        handle: owner::Handle,
        discover: impl Fn() -> Result<config::Inventory, String> + Send + 'static,
    ) -> std::io::Result<Self> {
        let (sender, input) = bounded::<(Job, Arc<AtomicBool>)>(1);
        let (output, receiver) = bounded(1);
        std::thread::Builder::new()
            .name("omatainer-audio-settings".into())
            .spawn(move || {
                while let Ok((job, cancel)) = input.recv() {
                    let event = match job {
                        Job::Preview(profile, saved) => match discover() {
                            Ok(inventory) => {
                                let target = config::plan(&saved, &inventory);
                                let calibration = handle
                                    .status()
                                    .active
                                    .as_ref()
                                    .ok_or_else(|| {
                                        "Recover a session output before calibration".into()
                                    })
                                    .and_then(|active| {
                                        let input = config::calibration_plan(
                                            &saved,
                                            &inventory,
                                            &active.plan,
                                        )?;
                                        Ok(calibration::Request {
                                            profile: profile.clone(),
                                            input,
                                            output: active.plan.clone(),
                                            input_channel: saved.calibration.channel,
                                            output_channel: saved.calibration.output_channel,
                                            level_db: saved.calibration.level_db,
                                        })
                                    });
                                if cancel.load(Ordering::Acquire) {
                                    Event::Failed("Preview cancelled".into())
                                } else {
                                    Event::Preview(Preview {
                                        profile,
                                        saved,
                                        inventory,
                                        output: target,
                                        calibration,
                                    })
                                }
                            }
                            Err(error) => Event::Failed(error),
                        },
                        Job::Apply(preview) => {
                            let result = preview.output.clone().and_then(|plan| {
                                handle.apply_preview(preview.saved.clone(), plan, cancel)
                            });
                            Event::Applied(preview, result)
                        }
                        Job::Calibrate(preview) => Event::Calibrated(
                            preview
                                .calibration
                                .and_then(|request| handle.calibrate(request, cancel)),
                        ),
                    };
                    if output.send(event).is_err() {
                        break;
                    }
                }
            })?;
        Ok(Self {
            sender,
            receiver,
            cancel: None,
        })
    }
    fn request(&mut self, job: Job) -> Result<(), String> {
        if self.cancel.is_some() {
            return Err("An audio operation is already pending".into());
        }
        let cancel = Arc::new(AtomicBool::new(false));
        self.sender
            .try_send((job, cancel.clone()))
            .map_err(|_| "Audio settings worker unavailable")?;
        self.cancel = Some(cancel);
        Ok(())
    }
    fn poll(&mut self) -> Option<Event> {
        match self.receiver.try_recv() {
            Ok(event) => {
                self.cancel = None;
                Some(event)
            }
            Err(crossbeam_channel::TryRecvError::Disconnected) if self.cancel.take().is_some() => {
                Some(Event::Failed("Audio settings worker closed".into()))
            }
            Err(_) => None,
        }
    }
    fn cancel(&self) {
        if let Some(cancel) = &self.cancel {
            cancel.store(true, Ordering::Release);
        }
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.cancel();
    }
}
#[derive(Clone, Copy)]
enum Confirm {
    Switch,
    Calibrate,
}
pub(super) struct Panel {
    pub open: bool,
    handle: Option<owner::Handle>,
    worker: Option<Worker>,
    preview: Option<Preview>,
    confirm: Option<Confirm>,
    message: String,
}
impl Panel {
    pub fn new(handle: Option<owner::Handle>) -> Self {
        Self::with_discovery(handle, config::discover)
    }
    fn with_discovery(
        handle: Option<owner::Handle>,
        discover: impl Fn() -> Result<config::Inventory, String> + Send + 'static,
    ) -> Self {
        let worker = handle
            .clone()
            .map(|handle| Worker::start(handle, discover))
            .transpose();
        let message = worker
            .as_ref()
            .err()
            .map(|e| e.to_string())
            .unwrap_or_default();
        Self {
            open: false,
            handle,
            worker: worker.ok().flatten(),
            preview: None,
            confirm: None,
            message,
        }
    }
    pub(super) fn notice(&self) -> Option<(&'static str, String)> {
        if self.busy() {
            return Some(("Audio operation pending", self.message.clone()));
        }
        if let Some(status) = self.handle.as_ref().map(owner::Handle::status) {
            if status.phase == owner::Phase::Offline {
                return Some(("Audio offline", status.message.clone()));
            }
        }
        (!self.message.is_empty()).then(|| ("Audio settings update", self.message.clone()))
    }
    fn busy(&self) -> bool {
        self.worker
            .as_ref()
            .is_some_and(|worker| worker.cancel.is_some())
    }
    fn request(&mut self, job: Job) {
        match self
            .worker
            .as_mut()
            .ok_or("Audio settings worker unavailable".into())
            .and_then(|worker| worker.request(job))
        {
            Ok(()) => self.message = "Audio operation pending".into(),
            Err(error) => self.message = error,
        }
    }
}
impl App {
    pub(super) fn poll_audio_settings(&mut self, ctx: &egui::Context) {
        while let Some(event) = self.audio_settings.worker.as_mut().and_then(Worker::poll) {
            match event {
                Event::Preview(preview) => {
                    self.settings.inventory = Some(preview.inventory.clone());
                    self.audio_settings.preview = Some(preview);
                    self.audio_settings.message =
                        "Preview ready; no device has changed and no probe has played".into();
                }
                Event::Applied(preview, Ok(status)) => {
                    if status.active.as_ref().is_some_and(|active| {
                        preview
                            .output
                            .as_ref()
                            .is_ok_and(|plan| plan == &active.plan)
                    }) {
                        self.settings.running_audio = preview.saved;
                    }
                    self.audio_settings.message = status.message.clone();
                }
                Event::Calibrated(Ok(status)) => {
                    self.audio_settings.message = status.message.clone()
                }
                Event::Applied(_, Err(error))
                | Event::Calibrated(Err(error))
                | Event::Failed(error) => self.audio_settings.message = error,
            }
        }
        if self.audio_settings.preview.as_ref().is_some_and(|preview| {
            preview.profile != self.settings.applied.active
                || preview.saved != self.settings.profile().audio
        }) {
            self.audio_settings.preview = None;
            self.audio_settings.confirm = None;
        }
        if self.audio_settings.busy() {
            ctx.request_repaint_after(std::time::Duration::from_millis(30));
        }
    }
    pub(super) fn audio_settings_ui(&mut self, ctx: &egui::Context) {
        if !self.audio_settings.open {
            return;
        }
        keyboard::block_for_dialog(ctx);
        let saved = self.settings.profile().audio.clone();
        let profile = self.settings.applied.active.clone();
        let metrics = self.engine.cmd.audio_metrics();
        let panel = &mut self.audio_settings;
        let mut open = true;
        egui::Window::new("Audio devices and latency").id(egui::Id::new("audio-devices-window")).open(&mut open).default_width(730.0).default_height(650.0).vscroll(true).show(ctx,|ui|{
            ui.label(format!("Saved profile: {profile}. Save edits in Preferences before previewing."));
            if let Some(handle)=&panel.handle {
                let status=handle.status();ui.label(&status.message).help(ui, HelpControl::AudioNotice);
                if let Some(active)=&status.active {
                    ui.label(format!("Backend-accepted logical output: {} / {} · {} Hz · {} · {} channels · buffer {}",active.backend,active.plan.device,active.plan.rate,active.format,active.plan.channels,active.plan.buffer.map(|n|n.to_string()).unwrap_or("backend default".into()))).help(ui, HelpControl::AudioActive);
                    ui.label(active.plan.route());
                    if let Some(measured)=metrics.last_callback.filter(|_|status.phase==owner::Phase::Running && metrics.callbacks>status.callback_floor) {
                        ui.label(format!("Observed output callback: {} frames ({:.3} ms at logical rate)",measured.frames,measured.frames as f64*1000.0/measured.sample_rate.max(1) as f64)).help(ui, HelpControl::AudioTiming);
                        ui.label(measured.output_latency_ns.map(|ns|format!("Backend output scheduling estimate: {:.3} ms",ns as f64/1e6)).unwrap_or("Backend output scheduling estimate unavailable".into())).help(ui, HelpControl::AudioTiming);
                    } else {ui.label("Waiting for output callback observations for this stream");}
                }else{ui.colored_label(Color32::YELLOW,"No active output. Session retained; Save, New/Open and Close remain available.");}
                ui.label("Physical negotiated sample rate, converter latency and exact driver roundtrip are unavailable through this backend API.");
                if let Some(evidence)=status.measurement.as_ref().filter(|e|!panel.busy() && e.identity.profile==profile && panel.preview.as_ref().and_then(|p|p.calibration.as_ref().ok())==Some(&e.identity)) {
                    let m=&evidence.measured;
                    ui.label(format!("Measured host callback-to-callback loopback return: {:.3} ms · nominal callback resolution {:.3} ms · repeat spread {:.3} ms",m.host_return_ns as f64/1e6,m.callback_resolution_ns as f64/1e6,m.spread().as_secs_f64()*1000.0)).help(ui, HelpControl::AudioMeasurement);
                    ui.label(format!("Measured route: {} input {} ← {} output {}; profile {}",evidence.identity.input.device,evidence.identity.input_channel+1,evidence.identity.output.device,evidence.identity.output_channel+1,evidence.identity.profile));
                } else {ui.label("Measured loopback return unavailable for the current preview; no physical measurement is inferred.").help(ui, HelpControl::AudioMeasurement);}
            }else{ui.label("No audio owner in this session");}
            ui.label(&panel.message).help(ui, HelpControl::AudioNotice);
            if !panel.busy() && !panel.message.is_empty() && ui.button("Dismiss audio notice").help(ui, HelpControl::AudioNotice).clicked(){panel.message.clear();}
            if panel.busy(){if ui.button("Cancel audio operation").help(ui, HelpControl::AudioCancel).clicked(){panel.worker.as_ref().unwrap().cancel();}}
            let allowed=!panel.busy()&&!self.project.committing()&&self.project.dialog_is_closed();
            ui.add_enabled_ui(allowed,|ui|{
                if ui.button("Preview saved audio").help(ui, HelpControl::AudioPreview).clicked(){panel.request(Job::Preview(profile.clone(),saved.clone()));}
                if let Some(preview)=panel.preview.clone(){
                    match &preview.output {Ok(plan)=>{ui.label(format!("Proposed output: {} / {} · {} Hz · {} · {} channels · {}",plan.backend,plan.device,plan.rate,plan.format,plan.channels,plan.route()));if let Some(warning)=&plan.warning{ui.label(warning);}},Err(error)=>{ui.colored_label(Color32::YELLOW,error);}}
                    if ui.add_enabled(preview.output.is_ok(),egui::Button::new("Use saved audio now")).help(ui, HelpControl::AudioUse).clicked(){panel.confirm=Some(Confirm::Switch);}
                    match &preview.calibration {
                        Ok(request)=>{
                            ui.label(format!("Calibration input: {} · {} Hz · {} · {} channels",request.input.device,request.input.rate,request.input.format,request.input.channels));
                            if let (Some(input),Some(output))=(request.input.buffer,request.output.buffer){ui.label(format!("Roundtrip buffer estimate: {:.3} ms (requested buffers only; driver and converter time excluded)",(input as f64+output as f64)*1000.0/request.output.rate as f64)).help(ui, HelpControl::AudioBufferEstimate);}else{ui.label("Roundtrip buffer estimate unavailable: one or both buffer sizes are backend-selected").help(ui, HelpControl::AudioBufferEstimate);}
                        },Err(error)=>{ui.label(format!("Calibration unavailable: {error}"));}
                    }
                    if ui.add_enabled(preview.calibration.is_ok(),egui::Button::new("Measure loopback")).help(ui, HelpControl::AudioMeasure).clicked(){panel.confirm=Some(Confirm::Calibrate);}
                    ui.collapsing("Advertised input and output capabilities",|ui|{capabilities(ui,&preview.inventory);}).header_response.help(ui, HelpControl::AudioCapabilities);
                }
                if let Some(confirm)=panel.confirm {
                    ui.separator();
                    match confirm {
                        Confirm::Switch=>{ui.label("Stop decks, clips, recording and held notes, then change output? Previous output will be restored if opening fails. Playback will remain stopped; press Play explicitly when ready.");},
                        Confirm::Calibrate=>{ui.label("Connect the chosen LINE output to the chosen LINE input using a suitable cable/interface loopback. Disable input monitoring, use line level (not a speaker output), and turn down external speakers. This stops performance, emits three short low-level coded probes on the chosen output, captures up to 3 seconds, and restores the session output without resuming playback.");if let Some(request)=panel.preview.as_ref().and_then(|p|p.calibration.as_ref().ok()){ui.label(format!("Confirm route: {} output {} → {} input {}; level {} dBFS",request.output.device,request.output_channel+1,request.input.device,request.input_channel+1,request.level_db));}},
                    }
                    ui.horizontal(|ui|{
                        if ui.button(match confirm{Confirm::Switch=>"Stop and change output",Confirm::Calibrate=>"Cable ready: stop and measure"}).help(ui,match confirm{Confirm::Switch=>HelpControl::AudioConfirm,Confirm::Calibrate=>HelpControl::AudioProbeConfirm}).clicked(){if let Some(preview)=panel.preview.clone(){panel.request(match confirm{Confirm::Switch=>Job::Apply(preview),Confirm::Calibrate=>Job::Calibrate(preview)});}panel.confirm=None;}
                        if ui.button("Keep current audio").help(ui, HelpControl::AudioKeep).clicked(){panel.confirm=None;}
                    });
                }
            });
        });
        if !open {
            panel.open = false;
            panel.confirm = None;
        }
    }
}
fn capabilities(ui: &mut egui::Ui, inventory: &config::Inventory) {
    ui.label("Device identities use the backend and exact device name. Persistent hardware serial identifiers are unavailable through CPAL; ambiguous duplicate names are rejected.");
    ui.label(format!("Backend: {}. Channel numbers are CPAL's ordered interleaved channels; physical connector names are unavailable.",inventory.backend));
    if inventory.truncated {
        ui.label("Capability inventory truncated at 256 devices / 4096 ranges per device");
    }
    if let Some(error) = &inventory.input_error {
        ui.label(format!("Input discovery: {error}"));
    }
    for (direction, devices) in [("Output", &inventory.devices), ("Input", &inventory.inputs)] {
        for (index, device) in devices.iter().enumerate() {
            ui.push_id((direction, index), |ui| {
                ui.collapsing(
                    format!(
                        "{direction}: {}{}",
                        device.name,
                        if device.default { " (default)" } else { "" }
                    ),
                    |ui| {
                        if let Some(error) = &device.error {
                            ui.label(error);
                        }
                        egui::ScrollArea::vertical().max_height(180.0).show_rows(
                            ui,
                            18.0,
                            device.ranges.len(),
                            |ui, rows| {
                                for row in rows {
                                    let r = &device.ranges[row];
                                    ui.label(format!(
                                        "{} · {} channels · {}–{} Hz · buffer {}",
                                        r.format,
                                        r.channels,
                                        r.min_rate,
                                        r.max_rate,
                                        r.buffer
                                            .map(|(min, max)| format!("{min}–{max} frames"))
                                            .unwrap_or("limits unavailable".into())
                                    )).help(ui, HelpControl::AudioCapabilities);
                                }
                            },
                        );
                    },
                ).header_response.help(ui, HelpControl::AudioCapabilities);
            });
        }
    }
}

/// The preference draft exposes only advertised choices. A stored unavailable
/// choice remains visible until deliberately changed; it is never normalized.
pub(super) fn edit_profile(
    ui: &mut egui::Ui,
    audio: &mut Audio,
    inventory: Option<&config::Inventory>,
) {
    let Some(inventory) = inventory else {
        ui.label("Preview the profile to discover audio capabilities. Saved unavailable selections are preserved.");
        return;
    };
    egui::ComboBox::from_label("Audio backend")
        .selected_text(audio.backend.as_deref().unwrap_or("System backend"))
        .show_ui(ui, |ui| {
            ui.selectable_value(&mut audio.backend, None, "System backend").help(ui, HelpControl::AudioBackend);
            ui.selectable_value(
                &mut audio.backend,
                Some(inventory.backend.clone()),
                &inventory.backend,
            ).help(ui, HelpControl::AudioBackend);
        }).response.help(ui, HelpControl::AudioBackend);
    device(ui, "Output device", &mut audio.device, &inventory.devices, HelpControl::PreferenceAudioDevice);
    let found = inventory.devices.iter().find(|device| {
        audio
            .device
            .as_ref()
            .map(|name| name == &device.name)
            .unwrap_or(device.default)
    });
    let mut rates = vec![44100, 48000, 96000, 192000];
    if let Some(found) = found {
        if let Some((_, rate, _)) = found.defaults {
            rates.push(rate);
        }
        for range in &found.ranges {
            rates.extend([range.min_rate, range.max_rate]);
        }
        rates.retain(|rate| {
            (8000..=384000).contains(rate)
                && found
                    .ranges
                    .iter()
                    .any(|r| (r.min_rate..=r.max_rate).contains(rate))
        });
    } else {
        rates.clear();
    }
    rates.sort();
    rates.dedup();
    choice(ui, "Sample rate Hz", &mut audio.sample_rate, &rates, HelpControl::PreferenceAudioRate);
    layout(
        ui,
        found,
        audio.sample_rate,
        &mut audio.channels,
        &mut audio.format,
        &mut audio.buffer_frames,
        "Output",
        [HelpControl::PreferenceAudioChannels, HelpControl::AudioOutputFormat, HelpControl::PreferenceAudioBuffer],
    );
    ui.label("Main left/right use outputs 1/2 (mono sums both); additional outputs are silent. Independent cue routing is not available.");
    ui.heading("Optional loopback calibration input");
    device(
        ui,
        "Calibration input device",
        &mut audio.calibration.device,
        &inventory.inputs,
        HelpControl::AudioInputDevice,
    );
    let found = inventory.inputs.iter().find(|device| {
        audio
            .calibration
            .device
            .as_ref()
            .map(|name| name == &device.name)
            .unwrap_or(device.default)
    });
    layout(
        ui,
        found,
        None,
        &mut audio.calibration.channels,
        &mut audio.calibration.format,
        &mut audio.calibration.buffer_frames,
        "Input",
        [HelpControl::AudioInputChannels, HelpControl::AudioInputFormat, HelpControl::AudioInputBuffer],
    );
    let mut input = audio.calibration.channel as f32 + 1.0;
    let mut output = audio.calibration.output_channel as f32 + 1.0;
    preferences::float_control(
        ui,
        "Calibration input channel",
        &mut input,
        1.0,
        64.0,
        1.0,
        "",
        HelpControl::AudioInputChannel,
    );
    preferences::float_control(ui, "Probe output channel", &mut output, 1.0, 64.0, 1.0, "", HelpControl::AudioOutputChannel);
    audio.calibration.channel = input.round() as u16 - 1;
    audio.calibration.output_channel = output.round() as u16 - 1;
    preferences::float_control(
        ui,
        "Probe level",
        &mut audio.calibration.level_db,
        -60.0,
        -24.0,
        1.0,
        " dBFS",
        HelpControl::AudioProbeLevel,
    );
    ui.label("Calibration always uses the active output's logical rate. Preview validates both selected channel numbers before a probe can run.");
}
fn device(ui: &mut egui::Ui, label: &str, value: &mut Option<String>, devices: &[config::Device], control: HelpControl) {
    egui::ComboBox::from_label(label)
        .selected_text(value.as_deref().unwrap_or("System default"))
        .show_ui(ui, |ui| {
            ui.selectable_value(value, None, "System default").help(ui, control);
            for device in devices {
                ui.selectable_value(value, Some(device.name.clone()), &device.name).help(ui, control);
            }
        }).response.help(ui, control);
}
fn choice<T: Copy + PartialEq + std::fmt::Display>(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut Option<T>,
    values: &[T],
    control: HelpControl,
) {
    egui::ComboBox::from_label(label)
        .selected_text(
            value
                .map(|n| n.to_string())
                .unwrap_or("Device default".into()),
        )
        .show_ui(ui, |ui| {
            ui.selectable_value(value, None, "Device default").help(ui, control);
            for n in values {
                ui.selectable_value(value, Some(*n), n.to_string()).help(ui, control);
            }
        }).response.help(ui, control);
}
fn layout(
    ui: &mut egui::Ui,
    device: Option<&config::Device>,
    rate: Option<u32>,
    channels: &mut Option<u16>,
    format: &mut Option<AudioFormat>,
    buffer: &mut Option<u32>,
    direction: &str,
    controls: [HelpControl; 3],
) {
    let ranges = device.map(|device| device.ranges.as_slice()).unwrap_or(&[]);
    let mut counts = ranges
        .iter()
        .filter(|r| rate.is_none_or(|rate| (r.min_rate..=r.max_rate).contains(&rate)))
        .map(|r| r.channels)
        .filter(|channels| (1..=64).contains(channels))
        .collect::<Vec<_>>();
    counts.sort();
    counts.dedup();
    choice(ui, &format!("{direction} channels"), channels, &counts, controls[0]);
    egui::ComboBox::from_label(format!("{direction} sample format"))
        .selected_text(
            format
                .map(|f| f.cpal().to_string())
                .unwrap_or("Device default".into()),
        )
        .show_ui(ui, |ui| {
            ui.selectable_value(format, None, "Device default").help(ui, controls[1]);
            for f in AudioFormat::ALL {
                if ranges.iter().any(|r| {
                    r.format == f.cpal()
                        && channels.is_none_or(|n| n == r.channels)
                        && rate.is_none_or(|n| (r.min_rate..=r.max_rate).contains(&n))
                }) {
                    ui.selectable_value(format, Some(f), f.cpal().to_string()).help(ui, controls[1]);
                }
            }
        }).response.help(ui, controls[1]);
    let mut sizes = vec![32, 64, 128, 256, 512, 1024, 2048, 4096, 8192];
    for range in ranges {
        if let Some((min, max)) = range.buffer {
            sizes.extend([min, max]);
        }
    }
    sizes.sort();
    sizes.dedup();
    sizes.retain(|n| {
        (16..=32768).contains(n)
            && ranges.iter().any(|r| {
                channels.is_none_or(|n| n == r.channels)
                    && format.is_none_or(|f| f.cpal() == r.format)
                    && rate.is_none_or(|n| (r.min_rate..=r.max_rate).contains(&n))
                    && r.buffer.is_some_and(|(min, max)| (min..=max).contains(n))
            })
    });
    choice(ui, &format!("{direction} buffer frames"), buffer, &sizes, controls[2]);
    if ranges.iter().any(|r| r.buffer.is_none()) {
        ui.label(format!("{direction} buffer limits are not advertised for some configurations; Device default is available."));
    }
}

#[cfg(test)]
mod tests;
