//! Prepared indices and fixed channel frames keep routing work off the allocator.
use super::model::*;
use crate::engine::session::Layout;
use std::collections::BTreeMap;
use std::sync::Arc;

pub type Frame = [f32; MAX_PORT_CHANNELS];

#[derive(Clone, Debug)]
pub struct Node {
    pub group: Group,
    pub slot: Option<usize>,
    pub width: usize,
    pub input: Frame,
    pub valid: bool,
    pub taps: [Frame; 3],
}

#[derive(Clone, Debug)]
struct Link {
    source: usize,
    tap: usize,
    map: Vec<ChannelMap>,
}

#[derive(Clone, Debug)]
pub struct Prepared {
    pub model: Arc<Model>,
    pub nodes: Vec<Node>,
    incoming: Vec<Vec<Link>>,
    outputs: Vec<usize>,
    pub scene_nodes: [Option<usize>; crate::engine::session::MAX_SCENES],
    pub main: usize,
    pub monitor_channels_free: bool,
}

impl Prepared {
    /// Prepare all routing storage.
    /// Takes a validated saved model and retained session; returns owned, indexed frames and maps.
    pub fn new(model: Arc<Model>, layout: &Layout) -> Result<Self, String> {
        let order = model.order(layout)?;
        let indices: BTreeMap<_, _> = order
            .iter()
            .enumerate()
            .map(|(index, group)| (*group, index))
            .collect();
        let mut incoming = vec![Vec::new(); order.len()];
        for connection in &model.connections {
            incoming[indices[&connection.destination]].push(Link {
                source: indices[&connection.source.group],
                tap: connection.source.tap.index(),
                map: connection.map.clone(),
            });
        }
        let scene_nodes = std::array::from_fn(|slot| {
            layout
                .scenes
                .get(slot)
                .map(|item| indices[&Group::Scene(item.id)])
        });
        let main = indices[&Group::Main];
        let outputs = order.iter().enumerate().filter_map(|(index, group)| {
            matches!(group, Group::Output(_)).then_some(index)
        }).collect();
        let nodes = order
            .into_iter()
            .map(|group| Node {
                group,
                slot: match group {
                    Group::Track(id) => layout.tracks.iter().position(|item| item.id == id),
                    Group::Scene(id) => layout.scenes.iter().position(|item| item.id == id),
                    Group::Input(id) | Group::Output(id) | Group::Record(id) => {
                        model.ports.iter().position(|port| port.id == id)
                    }
                    Group::Bus(id) => model.buses.iter().position(|bus| bus.id == id),
                    _ => None,
                },
                width: model.width(group, layout).unwrap(),
                input: [0.0; MAX_PORT_CHANNELS],
                valid: true,
                taps: [[0.0; MAX_PORT_CHANNELS]; 3],
            })
            .collect();
        let monitor_channels_free = !model.ports.iter().any(|port| port.direction == Direction::Output && port.channels.iter().any(|channel| matches!(channel, 2 | 3)));
        Ok(Self {
            model,
            nodes,
            incoming,
            outputs,
            scene_nodes,
            main,
            monitor_channels_free,
        })
    }

    /// Begin one sample frame.
    /// Takes mutable prepared state; clears only bounded destination channels without allocation.
    pub fn begin(&mut self) {
        for node in &mut self.nodes {
            node.input[..node.width].fill(0.0);
            node.valid = true;
        }
    }

    /// Sum a node's explicit incoming maps.
    /// Takes its prepared index; sums into its prepared input after all earlier sources have rendered.
    pub fn gather(&mut self, index: usize) {
        let (sources, destination) = self.nodes.split_at_mut(index);
        let destination = &mut destination[0];
        for link in &self.incoming[index] {
            let source = &sources[link.source].taps[link.tap];
            for map in &link.map {
                destination.input[usize::from(map.destination)] += source[usize::from(map.source)] * map.gain;
                if map.gain != 0.0 {
                    destination.valid &= sources[link.source].valid;
                }
            }
        }
    }

    /// Publish a node's three tap points.
    /// Takes its index and rendered frames; replaces only its bounded sample channels.
    pub fn publish(&mut self, index: usize, taps: [Frame; 3]) {
        let node = &mut self.nodes[index];
        for (destination, source) in node.taps.iter_mut().zip(&taps) {
            destination[..node.width].copy_from_slice(&source[..node.width]);
        }
    }

    /// Publish native stereo taps directly.
    /// Takes a stereo node index and three sample pairs; writes only its two channels.
    pub fn publish_stereo(&mut self, index: usize, taps: [[f32; 2]; 3]) {
        let node = &mut self.nodes[index];
        debug_assert_eq!(node.width, 2);
        for (destination, source) in node.taps.iter_mut().zip(taps) {
            destination[..2].copy_from_slice(&source);
        }
    }

    /// Add one legacy stereo send.
    /// Takes its destination, stereo samples and continuity; sums into the existing mixer without changing alias maps.
    pub fn legacy_send(&mut self, index: usize, frame: [f32; 2], valid: bool) {
        self.nodes[index].input[0] += frame[0];
        self.nodes[index].input[1] += frame[1];
        self.nodes[index].valid &= valid;
    }

    /// Assemble physical outputs.
    /// Takes the active channel count; returns exact retained mappings, leaving absent channels silent.
    pub fn outputs(&self, channels: usize) -> [f32; MAX_PHYSICAL_CHANNELS] {
        let mut result = [0.0; MAX_PHYSICAL_CHANNELS];
        for &index in &self.outputs {
            let node = &self.nodes[index];
            let port = &self.model.ports[node.slot.unwrap()];
            for (&destination, value) in port.channels.iter().zip(node.input) {
                if usize::from(destination) < channels.min(MAX_PHYSICAL_CHANNELS) {
                    result[usize::from(destination)] += value;
                }
            }
        }
        result
    }

    /// Count retained prepared storage.
    /// Takes this graph; returns its owned frame, index and channel-map bytes for budget checks.
    pub fn bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + self.model.bytes()
            + self.nodes.capacity() * std::mem::size_of::<Node>()
            + self.incoming.capacity() * std::mem::size_of::<Vec<Link>>()
            + self.outputs.capacity() * std::mem::size_of::<usize>()
            + self
                .incoming
                .iter()
                .map(|links| {
                    links.capacity() * std::mem::size_of::<Link>()
                        + links
                            .iter()
                            .map(|link| link.map.capacity() * std::mem::size_of::<ChannelMap>())
                            .sum::<usize>()
                })
                .sum::<usize>()
    }
}
