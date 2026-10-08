use super::*;
use crate::engine::audio::routing::model::{
    ChannelMap, Connection, Group, Model, Source as TapSource, Tap,
};

fn connect(graph: &mut Model, from: Group, to: Group, tap: Tap, gain: f32) -> Result<(), String> {
    if graph.connections.len() >= crate::engine::audio::routing::model::MAX_CONNECTIONS {
        return Err(
            "Imported routing exceeds 256 connections; migrate fewer tracks or sends".into(),
        );
    }
    graph.connections.push(Connection {
        source: TapSource { group: from, tap },
        destination: to,
        map: vec![
            ChannelMap {
                source: 0,
                destination: 0,
                gain,
            },
            ChannelMap {
                source: 1,
                destination: 1,
                gain,
            },
        ],
    });
    Ok(())
}

/// Preserve internal group and return signal paths.
/// Takes source tracks, their fresh native identities and retained review; returns an acyclic stereo graph without automatically opening physical inputs.
pub(super) fn convert(
    set: &Element,
    nodes: &Element,
    tracks: &[Track],
    layout: &session::Layout,
    native: &mut [project::Track],
    source: &mut Source,
) -> Result<Model, String> {
    let mut graph = Model::default();
    let returns: Vec<_> = tracks.iter().filter(|t| t.role == "ReturnTrack").collect();
    for (index, (node, track)) in nodes.children.iter().zip(tracks).enumerate() {
        let group = Group::Track(track.native.id);
        let target = value(node, &["DeviceChain", "AudioOutputRouting", "Target"])?;
        let destination = if target == "AudioOut/Master"
            || target == "AudioOut/Main" && source.format.starts_with("12.")
            || target.is_empty() && track.parent == -1
        {
            Some(Group::Main)
        } else if target == "AudioOut/GroupTrack" || target.is_empty() && track.parent != -1 {
            let parent = tracks
                .iter()
                .find(|t| t.source_id == track.parent && t.role == "GroupTrack")
                .ok_or("Live group output has no matching group track")?;
            Some(Group::Track(parent.native.id))
        } else if target == "AudioOut/None" {
            None
        } else {
            source.difference(format!("track:{}",track.source_id),"Audio output",format!("Unresolved source route {target}; native default output is disconnected until explicitly resolved"))?;
            None
        };
        graph.tracks_without_default_send.push(track.native.id);
        if let Some(destination) = destination {
            connect(&mut graph, group, destination, Tap::PostMixer, 1.)?;
        }
        if matches!(track.role.as_str(), "GroupTrack" | "ReturnTrack") {
            native[index].input_monitor = Some(engine::input_monitor::Mode::In);
            source.difference(format!("track:{}",track.source_id),"Internal bus",format!("{} receives only reviewed internal graph connections. No physical input was assigned",track.role))?;
        }
        if let Some(sends) = at(node, &["DeviceChain", "Mixer", "Sends"])? {
            let mut indices = BTreeSet::new();
            for holder in &sends.children {
                if holder.name != "TrackSendHolder" {
                    return Err("Unknown Live return send record".into());
                }
                let ordinal = integer(holder.attr("Id"))?;
                if ordinal < 0 || !indices.insert(ordinal) {
                    return Err("Return send identity is invalid or repeated".into());
                }
                let amount = number(holder, &["Send", "Manual"], 0.)?;
                if !(0.0..=1.).contains(&amount) {
                    return Err("Source send gain is outside supported 0–1 range".into());
                }
                let enabled_by_user = boolean(holder, &["EnabledByUser"], true)?;
                let active = boolean(holder, &["Active"], true)?;
                if amount == 0. || !enabled_by_user || !active {
                    continue;
                }
                let Some(return_track) = returns.get(ordinal as usize) else {
                    return Err("Live send references an absent return track".into());
                };
                let mut pre = false;
                if let Some(positions) = child(set, "SendsPre")? {
                    let mut seen = BTreeSet::new();
                    for position in &positions.children {
                        if position.name != "SendPreBool"
                            || !seen.insert(integer(position.attr("Id"))?)
                        {
                            return Err("Unknown or repeated send position".into());
                        }
                        if integer(position.attr("Id"))? == ordinal {
                            pre = match position.attr("Value") {
                                "true" => true,
                                "false" => false,
                                _ => return Err("Invalid pre-fader send flag".into()),
                            };
                        }
                    }
                }
                let tap = if pre { Tap::PostFx } else { Tap::PostMixer };
                connect(
                    &mut graph,
                    group,
                    Group::Track(return_track.native.id),
                    tap,
                    amount as f32,
                )?;
            }
        }
    }
    graph.order(layout)?;
    Ok(graph)
}
