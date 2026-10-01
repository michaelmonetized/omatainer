//! Immutable identity lookup built beside each exact row/catalog publication.
//! No filesystem access, alias substitution, or table construction on the GUI.
use super::{Arc, LibItem, Weak};
use crate::library::{Catalog, TrackId};
use std::collections::HashMap;

#[derive(Clone, Copy)]
struct Entry {
    track: usize,
    row: Option<usize>,
}

#[derive(Default)]
pub(in crate::ui) struct CollectionRows {
    rows: Weak<Vec<LibItem>>,
    catalog: Weak<Catalog>,
    entries: HashMap<TrackId, Entry>,
    #[cfg(test)]
    dropped: std::sync::Mutex<Option<std::sync::mpsc::Sender<std::thread::ThreadId>>>,
}

impl CollectionRows {
    pub(super) fn build(rows: &Arc<Vec<LibItem>>, catalog: &Arc<Catalog>) -> Self {
        // Catalog validation caps this at 100,000 tracks. Every known identity
        // gets a label index, including members absent from this visible view.
        let mut entries: HashMap<_, _> = catalog.tracks.iter().enumerate()
            .map(|(track, item)| (item.id.clone(), Entry { track, row: None }))
            .collect();
        for (row, item) in rows.iter().enumerate() {
            let Some(track) = catalog.track(&item.source) else { continue };
            if track.versions[track.current].fingerprint != item.fingerprint { continue; }
            if let Some(entry) = entries.get_mut(&track.id) {
                entry.row.get_or_insert(row);
            }
        }
        Self {
            rows: Arc::downgrade(rows), catalog: Arc::downgrade(catalog), entries,
            #[cfg(test)]
            dropped: std::sync::Mutex::new(None),
        }
    }

    pub fn is_for(&self, rows: &Arc<Vec<LibItem>>, catalog: &Arc<Catalog>) -> bool {
        self.rows.as_ptr() == Arc::as_ptr(rows) && self.catalog.as_ptr() == Arc::as_ptr(catalog)
    }

    /// An absent row never means a lost membership. Display a saved placeholder
    /// via track_index; a false is_for means the new view is still preparing.
    pub fn row(&self, id: &TrackId, rows: &Arc<Vec<LibItem>>, catalog: &Arc<Catalog>) -> Option<usize> {
        self.is_for(rows, catalog).then(|| self.entries.get(id)?.row).flatten()
    }

    pub fn track_index(&self, id: &TrackId, catalog: &Arc<Catalog>) -> Option<usize> {
        (self.catalog.as_ptr() == Arc::as_ptr(catalog)).then(|| self.entries.get(id).map(|entry| entry.track)).flatten()
    }
}

#[cfg(test)]
impl Drop for CollectionRows {
    fn drop(&mut self) {
        if let Some(dropped) = self.dropped.get_mut().unwrap().take() { let _ = dropped.send(std::thread::current().id()); }
    }
}

#[cfg(test)]
mod tests;
