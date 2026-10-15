//! Offline contribution selection observes the normal graph without muting its feeders.
use super::model::*;
use crate::engine::{project::Captured, session::Id};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", content = "id", rename_all = "snake_case")]
pub enum Target {
    Track(Id),
    Scene(Id),
    Return(u64),
    FixedReturn(u8),
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Policy {
    #[default]
    PostDevice,
    Dry,
    MasteredSolo,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Selection {
    pub namespace: [u64; 2],
    pub targets: Vec<Target>,
    pub policy: Policy,
}

impl Selection {
    /// Resolve immutable namespaced contribution identities.
    /// Takes the exact capture and selected output alias; returns a fully validated observation graph, leaving capture state untouched on refusal.
    pub fn plan(&self, captured: &Captured, alias: Option<u64>) -> Result<Model, String> {
        let state = &captured.state;
        let layout = state
            .session
            .as_ref()
            .ok_or("Contribution rendering requires stable project identities")?;
        if self.namespace != layout.namespace
            || self.targets.is_empty()
            || self.targets.len() > 256
            || self.targets.iter().copied().collect::<BTreeSet<_>>().len() != self.targets.len()
        {
            return Err(
                "Contribution selection needs unique targets from this exact project".into(),
            );
        }
        let mut model = state.routing.as_deref().cloned().unwrap_or_default();
        let alias = if state.routing.is_none() {
            if alias.is_some() {
                return Err("Reviewed output alias no longer exists".into());
            }
            1
        } else {
            alias.ok_or("Choose the saved program output for contribution rendering")?
        };
        let port = model
            .port(alias, Direction::Output)
            .ok_or("Contribution output alias was removed")?;
        let width = port.channels.len();
        if model.monitor_output == Some(alias) {
            return Err("Headphones are separate from contribution delivery".into());
        }
        let mut groups = Vec::with_capacity(self.targets.len());
        for target in &self.targets {
            let group = match *target {
                Target::Track(id) => {
                    let slot = layout
                        .tracks
                        .iter()
                        .position(|t| t.active && t.id == id)
                        .ok_or("Selected track was removed or its identity changed")?;
                    let scene = layout.scenes[state.tracks[slot].scene_bus].id;
                    if self.targets.contains(&Target::Scene(scene)) {
                        return Err(
                            "Track and its shared scene FX overlap; render them in separate passes"
                                .into(),
                        );
                    }
                    if self.policy == Policy::Dry {
                        Group::Track(id)
                    } else {
                        let owners = model
                            .plugins
                            .iter()
                            .filter(|p| p.scene_track == Some(id))
                            .collect::<Vec<_>>();
                        if owners.len() > 1 {
                            return Err("Selected track has ambiguous final device routes".into());
                        }
                        owners
                            .first()
                            .map_or(Group::Track(id), |p| Group::Plugin(p.id))
                    }
                }
                Target::Scene(id) => {
                    if !layout.scenes.iter().any(|s| s.active && s.id == id) {
                        return Err("Selected scene/group was removed".into());
                    }
                    Group::Scene(id)
                }
                Target::Return(id) => {
                    if !model.buses.iter().any(|b| b.id == id) {
                        return Err("Selected return was removed or its identity changed".into());
                    }
                    let mut group = Group::Bus(id);
                    if self.policy != Policy::Dry {
                        loop {
                            let outgoing = model
                                .links()
                                .filter(|r| {
                                    r.source.group == group && r.map.iter().any(|m| m.gain != 0.)
                                })
                                .collect::<Vec<_>>();
                            let processors = outgoing
                                .iter()
                                .filter_map(|r| {
                                    if let Group::Plugin(id) = r.destination {
                                        Some(id)
                                    } else {
                                        None
                                    }
                                })
                                .collect::<Vec<_>>();
                            if processors.is_empty() {
                                break;
                            }
                            if processors.len() != 1 || outgoing.len() != 1 {
                                return Err("Return has ambiguous parallel device paths; select an explicit shared group pass".into());
                            }
                            group = Group::Plugin(processors[0]);
                        }
                    }
                    group
                }
                Target::FixedReturn(index) => {
                    if index >= 2 {
                        return Err("Fixed return must be reverb or echo".into());
                    }
                    Group::FixedReturn(index)
                }
            };
            let tap = if self.policy == Policy::Dry {
                Tap::PreFx
            } else {
                Tap::PostMixer
            };
            let source = Source { group, tap };
            let source_width = model
                .source_width(source, layout)
                .ok_or("Contribution source no longer exists")?;
            if source_width > width {
                return Err(
                    "Choose a program alias wide enough for every selected contribution".into(),
                );
            }
            groups.push((source, source_width));
        }
        if self.policy == Policy::MasteredSolo
            && model.links().any(|r| {
                r.destination == Group::Output(alias)
                    && r.source.group != Group::Main
                    && r.map.iter().any(|m| m.gain != 0.)
            })
        {
            return Err(
                "Mastered solo requires a reviewed output supplied exclusively by Main".into(),
            );
        }
        model.order(layout)?;
        for connection in &mut model.connections {
            if matches!(connection.destination, Group::Output(_) | Group::Record(_))
                && !(self.policy == Policy::MasteredSolo && connection.source.group == Group::Main)
            {
                connection.map.iter_mut().for_each(|m| m.gain = 0.);
            }
            if self.policy == Policy::MasteredSolo && connection.destination == Group::Main {
                connection.map.iter_mut().for_each(|m| m.gain = 0.);
            }
        }
        for send in &mut model.sends {
            if matches!(send.route.destination, Group::Output(_) | Group::Record(_))
                || self.policy == Policy::MasteredSolo && send.route.destination == Group::Main
            {
                send.route.map.iter_mut().for_each(|m| m.gain = 0.);
            }
        }
        let destination = if self.policy == Policy::MasteredSolo {
            Group::Main
        } else {
            Group::Output(alias)
        };
        let destination_width = model.input_width(destination, layout).unwrap();
        for (source, source_width) in groups {
            let map = if source_width == 1 && destination_width == 2 {
                vec![
                    ChannelMap {
                        source: 0,
                        destination: 0,
                        gain: 1.,
                    },
                    ChannelMap {
                        source: 0,
                        destination: 1,
                        gain: 1.,
                    },
                ]
            } else {
                (0..source_width.min(destination_width))
                    .map(|i| ChannelMap {
                        source: i as u8,
                        destination: i as u8,
                        gain: 1.,
                    })
                    .collect()
            };
            model.connections.push(Connection {
                source,
                destination,
                map,
            });
        }
        model.order(layout)?;
        Ok(model)
    }
}
