//! Prepared layout inverses; audio swaps owned metadata without allocating.
use super::{Axis, Id, Layout};
use crate::engine::{
    midi_edit::{Ack, Outcome},
    Command, RtEngine,
};
mod structural;
mod import;
use import::Import;
pub(crate) use import::Selection as ImportSelection;
#[cfg(test)]
mod tests;

#[derive(Clone, Debug)]
pub(crate) enum Action {
    SceneProperties { id: Id, properties: crate::engine::scene::Properties },
    Rename {
        axis: Axis,
        id: Id,
        name: String,
    },
    Color {
        axis: Axis,
        id: Id,
        color: Option<[u8; 3]>,
    },
    Move {
        axis: Axis,
        id: Id,
        position: usize,
    },
    Delete {
        axis: Axis,
        id: Id,
    },
}

#[derive(Clone, Debug)]
pub(crate) struct Request {
    pub(super) namespace: [u64; 2],
    pub(super) generation: u64,
    pub(super) epoch: u64,
    pub(crate) inverse: Option<Box<Inverse>>,
    pub(crate) ack: Ack,
    disruptive: bool,
    receipt: Option<(u64, u32)>,
}

#[derive(Clone, Debug)]
pub(crate) struct Inverse {
    layout: Layout,
    routing: Option<Option<Box<crate::engine::audio::routing::prepared::Prepared>>>,
    track_name: Option<(usize, String)>,
    focus: Option<Focus>,
    bus_mask: u128,
    scene_buses: [usize; super::MAX_TRACKS],
    pub(super) content: Option<Content>,
    reserved_heap: usize,
    // Both sides remain pinned, making undo's media registry independent of swap direction.
    media: Vec<std::sync::Arc<crate::engine::Sample>>,
    fx_storage: Vec<crate::engine::fx::FxId>,
    reserved_fx_bytes: usize,
}

#[derive(Clone, Debug)]
pub(super) enum Content {
    Import(Import),
    Track {
        slot: usize,
        node: Option<Box<crate::engine::TrackRt>>,
    },
    Scene {
        slot: usize,
        cells: Vec<Option<crate::engine::Clip>>,
        rack: Option<crate::engine::fx::FxChain>,
    },
}

#[derive(Clone, Debug)]
pub(crate) enum Structure {
    Track {
        name: String,
        audio: bool,
        position: usize,
    },
    Scene {
        name: String,
        position: usize,
    },
    Duplicate {
        axis: Axis,
        id: Id,
        name: String,
        position: usize,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Focus {
    track: usize,
    scene: usize,
    compose: Option<crate::engine::ComposeTarget>,
    fx: i16,
}

impl Request {
    /// Prepare an atomic routing edit.
    /// Takes a captured project, output rate and optional saved graph; returns a guarded, undoable edit and acknowledgment.
    pub(crate) fn routing(captured: crate::engine::project::Captured, rate: u32, model: Option<std::sync::Arc<crate::engine::audio::routing::model::Model>>) -> Result<(Self, Ack), String> {
        let layout = captured.state.session.as_ref().ok_or("Session identity is unavailable")?;
        layout.validate()?;
        if let Some(cfg)=captured.state.mic_aux {cfg.validate(model.as_deref()).map_err(str::to_owned)?;}
        let latency_only = model.as_deref().zip(captured.state.routing.as_deref()).is_some_and(|(next, old)| next.latency_edit_of(old));
        let prepared = model.map(|model| crate::engine::audio::routing::prepared::Prepared::at_rate(model, layout,rate).map(Box::new)).transpose()?;
        let ack = Ack::new();
        Ok((Self {
            namespace: layout.namespace, generation: layout.generation, epoch: captured.checkpoint.epoch,
            inverse: Some(Box::new(Inverse {
                layout: layout.clone(), routing: Some(prepared), track_name: None, focus: None,
                bus_mask: 0, scene_buses: [0; super::MAX_TRACKS], content: None,
                reserved_heap: 0, media: Vec::new(), fx_storage: Vec::new(), reserved_fx_bytes: 0,
            })), ack: ack.clone(), disruptive: !latency_only, receipt: Some((captured.revision, rate)),
        }, ack))
    }
    /// Producer only. No mutation has happened when validation returns an error.
    pub fn metadata(layout: &Layout, epoch: u64, action: Action) -> Result<(Self, Ack), String> {
        layout.validate()?;
        let disruptive = matches!(action, Action::Delete { .. });
        let mut next = layout.clone();
        let mut track_name = None;
        match action {
            Action::SceneProperties { id, properties } => next.scene_properties(id, properties)?,
            Action::Rename { axis, id, name } => {
                let slot = next
                    .resolve(axis, id)
                    .ok_or("Session identity no longer exists")?;
                next.rename(axis, id, name.clone())?;
                if axis == Axis::Track {
                    track_name = Some((slot, name));
                }
            }
            Action::Color { axis, id, color } => next.color(axis, id, color)?,
            Action::Move { axis, id, position } => next.reorder(axis, id, position)?,
            Action::Delete { axis, id } => {
                next.delete(axis, id)?;
            }
        }
        next.validate()?;
        let ack = Ack::new();
        Ok((
            Self {
                namespace: layout.namespace,
                generation: layout.generation,
                epoch,
                inverse: Some(Box::new(Inverse {
                    layout: next,
                    routing: None,
                    track_name,
                    focus: None,
                    bus_mask: 0,
                    scene_buses: [0; super::MAX_TRACKS],
                    content: None,
                    reserved_heap: 0,
                    media: Vec::new(),
                    fx_storage: Vec::new(),
                    reserved_fx_bytes: 0,
                })),
                ack: ack.clone(),
                disruptive,
                receipt: None,
            },
            ack,
        ))
    }
    /// Require the reviewed project revision when this edit applies.
    /// Takes the revision and sample rate; returns the prepared edit with its atomic conflict guard.
    pub(crate) fn at_revision(mut self, revision: u64, rate: u32) -> Self {
        self.receipt = Some((revision, rate)); self
    }

    pub(crate) fn track_count(&self) -> usize {
        self.inverse
            .as_ref()
            .map_or(0, |inverse| inverse.layout.tracks.len())
    }
    pub(crate) fn disruptive(&self) -> bool {
        self.disruptive
    }
    pub(crate) fn bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + self.inverse.as_ref().map_or(0, |inverse| {
                inverse.bytes().saturating_add(inverse.media_bytes())
            })
    }
    pub(crate) fn current(&self, rt: &RtEngine) -> bool {
        self.ack.state() == Outcome::Pending
            && self.epoch == rt.undo.checkpoint().epoch
            && self.namespace == rt.session.namespace
            && self.generation == rt.session.generation
            && rt.session.generation < u64::MAX
            && self.inverse.as_ref().is_none_or(|inverse| inverse.routing.is_none() || inverse.content.is_some()
                || !self.disruptive && !rt.recording && !rt.routing_pipe.recorder.busy()
                    && rt.routing.as_ref().is_some_and(|graph| graph.latency_status().transition_frames == 0 && graph.latency_status().priming_frames == 0)
                || !rt.playing && !rt.recording && !rt.decks.iter().any(|deck| deck.playing || deck.touching))
            && self.receipt.is_none_or(|(revision, rate)| {
                rt.project.revision() == revision && rt.sr as u32 == rate
            })
            && self
                .inverse
                .as_ref()
                .is_some_and(|inverse| inverse.valid(rt))
    }
}

impl Inverse {
    pub(crate) fn processor_delta(&self, rt: &RtEngine) -> i128 {
        let bytes = |rack: &crate::engine::fx::FxChain| {
            rack.slots
                .iter()
                .map(crate::engine::fx::FxSlot::storage_bytes)
                .sum::<usize>() as i128
        };
        match &self.content {
            Some(Content::Import(import)) => import.processor_delta(rt),
            Some(Content::Track { slot, node }) => {
                node.as_ref().map_or(0, |track| bytes(&track.fx))
                    - rt.tracks.get(*slot).map_or(0, |track| bytes(&track.fx))
            }
            Some(Content::Scene { slot, rack, .. }) => {
                rack.as_ref().map_or(0, bytes) - rt.scene_fx.get(*slot).map_or(0, bytes)
            }
            None => 0,
        }
    }
    pub(crate) fn bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + self.layout.tracks.capacity() * std::mem::size_of::<super::Item>()
            + self.layout.scenes.capacity() * std::mem::size_of::<super::Item>()
            + self.layout.track_order.capacity()
            + 2 * self.layout.scene_order.capacity()
            + self
                .layout
                .tracks
                .iter()
                .chain(&self.layout.scenes)
                .map(|item| item.name.capacity())
                .sum::<usize>()
            + self.reserved_heap
            + self.routing.as_ref().and_then(Option::as_ref).map_or(0, |graph| graph.bytes())
            + self.fx_storage.capacity() * std::mem::size_of::<crate::engine::fx::FxId>()
            + self.media.capacity() * std::mem::size_of::<std::sync::Arc<crate::engine::Sample>>()
            + self
                .track_name
                .as_ref()
                .map_or(0, |(_, name)| name.capacity())
    }
    pub(crate) fn valid(&self, rt: &RtEngine) -> bool {
        rt.mic_aux.configuration().is_none_or(|cfg|cfg.validate(self.routing.as_ref().map_or_else(||rt.routing.as_ref().map(|r|r.model.as_ref()),|routing|routing.as_ref().map(|r|r.model.as_ref()))).is_ok())
            && self.layout.namespace == rt.session.namespace
            && rt.session.generation < u64::MAX
            && match &self.content {
                Some(Content::Import(import)) => import.valid(rt),
                None => {
                    self.layout.tracks.len() == rt.tracks.len()
                        && self.layout.scenes.len() == rt.scene_fx.len()
                }
                Some(Content::Track { slot, node }) => {
                    self.layout.scenes.len() == rt.scene_fx.len()
                        && self.layout.tracks.len().abs_diff(rt.tracks.len()) <= 1
                        && (*slot < rt.tracks.len() || *slot == rt.tracks.len() && node.is_some())
                        && rt.tracks.capacity() >= self.layout.tracks.len()
                }
                Some(Content::Scene { slot, cells, rack }) => {
                    self.layout.tracks.len() == rt.tracks.len()
                        && self.layout.scenes.len().abs_diff(rt.scene_fx.len()) <= 1
                        && cells.len() == rt.tracks.len()
                        && (*slot < rt.scene_fx.len()
                            || *slot == rt.scene_fx.len()
                                && rack.is_some()
                                && cells.iter().all(Option::is_some))
                        && rt.scene_fx.capacity() >= self.layout.scenes.len()
                        && rt
                            .tracks
                            .iter()
                            .all(|t| t.clips.capacity() >= self.layout.scenes.len())
                }
            }
    }
    pub(crate) fn swap(&mut self, rt: &mut RtEngine) {
        rt.routing_pipe.recorder.invalidate();
        let generation = rt.session.generation + 1;
        let next_id = rt.session.next_id.max(self.layout.next_id);
        let current_focus = Focus {
            track: rt.selected_track,
            scene: rt.selected_scene,
            compose: rt.compose_target,
            fx: rt.fx_view,
        };
        for track in 0..rt.tracks.len() {
            if self.bus_mask & (1u128 << track) != 0 {
                if rt.tracks[track]
                    .playing
                    .is_none_or(|p| usize::from(p.scene) == self.scene_buses[track])
                {
                    std::mem::swap(
                        &mut rt.tracks[track].scene_bus,
                        &mut self.scene_buses[track],
                    );
                } else {
                    self.scene_buses[track] = rt.tracks[track].scene_bus;
                }
            }
        }
        for slot in 0..rt.session.tracks.len() {
            if rt.session.tracks[slot].active
                && self
                    .layout
                    .tracks
                    .get(slot)
                    .is_none_or(|next| !next.active || next.id != rt.session.tracks[slot].id)
            {
                rt.finish_recording_track(slot);
                rt.tracks[slot].stop_clip();
                rt.tracks[slot].poly.release_all();
                rt.tracks[slot].drum_pos.fill(None);
                rt.midi_routing.clear_track(slot as u8);
                for voice in rt.pad_voices.iter_mut().flatten() {
                    if voice.track == slot {
                        voice.position = voice.end;
                    }
                }
            }
        }
        for slot in 0..rt.session.scenes.len() {
            if rt.session.scenes[slot].active
                && self
                    .layout
                    .scenes
                    .get(slot)
                    .is_none_or(|next| !next.active || next.id != rt.session.scenes[slot].id)
            {
                for track in 0..rt.tracks.len() {
                    rt.tracks[track].launch.cancel_scene(slot as u16);
                    if rt.tracks[track].scene_bus == slot {
                        self.scene_buses[track] = slot;
                        self.bus_mask |= 1u128 << track;
                        rt.tracks[track].scene_bus = usize::from(self.layout.scene_order[0]);
                    }
                    if rt.tracks[track]
                        .playing
                        .or(rt.tracks[track].project_resume)
                        .is_some_and(|p| usize::from(p.scene) == slot)
                    {
                        rt.finish_recording_track(track);
                        rt.tracks[track].stop_clip();
                    }
                }
            }
        }
        if let Some(content) = &mut self.content {
            content.swap(rt, &self.layout);
        }
        std::mem::swap(&mut rt.session, &mut self.layout);
        if let Some(routing) = &mut self.routing {
            if let(Some(next),Some(prior))=(routing.as_mut(),rt.routing.as_mut()){next.inherit(prior);}
            std::mem::swap(&mut rt.routing, routing);
        }
        rt.session.generation = generation;
        rt.session.next_id = next_id;
        rt.midi_routing.identity.publish(&rt.session);
        if let Some((slot, name)) = &mut self.track_name {
            std::mem::swap(&mut rt.tracks[*slot].name, name);
        }
        if let Some(focus) = self.focus {
            rt.selected_track = focus.track;
            rt.selected_scene = focus.scene;
            rt.compose_target = focus.compose;
            rt.fx_view = focus.fx;
        } else {
            if rt
                .session
                .tracks
                .get(rt.selected_track)
                .is_none_or(|item| !item.active)
            {
                rt.selected_track = usize::from(rt.session.track_order[0]);
            }
            if rt
                .session
                .scenes
                .get(rt.selected_scene)
                .is_none_or(|item| !item.active)
            {
                rt.selected_scene = usize::from(rt.session.scene_order[0]);
            }
            if rt.compose_target.is_some_and(|target| {
                rt.session
                    .tracks
                    .get(target.track)
                    .is_none_or(|item| !item.active)
                    || rt
                        .session
                        .scenes
                        .get(target.scene)
                        .is_none_or(|item| !item.active)
            }) {
                rt.compose_target = None;
            }
            if rt.fx_view >= super::SCENE_FX_BASE
                && rt
                    .session
                    .scenes
                    .get((rt.fx_view - super::SCENE_FX_BASE) as usize)
                    .is_none_or(|item| !item.active)
                || rt.fx_view >= 0
                    && rt.fx_view < super::SCENE_FX_BASE
                    && rt
                        .session
                        .tracks
                        .get(rt.fx_view as usize)
                        .is_none_or(|item| !item.active)
            {
                rt.fx_view = -1;
            }
        }
        rt.scene_metadata_changed();
        let next_focus = Focus {
            track: rt.selected_track,
            scene: rt.selected_scene,
            compose: rt.compose_target,
            fx: rt.fx_view,
        };
        if self.focus.is_some() || next_focus != current_focus {
            self.focus = Some(current_focus);
        }
    }
}

impl Content {
    fn swap(&mut self, rt: &mut RtEngine, next: &Layout) {
        match self {
            Self::Import(import) => import.swap(rt),
            Self::Track { slot, node } => {
                if *slot == rt.tracks.len() {
                    rt.tracks.push(node.take().unwrap());
                } else if next.tracks.len() < rt.tracks.len() {
                    *node = rt.tracks.pop();
                } else {
                    std::mem::swap(&mut rt.tracks[*slot], node.as_mut().unwrap());
                }
            }
            Self::Scene { slot, cells, rack } => {
                if *slot == rt.scene_fx.len() {
                    for (track, cell) in rt.tracks.iter_mut().zip(cells) {
                        track.clips.push(cell.take().unwrap());
                    }
                    rt.scene_fx.push(rack.take().unwrap());
                } else if next.scenes.len() < rt.scene_fx.len() {
                    for (track, cell) in rt.tracks.iter_mut().zip(cells) {
                        *cell = track.clips.pop();
                    }
                    *rack = rt.scene_fx.pop();
                } else {
                    for (track, cell) in rt.tracks.iter_mut().zip(cells) {
                        std::mem::swap(&mut track.clips[*slot], cell.as_mut().unwrap());
                    }
                    std::mem::swap(&mut rt.scene_fx[*slot], rack.as_mut().unwrap());
                }
            }
        }
    }
}
impl Inverse {
    pub(crate) fn reserve(&mut self, rt: &RtEngine) {
        self.reserved_heap = match &self.content {
            Some(Content::Import(import)) => import.bytes() + if import.arrangement.is_some() {rt.arrangement.storage_bytes()} else {0},
            None => 0,
            Some(Content::Track { slot, node }) => {
                node.as_ref().map_or(0, |node| node.retained_bytes())
                    + rt.tracks.get(*slot).map_or(0, |node| node.retained_bytes())
            }
            Some(Content::Scene { slot, cells, rack }) => {
                cells.capacity() * std::mem::size_of::<Option<crate::engine::Clip>>()
                    + cells
                        .iter()
                        .flatten()
                        .map(crate::engine::Clip::retained_bytes)
                        .sum::<usize>()
                    + rack.as_ref().map_or(0, |rack| rack.retained_bytes())
                    + rt.tracks
                        .iter()
                        .filter_map(|t| t.clips.get(*slot))
                        .map(crate::engine::Clip::retained_bytes)
                        .sum::<usize>()
                    + rt.scene_fx
                        .get(*slot)
                        .map_or(0, |rack| rack.retained_bytes())
            }
        };
        if self.routing.is_some() { self.reserved_heap = self.reserved_heap.saturating_add(rt.routing.as_ref().map_or(0, |graph| graph.bytes())); }
    }
    pub(crate) fn media_reservations(&self, mut add: impl FnMut(usize, usize)) {
        for sample in &self.media {
            add(
                std::sync::Arc::as_ptr(sample) as usize,
                crate::engine::undo::sample_bytes(sample),
            );
        }
    }
    pub(crate) fn media_bytes(&self) -> usize {
        self.media
            .iter()
            .map(|s| crate::engine::undo::sample_bytes(s))
            .sum()
    }
}

impl Inverse {
    pub(crate) fn rate_bytes(&self, sr: f32) -> usize {
        let graph = self.routing.as_ref().and_then(Option::as_ref);
        let routing_bytes = match graph.map(|graph| graph.rate_bytes(&self.layout, sr as u32)).transpose() {
            Ok(bytes) => bytes.unwrap_or(0),
            Err(_) => return usize::MAX / 1024,
        };
        self.bytes().saturating_sub(self.reserved_fx_bytes).saturating_sub(graph.map_or(0, |graph| graph.bytes()))
            .saturating_sub(graph.map_or(0, |graph| self.reserved_heap.min(graph.bytes())))
            .saturating_add(routing_bytes)
            .saturating_add(routing_bytes)
            + self
                .fx_storage
                .iter()
                .map(|id| crate::engine::fx::FxSlot::required_storage(*id, sr))
                .sum::<usize>()
    }
    pub(crate) fn prepare_rate(&mut self, sr: f32) {
        if let Some(Some(graph)) = &mut self.routing {
            self.reserved_heap = self.reserved_heap.saturating_sub(graph.bytes());
            **graph = crate::engine::audio::routing::prepared::Prepared::at_rate(graph.model.clone(), &self.layout, sr as u32).expect("History rate admission validated routing storage");
            self.reserved_heap = self.reserved_heap.saturating_add(graph.bytes());
        }
        if let Some(content) = &mut self.content {
            match content {
                Content::Import(import) => import.prepare_rate(sr),
                Content::Track {
                    node: Some(node), ..
                } => {
                    node.fx.set_sample_rate(sr);
                    node.poly.set_sample_rate(sr);
                    node.eq.set_sample_rate(sr);
                    node.eq_right.set_sample_rate(sr);
                    node.mixer_gain = crate::engine::mixer_gain::GainPair::default();
                    node.input_gain = crate::engine::mixer_gain::GainPair::default();
                    node.stop_clip();
                    node.drum_pos.fill(None);
                }
                Content::Scene {
                    rack: Some(rack), ..
                } => rack.set_sample_rate(sr),
                _ => {}
            }
        }
        let next = self
            .fx_storage
            .iter()
            .map(|id| crate::engine::fx::FxSlot::required_storage(*id, sr))
            .sum::<usize>();
        self.reserved_heap = self.reserved_heap.saturating_sub(self.reserved_fx_bytes) + next;
        self.reserved_fx_bytes = next;
    }
}
