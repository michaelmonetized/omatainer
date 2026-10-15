//! Named auxiliary routing uses the existing acyclic graph and sample clock.
use super::{model::*, prepared::Prepared};
use serde::{Deserialize, Serialize};

pub const MAX_SENDS: usize = 256;
pub const MAX_LANE_POINTS: usize = 16384;
pub const MAX_MIXER_POINTS: usize = 65536;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Parameter {
    Level,
    Pan,
    Mute,
    Solo,
    SoloSafe,
}
impl Parameter {
    /// Validate a mixer value.
    /// Takes a scalar; returns whether it belongs to this parameter's domain.
    pub fn valid(self, value: f32) -> bool {
        value.is_finite()
            && match self {
                Self::Level => (0.0..=1.5).contains(&value),
                Self::Pan => (-1.0..=1.0).contains(&value),
                _ => (0.0..=1.0).contains(&value),
            }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Lane {
    pub parameter: Parameter,
    pub points: Vec<[f64; 2]>,
}
impl Lane {
    /// Read one deterministic automation value.
    /// Takes song beats; returns endpoint-held linear levels/pans or stepped switches.
    pub fn at(&self, beat: f64) -> f32 {
        let next = self.points.partition_point(|p| p[0] <= beat);
        if next == 0 {
            return self.points[0][1] as f32;
        }
        let old = self.points[next - 1];
        self.points.get(next).map_or(old[1], |p| {
            if matches!(self.parameter, Parameter::Level | Parameter::Pan) {
                old[1] + (p[1] - old[1]) * ((beat - old[0]) / (p[0] - old[0]))
            } else {
                old[1]
            }
        }) as f32
    }
}
/// Admit bounded mixer automation.
/// Takes lanes and whether the target is a return; returns a refusal before installing invalid or oversized state.
pub fn validate_lanes(lanes: &[Lane], returns: bool) -> Result<(), String> {
    let mut ids = std::collections::BTreeSet::new();
    let mut count = 0;
    for lane in lanes {
        count += lane.points.len();
        if !ids.insert(lane.parameter)
            || !returns && matches!(lane.parameter, Parameter::Solo | Parameter::SoloSafe)
            || lane.points.is_empty()
            || count > MAX_LANE_POINTS
            || lane.points.iter().any(|p| {
                !p[0].is_finite()
                    || !(0.0..=1.0e9).contains(&p[0])
                    || !p[1].is_finite()
                    || !lane.parameter.valid(p[1] as f32)
            })
            || lane.points.windows(2).any(|p| p[0][0] >= p[1][0])
        {
            return Err(
                "Mixer automation needs unique parameters and ordered bounded points".into(),
            );
        }
    }
    Ok(())
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Strip {
    pub pan: f32,
    pub solo: bool,
    pub solo_safe: bool,
    pub automation: Vec<Lane>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Send {
    pub id: u64,
    pub name: String,
    pub route: Connection,
    pub level: f32,
    pub pan: f32,
    pub mute: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub automation: Vec<Lane>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "id", rename_all = "snake_case")]
pub enum Target {
    Send(u64),
    Return(u64),
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Assignment {
    pub namespace: [u64; 2],
    pub target: Target,
    pub parameter: Parameter,
}
impl Assignment {
    /// Validate a saved MIDI mixer assignment.
    /// Takes this identity; returns whether it addresses an explicit project and supported target parameter.
    pub fn valid(self) -> bool {
        self.namespace != [0, 0]
            && match self.target {
                Target::Send(id) => {
                    id != 0 && !matches!(self.parameter, Parameter::Solo | Parameter::SoloSafe)
                }
                Target::Return(id) => id != 0,
            }
    }
    /// Convert a normalized MIDI value to a typed routing command.
    /// Takes a finite normalized control; returns the same namespaced command used by UI and automation.
    pub fn command(self, value: f32) -> crate::engine::Command {
        crate::engine::Command::RoutingMix {
            namespace: self.namespace,
            target: self.target,
            parameter: self.parameter,
            value: match self.parameter {
                Parameter::Level => value * 1.5,
                Parameter::Pan => value * 2. - 1.,
                _ => value,
            },
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Name {
    bytes: [u8; 128],
    len: u8,
}
impl Name {
    /// Retain an admitted routing name in fixed callback storage.
    /// Takes validated UTF-8 text of at most 128 bytes; returns a copyable display name.
    fn new(text: &str) -> Self {
        let mut bytes = [0; 128];
        bytes[..text.len()].copy_from_slice(text.as_bytes());
        Self {
            bytes,
            len: text.len() as u8,
        }
    }
    /// Read the original routing name.
    /// Takes this retained name; returns its validated UTF-8 view without allocation.
    pub fn text(&self) -> &str {
        std::str::from_utf8(&self.bytes[..usize::from(self.len)]).expect("Validated routing name")
    }
}
impl serde::Serialize for Name {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.text())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Values {
    pub level: f32,
    pub pan: f32,
    pub mute: bool,
    pub solo: bool,
    pub solo_safe: bool,
}
impl Values {
    /// Apply a validated control value.
    /// Takes parameter and scalar; returns false without mutation for invalid input.
    pub fn set(&mut self, parameter: Parameter, value: f32) -> bool {
        if !parameter.valid(value) {
            return false;
        }
        match parameter {
            Parameter::Level => self.level = value,
            Parameter::Pan => self.pan = value,
            Parameter::Mute => self.mute = value >= 0.5,
            Parameter::Solo => self.solo = value >= 0.5,
            Parameter::SoloSafe => self.solo_safe = value >= 0.5,
        }
        true
    }
    /// Read the channel gain for an explicit map.
    /// Takes its prepared left/right balance index, or any other index for a mono/unbalanced channel; returns the existing unity-center stereo balance law.
    pub fn gain(self, channel: usize) -> f32 {
        if self.mute {
            0.
        } else {
            match channel {
                0 => (1. - self.pan.max(0.)).sqrt() * self.level,
                1 => (1. + self.pan.min(0.)).sqrt() * self.level,
                _ => self.level,
            }
        }
    }
}
impl Bus {
    /// Read the saved return mixer.
    /// Takes this bus; returns its legacy gain/mute and default-compatible extra controls.
    pub fn values(&self) -> Values {
        Values {
            level: self.gain,
            pan: self.mixer.pan,
            mute: self.mute,
            solo: self.mixer.solo,
            solo_safe: self.mixer.solo_safe,
        }
    }
}
impl Send {
    /// Read a saved send mixer.
    /// Takes this send; returns its live scalar controls without route allocation.
    pub fn values(&self) -> Values {
        Values {
            level: self.level,
            pan: self.pan,
            mute: self.mute,
            solo: false,
            solo_safe: false,
        }
    }
}
#[derive(Clone, Copy, Debug, Serialize)]
pub struct Channel {
    pub name: Name,
    pub width: u8,
    pub target: Target,
    pub values: Values,
    pub effective: Values,
    pub peaks: [f32; MAX_PORT_CHANNELS],
    #[serde(skip)]
    pub automation_delay: u32,
    #[serde(skip)]
    automation_node: usize,
    #[serde(skip)]
    automation_tap: Option<usize>,
}
#[derive(Debug)]
pub struct Runtime {
    pub channels: Vec<Channel>,
    pub ancestry: Vec<u32>,
    pub feeders: Vec<u32>,
    pub solo: u32,
    pub safe: u32,
}
impl Runtime {
    /// Prepare live controls and return ancestry outside the callback.
    /// Takes the saved graph and topological nodes; returns bounded scalar and meter storage.
    pub fn new(
        model: &Model,
        order: &[Group],
        layout: &crate::engine::session::Layout,
        plan: Option<&super::latency::Plan>,
    ) -> Self {
        let channels = model
            .sends
            .iter()
            .map(|s| {
                (
                    Target::Send(s.id),
                    s.values(),
                    Name::new(&s.name),
                    model.input_width(s.route.destination, layout).unwrap() as u8,
                )
            })
            .chain(model.buses.iter().map(|b| {
                (
                    Target::Return(b.id),
                    b.values(),
                    Name::new(&b.alias),
                    b.channels,
                )
            }))
            .map(|(target, values, name, width)| {
                let (group, tap) = match target {
                    Target::Send(id) => (
                        model
                            .sends
                            .iter()
                            .find(|s| s.id == id)
                            .unwrap()
                            .route
                            .destination,
                        None,
                    ),
                    Target::Return(id) => model
                        .plugins
                        .iter()
                        .find(|p| {
                            p.return_bus == Some(id)
                                && !model.links().any(|r| {
                                    r.source.group == Group::Plugin(p.id)
                                        && model.plugins.iter().any(|next| {
                                            r.destination == Group::Plugin(next.id)
                                                && next.return_bus == Some(id)
                                        })
                                })
                        })
                        .map_or((Group::Bus(id), None), |p| {
                            (Group::Plugin(p.id), Some(Tap::PostFx.index()))
                        }),
                };
                let node = order.iter().position(|g| *g == group).unwrap();
                Channel {
                    name,
                    width,
                    target,
                    values,
                    effective: values,
                    peaks: [0.; MAX_PORT_CHANNELS],
                    automation_delay: plan
                        .map_or(0, |p| tap.map_or(p.inputs[node], |tap| p.taps[node][tap])),
                    automation_node: node,
                    automation_tap: tap,
                }
            })
            .collect();
        let indices = order
            .iter()
            .enumerate()
            .map(|(i, g)| (*g, i))
            .collect::<std::collections::BTreeMap<_, _>>();
        let mut edges = model
            .links()
            .map(|r| {
                let audible = if let Group::Plugin(id) = r.destination {
                    r.map.iter().any(|m| {
                        usize::from(m.destination)
                            < usize::from(
                                model
                                    .plugins
                                    .iter()
                                    .find(|p| p.id == id)
                                    .unwrap()
                                    .inputs
                                    .first()
                                    .copied()
                                    .unwrap_or(0),
                            )
                    })
                } else {
                    true
                };
                (indices[&r.source.group], indices[&r.destination], audible)
            })
            .collect::<Vec<_>>();
        for track in &layout.tracks {
            let source = indices[&Group::Track(track.id)];
            if !model.tracks_without_default_send.contains(&track.id) {
                for scene in &layout.scenes {
                    edges.push((source, indices[&Group::Scene(scene.id)], true));
                }
            }
            for send in 0..2 {
                edges.push((source, indices[&Group::FixedReturn(send)], true));
            }
        }
        for scene in &layout.scenes {
            edges.push((
                indices[&Group::Scene(scene.id)],
                indices[&Group::Main],
                true,
            ));
        }
        for plugin in &model.plugins {
            if plugin.scene_track.is_some() {
                for scene in &layout.scenes {
                    edges.push((
                        indices[&Group::Plugin(plugin.id)],
                        indices[&Group::Scene(scene.id)],
                        true,
                    ));
                }
            }
        }
        for deck in 0..2 {
            if !model.decks_without_default_send[deck] {
                edges.push((
                    indices[&Group::Deck(deck as u8)],
                    indices[&Group::Main],
                    true,
                ));
            }
        }
        for send in 0..2 {
            if !model.fixed_returns_without_default_send[send] {
                edges.push((
                    indices[&Group::FixedReturn(send as u8)],
                    indices[&Group::Main],
                    true,
                ));
            }
        }
        let mut ancestry = vec![0u32; order.len()];
        let mut feeders = vec![0u32; order.len()];
        for (index, group) in order.iter().enumerate() {
            if let Group::Bus(id) = group {
                let bit = 1u32 << model.buses.iter().position(|b| b.id == *id).unwrap();
                ancestry[index] = bit;
                feeders[index] = bit;
            }
        }
        for index in 0..order.len() {
            for &(source, _, audible) in edges.iter().filter(|e| e.1 == index) {
                if audible {
                    let carried = ancestry[source];
                    ancestry[index] |= carried;
                }
            }
        }
        for index in (0..order.len()).rev() {
            for &(_, destination, _) in edges.iter().filter(|e| e.0 == index) {
                let carried = feeders[destination];
                feeders[index] |= carried;
            }
        }
        Self {
            channels,
            ancestry,
            feeders,
            solo: 0,
            safe: 0,
        }
    }
    /// Gate audible sums while retaining source and sidechain processing.
    /// Takes source/destination indices and terminal/sidechain classification; returns whether this contribution belongs to a soloed or solo-safe path or feeds one.
    pub(super) fn audible(
        &self,
        source: usize,
        destination: usize,
        terminal: bool,
        sidechain: bool,
    ) -> bool {
        let selected = self.solo | self.safe;
        self.solo == 0
            || sidechain
            || self.feeders[destination] & selected != 0
            || self.ancestry[source] & selected != 0
            || !terminal && self.ancestry[destination] & selected == 0
    }
    /// Follow the current native compensation plan.
    /// Takes admitted delay tables; refreshes scalar lane clocks after offline-floor or native latency adoption without graph searches or allocation.
    pub(super) fn delays(&mut self, plan: Option<&super::latency::Plan>) {
        for channel in &mut self.channels {
            channel.automation_delay = plan.map_or(0, |p| {
                channel
                    .automation_tap
                    .map_or(p.inputs[channel.automation_node], |tap| {
                        p.taps[channel.automation_node][tap]
                    })
            });
        }
    }
    /// Evaluate sample-accurate saved automation.
    /// Takes graph and current song clock; updates effective controls while leaving recalled manual values intact.
    pub fn evaluate(&mut self, model: &Model, beat: f64, playing: bool) {
        self.evaluate_at(model, playing, |_| beat);
    }
    /// Evaluate controls at the source time of each compensated sum.
    /// Takes the graph, playback state and delay-to-beat converter; updates each lane at its own aligned sample without allocating.
    pub fn evaluate_at(&mut self, model: &Model, playing: bool, beat: impl Fn(u32) -> f64) {
        self.solo = 0;
        self.safe = 0;
        for (index, channel) in self.channels.iter_mut().enumerate() {
            channel.effective = channel.values;
            let lanes = if index < model.sends.len() {
                &model.sends[index].automation
            } else {
                &model.buses[index - model.sends.len()].mixer.automation
            };
            if playing {
                for lane in lanes {
                    channel
                        .effective
                        .set(lane.parameter, lane.at(beat(channel.automation_delay)));
                }
            }
            for peak in &mut channel.peaks[..usize::from(channel.width)] {
                *peak *= 0.9995;
            }
            if index >= model.sends.len() && channel.effective.solo_safe {
                self.safe |= 1u32 << (index - model.sends.len());
            }
            if index >= model.sends.len() && channel.effective.solo {
                self.solo |= 1u32 << (index - model.sends.len());
            }
        }
    }
    /// Save manually edited values into a worker-owned graph.
    /// Takes the captured controls and mutable model; returns after copying values without changing identities or lanes.
    pub fn save(channels: &[Channel], model: &mut Model) {
        for channel in channels {
            match channel.target {
                Target::Send(id) => {
                    if let Some(s) = model.sends.iter_mut().find(|s| s.id == id) {
                        s.level = channel.values.level;
                        s.pan = channel.values.pan;
                        s.mute = channel.values.mute;
                    }
                }
                Target::Return(id) => {
                    if let Some(b) = model.buses.iter_mut().find(|b| b.id == id) {
                        b.gain = channel.values.level;
                        b.mute = channel.values.mute;
                        b.mixer.pan = channel.values.pan;
                        b.mixer.solo = channel.values.solo;
                        b.mixer.solo_safe = channel.values.solo_safe;
                    }
                }
            }
        }
    }
}
impl Prepared {
    /// Resolve a stable live mixer identity.
    /// Takes a target; returns its current manual values if the graph still retains it.
    pub fn mix_values(&self, target: Target) -> Option<Values> {
        self.mixer
            .channels
            .iter()
            .find(|c| c.target == target)
            .map(|c| c.values)
    }
    /// Edit a stable live mixer without rebuilding the graph.
    /// Takes target, parameter and value; returns false for stale or invalid controls.
    pub fn mix_control(&mut self, target: Target, parameter: Parameter, value: f32) -> bool {
        if matches!(target, Target::Send(_))
            && matches!(parameter, Parameter::Solo | Parameter::SoloSafe)
        {
            return false;
        }
        self.mixer
            .channels
            .iter_mut()
            .find(|c| c.target == target)
            .is_some_and(|c| c.values.set(parameter, value))
    }
    /// Swap a complete inverse mixer value.
    /// Takes a retained target and mutable inverse; returns whether the atomic history swap applied.
    pub fn mix_swap(&mut self, target: Target, value: &mut Values) -> bool {
        if let Some(c) = self.mixer.channels.iter_mut().find(|c| c.target == target) {
            std::mem::swap(&mut c.values, value);
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests;

impl Model {
    /// Duplicate a track's owned processors and explicit routing.
    /// Takes old/new stable identities and the prepared destination layout; returns a validated graph retaining shared returns and independent sidechain feeders.
    pub(crate) fn duplicate_track(
        &self,
        from: crate::engine::session::Id,
        to: crate::engine::session::Id,
        layout: &crate::engine::session::Layout,
    ) -> Result<Self, String> {
        let mut next = self.clone();
        let mut ids = std::collections::BTreeMap::new();
        for plugin in self
            .plugins
            .iter()
            .filter(|p| p.midi_track == Some(from) || p.scene_track == Some(from))
        {
            let id = next.next_id;
            next.next_id = id
                .checked_add(1)
                .ok_or("Duplicated processors exhausted routing identities")?;
            ids.insert(plugin.id, id);
            let mut plugin = plugin.clone();
            plugin.id = id;
            plugin.name = format!("{} copy {id}", plugin.name);
            if plugin.midi_track == Some(from) {
                plugin.midi_track = Some(to);
            }
            if plugin.scene_track == Some(from) {
                plugin.scene_track = Some(to);
            }
            next.plugins.push(plugin);
        }
        let owned = |g: Group| {
            g == Group::Track(from) || matches!(g,Group::Plugin(id)if ids.contains_key(&id))
        };
        let remap = |g: Group| match g {
            Group::Track(id) if id == from => Group::Track(to),
            Group::Plugin(id) => Group::Plugin(ids.get(&id).copied().unwrap_or(id)),
            _ => g,
        };
        let route = |r: &Connection| {
            let mut r = r.clone();
            r.source.group = remap(r.source.group);
            r.destination = remap(r.destination);
            r
        };
        next.speaker_layouts.extend(self.speaker_layouts.iter().filter(|b|owned(b.group)).map(|b|{let mut b=*b;b.group=remap(b.group);b}));
        next.connections.extend(
            self.connections
                .iter()
                .filter(|r| owned(r.source.group) || owned(r.destination))
                .map(route),
        );
        for send in self
            .sends
            .iter()
            .filter(|s| owned(s.route.source.group) || owned(s.route.destination))
        {
            let mut send = send.clone();
            send.id = next.next_id;
            next.next_id = next
                .next_id
                .checked_add(1)
                .ok_or("Duplicated sends exhausted routing identities")?;
            send.name = format!("{} copy {}", send.name, send.id);
            send.route = route(&send.route);
            next.sends.push(send);
        }
        if self.tracks_without_default_send.contains(&from) {
            next.tracks_without_default_send.push(to);
        }
        if let Some(latency) = &mut next.latency {
            if let Some(original) = &self.latency {
                latency
                    .reports
                    .extend(original.reports.iter().filter(|r| owned(r.group)).map(|r| {
                        let mut r = *r;
                        r.group = remap(r.group);
                        r
                    }));
            }
        }
        next.order(layout)?;
        Ok(next)
    }
}
