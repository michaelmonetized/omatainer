//! Native aliases, explicit taps and reviewed routing changes.
use super::*;
use crate::engine::{
    audio::routing::{input, model::*},
    midi_edit::{Ack, Outcome},
    session::Layout,
};
use std::sync::atomic::{AtomicBool, Ordering};
mod worker;
use worker::{Event, Job, Worker};

#[derive(Clone)]
struct Draft {
    namespace: [u64; 2],
    generation: u64,
    revision: u64,
    rate: u32,
    layout: Layout,
    enabled: bool,
    model: Model,
}
#[derive(Clone)]
struct InputPreview {
    saved: InputConfig,
    plan: crate::engine::audio::config::Plan,
    output_generation: u64,
}
#[derive(Default)]
pub(super) struct Panel {
    pub open: bool,
    worker: Option<Worker>,
    cancel: Option<Arc<AtomicBool>>,
    pending: Option<Ack>,
    draft: Option<Draft>,
    input_preview: Option<InputPreview>,
    confirm: bool,
    message: String,
    error: Option<String>,
    record_alias: u64,
    record_seconds: u32,
    record_path: String,
}
impl Drop for Panel {
    fn drop(&mut self) {
        self.cancel();
    }
}
impl Panel {
    /// Read pending routing work.
    /// Takes this panel; returns whether a worker or commit is active.
    fn busy(&self) -> bool {
        self.cancel.is_some() || self.pending.is_some()
    }
    /// Cancel pending routing work.
    /// Takes this panel; signals its worker and pending commit without undoing completed work.
    fn cancel(&self) {
        if let Some(cancel) = &self.cancel {
            cancel.store(true, Ordering::Release);
        }
        if let Some(ack) = &self.pending {
            ack.cancel();
        }
    }
    /// Receive routing results.
    /// Takes this panel; updates the draft, errors and honest commit status.
    fn poll(&mut self) {
        let event = self.worker.as_ref().map(|worker| worker.events.try_recv());
        if matches!(
            event,
            Some(Err(crossbeam_channel::TryRecvError::Disconnected))
        ) {
            self.cancel();
            self.cancel = None;
            self.pending = None;
            self.worker = None;
            self.error = Some("Routing worker closed before completion. Refresh to retry.".into());
        }
        if let Some(Ok(event)) = event {
            let cancelled = self
                .cancel
                .take()
                .is_some_and(|cancel| cancel.load(Ordering::Acquire));
            match event {
                Event::Inspected(result) => {
                    match result {
                        Ok(draft) if !cancelled => {
                            self.draft = Some(draft);
                            self.error = None;
                            self.message = "Routes inspected. Draft edits affect audio only after confirmation.".into();
                        }
                        Ok(_) => self.message = "Inspection cancelled".into(),
                        Err(error) => self.error = Some(error),
                    }
                }
                Event::Applied(result) => match result {
                    Ok(ack) => {
                        if cancelled {
                            ack.cancel();
                        }
                        self.pending = Some(ack);
                    }
                    Err(error) => self.error = Some(error),
                },
                Event::InputPreview(result) => match result {
                    Ok(preview) if !cancelled => {
                        self.input_preview = Some(preview);
                        self.error = None;
                    }
                    Ok(_) => self.message = "Input preview cancelled".into(),
                    Err(error) => self.error = Some(error),
                },
                Event::InputChanged(result) => match result {
                    Ok(message) => {
                        self.message = message;
                        self.error = None;
                        self.input_preview = None;
                    }
                    Err(error) => self.error = Some(error),
                },
                Event::Recorded(result) => match result {
                    Ok(path) => {
                        self.message = format!("Record-source audio saved to {}", path.display());
                        self.error = None;
                    }
                    Err(error) => self.error = Some(error),
                },
            }
        }
        if let Some(ack) = &self.pending {
            match ack.state() {
                Outcome::Pending => {}
                Outcome::Applied => {
                    self.pending = None;
                    self.draft = None;
                    self.message = "Routing applied. Save stores aliases, taps and input choices; History can undo this edit.".into();
                    self.error = None;
                }
                Outcome::Cancelled => {
                    self.pending = None;
                    self.message = "Routing cancelled before application".into();
                }
                Outcome::Rejected => {
                    self.pending = None;
                    self.error = Some("Routing was not applied because the project, recording, protection or undo state changed. Refresh and retry.".into());
                }
            }
        }
    }
    /// Submit one routing operation.
    /// Takes the engine and job constructor; starts a bounded worker and reports refused work.
    fn request(&mut self, engine: &Engine, make: impl FnOnce(Arc<AtomicBool>) -> Job) {
        if self.busy() {
            return;
        }
        let cancel = Arc::new(AtomicBool::new(false));
        let result = (|| {
            if self.worker.is_none() {
                self.worker = Some(Worker::start(
                    engine.project.clone(),
                    engine.cmd.clone(),
                    engine.audio_handle(),
                    engine.input_handle(),
                    engine.routing.recorder.clone(),
                )?);
            }
            self.worker
                .as_ref()
                .unwrap()
                .jobs
                .try_send(make(cancel.clone()))
                .map_err(|_| "Routing worker is busy".to_string())
        })();
        match result {
            Ok(()) => {
                self.cancel = Some(cancel);
                self.error = None;
            }
            Err(error) => self.error = Some(error),
        }
    }
    /// Prepare a reviewed routing edit.
    /// Takes the engine; submits the captured draft under the performance guard.
    fn apply(&mut self, engine: &Engine) {
        let Some(draft) = self.draft.clone() else {
            return;
        };
        match engine.cmd.performance().optional_work() {
            Ok(work) => {
                let cancel = work.cancel();
                self.request(engine, |_| Job::Apply(draft, work));
                if self.cancel.is_some() {
                    self.cancel = Some(cancel);
                }
            }
            Err(error) => self.error = Some(error.to_string()),
        }
    }
    /// Prepare an explicit record capture.
    /// Takes the engine; validates the destination and submits the selected alias on a worker.
    fn record(&mut self, engine: &Engine) {
        let Some(draft) = self.draft.clone() else {
            return;
        };
        let alias = self.record_alias;
        let seconds = self.record_seconds;
        let path = PathBuf::from(self.record_path.trim());
        if path.as_os_str().is_empty() {
            self.error = Some("Choose a new WAV file path".into());
            return;
        }
        match engine.cmd.performance().optional_work() {
            Ok(work) => {
                let cancel = work.cancel();
                self.request(engine, |_| Job::Record(draft, alias, seconds, path, work));
                if self.cancel.is_some() {
                    self.cancel = Some(cancel);
                }
            }
            Err(error) => self.error = Some(error.to_string()),
        }
    }
}

/// Name one retained endpoint.
/// Takes its identity, model and layout; returns the saved alias or missing identity.
fn endpoint(group: Group, model: &Model, layout: &Layout) -> String {
    match group {
        Group::Input(id) | Group::Output(id) | Group::Record(id) => model
            .ports
            .iter()
            .find(|port| port.id == id)
            .map_or_else(|| format!("Missing alias {id}"), |port| port.alias.clone()),
        Group::Bus(id) => model
            .buses
            .iter()
            .find(|bus| bus.id == id)
            .map_or_else(|| format!("Missing bus {id}"), |bus| bus.alias.clone()),
        Group::Track(id) => layout.tracks.iter().find(|item| item.id == id).map_or_else(
            || format!("Retired track {}", id.0),
            |item| format!("Track: {}", item.name),
        ),
        Group::Scene(id) => layout.scenes.iter().find(|item| item.id == id).map_or_else(
            || format!("Retired scene {}", id.0),
            |item| format!("Scene FX: {}", item.name),
        ),
        Group::Deck(0) => "Deck A".into(),
        Group::Deck(_) => "Deck B".into(),
        Group::Main => "Main mixer".into(),
    }
}
/// List selectable endpoints.
/// Takes the saved model, layout and source direction; returns stable endpoint identities.
fn endpoints(model: &Model, layout: &Layout, source: bool) -> Vec<Group> {
    let mut groups = vec![Group::Main];
    groups.extend(
        layout
            .tracks
            .iter()
            .filter(|item| item.active)
            .map(|item| Group::Track(item.id)),
    );
    groups.extend(
        layout
            .scenes
            .iter()
            .filter(|item| item.active)
            .map(|item| Group::Scene(item.id)),
    );
    if source {
        groups.extend([Group::Deck(0), Group::Deck(1)]);
    }
    groups.extend(model.buses.iter().map(|bus| Group::Bus(bus.id)));
    groups.extend(
        model
            .ports
            .iter()
            .filter_map(|port| match (source, port.direction) {
                (true, Direction::Input) => Some(Group::Input(port.id)),
                (false, Direction::Output) => Some(Group::Output(port.id)),
                (false, Direction::Record) => Some(Group::Record(port.id)),
                _ => None,
            }),
    );
    groups
}
/// Choose an endpoint.
/// Takes the selected identity, available groups and names; changes only the draft.
fn choose(
    ui: &mut Ui,
    label: &str,
    group: &mut Group,
    choices: &[Group],
    model: &Model,
    layout: &Layout,
) {
    egui::ComboBox::from_id_salt(label)
        .selected_text(endpoint(*group, model, layout))
        .show_ui(ui, |ui| {
            for choice in choices {
                ui.selectable_value(group, *choice, endpoint(*choice, model, layout));
            }
        });
}
/// Name a port direction.
/// Takes its direction; returns the displayed label.
fn direction_name(direction: Direction) -> &'static str {
    match direction {
        Direction::Input => "Input",
        Direction::Output => "Output",
        Direction::Record => "Record source",
    }
}
/// Remove one alias and its maps.
/// Takes the saved draft and identity; removes dependent connections together.
fn remove_alias(model: &mut Model, id: u64) {
    model.ports.retain(|port| port.id != id);
    model.buses.retain(|bus| bus.id != id);
    model.connections.retain(|connection| !matches!(connection.source.group, Group::Input(value) | Group::Bus(value) if value == id)
        && !matches!(connection.destination, Group::Output(value) | Group::Record(value) | Group::Bus(value) if value == id));
}

/// Edit one routing value through pointer, keyboard or native accessibility.
/// Takes its displayed name, value, bounds and step; returns a bounded changed value.
fn number(ui: &mut Ui, label: &str, mut value: f32, min: f32, max: f32, step: f32) -> Option<f32> {
    let response = ui.add(
        egui::DragValue::new(&mut value)
            .range(min..=max)
            .speed(step)
            .prefix(format!("{label} ")),
    );
    let accessible = accessibility::numeric(ui, &response, label, value, min, max, step, "");
    accessible.or_else(|| response.changed().then_some(value))
}

/// Name a saved source tap.
/// Takes its position; returns the displayed plain language label.
fn tap_name(tap: Tap) -> &'static str {
    match tap {
        Tap::PreFx => tr!("Pre FX"),
        Tap::PostFx => tr!("Post FX"),
        Tap::PostMixer => tr!("Post mixer"),
    }
}

/// Edit bounded channel aliases.
/// Takes the UI and draft; edits ports and buses without changing live audio.
fn aliases(ui: &mut Ui, model: &mut Model) {
    let mut remove = None;
    for port in &mut model.ports {
        ui.push_id(("alias", port.id), |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(direction_name(port.direction));
                ui.add(
                    egui::TextEdit::singleline(&mut port.alias)
                        .char_limit(128)
                        .desired_width(180.0)
                        .hint_text("Channel alias"),
                );
                let max = if port.direction == Direction::Record {
                    MAX_RECORD_CHANNELS
                } else {
                    MAX_PORT_CHANNELS
                };
                if let Some(width) = number(
                    ui,
                    &format!("Alias {} channels", port.id),
                    port.channels.len() as f32,
                    1.0,
                    max as f32,
                    1.0,
                ) {
                    port.channels = (0..width as u16).collect();
                }
                for (index, channel) in port.channels.iter_mut().enumerate() {
                    ui.add_enabled_ui(port.direction != Direction::Record, |ui| {
                        if let Some(value) = number(
                            ui,
                            &format!("Alias {} physical channel {}", port.id, index + 1),
                            f32::from(*channel) + 1.0,
                            1.0,
                            MAX_PHYSICAL_CHANNELS as f32,
                            1.0,
                        ) {
                            *channel = value as u16 - 1;
                        }
                    });
                }
                if ui.button(tr!("Remove alias")).clicked() {
                    remove = Some(port.id);
                }
            });
        });
    }
    for bus in &mut model.buses {
        ui.push_id(("bus", bus.id), |ui| {
            ui.horizontal(|ui| {
                ui.label(tr!("Bus"));
                ui.add(
                    egui::TextEdit::singleline(&mut bus.alias)
                        .char_limit(128)
                        .desired_width(180.0),
                );
                ui.add(
                    egui::DragValue::new(&mut bus.channels)
                        .range(1..=MAX_PORT_CHANNELS as u8)
                        .prefix("Channels "),
                );
                ui.add(
                    egui::DragValue::new(&mut bus.gain)
                        .range(0.0..=1.5)
                        .speed(0.01)
                        .prefix("Gain "),
                );
                ui.checkbox(&mut bus.mute, tr!("Mute"));
                if ui.button(tr!("Remove bus")).clicked() {
                    remove = Some(bus.id);
                }
            });
        });
    }
    if let Some(id) = remove {
        remove_alias(model, id);
    }
    ui.horizontal_wrapped(|ui| {
        for direction in [Direction::Input, Direction::Output, Direction::Record] {
            if ui
                .add_enabled(
                    model
                        .ports
                        .iter()
                        .filter(|port| port.direction == direction)
                        .count()
                        < MAX_PORTS
                        && model.next_id < u64::MAX,
                    egui::Button::new(format!("Add {} alias", direction_name(direction))),
                )
                .clicked()
            {
                let id = model.next_id;
                model.next_id += 1;
                model.ports.push(Port {
                    id,
                    alias: format!("{} {id}", direction_name(direction)),
                    direction,
                    channels: vec![0, 1],
                });
            }
        }
        if ui
            .add_enabled(
                model.buses.len() < MAX_BUSES && model.next_id < u64::MAX,
                egui::Button::new(tr!("Add bus")),
            )
            .clicked()
        {
            let id = model.next_id;
            model.next_id += 1;
            model.buses.push(Bus {
                id,
                alias: format!("Bus {id}"),
                channels: 2,
                gain: 1.0,
                mute: false,
            });
        }
    });
}
/// Edit explicit channel maps.
/// Takes the UI, draft and retained layout; changes bounded routes and source taps.
fn connections(ui: &mut Ui, model: &mut Model, layout: &Layout) {
    let sources = endpoints(model, layout, true);
    let destinations = endpoints(model, layout, false);
    let labels = model.clone();
    let mut remove = None;
    for (index, connection) in model.connections.iter_mut().enumerate() {
        ui.push_id(("connection", index), |ui| {
            ui.horizontal_wrapped(|ui| {
                choose(
                    ui,
                    "Source",
                    &mut connection.source.group,
                    &sources,
                    &labels,
                    layout,
                );
                egui::ComboBox::from_id_salt("Tap")
                    .selected_text(tap_name(connection.source.tap))
                    .show_ui(ui, |ui| {
                        for (tap, label) in [
                            (Tap::PreFx, "Pre FX"),
                            (Tap::PostFx, "Post FX"),
                            (Tap::PostMixer, "Post mixer"),
                        ] {
                            ui.selectable_value(&mut connection.source.tap, tap, label);
                        }
                    });
                ui.label(tr!("→"));
                choose(
                    ui,
                    "Destination",
                    &mut connection.destination,
                    &destinations,
                    &labels,
                    layout,
                );
                if ui.button(tr!("Remove route")).clicked() {
                    remove = Some(index);
                }
            });
            let source_width = labels.width(connection.source.group, layout).unwrap_or(1);
            let destination_width = labels.width(connection.destination, layout).unwrap_or(1);
            let mut remove_map = None;
            for (map_index, map) in connection.map.iter_mut().enumerate() {
                ui.push_id(map_index, |ui| {
                    ui.horizontal(|ui| {
                        let mut from = usize::from(map.source) + 1;
                        let mut to = usize::from(map.destination) + 1;
                        if ui
                            .add(
                                egui::DragValue::new(&mut from)
                                    .range(1..=source_width)
                                    .prefix("Source channel "),
                            )
                            .changed()
                        {
                            map.source = (from - 1) as u8;
                        }
                        if ui
                            .add(
                                egui::DragValue::new(&mut to)
                                    .range(1..=destination_width)
                                    .prefix("Destination channel "),
                            )
                            .changed()
                        {
                            map.destination = (to - 1) as u8;
                        }
                        ui.add(
                            egui::DragValue::new(&mut map.gain)
                                .range(-1.0..=1.0)
                                .speed(0.01)
                                .prefix("Map gain "),
                        );
                        if ui.button(tr!("Remove map")).clicked() {
                            remove_map = Some(map_index);
                        }
                    });
                });
            }
            if let Some(index) = remove_map {
                connection.map.remove(index);
            }
            if ui
                .add_enabled(
                    connection.map.len() < MAX_MAPS,
                    egui::Button::new(tr!("Add channel map")),
                )
                .clicked()
            {
                connection.map.push(ChannelMap {
                    source: 0,
                    destination: 0,
                    gain: 1.0,
                });
            }
            ui.separator();
        });
    }
    if let Some(index) = remove {
        model.connections.remove(index);
    }
    if ui
        .add_enabled(
            model.connections.len() < MAX_CONNECTIONS && !destinations.is_empty(),
            egui::Button::new(tr!("Add route")),
        )
        .clicked()
    {
        model.connections.push(Connection {
            source: Source {
                group: Group::Main,
                tap: Tap::PostMixer,
            },
            destination: destinations[0],
            map: vec![ChannelMap {
                source: 0,
                destination: 0,
                gain: 1.0,
            }],
        });
    }
}

impl App {
    pub(super) fn audio_routing_ui(&mut self, ctx: &egui::Context) {
        self.audio_routing.poll();
        if !self.audio_routing.open {
            return;
        }
        keyboard::block_for_dialog(ctx);
        let panel = &mut self.audio_routing;
        let mut open = true;
        egui::Window::new(tr!("Audio routing")).id(egui::Id::new("audio-routing-window")).open(&mut open).default_width(850.0).default_height(650.0).vscroll(true).show(ctx, |ui| {
            ui.label(tr!("Aliases retain exact physical channels. Missing channels stay silent. Drafts require confirmation; routes save with the project."));
            ui.label(&panel.message);
            if let Some(error) = &panel.error { ui.colored_label(ui.visuals().warn_fg_color, error); }
            if panel.busy() { if ui.button(tr!("Cancel routing operation")).clicked() { panel.cancel(); } }
            let ready = !panel.busy() && !self.project.committing();
            let mut preview_input = None;
            let mut record = false;
            ui.add_enabled_ui(ready, |ui| {
                if ui.button(tr!("Refresh routes")).help(ui, HelpControl::AudioRouting).clicked() { panel.request(&self.engine, Job::Inspect); }
                if let Some(draft) = &mut panel.draft {
                    ui.checkbox(&mut draft.enabled, tr!("Use explicit routing"));
                    if draft.enabled {
                        ui.collapsing(tr!("Channel aliases and buses"), |ui| aliases(ui, &mut draft.model));
                        ui.collapsing(tr!("Default sends"), |ui| {
                            for track in draft.layout.tracks.iter().filter(|item| item.active) {
                                let mut send = !draft.model.tracks_without_default_send.contains(&track.id);
                                if ui.checkbox(&mut send, format!("{} → scene FX → main", track.name)).changed() {
                                    draft.model.tracks_without_default_send.retain(|id| *id != track.id);
                                    if !send { draft.model.tracks_without_default_send.push(track.id); }
                                }
                            }
                            for (deck, disabled) in draft.model.decks_without_default_send.iter_mut().enumerate() {
                                let mut send = !*disabled;
                                if ui.checkbox(&mut send, if deck == 0 { "Deck A → main" } else { "Deck B → main" }).changed() { *disabled = !send; }
                            }
                        });
                        ui.collapsing(tr!("Routes and tap positions"), |ui| connections(ui, &mut draft.model, &draft.layout));
                        ui.label(tr!("Track pre FX includes instruments and mapped input; post mixer includes mute, solo, gain and pan. Deck pre FX is the source; post FX includes deck gain/EQ/filter and transition; post mixer adds crossfader gain."));
                        ui.label(tr!("Stereo performance source attribution is unavailable while explicit routing is active. Playlist events remain available."));
                        ui.collapsing(tr!("Live input choices"), |ui| {
                            let mut configured = draft.model.input.is_some();
                            if ui.checkbox(&mut configured, tr!("Save input choice")).changed() {
                                draft.model.input = configured.then(|| InputConfig { backend: self.engine.output_info().map_or_else(|| "ALSA".into(), |value| value.backend), device: String::new(), channels: 2, format: crate::preferences::AudioFormat::F32, buffer_frames: None });
                                panel.input_preview = None;
                            }
                            if let Some(saved) = &mut draft.model.input {
                                ui.horizontal(|ui| { ui.label(tr!("Exact backend")); ui.text_edit_singleline(&mut saved.backend); });
                                ui.horizontal(|ui| { ui.label(tr!("Exact input device")); ui.text_edit_singleline(&mut saved.device); });
                                ui.add(egui::DragValue::new(&mut saved.channels).range(1..=64).prefix("Input channels "));
                                egui::ComboBox::from_id_salt("input-format").selected_text(format!("{:?}", saved.format)).show_ui(ui, |ui| { for format in crate::preferences::AudioFormat::ALL { ui.selectable_value(&mut saved.format, format, format!("{format:?}")); } });
                                let mut requested = saved.buffer_frames.is_some();
                                if ui.checkbox(&mut requested, tr!("Request input buffer size")).changed() { saved.buffer_frames = requested.then_some(512); panel.input_preview = None; }
                                if let Some(frames) = &mut saved.buffer_frames {
                                    if let Some(value) = number(ui, "Input buffer frames", *frames as f32, 1.0, 2730.0, 1.0) { *frames = value.round() as u32; panel.input_preview = None; }
                                }
                                if ui.button(tr!("Preview input")).help(ui, HelpControl::AudioRoutingInput).clicked() { preview_input = Some(saved.clone()); }
                            }
                        });
                        ui.collapsing(tr!("Capture a record source"), |ui| {
                            egui::ComboBox::from_id_salt("record-source-alias").selected_text(draft.model.port(panel.record_alias, Direction::Record).map_or("Choose a record source", |port| port.alias.as_str())).show_ui(ui, |ui| {
                                for port in draft.model.ports.iter().filter(|port| port.direction == Direction::Record) { ui.selectable_value(&mut panel.record_alias, port.id, &port.alias); }
                            });
                            ui.horizontal(|ui| { ui.label(tr!("New WAV file path")); ui.text_edit_singleline(&mut panel.record_path); });
                            if panel.record_seconds == 0 { panel.record_seconds = 30; }
                            ui.add(egui::DragValue::new(&mut panel.record_seconds).range(1..=600).prefix("Maximum seconds "));
                            ui.label(tr!("Apply routes before capture. A worker writes up to 26 channels and 128 MiB. Stop keeps completed audio; Cancel discards the partial file. Existing files are preserved."));
                            if ui.button(tr!("Record source to WAV")).help(ui, HelpControl::AudioRoutingRecord).clicked() { record = true; }
                        });
                    }
                    if ui.button(tr!("Review routing change…")).clicked() { panel.confirm = true; }
                }
            });
            if let Some(saved) = preview_input { panel.request(&self.engine, |cancel| Job::PreviewInput(saved, cancel)); }
            if record { panel.record(&self.engine); }
            if self.engine.routing.recorder.alias() != 0 {
                ui.label(format!("Record-source frames captured: {}", self.engine.routing.recorder.frames()));
                if ui.button(tr!("Stop record-source capture")).clicked() { self.engine.routing.recorder.stop(); }
            }
            if let Some(handle) = self.engine.input_handle() {
                let status = handle.status(); ui.label(&status.message);
                ui.label(format!("Captured input frames: {} · missing frames: {} · queue overflow: {}", handle.shared().captured.load(Ordering::Relaxed), handle.shared().underrun.load(Ordering::Relaxed), handle.shared().overflow.load(Ordering::Relaxed)));
                let cushion = handle.shared().cushion.load(Ordering::Relaxed);
                let rate = status.active.as_ref().map_or(0, |plan| plan.rate);
                let milliseconds = if rate == 0 { 0.0 } else { f64::from(cushion) * 1000.0 / f64::from(rate) };
                ui.label(format!("Input cushion target: {cushion} frames ({milliseconds:.3} ms nominal) · startup silence: {} frames · source discontinuities: {}", handle.shared().priming.load(Ordering::Relaxed), handle.shared().discontinuities.load(Ordering::Relaxed)));
                ui.collapsing(tr!("Physical channel meters"), |ui| {
                    let channels = self.engine.output_info().map_or(0, |output| usize::from(output.plan.channels));
                    for channel in 0..channels.min(MAX_PHYSICAL_CHANNELS) { ui.horizontal(|ui| {
                        ui.label(format!("Output {}: {:.5}", channel + 1, f32::from_bits(handle.shared().output[channel].load(Ordering::Relaxed))));
                        if ui.button(format!("Test output {} at −40 dBFS", channel + 1)).clicked() { if let Err(error) = self.engine.test_output(channel as u16) { panel.error = Some(error); } }
                    }); }
                    for channel in 0..status.active.as_ref().map_or(0, |input| usize::from(input.channels)) { ui.label(format!("Input {}: {:.5}", channel + 1, f32::from_bits(handle.shared().input[channel].load(Ordering::Relaxed)))); }
                });
                if self.engine.routing.shared.probe.load(Ordering::Acquire) != 0 && ui.button(tr!("Cancel channel test")).clicked() { self.engine.routing.shared.probe.store(0, Ordering::Release); }
                if ready && status.active.is_some() && ui.button(tr!("Stop and disable input")).clicked() {
                    if let Some(output) = self.engine.audio_handle() { let generation = output.status().generation; panel.request(&self.engine, |cancel| Job::DisableInput(generation, cancel)); }
                }
            }
        });
        panel.open = open;
        if !open {
            panel.cancel();
            panel.confirm = false;
        }
        if panel.confirm && !panel.busy() {
            egui::Window::new(tr!("Confirm routing change")).id(egui::Id::new("confirm-routing")).collapsible(false).show(ctx, |ui| {
                ui.label(tr!("Apply this draft to the current project? Cycles, invalid maps and stale project state reject the entire change. Stop playback before changing routes."));
                if ui.add_enabled(!self.snap.playing && !self.snap.recording && !self.snap.decks.iter().any(|deck| deck.playing || deck.touching), egui::Button::new(tr!("Apply routing"))).clicked() { panel.confirm = false; panel.apply(&self.engine); }
                if ui.button(tr!("Cancel routing change")).clicked() { panel.confirm = false; }
            });
        }
        if let Some(preview) = panel.input_preview.clone().filter(|preview| {
            panel
                .draft
                .as_ref()
                .and_then(|draft| draft.model.input.as_ref())
                == Some(&preview.saved)
        }) {
            egui::Window::new(tr!("Confirm live input")).id(egui::Id::new("confirm-routing-input")).collapsible(false).show(ctx, |ui| {
                ui.label(format!("{} / {} · {} channels · {} Hz · {}. Input may feed any saved route. Enabling stops playback; reconnect requires another confirmation.", preview.plan.backend, preview.plan.device, preview.plan.channels, preview.plan.rate, preview.plan.format));
                ui.label(format!("Requested input buffer: {}", preview.plan.buffer.map_or_else(|| "backend selected".into(), |frames| format!("{frames} frames"))));
                ui.label(tr!("Live input waits for two actual callback blocks. Larger buffers add latency; independent device clocks can still drift. The running cushion target and gaps remain visible."));
                if let Some(warning) = &preview.plan.warning { ui.label(warning); }
                if ui.add_enabled(!panel.busy(), egui::Button::new(tr!("Stop and enable input"))).clicked() { panel.request(&self.engine, |cancel| Job::EnableInput(preview, cancel)); }
                if ui.button(tr!("Cancel input preview")).clicked() { panel.input_preview = None; }
            });
        }
    }
}

#[cfg(test)]
mod tests;
