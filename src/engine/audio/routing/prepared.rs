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
    pub taps: [Frame; 3],
    pub meter: Frame,
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
    pub scene_nodes: [Option<usize>; crate::engine::session::MAX_SCENES],
    pub main: usize,
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
        let nodes = order
            .into_iter()
            .map(|group| Node {
                group,
                slot: match group {
                    Group::Track(id) => layout.tracks.iter().position(|item| item.id == id),
                    Group::Scene(id) => layout.scenes.iter().position(|item| item.id == id),
                    _ => None,
                },
                width: model.width(group, layout).unwrap(),
                input: [0.0; MAX_PORT_CHANNELS],
                taps: [[0.0; MAX_PORT_CHANNELS]; 3],
                meter: [0.0; MAX_PORT_CHANNELS],
            })
            .collect();
        Ok(Self {
            model,
            nodes,
            incoming,
            scene_nodes,
            main,
        })
    }

    /// Begin one sample frame.
    /// Takes mutable prepared state; clears only bounded destination channels without allocation.
    pub fn begin(&mut self) {
        for node in &mut self.nodes {
            node.input[..node.width].fill(0.0);
        }
    }

    /// Sum a node's explicit incoming maps.
    /// Takes its prepared index; returns the mapped input after all earlier sources have rendered.
    pub fn gather(&mut self, index: usize) -> Frame {
        for link in &self.incoming[index] {
            for map in &link.map {
                let value =
                    self.nodes[link.source].taps[link.tap][usize::from(map.source)] * map.gain;
                self.nodes[index].input[usize::from(map.destination)] += value;
            }
        }
        self.nodes[index].input
    }

    /// Publish a node's three tap points.
    /// Takes its index and rendered frames; replaces samples and updates independent peak meters.
    pub fn publish(&mut self, index: usize, taps: [Frame; 3]) {
        let node = &mut self.nodes[index];
        node.taps = taps;
        for (meter, value) in node.meter[..node.width].iter_mut().zip(taps[2]) {
            *meter = if value.is_finite() {
                (*meter * 0.999).max(value.abs())
            } else {
                f32::INFINITY
            };
        }
    }

    /// Add one legacy stereo send.
    /// Takes its destination and stereo samples; sums into the existing mixer without changing alias maps.
    pub fn legacy_send(&mut self, index: usize, frame: [f32; 2]) {
        self.nodes[index].input[0] += frame[0];
        self.nodes[index].input[1] += frame[1];
    }

    /// Assemble physical outputs.
    /// Takes the active channel count; returns exact retained mappings, leaving absent channels silent.
    pub fn outputs(&self, channels: usize) -> [f32; MAX_PHYSICAL_CHANNELS] {
        let mut result = [0.0; MAX_PHYSICAL_CHANNELS];
        for node in &self.nodes {
            let Group::Output(id) = node.group else {
                continue;
            };
            let port = self.model.port(id, Direction::Output).unwrap();
            for (&destination, value) in port.channels.iter().zip(node.taps[2]) {
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

/// Expand a stereo tap into a routing frame.
/// Takes left/right samples; returns a zero-filled bounded channel frame.
pub fn stereo(frame: [f32; 2]) -> Frame {
    let mut result = [0.0; MAX_PORT_CHANNELS];
    result[..2].copy_from_slice(&frame);
    result
}
