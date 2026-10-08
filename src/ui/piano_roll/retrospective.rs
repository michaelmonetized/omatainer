use super::*;
use crate::engine::retrospective::{Choice, Prepared, Snapshot};

struct Worker {
    receiver: mpsc::Receiver<Result<Prepared, String>>,
    cancel: Arc<AtomicBool>,
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
    }
}
pub(super) struct State {
    open: bool,
    snapshot: Option<Snapshot>,
    choice: Choice,
    worker: Option<Worker>,
    prepared: Option<Prepared>,
    transfer: Option<(Prepared, u8, u16)>,
    message: String,
}
impl Default for State {
    fn default() -> Self {
        Self {
            open: false,
            snapshot: None,
            choice: Choice {
                track: 0,
                source: 0,
                from_seconds_ago: 8.0,
                to_seconds_ago: 0.0,
                tempo: 120.0,
                loop_beats: 16.0,
            },
            worker: None,
            prepared: None,
            transfer: None,
            message: String::new(),
        }
    }
}
impl State {
    fn refresh(&mut self, snapshot: Snapshot, track: u8, tempo: f64) {
        self.worker = None;
        self.prepared = None;
        self.transfer = None;
        self.choice.track = track;
        self.choice.source = snapshot
            .events
            .iter()
            .find(|e| e.track == track && e.length != 0)
            .map_or(0, |e| e.source);
        if self.choice.source == 0 {
            if let Some(event) = snapshot.events.iter().find(|e| e.length != 0) {
                self.choice.track = event.track;
                self.choice.source = event.source;
            }
        }
        self.choice.tempo = tempo.clamp(30.0, 300.0);
        self.choice.from_seconds_ago = snapshot
            .captured_at
            .duration_since(snapshot.origin)
            .as_secs_f64()
            .min(8.0);
        self.choice.to_seconds_ago = 0.0;
        self.choice.loop_beats = (self.choice.from_seconds_ago * self.choice.tempo / 60.0 / 4.0)
            .ceil()
            .max(1.0)
            * 4.0;
        self.snapshot = Some(snapshot);
        self.message="Choose one monitored source, review the time range and tempo, then preview. The project stays unchanged until Apply MIDI edit.".into();
    }
    fn preview(&mut self) -> Result<(), String> {
        if self.worker.is_some() {
            return Err("Wait for the current capture preview or cancel it".into());
        }
        let snapshot = self
            .snapshot
            .clone()
            .ok_or("Refresh recent MIDI before previewing")?;
        let choice = self.choice;
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = cancel.clone();
        let (sender, receiver) = mpsc::sync_channel(1);
        std::thread::Builder::new()
            .name("omat-midi-retrospective".into())
            .spawn(move || {
                let result =
                    crate::engine::retrospective::prepare(&snapshot, choice, &worker_cancel);
                let _ = sender.send(result);
            })
            .map_err(|e| format!("Recent MIDI worker unavailable: {e}"))?;
        self.worker = Some(Worker { receiver, cancel });
        self.prepared = None;
        self.message = "Preparing the reviewed recent performance…".into();
        Ok(())
    }
    fn poll(&mut self, epoch: u64) {
        if self.snapshot.as_ref().is_some_and(|s| s.epoch != epoch) {
            self.worker = None;
            self.prepared = None;
            self.snapshot = None;
            self.message="MIDI history was cleared, reconfigured or disconnected. Refresh before reviewing another performance.".into();
        }
        let Some(worker) = &self.worker else {
            return;
        };
        let result = match worker.receiver.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => {
                Err("Recent MIDI worker disconnected; the project is unchanged".into())
            }
        };
        let worker = self.worker.take().unwrap();
        match result {
            Ok(prepared) if !worker.cancel.load(Ordering::Acquire) && prepared.epoch == epoch => {
                self.message = format!(
                    "{} editable notes and {} expression/control events are ready for review.",
                    prepared.content.notes.len(),
                    prepared.content.messages.len()
                );
                self.prepared = Some(prepared);
            }
            Ok(_) => {
                self.message =
                    "Capture cancelled or its private history changed; the project is unchanged."
                        .into()
            }
            Err(error) => self.message = error,
        }
    }
}

impl App {
    /// Open the explicit recent-input review.
    /// Takes the current session selection and history owner; shows bounded private input without changing the project or opening a device.
    pub(in crate::ui) fn open_recent_midi(&mut self) {
        let snapshot = self.engine.cmd.retrospective().snapshot(Instant::now());
        self.piano_roll.recent.refresh(
            snapshot,
            self.snap.selected_track as u8,
            f64::from(self.snap.bpm),
        );
        self.piano_roll.recent.open = true;
    }

    /// Review recent monitored MIDI and hand one candidate to the ordinary editor.
    /// Takes the native egui context; advances bounded workers, refuses stale history or a nonempty target, and leaves publication to Apply MIDI edit.
    pub(in crate::ui) fn recent_midi_ui(&mut self, ctx: &egui::Context) {
        let epoch = self.engine.cmd.retrospective().epoch();
        self.piano_roll.recent.poll(epoch);
        if self.piano_roll.recent.transfer.is_some() && !self.piano_roll.busy() {
            let (mut prepared, track, scene) = self.piano_roll.recent.transfer.take().unwrap();
            let result = (|| {
                if prepared.epoch != epoch {
                    return Err("MIDI history changed before the draft was inspected".to_string());
                }
                let draft = self.piano_roll.draft.as_mut().ok_or_else(|| {
                    self.piano_roll
                        .error
                        .clone()
                        .unwrap_or("The capture destination could not be inspected".into())
                })?;
                if draft.baseline.track != track
                    || draft.baseline.scene != scene
                    || draft.baseline.kind != crate::engine::ClipKind::Empty
                    || draft.dirty
                {
                    return Err("Choose an empty Session slot; current musical material and the capture preview are retained".into());
                }
                let beats = prepared.content.end_tick as f64 / f64::from(prepared.content.ppqn);
                let region = Region::full((beats / 4.0) as f32);
                if !region.valid() {
                    return Err("The capture loop is outside editable musical bounds".into());
                }
                if !region.allows(&prepared.content.notes) {
                    return Err("This capture exceeds the selected editable loop limits".into());
                }
                draft.name = "Captured MIDI".into();
                draft.region = region;
                draft.notes = std::mem::take(&mut prepared.content.notes);
                draft.selected = draft.notes.iter().map(|n| n.id).collect();
                draft.controls.install(&mut prepared.content, true);
                draft.dirty = true;
                self.piano_roll.message="Captured ordinary notes are editable. Correct them before Apply MIDI edit; Apply is one Undo.".into();
                Ok(())
            })();
            match result {
                Ok(()) => {
                    self.piano_roll.recent.open = false;
                    self.piano_roll.recent.snapshot = None;
                    self.piano_roll.recent.message = "The captured draft is open. The project is unchanged until you Apply MIDI edit.".into();
                }
                Err(error) => {
                    self.piano_roll.recent.message = error;
                    if prepared.epoch == epoch {
                        self.piano_roll.recent.prepared = Some(prepared);
                    }
                }
            }
        }
        if !self.piano_roll.recent.open {
            return;
        }
        let mut open = true;
        let mut refresh = false;
        let mut preview = false;
        let mut transfer = false;
        let mut clear = false;
        let mut preferences = false;
        let unavailable = self.piano_roll.blocks_close();
        let state = &mut self.piano_roll.recent;
        egui::Window::new("Capture recent MIDI").id(egui::Id::new("omat-recent-midi")).open(&mut open).default_width(720.0).default_height(620.0).vscroll(true).show(ctx,|ui| {
            accessibility::scope(ui,"Recent MIDI capture",|ui| {
                ui.label("Recent input stays in memory. Capture uses enabled MIDI routes with Monitor selected, their channel/message filters and original input timestamps. No record arming is required.");
                ui.horizontal_wrapped(|ui| {
                    for (label,flag) in [("Refresh recent MIDI",&mut refresh),("Clear recent MIDI history",&mut clear),("Recent MIDI privacy preferences",&mut preferences)] {
                        let response=ui.button(label);accessibility::button(ui,&response,label,None);*flag=response.clicked();
                    }
                });
                if let Some(snapshot)=&state.snapshot {
                    if !snapshot.config.enabled {ui.label("Recent MIDI history is off. Enable it in privacy preferences before performing; earlier input cannot be recovered.");}
                    ui.label(format!("{} retained events · {} discarded events · review anchored {:.1} seconds ago",snapshot.events.len(),snapshot.dropped,snapshot.captured_at.elapsed().as_secs_f64()));
                    let tracks:BTreeSet<_>=snapshot.events.iter().filter(|e|e.length!=0).map(|e|e.track).collect();
                    ui.add_enabled_ui(state.worker.is_none()&&state.transfer.is_none(),|ui| {
                        let old=state.choice;
                        let before=state.choice.track;
                        let menu=egui::ComboBox::from_id_salt("recent-midi-track").selected_text(format!("Input track {}",state.choice.track+1)).show_ui(ui,|ui| {
                            for track in tracks {ui.selectable_value(&mut state.choice.track,track,format!("Input track {}",track+1));}
                        });
                        menu.response.widget_info(||egui::WidgetInfo::labeled(egui::WidgetType::ComboBox,true,"Recent MIDI input track"));
                        let sources:BTreeSet<_>=snapshot.events.iter().filter(|e|e.track==state.choice.track&&e.length!=0).map(|e|e.source).collect();
                        if before!=state.choice.track||!sources.contains(&state.choice.source) {state.choice.source=sources.iter().next().copied().unwrap_or(0);state.prepared=None;}
                        let title=|source:u64|self.engine.cmd.midi_routing().source_endpoint(source).map_or_else(||format!("MIDI source {source}"),|e|format!("{} · {} · source {source}",e.name,e.id.unwrap_or_default()));
                        let menu=egui::ComboBox::from_id_salt("recent-midi-source").selected_text(title(state.choice.source)).show_ui(ui,|ui| {
                            for source in sources {ui.selectable_value(&mut state.choice.source,source,title(source));}
                        });
                        menu.response.widget_info(||egui::WidgetInfo::labeled(egui::WidgetType::ComboBox,true,"Recent MIDI input source"));
                        ui.horizontal_wrapped(|ui| {
                            number(ui,"Start seconds ago",&mut state.choice.from_seconds_ago,0.0,f64::from(snapshot.config.seconds));
                            number(ui,"End seconds ago",&mut state.choice.to_seconds_ago,0.0,f64::from(snapshot.config.seconds));
                            number(ui,"Capture tempo",&mut state.choice.tempo,30.0,300.0);
                            number(ui,"Capture loop beats",&mut state.choice.loop_beats,0.25,4096.0);
                        });
                        if old!=state.choice {state.prepared=None;}
                    });
                    let button=ui.add_enabled(state.worker.is_none()&&state.transfer.is_none()&&snapshot.config.enabled&&state.choice.source!=0,egui::Button::new("Preview recent MIDI capture"));
                    accessibility::button(ui,&button,"Preview recent MIDI capture",None);preview=button.clicked();
                }
                if state.worker.is_some() {
                    let button=ui.button("Cancel recent MIDI preparation");accessibility::button(ui,&button,"Cancel recent MIDI preparation",None);
                    if button.clicked() {state.worker=None;state.message="Preparation cancelled; the project is unchanged.".into();}
                }
                if let Some(prepared)=&state.prepared {
                    ui.label(format!("{} notes start at the range boundary; {} held notes end at the range boundary; {} shorter-than-one-tick gates were extended.",prepared.clipped_at_start,prepared.held_at_end,prepared.short_gates_extended));
                    egui::ScrollArea::vertical().id_salt("recent-midi-preview-notes").max_height(240.0).show(ui,|ui| {
                        for (index,note) in prepared.content.notes.iter().take(64).enumerate() {ui.label(format!("Note {}: {} · channel {} · beat {:.6} · length {:.6} · velocity {} · release {}",index+1,pitch_name(note.pitch),note.channel+1,note.source_start(),note.source_duration(),note.vel,note.release_vel));}
                        if prepared.content.notes.len()>64 {ui.label("The first 64 notes are shown here. The ordinary editor opens the complete capture.");}
                    });
                    ui.label(format!("Destination: selected empty Session slot, track {}, scene {}",self.snap.selected_track+1,self.snap.selected_scene+1));
                    let button=ui.add_enabled(!unavailable,egui::Button::new("Open captured MIDI draft"));accessibility::button(ui,&button,"Open captured MIDI draft",None);transfer=button.clicked();
                    if unavailable {ui.label("Finish or discard the existing MIDI draft before opening this captured performance.");}
                }
                ui.label(&state.message);
            });
        });
        if !open {
            state.open = false;
            state.worker = None;
            state.prepared = None;
            state.snapshot = None;
        }
        if clear {
            self.engine.cmd.retrospective().clear(Instant::now());
            let snapshot = self.engine.cmd.retrospective().snapshot(Instant::now());
            self.piano_roll.recent.refresh(
                snapshot,
                self.snap.selected_track as u8,
                f64::from(self.snap.bpm),
            );
            self.piano_roll.recent.message="Private recent MIDI history was discarded. The project and any explicitly opened draft are unchanged.".into();
        } else if refresh {
            self.open_recent_midi();
        }
        if preferences {
            self.settings.open = true;
        }
        if preview {
            if let Err(error) = self.piano_roll.recent.preview() {
                self.piano_roll.recent.message = error;
            }
        }
        if transfer {
            if let Some(prepared) = self.piano_roll.recent.prepared.take() {
                let track = self.snap.selected_track as u8;
                let scene = self.snap.selected_scene as u16;
                self.piano_roll.recent.transfer = Some((prepared, track, scene));
                self.piano_roll.load(&self.engine, track, scene);
                self.piano_roll.recent.message =
                    "Inspecting the chosen empty destination before opening the captured draft…"
                        .into();
            }
        }
    }
}

#[cfg(test)]
mod tests;
