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
    pub monitor_output: Option<(u64, [usize; 2])>,
    pub(super) latency: Option<super::latency::Runtime>,
}

impl Prepared {
    /// Prepare a 48 kHz routing fixture without connected devices.
    /// Takes a validated saved model and retained session; returns owned test frames and maps.
    #[cfg(test)]
    pub fn new(model: Arc<Model>, layout: &Layout) -> Result<Self, String> {
        Self::at_rate(model, layout, 48000)
    }

    /// Prepare routing and causal latency storage for the actual output clock.
    /// Takes saved aliases, retained session and logical output rate; returns a bounded graph before renderer admission.
    pub fn at_rate(model: Arc<Model>, layout: &Layout, rate: u32) -> Result<Self, String> {
        let order = model.order(layout)?;
        let latency = super::latency::Plan::new(&model, layout, &order, rate)?
            .map(|plan|super::latency::Runtime::new(plan,&order.iter().map(|group|model.width(*group,layout).unwrap()).collect::<Vec<_>>(),rate));
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
        let monitor_output = model.monitor_output.map(|id| {
            let port = model.port(id, Direction::Output).unwrap();
            (id, [usize::from(port.channels[0]), usize::from(port.channels[1])])
        });
        Ok(Self {
            model,
            nodes,
            incoming,
            outputs,
            scene_nodes,
            main,
            monitor_channels_free,
            monitor_output,
            latency,
        })
    }

    /// Begin one sample frame.
    /// Takes mutable prepared state; clears only bounded destination channels without allocation.
    pub fn begin(&mut self) {
        if let Some(latency)=&mut self.latency {latency.begin();}
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
                let(value,valid)=self.latency.as_ref().map_or((source[usize::from(map.source)],sources[link.source].valid),
                    |latency|latency.routed(link.source,link.tap,index,usize::from(map.source)));
                destination.input[usize::from(map.destination)] += value * map.gain;
                if map.gain != 0.0 {
                    destination.valid &= valid;
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
        if let Some(latency)=&mut self.latency {latency.publish(index,&node.taps,node.valid);}
    }

    /// Publish native stereo taps directly.
    /// Takes a stereo node index and three sample pairs; writes only its two channels.
    pub fn publish_stereo(&mut self, index: usize, taps: [[f32; 2]; 3]) {
        let node = &mut self.nodes[index];
        debug_assert_eq!(node.width, 2);
        for (destination, source) in node.taps.iter_mut().zip(taps) {
            destination[..2].copy_from_slice(&source);
        }
        if let Some(latency)=&mut self.latency {latency.publish(index,&node.taps,node.valid);}
    }

    /// Add one legacy stereo send.
    /// Takes its destination, stereo samples and continuity; sums into the existing mixer without changing alias maps.
    pub fn legacy_send(&mut self, index: usize, frame: [f32; 2], valid: bool) {
        self.nodes[index].input[0] += frame[0];
        self.nodes[index].input[1] += frame[1];
        self.nodes[index].valid &= valid;
    }

    /// Align an implicit send through its published original source.
    /// Takes source and destination indices plus fallback audio/continuity; reads the same prepared history used by explicit links.
    pub(super) fn source_send(&mut self,source:usize,destination:usize,frame:[f32;2],valid:bool) {
        if let Some(latency)=&self.latency {
            for channel in 0..2 {
                let(value,complete)=latency.routed(source,super::model::Tap::PostMixer.index(),destination,channel);
                self.nodes[destination].input[channel]+=value;self.nodes[destination].valid&=complete;
            }
        }else{self.legacy_send(destination,frame,valid);}
    }

    /// Reuse compatible source histories before an applied graph replaces its predecessor.
    /// Takes the active graph; preserves existing delay storage without callback allocation or deallocation.
    pub(crate) fn inherit(&mut self,old:&mut Self) {
        if !self.model.latency_edit_of(&old.model) { return; }
        if let(Some(latency),Some(prior))=(&mut self.latency,&mut old.latency) {
            latency.inherit(prior,&self.nodes,&old.nodes);
        }
    }

    /// Read the active graph's scalar timing state.
    /// Takes this graph; returns disabled timing when compensation is omitted.
    pub(crate) fn latency_status(&self) -> super::latency::Status {
        self.latency.as_ref().map_or(Default::default(), super::latency::Runtime::status)
    }

    /// Retire delayed samples after transport or safety ownership changes.
    /// Takes this graph; resets only history counters without allocating or freeing audio storage.
    pub(crate) fn reset_latency(&mut self) {
        if let Some(latency) = &mut self.latency { latency.reset(); }
    }

    /// Locate a selected output's delay inside the software graph.
    /// Takes a retained output alias; returns its causal render delay, excluding latency outside the output callback.
    pub(crate) fn output_delay(&self, alias: u64) -> u32 {
        self.latency.as_ref().and_then(|latency| self.nodes.iter().position(|node| matches!(node.group, Group::Output(id) | Group::Record(id) if id == alias)).map(|index| latency.plan.inputs[index])).unwrap_or(0)
    }

    /// Recreate an exact selected terminal mix at headphone time without advancing any source.
    /// Takes its prepared terminal index; returns ordered channel maps and original continuity before voice addition, or no alternate mix when compensation is disabled.
    pub(super) fn monitor_mix(&self, index: usize) -> Option<(Frame, bool)> {
        let latency = self.latency.as_ref()?;
        let mut frame = [0.0; MAX_PORT_CHANNELS];
        let mut valid = true;
        for link in &self.incoming[index] {
            for map in &link.map {
                let (value, complete) = latency.routed_monitor(link.source, link.tap, usize::from(map.source));
                frame[usize::from(map.destination)] += value * map.gain;
                if map.gain != 0.0 { valid &= complete; }
            }
        }
        Some((frame, valid))
    }

    /// Estimate a stopped graph's changed-rate storage before allocating its replacement.
    /// Takes retained session identity and prospective rate; returns complete charged graph bytes or a preparation refusal.
    pub(crate) fn rate_bytes(&self, layout: &Layout, rate: u32) -> Result<usize, String> {
        let order = self.model.order(layout)?;
        let next = super::latency::Plan::new(&self.model, layout, &order, rate)?;
        Ok(self.bytes().saturating_sub(self.latency_status().history_bytes) + next.map_or(0, |plan| plan.storage_bytes))
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
            + self.latency.as_ref().map_or(0,super::latency::Runtime::bytes)
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
