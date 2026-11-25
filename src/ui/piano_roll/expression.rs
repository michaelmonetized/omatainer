use super::*;
use crate::engine::{
    midi_tools::Content,
    mpe::envelope::{self, Dimension, Edit, Edited, Envelope},
    performance::WorkPermit,
};
use crossbeam_channel::{bounded, Receiver, Sender};

enum Operation {
    Inspect,
    Edit(Edit),
}
struct Job {
    source: Arc<Content>,
    note: NoteId,
    operation: Operation,
    work: WorkPermit,
}
enum ResultValue {
    Inspection(Envelope),
    Edited(Edited),
}
struct Event {
    source: Arc<Content>,
    note: NoteId,
    result: Result<ResultValue, String>,
}
struct Worker {
    jobs: Sender<Job>,
    events: Receiver<Event>,
}
impl Worker {
    fn start() -> Result<Self, String> {
        let (jobs, incoming) = bounded::<Job>(1);
        let (completed, events) = bounded(1);
        std::thread::Builder::new()
            .name("omatainer-note-expression".into())
            .spawn(move || {
                while let Ok(job) = incoming.recv() {
                    let cancel = job.work.cancel();
                    let result = (|| {
                        let ticket = job.work.background(
                            crate::background::Kind::Prepare,
                            "note-expression".into(),
                            64 * 1024 * 1024,
                        )?;
                        let _running = ticket.enter(|| cancel.load(Ordering::Acquire))?;
                        match job.operation {
                            Operation::Inspect => envelope::inspect(&job.source, job.note, &cancel)
                                .map(ResultValue::Inspection),
                            Operation::Edit(edit) => {
                                envelope::edit(&job.source, job.note, edit, &cancel)
                                    .map(ResultValue::Edited)
                            }
                        }
                    })();
                    if completed
                        .send(Event {
                            source: job.source,
                            note: job.note,
                            result,
                        })
                        .is_err()
                    {
                        break;
                    }
                }
            })
            .map_err(|error| format!("Note expression worker could not start: {error}"))?;
        Ok(Self { jobs, events })
    }
}

pub(super) struct Controls {
    worker: Option<Worker>,
    cancel: Option<Arc<AtomicBool>>,
    source: Option<Arc<Content>>,
    inspection: Option<Envelope>,
    dimension: Dimension,
    beat: f64,
    value: f64,
    selected: Option<crate::midi_file::Message>,
    message: String,
}
impl Default for Controls {
    fn default() -> Self {
        Self {
            worker: None,
            cancel: None,
            source: None,
            inspection: None,
            dimension: Dimension::Pitch,
            beat: 0.0,
            value: 0.0,
            selected: None,
            message: String::new(),
        }
    }
}
impl Drop for Controls {
    fn drop(&mut self) {
        if let Some(cancel) = &self.cancel {
            cancel.store(true, Ordering::Release);
        }
    }
}
impl Controls {
    pub(super) fn busy(&self) -> bool {
        self.cancel.is_some()
    }
    fn current(&self, draft: &Draft) -> bool {
        self.source
            .as_ref()
            .is_some_and(|source| source.notes == draft.notes && draft.controls.matches(source))
    }
    fn start(
        &mut self,
        draft: &Draft,
        engine: &Engine,
        note: NoteId,
        operation: Operation,
    ) -> Result<(), String> {
        if self.busy() {
            return Err("Wait for the current note expression operation or cancel it".into());
        }
        let work = engine
            .cmd
            .performance()
            .optional_work()
            .map_err(|error| error.to_string())?;
        if self.worker.is_none() {
            self.worker = Some(Worker::start()?);
        }
        let source = Arc::new(draft.controls.content(&draft.notes));
        let cancel = work.cancel();
        self.worker
            .as_ref()
            .unwrap()
            .jobs
            .try_send(Job {
                source,
                note,
                operation,
                work,
            })
            .map_err(|_| "Note expression worker is busy or closed".to_string())?;
        self.cancel = Some(cancel);
        self.message = "Preparing this note's expression…".into();
        Ok(())
    }
    fn poll(&mut self, draft: &mut Draft) -> Result<bool, String> {
        let Some(worker) = &self.worker else {
            return Ok(false);
        };
        let Ok(event) = worker.events.try_recv() else {
            return Ok(false);
        };
        let cancelled = self
            .cancel
            .take()
            .is_some_and(|cancel| cancel.load(Ordering::Acquire));
        if cancelled {
            self.message = "Note expression operation cancelled. The draft is preserved.".into();
            return Ok(false);
        }
        if event.source.notes != draft.notes || !draft.controls.matches(&event.source) {
            return Err(
                "The draft changed during expression preparation; inspect this note again".into(),
            );
        }
        match event.result? {
            ResultValue::Inspection(inspection) => {
                self.inspection = Some(inspection);
                self.source = Some(event.source);
                self.selected = None;
                self.beat = 0.0;
                self.value = self
                    .inspection
                    .as_ref()
                    .map_or(0.0, |inspection| f64::from(inspection.initial.pitch));
                self.dimension = Dimension::Pitch;
                self.message =
                    "Choose a curve and edit its points. Apply the MIDI draft to keep the change."
                        .into();
                Ok(false)
            }
            ResultValue::Edited(mut edited) => {
                draft.notes = std::mem::take(&mut edited.content.notes);
                draft.controls.install(&mut edited.content, true);
                draft.dirty = true;
                self.source = None;
                self.inspection = None;
                self.selected = None;
                self.message = format!("Edited note {} on member channel {}. Reuse state is restored after release. Maximum timing rounding: {:.12} beats. Inspect again to continue editing; Apply keeps this draft.", event.note, edited.channel + 1, edited.maximum_rounding_beats);
                Ok(true)
            }
        }
    }
}

fn enabled(ui: &mut Ui, label: &str, enabled: bool) -> egui::Response {
    let response = ui.add_enabled(enabled, egui::Button::new(label));
    accessibility::button(ui, &response, label, None);
    response
}
pub(super) fn poll(draft: &mut Draft) -> Result<bool, String> {
    let mut controls = std::mem::take(&mut draft.expression);
    let result = controls.poll(draft);
    draft.expression = controls;
    result
}

/// Edit a selected note's pitch, pressure and timbre.
/// Takes the native piano-roll draft, theme and command producer; prepares guarded changes off the GUI and audio threads and returns whether the draft changed.
pub(super) fn show(
    ui: &mut Ui,
    draft: &mut Draft,
    theme: &Theme,
    engine: &Engine,
) -> Result<bool, String> {
    let mut controls = std::mem::take(&mut draft.expression);
    let mut outcome = controls.poll(draft);
    let mut inspect = false;
    let mut modify = None;
    egui::CollapsingHeader::new("Per-note expression").id_salt("note-expression").show(ui, |ui| {
        let note = (draft.selected.len() == 1).then(|| draft.selected.iter().next().copied()).flatten();
        if enabled(ui, "Inspect selected note expression", note.is_some() && !controls.busy()).clicked() { inspect = true; }
        if controls.busy() && enabled(ui, "Cancel note expression operation", true).clicked() { if let Some(cancel) = &controls.cancel { cancel.store(true, Ordering::Release); } }
        ui.label("Pitch uses semitones. Pressure opens the synth filter; timbre changes its cutoff. Manager controls retain their timeline. Shared members move to a free channel when edited, and the previous channel state is restored after release.");
        if let Some(inspection) = &controls.inspection {
            ui.label(format!("Note {} · member {} · manager {} · {:.9} beats · initial bend range ±{} semitones", inspection.note, inspection.channel + 1, inspection.manager + 1, inspection.duration, inspection.range));
            let current = controls.current(draft) && !controls.busy();
            let before = controls.dimension;
            ui.horizontal_wrapped(|ui| {
                for (dimension, label) in [(Dimension::Pitch,"Note pitch envelope"),(Dimension::Pressure,"Note pressure envelope"),(Dimension::Timbre,"Note timbre envelope")] {
                    let response = ui.selectable_value(&mut controls.dimension,dimension,label);
                    accessibility::button(ui,&response,label,Some(controls.dimension==dimension));
                }
            });
            if before != controls.dimension {
                controls.selected = None; controls.beat = 0.0;
                controls.value = match controls.dimension { Dimension::Pitch => f64::from(inspection.initial.pitch), Dimension::Pressure => f64::from(inspection.initial.pressure), Dimension::Timbre => f64::from(inspection.initial.timbre) };
            }
            let maximum = if controls.dimension == Dimension::Pitch { 96.0 } else { 127.0 };
            let minimum = if controls.dimension == Dimension::Pitch { -96.0 } else { 0.0 };
            ui.horizontal_wrapped(|ui| {
                number(ui,"Expression beat within note",&mut controls.beat,0.0,inspection.duration);
                number(ui,"Per-note expression value",&mut controls.value,minimum,maximum);
                if enabled(ui,"Add or update note expression point",current).clicked() { modify = Some(Some(controls.value)); }
                if enabled(ui,"Remove selected note expression point",current && controls.selected.is_some()).clicked() { modify = Some(None); }
                if enabled(ui,"New note expression point",current).clicked() { controls.selected=None; }
            });
            let (rect,response)=ui.allocate_exact_size(egui::vec2(ui.available_width(),80.0),egui::Sense::hover());
            let painter=ui.painter_at(rect);painter.rect_filled(rect,0.0,theme.bg);
            let initial = match controls.dimension { Dimension::Pitch=>f64::from(inspection.initial.pitch), Dimension::Pressure=>f64::from(inspection.initial.pressure), Dimension::Timbre=>f64::from(inspection.initial.timbre) };
            let at=|beat:f64,value:f64|Pos2::new(rect.left()+(beat/inspection.duration)as f32*rect.width(),rect.bottom()-((value-minimum)/(maximum-minimum))as f32*rect.height());
            let mut last=at(0.0,initial);
            let points=&inspection.dimensions[controls.dimension.index()];
            let stride=points.len().div_ceil(1024).max(1);
            for position in points.iter().step_by(stride) {
                let point=&inspection.points[*position];
                let position=at(point.beat,point.value);
                painter.line_segment([last,Pos2::new(position.x,last.y)],egui::Stroke::new(1.0,theme.cyan));
                painter.line_segment([Pos2::new(position.x,last.y),position],egui::Stroke::new(1.0,theme.cyan));last=position;
            }
            painter.line_segment([last,Pos2::new(rect.right(),last.y)],egui::Stroke::new(1.0,theme.cyan));
            response.widget_info(||egui::WidgetInfo::labeled(egui::WidgetType::Other,true,"Per-note expression curve"));
            let shown=egui::ScrollArea::vertical().id_salt("note-expression-points").max_height(110.0).show_rows(ui,22.0,points.len(),|ui,range| {
                for row in range { let point=&inspection.points[points[row]];let label=format!("Note expression point {}: {:.9} beats · value {:.9} · bend range {}",row+1,point.beat,point.value,point.range);
                    let response=ui.selectable_label(controls.selected==Some(point.message),&label);accessibility::button(ui,&response,&label,Some(controls.selected==Some(point.message)));
                    if response.clicked(){controls.selected=Some(point.message);controls.beat=point.beat;controls.value=point.value;}
                }
            });
            accessibility::scrollbars(ui,"Note expression point list",&shown);
            if !current { ui.label("The draft changed. Inspect the selected note again before editing expression."); }
        }
        if !controls.message.is_empty(){let response=ui.label(&controls.message);accessibility::status(ui,&response,&controls.message);}
    });
    if inspect {
        let note = *draft.selected.iter().next().unwrap();
        if let Err(error) = controls.start(draft, engine, note, Operation::Inspect) {
            outcome = Err(error);
        }
    } else if let Some(value) = modify {
        let note = controls.inspection.as_ref().unwrap().note;
        let edit = Edit {
            dimension: controls.dimension,
            beat: controls.beat,
            value,
            original: controls.selected,
        };
        if let Err(error) = controls.start(draft, engine, note, Operation::Edit(edit)) {
            outcome = Err(error);
        }
    }
    if controls.busy() {
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(20));
    }
    draft.expression = controls;
    outcome
}

#[cfg(test)]
mod tests;
