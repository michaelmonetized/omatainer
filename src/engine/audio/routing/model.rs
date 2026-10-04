//! Saved aliases and explicit channel maps; preparation never depends on device presence.
use crate::engine::session::{Id, Layout};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

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
    pub connections: Vec<Connection>,
    pub tracks_without_default_send: Vec<Id>,
    pub decks_without_default_send: [bool; 2],
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<InputConfig>,
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
        }
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
        }
    }

    /// Validate and order a complete routing graph.
    /// Takes the retained session; returns each processing group once, or rejects invalid maps and feedback.
    pub fn order(&self, layout: &Layout) -> Result<Vec<Group>, String> {
        layout.validate()?;
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
        if self.version != 1
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
        for deck in 0..2 {
            graph.insert(Group::Deck(deck), BTreeSet::new());
        }
        for bus in &self.buses {
            graph.insert(Group::Bus(bus.id), BTreeSet::new());
        }
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
                .width(connection.source.group, layout)
                .ok_or("Routing source no longer exists")?;
            let to = self
                .width(connection.destination, layout)
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
}
