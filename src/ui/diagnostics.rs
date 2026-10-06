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
            audio_backend: output.as_ref().map(|info| info.backend.clone()),
            output_format: output.as_ref().map(|info| info.format.clone()),
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
        egui::Window::new(tr!("Performance diagnostics")).id(egui::Id::new("Performance diagnostics")).open(&mut open).default_width(780.0).default_height(800.0).vscroll(true).show(ctx, |ui| {
            let feedback = self.engine.midi.feedback_stats();
            ui.label(format!("MIDI feedback: {} outputs, {} sent, {} failed", feedback.connected, feedback.sent, feedback.failed));
            ui.label(tr!("Live output device · measured service times; backend XRUN count unavailable"));
            show_sample(ui, &live);
            ui.separator();
            ui.label(tr!("Capture: at most 30 seconds / 120 GUI samples / 2 MiB export. Missing GUI intervals are not reconstructed."));
            ui.label(tr!("Redacted by construction: no media paths, titles, usernames, hostnames, port names or MIDI note data. Project metadata is counts and tempo; hardware is backend, format, rate, channels, OS/CPU architecture and logical CPU count."));
            let running = self.diagnostics.capture.running();
            let busy = self.diagnostics.worker.is_some();
            ui.horizontal_wrapped(|ui| {
                let response = ui.add_enabled(!busy && !running, egui::Button::new(tr!("Start capture")));
                help::annotate(ui, &response, help::Control::DiagnosticCapture);
                if response.clicked() {
                    self.diagnostics.capture.start(self.diagnostic_metadata(), Instant::now());
                    self.diagnostics.status = "Capturing".into();
                }
                let response = ui.add_enabled(running, egui::Button::new(tr!("Stop capture")));
                help::annotate(ui, &response, help::Control::DiagnosticStop);
                if response.clicked() { self.diagnostics.capture.stop(); self.diagnostics.status = "Capture stopped".into(); }
                let response = ui.add_enabled(running, egui::Button::new(tr!("Cancel capture")));
                help::annotate(ui, &response, help::Control::DiagnosticCancel);
                if response.clicked() { self.diagnostics.capture.cancel(); self.diagnostics.status = "Capture cancelled".into(); }
            });
            ui.label(&self.diagnostics.status);
            if let Some(report) = &self.diagnostics.capture.report {
                ui.label({ let __omatainer_args = (&(report.samples.len()),&(report.schema),&(report.metadata.app_version),); crate::localization::format("Captured samples: {} · schema {} · version {}", &[format!("{}", __omatainer_args.0), format!("{}", __omatainer_args.1), format!("{}", __omatainer_args.2)]) });
                if let Some(sample) = report.samples.last() {
                    let sample = egui::CollapsingHeader::new("Captured last sample (not live)").show(ui, |ui| show_sample(ui, sample));
                    help::annotate(ui, &sample.header_response, help::Control::Diagnostics);
                }
            }
            ui.horizontal(|ui| {
                ui.label(tr!("Diagnostic file"));
                let path = ui.add(egui::TextEdit::singleline(&mut self.diagnostics.path).id_salt("diagnostic-path").desired_width(490.0));
                path.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, "Diagnostic file path"));
                help::annotate(ui, &path, help::Control::DiagnosticPath);
            });
            ui.label(tr!("Export creates a new private file; existing files are never overwritten. Reopen displays measurements only."));
            ui.horizontal_wrapped(|ui| {
                let has_samples = self.diagnostics.capture.report.as_ref().is_some_and(|r| !r.samples.is_empty());
                let response = ui.add_enabled(!busy && !running && has_samples, egui::Button::new(tr!("Export redacted")));
                help::annotate(ui, &response, help::Control::DiagnosticExport);
                if response.clicked() {
                    self.start_diagnostic_file(self.diagnostics.capture.report.clone());
                }
                let response = ui.add_enabled(!busy && !running, egui::Button::new(tr!("Reopen capture")));
                help::annotate(ui, &response, help::Control::DiagnosticReopen);
                if response.clicked() { self.start_diagnostic_file(None); }
                let response = ui.add_enabled(busy, egui::Button::new(tr!("Cancel file operation")));
                help::annotate(ui, &response, help::Control::DiagnosticFileCancel);
                if response.clicked() {
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
        match Worker::start_for_show(
            PathBuf::from(&self.diagnostics.path),
            report,
            self.engine.cmd.performance(),
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
        ui.label({ let __omatainer_args = (&(callback.elapsed_ns as f64 / 1e6),&(callback.budget_ns as f64 / 1e6),); crate::localization::format("Render CPU {cpu} · callback wall {:.3} ms / {:.3} ms deadline", &[format!("{}", cpu), format!("{:.3}", __omatainer_args.0), format!("{:.3}", __omatainer_args.1)]) });
        ui.label({ let __omatainer_args = (&(callback.sample_rate),&(callback.channels),&(callback.frames),); crate::localization::format("Output {} Hz · {} channels · {} frames", &[format!("{}", __omatainer_args.0), format!("{}", __omatainer_args.1), format!("{}", __omatainer_args.2)]) });
        ui.label(callback.output_latency_ns.map(|ns| { let __omatainer_args = (&(ns as f64 / 1e6),); crate::localization::format("Predicted output latency {:.3} ms", &[format!("{:.3}", __omatainer_args.0)]) }).unwrap_or_else(|| tr!("Predicted output latency unavailable").into()))
            .on_hover_text(tr!("CPAL playback timestamp minus callback timestamp. Backend prediction to playback, not measured round-trip latency or UI responsiveness."));
    } else {
        ui.label(tr!("Audio callback measurement unavailable"));
    }
    ui.label({ let __omatainer_args = (&(sample.audio.callbacks),&(sample.audio.deadline_overruns),&(sample.audio.max_elapsed_ns as f64 / 1e6),&(sample.audio.backend_errors),&(sample.audio.device_lost),); crate::localization::format("Callbacks {} · deadline overruns {} · maximum wall {:.3} ms · backend errors {} · device lost {} · XRUNs unavailable", &[format!("{}", __omatainer_args.0), format!("{}", __omatainer_args.1), format!("{:.3}", __omatainer_args.2), format!("{}", __omatainer_args.3), format!("{}", __omatainer_args.4)]) });
    ui.label(sample.ui_update_ns.map(|ns| { let __omatainer_args = (&(ns as f64 / 1e6),); crate::localization::format("UI update wall {:.3} ms", &[format!("{:.3}", __omatainer_args.0)]) }).unwrap_or_else(|| tr!("UI update wall unavailable").into()))
        .on_hover_text(tr!("Previous App update call, including controls/layout. Excludes eframe GPU presentation and frame pacing; not FPS or audio latency."));
    let queue = sample.commands;
    ui.label({ let __omatainer_args = (&(queue.pending),&(queue.capacity),&(queue.observed_high_water),&(queue.reserved_releases),&(queue.full_rejections),&(queue.rejected),); crate::localization::format("Audio queue {}/{} · observed high-water {} · reserved releases {} · full rejections {} · all rejections {}", &[format!("{}", __omatainer_args.0), format!("{}", __omatainer_args.1), format!("{}", __omatainer_args.2), format!("{}", __omatainer_args.3), format!("{}", __omatainer_args.4), format!("{}", __omatainer_args.5)]) });
    ui.label({ let __omatainer_args = (&(sample.library_queue[0]),&(sample.library_queue[1]),&(sample.library_queue[4]),&(sample.midi[4]),&(sample.midi[5]),); crate::localization::format("Library queue {}/{} · rejected {} · MIDI discarded {} · source resets {}", &[format!("{}", __omatainer_args.0), format!("{}", __omatainer_args.1), format!("{}", __omatainer_args.2), format!("{}", __omatainer_args.3), format!("{}", __omatainer_args.4)]) });
    if let Some(profile) = &sample.profile {
        ui.label({ let __omatainer_args = (&(profile.frame),&(crate::engine::diagnostics::STRIDE),&(crate::engine::diagnostics::SLOTS),&(profile.omitted_devices),); crate::localization::format("Sampled load at audio frame {}: one frame every {}. First {} devices per chain; {} omitted.", &[format!("{}", __omatainer_args.0), format!("{}", __omatainer_args.1), format!("{}", __omatainer_args.2), format!("{}", __omatainer_args.3)]) });
        ui.label(tr!("Wall ns per sampled frame; estimated buffer share assumes that frame repeats. Track/scene totals include their devices; do not add them. Scheduling and timer overhead are included. Not per-track CPU or worst-case headroom."));
        let details = egui::CollapsingHeader::new("Track and device samples")
            .default_open(false)
            .show(ui, |ui| {
                egui::Grid::new("diagnostic-costs")
                    .striped(true)
                    .show(ui, |ui| {
                        for cost in &profile.costs {
                            ui.label(cost.label());
                            ui.label(if cost.elapsed_ns == 0 {
                                tr!("Below timer resolution").into()
                            } else {
                                { let __omatainer_args = (&(cost.elapsed_ns),); crate::localization::format("{} ns/frame", &[format!("{}", __omatainer_args.0)]) }
                            });
                            ui.label({ let __omatainer_args = (&(cost.elapsed_ns as f64 * profile.sample_rate as f64 / 1e7),); crate::localization::format("{:.3}% estimated", &[format!("{:.3}", __omatainer_args.0)]) });
                            ui.end_row();
                        }
                    });
            });
        help::annotate(ui, &details.header_response, help::Control::Diagnostics);
    } else {
        ui.label(tr!("Sampled track/device load awaiting audio (profiling is enabled while this window or a capture is active)"));
    }
}
