use super::*;
use capture::{Capture, Completed, Metadata, Sample, Worker};
mod capture;
#[cfg(test)]
mod tests;

pub(super) struct Diagnostics {
    pub open: bool,
    pub ui_update_ns: Option<u64>,
    capture: Capture,
    path: String,
    status: String,
    worker: Option<Worker>,
    #[cfg(test)]
    file_gate: Option<Arc<std::sync::Barrier>>,
    #[cfg(test)]
    pub(super) ui_delay: std::time::Duration,
}
impl Default for Diagnostics {
    fn default() -> Self {
        let stamp = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        Self {
            open: false,
            ui_update_ns: None,
            capture: Capture::default(),
            path: std::env::temp_dir()
                .join(format!("omatainer-diagnostics-{stamp}.json"))
                .display()
                .to_string(),
            status: "No capture".into(),
            worker: None,
            #[cfg(test)]
            file_gate: None,
            #[cfg(test)]
            ui_delay: std::time::Duration::ZERO,
        }
    }
}
impl App {
    fn diagnostic_metadata(&self) -> Metadata {
        let clips = self.snap.tracks.iter().flat_map(|track| &track.clips);
        let output = self.engine.output_info();
        Metadata {
            app_version: env!("CARGO_PKG_VERSION").into(),
            revision: option_env!("OMATAINER_REVISION")
                .unwrap_or("unavailable")
                .into(),
            os: std::env::consts::OS.into(),
            architecture: std::env::consts::ARCH.into(),
            logical_cpus: std::thread::available_parallelism()
                .map(usize::from)
                .unwrap_or(0),
            audio_backend: output.map(|info| info.backend.clone()),
            output_format: output.map(|info| info.format.clone()),
            midi_status_entries: self.snap.midi.len(),
            tracks: self.snap.tracks.len(),
            scenes: SCENES,
            midi_clips: clips.clone().filter(|clip| clip.kind == 1).count(),
            audio_clips: clips.filter(|clip| clip.kind == 2).count(),
            session_bpm: self.snap.bpm,
        }
    }
    fn diagnostic_sample(&self) -> Sample {
        let midi = self.engine.midi.input_stats();
        let library = self.engine.cmd.ui_request_stats();
        Sample {
            elapsed_ms: 0,
            audio: self.engine.cmd.audio_metrics(),
            commands: self.engine.cmd.queue_pressure(),
            ui_update_ns: self.diagnostics.ui_update_ns,
            midi: [
                midi.received,
                midi.queued,
                midi.dispatched,
                midi.coalesced,
                midi.dropped,
                midi.resets,
                midi.oversized,
                midi.disconnected,
            ],
            library_queue: [
                library.pending as u64,
                crate::engine::ui_requests::CAPACITY as u64,
                library.accepted,
                library.dispatched,
                library.rejected,
            ],
            profile: self.engine.cmd.load_profile(),
        }
    }
    pub(super) fn diagnostics_panel(&mut self, ctx: &egui::Context) {
        if let Some(done) = self.diagnostics.worker.as_ref().and_then(Worker::poll) {
            self.diagnostics.worker = None;
            self.diagnostics.status = match done {
                Completed::Exported => "Redacted capture exported".into(),
                Completed::Reopened(report) => {
                    self.diagnostics.capture.report = Some(report);
                    "Capture reopened; live engine unchanged".into()
                }
                Completed::Cancelled => "File operation cancelled".into(),
                Completed::Failed(error) => format!("Diagnostic file failed: {error}"),
            };
        }
        let now = Instant::now();
        if self.diagnostics.capture.due(now) {
            let sample = self.diagnostic_sample();
            self.diagnostics.capture.record(sample, now);
            if !self.diagnostics.capture.running() {
                self.diagnostics.status = "Bounded capture complete".into();
            }
        }
        self.engine
            .cmd
            .set_profiling(self.diagnostics.open || self.diagnostics.capture.running());
        if !self.diagnostics.open {
            return;
        }
        let live = self.diagnostic_sample();
        let mut open = true;
        egui::Window::new("Performance diagnostics").open(&mut open).default_width(780.0).default_height(800.0).vscroll(true).show(ctx, |ui| {
            ui.label("Live output device · measured service times; backend XRUN count unavailable");
            show_sample(ui, &live);
            ui.separator();
            ui.label("Capture: at most 30 seconds / 120 GUI samples / 2 MiB export. Missing GUI intervals are not reconstructed.");
            ui.label("Redacted by construction: no media paths, titles, usernames, hostnames, port names or MIDI note data. Project metadata is counts and tempo; hardware is backend, format, rate, channels, OS/CPU architecture and logical CPU count.");
            let running = self.diagnostics.capture.running();
            let busy = self.diagnostics.worker.is_some();
            ui.horizontal_wrapped(|ui| {
                if ui.add_enabled(!busy && !running, egui::Button::new("Start capture")).clicked() {
                    self.diagnostics.capture.start(self.diagnostic_metadata(), Instant::now());
                    self.diagnostics.status = "Capturing".into();
                }
                if ui.add_enabled(running, egui::Button::new("Stop capture")).clicked() { self.diagnostics.capture.stop(); self.diagnostics.status = "Capture stopped".into(); }
                if ui.add_enabled(running, egui::Button::new("Cancel capture")).clicked() { self.diagnostics.capture.cancel(); self.diagnostics.status = "Capture cancelled".into(); }
            });
            ui.label(&self.diagnostics.status);
            if let Some(report) = &self.diagnostics.capture.report {
                ui.label(format!("Captured samples: {} · schema {} · version {}", report.samples.len(), report.schema, report.metadata.app_version));
                if let Some(sample) = report.samples.last() {
                    egui::CollapsingHeader::new("Captured last sample (not live)").show(ui, |ui| show_sample(ui, sample));
                }
            }
            ui.horizontal(|ui| {
                ui.label("Diagnostic file");
                ui.add(egui::TextEdit::singleline(&mut self.diagnostics.path).id_salt("diagnostic-path").desired_width(490.0));
            });
            ui.label("Export creates a new private file; existing files are never overwritten. Reopen displays measurements only.");
            ui.horizontal_wrapped(|ui| {
                let has_samples = self.diagnostics.capture.report.as_ref().is_some_and(|r| !r.samples.is_empty());
                if ui.add_enabled(!busy && !running && has_samples, egui::Button::new("Export redacted")).clicked() {
                    self.start_diagnostic_file(self.diagnostics.capture.report.clone());
                }
                if ui.add_enabled(!busy && !running, egui::Button::new("Reopen capture")).clicked() { self.start_diagnostic_file(None); }
                if ui.add_enabled(busy, egui::Button::new("Cancel file operation")).clicked() {
                    if let Some(worker) = &self.diagnostics.worker { worker.cancel(); }
                    self.diagnostics.status = "Cancelling file operation".into();
                }
            });
        });
        self.diagnostics.open = open;
        self.engine
            .cmd
            .set_profiling(open || self.diagnostics.capture.running());
    }
    fn start_diagnostic_file(&mut self, report: Option<capture::Report>) {
        self.diagnostics.status = if report.is_some() {
            "Exporting redacted capture"
        } else {
            "Reopening capture"
        }
        .into();
        match Worker::start(
            PathBuf::from(&self.diagnostics.path),
            report,
            #[cfg(test)]
            self.diagnostics.file_gate.clone(),
        ) {
            Ok(worker) => self.diagnostics.worker = Some(worker),
            Err(error) => self.diagnostics.status = format!("Diagnostic worker failed: {error}"),
        }
    }
}

fn show_sample(ui: &mut Ui, sample: &Sample) {
    if let Some(callback) = sample.audio.last_callback {
        let cpu = callback
            .render_cpu_fraction()
            .map(|f| format!("{:.2}%", f * 100.0))
            .unwrap_or_else(|| "unavailable".into());
        ui.label(format!(
            "Render CPU {cpu} · callback wall {:.3} ms / {:.3} ms deadline",
            callback.elapsed_ns as f64 / 1e6,
            callback.budget_ns as f64 / 1e6
        ));
        ui.label(format!(
            "Output {} Hz · {} channels · {} frames",
            callback.sample_rate, callback.channels, callback.frames
        ));
        ui.label(callback.output_latency_ns.map(|ns| format!("Predicted output latency {:.3} ms", ns as f64 / 1e6)).unwrap_or_else(|| "Predicted output latency unavailable".into()))
            .on_hover_text("CPAL playback timestamp minus callback timestamp. Backend prediction to playback, not measured round-trip latency or UI responsiveness.");
    } else {
        ui.label("Audio callback measurement unavailable");
    }
    ui.label(format!("Callbacks {} · deadline overruns {} · maximum wall {:.3} ms · backend errors {} · device lost {} · XRUNs unavailable", sample.audio.callbacks, sample.audio.deadline_overruns, sample.audio.max_elapsed_ns as f64 / 1e6, sample.audio.backend_errors, sample.audio.device_lost));
    ui.label(sample.ui_update_ns.map(|ns| format!("UI update wall {:.3} ms", ns as f64 / 1e6)).unwrap_or_else(|| "UI update wall unavailable".into()))
        .on_hover_text("Previous App update call, including controls/layout. Excludes eframe GPU presentation and frame pacing; not FPS or audio latency.");
    let queue = sample.commands;
    ui.label(format!("Audio queue {}/{} · observed high-water {} · reserved releases {} · full rejections {} · all rejections {}", queue.pending, queue.capacity, queue.observed_high_water, queue.reserved_releases, queue.full_rejections, queue.rejected));
    ui.label(format!(
        "Library queue {}/{} · rejected {} · MIDI discarded {} · source resets {}",
        sample.library_queue[0],
        sample.library_queue[1],
        sample.library_queue[4],
        sample.midi[4],
        sample.midi[5]
    ));
    if let Some(profile) = &sample.profile {
        ui.label(format!("Sampled load at audio frame {}: one frame every {}. First {} devices per chain; {} omitted.", profile.frame, crate::engine::diagnostics::STRIDE, crate::engine::diagnostics::SLOTS, profile.omitted_devices));
        ui.label("Wall ns per sampled frame; estimated buffer share assumes that frame repeats. Track/scene totals include their devices; do not add them. Scheduling and timer overhead are included. Not per-track CPU or worst-case headroom.");
        egui::CollapsingHeader::new("Track and device samples")
            .default_open(false)
            .show(ui, |ui| {
                egui::Grid::new("diagnostic-costs")
                    .striped(true)
                    .show(ui, |ui| {
                        for cost in &profile.costs {
                            ui.label(cost.label());
                            ui.label(if cost.elapsed_ns == 0 {
                                "Below timer resolution".into()
                            } else {
                                format!("{} ns/frame", cost.elapsed_ns)
                            });
                            ui.label(format!(
                                "{:.3}% estimated",
                                cost.elapsed_ns as f64 * profile.sample_rate as f64 / 1e7
                            ));
                            ui.end_row();
                        }
                    });
            });
    } else {
        ui.label("Sampled track/device load awaiting audio (profiling is enabled while this window or a capture is active)");
    }
}
