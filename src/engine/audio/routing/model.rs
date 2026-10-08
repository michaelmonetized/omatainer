//! Saved aliases and explicit channel maps; preparation never depends on device presence.
use crate::engine::session::{Id, Layout};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
pub(crate) use super::latency::{Configuration as LatencyConfiguration, Report as LatencyReport};

pub const MAX_PHYSICAL_CHANNELS: usize = 64;
pub const MAX_PORT_CHANNELS: usize = 32;
pub const MAX_RECORD_CHANNELS: usize = 26;
pub const MAX_PORTS: usize = 32;
pub const MAX_BUSES: usize = 32;
pub const MAX_CONNECTIONS: usize = 256;
pub const MAX_MAPS: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Tap {
    PreFx,
    PostFx,
    PostMixer,
}
impl Tap {
    /// Select a prepared tap.
    /// Takes a saved tap name; returns its fixed three-frame index.
    pub fn index(self) -> usize {
        match self {
            Self::PreFx => 0,
            Self::PostFx => 1,
            Self::PostMixer => 2,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    Input,
    Output,
    Record,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Port {
    pub id: u64,
    pub alias: String,
    pub direction: Direction,
    pub channels: Vec<u16>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Bus {
    pub id: u64,
    pub alias: String,
    pub channels: u8,
    pub gain: f32,
    pub mute: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", content = "id", rename_all = "snake_case")]
pub enum Group {
    Input(u64),
    Track(Id),
    Scene(Id),
    Deck(u8),
    Bus(u64),
    Plugin(u64),
    Main,
    Output(u64),
    Record(u64),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Source {
    pub group: Group,
    pub tap: Tap,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChannelMap {
    pub source: u8,
    pub destination: u8,
    pub gain: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Connection {
    pub source: Source,
    pub destination: Group,
    pub map: Vec<ChannelMap>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Model {
    pub version: u32,
    pub next_id: u64,
    pub ports: Vec<Port>,
    pub buses: Vec<Bus>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) plugins: Vec<super::plugins::Instance>,
    pub connections: Vec<Connection>,
    pub tracks_without_default_send: Vec<Id>,
    pub decks_without_default_send: [bool; 2],
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<InputConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub monitor_output: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latency: Option<super::latency::Configuration>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InputConfig {
    pub backend: String,
    pub device: String,
    pub channels: u16,
    pub format: crate::preferences::AudioFormat,
    pub buffer_frames: Option<u32>,
}
impl Default for Model {
    fn default() -> Self {
        Self {
            version: 1,
            next_id: 2,
            ports: vec![Port {
                id: 1,
                alias: "Main output".into(),
                direction: Direction::Output,
                channels: vec![0, 1],
            }],
            buses: Vec::new(),
            plugins: Vec::new(),
            connections: vec![Connection {
                source: Source {
                    group: Group::Main,
                    tap: Tap::PostMixer,
                },
                destination: Group::Output(1),
                map: vec![
                    ChannelMap {
                        source: 0,
                        destination: 0,
                        gain: 1.0,
                    },
                    ChannelMap {
                        source: 1,
                        destination: 1,
                        gain: 1.0,
                    },
                ],
            }],
            tracks_without_default_send: Vec::new(),
            decks_without_default_send: [false; 2],
            input: None,
            monitor_output: None,
            latency: None,
        }
    }
}

impl Model {
    /// Identify a report-only edit that can retain the running signal graph.
    /// Takes the active saved model; returns true only for enabled compensation with unchanged reserve, aliases, taps, sends and input choices.
    pub(crate) fn latency_edit_of(&self, prior: &Self) -> bool {
        self.latency.as_ref().zip(prior.latency.as_ref()).is_some_and(|(next, old)| next.reserve_micros == old.reserve_micros)
            && self.version == prior.version && self.next_id == prior.next_id
            && self.ports == prior.ports && self.buses == prior.buses
            && self.plugins == prior.plugins
            && self.connections == prior.connections
            && self.tracks_without_default_send == prior.tracks_without_default_send
            && self.decks_without_default_send == prior.decks_without_default_send
            && self.input == prior.input && self.monitor_output == prior.monitor_output
    }
}

/// Validate a saved alias name.
/// Takes its text; returns whether its bounded name can be displayed safely.
fn named(name: &str) -> bool {
    !name.trim().is_empty() && name.len() <= 128 && !name.chars().any(char::is_control)
}

impl Model {
    /// Count bounded saved routing storage.
    /// Takes this model; returns its owned aliases, maps and selection bytes for admission checks.
    pub(crate) fn bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + self.ports.capacity() * std::mem::size_of::<Port>()
            + self
                .ports
                .iter()
                .map(|port| port.alias.capacity() + port.channels.capacity() * 2)
                .sum::<usize>()
            + self.buses.capacity() * std::mem::size_of::<Bus>()
            + self.plugins.iter().map(super::plugins::Instance::bytes).sum::<usize>()
            + self
                .buses
                .iter()
                .map(|bus| bus.alias.capacity())
                .sum::<usize>()
            + self.connections.capacity() * std::mem::size_of::<Connection>()
            + self
                .connections
                .iter()
                .map(|connection| connection.map.capacity() * std::mem::size_of::<ChannelMap>())
                .sum::<usize>()
            + self.tracks_without_default_send.capacity() * std::mem::size_of::<Id>()
            + self.input.as_ref().map_or(0, |input| {
                input.backend.capacity() + input.device.capacity()
            })
            + self.latency.as_ref().map_or(0, |config| config.reports.capacity() * std::mem::size_of::<LatencyReport>())
    }
    /// Retain both sides of a mono default mix.
    /// Takes the active output width; returns explicit stereo or averaged mono routes.
    pub fn for_output_channels(channels: usize) -> Self {
        let mut model = Self::default();
        if channels == 1 {
            model.ports[0].channels = vec![0];
            for map in &mut model.connections[0].map {
                map.destination = 0;
                map.gain = 0.5;
            }
        }
        model
    }
    /// Resolve a saved port alias.
    /// Takes its stable ID and direction; returns its retained channel definition.
    pub fn port(&self, id: u64, direction: Direction) -> Option<&Port> {
        self.ports
            .iter()
            .find(|port| port.id == id && port.direction == direction)
    }

    /// Resolve logical channel width.
    /// Takes a saved endpoint and session; returns its bounded width, including inactive identities.
    pub fn width(&self, group: Group, layout: &Layout) -> Option<usize> {
        match group {
            Group::Input(id) => self
                .port(id, Direction::Input)
                .map(|port| port.channels.len()),
            Group::Output(id) => self
                .port(id, Direction::Output)
                .map(|port| port.channels.len()),
            Group::Record(id) => self
                .port(id, Direction::Record)
                .map(|port| port.channels.len()),
            Group::Track(id) => (id.0 > 0
                && id.0 < layout.next_id
                && !layout.scenes.iter().any(|item| item.id == id))
            .then_some(2),
            Group::Scene(id) => (id.0 > 0
                && id.0 < layout.next_id
                && !layout.tracks.iter().any(|item| item.id == id))
            .then_some(2),
            Group::Deck(id) => (id < 2).then_some(2),
            Group::Bus(id) => self
                .buses
                .iter()
                .find(|bus| bus.id == id)
                .map(|bus| usize::from(bus.channels)),
            Group::Main => Some(2),
            Group::Plugin(id) => self.plugins.iter().find(|p| p.id == id).map(super::plugins::Instance::output_width),
        }
    }

    /// Validate and order a complete routing graph.
    /// Takes the retained session; returns each processing group once, or rejects invalid maps and feedback.
    pub fn order(&self, layout: &Layout) -> Result<Vec<Group>, String> {
        layout.validate()?;
        if let Some(latency)=&self.latency {latency.validate(self,layout)?;}
        if self.input.as_ref().is_some_and(|input| {
            !named(&input.backend)
                || !named(&input.device)
                || !(1..=MAX_PHYSICAL_CHANNELS).contains(&usize::from(input.channels))
                || input
                    .buffer_frames
                    .is_some_and(|frames| !(16..=16384).contains(&frames))
        }) {
            return Err("Invalid saved input configuration".into());
        }
        if !(1..=3).contains(&self.version)
            || (self.version == 1 && self.monitor_output.is_some())
            || (self.version < 3 && !self.plugins.is_empty())
            || self.plugins.len() > super::plugins::MAX_PLUGINS
            || self.next_id == 0
            || self.buses.len() > MAX_BUSES
            || self.connections.len() > MAX_CONNECTIONS
            || self.ports.len() > MAX_PORTS * 3
        {
            return Err("Routing graph exceeds its supported version or storage limits".into());
        }
        let mut ids = BTreeSet::new();
        let mut names = BTreeSet::new();
        for port in &self.ports {
            if port.id == 0
                || port.id >= self.next_id
                || !ids.insert(port.id)
                || !named(&port.alias)
                || !names.insert(port.alias.as_str())
                || port.channels.is_empty()
                || port.channels.len() > MAX_PORT_CHANNELS
                || port
                    .channels
                    .iter()
                    .any(|channel| usize::from(*channel) >= MAX_PHYSICAL_CHANNELS)
                || port.channels.iter().copied().collect::<BTreeSet<_>>().len()
                    != port.channels.len()
            {
                return Err("Invalid, repeated or oversized channel alias".into());
            }
            if port.direction == Direction::Record
                && (port.channels.len() > MAX_RECORD_CHANNELS
                    || port
                        .channels
                        .iter()
                        .enumerate()
                        .any(|(index, channel)| index != usize::from(*channel)))
            {
                return Err("Record aliases support 1–26 channels ordered from channel one".into());
            }
        }
        for direction in [Direction::Input, Direction::Output, Direction::Record] {
            if self
                .ports
                .iter()
                .filter(|port| port.direction == direction)
                .count()
                > MAX_PORTS
            {
                return Err("At most 32 aliases per direction are supported".into());
            }
        }
        for bus in &self.buses {
            if bus.id == 0
                || bus.id >= self.next_id
                || !ids.insert(bus.id)
                || !named(&bus.alias)
                || !names.insert(bus.alias.as_str())
                || !(1..=MAX_PORT_CHANNELS).contains(&usize::from(bus.channels))
                || !bus.gain.is_finite()
                || !(0.0..=1.5).contains(&bus.gain)
            {
                return Err("Invalid, repeated or oversized virtual bus".into());
            }
        }
        let mut plugin_bytes = 0usize;
        let mut instruments = BTreeSet::new();
        for plugin in &self.plugins {
            plugin.validate(layout)?;
            plugin_bytes = plugin_bytes.saturating_add(plugin.bytes());
            if plugin.id == 0 || plugin.id >= self.next_id || !ids.insert(plugin.id) || !names.insert(plugin.name.as_str())
                || plugin_bytes > 32 * 1024 * 1024 || plugin.instrument && !instruments.insert(plugin.midi_track.unwrap())
            { return Err("Plugins need unique IDs, names and instrument tracks within 32 MiB of saved state".into()); }
        }
        super::plugins::validate_encoded(&self.plugins)?;
        if self.tracks_without_default_send.len() > MAX_CONNECTIONS
            || self
                .tracks_without_default_send
                .iter()
                .any(|id| id.0 == 0 || id.0 >= layout.next_id)
            || self
                .tracks_without_default_send
                .iter()
                .copied()
                .collect::<BTreeSet<_>>()
                .len()
                != self.tracks_without_default_send.len()
        {
            return Err("A default-send override has no unique retained track".into());
        }
        let mut graph: BTreeMap<Group, BTreeSet<Group>> = BTreeMap::new();
        graph.insert(Group::Main, BTreeSet::new());
        for track in &layout.tracks {
            graph.insert(Group::Track(track.id), BTreeSet::new());
        }
        for scene in &layout.scenes {
            graph.insert(Group::Scene(scene.id), BTreeSet::new());
        }
        for plugin in &self.plugins { if plugin.scene_track.is_some() { for scene in &layout.scenes { graph.get_mut(&Group::Scene(scene.id)).unwrap().insert(Group::Plugin(plugin.id)); } } }
        for deck in 0..2 {
            graph.insert(Group::Deck(deck), BTreeSet::new());
        }
        for bus in &self.buses {
            graph.insert(Group::Bus(bus.id), BTreeSet::new());
        }
        for plugin in &self.plugins { graph.insert(Group::Plugin(plugin.id), BTreeSet::new()); }
        for port in &self.ports {
            graph.insert(
                match port.direction {
                    Direction::Input => Group::Input(port.id),
                    Direction::Output => Group::Output(port.id),
                    Direction::Record => Group::Record(port.id),
                },
                BTreeSet::new(),
            );
        }
        for track in &layout.tracks {
            if !self.tracks_without_default_send.contains(&track.id) {
                for scene in &layout.scenes {
                    graph
                        .get_mut(&Group::Scene(scene.id))
                        .unwrap()
                        .insert(Group::Track(track.id));
                }
            }
        }
        for scene in &layout.scenes {
            graph
                .get_mut(&Group::Main)
                .unwrap()
                .insert(Group::Scene(scene.id));
        }
        for deck in 0..2 {
            if !self.decks_without_default_send[usize::from(deck)] {
                graph
                    .get_mut(&Group::Main)
                    .unwrap()
                    .insert(Group::Deck(deck));
            }
        }
        let mut maps = 0usize;
        for connection in &self.connections {
            let from = self
                .source_width(connection.source, layout)
                .ok_or("Routing source no longer exists")?;
            let to = self
                .input_width(connection.destination, layout)
                .ok_or("Routing destination no longer exists")?;
            maps = maps.saturating_add(connection.map.len());
            if matches!(connection.source.group, Group::Output(_) | Group::Record(_))
                || matches!(connection.destination, Group::Input(_) | Group::Deck(_))
                || connection.map.is_empty()
                || maps > MAX_MAPS
                || connection.map.iter().any(|map| {
                    usize::from(map.source) >= from
                        || usize::from(map.destination) >= to
                        || !map.gain.is_finite()
                        || !(-1.0..=1.0).contains(&map.gain)
                })
            {
                return Err("Invalid routing direction or channel map".into());
            }
            graph.entry(connection.source.group).or_default();
            graph
                .entry(connection.destination)
                .or_default()
                .insert(connection.source.group);
        }
        if let Some(id) = self.monitor_output {
            let monitor = self.port(id, Direction::Output).filter(|port| port.channels.len() == 2)
                .ok_or("Headphones require a distinct stereo output alias")?;
            for connection in &self.connections {
                if let Group::Output(output) = connection.destination {
                    let port = self.port(output, Direction::Output).unwrap();
                    if output == id || connection.map.iter().any(|map| monitor.channels.contains(&port.channels[usize::from(map.destination)])) {
                        return Err("Headphone channels overlap a program output route".into());
                    }
                }
            }
        }
        let mut order = Vec::with_capacity(graph.len());
        while !graph.is_empty() {
            let ready = graph
                .iter()
                .find(|(_, sources)| sources.is_empty())
                .map(|(group, _)| *group)
                .ok_or("Routing creates a feedback cycle; no change was applied")?;
            graph.remove(&ready);
            for sources in graph.values_mut() {
                sources.remove(&ready);
            }
            order.push(ready);
        }
        Ok(order)
    }

    /// Resolve a destination's input width.
    /// Takes the saved group and session; returns distinct plugin input buses or the native endpoint width.
    pub(crate) fn input_width(&self, group: Group, layout: &Layout) -> Option<usize> {
        if let Group::Plugin(id) = group { self.plugins.iter().find(|p| p.id == id).map(super::plugins::Instance::input_width) }
        else { self.width(group, layout) }
    }
    /// Resolve the channel width at an explicit source tap.
    /// Takes its saved tap and retained session; returns raw plugin inputs or processed outputs.
    pub(crate) fn source_width(&self, source: Source, layout: &Layout) -> Option<usize> {
        if source.tap == Tap::PreFx && matches!(source.group, Group::Plugin(_)) { self.input_width(source.group, layout) }
        else { self.width(source.group, layout) }
    }
}
