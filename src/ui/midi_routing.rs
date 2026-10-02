//! Draft routing edits use the existing Preferences Preview/Cancel/Save flow.
use super::*;
use crate::engine::midi::routing::{Endpoint, Filter, Input, Route, Routing, Status};

fn text(ui: &mut Ui, label: &str, value: &mut String) {
    ui.label(label);
    let response = ui.add(
        egui::TextEdit::singleline(value)
            .char_limit(1024)
            .desired_width(360.0),
    );
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, response.enabled(), label)
    });
    help::annotate(ui, &response, HelpControl::MidiRoutePort);
}
fn port(ui: &mut Ui, label: &str, value: &mut Endpoint, choices: &[Endpoint]) {
    ui.push_id(label, |ui| {
        let combo = egui::ComboBox::from_label(format!("Choose {label}"))
            .selected_text(&value.name)
            .show_ui(ui, |ui| {
                for choice in choices {
                    let display = format!(
                        "{} [{}]",
                        choice.name,
                        choice.id.as_deref().unwrap_or("no backend id")
                    );
                    if ui.selectable_label(value == choice, &display).clicked() {
                        *value = choice.clone();
                    }
                }
            });
        help::annotate(ui, &combo.response, HelpControl::MidiRoutePort);
        text(ui, &format!("{label} exact name"), &mut value.name);
        let mut exact = value.id.is_some();
        if ui
            .checkbox(
                &mut exact,
                format!("{label}: require exact backend port id"),
            )
            .help(ui, HelpControl::MidiRoutePort)
            .changed()
        {
            value.id = exact.then(String::new);
        }
        if let Some(id) = &mut value.id {
            text(ui, &format!("{label} exact id"), id);
        }
    });
}
fn channel(ui: &mut Ui, label: &str, value: &mut Option<u8>) {
    let mut preserve = value.is_none();
    if ui
        .checkbox(&mut preserve, format!("{label}: preserve source channel"))
        .help(ui, HelpControl::MidiRouteChannel)
        .changed()
    {
        *value = (!preserve).then_some(0);
    }
    if let Some(channel) = value {
        let mut shown = f32::from(*channel) + 1.0;
        let response = ui.add(
            egui::DragValue::new(&mut shown)
                .range(1.0..=16.0)
                .speed(1.0)
                .prefix(format!("{label}: ")),
        );
        if let Some(next) = accessibility::numeric(ui, &response, label, shown, 1.0, 16.0, 1.0, "")
        {
            shown = next;
        }
        *channel = shown.round().clamp(1.0, 16.0) as u8 - 1;
        help::annotate(ui, &response, HelpControl::MidiRouteChannel);
    }
}
pub(super) fn edit(ui: &mut Ui, routing: &mut Routing, status: Option<&Status>) {
    ui.heading("Track MIDI routing");
    ui.checkbox(&mut routing.enabled, "Use explicit track MIDI routing")
        .help(ui, HelpControl::MidiRouteEnable);
    ui.label("Routes are saved with this profile. Input policy above must also permit each chosen input. Internal monitoring plays notes; other live message types go to the external output when Live thru is enabled. External clip output plays source MIDI; audio faders, mute, solo and the internal arpeggiator affect the internal sound.");
    ui.label("Feedback guard forbids configured or active inputs on an output device/client. Select controller inputs explicitly and deselect the output device in Input policy. Check physical cable/thru loops separately. Missing or ambiguous destinations stay unavailable.");
    let inputs = status.map_or(&[][..], |s| s.inputs.as_slice());
    let outputs = status.map_or(&[][..], |s| s.outputs.as_slice());
    ui.add_enabled_ui(routing.enabled, |ui| {
        for t in 0..TRACKS {
            let mut enabled = routing.routes.iter().any(|r| usize::from(r.track) == t);
            if ui
                .checkbox(&mut enabled, format!("Route MIDI track {}", t + 1))
                .help(ui, HelpControl::MidiRouteTrack)
                .changed()
            {
                if enabled {
                    routing.routes.push(Route {
                        track: t as u8,
                        inputs: Vec::new(),
                        output: None,
                        output_channel: None,
                        monitor: true,
                        thru: false,
                        filter: Filter::default(),
                    });
                } else {
                    routing.routes.retain(|r| usize::from(r.track) != t);
                }
            }
            let Some(route) = routing
                .routes
                .iter_mut()
                .find(|r| usize::from(r.track) == t)
            else {
                continue;
            };
            egui::CollapsingHeader::new(format!(
                "Track {} MIDI ports, channels and filters",
                t + 1
            ))
            .id_salt(("midi-route", t))
            .show(ui, |ui| {
                ui.checkbox(
                    &mut route.monitor,
                    format!("Track {}: monitor notes internally", t + 1),
                )
                .help(ui, HelpControl::MidiRouteMonitor);
                ui.checkbox(
                    &mut route.thru,
                    format!("Track {}: Live thru to external output", t + 1),
                )
                .help(ui, HelpControl::MidiRouteThru);
                let mut external = route.output.is_some();
                if ui
                    .checkbox(
                        &mut external,
                        format!("Track {}: external MIDI output", t + 1),
                    )
                    .help(ui, HelpControl::MidiRoutePort)
                    .changed()
                {
                    route.output = external.then(|| Endpoint {
                        name: String::new(),
                        id: None,
                    });
                    if !external {
                        route.thru = false;
                    }
                }
                if let Some(output) = &mut route.output {
                    port(ui, &format!("Track {} output", t + 1), output, outputs);
                    channel(
                        ui,
                        &format!("Track {} output channel", t + 1),
                        &mut route.output_channel,
                    );
                }
                let mut remove = None;
                for (n, input) in route.inputs.iter_mut().enumerate() {
                    egui::CollapsingHeader::new(format!("Track {} input {}", t + 1, n + 1))
                        .id_salt((t, n))
                        .show(ui, |ui| {
                            port(
                                ui,
                                &format!("Track {} input {}", t + 1, n + 1),
                                &mut input.port,
                                inputs,
                            );
                            let mut all = input.channels == u16::MAX;
                            if ui
                                .checkbox(
                                    &mut all,
                                    format!("Track {} input {}: all channels", t + 1, n + 1),
                                )
                                .help(ui, HelpControl::MidiRouteChannel)
                                .changed()
                            {
                                input.channels = if all { u16::MAX } else { 1 };
                            }
                            if !all {
                                ui.horizontal_wrapped(|ui| {
                                    for ch in 0..16 {
                                        let mut on = input.channels & (1 << ch) != 0;
                                        if ui
                                            .checkbox(
                                                &mut on,
                                                format!("T{} I{} channel {}", t + 1, n + 1, ch + 1),
                                            )
                                            .help(ui, HelpControl::MidiRouteChannel)
                                            .changed()
                                        {
                                            if on {
                                                input.channels |= 1 << ch;
                                            } else {
                                                input.channels &= !(1 << ch);
                                            }
                                        }
                                    }
                                });
                            }
                            if ui
                                .button(format!("Remove track {} input {}", t + 1, n + 1))
                                .help(ui, HelpControl::MidiRoutePort)
                                .clicked()
                            {
                                remove = Some(n);
                            }
                        });
                }
                if let Some(n) = remove {
                    route.inputs.remove(n);
                }
                if ui
                    .add_enabled(
                        route.inputs.len() < 8,
                        egui::Button::new(format!("Add track {} MIDI input", t + 1)),
                    )
                    .help(ui, HelpControl::MidiRoutePort)
                    .clicked()
                {
                    route.inputs.push(Input {
                        port: Endpoint {
                            name: String::new(),
                            id: None,
                        },
                        channels: u16::MAX,
                    });
                }
                ui.horizontal_wrapped(|ui| {
                    for (label, value) in [
                        ("Notes", &mut route.filter.notes),
                        ("CC", &mut route.filter.cc),
                        ("Bank CC0/32", &mut route.filter.bank),
                        ("Program", &mut route.filter.program),
                        ("Pressure", &mut route.filter.pressure),
                        ("Pitch bend", &mut route.filter.bend),
                        ("Complete SysEx ≤256 bytes", &mut route.filter.sysex),
                    ] {
                        ui.checkbox(value, format!("Track {}: {label}", t + 1))
                            .help(ui, HelpControl::MidiRouteFilter);
                    }
                });
            });
        }
    });
    if let Err(error) = routing.validate() {
        ui.colored_label(Color32::YELLOW, error);
    }
}
impl App {
    pub(super) fn midi_routing_status_ui(&mut self, ui: &mut Ui, ctx: &egui::Context) {
        ui.separator();
        ui.heading("Track routing and output activity");
        if let Some(status) = self.engine.midi.routing_status() {
            ui.label(format!(
                "Routing request {} · applied {}",
                status.requested_generation, status.applied_generation
            ));
            if status.pending {
                ui.label("Routing change pending; backend operations finish on their worker.");
                ctx.request_repaint_after(std::time::Duration::from_millis(50));
                if ui
                    .button("Cancel pending MIDI routing")
                    .help(ui, HelpControl::MidiRouteCancel)
                    .clicked()
                {
                    self.engine.midi.cancel_routing();
                }
            }
            if let Some(error) = &status.error {
                ui.colored_label(Color32::YELLOW, error);
            }
        } else {
            ui.label("MIDI routing/output owner unavailable in this session.");
        }
        let (tracks, global) = self.engine.cmd.midi_routing().activity();
        for (t, c) in tracks
            .iter()
            .enumerate()
            .filter(|(_, c)| c.received > 0 || c.sent > 0 || c.failed > 0 || c.clip_refused)
        {
            ui.label(format!("Track {}: {} input · {} routed · {} filtered/merged · {} sent · {} failed · {} overruns · clip refused {} · last {:02X} channel {}",t+1,c.received,c.routed,c.filtered,c.sent,c.failed,c.overruns,c.clip_refused,c.last_status,c.last_channel));
        }
        ui.label(format!("{} invalid/unsupported packets", global.malformed));
        ui.label("An explicit reset or output queue/block overrun resets outputs and stops that clip’s external stream until relaunch, MIDI edit, or routing apply. Sent counts mean the backend accepted packets; physical delivery and timing need controller QA.");
        if ui
            .button("All notes off / reset MIDI outputs")
            .help(ui, HelpControl::MidiRouteReset)
            .clicked()
        {
            self.engine.cmd.midi_routing().reset_outputs();
        }
        if ui
            .button("Edit MIDI routing in Preferences")
            .help(ui, HelpControl::MidiRouteEnable)
            .clicked()
        {
            self.settings.open = true;
        }
    }
}
