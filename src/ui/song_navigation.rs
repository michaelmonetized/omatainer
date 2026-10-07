use super::*;
use crate::engine::{
    clip_launch::Grid,
    midi_edit::{Ack, Outcome},
    song_navigation::{metadata::Loop, Action, Model, Saved},
};
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(test)]
mod tests;
mod worker;

#[derive(Default)]
pub(super) struct Panel {
    review: Option<Saved>,
    namespace: Option<[u64; 2]>,
    model: Model,
    selected: Option<u16>,
    name: String,
    beat: f64,
    seconds: f64,
    grid: Grid,
    bounds: [f64; 2],
    dirty: bool,
    worker: Option<worker::Worker>,
    active: Option<Arc<AtomicBool>>,
    pending: Option<Ack>,
    message: String,
}
impl Drop for Panel {
    fn drop(&mut self) {
        self.cancel();
    }
}
fn same(a: Option<&Saved>, b: Option<&Saved>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => a.same(b),
        _ => false,
    }
}
impl Panel {
    fn busy(&self) -> bool {
        self.active.is_some() || self.pending.is_some()
    }
    pub(super) fn cancel(&mut self) {
        if let Some(cancel) = &self.active {
            cancel.store(true, Ordering::Release);
        }
        if let Some(ack) = &self.pending {
            ack.cancel();
        }
    }
    fn refresh(&mut self, snapshot: &Snapshot) {
        self.review = snapshot.navigation.clone();
        self.namespace = snapshot.session.as_ref().map(|s| s.namespace);
        self.model = self
            .review
            .as_ref()
            .map_or_else(Model::default, |s| (*s.model).clone());
        if let Some(saved) = &self.review {
            self.model.next_id = saved.next_id;
        }
        self.bounds = self
            .model
            .loop_region
            .map_or([0.0, 4.0], |r| [r.start, r.end]);
        self.dirty = false;
        if !self
            .model
            .locators
            .iter()
            .any(|l| Some(l.id) == self.selected)
        {
            self.selected = self.model.locators.first().map(|l| l.id);
        }
        self.select();
    }
    fn select(&mut self) {
        if let Some(locator) = self
            .model
            .locators
            .iter()
            .find(|l| Some(l.id) == self.selected)
        {
            self.name = locator.name.clone();
            self.beat = locator.beat;
        }
    }
    fn start(
        &mut self,
        engine: &Engine,
        expected: Option<Saved>,
        model: Model,
        looping: bool,
        jump: Option<(f64, Grid)>,
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
            let cancel = work.cancel();
            if self.worker.is_none() {
                self.worker = Some(worker::Worker::start(
                    engine.project.clone(),
                    engine.cmd.clone(),
                )?);
            }
            self.worker
                .as_ref()
                .unwrap()
                .jobs
                .try_send(worker::Job {
                    expected,
                    model,
                    looping,
                    jump,
                    work,
                })
                .map_err(|_| "Song section worker is busy or disconnected")?;
            self.active = Some(cancel);
            self.message = "Preparing reviewed song sections…".into();
            Ok(())
        })();
        if let Err(error) = result {
            self.message = error;
        }
    }
    pub(super) fn poll(&mut self) {
        if let Some(worker) = &self.worker {
            if let Ok(event) = worker.events.try_recv() {
                let cancelled = self
                    .active
                    .take()
                    .is_some_and(|c| c.load(Ordering::Acquire));
                match event {
                    worker::Event::Queued(ack) => {
                        if cancelled {
                            ack.cancel();
                        }
                        self.pending = Some(ack);
                    }
                    worker::Event::Failed(error) => self.message = error,
                }
            }
        }
        if let Some(ack) = &self.pending {
            let state = ack.state();
            if !matches!(state, Outcome::Pending) {
                self.pending = None;
                self.message=match state {
                    Outcome::Applied=>{self.dirty=false;"Song sections saved in the current project. Save the project to keep them on disk."},
                    Outcome::Cancelled=>"Section edit cancelled; the live song is unchanged.",
                    _=>"Sections changed during review; discard the draft and review again.",
                }.into();
            }
        }
    }
}
fn number(ui: &mut Ui, label: &str, value: &mut f64, min: f64, max: f64) -> bool {
    let before = *value;
    ui.horizontal(|ui| {
        ui.label(label);
        let response = ui.add(egui::DragValue::new(value).range(min..=max).speed(0.25));
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::DragValue, response.enabled(), label)
        });
        ui.ctx().accesskit_node_builder(response.id, |n| {
            n.set_label(label);
            n.set_numeric_value(*value);
            n.set_min_numeric_value(min);
            n.set_max_numeric_value(max);
            n.add_action(egui::accesskit::Action::SetValue);
        });
        if response.enabled() {
            ui.input(|i| {
                for a in i.accesskit_action_requests(response.id, egui::accesskit::Action::SetValue)
                {
                    if let Some(egui::accesskit::ActionData::NumericValue(v)) = a.data {
                        if v.is_finite() {
                            *value = v.clamp(min, max);
                        }
                    }
                }
            });
        }
    });
    *value != before
}
impl App {
    pub(super) fn song_navigation_ui(&mut self, ui: &mut Ui, start: f64, width: f64) {
        let mut panel = std::mem::take(&mut self.song_navigation);
        panel.poll();
        let namespace = self.snap.session.as_ref().map(|s| s.namespace);
        if panel.namespace != namespace {
            panel.cancel();
            panel.refresh(&self.snap);
        } else if !panel.busy()
            && !panel.dirty
            && !same(panel.review.as_ref(), self.snap.navigation.as_ref())
        {
            panel.refresh(&self.snap);
        }
        ui.push_id("song_navigation",|ui|{
            egui::CollapsingHeader::new("Song sections and loop").show(ui, |ui| {
            ui.label("Beat positions count quarter notes from zero. Sections keep their MIDI ID when moved or renamed.");
            ui.horizontal_wrapped(|ui|{
                let response=egui::ComboBox::from_id_salt("section_timing").selected_text(panel.grid.label()).show_ui(ui,|ui|{
                    for grid in Grid::ALL{if ui.selectable_value(&mut panel.grid,grid,grid.label()).clicked(){ui.close();}}
                });
                accessibility::button(ui,&response.response,"Section jump timing",None);
                for (label,action) in [("Previous section",Action::Previous(panel.grid)),("Next section",Action::Next(panel.grid)),("Toggle song loop",Action::ToggleLoop),("Cancel queued song jump",Action::Cancel)] {
                    if ui.button(label).clicked(){self.send(Command::SongNavigation(action));}
                }
                ui.label(if self.snap.navigation.as_ref().is_some_and(|s|s.looping){"Loop on"}else{"Loop off"});
            });
            ui.horizontal_wrapped(|ui|{
                number(ui,"Jump to beat",&mut panel.beat,0.0,262144.0);
                if ui.button("Jump to entered beat").clicked(){self.send(Command::SongNavigation(Action::Beat{beat:panel.beat,grid:panel.grid}));}
                number(ui,"Jump to seconds",&mut panel.seconds,0.0,86400.0);
                if ui.button("Jump to entered time").clicked(){self.send(Command::SongNavigation(Action::Seconds(panel.seconds)));}
            });
            if let Some([beat,when])=self.snap.navigation_pending{ui.label(format!("Queued: beat {beat:.6} at beat {when:.6}"));}
            if let Some(error)=self.snap.navigation_error{ui.label(error.label());}
            marks(ui,&self.snap.navigation,start,width,self.snap.beat,&self.theme);
            ui.label(&panel.message);
            if panel.busy(){ui.spinner();if ui.button("Cancel section edit").clicked(){panel.cancel();}}
            let allowed=!panel.busy() && !self.engine.cmd.performance().protected() && !self.project.committing();
            ui.add_enabled_ui(allowed,|ui|{
                if panel.dirty && !same(panel.review.as_ref(),self.snap.navigation.as_ref()){ui.label("Live sections changed. Discard this draft before editing the new song state.");}
                egui::ScrollArea::vertical().id_salt("sections").max_height(140.0).show(ui,|ui|{
                    let mut select=None;
                    for locator in &panel.model.locators{
                        ui.push_id(locator.id,|ui|ui.horizontal(|ui|{
                            if ui.selectable_label(panel.selected==Some(locator.id),format!("{} · ID {} · beat {}",locator.name,locator.id,locator.beat)).clicked(){select=Some(locator.id);}
                            let live=self.snap.navigation.as_ref().is_some_and(|s|s.model.destination(locator.id).is_some());
                            if ui.add_enabled(live,egui::Button::new(format!("Jump to {}",locator.name))).clicked(){self.send(Command::SongNavigation(Action::Locator{id:locator.id,grid:panel.grid}));}
                        }));
                    }
                    if let Some(id)=select{panel.selected=Some(id);panel.select();}
                });
                ui.horizontal_wrapped(|ui|{
                    preferences::text(ui,"Section name",&mut panel.name,HelpControl::Arrangement);
                    number(ui,"Section beat",&mut panel.beat,0.0,262144.0);
                    if ui.button("Use current song beat").clicked(){panel.beat=self.snap.beat;}
                    if ui.button("Add section").clicked(){match panel.model.add(panel.name.clone(),panel.beat){Ok(id)=>{panel.selected=Some(id);panel.dirty=true;},Err(e)=>panel.message=e}}
                    for (label,delete) in [("Update section",false),("Delete section",true)]{
                        if ui.add_enabled(panel.selected.is_some(),egui::Button::new(label)).clicked(){let id=panel.selected.unwrap();let result=if delete{panel.model.delete(id)}else{panel.model.update(id,panel.name.clone(),panel.beat)};match result{Ok(())=>{panel.dirty=true;if delete{panel.selected=None;}},Err(e)=>panel.message=e};}
                    }
                });
                ui.horizontal_wrapped(|ui|{
                    number(ui,"Song loop start",&mut panel.bounds[0],0.0,262144.0);
                    number(ui,"Song loop end",&mut panel.bounds[1],0.0,262144.0);
                    if ui.button("Set loop braces").clicked(){let region=Loop{start:panel.bounds[0],end:panel.bounds[1]};if region.valid(){panel.model.loop_region=Some(region);panel.dirty=true;}else{panel.message="Loop end must follow loop start by at least 1/960 beat.".into();}}
                    if ui.button("Remove loop braces").clicked(){panel.model.loop_region=None;panel.dirty=true;}
                    if ui.add_enabled(!panel.dirty && panel.selected.is_some(),egui::Button::new("Loop section to next")).clicked(){
                        let result=panel.model.loop_to_next(panel.selected.unwrap());
                        match result{Ok(region)=>{let mut model=panel.model.clone();model.loop_region=Some(region);panel.start(&self.engine,self.snap.navigation.clone(),model,true,Some((region.start,panel.grid)));},Err(e)=>panel.message=e}
                    }
                });
                ui.horizontal(|ui|{
                    if ui.add_enabled(panel.dirty,egui::Button::new("Save song sections")).clicked(){
                        let looping=panel.model.loop_region.is_some() && panel.review.as_ref().is_some_and(|s|s.looping);
                        panel.start(&self.engine,panel.review.clone(),panel.model.clone(),looping,None);
                    }
                    if ui.button("Discard section draft").clicked(){panel.refresh(&self.snap);panel.message.clear();}
                    if panel.dirty{ui.label("Unsaved section draft");}
                });
            });
            ui.label("A jump exits the current loop, releases old song notes and restores destination MIDI state. Jumps and loop wraps finish the current clip recording take. Pending jumps are cleared on Stop or project replacement; reopening remembers braces and the loop switch, and waits for Play.");
            ui.separator();
            });
        });
        self.song_navigation = panel;
    }
}
/// Draw saved section markers and editable loop bounds beside the song timeline.
/// Takes the native UI, saved model, visible beat range, playhead and theme; returns one clipped marker strip without touching audio ownership.
pub(super) fn marks(
    ui: &mut Ui,
    saved: &Option<Saved>,
    start: f64,
    width: f64,
    beat: f64,
    theme: &Theme,
) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 42.0), Sense::hover());
    let painter = ui.painter().with_clip_rect(rect);
    painter.rect_filled(rect, 2.0, theme.bg);
    let x = |beat: f64| rect.left() + rect.width() * ((beat - start) / width.max(0.25)) as f32;
    if let Some(saved) = saved {
        if let Some(region) = saved.model.loop_region {
            let bounds = Rect::from_x_y_ranges(x(region.start)..=x(region.end), rect.y_range());
            painter.rect_filled(
                bounds,
                0.0,
                theme
                    .fg
                    .gamma_multiply(if saved.looping { 0.2 } else { 0.08 }),
            );
            painter.rect_stroke(
                bounds,
                0.0,
                Stroke::new(1.5_f32, theme.fg),
                egui::StrokeKind::Inside,
            );
        }
        for locator in &saved.model.locators {
            let at = x(locator.beat);
            if rect.x_range().contains(at) {
                painter.line_segment(
                    [Pos2::new(at, rect.top()), Pos2::new(at, rect.bottom())],
                    Stroke::new(1.0_f32, theme.fg_dim),
                );
                painter.text(
                    Pos2::new(at + 3.0, rect.top() + 3.0),
                    egui::Align2::LEFT_TOP,
                    &locator.name,
                    FontId::proportional(11.0),
                    theme.fg,
                );
            }
        }
    }
    painter.line_segment(
        [
            Pos2::new(x(beat), rect.top()),
            Pos2::new(x(beat), rect.bottom()),
        ],
        Stroke::new(2.0_f32, Color32::WHITE),
    );
}
