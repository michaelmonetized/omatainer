use super::*;
use crate::engine::audio::routing::mixer::{self, Lane, Parameter, Send, Target};

/// Edit bounded mixer automation on the routing draft.
/// Takes a stable UI owner and lanes; updates points for the existing reviewed, undoable graph transaction.
fn lanes(ui: &mut Ui, lanes: &mut Vec<Lane>, returns: bool) {
    ui.collapsing("Automation in song beats", |ui| {
        let mut remove = None;
        for (index, lane) in lanes.iter_mut().enumerate() {
            ui.push_id(index, |ui| {
                ui.label(format!("{:?}", lane.parameter));
                let mut point_remove = None;
                for (point_index, point) in lane.points.iter_mut().enumerate() {
                    ui.push_id(point_index, |ui| {
                        ui.horizontal(|ui| {
                            if let Some(value) =
                                number(ui, "Beat", point[0] as f32, 0., 1.0e9, 0.01)
                            {
                                point[0] = f64::from(value);
                            }
                            let (min, max) = match lane.parameter {
                                Parameter::Level => (0., 1.5),
                                Parameter::Pan => (-1., 1.),
                                _ => (0., 1.),
                            };
                            if let Some(value) =
                                number(ui, "Value", point[1] as f32, min, max, 0.01)
                            {
                                point[1] = f64::from(value);
                            }
                            if ui.button("Remove point").clicked() {
                                point_remove = Some(point_index);
                            }
                        })
                    });
                }
                if let Some(index) = point_remove {
                    lane.points.remove(index);
                }
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(
                            lane.points.len() < mixer::MAX_LANE_POINTS,
                            egui::Button::new("Add point"),
                        )
                        .clicked()
                    {
                        let point = lane.points.last().copied().unwrap_or([0., 0.]);
                        lane.points.push([point[0] + 1., point[1]]);
                    }
                    if ui.button("Remove lane").clicked() {
                        remove = Some(index);
                    }
                });
            });
        }
        if let Some(index) = remove {
            lanes.remove(index);
        }
        ui.horizontal_wrapped(|ui| {
            for parameter in [
                Parameter::Level,
                Parameter::Pan,
                Parameter::Mute,
                Parameter::Solo,
                Parameter::SoloSafe,
            ] {
                if !returns && matches!(parameter, Parameter::Solo | Parameter::SoloSafe) {
                    continue;
                }
                if ui
                    .add_enabled(
                        !lanes.iter().any(|l| l.parameter == parameter),
                        egui::Button::new(format!("Automate {parameter:?}")),
                    )
                    .clicked()
                {
                    lanes.push(Lane {
                        parameter,
                        points: vec![[
                            0.,
                            if parameter == Parameter::Level {
                                1.
                            } else {
                                0.
                            },
                        ]],
                    });
                }
            }
        });
    });
}

/// Author named sends and returns without changing active audio.
/// Takes the retained routing draft; updates explicit routes, taps and automation for its normal review and Undo path.
pub(super) fn draft(ui: &mut Ui, draft: &mut Draft) {
    ui.label("Pre FX and post FX sends bypass source mute/solo/fader/crossfader. Post mixer includes those controls. Return mute affects its post-mixer tap; pre taps keep running. Soloed returns and their downstream paths are heard at terminal sums; feeders keep processing. Solo safe returns remain audible. Active song automation owns its parameter while playing.");
    let sources = endpoints(&draft.model, &draft.layout, true);
    let destinations = endpoints(&draft.model, &draft.layout, false);
    let model = draft.model.clone();
    let mut remove = None;
    for send in &mut draft.model.sends {
        ui.push_id(("named-send", send.id), |ui| {
            ui.separator();
            ui.horizontal(|ui| {
                ui.label(format!("Send {}", send.id));
                ui.add(egui::TextEdit::singleline(&mut send.name).char_limit(128));
                if ui.button("Remove send").clicked() {
                    remove = Some(send.id);
                }
            });
            choose(
                ui,
                "Send source",
                &mut send.route.source.group,
                &sources,
                &model,
                &draft.layout,
            );
            egui::ComboBox::from_id_salt("send-tap")
                .selected_text(tap_name(send.route.source.tap))
                .show_ui(ui, |ui| {
                    for tap in [Tap::PreFx, Tap::PostFx, Tap::PostMixer] {
                        ui.selectable_value(&mut send.route.source.tap, tap, tap_name(tap));
                    }
                });
            choose(
                ui,
                "Send destination",
                &mut send.route.destination,
                &destinations,
                &model,
                &draft.layout,
            );
            if let Some(level) = number(ui, "Saved send level", send.level, 0., 1.5, 0.01) {
                send.level = level;
            }
            if let Some(pan) = number(ui, "Saved send pan", send.pan, -1., 1., 0.01) {
                send.pan = pan;
            }
            ui.checkbox(&mut send.mute, "Saved send mute");
            let source = model
                .source_width(send.route.source, &draft.layout)
                .unwrap_or(1);
            let destination = model
                .input_width(send.route.destination, &draft.layout)
                .unwrap_or(1);
            let mut map_remove = None;
            for (index, map) in send.route.map.iter_mut().enumerate() {
                ui.push_id(index, |ui| {
                    ui.horizontal(|ui| {
                        if let Some(value) = number(
                            ui,
                            "From channel",
                            f32::from(map.source) + 1.,
                            1.,
                            source as f32,
                            1.,
                        ) {
                            map.source = value as u8 - 1;
                        }
                        if let Some(value) = number(
                            ui,
                            "To channel",
                            f32::from(map.destination) + 1.,
                            1.,
                            destination as f32,
                            1.,
                        ) {
                            map.destination = value as u8 - 1;
                        }
                        if let Some(value) = number(ui, "Map gain", map.gain, -1., 1., 0.01) {
                            map.gain = value;
                        }
                        if ui.button("Remove channel map").clicked() {
                            map_remove = Some(index);
                        }
                    })
                });
            }
            if let Some(index) = map_remove {
                send.route.map.remove(index);
            }
            if ui
                .add_enabled(
                    send.route.map.len() < MAX_MAPS,
                    egui::Button::new("Add channel map"),
                )
                .clicked()
            {
                send.route.map.push(ChannelMap {
                    source: 0,
                    destination: 0,
                    gain: 1.,
                });
            }
            lanes(ui, &mut send.automation, false);
        });
    }
    if let Some(id) = remove {
        draft.model.sends.retain(|s| s.id != id);
    }
    for bus in &mut draft.model.buses {
        ui.push_id(("return-extra", bus.id), |ui| {
            ui.label(format!("Return {} · {}", bus.id, bus.alias));
            if let Some(pan) = number(ui, "Saved return pan", bus.mixer.pan, -1., 1., 0.01) {
                bus.mixer.pan = pan;
            }
            ui.checkbox(&mut bus.mixer.solo, "Saved return solo");
            ui.checkbox(&mut bus.mixer.solo_safe, "Saved return solo safe");
            lanes(ui, &mut bus.mixer.automation, true);
        });
    }
    ui.horizontal_wrapped(|ui| {
        if ui
            .add_enabled(
                draft.model.buses.len() < MAX_BUSES
                    && draft.model.connections.len() + draft.model.sends.len() < MAX_CONNECTIONS
                    && draft.model.next_id < u64::MAX,
                egui::Button::new("Add named stereo return"),
            )
            .clicked()
        {
            let id = draft.model.next_id;
            draft.model.next_id += 1;
            draft.model.buses.push(Bus {
                id,
                alias: format!("Return {id}"),
                channels: 2,
                gain: 1.,
                mute: false,
                mixer: Default::default(),
            });
            draft.model.connections.push(Connection {
                source: Source {
                    group: Group::Bus(id),
                    tap: Tap::PostMixer,
                },
                destination: Group::Main,
                map: vec![
                    ChannelMap {
                        source: 0,
                        destination: 0,
                        gain: 1.,
                    },
                    ChannelMap {
                        source: 1,
                        destination: 1,
                        gain: 1.,
                    },
                ],
            });
        }
        if ui
            .add_enabled(
                draft.model.connections.len() + draft.model.sends.len() < MAX_CONNECTIONS
                    && !draft.model.buses.is_empty()
                    && draft.model.next_id < u64::MAX,
                egui::Button::new("Add named send"),
            )
            .clicked()
        {
            let id = draft.model.next_id;
            draft.model.next_id += 1;
            let source = draft
                .layout
                .tracks
                .iter()
                .find(|t| t.active)
                .map(|t| Group::Track(t.id))
                .unwrap_or(Group::Deck(0));
            let bus = &draft.model.buses[0];
            draft.model.sends.push(Send {
                id,
                name: format!("Send {id}"),
                route: Connection {
                    source: Source {
                        group: source,
                        tap: Tap::PostMixer,
                    },
                    destination: Group::Bus(bus.id),
                    map: (0..usize::from(bus.channels).min(2))
                        .map(|channel| ChannelMap {
                            source: channel as u8,
                            destination: channel as u8,
                            gain: 1.,
                        })
                        .collect(),
                },
                level: 0.,
                pan: 0.,
                mute: false,
                automation: vec![],
            });
        }
    });
}

/// Control and meter the active saved sends and returns.
/// Takes renderer-confirmed controls and typed command admission; submits named keyboard-accessible edits through existing gesture history.
pub(super) fn live(
    ui: &mut Ui,
    snapshot: &Snapshot,
    commands: &crate::engine::CommandPort,
    history: &undo::History,
    model: Option<&Model>,
) {
    let Some(layout) = &snapshot.session else {
        return;
    };
    ui.collapsing("Track fixed sends and crossfader assignments", |ui| {
        for (slot, track) in snapshot
            .tracks
            .iter()
            .enumerate()
            .filter(|(slot, _)| layout.tracks.get(*slot).is_some_and(|t| t.active))
        {
            ui.push_id(("track-fixed-mix", layout.tracks[slot].id.0), |ui| {
                ui.label(&track.name);
                let submit =
                    |ui: &mut Ui, command| match crate::engine::session::Scoped::qualify_layout(
                        history.wrap(command),
                        layout,
                    ) {
                        Ok(command) => {
                            if let Err(error) = commands.send(command) {
                                ui.colored_label(ui.visuals().warn_fg_color, error.to_string());
                            }
                        }
                        Err(_) => {
                            ui.colored_label(
                                ui.visuals().warn_fg_color,
                                "Track identity changed; refresh the mixer",
                            );
                        }
                    };
                ui.horizontal_wrapped(|ui| {
                    for (send, label) in [(0, "Reverb send"), (1, "Echo send")] {
                        if let Some(value) = number(
                            ui,
                            &format!("{} {label}", track.name),
                            track.surface.sends[send],
                            0.,
                            1.,
                            0.01,
                        ) {
                            submit(
                                ui,
                                Command::TrackSend {
                                    track: slot as u8,
                                    send: send as u8,
                                    value,
                                },
                            );
                        }
                    }
                    let mut assignment = track.surface.crossfader;
                    let response = egui::ComboBox::from_id_salt("track-crossfader")
                        .selected_text(
                            ["Crossfader unassigned", "Crossfader A", "Crossfader B"]
                                [usize::from(assignment)],
                        )
                        .show_ui(ui, |ui| {
                            for (id, label) in ["Unassigned", "A", "B"].into_iter().enumerate() {
                                ui.selectable_value(&mut assignment, id as u8, label);
                            }
                        })
                        .response;
                    accessibility::focus(ui, &response);
                    if assignment != track.surface.crossfader {
                        submit(
                            ui,
                            Command::TrackCrossfader {
                                track: slot as u8,
                                assignment,
                            },
                        );
                    }
                });
            });
        }
        for (index, label) in ["Fixed reverb return", "Fixed echo return"]
            .into_iter()
            .enumerate()
        {
            let peaks = snapshot.surfaces.return_peaks[index];
            ui.label(format!(
                "{label} peak L {:.5} · R {:.5}",
                peaks[0], peaks[1]
            ));
        }
    });
    for channel in &snapshot.routing_mixer {
        ui.push_id(
            ("live-routing-mix", format!("{:?}", channel.target)),
            |ui| {
                let name = model
                    .and_then(|m| match channel.target {
                        Target::Send(id) => {
                            m.sends.iter().find(|s| s.id == id).map(|s| s.name.as_str())
                        }
                        Target::Return(id) => m
                            .buses
                            .iter()
                            .find(|b| b.id == id)
                            .map(|b| b.alias.as_str()),
                    })
                    .map(str::to_owned)
                    .unwrap_or_else(|| format!("{:?}", channel.target));
                ui.label(&name);
                let submit = |ui: &mut Ui, parameter, value| {
                    if let Err(error) = commands.send(history.wrap(Command::RoutingMix {
                        namespace: layout.namespace,
                        target: channel.target,
                        parameter,
                        value,
                    })) {
                        ui.colored_label(ui.visuals().warn_fg_color, error.to_string());
                    }
                };
                ui.horizontal_wrapped(|ui| {
                    if let Some(value) = number(
                        ui,
                        &format!("{name} level"),
                        channel.values.level,
                        0.,
                        1.5,
                        0.01,
                    ) {
                        submit(ui, Parameter::Level, value);
                    }
                    if let Some(value) = number(
                        ui,
                        &format!("{name} pan"),
                        channel.values.pan,
                        -1.,
                        1.,
                        0.01,
                    ) {
                        submit(ui, Parameter::Pan, value);
                    }
                    let mut mute = channel.values.mute;
                    if ui.checkbox(&mut mute, format!("{name} mute")).changed() {
                        submit(ui, Parameter::Mute, if mute { 1. } else { 0. });
                    }
                    if matches!(channel.target, Target::Return(_)) {
                        let mut solo = channel.values.solo;
                        if ui.checkbox(&mut solo, format!("{name} solo")).changed() {
                            submit(ui, Parameter::Solo, if solo { 1. } else { 0. });
                        }
                        let mut safe = channel.values.solo_safe;
                        if ui
                            .checkbox(&mut safe, format!("{name} solo safe"))
                            .changed()
                        {
                            submit(ui, Parameter::SoloSafe, if safe { 1. } else { 0. });
                        }
                    }
                });
                ui.label(format!(
                    "Peak L {:.5} · R {:.5} · active level {:.3}",
                    channel.peaks[0], channel.peaks[1], channel.effective.level
                ));
            },
        );
    }
}
