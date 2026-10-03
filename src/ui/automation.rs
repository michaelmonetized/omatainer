use super::*;
use crate::automation::{Expected, Key as SessionKey, ObjectId, Target};
use serde_json::json;

pub(super) struct Panel {
    pub open: bool,
    endpoint: Option<PathBuf>,
    gain: f32,
    beat: f64,
    name: String,
    job: String,
    result: String,
    show_token: bool,
}
impl Default for Panel {
    fn default() -> Self {
        Self {
            open: false,
            endpoint: None,
            gain: 1.0,
            beat: 1.0,
            name: String::new(),
            job: String::new(),
            result: String::new(),
            show_token: false,
        }
    }
}

impl App {
    /// Show the actual local service path.
    /// Takes the bound desktop socket path; stores it for the automation inspector.
    pub(crate) fn automation_endpoint(&mut self, path: PathBuf) {
        self.automation_panel.endpoint = Some(path);
    }

    /// Apply saved optional network intent.
    /// Takes the current app profile; queues worker configuration and reports admission errors.
    pub(super) fn apply_automation(&mut self) {
        if self.engine.safe_mode() {
            return;
        }
        if let Err(error) = self
            .automation_network
            .configure(self.settings.profile().automation)
        {
            self.settings
                .message
                .push_str(&format!(" Saved OSC settings are not applied: {error}"));
        }
    }

    /// Inspect and exercise the versioned API with native widgets.
    /// Takes the frame context; submits only explicitly clicked typed operations and shows their real receipts.
    pub(super) fn automation_ui(&mut self, ctx: &egui::Context) {
        if !self.automation_panel.open {
            return;
        }
        keyboard::block_for_dialog(ctx);
        let network = self.automation_network.status();
        let mut open = true;
        let mut request = None;
        let panel = &mut self.automation_panel;
        egui::Window::new("Automation and remote control")
            .id(egui::Id::new("automation-window"))
            .open(&mut open)
            .default_width(660.0)
            .vscroll(true)
            .hscroll(true)
            .max_height(self.theme.window_height(ctx))
            .show(ctx, |ui| {
                ui.heading("Automation API v1");
                if let Some(path) = &panel.endpoint {
                    ui.label(format!("Local socket: {}", path.display()));
                } else {
                    ui.label("The desktop launch publishes its local socket path here.");
                }
                ui.label("Linux local control requires the same effective user and socket permissions 0600.");
                ui.label(
                    "Commands return pending jobs. Refresh the job to see applied, rejected or cancelled.",
                );
                if ui
                    .button("Read API state")
                    .help(ui, HelpControl::AutomationState)
                    .clicked()
                {
                    request = Some(json!({"op":"state"}));
                }
                if let Some(layout) = &self.snap.session {
                    ui.label(format!(
                        "Session: {} · revision {}",
                        String::from(SessionKey(layout.namespace)),
                        self.snap.project_revision
                    ));
                    ui.label(format!("Current quarter-note beat: {:.4}", self.snap.beat));
                    if let Some(item) = layout
                        .tracks
                        .get(self.snap.selected_track)
                        .filter(|item| item.active)
                    {
                        let target = Target {
                            namespace: SessionKey(layout.namespace),
                            axis: crate::automation::Axis::Track,
                            id: ObjectId(item.id.0),
                        };
                        ui.label(format!(
                            "Selected track: {} · id {}",
                            item.name,
                            String::from(ObjectId(item.id.0))
                        ));
                        ui.add(egui::Slider::new(&mut panel.gain, 0.0..=1.5).text("Scheduled gain"))
                            .help(ui, HelpControl::AutomationGain);
                        ui.horizontal(|ui| {
                            let label = ui.label("At quarter-note beat");
                            let response = ui
                                .add(
                                    egui::DragValue::new(&mut panel.beat)
                                        .speed(0.25)
                                        .range(0.0..=f64::MAX),
                                )
                                .labelled_by(label.id);
                            help::annotate(ui, &response, HelpControl::AutomationBeat);
                            if ui
                                .button("Next beat")
                                .help(ui, HelpControl::AutomationBeat)
                                .clicked()
                            {
                                panel.beat = self.snap.beat.ceil() + 1.0;
                            }
                        });
                        if ui
                            .add_enabled(
                                self.snap.playing,
                                egui::Button::new("Schedule selected track gain"),
                            )
                            .help(ui, HelpControl::AutomationSchedule)
                            .clicked()
                        {
                            request = Some(
                                json!({"op":"schedule","namespace":SessionKey(layout.namespace),"beat":panel.beat,"action":{"op":"track_gain","target":target,"value":panel.gain}}),
                            );
                        }
                        let label = ui.label("API track name");
                        let response = ui
                            .add(egui::TextEdit::singleline(&mut panel.name).char_limit(1024))
                            .labelled_by(label.id);
                        ui.ctx()
                            .accesskit_node_builder(response.id, |node| node.set_label("API track name"));
                        help::annotate(ui, &response, HelpControl::AutomationRename);
                        if ui
                            .button("Apply API track name")
                            .help(ui, HelpControl::AutomationRename)
                            .clicked()
                        {
                            let expected = Expected {
                                namespace: SessionKey(layout.namespace),
                                generation: crate::automation::Count(layout.generation),
                                revision: crate::automation::Count(self.snap.project_revision),
                            };
                            request = Some(
                                json!({"op":"edit","expected":expected,"target":target,"action":{"op":"rename","name":panel.name}}),
                            );
                        }
                    }
                }
                ui.horizontal_wrapped(|ui| {
                    if ui
                        .add_enabled(!panel.job.is_empty(), egui::Button::new("Refresh API job"))
                        .help(ui, HelpControl::AutomationJob)
                        .clicked()
                    {
                        request = Some(json!({"op":"job","id":panel.job}));
                    }
                    if ui
                        .add_enabled(!panel.job.is_empty(), egui::Button::new("Cancel API job"))
                        .help(ui, HelpControl::AutomationCancel)
                        .clicked()
                    {
                        request = Some(json!({"op":"cancel","id":panel.job}));
                    }
                });
                if !panel.result.is_empty() {
                    ui.label(&panel.result);
                }
                ui.separator();
                ui.heading("Optional loopback OSC");
                if network.pending {
                    ui.label("Listener configuration is pending");
                    ctx.request_repaint_after(std::time::Duration::from_millis(20));
                }
                if let Some(port) = network.port {
                    ui.label(format!("Listening: 127.0.0.1:{port} · /omatainer/v1 ,sb"));
                    ui.checkbox(&mut panel.show_token, "Show OSC access token")
                        .help(ui, HelpControl::AutomationToken);
                    if panel.show_token {
                        if let Some(token) = &network.token {
                            ui.label(format!("Access token: {token}"));
                        }
                    }
                } else {
                    ui.label("OSC is disabled");
                }
                if let Some(error) = &network.error {
                    ui.colored_label(ui.visuals().warn_fg_color, error);
                }
                ui.label("OSC takes the current token and a UTF-8 API JSON blob. Subscriptions use the Unix stream. Enable or change the loopback listener in Preferences.");
                if ui
                    .button("Preferences")
                    .help(ui, HelpControl::Preferences)
                    .clicked()
                {
                    self.settings.open = true;
                }
            });
        self.automation_panel.open = open;
        if let Some(request) = request {
            let envelope =
                json!({"op":"api","version":1,"id":"native-inspector","request":request});
            let (response, _) = crate::automation::reply(
                &envelope,
                &self.engine.cmd,
                &self.engine.snap,
                crate::ipc_transport::Limits::default(),
                false,
            );
            if let Some(job) = response["result"]["job"].as_str() {
                self.automation_panel.job = job.to_owned();
            }
            self.automation_panel.result = serde_json::to_string_pretty(&response).unwrap();
        }
    }
}

#[cfg(test)]
mod tests;
