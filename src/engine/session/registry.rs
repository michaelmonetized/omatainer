//! Nonblocking identity receipts for command and MIDI producers. Audio is the
//! sole writer; consumers make one bounded read attempt during publication.
use super::{Axis, Id, Layout, MAX_SCENES, MAX_TRACKS};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering::*};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Reference {
    pub namespace: [u64; 2],
    pub id: Id,
}

pub(crate) struct Registry {
    sequence: AtomicU64,
    known: AtomicBool,
    namespace: [AtomicU64; 2],
    tracks: [AtomicU64; MAX_TRACKS],
    scenes: [AtomicU64; MAX_SCENES],
    track_count: AtomicUsize,
    scene_count: AtomicUsize,
}
impl Default for Registry {
    fn default() -> Self {
        Self {
            sequence: AtomicU64::new(0),
            known: AtomicBool::new(false),
            namespace: std::array::from_fn(|_| AtomicU64::new(0)),
            tracks: std::array::from_fn(|_| AtomicU64::new(0)),
            scenes: std::array::from_fn(|_| AtomicU64::new(0)),
            track_count: AtomicUsize::new(super::super::TRACKS),
            scene_count: AtomicUsize::new(super::super::SCENES),
        }
    }
}
impl Registry {
    pub fn publish(&self, layout: &Layout) {
        // Names, colors and display order do not change a control destination.
        let changed = !self.known.load(Relaxed)
            || self.namespace[0].load(Relaxed) != layout.namespace[0]
            || self.namespace[1].load(Relaxed) != layout.namespace[1]
            || self.track_count.load(Relaxed) != layout.tracks.len()
            || self.scene_count.load(Relaxed) != layout.scenes.len()
            || self.tracks.iter().enumerate().any(|(slot, value)| {
                value.load(Relaxed)
                    != layout
                        .tracks
                        .get(slot)
                        .filter(|item| item.active)
                        .map_or(0, |item| item.id.0)
            })
            || self.scenes.iter().enumerate().any(|(slot, value)| {
                value.load(Relaxed)
                    != layout
                        .scenes
                        .get(slot)
                        .filter(|item| item.active)
                        .map_or(0, |item| item.id.0)
            });
        if !changed {
            return;
        }
        self.sequence.fetch_add(1, AcqRel);
        self.namespace[0].store(layout.namespace[0], Relaxed);
        self.namespace[1].store(layout.namespace[1], Relaxed);
        for (slot, value) in self.tracks.iter().enumerate() {
            value.store(
                layout
                    .tracks
                    .get(slot)
                    .filter(|item| item.active)
                    .map_or(0, |item| item.id.0),
                Relaxed,
            );
        }
        for (slot, value) in self.scenes.iter().enumerate() {
            value.store(
                layout
                    .scenes
                    .get(slot)
                    .filter(|item| item.active)
                    .map_or(0, |item| item.id.0),
                Relaxed,
            );
        }
        self.track_count.store(layout.tracks.len(), Relaxed);
        self.scene_count.store(layout.scenes.len(), Relaxed);
        self.known.store(true, Relaxed);
        self.sequence.fetch_add(1, Release);
    }
    pub fn known(&self) -> bool {
        self.known.load(Acquire)
    }
    pub fn reference(&self, axis: Axis, slot: usize) -> Option<Reference> {
        if !self.known() {
            return None;
        }
        let before = self.sequence.load(Acquire);
        if before & 1 != 0 {
            return None;
        }
        let id = match axis {
            Axis::Track => self.tracks.get(slot)?,
            Axis::Scene => self.scenes.get(slot)?,
        }
        .load(Relaxed);
        let namespace = [
            self.namespace[0].load(Relaxed),
            self.namespace[1].load(Relaxed),
        ];
        if id == 0 || before != self.sequence.load(Acquire) {
            return None;
        }
        Some(Reference {
            namespace,
            id: Id(id),
        })
    }
    pub fn scene_count(&self) -> usize {
        self.scene_count.load(Acquire)
    }
    pub fn scene_exists(&self, slot: usize) -> bool {
        if !self.known() {
            slot < super::super::SCENES
        } else {
            self.reference(Axis::Scene, slot).is_some()
        }
    }
}

impl Layout {
    pub(crate) fn reference(&self, axis: Axis, slot: usize) -> Option<Reference> {
        self.items(axis)
            .get(slot)
            .filter(|item| item.active)
            .map(|item| Reference {
                namespace: self.namespace,
                id: item.id,
            })
    }
    pub(crate) fn resolves(&self, axis: Axis, slot: usize, reference: Reference) -> bool {
        self.reference(axis, slot) == Some(reference)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn order_and_names_keep_receipts_valid_but_deletion_reuse_and_other_projects_do_not() {
        let registry = Registry::default();
        let mut layout = Layout::legacy(["One".into(), "Two".into()], 2);
        registry.publish(&layout);
        let original = registry.reference(Axis::Track, 0).unwrap();
        let sequence = registry.sequence.load(Relaxed);
        layout.reorder(Axis::Track, original.id, 1).unwrap();
        layout
            .rename(Axis::Track, original.id, "Moved".into())
            .unwrap();
        registry.publish(&layout);
        assert_eq!(registry.reference(Axis::Track, 0), Some(original));
        assert_eq!(registry.sequence.load(Relaxed), sequence);
        layout.delete(Axis::Track, original.id).unwrap();
        registry.publish(&layout);
        assert_eq!(registry.reference(Axis::Track, 0), None);
        layout.create(Axis::Track, "New".into(), None, 0).unwrap();
        registry.publish(&layout);
        assert_ne!(registry.reference(Axis::Track, 0), Some(original));
        layout.namespace = [1, 2];
        registry.publish(&layout);
        assert!(!layout.resolves(Axis::Track, 0, original));
    }
    #[test]
    fn all_slots_are_available_and_publication_never_returns_a_partial_receipt() {
        let registry = Registry::default();
        let layout = Layout::legacy((0..MAX_TRACKS).map(|i| format!("Track {i}")), MAX_SCENES);
        registry.publish(&layout);
        assert!(registry.reference(Axis::Track, 127).is_some());
        assert!(registry.scene_exists(511));
        assert!(!registry.scene_exists(512));
        registry.sequence.fetch_add(1, AcqRel);
        assert_eq!(registry.reference(Axis::Track, 127), None);
        assert!(!registry.scene_exists(511));
    }
}
