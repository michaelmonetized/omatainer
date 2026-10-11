use super::*;
use crate::engine::performance::{Error, Safety};

#[derive(Clone, Copy)]
enum Decision {
    Leave,
    Stop,
    Silence,
    Recover,
}
#[derive(Default)]
pub(super) struct Panel {
    decision: Option<Decision>,
    inputs_released: bool,
}
impl Panel {
    /// Open the explicit emergency-silence decision from any workspace window.
    /// Takes no arguments; returns no value and does not silence audio before confirmation.
    pub(super) fn silence(&mut self) { self.decision=Some(Decision::Silence); }
}
impl App {
    /// Refuse before invalidating a pending load/receipt or spawning a decoder.
    /// The renderer repeats the check against its actual current deck state.
    pub(super) fn performance_allows(&self, command: &Command) -> bool {
        match self.engine.cmd.performance().check(command, None) {
            Ok(()) => true,
            Err(reason) => {
                self.engine.cmd.performance().reject(reason);
                self.submission_error
                    .set(Some(crate::engine::SubmissionError::Performance(reason)));
                false
            }
        }
    }
    pub(super) fn performance_ui(&mut self, ctx: &egui::Context) { self.performance_controls(ctx, None); }
    pub(super) fn performance_controls(&mut self, ctx: &egui::Context, parent: Option<&mut Ui>) {
        let status = self.engine.cmd.performance().status();
        let compact = ctx.screen_rect().width() < 600.0 || ctx.screen_rect().height() < 320.0;
        toolbar(ctx, parent, "show-protection", if status.output_muted { if compact { "Muted" } else { "Output muted" } } else if status.recovery { if compact { "Paused" } else { "Controls paused" } } else if compact { "Live" } else { "Performance" }, |ui| {
            ui.vertical(|ui| {
                ui.strong(if status.recovery { "Controls paused" } else if status.protected { "Performance mode on" } else { "Performance mode off" });
                if ui.button(if status.protected { tr!("Turn off performance mode…") } else { tr!("Turn on performance mode") }).help(ui, HelpControl::PerformanceMode).clicked() {
                    if status.protected { self.performance_panel.decision = Some(Decision::Leave); }
                    else { self.submit(Command::PerformanceMode(true)); }
                }
                if ui.button(tr!("Stop everything…")).help(ui, HelpControl::PerformanceStop).clicked() { self.performance_panel.decision = Some(Decision::Stop); ui.close(); }
                if ui.button(tr!("Mute output…")).help(ui, HelpControl::PerformanceSilence).clicked() { self.performance_panel.decision = Some(Decision::Silence); }
                if status.recovery && ui.button(tr!("Resume controls…")).help(ui, HelpControl::PerformanceRecovery).clicked() {
                    self.performance_panel.inputs_released = false;
                    self.performance_panel.decision = Some(Decision::Recover);
                    ui.close();
                }
            });
            if status.protected {
                ui.label(tr!("Protects playing decks and device settings. Mixing, recording and saving stay available."));
            }
            if status.changing { ui.label(tr!("Finish the current project or device change first.")); }
            if status.recovery && !status.stopped { ui.label(tr!("Stopping audio…")); }
            if status.output_muted {
                ui.colored_label(ui.visuals().warn_fg_color, tr!("Output is muted. Reset audio to unmute; playback stays stopped."));
                ui.collapsing(tr!("Audio details"), |ui| {
                if status.observation_complete {
                    ui.label(if status.nonfinite_tail { tr!("2 s observation complete: nonfinite tail detected. DSP reset required.") }
                        else { tr!("2 s tail observation complete. Quiet samples do not prove delayed/bypassed history is cleared.") });
                } else { ui.label(tr!("Observing natural tails for at most 2 s; quiet threshold −80 dBFS. Emergency mute remains latched.")); }
                });
                // The audio-owner implementation is available in normal desktop
                // sessions; headless/offline sessions retain mute and allow Save.
                self.performance_reset_button(ui);
            }
        });
        self.performance_decision(ctx);
    }

    /// Render the shared safety decision in the native window that owns input.
    /// Takes its context; returns no value and uses ordinary safety admission.
    pub(super) fn performance_decision(&mut self,ctx:&egui::Context) {
        let status = self.engine.cmd.performance().status();
        let Some(decision) = self.performance_panel.decision else {
            return;
        };
        keyboard::block_for_dialog(ctx);
        let mut close = false;
        egui::Window::new(crate::localization::text_dynamic(match decision { Decision::Leave => "Turn off performance mode…", Decision::Stop => "Stop everything…", Decision::Silence => "Mute output…", Decision::Recover => "Resume controls…" })).default_width(380.0).id(egui::Id::new("performance-decision")).collapsible(false).resizable(false).show(ctx, |ui| {
            let (text, action) = match decision {
                Decision::Leave => ("Turn off performance mode? Playing decks and device settings can be changed again. Muted output stays muted.", "Turn off performance mode"),
                Decision::Stop => ("Stop both decks and the session, finish recording and release held notes. Effect tails may continue. Release your controls before using them again.", "Stop everything"),
                Decision::Silence => ("Stop playback and recording, release held notes and mute output. Reset audio when you are ready to unmute.", "Mute output"),
                Decision::Recover => ("Release all keys, pads and platter touches, then resume controls. Playback stays stopped. Muted output stays muted.", "Resume controls"),
            };
            ui.label(crate::localization::text_dynamic(text));
            if matches!(decision, Decision::Recover) {
                ui.checkbox(&mut self.performance_panel.inputs_released, tr!("All controls are released")).help(ui, HelpControl::PerformanceRecovery);
            }
            let enabled = !matches!(decision, Decision::Recover) || (self.performance_panel.inputs_released && status.stopped);
            if ui.add_enabled(enabled, egui::Button::new(crate::localization::text_dynamic(action))).help(ui, HelpControl::PerformanceRecovery).clicked() {
                let command = match decision { Decision::Leave => Command::PerformanceMode(false), Decision::Stop => Command::SafetyStop(Safety::Stop), Decision::Silence => Command::SafetyStop(Safety::Silence), Decision::Recover => Command::RecoverPerformance };
                if self.submit(command) { close = true; }
            }
            if ui.button(tr!("Cancel")).help(ui, HelpControl::PerformanceCancel).clicked() { close = true; }
        });
        if close {
            self.performance_panel.decision = None;
        }
    }
    pub(super) fn reject_protected_project(&mut self) -> bool {
        if self.engine.cmd.performance().protected() {
            self.project_performance_message(Error::Protected.to_string());
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests;
