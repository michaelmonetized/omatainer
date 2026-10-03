use super::*;

impl App {
    pub(super) fn audio_status(&mut self, ctx: &egui::Context) { self.setup_controls(ctx, None); }
    pub(super) fn setup_controls(&mut self, ctx: &egui::Context, parent: Option<&mut Ui>) {
        let metrics = self.snap.audio;
        toolbar(ctx, parent, "audio-status", "Setup", |ui| {
            ui.horizontal_wrapped(|ui| {
                accessibility::action_button(ui);
                if ui.button(tr!("Help")).help(ui, HelpControl::Help).clicked() { self.keys_open = true; }
                if ui.button(tr!("MIDI")).help(ui, HelpControl::Midi).clicked() { self.midi_open = true; }
                if ui.button(tr!("Diagnostics")).help(ui, HelpControl::Diagnostics).clicked() { self.diagnostics.open = true; }
                if ui.button(tr!("Content & licenses")).help(ui, HelpControl::License).clicked() { self.licenses.open = true; }
                if ui.button(tr!("Music providers")).help_detail(ui, HelpControl::MusicProvider, "Browse licensed remote music with explicit provider capabilities.").clicked() { self.music_provider.open = true; }
                if ui.button(tr!("Video")).help(ui, HelpControl::VideoImport).clicked() { self.video.open = true; }
                if ui.button(tr!("Automation")).help(ui, HelpControl::AutomationOpen).clicked() { self.automation_panel.open = true; }
                if ui.button(tr!("Preferences")).help(ui, HelpControl::Preferences).clicked() { self.settings.open = true; }
                if !self.settings.open && !self.settings.message.is_empty() {
                    let label = if self.settings.message.starts_with("Preferences failed") { "Preferences failed" } else { "Preferences update" };
                    if ui.button(label).help_detail(ui, HelpControl::Preferences, &self.settings.message).clicked() { self.settings.open = true; }
                }
                if self.settings.pending_restart() && ui.button(tr!("Saved audio pending")).help(ui, HelpControl::AudioDevices).clicked() {self.audio_settings.open=true;}
                if !self.audio_settings.open {if let Some((label,message))=self.audio_settings.notice(){if ui.button(label).help_detail(ui, HelpControl::AudioNotice, &message).clicked(){self.audio_settings.open=true;}}}
                if let Some(notice) = &self.settings.startup_notice {
                    if ui.button(tr!("Setup notice")).help_detail(ui, HelpControl::Preferences, notice).clicked() { self.settings.open = true; }
                }
                if let Some(sample) = metrics.last_callback {
                    let cpu = sample.render_cpu_fraction().map(|value| format!("{:.1}%", value * 100.0))
                        .unwrap_or_else(|| "unavailable".into());
                    ui.label(crate::localization::format("Render CPU: {cpu}", &[format!("{}", cpu)]))
                        .on_hover_text(tr!("Rendering thread CPU time / buffer duration. Excludes control handling, conversion and snapshot publication. Wall-clock waits are not CPU time."));
                    ui.label({ let __omatainer_args = (&(sample.elapsed_ns as f64 / 1e6),&(sample.budget_ns as f64 / 1e6),); crate::localization::format("Callback elapsed: {:.2} ms / {:.2} ms budget", &[format!("{:.2}", __omatainer_args.0), format!("{:.2}", __omatainer_args.1)]) })
                        .on_hover_text(tr!("Last completed output callback: command handoff, rendering, channel/sample conversion and snapshot handoff. Includes scheduling delays and waits."));
                } else {
                    ui.label(tr!("Render CPU: unavailable"));
                    ui.label(tr!("Callback elapsed: awaiting measurement"));
                }
                ui.label({ let __omatainer_args = (&(metrics.deadline_overruns),); crate::localization::format("Deadline overruns: {}", &[format!("{}", __omatainer_args.0)]) })
                    .on_hover_text({ let __omatainer_args = (&(metrics.max_elapsed_ns as f64 / 1e6),&(metrics.max_overrun_ns as f64 / 1e6),); crate::localization::format("Callbacks longer than their buffer duration. Longest callback: {:.2} ms; greatest overrun: {:.2} ms. These count service-time overruns, not measured hardware dropouts.", &[format!("{:.2}", __omatainer_args.0), format!("{:.2}", __omatainer_args.1)]) });
                ui.label({ let __omatainer_args = (&(metrics.backend_errors),&(metrics.device_lost),); crate::localization::format("Audio errors: {} · device lost: {}", &[format!("{}", __omatainer_args.0), format!("{}", __omatainer_args.1)]) });
                ui.label(tr!("Dropped buffers: unavailable"))
                    .on_hover_text(tr!("The current CPAL backend does not expose an exact dropped-buffer counter. Overruns and backend errors are reported separately."));
            });
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn visible_telemetry_distinguishes_cpu_wall_deadlines_and_unknown_drops() {
        let mut fixture = test_support::Fixture::new(32);
        fixture.app.snap.audio = crate::engine::audio_metrics::AudioMetrics {
            last_callback: Some(crate::engine::audio_metrics::CallbackMeasurement {
                elapsed_ns: 20_000_000,
                budget_ns: 10_000_000,
                render_cpu_ns: Some(1_000_000),
                overrun_ns: 10_000_000,
                ..Default::default()
            }),
            deadline_overruns: 3,
            backend_errors: 1,
            ..Default::default()
        };
        let ctx = egui::Context::default();
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1440.0, 900.0))),
                ..Default::default()
            },
            |ctx| fixture.app.audio_status(ctx),
        );
        for expected in [
            "Render CPU: 10.0%",
            "Callback elapsed: 20.00 ms / 10.00 ms budget",
            "Deadline overruns: 3",
            "Audio errors: 1",
            "Dropped buffers: unavailable",
        ] {
            assert!(
                output.shapes.iter().any(|shape| matches!(&shape.shape,
                egui::epaint::Shape::Text(text) if text.galley.text().contains(expected))),
                "missing {expected}"
            );
        }
    }
}
