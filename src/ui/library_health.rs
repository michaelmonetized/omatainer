//! Captured crate validation never installs media or writes preparation.
use super::*;
use crate::engine::{
    media_analysis::Token,
    media_health::{self, Observation},
};

const MAX_CHECKS: usize = 4096;
struct Captured {
    rows: Arc<Vec<LibItem>>,
    indices: Arc<Vec<usize>>,
    cursor: usize,
    end: usize,
    done: usize,
    failed: usize,
    cancelled: bool,
}
struct Checked {
    fingerprint: Option<FileFingerprint>,
    observation: Observation,
}
#[derive(Default)]
pub(super) struct Panel {
    pub open: bool,
    queue: Option<Captured>,
    token: Option<Token>,
    observations: HashMap<LibSource, Checked>,
    retiring: Option<(Arc<Vec<LibItem>>, Arc<Vec<usize>>)>,
    message: String,
}
impl Panel {
    /// Read the status of this exact published row version.
    /// Takes a library row; returns the last matching observation, without filesystem work.
    pub fn observation(&self, item: &LibItem) -> Option<&Observation> {
        self.observations
            .get(&item.source)
            .filter(|checked| checked.fingerprint == item.fingerprint)
            .map(|checked| &checked.observation)
    }
    /// Identify retained validation work.
    /// Takes this panel; returns whether its queue or captured-row retirement is pending.
    pub(super) fn busy(&self) -> bool {
        self.queue.is_some() || self.retiring.is_some()
    }
    /// Cancel remaining captured checks.
    /// Takes this panel; cancels its current decoder token while retaining completed observations.
    pub(super) fn cancel(&mut self) {
        if let Some(queue) = &mut self.queue {
            queue.cancelled = true;
        }
        if let Some(token) = &self.token {
            token.cancel();
        }
    }
}
impl App {
    /// Capture the requested browser scope for read-only validation.
    /// Takes the selected-versus-filtered choice; starts a bounded queue or explains the refused admission.
    fn start_library_validation(&mut self, filtered: bool) {
        if self.library_health.busy() {
            return;
        }
        if self.library_closing() || !self.library_metadata.analysis_worker_available() {
            self.library_health.message =
                "Library owner is unavailable or closing; no validation was queued.".into();
            return;
        }
        if self.loader.is_none() {
            self.library_health.message = "Media decoder is unavailable.".into();
            return;
        }
        if self.library_analysis.busy() {
            self.library_health.message = "Finish or cancel the analysis queue first.".into();
            return;
        }
        if let Err(error) = self.engine.cmd.performance().optional_work() {
            self.library_health.message = error.to_string();
            return;
        }
        self.refresh_library_view();
        let begin = if filtered { 0 } else { self.lib_sel };
        let end = if filtered {
            self.library_view.indices.len()
        } else {
            begin.saturating_add(1).min(self.library_view.indices.len())
        };
        if begin >= end {
            self.library_health.message = "Select a row or a nonempty crate first.".into();
            return;
        }
        if end - begin > MAX_CHECKS {
            self.library_health.message =
                format!("Narrow the crate to at most {MAX_CHECKS} rows; nothing was queued.");
            return;
        }
        self.library_health.queue = Some(Captured {
            rows: self.library.clone(),
            indices: self.library_view.indices.clone(),
            cursor: begin,
            end,
            done: 0,
            failed: 0,
            cancelled: false,
        });
        self.library_health.message = format!(
            "Captured {} rows. Validation reads media; live decks keep their own sources.",
            end - begin
        );
    }
    /// Report the actual completed queue outcome.
    /// Takes an optional stop reason; retains completed observations and sends captured rows to their existing retirement owner.
    fn finish_library_validation(&mut self, reason: Option<String>) {
        let queue = self.library_health.queue.take().unwrap();
        self.library_health.message = format!(
            "{} {} checked, {} need attention. Earlier results remain visible.",
            reason.unwrap_or_else(|| if queue.cancelled {
                "Validation cancelled.".into()
            } else {
                "Validation finished.".into()
            }),
            queue.done,
            queue.failed
        );
        self.library_health.retiring = Some((queue.rows, queue.indices));
        self.library_health.token = None;
    }
    /// Advance one captured check without waiting on I/O.
    /// Takes the app; accepts only its current token's result and queues the next original source.
    pub(super) fn poll_library_validation(&mut self) {
        if let Some(pair) = self.library_health.retiring.take() {
            if let Err(pair) = self.library_metadata.retire_analysis_rows(pair.0, pair.1) {
                self.library_health.retiring = Some(pair);
            }
            return;
        }
        if let Some(completion) = self.loader.as_ref().and_then(Loader::take_health_ready) {
            let matches = self
                .library_health
                .token
                .as_ref()
                .is_some_and(|token| token.id == completion.token.id);
            if !matches {
                return;
            }
            self.library_health.token = None;
            let Some(queue) = &mut self.library_health.queue else {
                return;
            };
            if queue.cancelled {
                self.finish_library_validation(None);
                return;
            }
            match completion.result {
                Ok(observation) => {
                    let item = &queue.rows[queue.indices[queue.cursor]];
                    queue.failed +=
                        usize::from(observation.condition != media_health::Condition::Ready);
                    if self.library_health.observations.len() == 100_000
                        && !self.library_health.observations.contains_key(&item.source)
                    {
                        if let Some(old) = self.library_health.observations.keys().next().cloned() {
                            self.library_health.observations.remove(&old);
                        }
                    }
                    self.library_health.observations.insert(
                        item.source.clone(),
                        Checked {
                            fingerprint: item.fingerprint,
                            observation,
                        },
                    );
                    queue.done += 1;
                    queue.cursor += 1;
                    self.library_view.cells.clear();
                }
                Err(reason) => {
                    let message = match reason {
                        crate::engine::media_analysis::Failure::Cancelled => "Validation cancelled.",
                        crate::engine::media_analysis::Failure::Preempted => "Validation stopped for an explicit deck or sampler load. Run it again when that load finishes.",
                        crate::engine::media_analysis::Failure::Protected => "Validation stopped for performance protection. Run it again in Studio.",
                        crate::engine::media_analysis::Failure::Failed(_) => "Validation could not finish this request. Scan and retry the captured media.",
                    };
                    self.finish_library_validation(Some(message.into()));
                    return;
                }
            }
        }
        let Some(queue) = &self.library_health.queue else {
            return;
        };
        if self.library_health.token.is_some() {
            return;
        }
        if queue.cancelled || queue.cursor >= queue.end {
            self.finish_library_validation(None);
            return;
        }
        let item = &queue.rows[queue.indices[queue.cursor]];
        let request = media_health::Request {
            source: item.source.clone(),
            fingerprint: item.fingerprint,
        };
        match self.loader.as_ref().unwrap().request_health(request) {
            Ok(token) => self.library_health.token = Some(token),
            Err(reason) => {
                self.finish_library_validation(Some(format!("Validation stopped: {reason}.")))
            }
        }
    }
    /// Show validation controls and the selected row's last observation.
    /// Takes the UI context; opens, cancels or hides checks without changing loaded media.
    pub(super) fn library_health_ui(&mut self, ctx: &egui::Context) {
        if !self.library_health.open {
            return;
        }
        keyboard::block_for_dialog(ctx);
        let mut open = true;
        let mut close = false;
        egui::Window::new(tr!("Library media health")).id(egui::Id::new("library-health-window")).open(&mut open).default_width(650.0).show(ctx, |ui| {
            ui.label("Validate a captured selection or filtered crate before loading. Closing this panel leaves the queue running.");
            ui.label("Checks are read-only, use one optional decoder lane and stop for performance protection or explicit media loads. Results describe the last checked source version.");
            ui.add_enabled_ui(!self.library_health.busy(), |ui| {
                ui.horizontal(|ui| {
                    if ui.button(tr!("Validate selected row")).help(ui, HelpControl::LibraryHealth).clicked() { self.start_library_validation(false); }
                    if ui.button(tr!("Validate filtered crate")).help(ui, HelpControl::LibraryHealth).clicked() { self.start_library_validation(true); }
                });
            });
            if let Some(queue) = &self.library_health.queue {
                ui.label(format!("{} rows checked; {} need attention", queue.done, queue.failed));
                let item = &queue.rows[queue.indices[queue.cursor.min(queue.end-1)]];
                ui.label(&item.title);
                if let Some(token) = &self.library_health.token {
                    if let Some(value) = token.progress().millionths { ui.add(egui::ProgressBar::new(value as f32 / 1_000_000.0).text("Checking audio packets")); }
                }
                if ui.button(tr!("Cancel media validation")).help(ui, HelpControl::LibraryHealth).clicked() { self.library_health.cancel(); }
            }
            ui.label(&self.library_health.message);
            self.refresh_library_view();
            if let Some(item) = self.library_view.indices.get(self.lib_sel).and_then(|&i| self.library.get(i)) {
                ui.separator(); ui.label(&item.title);
                if let Some(observation) = self.library_health.observation(item) {
                    ui.label(observation.description());
                    ui.small(format!("Last checked: {}. Read-only files retain sidecar preparation.", play_time::format(Some(observation.checked), SystemTime::now()).label));
                } else { ui.label("Not validated for this source version. Scan and validate after replacing or reconnecting media."); }
            }
            if ui.button(tr!("Close media health")).help(ui, HelpControl::LibraryHealth).clicked() { close=true; }
        });
        self.library_health.open = open && !close;
    }
}

#[cfg(test)]
mod tests;
