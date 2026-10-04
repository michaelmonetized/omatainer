//! One frozen crate view, one low-priority decoder token, and one durable
//! catalog receipt at a time. Filesystem work stays on the existing owners.
use super::*;
use crate::engine::{
    media_analysis::Stage,
    media_load::{AnalysisCompletion, AnalysisRequest, AnalysisToken},
};
use crate::sampler_bank::SourceRef;
use crate::track_analysis::Fields;
use library_metadata::{AnalysisInspect, AnalysisInspected};
use std::sync::atomic::{AtomicBool, Ordering};
mod changes;

pub(super) const MAX_TRACKS: usize = 4096;
#[derive(Clone, Copy, PartialEq, Eq)]
enum Purpose {
    Analyze,
    Inspect,
    Preview,
    Refresh,
}
struct Captured {
    rows: Arc<Vec<LibItem>>,
    indices: Arc<Vec<usize>>,
    begin: usize,
    end: usize,
    cursor: usize,
    fields: Fields,
    force: bool,
    purpose: Purpose,
    cancelled: bool,
    cached: usize,
    saved: usize,
    unconfirmed: usize,
    skipped: usize,
}
impl Captured {
    fn item(&self) -> Option<&LibItem> {
        self.rows.get(*self.indices.get(self.cursor)?)
    }
    fn total(&self) -> usize {
        self.end - self.begin
    }
    fn done(&self) -> usize {
        self.cursor - self.begin
    }
}
enum Step {
    Ready,
    RefreshReady,
    Inspecting {
        id: u64,
        cancel: Arc<AtomicBool>,
        purpose: Purpose,
    },
    Decoding(AnalysisToken),
    Saving(AnalysisToken),
    Paused(String),
}
impl Default for Step {
    fn default() -> Self {
        Self::Ready
    }
}
pub(super) struct Panel {
    pub open: bool,
    fields: Fields,
    force: bool,
    generation: u64,
    next_inspection: u64,
    queue: Option<Captured>,
    step: Step,
    completion: Option<AnalysisCompletion>,
    preview: Option<AnalysisInspected>,
    preview_title: String,
    current_title: String,
    changes: Vec<changes::Row>,
    message: String,
    retiring: Option<(Arc<Vec<LibItem>>, Arc<Vec<usize>>)>,
}
impl Default for Panel {
    fn default() -> Self {
        Self {
            open: false,
            fields: Fields::ALL,
            force: false,
            generation: 0,
            next_inspection: 0,
            queue: None,
            step: Step::Ready,
            completion: None,
            preview: None,
            preview_title: String::new(),
            current_title: String::new(),
            changes:Vec::new(),
            message: String::new(),
            retiring: None,
        }
    }
}
impl Panel {
    pub(super) fn busy(&self) -> bool {
        self.queue.is_some() || self.retiring.is_some()
    }
    fn cancel(&mut self) {
        if let Some(queue) = &mut self.queue {
            queue.cancelled = true;
        }
        match &self.step {
            Step::Inspecting { cancel, .. } => cancel.store(true, Ordering::Release),
            Step::Decoding(token) | Step::Saving(token) => {
                token.cancel();
            }
            _ => {}
        }
        self.message = "Cancellation requested. A publication already claimed still reports its actual save result.".into();
    }
}
impl App {
    fn retire_analysis_preview(&mut self) -> bool {
        if !self.library_metadata.analysis_worker_available() {
            return false;
        }
        let Some(preview) = self.library_analysis.preview.take() else {
            return true;
        };
        if let Err(preview) = self.library_metadata.retire_analysis_inspection(preview) {
            self.library_analysis.preview = Some(preview);
            return false;
        }
        true
    }
    fn finish_analysis_queue(&mut self) {
        let Some(queue) = self.library_analysis.queue.take() else {
            return;
        };
        if queue.cancelled {
            self.library_analysis.message = format!("Queue cancelled; {} saved, {} reused from cache, {} skipped. Earlier committed results remain saved.", queue.saved, queue.cached, queue.skipped);
        } else if queue.purpose==Purpose::Preview {
            self.library_analysis.message=format!("Previewed {} captured rows. Preview does not decode or save. Source versions and locks are checked again when analysis runs.", self.library_analysis.changes.len());
        } else {
            self.library_analysis.message = format!(
                "Queue finished: {} saved, {} reused from cache, {} skipped of {} captured rows.",
                queue.saved,
                queue.cached,
                queue.skipped,
                queue.total()
            );
        }
        if queue.unconfirmed > 0 {
            self.library_analysis.message.push_str(&format!(
                " {} committed but not confirmed durable.",
                queue.unconfirmed
            ));
        }
        if let Step::Paused(detail) = &self.library_analysis.step {
            self.library_analysis.message.push_str(" Last outcome: ");
            self.library_analysis.message.push_str(detail);
        }
        self.library_analysis.retiring = Some((queue.rows, queue.indices));
        self.library_analysis.step = Step::Ready;
    }
    fn start_library_analysis(&mut self, filtered: bool, purpose: Purpose) {
        if !self.library_metadata.analysis_worker_available() {
            self.library_analysis.message = "Analysis owner is unavailable. Retained payloads remain local; restart before another queue. Pending publication may have an unknown outcome.".into();
            return;
        }
        if self.library_closing() {
            self.library_analysis.message =
                "Library close is in progress; no analysis was queued.".into();
            return;
        }
        if self.library_analysis.busy() {
            return;
        }
        if self.library_health.busy() {
            self.library_analysis.message = "Finish or cancel media validation first.".into();
            return;
        }
        if self.loader.is_none() {
            self.library_analysis.message =
                "The media decoder is unavailable. No analysis was queued.".into();
            return;
        }
        if self.project.committing() || !self.project.dialog_is_closed() {
            return;
        }
        if let Err(error) = self.engine.cmd.performance().optional_work() {
            self.library_analysis.message = error.to_string();
            return;
        }
        self.refresh_library_view();
        let (begin, end) = if filtered {
            (0, self.library_view.indices.len())
        } else {
            (
                self.lib_sel,
                self.lib_sel
                    .saturating_add(1)
                    .min(self.library_view.indices.len()),
            )
        };
        if end <= begin {
            self.library_analysis.message = "Select a library row first.".into();
            return;
        }
        if end - begin > MAX_TRACKS {
            self.library_analysis.message = format!("This filtered crate has {} rows. Narrow it to at most {MAX_TRACKS}; no rows were queued.", end-begin);
            return;
        }
        if !self.library_analysis.fields.valid() {
            self.library_analysis.message = "Choose at least one analysis field.".into();
            return;
        }
        let Some(generation) = self.library_analysis.generation.checked_add(1) else {
            self.library_analysis.message =
                "Analysis queue identity exhausted; restart before another queue.".into();
            return;
        };
        self.library_analysis.generation = generation;
        if purpose==Purpose::Preview {self.library_analysis.changes.clear();}
        self.library_analysis.queue = Some(Captured {
            rows: self.library.clone(),
            indices: self.library_view.indices.clone(),
            begin,
            end,
            cursor: begin,
            fields: self.library_analysis.fields,
            force: self.library_analysis.force,
            purpose,
            cancelled: false,
            cached: 0,
            saved: 0,
            unconfirmed: 0,
            skipped: 0,
        });
        self.library_analysis.step = Step::Ready;
        self.library_analysis.message = format!(
            "Captured {} rows. Later filtering, selection or scans cannot retarget this queue.",
            end - begin
        );
    }
    fn next_analysis_item(&mut self) {
        if let Some(queue) = &mut self.library_analysis.queue {
            queue.cursor += 1;
        }
        self.library_analysis.step = Step::Ready;
    }
    fn pause_analysis(&mut self, message: String) {
        self.library_analysis.message = message.clone();
        self.library_analysis.step = Step::Paused(message);
    }
    fn request_analysis_inspection(&mut self, purpose: Purpose) {
        if !self.retire_analysis_preview() {
            return;
        }
        let Some(queue) = &self.library_analysis.queue else {
            return;
        };
        let Some(item) = queue.item() else {
            self.pause_analysis("Captured row is invalid; no source was substituted.".into());
            return;
        };
        self.library_analysis.current_title = item.title.clone();
        if !matches!(item.source, LibSource::File(_) | LibSource::Removable {..}) {
            self.library_analysis.message = format!(
                "Skipped {}: background analysis supports local files and removable volumes.",
                item.title
            );
            self.library_analysis.queue.as_mut().unwrap().skipped += 1;
            self.next_analysis_item();
            return;
        }
        let Some(fingerprint) = item.fingerprint else {
            self.pause_analysis(
                "Captured file has no verified scan identity; scan and save the library first."
                    .into(),
            );
            return;
        };
        let Some(track) = self
            .library_metadata
            .catalog
            .track_for_version(&item.source, Some(fingerprint))
        else {
            self.pause_analysis("Captured file version is not saved in the library yet. Wait for the library save, then Retry current.".into());
            return;
        };
        let reference = SourceRef {
            track: track.id.clone(),
            source: item.source.clone(),
            fingerprint,
            content_hash: track
                .versions
                .iter()
                .find(|version| version.fingerprint == Some(fingerprint))
                .and_then(|version| version.content_hash),
        };
        let work = match self.engine.cmd.performance().optional_work() {
            Ok(work) => Arc::new(work),
            Err(error) => {
                self.pause_analysis(error.to_string());
                return;
            }
        };
        let Some(id) = self.library_analysis.next_inspection.checked_add(1) else {
            self.pause_analysis("Analysis inspection identity exhausted.".into());
            return;
        };
        let cancel = Arc::new(AtomicBool::new(false));
        let request = AnalysisInspect {
            id,
            reference,
            fields: queue.fields,
            force: matches!(purpose,Purpose::Analyze|Purpose::Preview) && queue.force,
            work,
            cancel: cancel.clone(),
        };
        match self.library_metadata.inspect_analysis(request) {
            Ok(()) => {
                self.library_analysis.next_inspection = id;
                self.library_analysis.step = Step::Inspecting {
                    id,
                    cancel,
                    purpose,
                };
            }
            Err(_) => { /* Metadata still owns its prior operation; retry next frame. */ }
        }
    }
    pub(super) fn poll_library_analysis(&mut self) {
        if self.library_metadata.analysis_worker_available() {
            if let Some(pair) = self.library_analysis.retiring.take() {
                if let Err(pair) = self.library_metadata.retire_analysis_rows(pair.0, pair.1) {
                    self.library_analysis.retiring = Some(pair);
                }
            }
        }
        if self
            .library_analysis
            .preview
            .as_ref()
            .is_some_and(|preview| {
                preview.work.cancelled() || preview.cancel.load(Ordering::Acquire)
            })
        {
            self.retire_analysis_preview();
        }
        if let Some(result) = self.library_metadata.take_analysis_inspection() {
            let purpose = match &self.library_analysis.step {
                Step::Inspecting { id, purpose, .. } if *id == result.id => Some(*purpose),
                _ => None,
            };
            // Only this panel submits inspections. Keeping an unmatched result in
            // the bounded preview slot also lets the owner retire its allocation.
            let aborted = result.work.cancelled()
                || result.cancel.load(Ordering::Acquire)
                || self
                    .library_analysis
                    .queue
                    .as_ref()
                    .is_none_or(|queue| queue.cancelled);
            let next = match (&result.outcome, purpose, aborted) {
                (_, _, true) => Err("Analysis inspection cancelled before display.".into()),
                (Err(error), _, false) => Err(error.clone()),
                (Ok(cached), Some(Purpose::Analyze), false) if cached.needed.valid() => {
                    Ok(Some(AnalysisRequest {
                        reference: result.reference.clone(),
                        fields: cached.needed,
                    }))
                }
                (Ok(_), Some(_), false) => Ok(None),
                _ => Err(
                    "Unexpected analysis inspection was retired without retargeting the queue."
                        .into(),
                ),
            };
            if purpose==Some(Purpose::Preview) && !aborted {
                if let Ok(cached)=&result.outcome {
                    let fields=self.library_analysis.queue.as_ref().unwrap().fields;
                    self.library_analysis.changes.push(changes::Row::new(&self.library_analysis.current_title,&result.reference,cached,fields));
                }
            }
            self.library_analysis.preview_title = self.library_analysis.current_title.clone();
            self.library_analysis.preview = Some(result);
            match next {
                Err(error) => self.pause_analysis(error),
                Ok(Some(request)) => {
                    match self
                        .loader
                        .as_ref()
                        .ok_or_else(|| "Media decoder is unavailable.".to_string())
                        .and_then(|loader| loader.request_analysis(request))
                    {
                        Ok(token) => self.library_analysis.step = Step::Decoding(token),
                        Err(error) => self.pause_analysis(error),
                    }
                }
                Ok(None) => {
                    if purpose == Some(Purpose::Analyze) {
                        let permitted=self.library_analysis.preview.as_ref().and_then(|preview|preview.outcome.as_ref().ok())
                            .is_some_and(|cached|cached.locks.analysis(self.library_analysis.queue.as_ref().unwrap().fields).valid());
                        let queue=self.library_analysis.queue.as_mut().unwrap();
                        if permitted {queue.cached+=1;} else {queue.skipped+=1;}
                    }
                    self.next_analysis_item();
                }
            }
        }
        if let Some(completion) = self.loader.as_ref().and_then(Loader::take_analysis_ready) {
            if matches!(&self.library_analysis.step, Step::Decoding(token) if token.id == completion.token.id)
            {
                // Terminal preemption/cancellation has is_current()==false;
                // match the awaited ID before interpreting its typed outcome.
                self.library_analysis.completion = Some(completion);
            } else {
                // No other UI analysis producer exists. Hand even stale results
                // to metadata for worker-side retirement and a terminal receipt.
                self.library_analysis.completion = Some(completion);
            }
        }
        if let Some(completion) = self.library_analysis.completion.take() {
            let token = completion.token.clone();
            match self.library_metadata.save_analysis(completion) {
                Ok(()) => self.library_analysis.step = Step::Saving(token),
                Err(completion) => self.library_analysis.completion = Some(completion),
            }
        }
        if let Some(receipt) = self.library_metadata.take_analysis_result() {
            if matches!(&self.library_analysis.step, Step::Saving(token) if token.id == receipt.id)
            {
                if receipt.committed {
                    if let Some(queue) = &mut self.library_analysis.queue {
                        if receipt.outcome.is_ok() {
                            queue.saved += 1;
                        } else {
                            queue.unconfirmed += 1;
                        }
                    }
                }
                match receipt.outcome {
                    Ok(()) => {
                        self.library_analysis.message =
                            "Analysis saved. Refreshing the verified cache…".into();
                        if self
                            .library_analysis
                            .queue
                            .as_ref()
                            .is_some_and(|queue| !queue.cancelled)
                        {
                            self.library_analysis.step = Step::RefreshReady;
                        } else {
                            self.next_analysis_item();
                        }
                    }
                    Err(error) if receipt.committed => {
                        // Publication won before a late Cancel or sync failure.
                        self.pause_analysis(format!("Analysis was committed, but durability needs attention: {error}. A retry creates a fresh operation; it does not undo this published result."));
                    }
                    Err(error) => self.pause_analysis(error.to_string()),
                }
            }
        }
        if !self.library_metadata.analysis_worker_available() {
            if self.library_analysis.queue.is_some() {
                self.library_analysis.cancel();
                self.pause_analysis("Analysis owner is unavailable; pending publication has an unknown outcome unless a terminal receipt above confirmed it. Retained payloads remain owned locally. Restart before another queue.".into());
                self.finish_analysis_queue();
            }
            return;
        }
        if self.library_analysis.queue.as_ref().is_some_and(|queue| {
            (queue.cancelled
                && matches!(
                    self.library_analysis.step,
                    Step::Ready | Step::RefreshReady | Step::Paused(_)
                ))
                || queue.cursor == queue.end
        }) {
            self.finish_analysis_queue();
            return;
        }
        if matches!(self.library_analysis.step, Step::RefreshReady) {
            self.request_analysis_inspection(Purpose::Refresh);
        } else if matches!(self.library_analysis.step, Step::Ready) {
            if let Some(queue) = &self.library_analysis.queue {
                let purpose = queue.purpose;
                self.request_analysis_inspection(purpose);
            }
        }
    }
    /// Close is centralized with library durability. Unclaimed decoder work is
    /// cancelled; an already handed-off save must produce its actual receipt.
    pub(super) fn stop_analysis_for_close(&mut self) -> bool {
        if self.library_analysis.queue.is_some() {
            self.library_analysis.cancel();
            if matches!(self.library_analysis.step, Step::Decoding(_)) {
                self.finish_analysis_queue();
            }
        }
        self.poll_library_analysis();
        !matches!(self.library_analysis.step, Step::Saving(_))
            && self.library_analysis.completion.is_none()
    }
    pub(super) fn library_analysis_ui(&mut self, ctx: &egui::Context) {
        if !self.library_analysis.open {
            return;
        }
        keyboard::block_for_dialog(ctx);
        let mut open = true;
        egui::Window::new(tr!("Background track analysis")).id(egui::Id::new("library-analysis-window"))
            .open(&mut open).default_width(660.0).vscroll(true).show(ctx, |ui| {
                ui.label(tr!("Closing this panel leaves an active queue running. Use Cancel analysis queue to stop it."));
                ui.label(tr!("Local files only. Results are source/version qualified. Manual BPM and locked preparation remain authoritative."));
                ui.label(tr!("Key analysis is unavailable. Existing filename hints and manual key values are not measured keys."));
                ui.add_enabled_ui(!self.library_analysis.busy(), |ui| {
                    ui.horizontal(|ui| {
                        ui.checkbox(&mut self.library_analysis.fields.bpm, tr!("Analyze BPM")).help(ui, HelpControl::AnalysisFields);
                        ui.checkbox(&mut self.library_analysis.fields.duration, tr!("Analyze duration")).help(ui, HelpControl::AnalysisFields);
                        ui.checkbox(&mut self.library_analysis.fields.waveform, tr!("Analyze waveform")).help(ui, HelpControl::AnalysisFields);
                        ui.checkbox(&mut self.library_analysis.fields.level, tr!("Analyze source level")).help(ui, HelpControl::AnalysisFields);
                    });
                    ui.checkbox(&mut self.library_analysis.force, tr!("Force selected fields")).help(ui, HelpControl::AnalysisForce);
                    ui.horizontal(|ui| {
                        if ui.button(tr!("Analyze selected row")).help(ui, HelpControl::AnalysisSelected).clicked() { self.start_library_analysis(false, Purpose::Analyze); }
                        if ui.button(tr!("Analyze filtered crate")).help(ui, HelpControl::AnalysisCrate).clicked() { self.start_library_analysis(true, Purpose::Analyze); }
                        if ui.button(tr!("Preview selected analysis changes")).help(ui, HelpControl::AnalysisInspect).clicked() { self.start_library_analysis(false, Purpose::Preview); }
                        if ui.button(tr!("Preview filtered analysis changes")).help(ui, HelpControl::AnalysisInspect).clicked() { self.start_library_analysis(true, Purpose::Preview); }
                    });
                    ui.horizontal(|ui| {
                        if ui.button(tr!("Inspect selected cache")).help(ui, HelpControl::AnalysisInspect).clicked() { self.start_library_analysis(false, Purpose::Inspect); }
                    });
                });
                ui.push_id("analysis-status", |ui| {
                    if let Some(queue) = &self.library_analysis.queue {
                        ui.label({ let __omatainer_args = (&(queue.done()),&(queue.total()),&(queue.saved),&(queue.cached),&(queue.skipped),); crate::localization::format("Captured queue: {} / {} rows complete; {} saved, {} cache hits, {} skipped", &[format!("{}", __omatainer_args.0), format!("{}", __omatainer_args.1), format!("{}", __omatainer_args.2), format!("{}", __omatainer_args.3), format!("{}", __omatainer_args.4)]) });
                        if queue.unconfirmed > 0 { ui.label({ let __omatainer_args = (&(queue.unconfirmed),); crate::localization::format("{} publication(s) committed without confirmed durability", &[format!("{}", __omatainer_args.0)]) }); }
                        ui.label(&self.library_analysis.current_title);
                    }
                    match &self.library_analysis.step {
                        Step::Inspecting { .. } => { ui.label(tr!("Checking the saved source version and waveform cache…")); }
                        Step::Decoding(token) => {
                            let progress = token.progress();
                            let stage = match progress.stage { Stage::Queued => "Queued", Stage::Hashing => "Hashing source", Stage::Decoding => "Decoding", Stage::Tempo => "Estimating BPM", Stage::Waveform => "Building waveform", Stage::Level => "Measuring source level", Stage::Ready => "Prepared; not saved yet" };
                            if let Some(value) = progress.millionths { ui.add(egui::ProgressBar::new(value as f32 / 1_000_000.0).text(stage)); } else { ui.label(stage); }
                        }
                        Step::Saving(_) => { ui.label(tr!("Waiting for the catalog publication and durability result…")); }
                        Step::Paused(error) => { ui.colored_label(ui.visuals().warn_fg_color, error); }
                        _ => {}
                    }
                    ui.label(&self.library_analysis.message);
                });
                ui.push_id(("analysis-actions", self.library_analysis.generation, self.library_analysis.queue.as_ref().map(|queue| queue.cursor)), |ui| {
                    // egui salts child IDs with the parent's auto counter.
                    // Keep every action's parent slot present in every phase.
                    ui.push_id("retry", |ui| {
                        if matches!(self.library_analysis.step, Step::Paused(_)) && self.library_analysis.queue.is_some()
                            && ui.button(tr!("Retry current analysis")).help(ui, HelpControl::AnalysisRetry).clicked() {
                            self.library_analysis.step = Step::Ready;
                        }
                    });
                    ui.push_id("skip", |ui| {
                        if matches!(self.library_analysis.step, Step::Paused(_)) && self.library_analysis.queue.is_some()
                            && ui.button(tr!("Skip current analysis")).help(ui, HelpControl::AnalysisSkip).clicked() {
                            self.library_analysis.queue.as_mut().unwrap().skipped += 1; self.next_analysis_item();
                        }
                    });
                    ui.push_id("cancel", |ui| {
                        if self.library_analysis.queue.is_some() && ui.button(tr!("Cancel analysis queue")).help(ui, HelpControl::AnalysisCancel).clicked() { self.library_analysis.cancel(); }
                    });
                });
                if !self.library_analysis.changes.is_empty() {
                    ui.separator(); ui.label(tr!("Analysis replacement preview"));
                    ui.label(tr!("Selected fields refresh their analysis records. User BPM, embedded tags, grids, cues and locks remain authoritative. No media files are written."));
                    let output=egui::ScrollArea::vertical().id_salt("analysis-change-preview").max_height(230.0)
                        .show_rows(ui,88.0,self.library_analysis.changes.len(),|ui,range| {
                            for index in range {self.library_analysis.changes[index].show(ui,&self.library_metadata.catalog);}
                        });
                    accessibility::scrollbars(ui,"Analysis replacement preview",&output);
                }
                ui.push_id("analysis-cache-preview", |ui| {
                    if let Some(preview) = &self.library_analysis.preview {
                        ui.separator(); ui.label({ let __omatainer_args = (&(self.library_analysis.preview_title),); crate::localization::format("Verified cached values: {}", &[format!("{}", __omatainer_args.0)]) });
                        if let Ok(cached) = &preview.outcome {
                            if let Some(notice) = &cached.notice { ui.label(notice); }
                            if let Some(record) = &cached.record {
                                if let Some(bpm) = &record.bpm { ui.label(match bpm.value { Some(value) => { let __omatainer_args = (&(bpm.algorithm),); crate::localization::format("Analyzed BPM: {value:.2} (heuristic, unverified; algorithm {})", &[format!("{:.2}", value), format!("{}", __omatainer_args.0)]) }, None => tr!("Analyzed BPM: no usable estimate").into() }); }
                                if let Some(duration) = &record.duration { ui.label({ let __omatainer_args = (&(duration.value),); crate::localization::format("Analyzed duration: {:.3} seconds", &[format!("{:.3}", __omatainer_args.0)]) }); }
                                if let Some(level) = &record.level { ui.label(crate::track_gain::description(level.value.level)); ui.label(match level.value.recommended_db { Some(db) => crate::localization::format("Recommended source trim: {} dB (-18 dBFS RMS / -3 dBFS sample peak)", &[crate::localization::number(f64::from(db), 2)]), None => tr!("No usable automatic gain recommendation").into() }); }
                            }
                            if cached.needed.valid() { ui.label(tr!("Some selected fields require fresh analysis. Inspect alone never decodes a source.")); }
                            if let Some(waveform) = &cached.waveform {
                                ui.label({ let __omatainer_args = (&(waveform.bands.len()),&(waveform.frames as f64 / f64::from(waveform.sample_rate)),&(waveform.sample_rate),&(waveform.channels),); crate::localization::format("Cached waveform: {} bins, {:.3} seconds, {} Hz, {} channels", &[format!("{}", __omatainer_args.0), format!("{:.3}", __omatainer_args.1), format!("{}", __omatainer_args.2), format!("{}", __omatainer_args.3)]) });
                                let (rect, response) = ui.allocate_exact_size(Vec2::new(ui.available_width().min(600.0), 80.0), Sense::hover());
                                response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Other, true, "Cached analysis waveform: low, mid and high band mean magnitudes in source order"));
                                for (index, bands) in waveform.bands.iter().enumerate() {
                                    let x = rect.left() + rect.width() * index as f32 / waveform.bands.len() as f32;
                                    for (band, color) in bands.iter().zip([Color32::from_rgb(90,150,230), Color32::from_rgb(100,200,130), Color32::from_rgb(230,170,80)]) {
                                        ui.painter().line_segment([Pos2::new(x, rect.center().y - *band * 38.0), Pos2::new(x, rect.center().y + *band * 38.0)], egui::Stroke::new(1.0_f32, color));
                                    }
                                }
                            }
                        }
                    }
                });
                if ui.button(tr!("Close analysis panel")).help(ui, HelpControl::AnalysisClose).clicked() { self.library_analysis.open = false; }
                if self.library_analysis.busy() { ctx.request_repaint_after(std::time::Duration::from_millis(33)); }
            });
        self.library_analysis.open &= open;
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
impl Panel {
    pub(super) fn evidence(&self) -> serde_json::Value {
        let cached = self
            .preview
            .as_ref()
            .and_then(|preview| preview.outcome.as_ref().ok());
        serde_json::json!({ "open":self.open, "busy":self.busy(), "message":self.message,
            "waveform_bins":cached.and_then(|cache| cache.waveform.as_ref()).map(|wave| wave.bands.len()),
            "record":cached.and_then(|cache| cache.record.as_ref()),
            "title":self.preview_title })
    }
}
