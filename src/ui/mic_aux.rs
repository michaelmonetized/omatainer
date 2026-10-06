use super::*;
use crate::engine::{
    audio::routing::{
        mic_aux::{
            control::{Control, Parameter, Request},
            Configuration, Override,
        },
        model::Direction,
    },
    midi_edit::{Ack, Outcome},
    project::Captured,
};
use crossbeam_channel::{bounded, Receiver};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

struct Draft {
    captured: Captured,
    rate: u32,
    configuration: Configuration,
}
struct Job {
    result: Receiver<Result<Box<Draft>, String>>,
    cancel: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl Job {
    fn start(engine: &Engine) -> Result<Self, String> {
        let project = engine.project.clone();
        let cancel = Arc::new(AtomicBool::new(false));
        let stop = cancel.clone();
        let (tx, result) = bounded(1);
        let thread = std::thread::Builder::new()
            .name("omatainer-mic-aux-review".into())
            .spawn(move || {
                let value = (|| {
                    let captured = project.capture(&stop).map_err(|e| e.to_string())?;
                    let configuration = captured.state.mic_aux.unwrap_or_default();
                    Ok(Box::new(Draft {
                        captured,
                        rate: project.sample_rate(),
                        configuration,
                    }))
                })();
                if stop.load(Ordering::Acquire) {
                    let _ = project.retire_cancelled_capture(&stop);
                }
                let _ = tx.try_send(value);
            })
            .map_err(|e| e.to_string())?;
        Ok(Self {
            result,
            cancel,
            thread: Some(thread),
        })
    }
    fn poll(&mut self) -> Option<Result<Box<Draft>, String>> {
        if !self.thread.as_ref()?.is_finished() {
            return None;
        }
        let _ = self.thread.take().unwrap().join();
        Some(
            self.result
                .try_recv()
                .unwrap_or_else(|_| Err("Mic/aux review ended without a result".into())),
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
    draft: Option<Box<Draft>>,
    job: Option<Job>,
    ack: Option<Ack>,
    message: String,
}
fn choices(ui: &mut egui::Ui, label: &str, value: &mut Option<u64>, ports: &[(u64, String)]) {
    egui::ComboBox::from_id_salt(label)
        .selected_text(
            ports
                .iter()
                .find(|p| Some(p.0) == *value)
                .map_or("Excluded / none", |p| p.1.as_str()),
        )
        .show_ui(ui, |ui| {
            ui.selectable_value(value, None, "Excluded / none");
            for (id, name) in ports {
                ui.selectable_value(value, Some(*id), name);
            }
        });
    ui.label(label);
}
impl App {
    pub(super) fn mic_aux_ui(&mut self, ctx: &egui::Context) {
        if self.mic_aux.job.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(50));
        }
        if let Some(result) = self.mic_aux.job.as_mut().and_then(Job::poll) {
            self.mic_aux.job = None;
            match result {
                Ok(draft) => {
                    self.mic_aux.draft = Some(draft);
                    self.mic_aux.message="Choose retained aliases. New sources start muted; enable a live input in Audio routing.".into();
                }
                Err(e) => self.mic_aux.message = e,
            }
        }
        if let Some(ack) = &self.mic_aux.ack {
            match ack.state() {
                Outcome::Pending => {
                    ctx.request_repaint_after(std::time::Duration::from_millis(30));
                }
                Outcome::Applied => {
                    self.mic_aux.message =
                        "Mic/aux selection applied. Save retains it; History can undo it.".into();
                    self.mic_aux.ack = None;
                    self.mic_aux.draft = None;
                }
                _ => {
                    self.mic_aux.message="Selection was not applied. Stop playback/recording, use Studio mode and refresh the source.".into();
                    self.mic_aux.ack = None;
                }
            }
        }
        if !self.mic_aux.open {
            return;
        }
        let mut open = true;
        let mut review = false;
        let mut configure = false;
        let mut remove = false;
        let mut controls = Vec::new();
        let cfg = self.snap.mic_aux.configuration;
        let namespace = self.snap.session.as_ref().map(|s| s.namespace);
        egui::Window::new(tr!("Mic & aux")).open(&mut open).default_width(620.0).show(ctx,|ui|{
            let panel=&mut self.mic_aux;
            ui.label(tr!("Use existing input aliases. Choose master, booth and recording inclusion separately."));
            ui.label(tr!("Final-output recording always includes the sound sent to that output. Use a raw recording mix to exclude a voice."));
            ui.add_enabled_ui(panel.job.is_none()&&panel.ack.is_none(),|ui|{if ui.button(tr!("Review mic/aux sources")).help(ui,HelpControl::MicAux).clicked(){review=true;}});
            if let Some(draft)=&mut panel.draft {
                let model=draft.captured.state.routing.as_ref();
                let ports=|direction|model.map_or(Vec::new(),|m|m.ports.iter().filter(|p|p.direction==direction&&(1..=2).contains(&p.channels.len())&&m.monitor_output!=Some(p.id)).map(|p|(p.id,p.alias.clone())).collect::<Vec<_>>());
                let inputs=ports(Direction::Input);let outputs=ports(Direction::Output);let records=ports(Direction::Record);
                for (i,label) in ["Mic","Aux"].into_iter().enumerate(){ui.push_id(i,|ui|{
                    ui.strong(label);let c=&mut draft.configuration.channels[i];let old=c.input;choices(ui,"Input alias",&mut c.input,&inputs);
                    if old!=c.input{c.mute=true;c.master=None;c.booth=None;c.record=None;}
                    if c.input.is_some(){choices(ui,"Master mix",&mut c.master,&outputs);choices(ui,"Booth mix",&mut c.booth,&outputs);choices(ui,"Recording mix",&mut c.record,&records);}
                });}
                ui.add_enabled_ui(!self.snap.performance.protected&&!self.snap.playing&&!self.snap.recording&&panel.ack.is_none(),|ui|{
                    if ui.button(tr!("Apply mic/aux selection")).clicked(){configure=true;}
                    if ui.button(tr!("Remove mic/aux selection")).clicked(){remove=true;}
                });
            }
            if let Some(mut cfg)=cfg {
                ui.separator();
                for (i,label) in ["Mic","Aux"].into_iter().enumerate(){ui.push_id(("live",i),|ui|{
                    let meter=self.snap.mic_aux.meters[i];ui.strong(format!("{label} · {} · input {:.3} · output {:.3}",if meter.available{"available"}else{"input missing"},meter.input_peak,meter.output_peak));
                    let c=&mut cfg.channels[i];
                    if ui.checkbox(&mut c.mute,format!("{label} mute")).changed(){controls.push((i,Parameter::Mute(c.mute)));}
                    if ui.add(egui::Slider::new(&mut c.gain,0.0..=4.0).text(format!("{label} gain"))).changed(){controls.push((i,Parameter::Gain(c.gain)));}
                    if ui.checkbox(&mut c.tone,format!("{label} tone")).changed(){controls.push((i,Parameter::Tone(c.tone)));}
                    ui.add_enabled_ui(c.tone,|ui|{for (b,name) in ["Low","Mid","High"].into_iter().enumerate(){if ui.add(egui::Slider::new(&mut c.eq_db[b],-18.0..=18.0).text(format!("{label} {name} dB"))).changed(){controls.push((i,Parameter::Eq{band:b as u8,db:c.eq_db[b]}));}}});
                });}
                ui.separator();let d=&mut cfg.duck;
                if ui.checkbox(&mut d.enabled,tr!("Talkover ducking")).changed(){controls.push((0,Parameter::Duck(d.enabled)));}
                ui.horizontal(|ui|{for (mode,label) in [(Override::Automatic,"Voice detector"),(Override::Held,"Hold duck"),(Override::Off,"Bypass duck")] {if ui.selectable_value(&mut d.mode,mode,label).changed(){controls.push((0,Parameter::Mode(mode)));}}});
                for (label,value,range,parameter) in [("Voice threshold",&mut d.threshold,0.00001..=1.0,Parameter::Threshold as fn(f32)->Parameter),("Music reduction dB",&mut d.reduction_db,0.0..=40.0,Parameter::Reduction),("Attack ms",&mut d.attack_ms,1.0..=2000.0,Parameter::Attack),("Release ms",&mut d.release_ms,1.0..=10000.0,Parameter::Release)] {
                    if ui.add(egui::Slider::new(value,range).text(label)).changed(){controls.push((0,parameter(*value)));}
                }
                ui.label(format!("Effective music level: {:.1}%",self.snap.mic_aux.duck_gain*100.0));
            }
            if !panel.message.is_empty(){ui.label(&panel.message);}
        });
        self.mic_aux.open = open;
        if review {
            match Job::start(&self.engine) {
                Ok(job) => self.mic_aux.job = Some(job),
                Err(e) => self.mic_aux.message = e,
            }
        }
        if configure || remove {
            if let Some(draft) = &self.mic_aux.draft {
                match Request::new(
                    &draft.captured,
                    draft.rate,
                    if remove {
                        None
                    } else {
                        Some(draft.configuration)
                    },
                ) {
                    Ok((request, ack)) => {
                        match self.engine.cmd.send(Command::MicAuxConfigure(request)) {
                            Ok(_) => self.mic_aux.ack = Some(ack),
                            Err(e) => self.mic_aux.message = e.to_string(),
                        }
                    }
                    Err(e) => self.mic_aux.message = e,
                }
            }
        }
        if let (Some(cfg), Some(namespace)) = (cfg, namespace) {
            for (role, parameter) in controls {
                let control = Control {
                    namespace,
                    role: role as u8,
                    input: cfg.channels[role].input,
                    parameter,
                };
                if let Err(e) = self.engine.cmd.send(Command::MicAuxControl(control)) {
                    self.mic_aux.message = e.to_string();
                }
            }
        }
    }
}
#[cfg(test)]
mod tests;
