//! Editable tempo, meter and click settings on the shared musical timeline.
use super::*;
use crate::engine::{
    midi_data::{Conductor, Meter, Tempo, TimingSettings},
    midi_edit::{Ack, Outcome},
};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
mod worker;
use worker::{Job, Worker};

#[derive(Clone)]
struct Draft {
    namespace: [u64; 2],
    baseline: Option<Conductor>,
    bpm: f32,
    ppqn: u16,
    tempos: String,
    meters: String,
    settings: TimingSettings,
    dirty: bool,
}
impl Draft {
    fn new(snap: &crate::engine::Snapshot) -> Option<Self> {
        let map = snap.timing.as_deref();
        let ppqn = map.map_or(960, |map| map.ppqn);
        Some(Self {
            namespace: snap.session.as_ref()?.namespace,
            baseline: map.cloned(),
            bpm: snap.bpm,
            ppqn,
            tempos: map.map_or_else(
                || format!("0 {:.6} step", snap.bpm),
                |map| {
                    map.tempos
                        .iter()
                        .map(|p| {
                            format!(
                                "{:.9} {:.9} {}",
                                p.tick as f64 / f64::from(ppqn),
                                60000000.0 / f64::from(p.micros),
                                if p.ramp { "ramp" } else { "step" }
                            )
                        })
                        .collect::<Vec<_>>()
                        .join("\n")
                },
            ),
            meters: map.map_or_else(
                || "0 4/4".into(),
                |map| {
                    map.meters
                        .iter()
                        .map(|m| {
                            format!(
                                "{:.9} {}/{}",
                                m.tick as f64 / f64::from(ppqn),
                                m.numerator,
                                1u16 << m.denominator_power
                            )
                        })
                        .collect::<Vec<_>>()
                        .join("\n")
                },
            ),
            settings: map.and_then(|map| map.native).unwrap_or_default(),
            dirty: false,
        })
    }
    fn map(&self) -> Result<Arc<Conductor>, String> {
        let tick = |text: &str| -> Result<u64, String> {
            let beat: f64 = text.parse().map_err(|_| "Beat position must be a number")?;
            let ticks = beat * f64::from(self.ppqn);
            if !beat.is_finite()
                || !(0.0..=262144.0).contains(&beat)
                || (ticks - ticks.round()).abs() > 0.0001
            {
                return Err(format!(
                    "Beat positions must be 0–262144 on the 1/{} beat grid",
                    self.ppqn
                ));
            }
            Ok(ticks.round() as u64)
        };
        let tempos = rows(&self.tempos)?
            .iter()
            .map(|row| {
                if row.len() != 3 || !["step", "ramp"].contains(&row[2]) {
                    return Err("Each tempo row needs: beat BPM step, or beat BPM ramp".into());
                }
                Tempo::new(
                    tick(row[0])?,
                    row[1].parse().map_err(|_| "BPM must be a number")?,
                    row[2] == "ramp",
                )
            })
            .collect::<Result<Vec<_>, String>>()?;
        let meters = rows(&self.meters)?
            .iter()
            .map(|row| {
                if row.len() != 2 {
                    return Err("Each meter row needs: beat numerator/denominator".into());
                }
                let (a, b) = row[1]
                    .split_once('/')
                    .ok_or("Enter meter as numerator/denominator")?;
                let numerator: u8 = a.parse().map_err(|_| "Meter numerator must be 1–255")?;
                let denominator: u16 = b
                    .parse()
                    .map_err(|_| "Meter denominator must be 1, 2, 4, 8, 16, 32, 64 or 128")?;
                if !denominator.is_power_of_two() || denominator > 128 {
                    return Err("Meter denominator must be a power of two through 128".into());
                }
                let meter = Meter {
                    tick: tick(row[0])?,
                    numerator,
                    denominator_power: denominator.trailing_zeros() as u8,
                    clocks: (96 / denominator).max(1) as u8,
                    thirty_seconds: 8,
                };
                Ok(self
                    .baseline
                    .as_ref()
                    .and_then(|map| {
                        map.meters.iter().find(|old| {
                            old.tick == meter.tick
                                && old.numerator == meter.numerator
                                && old.denominator_power == meter.denominator_power
                        })
                    })
                    .copied()
                    .unwrap_or(meter))
            })
            .collect::<Result<Vec<_>, String>>()?;
        Conductor::native(self.ppqn, tempos, meters, self.settings)
    }
}

fn rows(text: &str) -> Result<Vec<Vec<&str>>, String> {
    let rows: Vec<_> = text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| line.split_whitespace().collect::<Vec<_>>())
        .collect();
    if rows.is_empty() || rows.len() > crate::engine::midi_data::MAX_CONDUCTOR_POINTS {
        return Err("Enter 1–4096 ordered points, starting at beat 0".into());
    }
    Ok(rows)
}

#[derive(Default)]
pub(super) struct Editor {
    pub open: bool,
    draft: Option<Draft>,
    worker: Option<Worker>,
    active: Option<Arc<AtomicBool>>,
    pending: Option<Ack>,
    message: String,
    error: Option<String>,
    confirm_discard: bool,
    discard_when_settled: bool,
}
impl Drop for Editor {
    fn drop(&mut self) {
        self.cancel();
    }
}
impl Editor {
    fn busy(&self) -> bool {
        self.active.is_some() || self.pending.is_some()
    }
    pub(super) fn blocks_close(&self) -> bool {
        self.busy() || self.draft.as_ref().is_some_and(|draft| draft.dirty)
    }
    /// Keep a timing draft when a project replacement is requested.
    /// Returns whether replacement is blocked and opens explicit keep/discard review.
    pub(super) fn guard_replacement(&mut self) -> bool {
        if !self.blocks_close() { return false; }
        self.open = true; self.confirm_discard = true; true
    }
    /// Clear the old session's settled editor after a guarded replacement.
    /// Reopening captures the new namespace and timing; no dirty draft is discarded.
    pub(super) fn reset_project(&mut self) {
        self.cancel(); self.draft = None; self.open = false; self.confirm_discard = false;
        self.discard_when_settled = false; self.message.clear(); self.error = None;
    }
    pub(super) fn cancel(&self) {
        if let Some(cancel) = &self.active {
            cancel.store(true, Ordering::Release);
        }
        if let Some(ack) = &self.pending {
            ack.cancel();
        }
    }
    pub(super) fn poll(&mut self, engine: &Engine) {
        if let Some(worker) = &self.worker {
            match worker.events.try_recv() {
                Ok(result) => {
                    let cancelled = self
                        .active
                        .take()
                        .is_some_and(|c| c.load(Ordering::Acquire));
                    match result {
                        Ok(ack) => {
                            if cancelled {
                                ack.cancel();
                            }
                            self.pending = Some(ack);
                        }
                        Err(error) => {
                            self.error = Some(error);
                        }
                    }
                }
                Err(crossbeam_channel::TryRecvError::Disconnected) if self.active.is_some() => {
                    self.active = None;
                    self.error = Some(
                        "Timing worker disconnected. Draft retained; restart to retry.".into(),
                    );
                }
                _ => {}
            }
        }
        if let Some(ack) = &self.pending {
            match ack.state() {
                Outcome::Applied => {
                    self.pending = None;
                    self.error = None;
                    if let Some(draft) = &mut self.draft {
                        draft.baseline = draft.map().ok().map(|map| (*map).clone());
                        draft.dirty = false;
                    }
                    self.message =
                        "Timing applied as one History entry. Save the project to keep it.".into();
                }
                Outcome::Rejected | Outcome::Cancelled => {
                    let state = ack.state();
                    self.pending = None;
                    self.error = Some(if state == Outcome::Cancelled { "Timing edit cancelled. Draft retained." } else { "Timing edit refused because the project, timing, recording or protection changed. Draft retained; review the current timeline." }.into());
                }
                Outcome::Pending if !engine.cmd.is_connected() => {
                    self.pending = None;
                    self.error = Some("Renderer disconnected before confirming the timing edit. Its outcome is unknown; draft retained.".into());
                }
                _ => {}
            }
        }
        if self.discard_when_settled && !self.busy() {
            self.draft = None;
            self.open = false;
            self.discard_when_settled = false;
        }
    }
    fn apply(&mut self, engine: &Engine) {
        if self.busy() {
            return;
        }
        let Some(draft) = self.draft.clone() else {
            return;
        };
        let result = (|| {
            let work = engine
                .cmd
                .performance()
                .optional_work()
                .map_err(|e| e.to_string())?;
            if self.worker.is_none() {
                self.worker = Some(Worker::start(engine.project.clone(), engine.cmd.clone())?);
            }
            let cancel = work.cancel();
            self.worker
                .as_ref()
                .unwrap()
                .jobs
                .try_send(Job { draft, work })
                .map_err(|_| "Timing worker is busy or disconnected")?;
            self.active = Some(cancel);
            self.error = None;
            self.message = "Preparing timing; waiting for the renderer’s confirmed result…".into();
            Ok::<_, String>(())
        })();
        if let Err(error) = result {
            self.error = Some(error);
        }
    }
}

fn number(ui: &mut Ui, label: &str, value: &mut f64, min: f64, max: f64) -> bool {
    ui.push_id(label, |ui| {
        ui.label(label);
        let response = ui.add(egui::DragValue::new(value).speed(0.05).range(min..=max));
        response
            .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::DragValue, true, label));
        let mut changed = response.changed();
        if let Some(next) = accessibility::numeric(
            ui,
            &response,
            label,
            *value as f32,
            min as f32,
            max as f32,
            0.05,
            "",
        ) {
            *value = f64::from(next);
            changed = true;
        }
        changed
    })
    .inner
}
fn text(ui: &mut Ui, label: &str, value: &mut String) -> bool {
    ui.label(label);
    let response = ui.add(
        egui::TextEdit::multiline(value)
            .desired_rows(4)
            .desired_width(f32::INFINITY)
            .char_limit(256 * 1024),
    );
    help::annotate(ui, &response, HelpControl::TimingClick);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, label));
    help::annotate(ui, &response, HelpControl::TimingPoints);
    response.changed()
}
impl App {
    pub(super) fn open_timing(&mut self) {
        if self.timing.draft.is_none() {
            self.timing.draft = Draft::new(&self.snap);
        }
        self.timing.open = true;
    }
    pub(super) fn timing_ui(&mut self, ctx: &egui::Context) {
        if !self.timing.open {
            return;
        }
        keyboard::block_for_dialog(ctx);
        let mut editor = std::mem::take(&mut self.timing);
        let mut open = true;
        let mut close = false;
        egui::Window::new("Tempo and meter").open(&mut open).default_width(580.0).vscroll(true).max_height(self.theme.window_height(ctx)).show(ctx, |ui| {
            ui.label("Positions are quarter-note beats. Each list starts at 0. A ramp rises or falls to the next point; the last point must use step.");
            ui.add_enabled_ui(!editor.busy() && !self.project.committing(), |ui| {
                if let Some(draft) = &mut editor.draft {
                    draft.dirty |= text(ui, "Tempo points: beat BPM step/ramp", &mut draft.tempos);
                    draft.dirty |= text(ui, "Meter markers: beat numerator/denominator", &mut draft.meters);
                    ui.horizontal_wrapped(|ui| {
                        draft.dirty |= number(ui, "Pickup length", &mut draft.settings.pickup, 0.0, 1024.0);
                        let mut subdivision = f64::from(draft.settings.subdivision);
                        if number(ui, "Click subdivisions (1, 2 or 4)", &mut subdivision, 1.0, 4.0) { draft.settings.subdivision = subdivision.round() as u8; draft.dirty = true; }
                        let mut count = f64::from(draft.settings.count_in);
                        if number(ui, "Count-in bars", &mut count, 0.0, 4.0) { draft.settings.count_in = count.round() as u8; draft.dirty = true; }
                        let mut gain = f64::from(draft.settings.accent_gain);
                        if number(ui, "Accent gain", &mut gain, 0.0, 2.0) { draft.settings.accent_gain = gain as f32; draft.dirty = true; }
                        let mut gain = f64::from(draft.settings.beat_gain);
                        if number(ui, "Beat gain", &mut gain, 0.0, 2.0) { draft.settings.beat_gain = gain as f32; draft.dirty = true; }
                    });
                }
                if ui.button("Apply timing").help(ui, HelpControl::TimingApply).clicked() { editor.apply(&self.engine); }
            });
            ui.label("Count-in holds clips and recording at the current position. It uses the starting meter and tempo; DJ decks keep playing. Native project files retain exact ramps. MIDI export samples ramps at each MIDI tick.");
            if editor.busy() {
                if ui.button("Cancel timing operation").help(ui, HelpControl::TimingCancel).clicked() { editor.cancel(); }
                ctx.request_repaint_after(Duration::from_millis(20));
            }
            if !editor.message.is_empty() { ui.label(&editor.message); }
            if let Some(error) = &editor.error { ui.colored_label(self.theme.red, error); }
            if ui.button("Close timing editor").help(ui, HelpControl::TimingClose).clicked() { close = true; }
        });
        if !open
            || close
            || ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, Key::Escape))
        {
            if editor.blocks_close() {
                editor.confirm_discard = true;
            } else {
                editor.open = false;
                editor.draft = None;
            }
        }
        if editor.confirm_discard {
            egui::Window::new("Unapplied timing").collapsible(false).show(ctx, |ui| {
                ui.label("Keep editing or discard the unapplied draft. Completed edits remain in History.");
                if ui.button("Keep timing draft").help(ui, HelpControl::TimingClose).clicked() { editor.confirm_discard = false; }
                if ui.button("Discard timing draft").help(ui, HelpControl::TimingClose).clicked() {
                    editor.cancel(); editor.confirm_discard = false; editor.discard_when_settled = true;
                    if !editor.busy() { editor.draft = None; editor.open = false; }
                }
            });
        }
        self.timing = editor;
    }
}

#[cfg(test)]
mod tests;
