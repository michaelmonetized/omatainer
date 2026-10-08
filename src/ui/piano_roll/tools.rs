use super::*;
use crate::engine::midi_tools::{
    self, Content, Expression, Kind, Parameters, Prepared, Property, Summary, CURVE_POINTS,
};
use std::collections::VecDeque;

pub(super) struct Tools {
    params: Parameters,
    seed: String,
    shape: usize,
    drag: Option<(usize, f64)>,
    warp_drag: usize,
    preview: Option<Preview>,
    worker: Option<Worker>,
    history: VecDeque<Parameters>,
    history_index: Option<usize>,
    message: String,
}
struct Original {
    content: Arc<Content>,
    region: Region,
    selected: BTreeSet<NoteId>,
    dirty: bool,
    controls_dirty: bool,
}
struct Preview {
    original: Arc<Original>,
    summary: Summary,
}
struct Worker {
    receiver: mpsc::Receiver<Result<Prepared, String>>,
    cancel: Arc<AtomicBool>,
    original: Arc<Original>,
    guard: Arc<Content>,
    region: Region,
    selected: BTreeSet<NoteId>,
    params: Parameters,
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
    }
}
impl Default for Tools {
    fn default() -> Self {
        Self{params:Parameters::default(),seed:"1".into(),shape:1,drag:None,warp_drag:1,preview:None,worker:None,history:VecDeque::new(),history_index:None,message:"Select notes, preview a transformation, then keep, restore or Apply MIDI edit. Ordinary draft editing waits until the preview is kept or restored.".into()}
    }
}
impl Tools {
    pub(super) fn busy(&self) -> bool {
        self.worker.is_some()
    }
    pub(super) fn editing(&self) -> bool {
        self.preview.is_none() && !self.busy()
    }
    pub(super) fn committed(&mut self) {
        self.preview = None;
        self.worker = None;
    }
    /// Read the current shared transformation parameters.
    /// Takes the tool panel state; returns validated repeatable seed parameters or a visible input error.
    pub(super) fn parameters(&self) -> Result<Parameters, String> {
        let mut params = self.params.clone();
        if params.kind == Kind::Recombine {
            params.seed = self
                .seed
                .parse::<u64>()
                .map_err(|_| "Seed must be a whole number from 0 through 18446744073709551615")?;
        }
        Ok(params)
    }
    fn preview(&mut self, draft: &Draft) -> Result<(), String> {
        if self.busy() {
            return Err("Wait for the current MIDI preview to finish".into());
        }
        if self.params.kind == Kind::Recombine {
            self.params.seed = self
                .seed
                .parse::<u64>()
                .map_err(|_| "Seed must be a whole number from 0 through 18446744073709551615")?;
        }
        let guard = Arc::new(draft.controls.content(&draft.notes));
        let original = self.preview.as_ref().map_or_else(
            || {
                Arc::new(Original {
                    content: guard.clone(),
                    region: draft.region,
                    selected: draft.selected.clone(),
                    dirty: draft.dirty,
                    controls_dirty: draft.controls.dirty,
                })
            },
            |p| p.original.clone(),
        );
        if draft.region != original.region || draft.selected != original.selected {
            return Err(
                "Keep or restore this preview before changing its note selection or clip bounds"
                    .into(),
            );
        }
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = cancel.clone();
        let source = original.clone();
        let params = self.params.clone();
        let worker_params = params.clone();
        let (sender, receiver) = mpsc::sync_channel(1);
        std::thread::Builder::new()
            .name("midi-transform-preview".into())
            .spawn(move || {
                let result = midi_tools::prepare(
                    &source.content,
                    &source.selected,
                    &worker_params,
                    &worker_cancel,
                );
                let _ = sender.send(result);
            })
            .map_err(|e| format!("MIDI preview worker could not start: {e}"))?;
        self.worker = Some(Worker {
            receiver,
            cancel,
            original,
            guard,
            region: draft.region,
            selected: draft.selected.clone(),
            params,
        });
        self.message = "Preparing the selected notes and their expression…".into();
        Ok(())
    }
    pub(super) fn poll(&mut self, draft: &mut Draft) -> Result<(), String> {
        let Some(worker) = &self.worker else {
            return Ok(());
        };
        let result = match worker.receiver.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => return Ok(()),
            Err(mpsc::TryRecvError::Disconnected) => {
                Err("MIDI preview worker disconnected; your draft is retained".into())
            }
        };
        let worker = self.worker.take().unwrap();
        if worker.guard.notes != draft.notes
            || !draft.controls.matches(&worker.guard)
            || worker.region != draft.region
            || worker.selected != draft.selected
        {
            self.preview = None;
            self.message = "Current draft retained after a stale preview.".into();
            return Err("The draft changed during preparation; the preview was refused and current work is retained".into());
        }
        if worker.cancel.load(Ordering::Acquire) {
            return Err("MIDI transformation cancelled; the draft is unchanged".into());
        }
        let mut prepared = result?;
        if !draft.region.allows(&prepared.content.notes) {
            return Err("This transformation would exceed the current loop's note-density limit; lengthen the loop or select fewer notes".into());
        }
        let controls_dirty = worker.original.controls_dirty
            || !worker.original.content.same_lanes(&prepared.content);
        draft.notes = std::mem::take(&mut prepared.content.notes);
        draft
            .controls
            .install(&mut prepared.content, controls_dirty);
        draft.dirty = true;
        if self
            .history_index
            .is_some_and(|i| i + 1 < self.history.len())
        {
            self.history.truncate(self.history_index.unwrap() + 1);
        }
        if self.history.back() != Some(&worker.params) {
            if self.history.len() == 32 {
                self.history.pop_front();
            }
            self.history.push_back(worker.params.clone());
        }
        self.history_index = self.history.len().checked_sub(1);
        self.message=format!("{} selected notes · {} linked expression events · seed {}. Preview is in the draft; Apply MIDI edit commits one history change.",prepared.summary.transformed.notes,prepared.summary.expression_events,worker.params.seed);
        self.preview = Some(Preview {
            original: worker.original.clone(),
            summary: prepared.summary,
        });
        Ok(())
    }
    fn restore(&mut self, draft: &mut Draft) -> Result<(), String> {
        if self.busy() {
            return Err("Wait for the current MIDI preview before restoring".into());
        }
        if let Some(preview) = self.preview.take() {
            let mut content = (*preview.original.content).clone();
            draft.notes = std::mem::take(&mut content.notes);
            draft
                .controls
                .install(&mut content, preview.original.controls_dirty);
            draft.region = preview.original.region;
            draft.selected = preview.original.selected.clone();
            draft.dirty = preview.original.dirty;
            self.message =
                "Original notes, controller content, selection and clip bounds restored.".into();
        }
        Ok(())
    }
    fn keep(&mut self, draft: &mut Draft) {
        if self.preview.take().is_some() {
            draft.steps.clear();
            draft.step_chord.clear();
            draft.rhythm.committed();
            self.message="Preview kept in this draft. Select notes for another tool or Apply MIDI edit to commit the complete draft once.".into();
        }
    }
    fn history(&mut self, offset: isize) {
        if let Some(index) = self.history_index {
            let next = index
                .saturating_add_signed(offset)
                .min(self.history.len().saturating_sub(1));
            self.params = self.history[next].clone();
            self.seed = self.params.seed.to_string();
            self.shape = 6;
            self.history_index = Some(next);
            self.message="Prior settings restored. Preview regenerates them from the captured original selection.".into();
        }
    }
}
fn check(ui: &mut Ui, label: &str, value: &mut bool) {
    let response = ui.checkbox(value, label);
    accessibility::button(ui, &response, label, Some(*value));
    help::annotate(ui, &response, HelpControl::MidiTransform);
}
fn enabled(ui: &mut Ui, label: &str, on: bool) -> egui::Response {
    let response = ui.add_enabled(on, egui::Button::new(label));
    accessibility::button(ui, &response, label, None);
    help::annotate(ui, &response, HelpControl::MidiTransform);
    response
}
fn value(ui: &mut Ui, label: &str, value: &mut u8, min: u8, max: u8) {
    let mut next = f64::from(*value);
    if number(ui, label, &mut next, f64::from(min), f64::from(max)) {
        *value = next.round() as u8;
    }
}
fn curve(ui: &mut Ui, tools: &mut Tools, theme: &Theme) {
    let p = &mut tools.params;
    let previous = tools.shape;
    egui::ComboBox::from_id_salt("midi-velocity-shape")
        .selected_text(
            [
                "Flat", "Rise", "Fall", "Sine", "Triangle", "Pulse", "Custom",
            ][tools.shape],
        )
        .show_ui(ui, |ui| {
            for (i, label) in [
                "Flat", "Rise", "Fall", "Sine", "Triangle", "Pulse", "Custom",
            ]
            .iter()
            .enumerate()
            {
                ui.selectable_value(&mut tools.shape, i, *label);
            }
        });
    if tools.shape != previous && tools.shape < 6 {
        p.curve = std::array::from_fn(|i| {
            let x = i as f64 / (CURVE_POINTS - 1) as f64;
            match tools.shape {
                0 => 0.5,
                1 => x,
                2 => 1.0 - x,
                3 => 0.5 - 0.5 * (x * std::f64::consts::TAU).cos(),
                4 => 1.0 - (2.0 * x - 1.0).abs(),
                _ => {
                    if x < 0.5 {
                        1.0
                    } else {
                        0.0
                    }
                }
            }
        });
    }
    ui.horizontal_wrapped(|ui| {
        value(ui, "Velocity minimum", &mut p.velocity[0], 1, 127);
        value(ui, "Velocity maximum", &mut p.velocity[1], 1, 127);
        value(ui, "Velocity cycles", &mut p.cycles, 1, 16);
        number(ui, "Velocity phase", &mut p.phase, 0.0, 1.0);
    });
    let (response, painter) = ui.allocate_painter(
        Vec2::new(ui.available_width().max(1.0), 110.0),
        egui::Sense::click_and_drag(),
    );
    let rect = response.rect;
    accessibility::button(ui, &response, "Draw velocity curve", None);
    painter.rect_filled(rect, 0.0, theme.bg_dark);
    if response.drag_started() || response.clicked() {
        tools.drag = None;
    }
    if response.dragged() || response.clicked() {
        if let Some(point) = response.interact_pointer_pos() {
            let x = ((point.x - rect.left()) / rect.width()).clamp(0.0, 1.0);
            let y = ((rect.bottom() - point.y) / rect.height()).clamp(0.0, 1.0) as f64;
            let index = (x * (CURVE_POINTS - 1) as f32).round() as usize;
            if let Some((old, level)) = tools.drag {
                let start = old.min(index);
                let end = old.max(index);
                for i in start..=end {
                    p.curve[i] = if index == old {
                        y
                    } else {
                        level + (y - level) * (i as f64 - old as f64) / (index as f64 - old as f64)
                    };
                }
            } else {
                p.curve[index] = y;
            }
            tools.drag = Some((index, y));
            tools.shape = 6;
        }
    }
    if response.drag_stopped() {
        tools.drag = None;
    }
    let point = |i: usize| {
        Pos2::new(
            rect.left() + i as f32 / (CURVE_POINTS - 1) as f32 * rect.width(),
            rect.bottom() - p.curve[i] as f32 * rect.height(),
        )
    };
    for i in 1..CURVE_POINTS {
        painter.line_segment(
            [point(i - 1), point(i)],
            egui::Stroke::new(1.5_f32, theme.cyan),
        );
    }
    ui.label("Draw a curve or edit its points. It follows the first through last selected note onset and repeats by Cycles. Strength blends it with the original velocities.");
    egui::CollapsingHeader::new("Velocity curve points")
        .id_salt("velocity-curve-points")
        .show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                for (i, v) in p.curve.iter_mut().enumerate() {
                    if number(ui, &format!("Velocity point {}", i + 1), v, 0.0, 1.0) {
                        tools.shape = 6;
                    }
                }
            });
        });
}
fn warp(ui: &mut Ui, tools: &mut Tools, theme: &Theme) {
    let p = &mut tools.params;
    ui.horizontal_wrapped(|ui| {
        number(ui, "Warp middle time", &mut p.warp_time, 0.01, 0.99);
        for (i, value) in p.warp_speed.iter_mut().enumerate() {
            number(ui, &format!("Warp speed {}", i + 1), value, 0.125, 8.0);
        }
    });
    check(ui, "Preserve phrase range", &mut p.preserve_range);
    check(ui, "Warp note ends", &mut p.ends);
    let (response, painter) = ui.allocate_painter(
        Vec2::new(ui.available_width().max(1.0), 100.0),
        egui::Sense::click_and_drag(),
    );
    let rect = response.rect;
    accessibility::button(ui, &response, "Draw time curve", None);
    painter.rect_filled(rect, 0.0, theme.bg_dark);
    let positions = [0.0, p.warp_time, 1.0];
    if response.drag_started() || response.clicked() {
        if let Some(point) = response.interact_pointer_pos() {
            let x = ((point.x - rect.left()) / rect.width()).clamp(0.0, 1.0) as f64;
            tools.warp_drag = (0..3)
                .min_by(|&a, &b| {
                    (positions[a] - x)
                        .abs()
                        .total_cmp(&(positions[b] - x).abs())
                })
                .unwrap();
        }
    }
    if response.dragged() || response.clicked() {
        if let Some(point) = response.interact_pointer_pos() {
            let x = ((point.x - rect.left()) / rect.width()).clamp(0.01, 0.99) as f64;
            let y = ((rect.bottom() - point.y) / rect.height()).clamp(0.0, 1.0) as f64;
            if tools.warp_drag == 1 {
                p.warp_time = x;
            }
            p.warp_speed[tools.warp_drag] = 2.0_f64.powf(6.0 * y - 3.0);
        }
    }
    let xs = [0.0, p.warp_time, 1.0];
    let point = |i: usize| {
        Pos2::new(
            rect.left() + xs[i] as f32 * rect.width(),
            rect.bottom() - ((p.warp_speed[i].log2() + 3.0) / 6.0) as f32 * rect.height(),
        )
    };
    for i in 1..3 {
        painter.line_segment(
            [point(i - 1), point(i)],
            egui::Stroke::new(1.5_f32, theme.cyan),
        );
    }
    for i in 0..3 {
        painter.circle_filled(point(i), 4.0, theme.yellow);
    }
    ui.label("Speed 1 preserves time; higher speeds shorten it. Drag the three points or edit their values. Preserve phrase range fits the curve to the original extent.");
}
pub(super) fn show(ui: &mut Ui, draft: &mut Draft, theme: &Theme) -> Result<bool, String> {
    let mut tools = std::mem::take(&mut draft.tools);
    let mut preview = false;
    let mut restore = false;
    let mut keep = false;
    egui::CollapsingHeader::new("MIDI transformations").id_salt("midi-tools").show(ui,|ui|{
        ui.label(&tools.message);
        egui::ComboBox::from_id_salt("midi-tool-kind").selected_text(match tools.params.kind{Kind::Quantize=>"Quantize",Kind::Recombine=>"Recombine",Kind::Velocity=>"Velocity curve",Kind::Stretch=>"Stretch",Kind::Reverse=>"Reverse phrase",Kind::Warp=>"Time curve"}).show_ui(ui,|ui|{for(kind,label)in[(Kind::Quantize,"Quantize"),(Kind::Recombine,"Recombine"),(Kind::Velocity,"Velocity curve"),(Kind::Stretch,"Stretch"),(Kind::Reverse,"Reverse phrase"),(Kind::Warp,"Time curve")]{ui.selectable_value(&mut tools.params.kind,kind,label);}});
        match tools.params.kind {
            Kind::Quantize=>{ui.horizontal_wrapped(|ui|{
                egui::ComboBox::from_id_salt("midi-tool-grid").selected_text(GRIDS.iter().skip(1).find(|(_,value)|(*value-tools.params.grid).abs()<1e-9).map_or("Custom grid",|(name,_)|*name)).show_ui(ui,|ui|{for(name,value)in GRIDS.iter().skip(1){ui.selectable_value(&mut tools.params.grid,*value,*name);}});
                check(ui,"Quantize starts",&mut tools.params.starts);check(ui,"Quantize ends",&mut tools.params.ends);number(ui,"Transform strength",&mut tools.params.strength,0.0,1.0);
            });}
            Kind::Recombine=>{ui.horizontal_wrapped(|ui|{
                egui::ComboBox::from_id_salt("midi-tool-property").selected_text(match tools.params.property{Property::Pitch=>"Pitch",Property::Position=>"Position",Property::Length=>"Length",Property::Velocity=>"Velocity"}).show_ui(ui,|ui|{for(property,label)in[(Property::Pitch,"Pitch"),(Property::Position,"Position"),(Property::Length,"Length"),(Property::Velocity,"Velocity")]{ui.selectable_value(&mut tools.params.property,property,label);}});
                check(ui,"Shuffle property",&mut tools.params.shuffle);check(ui,"Mirror property",&mut tools.params.mirror);let mut value=f64::from(tools.params.rotation);if number(ui,"Property rotation",&mut value,-8191.0,8191.0){tools.params.rotation=value.round()as i32;}
            });let label=ui.label("Seed");let response=ui.add(egui::TextEdit::singleline(&mut tools.seed).char_limit(20).desired_width(150.0)).labelled_by(label.id);response.widget_info(||egui::WidgetInfo::labeled(egui::WidgetType::TextEdit,true,"Transformation seed"));ui.label("Shuffle, then Mirror, then Rotation move only the chosen property. Stable note identities retain their expression.");}
            Kind::Velocity=>{number(ui,"Transform strength",&mut tools.params.strength,0.0,1.0);curve(ui,&mut tools,theme);}
            Kind::Stretch=>{number(ui,"Stretch factor",&mut tools.params.stretch,0.125,8.0);ui.label("Stretch moves starts and ends around the first selected note. Clip and loop bounds stay fixed.");}
            Kind::Reverse=>{ui.label("Reverse reflects the selected phrase and its expression around its first start and last end. Pitch, note identity and velocity stay attached.");}
            Kind::Warp=>warp(ui,&mut tools,theme),
        }
        ui.horizontal_wrapped(|ui|{
            let mode=match tools.params.expression{Expression::PolyPressure=>0,Expression::Lower(_)=>1,Expression::Upper(_)=>2};let mut chosen=mode;
            egui::ComboBox::from_id_salt("midi-tool-expression").selected_text(["Poly pressure only","MPE lower zone","MPE upper zone"][mode]).show_ui(ui,|ui|{for(i,label)in["Poly pressure only","MPE lower zone","MPE upper zone"].iter().enumerate(){ui.selectable_value(&mut chosen,i,*label);}});
            let mut members=match tools.params.expression{Expression::PolyPressure=>15,Expression::Lower(n)|Expression::Upper(n)=>n};if chosen>0{value(ui,"MPE member channels",&mut members,1,15);}tools.params.expression=match chosen{1=>Expression::Lower(members),2=>Expression::Upper(members),_=>Expression::PolyPressure};
        });
        ui.label("MPE member pitch bend, pressure and CC74 follow their unique note. Manager and other channel automation keep their timeline. Overlapping or unowned expression that could affect another voice refuses the preview. Imported timing uses its original ticks; rounding is shown below.");
        if let Some(p)=&tools.preview{let a=p.summary.original;let b=p.summary.transformed;let label=format!("Original {:.9}–{:.9} beats · duration {:.9} · gaps {:.9} · overlap {}. Preview {:.9}–{:.9} · duration {:.9} · gaps {:.9} · overlap {}. Mean start shift {:.9}; maximum timing rounding {:.12} beats.",a.first,a.end,a.total_length,a.total_gap,a.maximum_overlap,b.first,b.end,b.total_length,b.total_gap,b.maximum_overlap,p.summary.mean_start_shift,p.summary.maximum_rounding_beats);let response=ui.label(&label);accessibility::status(ui,&response,&label);}
        ui.horizontal_wrapped(|ui|{
            preview=enabled(ui,"Preview MIDI transformation",!tools.busy()).clicked();restore=enabled(ui,"Restore original MIDI preview",tools.preview.is_some()&&!tools.busy()).clicked();keep=enabled(ui,"Keep MIDI preview in draft",tools.preview.is_some()&&!tools.busy()).clicked();
            if enabled(ui,"Previous transform settings",tools.history_index.is_some_and(|i|i>0)&&!tools.busy()).clicked(){tools.history(-1);}
            if enabled(ui,"Next transform settings",tools.history_index.is_some_and(|i|i+1<tools.history.len())&&!tools.busy()).clicked(){tools.history(1);}
        });
    });
    let result = if preview {
        tools.preview(draft)
    } else if restore {
        tools.restore(draft)
    } else {
        if keep {
            tools.keep(draft);
        }
        Ok(())
    };
    draft.tools = tools;
    if result.is_ok() && (preview || restore || keep) {
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(20));
    }
    result.map(|_| preview || restore || keep)
}

#[cfg(test)]
mod tests;
