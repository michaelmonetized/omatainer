use super::*;
use crate::engine::audio::routing::speakers::{BassManagement, Layout as SpeakerLayout};

/// Edit one declared speaker choice using the native keyboard route.
/// Takes a stable field name and layout; returns the selected canonical layout without changing live audio.
fn choice(
    ui: &mut Ui,
    key: impl std::hash::Hash,
    label: &str,
    current: SpeakerLayout,
    mono: bool,
) -> SpeakerLayout {
    let mut layout = current;
    let response = egui::ComboBox::new(key, label)
        .selected_text(current.label())
        .show_ui(ui, |ui| {
            for &value in SpeakerLayout::ALL
                .iter()
                .filter(|value| mono || value.channels() > 1)
            {
                ui.selectable_value(&mut layout, value, value.label());
            }
        })
        .response;
    accessibility::focus(ui, &response);
    layout
}

/// Retain existing addresses when a reviewed alias changes speaker width.
/// Takes a draft alias and target count; extends with unique physical addresses, while recording keeps its existing logical channel order.
fn resize(port: &mut Port, width: usize) {
    if port.direction == Direction::Record {
        port.channels = (0..width as u16).collect();
        return;
    }
    port.channels.truncate(width);
    for channel in 0..MAX_PHYSICAL_CHANNELS as u16 {
        if port.channels.len() == width {
            break;
        }
        if !port.channels.contains(&channel) {
            port.channels.push(channel);
        }
    }
}

/// Map the declared program bed into one reviewed physical alias.
/// Takes the selected alias and canonical layout; preserves retained addresses where possible and replaces only its Main post-mixer map.
fn program_output(model: &mut Model, id: u64, layout: SpeakerLayout) {
    let occupied = model
        .ports
        .iter()
        .filter(|p| p.id != id && p.direction == Direction::Output)
        .flat_map(|p| p.channels.iter().copied())
        .collect::<std::collections::BTreeSet<_>>();
    let Some(port) = model
        .ports
        .iter_mut()
        .find(|p| p.direction == Direction::Output && p.id == id)
    else {
        return;
    };
    port.channels.truncate(layout.channels());
    for channel in 0..MAX_PHYSICAL_CHANNELS as u16 {
        if port.channels.len() == layout.channels() {
            break;
        }
        if !port.channels.contains(&channel) && !occupied.contains(&channel) {
            port.channels.push(channel);
        }
    }
    model.set_speaker_layout(Group::Output(id), layout);
    let route = Connection {
        source: Source {
            group: Group::Main,
            tap: Tap::PostMixer,
        },
        destination: Group::Output(id),
        map: (0..layout.channels() as u8)
            .map(|channel| ChannelMap {
                source: channel,
                destination: channel,
                gain: 1.,
            })
            .collect(),
    };
    if let Some(current) = model
        .connections
        .iter_mut()
        .find(|r| r.source == route.source && r.destination == route.destination)
    {
        *current = route;
    } else {
        model.connections.push(route);
    }
}

/// Review explicit speaker beds and the separate software monitoring split.
/// Takes the retained routing draft; edits saved identities, channel maps and monitoring policy through the existing atomic review/Undo transaction.
pub(super) fn controls(ui: &mut Ui, draft: &mut Draft) {
    ui.push_id(("speaker-controls", draft.namespace), |ui| {
        controls_inner(ui, draft)
    });
}

/// Edit the retained speaker controls inside their immutable project namespace.
/// Takes a scoped UI and draft; changes only producer-owned reviewed state.
fn controls_inner(ui: &mut Ui, draft: &mut Draft) {
    ui.label("Speaker order belongs to the saved graph. A wide port alone is not surround. Built-in stereo inserts process FL/FR; LFE stays separate at unity. Faders, mute/solo and crossfader apply to all channels. Mono/stereo folds exclude LFE. Native wide returns must preserve their declared primary bed.");
    let current = draft.model.speaker_layout(Group::Main).unwrap_or_default();
    let selected = choice(
        ui,
        "program-layout",
        "Program speaker layout",
        current,
        false,
    );
    if selected != current {
        draft.model.set_speaker_layout(Group::Main, selected);
        for scene in &draft.layout.scenes {
            draft
                .model
                .set_speaker_layout(Group::Scene(scene.id), selected);
        }
    }
    let outputs = draft
        .model
        .ports
        .iter()
        .filter(|p| p.direction == Direction::Output && draft.model.monitor_output != Some(p.id))
        .map(|p| (p.id, p.alias.clone()))
        .collect::<Vec<_>>();
    for (id, name) in outputs {
        if ui
            .button(format!("Map program speakers to {name}"))
            .clicked()
        {
            program_output(&mut draft.model, id, selected);
        }
    }
    ui.collapsing("Track speaker layouts", |ui| {
        for track in draft.layout.tracks.iter().filter(|t| t.active) {
            let group = Group::Track(track.id);
            let current = draft.model.speaker_layout(group).unwrap_or_default();
            let next = choice(
                ui,
                ("track-layout", track.id.0),
                &format!("{} track speaker layout", track.name),
                current,
                false,
            );
            if next != current {
                draft.model.set_speaker_layout(group, next);
            }
        }
    });
    ui.collapsing("Scene speaker layouts", |ui| {
        for scene in draft.layout.scenes.iter().filter(|s| s.active) {
            let group = Group::Scene(scene.id);
            let current = draft.model.speaker_layout(group).unwrap_or_default();
            let next = choice(
                ui,
                ("scene-layout", scene.id.0),
                &format!("{} scene speaker layout", scene.name),
                current,
                false,
            );
            if next != current {
                draft.model.set_speaker_layout(group, next);
            }
        }
    });
    let ports = draft
        .model
        .ports
        .iter()
        .map(|p| (p.id, p.alias.clone(), p.direction))
        .collect::<Vec<_>>();
    for (id, name, direction) in ports {
        ui.push_id(("speaker-alias", id), |ui| {
            let group = match direction {
                Direction::Input => Group::Input(id),
                Direction::Output => Group::Output(id),
                Direction::Record => Group::Record(id),
            };
            let mut next = draft
                .model
                .speaker_layouts
                .iter()
                .find(|b| b.group == group)
                .map(|b| b.layout);
            let before = next;
            let response =
                egui::ComboBox::new("alias-layout", format!("{name} alias speaker layout"))
                    .selected_text(next.map_or("Legacy / discrete channels", SpeakerLayout::label))
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut next, None, "Legacy / discrete channels");
                        for &value in SpeakerLayout::ALL {
                            ui.selectable_value(&mut next, Some(value), value.label());
                        }
                    })
                    .response;
            accessibility::focus(ui, &response);
            if next != before {
                if let Some(layout) = next {
                    resize(
                        draft.model.ports.iter_mut().find(|p| p.id == id).unwrap(),
                        layout.channels(),
                    );
                    draft.model.set_speaker_layout(group, layout);
                } else {
                    draft.model.speaker_layouts.retain(|b| b.group != group);
                }
            }
            if let Some(layout) = next {
                ui.label(format!(
                    "Ordered speakers {:?} · WAVE mask 0x{:X}",
                    layout.speakers(),
                    layout.wave_mask()
                ));
            }
        });
    }
    let buses = draft
        .model
        .buses
        .iter()
        .map(|b| (b.id, b.alias.clone()))
        .collect::<Vec<_>>();
    for (id, name) in buses {
        let current = draft
            .model
            .speaker_layout(Group::Bus(id))
            .unwrap_or_default();
        let next = choice(
            ui,
            ("return-layout", id),
            &format!("{name} return speaker layout"),
            current,
            true,
        );
        if next != current {
            draft
                .model
                .buses
                .iter_mut()
                .find(|b| b.id == id)
                .unwrap()
                .channels = next.channels() as u8;
            draft.model.set_speaker_layout(Group::Bus(id), next);
        }
    }
    if ui
        .add_enabled(
            selected.channels() > 2
                && draft.model.buses.len() < MAX_BUSES
                && draft.model.next_id < u64::MAX,
            egui::Button::new(format!("Add declared {} return", selected.short_label())),
        )
        .clicked()
    {
        let id = draft.model.next_id;
        draft.model.next_id += 1;
        draft.model.buses.push(Bus {
            id,
            alias: format!("{} return {id}", selected.short_label()),
            channels: selected.channels() as u8,
            gain: 1.,
            mute: false,
            mixer: Default::default(),
        });
        draft.model.set_speaker_layout(Group::Bus(id), selected);
        draft.model.connections.push(Connection {
            source: Source {
                group: Group::Bus(id),
                tap: Tap::PostMixer,
            },
            destination: Group::Main,
            map: (0..selected.channels() as u8)
                .map(|channel| ChannelMap {
                    source: channel,
                    destination: channel,
                    gain: 1.,
                })
                .collect(),
        });
    }
    ui.separator();
    ui.label("Optional monitor bass split: one-pole complementary high/low pass, averaged full-range channel bass added to the existing unity LFE. This affects physical monitoring only. Headphones, recording and delivery retain the declared program bed. It does not calibrate speakers.");
    let wide = draft
        .model
        .ports
        .iter()
        .filter(|p| {
            p.direction == Direction::Output
                && draft
                    .model
                    .speaker_layout(Group::Output(p.id))
                    .is_some_and(|layout| {
                        layout.speakers().contains(
                            &crate::engine::audio::routing::speakers::Speaker::LowFrequency,
                        )
                    })
        })
        .map(|p| (p.id, p.alias.clone()))
        .collect::<Vec<_>>();
    let mut enabled = draft.model.monitor_bass.is_some();
    if ui
        .add_enabled(
            !wide.is_empty(),
            egui::Checkbox::new(&mut enabled, "Enable monitor bass management"),
        )
        .changed()
    {
        draft.model.monitor_bass = if enabled {
            Some(BassManagement {
                output: wide[0].0,
                cutoff: 80.,
                level: 1.,
            })
        } else {
            None
        };
    }
    if let Some(bass) = &mut draft.model.monitor_bass {
        let response = egui::ComboBox::new("bass-output", "Bass-managed output")
            .selected_text(
                wide.iter()
                    .find(|p| p.0 == bass.output)
                    .map_or("Missing output", |p| p.1.as_str()),
            )
            .show_ui(ui, |ui| {
                for (id, name) in &wide {
                    ui.selectable_value(&mut bass.output, *id, name);
                }
            })
            .response;
        accessibility::focus(ui, &response);
        if let Some(value) = number(ui, "Bass cutoff (Hz)", bass.cutoff, 20., 200., 1.) {
            bass.cutoff = value;
        }
        if let Some(value) = number(ui, "Monitor sub level", bass.level, 0., 1.5, 0.01) {
            bass.level = value;
        }
    }
}
