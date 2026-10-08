use crate::engine::{
    audio::routing::model::{Direction, Group, Model},
    session::{Layout, Reference},
};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    sync::Arc,
};

/// Merge only a selected internal signal graph.
/// Takes source and destination routing plus newly allocated musical identities; returns a complete validated graph or refuses unresolved physical/master dependencies.
pub(super) fn merge(
    current: Option<&Arc<Model>>,
    source: &Model,
    source_layout: &Layout,
    next: &Layout,
    tracks: &HashMap<Reference, Reference>,
    scenes: &HashMap<usize, usize>,
) -> Result<(Arc<Model>, BTreeMap<u64, u64>), String> {
    let mut graph = current.map(|m| (**m).clone()).unwrap_or_default();
    let mut groups = BTreeSet::new();
    let mut mapped = BTreeMap::new();
    for (old, new) in tracks {
        groups.insert(Group::Track(old.id));
        mapped.insert(Group::Track(old.id), Group::Track(new.id));
    }
    for (&old, &new) in scenes {
        let from = Group::Scene(source_layout.scenes[old].id);
        groups.insert(from);
        mapped.insert(from, Group::Scene(next.scenes[new].id));
    }
    for plugin in &source.plugins {
        if plugin
            .midi_track
            .is_some_and(|id| groups.contains(&Group::Track(id)))
            || plugin
                .scene_track
                .is_some_and(|id| groups.contains(&Group::Track(id)))
        {
            groups.insert(Group::Plugin(plugin.id));
        }
    }
    loop {
        let before = groups.len();
        for route in &source.connections {
            let a = route.source.group;
            let b = route.destination;
            if a != Group::Main && groups.contains(&a) || b != Group::Main && groups.contains(&b) {
                for group in [a, b] {
                    if matches!(group, Group::Bus(_) | Group::Plugin(_) | Group::Record(_)) {
                        groups.insert(group);
                    }
                }
            }
        }
        if before == groups.len() {
            break;
        }
    }
    let mut identity = || -> Result<u64, String> {
        let id = graph.next_id;
        graph.next_id = id
            .checked_add(1)
            .ok_or("Imported routing exhausted its identities")?;
        Ok(id)
    };
    for &group in &groups {
        if matches!(group, Group::Bus(_) | Group::Plugin(_) | Group::Record(_)) {
            let id = identity()?;
            mapped.insert(
                group,
                match group {
                    Group::Bus(_) => Group::Bus(id),
                    Group::Plugin(_) => Group::Plugin(id),
                    Group::Record(_) => Group::Record(id),
                    _ => unreachable!(),
                },
            );
        }
    }
    drop(identity);
    let resolve = |group: Group| -> Result<Group, String> {
        if group == Group::Main {
            return Ok(group);
        }
        mapped.get(&group).copied().ok_or_else(||"Imported route depends on an unselected track, scene or physical endpoint. Select its musical dependencies or open the complete source project and review its routes".into())
    };
    for bus in &source.buses {
        if groups.contains(&Group::Bus(bus.id)) {
            let mut bus = bus.clone();
            let Group::Bus(id) = resolve(Group::Bus(bus.id))? else {
                unreachable!()
            };
            bus.id = id;
            graph.buses.push(bus);
        }
    }
    for port in &source.ports {
        if port.direction == Direction::Record && groups.contains(&Group::Record(port.id)) {
            let mut port = port.clone();
            let Group::Record(id) = resolve(Group::Record(port.id))? else {
                unreachable!()
            };
            port.id = id;
            graph.ports.push(port);
        }
    }
    for plugin in &source.plugins {
        if !groups.contains(&Group::Plugin(plugin.id)) {
            continue;
        }
        let mut plugin = plugin.clone();
        let Group::Plugin(id) = resolve(Group::Plugin(plugin.id))? else {
            unreachable!()
        };
        plugin.id = id;
        let musical = |id| -> Result<_, String> {
            let Group::Track(id) = resolve(Group::Track(id))? else {
                unreachable!()
            };
            Ok(id)
        };
        plugin.midi_track = plugin.midi_track.map(musical).transpose()?;
        plugin.scene_track = plugin.scene_track.map(musical).transpose()?;
        graph.plugins.push(plugin);
    }
    for route in &source.connections {
        let from = route.source.group;
        let to = route.destination;
        if from == Group::Main && matches!(to, Group::Output(_)) {
            continue;
        }
        if !groups.contains(&from) && !groups.contains(&to) {
            continue;
        }
        if from == Group::Main {
            return Err("Source master processing requires opening the complete project; merging it would alter existing destination tracks".into());
        }
        let mut route = route.clone();
        route.source.group = resolve(from)?;
        route.destination = resolve(to)?;
        graph.connections.push(route);
    }
    for &id in &source.tracks_without_default_send {
        if let Some(Group::Track(id)) = mapped.get(&Group::Track(id)) {
            graph.tracks_without_default_send.push(*id);
        }
    }
    if let Some(original) = &source.latency {
        let latency = graph.latency.get_or_insert_with(Default::default);
        latency.reserve_micros = latency.reserve_micros.max(original.reserve_micros);
        for report in &original.reports {
            if let Some(&group) = mapped.get(&report.group) {
                let mut report = *report;
                report.group = group;
                latency.reports.push(report);
            }
        }
    }
    graph.version = graph.version.max(source.version);
    graph.order(next)?;
    let plugins = mapped
        .iter()
        .filter_map(|(old, new)| {
            if let (Group::Plugin(old), Group::Plugin(new)) = (old, new) {
                Some((*old, *new))
            } else {
                None
            }
        })
        .collect();
    Ok((Arc::new(graph), plugins))
}
