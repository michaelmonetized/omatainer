//! Prepared layout inverses; audio swaps owned metadata without allocating.
use super::{Axis, Id, Layout};
use crate::engine::{
    midi_edit::{Ack, Outcome},
    Command, RtEngine,
};
#[cfg(test)]
mod tests;

#[derive(Clone, Debug)]
pub(crate) enum Action {
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
}

#[derive(Clone, Debug)]
pub(crate) struct Inverse {
    layout: Layout,
    track_name: Option<(usize, String)>,
    focus: Option<Focus>,
    bus_mask: u128,
    scene_buses: [usize; super::MAX_TRACKS],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Focus {
    track: usize,
    scene: usize,
    compose: Option<crate::engine::ComposeTarget>,
    fx: i16,
}

impl Request {
    /// Producer only. No mutation has happened when validation returns an error.
    pub fn metadata(layout: &Layout, epoch: u64, action: Action) -> Result<(Self, Ack), String> {
        layout.validate()?;
        let disruptive = matches!(action, Action::Delete { .. });
        let mut next = layout.clone();
        let mut track_name = None;
        match action {
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
                    track_name,
                    focus: None,
                    bus_mask: 0,
                    scene_buses: [0; super::MAX_TRACKS],
                })),
                ack: ack.clone(),
                disruptive,
            },
            ack,
        ))
    }
    pub(crate) fn disruptive(&self) -> bool {
        self.disruptive
    }
    pub(crate) fn bytes(&self) -> usize {
        std::mem::size_of::<Self>() + self.inverse.as_ref().map_or(0, |inverse| inverse.bytes())
    }
    pub(crate) fn current(&self, rt: &RtEngine) -> bool {
        self.ack.state() == Outcome::Pending
            && self.epoch == rt.undo.checkpoint().epoch
            && self.namespace == rt.session.namespace
            && self.generation == rt.session.generation
            && rt.session.generation < u64::MAX
            && self.inverse.is_some()
    }
}

impl Inverse {
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
            + self
                .track_name
                .as_ref()
                .map_or(0, |(_, name)| name.capacity())
    }
    pub(crate) fn valid(&self, rt: &RtEngine) -> bool {
        self.layout.namespace == rt.session.namespace
            && rt.session.generation < u64::MAX
            && self.layout.tracks.len() <= rt.tracks.len()
            && self.layout.scenes.len() <= rt.scene_fx.len()
    }
    pub(crate) fn swap(&mut self, rt: &mut RtEngine) {
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
            if rt.session.tracks[slot].active && !self.layout.tracks[slot].active {
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
            if rt.session.scenes[slot].active && !self.layout.scenes[slot].active {
                for track in 0..rt.tracks.len() {
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
        std::mem::swap(&mut rt.session, &mut self.layout);
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
            if !rt.session.tracks[rt.selected_track].active {
                rt.selected_track = usize::from(rt.session.track_order[0]);
            }
            if !rt.session.scenes[rt.selected_scene].active {
                rt.selected_scene = usize::from(rt.session.scene_order[0]);
            }
            if rt.compose_target.is_some_and(|target| {
                !rt.session.tracks[target.track].active || !rt.session.scenes[target.scene].active
            }) {
                rt.compose_target = None;
            }
            if rt.fx_view >= super::SCENE_FX_BASE
                && !rt.session.scenes[(rt.fx_view - super::SCENE_FX_BASE) as usize].active
                || rt.fx_view >= 0
                    && rt.fx_view < super::SCENE_FX_BASE
                    && !rt.session.tracks[rt.fx_view as usize].active
            {
                rt.fx_view = -1;
            }
        }
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
