//! Native Standard MIDI File review, mapping, import and export.
use super::*;
use crate::engine::{
    midi_edit::{Ack, Outcome},
    midi_interchange::{self, ExportOptions, Mapping, Request, TempoChoice},
};
use std::{
    collections::BTreeSet,
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};
mod worker;
use worker::{Event, Job, Preview, Worker};

pub(super) struct Editor {
    open: bool,
    exporting: bool,
    path: String,
    worker: Option<Worker>,
    active: Option<Arc<AtomicBool>>,
    pending: Option<Ack>,
    preview: Option<Arc<Preview>>,
    mappings: Vec<Mapping>,
    split: bool,
    merge: bool,
    tempo: TempoChoice,
    reviewed: bool,
    rounding: bool,
    cells: BTreeSet<(u8, u8)>,
    ppqn: u16,
    single_track: bool,
    include_muted: bool,
    session_conductor: bool,
    inspect_track: usize,
    message: String,
    error: Option<String>,
}
impl Default for Editor {
    fn default() -> Self {
        Self {
            open: false,
            exporting: false,
            path: String::new(),
            worker: None,
            active: None,
            pending: None,
            preview: None,
            mappings: vec![],
            split: false,
            merge: false,
            tempo: TempoChoice::KeepSession,
            reviewed: false,
            rounding: false,
            cells: BTreeSet::new(),
            ppqn: 960,
            single_track: false,
            include_muted: true,
            session_conductor: true,
            inspect_track: 0,
            message: "Choose a MIDI file to inspect its tracks and source events.".into(),
            error: None,
        }
    }
}
impl Drop for Editor {
    fn drop(&mut self) {
        self.cancel();
    }
}
impl Editor {
    pub(super) fn busy(&self) -> bool {
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
    fn map(&mut self, track: usize, scene: usize) {
        let Some(preview) = &self.preview else {
            return;
        };
        let start = track * SCENES + scene;
        self.mappings = midi_interchange::sources(&preview.file, self.split)
            .into_iter()
            .enumerate()
            .map(|(i, source)| {
                let index = start + i;
                Mapping {
                    source,
                    destination: (index < TRACKS * SCENES)
                        .then_some(((index / SCENES) as u8, (index % SCENES) as u8)),
                }
            })
            .collect();
    }
    fn poll(&mut self, track: usize, scene: usize) {
        if let Some(worker) = &self.worker {
            let event = worker.events.try_recv();
            if matches!(event, Err(crossbeam_channel::TryRecvError::Disconnected)) {
                self.active = None;
                self.error =
                    Some("MIDI worker disconnected; reopen the application to retry.".into());
            }
            if let Ok(event) = event {
                let cancelled = self
                    .active
                    .take()
                    .is_some_and(|c| c.load(Ordering::Acquire));
                match event {
                    Event::Preview(preview) if !cancelled => {
                        self.ppqn = preview.file.ppqn;
                        self.preview = Some(preview);
                        self.inspect_track = 0;
                        self.reviewed = false;
                        self.error = None;
                        self.map(track, scene);
                        self.message = "Inspected file. Review destinations, source events and conductor choice, then import.".into();
                    }
                    Event::Preview(_) => {
                        self.message = "MIDI inspection cancelled.".into();
                    }
                    Event::Queued(ack) => {
                        if cancelled {
                            ack.cancel();
                        }
                        self.pending = Some(ack);
                        self.message =
                            "Waiting for the renderer to confirm the mapped MIDI import…".into();
                    }
                    Event::Written(path) => {
                        self.error = None;
                        self.message = format!("MIDI file exported: {}", path.display());
                    }
                    Event::Failed(error) => {
                        self.error = Some(error);
                    }
                }
            }
        }
        if let Some(ack) = &self.pending {
            let state = ack.state();
            if state != Outcome::Pending {
                self.pending = None;
                match state {
                    Outcome::Applied => {
                        self.error = None;
                        self.message = "MIDI import applied. All mapped clips and the chosen conductor form one History entry.".into();
                    }
                    Outcome::Rejected => {
                        self.error = Some("MIDI import was rejected because a target, recording, protection or history state changed. Current work was preserved; retry after reviewing the session.".into());
                    }
                    Outcome::Cancelled => {
                        self.error = None;
                        self.message = "MIDI import cancelled before renderer ownership; the session is unchanged.".into();
                    }
                    Outcome::Pending => unreachable!(),
                }
            }
        }
    }
    fn start(
        &mut self,
        engine: &Engine,
        job: impl FnOnce(crate::engine::performance::WorkPermit) -> Job,
    ) {
        if self.busy() {
            return;
        }
        let result = (|| {
            let work = engine
                .cmd
                .performance()
                .optional_work()
                .map_err(|e| e.to_string())?;
            let cancel = work.cancel();
            if self.worker.is_none() {
                self.worker = Some(Worker::start(engine.project.clone(), engine.cmd.clone())?);
            }
            self.worker
                .as_ref()
                .unwrap()
                .jobs
                .try_send(job(work))
                .map_err(|_| "MIDI worker is busy or disconnected".to_owned())?;
            self.active = Some(cancel);
            self.error = None;
            self.message = "Preparing MIDI file operation on the worker…".into();
            Ok::<_, String>(())
        })();
        if let Err(error) = result {
            self.error = Some(error);
        }
    }
}

fn button(ui: &mut Ui, title: &str, enabled: bool, control: HelpControl) -> egui::Response {
    let response = ui
        .add_enabled(enabled, egui::Button::new(title))
        .help(ui, control);
    accessibility::button(ui, &response, title, None);
    response
}
fn text(ui: &mut Ui, value: &mut String, label: &str) {
    ui.label(label);
    ui.add(egui::TextEdit::singleline(value).char_limit(4096))
        .help(ui, HelpControl::MidiFilePath)
        .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, label));
}
impl App {
    pub(super) fn open_midi_files(&mut self, exporting: bool) {
        if self.midi_files.busy() {
            self.midi_files.open = true;
            return;
        }
        self.midi_files.open = true;
        self.midi_files.exporting = exporting;
        self.midi_files.path.clear();
        self.midi_files.error = None;
        self.midi_files.cells.clear();
        self.midi_files.cells.insert((
            self.snap.selected_track as u8,
            self.snap.selected_scene as u8,
        ));
        self.midi_files.message = if exporting { "Select clips and choose an unused .mid path. Export uses source note coordinates and retains trailing file silence; clip loop/launch transforms are not flattened." }
            else { "Inspect a Standard MIDI File, then review track/channel mapping and tempo choices." }.into();
    }
    pub(super) fn poll_midi_files(&mut self) {
        self.midi_files
            .poll(self.snap.selected_track, self.snap.selected_scene);
    }
    pub(super) fn midi_files_ui(&mut self, ctx: &egui::Context) {
        if !self.midi_files.open {
            return;
        }
        let mut editor = std::mem::take(&mut self.midi_files);
        let mut visible = true;
        let busy = editor.busy();
        let allowed =
            !busy && !self.recovery_project_busy() && !self.engine.cmd.performance().protected();
        let mut inspect = false;
        let mut import = false;
        let mut export = false;
        egui::Window::new(if editor.exporting { "Export Standard MIDI File" } else { "Import Standard MIDI File" })
            .id(egui::Id::new("standard-midi-file")).open(&mut visible).default_width(800.0).vscroll(true).max_height((ctx.screen_rect().height() - 80.0).max(240.0)).show(ctx, |ui| {
                keyboard::block_for_dialog(ctx);
                ui.label("SMF 0 (one track) and SMF 1 (parallel tracks), PPQN 1–32767. Format 2, SMPTE, RMID and proprietary forms require conversion before import.");
                ui.label("Controller/program lanes are retained for file interchange. Native notes use the track's selected sound.");
                ui.add_enabled_ui(!busy, |ui| text(ui, &mut editor.path, "MIDI file path"));
                if editor.exporting {
                    ui.label("Each selected clip becomes a file track, all at source beat zero. Select only the clips you intend to combine.");
                    egui::Grid::new("smf-export-cells").show(ui, |ui| {
                        for t in 0..TRACKS {
                            for s in 0..SCENES {
                                let midi = self.snap.tracks.get(t).and_then(|t| t.clips.get(s)).is_some_and(|c| c.kind == 1);
                                let mut checked = editor.cells.contains(&(t as u8, s as u8));
                                let response = ui.add_enabled(!busy && midi, egui::Checkbox::new(&mut checked, format!("T{} S{}", t + 1, s + 1))).help(ui, HelpControl::MidiFileMapping);
                                response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Checkbox, !busy && midi, format!("Track {} scene {}", t + 1, s + 1)));
                                if response.changed() { if checked { editor.cells.insert((t as u8, s as u8)); } else { editor.cells.remove(&(t as u8, s as u8)); } }
                            }
                            ui.end_row();
                        }
                    });
                    ui.add_enabled_ui(!busy, |ui| {
                        ui.horizontal(|ui| { ui.label("Export PPQN"); number(ui, "Export PPQN", &mut editor.ppqn, 1, 32767).help(ui, HelpControl::MidiFilePrecision); });
                        ui.checkbox(&mut editor.single_track, "Merge to SMF 0").help(ui, HelpControl::MidiFileExport);
                        ui.checkbox(&mut editor.include_muted, "Include muted notes").help(ui, HelpControl::MidiFileExport);
                        ui.checkbox(&mut editor.session_conductor, "Export current session tempo and meter").help(ui, HelpControl::MidiFileTempo);
                        if !editor.session_conductor { ui.label("Original imported conductor events remain attached to their source clips. Export their conductor clip as well to retain a separate SMF 1 conductor track."); }
                        ui.checkbox(&mut editor.rounding, "Allow nearest-tick rounding").help(ui, HelpControl::MidiFilePrecision);
                    });
                    if button(ui, "Export new MIDI file", allowed && !editor.path.trim().is_empty() && !editor.cells.is_empty(), HelpControl::MidiFileExport).clicked() { export = true; }
                } else {
                    if button(ui, "Inspect MIDI file", allowed && !editor.path.trim().is_empty(), HelpControl::MidiFileImport).clicked() { inspect = true; }
                    if let Some(preview) = editor.preview.clone() {
                        ui.label(format!("{} · PPQN {} · {} tracks · {} notes · {} channel events", preview.path.display(), preview.file.ppqn,
                            preview.file.tracks.len(), preview.file.tracks.iter().map(|t| t.notes.len()).sum::<usize>(), preview.file.tracks.iter().map(|t| t.messages.len()).sum::<usize>()));
                        ui.add_enabled_ui(!busy, |ui| {
                            if ui.checkbox(&mut editor.split, "Split file tracks by MIDI channel").help(ui, HelpControl::MidiFileMapping).changed() { editor.map(self.snap.selected_track, self.snap.selected_scene); }
                            ui.checkbox(&mut editor.merge, "Merge with existing mapped MIDI clips").help(ui, HelpControl::MidiFileMapping);
                            if !editor.merge { ui.label("Import replaces the mapped MIDI clip contents; clip gain and other track controls are preserved."); }
                            egui::ScrollArea::vertical().id_salt("smf-map").max_height(240.0).show_rows(ui, 28.0, editor.mappings.len(), |ui, rows| {
                                for row in rows {
                                    let m = &mut editor.mappings[row];
                                    ui.push_id(row, |ui| ui.horizontal(|ui| {
                                        let label = format!("File track {}{}", m.source.track + 1, m.source.channel.map_or(String::new(), |c| format!(" channel {}", c + 1)));
                                        ui.label(&label);
                                        let mut used = m.destination.is_some();
                                        if ui.checkbox(&mut used, format!("Import {label}")).help(ui, HelpControl::MidiFileMapping).changed() {
                                            m.destination = used.then_some((self.snap.selected_track as u8, self.snap.selected_scene as u8));
                                        }
                                        if let Some((track, scene)) = &mut m.destination {
                                            let mut t = u16::from(*track) + 1; let mut s = u16::from(*scene) + 1;
                                            ui.label("Track"); number(ui, &format!("{label} destination track"), &mut t, 1, TRACKS as u16);
                                            ui.label("Scene"); number(ui, &format!("{label} destination scene"), &mut s, 1, SCENES as u16);
                                            *track = (t - 1) as u8; *scene = (s - 1) as u8;
                                        }
                                    }));
                                }
                            });
                            let label = match editor.tempo { TempoChoice::KeepSession => "Keep session tempo and meter".into(), TempoChoice::File(None) => "Use complete file conductor".into(), TempoChoice::File(Some(i)) => format!("Use file track {} conductor", i + 1) };
                            egui::ComboBox::from_id_salt("smf-conductor").selected_text(label).show_ui(ui, |ui| {
                                ui.selectable_value(&mut editor.tempo, TempoChoice::KeepSession, "Keep session tempo and meter");
                                ui.selectable_value(&mut editor.tempo, TempoChoice::File(None), "Use complete file conductor");
                                for i in 0..preview.file.tracks.len() { ui.selectable_value(&mut editor.tempo, TempoChoice::File(Some(i)), format!("Use file track {} conductor", i + 1)); }
                            }).response.help(ui, HelpControl::MidiFileTempo);
                            ui.checkbox(&mut editor.rounding, "Allow nearest-tick rounding when merging").help(ui, HelpControl::MidiFilePrecision);
                            if !preview.file.warnings.is_empty() {
                                ui.label("Unsupported data requiring explicit omission review:");
                                egui::ScrollArea::vertical().id_salt("smf-warnings").max_height(120.0).show_rows(ui, 20.0, preview.file.warnings.len(), |ui, rows| {
                                    for row in rows { let w = &preview.file.warnings[row]; ui.label(format!("Track {:?}, tick {}: {:?}", w.track.map(|t| t + 1), w.tick, w.kind)); }
                                });
                                ui.checkbox(&mut editor.reviewed, "Accept listed unsupported omissions").help(ui, HelpControl::MidiFileImport);
                            }
                        });
                        ui.collapsing("Inspect note, controller and conductor lanes", |ui| {
                            ui.horizontal(|ui| { ui.label("File track"); let mut track = editor.inspect_track as u16 + 1; number(ui, "MIDI source inspector track", &mut track, 1, preview.file.tracks.len() as u16); editor.inspect_track = track as usize - 1; });
                            let data = &preview.file.tracks[editor.inspect_track];
                            let count = data.notes.len() + data.messages.len() + data.meta.len();
                            egui::ScrollArea::vertical().id_salt("smf-source-events").max_height(180.0).show_rows(ui, 20.0, count, |ui, rows| {
                                for row in rows {
                                    let value = if row < data.notes.len() { let n = &data.notes[row]; format!("Tick {} length {} · channel {} note {} velocity {} release {}", n.start_tick, n.duration_ticks, n.channel + 1, n.pitch, n.velocity, n.release_velocity) }
                                        else if row < data.notes.len() + data.messages.len() { let m = &data.messages[row - data.notes.len()]; format!("Tick {} channel {} · {:02x?} ({} bytes)", m.tick, (m.bytes[0] & 15) + 1, m.bytes, m.length) }
                                        else { let m = &data.meta[row - data.notes.len() - data.messages.len()]; match &m.value { crate::midi_file::MetaValue::Text { kind, bytes } => format!("Tick {} text {:02x}, {} bytes", m.tick, kind, bytes.len()), _ => format!("Tick {} {:?}", m.tick, m.value) } };
                                    ui.label(value);
                                }
                            });
                        });
                        if button(ui, "Import inspected MIDI", allowed && (preview.file.warnings.is_empty() || editor.reviewed), HelpControl::MidiFileImport).clicked() { import = true; }
                    }
                }
                ui.label(&editor.message);
                if let Some(error) = &editor.error { ui.colored_label(ui.visuals().error_fg_color, error); }
                if busy && button(ui, "Cancel MIDI operation", true, HelpControl::MidiFileCancel).clicked() { editor.cancel(); }
            });
        if !visible {
            editor.cancel();
            editor.open = busy;
        }
        if inspect {
            editor.preview = None;
            editor.mappings.clear();
            let path = PathBuf::from(editor.path.trim());
            editor.start(&self.engine, |work| Job::Inspect { path, work });
        }
        if import {
            let preview = editor.preview.as_ref().unwrap().clone();
            let mappings = editor.mappings.clone();
            let (split, merge, tempo, reviewed, rounding) = (
                editor.split,
                editor.merge,
                editor.tempo,
                editor.reviewed,
                editor.rounding,
            );
            editor.start(&self.engine, |work| Job::Import {
                preview,
                mappings,
                split,
                merge,
                tempo,
                reviewed,
                rounding,
                work,
            });
        }
        if export {
            let path = PathBuf::from(editor.path.trim());
            let cells = editor.cells.iter().copied().collect();
            let options = ExportOptions {
                ppqn: editor.ppqn,
                single_track: editor.single_track,
                include_muted: editor.include_muted,
                allow_rounding: editor.rounding,
                session_conductor: editor.session_conductor,
            };
            editor.start(&self.engine, |work| Job::Export {
                path,
                cells,
                options,
                work,
            });
        }
        if editor.busy() {
            ctx.request_repaint_after(Duration::from_millis(20));
        }
        self.midi_files = editor;
    }
}

#[cfg(test)]
mod tests;

fn number(ui: &mut Ui, label: &str, value: &mut u16, min: u16, max: u16) -> egui::Response {
    let response = ui.add(egui::DragValue::new(value).range(min..=max));
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::DragValue, response.enabled(), label)
    });
    if let Some(next) = accessibility::numeric(
        ui,
        &response,
        label,
        f32::from(*value),
        f32::from(min),
        f32::from(max),
        1.0,
        "",
    ) {
        *value = next.round() as u16;
    }
    response
}
