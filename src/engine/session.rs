//! Persistent display order is separate from storage. Reorder never moves a
//! clip, processor, recording destination or sounding input gate.
use serde::{Deserialize, Serialize};
mod registry;
mod scoped;
pub(crate) use scoped::Scoped;
mod transaction;
pub(crate) use registry::{Reference, Registry};
pub(crate) use transaction::{Structure, Action, Inverse, Request};
pub(crate) use transaction::ImportSelection;

pub const MAX_TRACKS: usize = 128;
pub const MAX_SCENES: usize = 512;
pub const MAX_NAME_BYTES: usize = 4096;
pub const MAX_PROCESSOR_BYTES: usize = 256 * 1024 * 1024;
pub const SCENE_FX_BASE: i16 = 1000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Id(pub u64);

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Item {
    pub id: Id,
    pub active: bool,
    pub name: String,
    pub color: Option<[u8; 3]>,
    #[serde(default, skip_serializing_if = "super::scene::Properties::is_default")]
    pub(crate) scene: super::scene::Properties,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Layout {
    /// Stable IDs are scoped to this project, including its saved copies.
    pub namespace: [u64; 2],
    pub next_id: u64,
    pub generation: u64,
    pub tracks: Vec<Item>,
    pub scenes: Vec<Item>,
    pub track_order: Vec<u8>,
    pub scene_order: Vec<u16>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Axis {
    Track,
    Scene,
}

impl Layout {
    pub fn legacy(track_names: impl IntoIterator<Item = String>, scene_count: usize) -> Self {
        let mut next_id = 1;
        let mut item = |name| {
            let result = Item {
                id: Id(next_id),
                active: true,
                name,
                color: None,
                scene: Default::default(),
            };
            next_id += 1;
            result
        };
        let tracks: Vec<_> = track_names.into_iter().map(&mut item).collect();
        let scenes: Vec<_> = (0..scene_count)
            .map(|i| item(format!("Scene {}", i + 1)))
            .collect();
        Self {
            namespace: [0, 1],
            next_id,
            generation: 0,
            track_order: (0..tracks.len()).map(|i| i as u8).collect(),
            scene_order: (0..scenes.len()).map(|i| i as u16).collect(),
            tracks,
            scenes,
        }
    }
    pub fn fresh(track_names: impl IntoIterator<Item = String>, scene_count: usize) -> Self {
        let mut result = Self::legacy(track_names, scene_count);
        result.namespace = super::midi_edit::NoteId::new().words();
        result
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.namespace == [0, 0]
            || self.tracks.is_empty()
            || self.scenes.is_empty()
            || self.tracks.len() > MAX_TRACKS
            || self.scenes.len() > MAX_SCENES
            || self.track_order.is_empty()
            || self.scene_order.is_empty()
        {
            return Err("Session requires 1–128 tracks and 1–512 scenes".into());
        }
        let mut ids = std::collections::HashSet::new();
        for item in self.tracks.iter().chain(&self.scenes) {
            if item.id.0 == 0
                || item.id.0 >= self.next_id
                || !ids.insert(item.id)
                || item.name.len() > MAX_NAME_BYTES
            {
                return Err("Invalid or repeated session identity or name".into());
            }
        }
        if self.tracks.iter().any(|item| !item.scene.is_default()) || self.scenes.iter().any(|item| !item.scene.valid()) { return Err("Scene properties must be valid and belong only to scenes".into()); }
        fn order(items: &[Item], slots: impl IntoIterator<Item = usize>) -> bool {
            let mut seen = vec![false; items.len()];
            for slot in slots {
                if slot >= items.len() || seen[slot] || !items[slot].active {
                    return false;
                }
                seen[slot] = true;
            }
            items
                .iter()
                .zip(seen)
                .all(|(item, seen)| item.active == seen)
        }
        if !order(
            &self.tracks,
            self.track_order.iter().map(|s| usize::from(*s)),
        ) || !order(
            &self.scenes,
            self.scene_order.iter().map(|s| usize::from(*s)),
        ) {
            return Err("Session order must contain each active identity exactly once".into());
        }
        Ok(())
    }
    pub fn resolve(&self, axis: Axis, id: Id) -> Option<usize> {
        self.items(axis)
            .iter()
            .position(|item| item.active && item.id == id)
    }
    pub fn item(&self, axis: Axis, id: Id) -> Option<&Item> {
        self.resolve(axis, id).map(|i| &self.items(axis)[i])
    }
    pub fn items(&self, axis: Axis) -> &[Item] {
        match axis {
            Axis::Track => &self.tracks,
            Axis::Scene => &self.scenes,
        }
    }
    /// Worker-only capacity preparation for the bounded audio snapshot handoff.
    pub(super) fn prepare_storage(&mut self, track_names: &[usize], scene_names: &[usize]) {
        let empty = || Item {
            id: Id(0),
            active: false,
            name: String::new(),
            color: None,
            scene: Default::default(),
        };
        self.tracks.resize_with(track_names.len(), empty);
        self.scenes.resize_with(scene_names.len(), empty);
        for (item, &needed) in self
            .tracks
            .iter_mut()
            .zip(track_names)
            .chain(self.scenes.iter_mut().zip(scene_names))
        {
            item.name.reserve(needed.saturating_sub(item.name.len()));
        }
        self.track_order
            .reserve(MAX_TRACKS.saturating_sub(self.track_order.len()));
        self.scene_order
            .reserve(MAX_SCENES.saturating_sub(self.scene_order.len()));
    }
    pub(super) fn fits(&self, source: &Self) -> bool {
        self.tracks.len() == source.tracks.len()
            && self.scenes.len() == source.scenes.len()
            && self.track_order.capacity() >= source.track_order.len()
            && self.scene_order.capacity() >= source.scene_order.len()
            && self
                .tracks
                .iter()
                .zip(&source.tracks)
                .chain(self.scenes.iter().zip(&source.scenes))
                .all(|(a, b)| a.name.capacity() >= b.name.len())
    }
    /// Audio only: called after fits, with every string and vector prepared.
    pub(super) fn copy_from_prepared(&mut self, source: &Self) {
        debug_assert!(self.fits(source));
        self.namespace = source.namespace;
        self.next_id = source.next_id;
        self.generation = source.generation;
        for (out, item) in self
            .tracks
            .iter_mut()
            .zip(&source.tracks)
            .chain(self.scenes.iter_mut().zip(&source.scenes))
        {
            out.id = item.id;
            out.active = item.active;
            out.color = item.color;
            out.scene = item.scene;
            out.name.clear();
            out.name.push_str(&item.name);
        }
        self.track_order.clear();
        self.track_order.extend_from_slice(&source.track_order);
        self.scene_order.clear();
        self.scene_order.extend_from_slice(&source.scene_order);
    }
    fn items_mut(&mut self, axis: Axis) -> &mut Vec<Item> {
        match axis {
            Axis::Track => &mut self.tracks,
            Axis::Scene => &mut self.scenes,
        }
    }
    fn changed(&mut self) -> Result<(), String> {
        self.generation = self
            .generation
            .checked_add(1)
            .ok_or("Session generation exhausted")?;
        Ok(())
    }
    fn can_change(&self) -> Result<(), String> {
        if self.generation == u64::MAX {
            Err("Session generation exhausted".into())
        } else {
            Ok(())
        }
    }
    pub fn create(
        &mut self,
        axis: Axis,
        name: String,
        color: Option<[u8; 3]>,
        position: usize,
    ) -> Result<(Id, usize), String> {
        self.can_change()?;
        if !valid_name(&name) {
            return Err("Use a name of 1–4096 bytes without NUL or line breaks".into());
        }
        let count = match axis {
            Axis::Track => self.track_order.len(),
            Axis::Scene => self.scene_order.len(),
        };
        let limit = match axis {
            Axis::Track => MAX_TRACKS,
            Axis::Scene => MAX_SCENES,
        };
        if count == limit {
            return Err(format!(
                "Session limit reached: {limit} {}",
                match axis {
                    Axis::Track => "tracks",
                    Axis::Scene => "scenes",
                }
            ));
        }
        if position > count {
            return Err("Session insertion position is outside its order".into());
        }
        let next = self
            .next_id
            .checked_add(1)
            .ok_or("Session identities exhausted")?;
        let id = Id(self.next_id);
        let slot = self
            .items(axis)
            .iter()
            .position(|item| !item.active)
            .unwrap_or(self.items(axis).len());
        let item = Item {
            id,
            active: true,
            name,
            color,
            scene: Default::default(),
        };
        let items = self.items_mut(axis);
        if slot == items.len() {
            items.push(item);
        } else {
            items[slot] = item;
        }
        match axis {
            Axis::Track => self.track_order.insert(position, slot as u8),
            Axis::Scene => self.scene_order.insert(position, slot as u16),
        }
        self.next_id = next;
        self.changed()?;
        Ok((id, slot))
    }
    pub fn rename(&mut self, axis: Axis, id: Id, name: String) -> Result<(), String> {
        self.can_change()?;
        if !valid_name(&name) {
            return Err("Use a name of 1–4096 bytes without NUL or line breaks".into());
        }
        let slot = self
            .resolve(axis, id)
            .ok_or("Session identity no longer exists")?;
        self.items_mut(axis)[slot].name = name;
        self.changed()
    }
    pub fn color(&mut self, axis: Axis, id: Id, color: Option<[u8; 3]>) -> Result<(), String> {
        self.can_change()?;
        let slot = self
            .resolve(axis, id)
            .ok_or("Session identity no longer exists")?;
        self.items_mut(axis)[slot].color = color;
        self.changed()
    }
    /// Set one scene launch policy.
    /// Takes its stable identity and validated properties; returns one generation change or a refusal without mutation.
    pub(crate) fn scene_properties(&mut self, id: Id, properties: super::scene::Properties) -> Result<(), String> {
        self.can_change()?;
        if !properties.valid() { return Err("Use a scene tempo of 40–240 BPM and a valid time signature".into()); }
        let slot = self.resolve(Axis::Scene, id).ok_or("Scene identity no longer exists")?;
        self.scenes[slot].scene = properties;
        self.changed()
    }
    pub fn reorder(&mut self, axis: Axis, id: Id, position: usize) -> Result<(), String> {
        self.can_change()?;
        let slot = self
            .resolve(axis, id)
            .ok_or("Session identity no longer exists")?;
        let count = match axis {
            Axis::Track => self.track_order.len(),
            Axis::Scene => self.scene_order.len(),
        };
        if position >= count {
            return Err("Session destination is outside its order".into());
        }
        match axis {
            Axis::Track => {
                let old = self
                    .track_order
                    .iter()
                    .position(|s| usize::from(*s) == slot)
                    .unwrap();
                let value = self.track_order.remove(old);
                self.track_order.insert(position, value);
            }
            Axis::Scene => {
                let old = self
                    .scene_order
                    .iter()
                    .position(|s| usize::from(*s) == slot)
                    .unwrap();
                let value = self.scene_order.remove(old);
                self.scene_order.insert(position, value);
            }
        }
        self.changed()
    }
    pub fn delete(&mut self, axis: Axis, id: Id) -> Result<usize, String> {
        self.can_change()?;
        let slot = self
            .resolve(axis, id)
            .ok_or("Session identity no longer exists")?;
        let count = match axis {
            Axis::Track => self.track_order.len(),
            Axis::Scene => self.scene_order.len(),
        };
        if count == 1 {
            return Err("Keep at least one track and one scene".into());
        }
        self.items_mut(axis)[slot].active = false;
        match axis {
            Axis::Track => self.track_order.retain(|s| usize::from(*s) != slot),
            Axis::Scene => self.scene_order.retain(|s| usize::from(*s) != slot),
        }
        self.changed()?;
        Ok(slot)
    }
}

fn valid_name(name: &str) -> bool {
    !name.trim().is_empty() && name.len() <= MAX_NAME_BYTES && !name.contains(['\0', '\r', '\n'])
}

#[cfg(test)]
mod tests {
    use super::*;
    fn layout() -> Layout {
        Layout::legacy(["Drums".into(), "Bass".into()], 2)
    }
    #[test]
    fn reorder_only_changes_order_and_reference_resolution() {
        let mut l = layout();
        let before = l.clone();
        let track = l.tracks[0].id;
        let scene = l.scenes[0].id;
        l.reorder(Axis::Track, track, 1).unwrap();
        l.reorder(Axis::Scene, scene, 1).unwrap();
        assert_eq!(l.tracks, before.tracks);
        assert_eq!(l.scenes, before.scenes);
        assert_eq!(l.resolve(Axis::Track, track), Some(0));
        assert_eq!(l.resolve(Axis::Scene, scene), Some(0));
        assert_eq!(l.track_order, [1, 0]);
        assert_eq!(l.scene_order, [1, 0]);
        l.validate().unwrap();
    }
    #[test]
    fn maximum_dimensions_round_trip_without_truncation_or_reused_ids() {
        let mut l = layout();
        while l.track_order.len() < MAX_TRACKS {
            l.create(
                Axis::Track,
                format!("Track {}", l.track_order.len() + 1),
                None,
                l.track_order.len(),
            )
            .unwrap();
        }
        while l.scene_order.len() < MAX_SCENES {
            l.create(
                Axis::Scene,
                format!("Scene {}", l.scene_order.len() + 1),
                None,
                l.scene_order.len(),
            )
            .unwrap();
        }
        l.validate().unwrap();
        let before = l.clone();
        assert!(l
            .create(Axis::Track, "Overflow".into(), None, 0)
            .unwrap_err()
            .contains("128 tracks"));
        assert!(l
            .create(Axis::Scene, "Overflow".into(), None, 0)
            .unwrap_err()
            .contains("512 scenes"));
        assert_eq!(l, before);
        let retired = l.tracks[17].id;
        l.delete(Axis::Track, retired).unwrap();
        let (replacement, slot) = l
            .create(Axis::Track, "Replacement".into(), Some([1, 2, 3]), 11)
            .unwrap();
        assert_eq!(slot, 17);
        assert_ne!(replacement, retired);
        assert_eq!(l.resolve(Axis::Track, retired), None);
        l.validate().unwrap();
        let bytes = serde_json::to_vec(&l).unwrap();
        let reopened: Layout = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(l, reopened);
        reopened.validate().unwrap();
    }
    #[test]
    fn malformed_orders_names_ids_and_counter_exhaustion_fail_without_mutation() {
        let mut l = layout();
        let id = l.tracks[0].id;
        let before = l.clone();
        for name in ["", " \t", "a\nb", "a\0b"] {
            assert!(l.rename(Axis::Track, id, name.into()).is_err());
            assert_eq!(l, before);
        }
        assert!(l.reorder(Axis::Scene, l.scenes[0].id, 2).is_err());
        assert_eq!(l, before);
        l.track_order[1] = 0;
        assert!(l.validate().is_err());
        l = before.clone();
        l.scenes[0].id = id;
        assert!(l.validate().is_err());
        l = before;
        l.generation = u64::MAX;
        let before = l.clone();
        assert!(l.delete(Axis::Track, id).is_err());
        assert_eq!(l, before);
    }
    #[test]
    fn renaming_coloring_deleting_and_reusing_scene_slots_keeps_old_references_invalid() {
        let mut l = layout();
        let id = l.scenes[1].id;
        l.rename(Axis::Scene, id, "Chorus".into()).unwrap();
        l.color(Axis::Scene, id, Some([23, 42, 81])).unwrap();
        assert_eq!(l.item(Axis::Scene, id).unwrap().name, "Chorus");
        let slot = l.delete(Axis::Scene, id).unwrap();
        let (new, replacement) = l.create(Axis::Scene, "Bridge".into(), None, 0).unwrap();
        assert_eq!(slot, replacement);
        assert_ne!(id, new);
        assert!(l.item(Axis::Scene, id).is_none());
        l.validate().unwrap();
    }
}

/// Producer admission failures settle the same receipt as renderer failures.
pub(crate) fn admission_ack(command: &super::Command) -> Option<super::midi_edit::Ack> {
    match command {
        super::Command::SessionEdit(request) => Some(request.ack.clone()),
        super::Command::Gesture { command, .. } => admission_ack(command),
        _ => None,
    }
}
