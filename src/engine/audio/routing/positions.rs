use super::{latency::Plan, model::*};
use crate::engine::{audible::Source as SourcePosition, session::Layout, DeckRt, DECKS};
use std::collections::BTreeMap;
#[cfg(test)]
mod tests;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Delay {
    #[default]
    Absent,
    Frames(u32),
    Mixed,
}
impl Delay {
    fn merge(self, other: Self) -> Self {
        match (self, other) {
            (Self::Absent, value) | (value, Self::Absent) => value,
            (Self::Frames(a), Self::Frames(b)) if a == b => self,
            _ => Self::Mixed,
        }
    }
    fn delayed(self, frames: u32) -> Self {
        match self {
            Self::Frames(delay) => delay.checked_add(frames).map_or(Self::Mixed, Self::Frames),
            _ => self,
        }
    }
}
#[derive(Debug)]
pub(super) struct Positions {
    physical: [[Delay; DECKS]; MAX_PHYSICAL_CHANNELS],
    history: Box<[[SourcePosition; DECKS]]>,
    head: usize,
    filled: usize,
    namespace: [u64; 2],
    generation: u64,
    processors: bool,
    geometry: Box<[(crate::engine::session::Id, bool)]>,
    tracks: usize,
    bound: bool,
}
impl Positions {
    /// Count the retained source positions before admitting routing histories.
    /// Takes the maximum causal delay and retained layout; returns position-ring, identity and physical-path storage bytes.
    pub(super) fn storage_bytes(reserve: u32, layout: &Layout) -> usize {
        (reserve as usize + 1) * std::mem::size_of::<[SourcePosition; DECKS]>()
            + std::mem::size_of::<Self>()
            + (layout.tracks.len() + layout.scenes.len())
                * std::mem::size_of::<(crate::engine::session::Id, bool)>()
    }

    /// Prepare channel-specific source paths outside the callback.
    /// Takes saved routing, retained layout, ordered groups and actual compensation; returns bounded source histories with ambiguous routes explicitly unavailable.
    pub(super) fn new(
        model: &Model,
        layout: &Layout,
        order: &[Group],
        plan: Option<&Plan>,
    ) -> Self {
        let indices: BTreeMap<_, _> = order
            .iter()
            .enumerate()
            .map(|(index, group)| (*group, index))
            .collect();
        let mut taps = vec![[[[Delay::Absent; DECKS]; MAX_PORT_CHANNELS]; 3]; order.len()];
        let mut physical = [[Delay::Absent; DECKS]; MAX_PHYSICAL_CHANNELS];
        for (index, group) in order.iter().copied().enumerate() {
            let mut input = [[Delay::Absent; DECKS]; MAX_PORT_CHANNELS];
            let offset = |source: usize, tap: usize| {
                plan.map_or(0, |plan| {
                    plan.inputs[index].saturating_sub(plan.taps[source][tap])
                })
            };
            for connection in model
                .connections
                .iter()
                .filter(|connection| connection.destination == group)
            {
                let source = indices[&connection.source.group];
                let tap = connection.source.tap.index();
                for map in connection.map.iter().filter(|map| map.gain != 0.0) {
                    for deck in 0..DECKS {
                        let incoming = taps[source][tap][usize::from(map.source)][deck]
                            .delayed(offset(source, tap));
                        let destination = &mut input[usize::from(map.destination)][deck];
                        *destination = destination.merge(incoming);
                    }
                }
            }
            let mut implicit = |source: usize| {
                for channel in 0..2 {
                    for deck in 0..DECKS {
                        input[channel][deck] = input[channel][deck]
                            .merge(taps[source][2][channel][deck].delayed(offset(source, 2)));
                    }
                }
            };
            match group {
                Group::Scene(_) => {
                    for track in layout.tracks.iter().filter(|track| {
                        track.active && !model.tracks_without_default_send.contains(&track.id)
                    }) {
                        implicit(indices[&Group::Track(track.id)]);
                    }
                }
                Group::Main => {
                    for scene in layout.scenes.iter().filter(|scene| scene.active) {
                        implicit(indices[&Group::Scene(scene.id)]);
                    }
                    for deck in 0..DECKS {
                        if !model.decks_without_default_send[deck] {
                            implicit(indices[&Group::Deck(deck as u8)]);
                        }
                    }
                }
                _ => {}
            }
            let stereo = std::array::from_fn(|deck| {
                if input[0][deck] == input[1][deck] {
                    input[0][deck]
                } else {
                    Delay::Mixed
                }
            });
            let mut processed = input;
            match group {
                Group::Deck(deck) => {
                    input = [[Delay::Absent; DECKS]; MAX_PORT_CHANNELS];
                    input[0][usize::from(deck)] = Delay::Frames(0);
                    input[1][usize::from(deck)] = Delay::Frames(0);
                    processed = input;
                }
                Group::Input(_) => {
                    input = [[Delay::Absent; DECKS]; MAX_PORT_CHANNELS];
                    processed = input;
                }
                Group::Track(id)
                    if !layout
                        .tracks
                        .iter()
                        .any(|track| track.active && track.id == id) =>
                {
                    input = [[Delay::Absent; DECKS]; MAX_PORT_CHANNELS];
                    processed = input;
                }
                Group::Scene(id)
                    if !layout
                        .scenes
                        .iter()
                        .any(|scene| scene.active && scene.id == id) =>
                {
                    input = [[Delay::Absent; DECKS]; MAX_PORT_CHANNELS];
                    processed = input;
                }
                Group::Track(_) | Group::Scene(_) | Group::Main => {
                    processed[0] = stereo;
                    processed[1] = stereo;
                }
                Group::Plugin(_) => {
                    let mixed = std::array::from_fn(|deck| {
                        if input.iter().all(|channel| channel[deck] == Delay::Absent) {
                            Delay::Absent
                        } else {
                            Delay::Mixed
                        }
                    });
                    processed.fill(mixed);
                }
                _ => {}
            }
            let mut post_mixer = processed;
            if matches!(group, Group::Bus(id) if model.buses.iter().any(|bus| bus.id == id && (bus.mute || bus.gain == 0.0)))
            {
                post_mixer.fill([Delay::Absent; DECKS]);
            }
            taps[index] = [input, processed, post_mixer];
            if let Group::Output(id) = group {
                let port = model.port(id, Direction::Output).unwrap();
                for (channel, physical_channel) in port.channels.iter().enumerate() {
                    for deck in 0..DECKS {
                        let destination = &mut physical[usize::from(*physical_channel)][deck];
                        *destination = destination.merge(input[channel][deck]);
                    }
                }
            }
        }
        let frames = plan.map_or(1, |plan| plan.reserve as usize + 1);
        Self {
            physical,
            history: vec![[SourcePosition::default(); DECKS]; frames].into_boxed_slice(),
            head: frames - 1,
            filled: 0,
            namespace: layout.namespace,
            generation: layout.generation,
            processors: !model.plugins.is_empty(),
            geometry: layout
                .tracks
                .iter()
                .chain(&layout.scenes)
                .map(|item| (item.id, item.active))
                .collect(),
            tracks: layout.tracks.len(),
            bound: true,
        }
    }

    /// Bind admitted source paths to the final session generation.
    /// Takes the applied layout; preserves metadata-only edits and rejects changed storage identities without allocating in the callback.
    pub(super) fn bind(&mut self, layout: &Layout) {
        let compatible = self.namespace == layout.namespace
            && self.tracks == layout.tracks.len()
            && self.geometry.len() == layout.tracks.len() + layout.scenes.len()
            && self
                .geometry
                .iter()
                .zip(layout.tracks.iter().chain(&layout.scenes))
                .all(|((id, active), item)| *id == item.id && *active == item.active);
        if !compatible || !self.bound {
            self.reset();
        }
        self.bound = compatible;
        self.generation = layout.generation;
    }

    /// Retain the positions actually rendered and select their physical program paths.
    /// Takes current decks/layout, opened outputs, headphone override and alignment-transition state; returns source-qualified positions or zero-key renderer fallbacks.
    pub(super) fn capture(
        &mut self,
        decks: &[DeckRt; DECKS],
        layout: &Layout,
        channels: usize,
        monitor: Option<[usize; 2]>,
        transitioning: bool,
    ) -> [SourcePosition; DECKS] {
        self.head += 1;
        if self.head == self.history.len() {
            self.head = 0;
        }
        self.history[self.head] =
            std::array::from_fn(|deck| SourcePosition::from_deck(&decks[deck]));
        self.filled = (self.filled + 1).min(self.history.len());
        if transitioning
            || self.processors
            || !self.bound
            || layout.namespace != self.namespace
            || layout.generation != self.generation
        {
            return [SourcePosition::default(); DECKS];
        }
        let mut paths = [Delay::Absent; DECKS];
        for (channel, sources) in self
            .physical
            .iter()
            .enumerate()
            .take(channels.min(MAX_PHYSICAL_CHANNELS))
        {
            if monitor.is_some_and(|pair| pair.contains(&channel)) {
                continue;
            }
            for deck in 0..DECKS {
                paths[deck] = paths[deck].merge(sources[deck]);
            }
        }
        std::array::from_fn(|deck| {
            let Delay::Frames(delay) = paths[deck] else {
                return SourcePosition::default();
            };
            let delay = delay as usize;
            if delay >= self.filled {
                return SourcePosition::default();
            }
            let index = if delay <= self.head {
                self.head - delay
            } else {
                self.history.len() - (delay - self.head)
            };
            self.history[index][deck]
        })
    }

    /// Reuse rendered source positions during a compatible report edit.
    /// Takes the preceding prepared positions; swaps equal-size storage without callback allocation or deallocation.
    pub(super) fn inherit(&mut self, prior: &mut Self) {
        if self.history.len() == prior.history.len()
            && self.namespace == prior.namespace
            && self.geometry == prior.geometry
            && self.tracks == prior.tracks
            && prior.bound
        {
            std::mem::swap(&mut self.history, &mut prior.history);
            self.head = prior.head;
            self.filled = prior.filled;
        }
    }

    /// Retire source positions at a transport or recovery boundary.
    /// Takes this prepared history; invalidates old frames using its filled counter only.
    pub(super) fn reset(&mut self) {
        self.filled = 0;
    }

    /// Account for retained routing position storage.
    /// Takes this prepared history; returns its complete owned storage bytes.
    pub(super) fn bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + std::mem::size_of_val(&*self.history)
            + std::mem::size_of_val(&*self.geometry)
    }
}
