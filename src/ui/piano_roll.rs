//! A native, editable MIDI draft. The project worker inspects coherent content;
//! Apply is one guarded renderer transaction and one existing history entry.
use super::*;
use crate::engine::{
    midi_edit::{Ack, Document, NoteId, Outcome, Region, Request},
    MidiNote,
};
use std::collections::BTreeSet;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc,
};
mod canvas;
mod comparison;
mod control;
mod rhythm;
mod step;
mod tools;

pub(super) struct Editor {
    open: bool,
    loading: Option<Loading>,
    preparing: Option<Preparing>,
    draft: Option<Draft>,
    pending: Option<Pending>,
    message: String,
    error: Option<String>,
    confirm_discard: bool,
    audition: Option<u64>,
    stop_requested: bool,
    next_audition: u64,
    keyboard: step::Keyboard,
    comparison: comparison::Comparison,
}
struct Loading {
    receiver: mpsc::Receiver<Result<(Arc<Document>, f32), String>>,
    cancel: Arc<AtomicBool>,
}
impl Drop for Loading {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
    }
}
struct Pending {
    ack: Ack,
    next: Arc<Document>,
}
type PreparedEdit = (Request, Ack, Arc<Document>);
struct Preparing {
    receiver: mpsc::Receiver<Result<PreparedEdit, String>>,
    cancel: Arc<AtomicBool>,
}
impl Drop for Preparing {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
    }
}
struct Draft {
    baseline: Arc<Document>,
    name: String,
    region: Region,
    notes: Vec<MidiNote>,
    selected: BTreeSet<NoteId>,
    dirty: bool,
    cursor: Values,
    grid: usize,
    draw: bool,
    view_beat: f64,
    view_high: u8,
    beat_pixels: f32,
    row_pixels: f32,
    fold: usize,
    root: u8,
    drag: Option<canvas::Drag>,
    steps: Vec<step::Edit>,
    step_record: bool,
    step_grid: usize,
    step_chord: BTreeSet<NoteId>,
    rhythm: rhythm::Generator,
    controls: control::Controls,
    tools: tools::Tools,
}
#[derive(Clone, Copy)]
struct Values {
    pitch: u8,
    start: f32,
    length: f32,
    velocity: u8,
    muted: bool,
}
const GRIDS: [(&str, f64); 8] = [
    ("Free", 0.0),
    ("Quarter notes", 1.0),
    ("Eighth notes", 0.5),
    ("Sixteenth notes", 0.25),
    ("Thirty-second notes", 0.125),
    ("Quarter triplets", 2.0 / 3.0),
    ("Eighth triplets", 1.0 / 3.0),
    ("Sixteenth triplets", 1.0 / 6.0),
];
impl Default for Editor {
    fn default() -> Self {
        Self {
            open: false,
            loading: None,
            preparing: None,
            draft: None,
            pending: None,
            message: "Choose an empty or MIDI slot to edit its notes.".into(),
            error: None,
            confirm_discard: false,
            audition: None,
            stop_requested: false,
            next_audition: 1,
            keyboard: step::Keyboard::default(),
            comparison: comparison::Comparison::default(),
        }
    }
}
impl Draft {
    fn new(baseline: Arc<Document>) -> Self {
        let empty = baseline.kind == crate::engine::ClipKind::Empty;
        Self {
            name: if empty {
                "MIDI clip".into()
            } else {
                baseline.name.clone()
            },
            region: if empty {
                Region::full(16.0)
            } else {
                baseline.playback_region()
            },
            notes: baseline.notes.clone(),
            controls: control::Controls::new(baseline.lanes.as_deref()),
            baseline,
            selected: BTreeSet::new(),
            dirty: false,
            cursor: Values {
                pitch: 60,
                start: 0.0,
                length: 0.25,
                velocity: 100,
                muted: false,
            },
            grid: 3,
            draw: true,
            view_beat: 0.0,
            view_high: 72,
            beat_pixels: 40.0,
            row_pixels: 18.0,
            fold: 0,
            root: 0,
            drag: None,
            steps: Vec::new(),
            step_record: false,
            step_grid: 3,
            step_chord: BTreeSet::new(),
            rhythm: rhythm::Generator::default(),
            tools: tools::Tools::default(),
        }
    }
    fn snap(&self, value: f64) -> f32 {
        let grid = GRIDS[self.grid].1;
        (if grid > 0.0 {
            (value / grid).round() * grid
        } else {
            value
        })
        .clamp(0.0, 262_144.0) as f32
    }
    fn select(&mut self, id: NoteId, extend: bool) {
        if !extend {
            self.selected.clear();
        }
        if extend && self.selected.contains(&id) {
            self.selected.remove(&id);
        } else {
            self.selected.insert(id);
        }
        if let Some(note) = self.notes.iter().find(|n| n.id == id) {
            self.cursor = Values {
                pitch: note.pitch,
                start: note.start,
                length: note.len,
                velocity: note.vel,
                muted: note.muted,
            };
        }
    }
    fn add(&mut self) -> Result<(), String> {
        if self.notes.len() >= crate::engine::project::MAX_NOTES_PER_CLIP {
            return Err("This clip already contains the maximum 8192 notes".into());
        }
        let id = NoteId::new();
        if !id.valid() {
            return Err("A stable note identity could not be created".into());
        }
        let value = self.cursor;
        self.notes.push(MidiNote {
            channel: 0,
            release_vel: 64,
            source_timing: None,
            id,
            pitch: value.pitch,
            start: value.start,
            len: value.length,
            vel: value.velocity,
            muted: value.muted,
        });
        self.selected.clear();
        self.selected.insert(id);
        self.dirty = true;
        Ok(())
    }
    fn delete(&mut self) {
        let before = self.notes.len();
        self.notes.retain(|n| !self.selected.contains(&n.id));
        self.selected.clear();
        self.dirty |= before != self.notes.len();
    }
    fn transform(
        &mut self,
        beats: f32,
        semitones: i16,
        length: f32,
        mute: bool,
    ) -> Result<(), String> {
        for n in self.notes.iter().filter(|n| self.selected.contains(&n.id)) {
            if !(0..=127).contains(&(n.pitch as i16 + semitones))
                || !(0.0..=262_144.0).contains(&(n.start + beats))
                || !(0.0..=262_144.0).contains(&(n.len + length))
            {
                return Err("Selected notes would leave the supported pitch or beat range".into());
            }
        }
        for n in self
            .notes
            .iter_mut()
            .filter(|n| self.selected.contains(&n.id))
        {
            n.start += beats;
            n.pitch = (n.pitch as i16 + semitones) as u8;
            n.len += length;
            n.reconcile_timing();
            if mute {
                n.muted = !n.muted;
            }
            self.dirty = true;
        }
        Ok(())
    }
    fn duplicate(&mut self) -> Result<(), String> {
        let selected: Vec<_> = self
            .notes
            .iter()
            .filter(|n| self.selected.contains(&n.id))
            .cloned()
            .collect();
        if self.notes.len() + selected.len() > crate::engine::project::MAX_NOTES_PER_CLIP {
            return Err("Duplicating would exceed 8192 notes".into());
        }
        let offset = selected.iter().map(|n| n.start + n.len).fold(0.0, f32::max)
            - selected
                .iter()
                .map(|n| n.start)
                .fold(f32::INFINITY, f32::min);
        if selected.iter().any(|n| n.start + offset > 262_144.0) {
            return Err("Duplicated notes would exceed the supported beat range".into());
        }
        let mut copies = Vec::with_capacity(selected.len());
        for mut n in selected {
            n.id = NoteId::new();
            if !n.id.valid() {
                return Err("A note identity could not be created".into());
            }
            n.start += offset;
            n.reconcile_timing();
            copies.push(n);
        }
        self.selected = copies.iter().map(|n| n.id).collect();
        self.dirty |= !copies.is_empty();
        self.notes.extend(copies);
        Ok(())
    }
    fn set_values(&mut self) {
        for n in self
            .notes
            .iter_mut()
            .filter(|n| self.selected.contains(&n.id))
        {
            n.pitch = self.cursor.pitch;
            n.start = self.cursor.start;
            n.len = self.cursor.length;
            n.reconcile_timing();
            n.vel = self.cursor.velocity;
            n.muted = self.cursor.muted;
            self.dirty = true;
        }
    }
    fn selected_label(&self) -> String {
        let Some(note) = self.notes.iter().find(|n| self.selected.contains(&n.id)) else {
            return format!(
                "No notes selected. {} notes in this draft.",
                self.notes.len()
            );
        };
        format!(
            "{} selected · {} · start {:.9} beats · length {:.9} beats · velocity {} · channel {} · release {} · {}{}",
            self.selected.len(),
            pitch_name(note.pitch),
            note.source_start(),
            note.source_duration(),
            note.vel,
            note.channel + 1,
            note.release_vel,
            if note.muted { "muted" } else { "audible" },
            note.source_timing.map_or(String::new(), |t| format!(" · source PPQN {} tick {} length {}", t.ppqn, t.start, t.duration))
        )
    }
}
fn pitch_name(pitch: u8) -> String {
    format!(
        "{}{} (MIDI {})",
        ["C", "C♯", "D", "D♯", "E", "F", "F♯", "G", "G♯", "A", "A♯", "B"][pitch as usize % 12],
        pitch as i16 / 12 - 1,
        pitch
    )
}
impl Editor {
    fn busy(&self) -> bool {
        self.comparison.busy()
            || self.loading.is_some()
            || self.preparing.is_some()
            || self.pending.is_some()
            || self.draft.as_ref().is_some_and(|draft| draft.tools.busy())
    }
    fn stop(&mut self, engine: &Engine) {
        if let Some(id) = self.audition {
            match engine.send(Command::MidiAudition {
                id,
                track: 0,
                note: 0,
                vel: 0,
                on: false,
            }) {
                Ok(_) => {
                    self.audition = None;
                    self.stop_requested = false;
                }
                Err(crate::engine::SubmissionError::Disconnected) => {
                    self.audition = None;
                    self.stop_requested = false;
                    self.error =
                        Some("Renderer disconnected; audition stop outcome is unknown.".into());
                }
                Err(error) => {
                    self.stop_requested = true;
                    self.error = Some(format!("Audition release not accepted: {error}; retrying."));
                }
            }
        }
        self.keyboard.chord.clear();
        self.stop_keyboard(engine);
    }
    pub(super) fn stop_for_close(&mut self, engine: &Engine) {
        self.stop(engine);
        if self.blocks_close() {
            self.open = true;
            self.confirm_discard = true;
        }
    }
    pub(super) fn blocks_close(&self) -> bool {
        self.busy()
            || self.stop_requested
            || self.comparison.dirty()
            || self.draft.as_ref().is_some_and(|d| d.dirty)
    }
    fn discard(&mut self, engine: &Engine) {
        self.stop(engine);
        self.loading = None;
        self.preparing = None;
        self.draft = None;
        self.comparison.discard();
        self.confirm_discard = false;
        if let Some(pending) = &self.pending {
            pending.ack.cancel();
        }
        self.open = self.pending.is_some() || self.comparison.busy();
        self.message =
            "Unapplied draft discarded. Completed renderer edits remain in History.".into();
    }
    fn load(&mut self, engine: &Engine, track: u8, scene: u16) {
        if self.busy() {
            return;
        }
        self.stop(engine);
        if self.stop_requested {
            return;
        }
        let project = engine.project.clone();
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = cancel.clone();
        let (sender, receiver) = mpsc::sync_channel(1);
        match std::thread::Builder::new()
            .name("midi-editor-inspect".into())
            .spawn(move || {
                let result = project
                    .capture(&worker_cancel)
                    .map_err(|e| e.to_string())
                    .and_then(|captured| {
                        let tuning = captured
                            .state
                            .tracks
                            .get(track as usize)
                            .ok_or("Unknown focused track")?
                            .synth
                            .tuning_hz;
                        Document::capture(captured, track, scene).map(|document| (document, tuning))
                    });
                let _ = sender.send(result);
            }) {
            Ok(_) => {
                self.loading = Some(Loading { receiver, cancel });
                self.open = true;
                self.error = None;
                self.message = "Inspecting the selected clip on the project worker…".into();
            }
            Err(error) => self.error = Some(format!("MIDI inspection worker unavailable: {error}")),
        }
    }
    fn poll(&mut self, engine: &Engine) {
        match self.comparison.poll(engine, self.draft.as_mut()) {
            Ok(Some(message)) => self.message = message.into(),
            Err(error) => self.error = Some(error),
            _ => {}
        }
        if let Some(draft) = &mut self.draft {
            let mut tools = std::mem::take(&mut draft.tools);
            if let Err(error) = tools.poll(draft) {
                self.error = Some(error);
            }
            draft.tools = tools;
        }
        if self.stop_requested {
            self.stop(engine);
        }
        if let Some(loading) = &self.loading {
            match loading.receiver.try_recv() {
                Ok(Ok((document, tuning))) => {
                    self.comparison = comparison::Comparison::default();
                    self.comparison.offset = document.playback_region().start;
                    self.comparison.tuning_hz = tuning;
                    self.draft = Some(Draft::new(document));
                    self.loading = None;
                    self.message = "Editing a draft. Apply MIDI edit commits it; Cancel / close preserves the current clip.".into();
                }
                Ok(Err(error)) => {
                    self.error = Some(error);
                    self.loading = None;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.error = Some(
                        "MIDI inspection worker disconnected before returning a document".into(),
                    );
                    self.loading = None;
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        if let Some(pending) = &self.pending {
            match pending.ack.state() {
                Outcome::Applied => {
                    if let Some(draft) = &mut self.draft {
                        draft.baseline = pending.next.clone();
                        draft.dirty = false;
                        draft.steps.clear();
                        draft.step_chord.clear();
                        draft.rhythm.committed();
                        draft.tools.committed();
                        draft.controls.dirty = false;
                    }
                    self.pending = None;
                    self.message =
                        "MIDI edit applied. Undo / Redo uses the session History.".into();
                }
                Outcome::Rejected => {
                    self.pending = None;
                    self.error = Some("Renderer rejected this edit. The target changed, recording is active, protection is enabled, or History is unavailable. Your draft is retained; refresh only after reviewing current work.".into());
                }
                Outcome::Cancelled => {
                    self.pending = None;
                    self.message = "Apply cancelled before renderer ownership.".into();
                }
                Outcome::Pending if !engine.cmd.is_connected() => {
                    self.pending = None;
                    self.error = Some("Renderer disconnected before acknowledging this edit. Outcome is unknown; draft retained.".into());
                }
                Outcome::Pending => {}
            }
        }
    }
    fn submit_prepared(&mut self, engine: &Engine) {
        if let Some(preparing) = &self.preparing {
            match preparing.receiver.try_recv() {
                Ok(Ok((request, ack, next))) => {
                    self.preparing = None;
                    match engine.send(Command::MidiEdit(request)) {
                        Ok(_) => {
                            self.pending = Some(Pending { ack, next });
                            self.message =
                                "Apply queued; waiting for the renderer’s actual outcome.".into();
                        }
                        Err(error) => {
                            self.error = Some(format!(
                                "MIDI edit was not accepted: {error}. Draft retained."
                            ))
                        }
                    }
                }
                Ok(Err(error)) => {
                    self.preparing = None;
                    self.error = Some(error);
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.preparing = None;
                    self.error =
                        Some("MIDI preparation worker disconnected; draft retained.".into());
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
    }
    fn apply(&mut self, engine: &Engine) {
        if self.busy() {
            return;
        }
        self.stop(engine);
        if self.stop_requested {
            return;
        }
        let Some(draft) = &self.draft else {
            return;
        };
        if !self.comparison.clips.is_empty() {
            match self.comparison.apply(engine, draft) {
                Ok(()) => {
                    self.error = None;
                    self.message =
                        "Preparing all changed captured MIDI clips for one Apply…".into();
                }
                Err(error) => self.error = Some(error),
            }
            return;
        }
        if draft.controls.dirty {
            let baseline = draft.baseline.clone();
            let name = draft.name.clone();
            let notes = draft.notes.clone();
            let region = draft.region;
            let controls = draft.controls.clone();
            let project = engine.project.clone();
            let cancel = Arc::new(AtomicBool::new(false));
            let worker_cancel = cancel.clone();
            let (sender, receiver) = mpsc::sync_channel(1);
            match std::thread::Builder::new()
                .name("midi-controller-prepare".into())
                .spawn(move || {
                    let result = controls
                        .prepared(&notes, region.end, &worker_cancel)
                        .and_then(|lanes| {
                            if worker_cancel.load(Ordering::Acquire) {
                                return Err("MIDI edit cancelled during preparation".into());
                            }
                            let captured =
                                project.capture(&worker_cancel).map_err(|e| e.to_string())?;
                            let (request, ack, next) =
                                Request::with_lanes(baseline, name, region, notes, lanes)?;
                            Ok((request.guard_metadata(captured, &worker_cancel)?, ack, next))
                        });
                    let _ = sender.send(result);
                }) {
                Ok(_) => {
                    self.preparing = Some(Preparing { receiver, cancel });
                    self.error = None;
                    self.message = "Preparing controller lanes for Apply…".into();
                }
                Err(error) => {
                    self.error = Some(format!("MIDI preparation worker unavailable: {error}"))
                }
            }
            return;
        }
        match Request::new(
            draft.baseline.clone(),
            draft.name.clone(),
            draft.region,
            draft.notes.clone(),
        ) {
            Ok((request, ack, next)) => match engine.send(Command::MidiEdit(request)) {
                Ok(_) => {
                    self.pending = Some(Pending { ack, next });
                    self.error = None;
                    self.message =
                        "Apply queued; waiting for the renderer’s actual outcome.".into();
                }
                Err(error) => {
                    self.error = Some(format!(
                        "MIDI edit was not accepted: {error}. Draft retained."
                    ))
                }
            },
            Err(error) => self.error = Some(error),
        }
    }
    fn audition(&mut self, engine: &Engine) {
        self.stop(engine);
        if self.stop_requested {
            return;
        }
        let Some(draft) = &self.draft else {
            return;
        };
        let Some(next) = self.next_audition.checked_add(1) else {
            self.error = Some("Audition identity exhausted; reopen the application".into());
            return;
        };
        let id = self.next_audition;
        self.next_audition = next;
        match engine.send(Command::MidiAudition {
            id,
            track: draft.baseline.track,
            note: draft.cursor.pitch,
            vel: draft.cursor.velocity,
            on: true,
        }) {
            Ok(_) => self.audition = Some(id),
            Err(error) => self.error = Some(format!("Note audition was not accepted: {error}")),
        }
    }
}
impl App {
    pub(super) fn open_piano_roll(&mut self) {
        if self.piano_roll.blocks_close() {
            self.piano_roll.open = true;
            self.piano_roll.message =
                "Finish or discard this captured draft before choosing another clip.".into();
            return;
        }
        self.piano_roll.load(
            &self.engine,
            self.snap.selected_track as u8,
            self.snap.selected_scene as u16,
        );
    }
    pub(super) fn poll_piano_roll(&mut self) {
        self.piano_roll.poll(&self.engine);
        self.piano_roll.submit_prepared(&self.engine);
    }
    pub(super) fn piano_roll_ui(&mut self, ctx: &egui::Context) {
        let mut editor = std::mem::take(&mut self.piano_roll);
        if !ctx.input(|i| i.focused) {
            editor.stop(&self.engine);
        }
        if !editor.open {
            self.piano_roll = editor;
            return;
        }
        keyboard::block_for_dialog(ctx);
        let escape = ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Escape));
        let musical_events = ctx.input(|i| i.events.clone());
        let available = ctx.screen_rect().shrink(8.0);
        let mut open = true;
        let mut close = escape;
        let mut apply = false;
        let mut audition = false;
        let mut stop = false;
        let mut refresh = false;
        let mut keyboard_focus = false;
        let mut step_action = None;
        let mut comparison_action = None;
        let keyboard_was_enabled = editor.keyboard.enabled;
        let mut keyboard_enabled = editor.keyboard.enabled;
        let mut octave = editor.keyboard.octave;
        let mut velocity = editor.keyboard.velocity;
        let busy = editor.busy();
        let shown = egui::Window::new(tr!("MIDI piano roll")).id(egui::Id::new("midi-piano-roll"))
            .open(&mut open).collapsible(false).resizable(true)
            .default_pos(available.left_top() + Vec2::new(16.0, 32.0))
            .default_width(980.0_f32.min(available.width())).min_width(260.0_f32.min(available.width()))
            .max_width(available.width()).max_height(available.height()).constrain_to(available)
            .show(ctx, |ui| {
                accessibility::scope(ui, "MIDI piano roll", |ui| {
                    ui.label(&editor.message);
                    if let Some(error) = &editor.error { ui.colored_label(self.theme.red, error); }
                    if busy { ui.spinner(); ctx.request_repaint_after(std::time::Duration::from_millis(20)); }
                    if let Some(draft) = &mut editor.draft {
                        ui.label({ let __omatainer_args = (&(draft.baseline.track + 1),&(draft.baseline.scene + 1),&(draft.notes.len()),&(if draft.dirty { "unapplied changes" } else { "unmodified draft" }),); crate::localization::format("Captured track {} · scene {} · {} notes · {}", &[format!("{}", __omatainer_args.0), format!("{}", __omatainer_args.1), format!("{}", __omatainer_args.2), format!("{}", __omatainer_args.3)]) });
                        if let Some(lanes) = &draft.baseline.lanes {
                            ui.label({ let __omatainer_args = (&(lanes.ppqn),&(lanes.end_tick),&(lanes.messages.len()),&(lanes.meta.len()),); crate::localization::format("Imported source PPQN {} · end tick {} · {} channel messages · {} standard metadata events", &[format!("{}", __omatainer_args.0), format!("{}", __omatainer_args.1), format!("{}", __omatainer_args.2), format!("{}", __omatainer_args.3)]) });
                        }
                        let scroll = egui::ScrollArea::vertical().id_salt("piano-roll-body")
                            .max_height((available.height() - 160.0).max(100.0)).show(ui, |ui| {
                            ui.add_enabled_ui(!busy && !self.project.committing() && editor.comparison.editing(draft), |ui| {
                                ui.horizontal_wrapped(|ui| {
                                    ui.label(tr!("Clip name")); let name = ui.add(egui::TextEdit::singleline(&mut draft.name).char_limit(4096));
                                    name.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, "MIDI clip name"));
                                    draft.dirty |= name.changed();
                                });
                                ui.horizontal_wrapped(|ui| {
                                    draft.dirty |= number(ui, "Clip start", &mut draft.region.start, 0.0, 262_144.0);
                                    draft.dirty |= number(ui, "Clip end", &mut draft.region.end, 0.0, 262_144.0);
                                    draft.dirty |= number(ui, "Loop start", &mut draft.region.loop_start, 0.0, 262_144.0);
                                    draft.dirty |= number(ui, "Loop end", &mut draft.region.loop_end, 0.0, 262_144.0);
                                    let looping = ui.checkbox(&mut draft.region.loop_enabled, tr!("Loop enabled"));
                                    accessibility::button(ui, &looping, "Loop enabled", Some(draft.region.loop_enabled));
                                    help::annotate(ui, &looping, HelpControl::MidiRegion);
                                    draft.dirty |= looping.changed();
                                });
                                if !draft.region.allows(&draft.notes) { ui.colored_label(self.theme.red, tr!("Set start ≤ loop start < loop end ≤ end. Each range must span at least 1/1024 beat; dense loops need a longer period.")); }
                                ui.horizontal_wrapped(|ui| {
                                    let grid = egui::ComboBox::from_id_salt("midi-grid").selected_text(GRIDS[draft.grid].0)
                                        .show_ui(ui, |ui| { for (i, (name, _)) in GRIDS.iter().enumerate() { ui.selectable_value(&mut draft.grid, i, *name); } });
                                    help::annotate(ui, &grid.response, HelpControl::MidiGrid);
                                    let draw = ui.checkbox(&mut draft.draw, tr!("Draw notes"));
                                    accessibility::button(ui, &draw, "Draw notes", Some(draft.draw));
                                    help::annotate(ui, &draw, HelpControl::MidiNotes);
                                    let fold = egui::ComboBox::from_id_salt("midi-fold").selected_text(["All pitches", "Used pitches", "Major scale", "Minor scale"][draft.fold])
                                        .show_ui(ui, |ui| { for (i, name) in ["All pitches", "Used pitches", "Major scale", "Minor scale"].iter().enumerate() { ui.selectable_value(&mut draft.fold, i, *name); } });
                                    help::annotate(ui, &fold.response, HelpControl::MidiView);
                                    let mut root = draft.root as f64; if number(ui, "Scale root", &mut root, 0.0, 11.0) { draft.root = root.round() as u8; }
                                    number(ui, "Time scroll", &mut draft.view_beat, if editor.comparison.clips.is_empty(){0.0}else{-524_288.0}, 524_288.0);
                                    let mut pitch = draft.view_high as f64; if number(ui, "Top pitch", &mut pitch, 0.0, 127.0) { draft.view_high = pitch.round() as u8; }
                                    let mut zoom = draft.beat_pixels as f64; if number(ui, "Time zoom", &mut zoom, 8.0, 200.0) { draft.beat_pixels = zoom as f32; }
                                    let mut height = draft.row_pixels as f64; if number(ui, "Pitch zoom", &mut height, 12.0, 32.0) { draft.row_pixels = height as f32; }
                                });
                                ui.horizontal_wrapped(|ui| {
                                    let record = ui.checkbox(&mut draft.step_record, "Record steps on key release");
                                    accessibility::button(ui, &record, "Record steps on key release", Some(draft.step_record));
                                    help::annotate(ui, &record, HelpControl::MidiStep);
                                    let duration = egui::ComboBox::from_id_salt("midi-step-duration").selected_text(format!("Step: {}", GRIDS[draft.step_grid].0))
                                        .show_ui(ui, |ui| { for (i, (name, _)) in GRIDS.iter().enumerate().skip(1) { ui.selectable_value(&mut draft.step_grid, i, *name); } });
                                    help::annotate(ui, &duration.response, HelpControl::MidiStep);
                                    ui.label(format!("Step cursor: {:.6} beats", draft.cursor.start));
                                    for (label, action) in [("Insert cursor pitch", step::Action::Pitch), ("Advance / insert held chord", step::Action::Advance),
                                        ("Rest step", step::Action::Rest), ("Tie previous step", step::Action::Tie), ("Delete last step", step::Action::Delete)] {
                                        if button(ui, label).clicked() { step_action = Some(action); }
                                    }
                                });
                                ui.horizontal_wrapped(|ui| {
                                    let enabled = ui.checkbox(&mut keyboard_enabled, "Computer musical keyboard");
                                    accessibility::button(ui, &enabled, "Computer musical keyboard", Some(keyboard_enabled));
                                    help::annotate(ui, &enabled, HelpControl::MidiStep);
                                    let mut value = octave as f64; if number(ui, "Keyboard octave", &mut value, -1.0, 9.0) { octave = value.round() as i8; }
                                    let mut value = velocity as f64; if number(ui, "Keyboard velocity", &mut value, 1.0, 127.0) { velocity = value.round() as u8; }
                                    let play = ui.add_enabled(keyboard_enabled, egui::Button::new("Focus musical keyboard"));
                                    accessibility::button(ui, &play, "Focus musical keyboard", None);
                                    help::annotate(ui, &play, HelpControl::MidiStep);
                                    if play.clicked() { play.request_focus(); }
                                    keyboard_focus = play.has_focus();
                                });
                                ui.label("Focus musical keyboard: A W S E D F T G Y H U J K play C through C. Z/X change octave; C/V change velocity. Space inserts held notes or a rest; Shift+Space ties; Backspace removes the last step. With Record steps enabled, releasing a chord inserts it and advances. Tab away releases all notes.");
                                match rhythm::show(ui, draft) { Ok(true) => editor.error = None, Err(error) => editor.error = Some(error), _ => {} }
                                match control::show(ui, draft, &self.theme) { Ok(true) => editor.error = None, Err(error) => editor.error = Some(error), _ => {} }
                                let layers=editor.comparison.layers();
                                if let Err(error) = canvas::show_compared(ui, &self.theme, draft, self.snap.timing.as_deref(),editor.comparison.offset,&layers) { editor.error = Some(error); }
                                let selected = ui.label(draft.selected_label());
                                accessibility::status(ui, &selected, &draft.selected_label());
                                ui.label(tr!("Focus the roll: arrows move / transpose; Shift+Left/Right resize; Ctrl+A selects all; Ctrl+D duplicates; M mutes; Delete removes. Draw on empty space; drag note bodies to move and right edges to resize. All values use quarter-note beats."));
                                ui.horizontal_wrapped(|ui| {
                                    let mut pitch = draft.cursor.pitch as f64; if number(ui, "Note pitch", &mut pitch, 0.0, 127.0) { draft.cursor.pitch = pitch.round() as u8; }
                                    let mut start = draft.cursor.start as f64; if number(ui, "Note start", &mut start, 0.0, 262_144.0) { draft.cursor.start = start as f32; }
                                    let mut length = draft.cursor.length as f64; if number(ui, "Note length", &mut length, 0.0, 262_144.0) { draft.cursor.length = length as f32; }
                                    let mut velocity = draft.cursor.velocity as f64; if number(ui, "Note velocity", &mut velocity, 1.0, 127.0) { draft.cursor.velocity = velocity.round() as u8; }
                                    let muted = ui.checkbox(&mut draft.cursor.muted, tr!("Note muted"));
                                    accessibility::button(ui, &muted, "Note muted", Some(draft.cursor.muted));
                                    help::annotate(ui, &muted, HelpControl::MidiValues);
                                });
                                ui.horizontal_wrapped(|ui| {
                                    if button(ui, "Add note").clicked() { if let Err(e) = draft.add() { editor.error = Some(e); } }
                                    if button(ui, "Set selected note values").clicked() { draft.set_values(); }
                                    if button(ui, "Select all notes").clicked() { draft.selected = draft.notes.iter().map(|n| n.id).collect(); }
                                    if button(ui, "Clear selection").clicked() { draft.selected.clear(); }
                                    if button(ui, "Duplicate notes").clicked() { if let Err(e) = draft.duplicate() { editor.error = Some(e); } }
                                    if button(ui, "Mute / unmute notes").clicked() { let _ = draft.transform(0.0, 0, 0.0, true); }
                                    if button(ui, "Delete notes").clicked() { draft.delete(); }
                                });
                                ui.horizontal_wrapped(|ui| {
                                    let step = if GRIDS[draft.grid].1 > 0.0 { GRIDS[draft.grid].1 as f32 } else { 1.0 / 64.0 };
                                    for (label, beat, pitch, len) in [("Move earlier", -step, 0, 0.0), ("Move later", step, 0, 0.0),
                                        ("Transpose down", 0.0, -1, 0.0), ("Transpose up", 0.0, 1, 0.0),
                                        ("Shorten notes", 0.0, 0, -step), ("Lengthen notes", 0.0, 0, step)] {
                                        if button(ui, label).clicked() { if let Err(e) = draft.transform(beat, pitch, len, false) { editor.error = Some(e); } }
                                    }
                                });
                                // Every note is available through a virtualized native list, even
                                // if it is outside the current painted time / pitch viewport.
                                let rows = egui::ScrollArea::vertical().id_salt("midi-note-list").max_height(130.0)
                                    .show_rows(ui, 22.0, draft.notes.len(), |ui, range| {
                                        for index in range { let note = &draft.notes[index]; let id = note.id;
                                            let label = format!("Note {}: {} · {:.6} beats · length {:.6} · velocity {} · {}", index + 1,
                                                pitch_name(note.pitch), note.start, note.len, note.vel, if note.muted { "muted" } else { "audible" });
                                            let response = ui.push_id(id, |ui| ui.selectable_label(draft.selected.contains(&id), &label)).inner;
                                            accessibility::button(ui, &response, &label, Some(draft.selected.contains(&id)));
                                            if response.clicked() { draft.select(id, ui.input(|i| i.modifiers.shift)); }
                                        }
                                    });
                                accessibility::scrollbars(ui, "MIDI note list", &rows);
                            });
                            ui.add_enabled_ui(!busy && !self.project.committing() && !editor.comparison.group.previewed(), |ui| {
                                match tools::show(ui,draft,&self.theme){Ok(true)=>{editor.error=None;stop=true;},Err(error)=>editor.error=Some(error),_=>{}}
                            });
                            comparison_action = comparison::view::show(ui,&mut editor.comparison,draft,
                                (self.snap.selected_track as u8,self.snap.selected_scene as u16),self.snap.session.as_ref().map_or(1,|s|s.tracks.len()),self.snap.session.as_ref().map_or(1,|s|s.scenes.len()));
                        });
                        accessibility::scrollbars(ui, "MIDI editor controls", &scroll);
                    }
                    ui.horizontal_wrapped(|ui| {
                        let response = ui.add_enabled(!busy && editor.draft.is_some() && !self.project.committing(), egui::Button::new(tr!("Apply MIDI edit")));
                        help::annotate(ui, &response, HelpControl::MidiApply);
                        accessibility::button(ui, &response, "Apply MIDI edit", None); apply = response.clicked();
                        let response = ui.add_enabled(!busy && editor.draft.is_some(), egui::Button::new(tr!("Audition note")));
                        help::annotate(ui, &response, HelpControl::MidiAudition);
                        accessibility::button(ui, &response, "Audition note", None); audition = response.clicked();
                        stop = button(ui, "Stop note audition").clicked();
                        let response = ui.add_enabled(!busy && !editor.comparison.dirty() && !editor.draft.as_ref().is_some_and(|d| d.dirty), egui::Button::new(tr!("Refresh current clip")));
                        help::annotate(ui, &response, HelpControl::PianoRoll);
                        accessibility::button(ui, &response, "Refresh current clip", None); refresh = response.clicked();
                        close |= button(ui, "Cancel / close MIDI editor").clicked();
                    });
                    if editor.confirm_discard {
                        ui.label(tr!("This editor has unapplied or pending work. Discard the draft and cancel Apply before closing? An edit already owned by the renderer must finish and remains in History."));
                        if button(ui, "Discard MIDI draft").clicked() { editor.discard(&self.engine); }
                        if button(ui, "Keep editing MIDI").clicked() { editor.confirm_discard = false; }
                    }
                });
            });
        if let Some(shown) = shown {
            let outside = ctx.input(|i| {
                i.pointer.any_pressed()
                    && i.pointer
                        .interact_pos()
                        .is_some_and(|p| !shown.response.rect.contains(p))
            });
            let other_focus = ctx
                .memory(|m| m.focused())
                .and_then(|id| ctx.read_response(id))
                .is_some_and(|r| r.layer_id != shown.response.layer_id);
            if outside || other_focus {
                editor.stop(&self.engine);
            }
        }
        if keyboard_was_enabled != keyboard_enabled
            || editor.keyboard.octave != octave
            || editor.keyboard.velocity != velocity
        {
            editor.stop(&self.engine);
        }
        editor.keyboard.enabled = keyboard_enabled;
        editor.keyboard.octave = octave;
        editor.keyboard.velocity = velocity;
        if let Some(action) = step_action {
            editor.step_action(action);
        }
        editor.keyboard_input(
            &self.engine,
            ctx,
            keyboard_focus
                && !busy
                && !self.project.committing()
                && editor
                    .draft
                    .as_ref()
                    .is_none_or(|draft| editor.comparison.editing(draft)),
            musical_events,
        );
        if let Some(action) = comparison_action {
            editor.stop(&self.engine);
            if !editor.stop_requested {
                if let Some(draft) = &mut editor.draft {
                    match editor.comparison.action(&self.engine, draft, action) {
                        Ok(()) => editor.error = None,
                        Err(error) => editor.error = Some(error),
                    }
                }
            }
        }
        if apply {
            editor.apply(&self.engine);
        }
        if audition {
            editor.audition(&self.engine);
        }
        if stop {
            editor.stop(&self.engine);
        }
        if refresh {
            if let Some(draft) = &editor.draft {
                let (track, scene) = (draft.baseline.track, draft.baseline.scene);
                editor.load(&self.engine, track, scene);
            }
        }
        if close || !open {
            editor.stop(&self.engine);
            if editor.blocks_close() {
                editor.confirm_discard = true;
            } else {
                editor.open = false;
                editor.draft = None;
            }
        }
        self.piano_roll = editor;
    }
}
fn button(ui: &mut Ui, label: &str) -> egui::Response {
    let response = ui.button(label);
    accessibility::button(ui, &response, label, None);
    help::annotate(
        ui,
        &response,
        if label.contains("audition") {
            HelpControl::MidiAudition
        } else if label.contains("step")
            || label.contains("held chord")
            || label.contains("cursor pitch")
        {
            HelpControl::MidiStep
        } else if label.contains("rhythm") {
            HelpControl::MidiRhythm
        } else if label.contains("close") || label.contains("Discard") || label.contains("Keep") {
            HelpControl::MidiCancel
        } else {
            HelpControl::MidiNotes
        },
    );
    response
}
fn number(ui: &mut Ui, label: &str, value: &mut f64, min: f64, max: f64) -> bool {
    ui.push_id(label, |ui| {
        ui.label(label);
        let integer = matches!(
            label,
            "Note pitch"
                | "Note velocity"
                | "Top pitch"
                | "Scale root"
                | "Keyboard octave"
                | "Keyboard velocity"
                | "Rhythm steps"
                | "Rhythm pulses"
                | "Rhythm rotation"
                | "Accent every"
                | "Rhythm pitch"
                | "Rhythm velocity"
                | "Rhythm accent"
        );
        let step = if integer { 1.0 } else { 0.01 };
        let response = ui.add(
            egui::DragValue::new(value)
                .speed(step)
                .range(min..=max)
                .max_decimals(if integer { 0 } else { 6 }),
        );
        help::annotate(
            ui,
            &response,
            if label.starts_with("Transform")
                || label.starts_with("Velocity")
                || label.starts_with("Warp")
                || label.starts_with("Property")
                || label.starts_with("MPE")
                || label.starts_with("Stretch")
            {
                HelpControl::MidiTransform
            } else if label.starts_with("Clip") || label.starts_with("Loop") {
                HelpControl::MidiRegion
            } else if label.starts_with("Note") {
                HelpControl::MidiValues
            } else if label.starts_with("Keyboard") {
                HelpControl::MidiStep
            } else if label.starts_with("Rhythm") || label == "Accent every" {
                HelpControl::MidiRhythm
            } else {
                HelpControl::MidiView
            },
        );
        let mut changed = response.changed();
        if let Some(next) = accessibility::numeric(
            ui,
            &response,
            label,
            *value as f32,
            min as f32,
            max as f32,
            step as f32,
            "",
        ) {
            *value = next as f64;
            changed = true;
        }
        changed
    })
    .inner
}

#[cfg(test)]
pub(crate) mod tests;
