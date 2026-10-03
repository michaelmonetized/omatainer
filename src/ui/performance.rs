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
        toolbar(ctx, parent, "show-protection", if status.output_muted { "MUTE" } else if status.recovery { "STOP" } else if status.protected { "Guard" } else { "Safety" }, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.strong(if status.recovery { "RECOVERY · playback stopped" } else if status.protected { "PERFORMANCE · protected" } else { "Studio · protection off" });
                if ui.button(if status.protected { tr!("Leave performance mode…") } else { tr!("Enable performance mode") }).help(ui, HelpControl::PerformanceMode).clicked() {
                    if status.protected { self.performance_panel.decision = Some(Decision::Leave); }
                    else { self.submit(Command::PerformanceMode(true)); }
                }
                if ui.button(tr!("Safe stop…")).help(ui, HelpControl::PerformanceStop).clicked() { self.performance_panel.decision = Some(Decision::Stop); }
                if ui.button(tr!("Emergency silence…")).help(ui, HelpControl::PerformanceSilence).clicked() { self.performance_panel.decision = Some(Decision::Silence); }
                if status.recovery && ui.button(tr!("Recover inputs…")).help(ui, HelpControl::PerformanceRecovery).clicked() {
                    self.performance_panel.inputs_released = false;
                    self.performance_panel.decision = Some(Decision::Recover);
                }
            });
            if status.protected {
                ui.label(tr!("Playing/touched deck replacement, destructive edits, project replacement and device changes are protected. Mixing, recording, stopped-deck loads and project Save remain available. Optional background work is deferred."));
            }
            if status.changing { ui.label(tr!("A deliberate project/device change is pending; mode entry waits for its completion or cancellation.")); }
            if status.recovery && !status.stopped { ui.label(tr!("Safety request pending renderer acknowledgment; do not infer silence yet.")); }
            if status.output_muted {
                ui.colored_label(ui.visuals().warn_fg_color, tr!("EMERGENCY OUTPUT MUTE · remains muted after input recovery. Deliberately reset stopped DSP before unmuting; no playback resumes automatically."));
                if status.observation_complete {
                    ui.label(if status.nonfinite_tail { tr!("2 s observation complete: nonfinite tail detected. DSP reset required.") }
                        else { tr!("2 s tail observation complete. Quiet samples do not prove delayed/bypassed history is cleared.") });
                } else { ui.label(tr!("Observing natural tails for at most 2 s; quiet threshold −80 dBFS. Emergency mute remains latched.")); }
                // The audio-owner implementation is available in normal desktop
                // sessions; headless/offline sessions retain mute and allow Save.
                self.performance_reset_button(ui);
            }
        });
        let Some(decision) = self.performance_panel.decision else {
            return;
        };
        keyboard::block_for_dialog(ctx);
        let mut close = false;
        egui::Window::new(tr!("Performance safety decision")).id(egui::Id::new("performance-decision")).collapsible(false).resizable(false).show(ctx, |ui| {
            let (text, action) = match decision {
                Decision::Leave => ("Leave protection deliberately. Playing deck replacement, destructive edits and optional background work will become available. Emergency mute, if present, stays latched.", "Leave protection"),
                Decision::Stop => ("Stop session and both decks, finalize recorded holds, disarm compose and release all input-owned synth gates. Finite hits and effect tails continue naturally. Recovery needs your explicit input-release acknowledgment.", "Stop all transports and release notes"),
                Decision::Silence => ("Finalize captured holds, stop session and both decks, release all notes and ramp output to silence over 2 ms. Output stays muted until a deliberate stopped DSP reset. This does not claim the device or physical controls are healthy.", "Confirm emergency silence"),
                Decision::Recover => ("Release physical keys, pads and platter touch controls first. Your confirmation is a user report, not a hardware check. This only reopens controls after queued pre-stop work drains; playback stays stopped and emergency output mute stays latched.", if status.output_muted { "Inputs released — keep output muted" } else { "Inputs released — keep playback stopped" }),
            };
            ui.label(crate::localization::text_dynamic(text));
            if matches!(decision, Decision::Recover) {
                ui.checkbox(&mut self.performance_panel.inputs_released, tr!("I have released the physical inputs")).help(ui, HelpControl::PerformanceRecovery);
            }
            let enabled = !matches!(decision, Decision::Recover) || (self.performance_panel.inputs_released && status.stopped);
            if ui.add_enabled(enabled, egui::Button::new(crate::localization::text_dynamic(action))).help(ui, HelpControl::PerformanceRecovery).clicked() {
                let command = match decision { Decision::Leave => Command::PerformanceMode(false), Decision::Stop => Command::SafetyStop(Safety::Stop), Decision::Silence => Command::SafetyStop(Safety::Silence), Decision::Recover => Command::RecoverPerformance };
                if self.submit(command) { close = true; }
            }
            if ui.button(tr!("Keep current safety state")).help(ui, HelpControl::PerformanceCancel).clicked() { close = true; }
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
