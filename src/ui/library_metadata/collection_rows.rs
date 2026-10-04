//! Immutable identity lookup built beside each exact row/catalog publication.
//! No filesystem access, alias substitution, or table construction on the GUI.
use super::{Arc, LibItem, Weak};
use crate::library::{Catalog, TrackId, crates::CrateId, smart_crates::Rule};
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
    smart: HashMap<CrateId, SmartMembership>,
    manual: HashMap<TrackId, Vec<usize>>,
    #[cfg(test)]
    dropped: std::sync::Mutex<Option<std::sync::mpsc::Sender<std::thread::ThreadId>>>,
}

struct SmartMembership {
    rule: Rule,
    mask: Vec<bool>,
    indices: Vec<usize>,
    #[cfg(test)]
    evaluated: usize,
}

impl CollectionRows {
    pub(super) fn build(rows: &Arc<Vec<LibItem>>, catalog: &Arc<Catalog>) -> Self {
        Self::build_incremental(rows, catalog, None)
    }

    /// Prepare exact row identities and automatic membership on the catalog worker.
    /// Takes current rows/catalog and an optional prior publication; evaluates only changed tracks for unchanged rules.
    pub(super) fn build_incremental(rows: &Arc<Vec<LibItem>>, catalog: &Arc<Catalog>, previous: Option<&Self>) -> Self {
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
        let old = previous.and_then(|index|Some((index, index.rows.upgrade()?, index.catalog.upgrade()?)));
        let mut smart = HashMap::new();
        for node in catalog.crates.nodes() {
            let Some(rule) = &node.smart_rule else { continue };
            let compiled = rule.compile().expect("validated catalog smart rule");
            let prior = old.as_ref().and_then(|(index, old_rows, old_catalog)|index.smart.get(&node.id)
                .filter(|membership|membership.rule == *rule).map(|membership|(index,old_rows,old_catalog,membership)));
            let mut membership = SmartMembership { rule: rule.clone(), mask: vec![false;rows.len()], indices: vec![], #[cfg(test)] evaluated: 0 };
            for (row, item) in rows.iter().enumerate() {
                let Some(track) = catalog.track(&item.source) else { continue };
                if track.versions[track.current].fingerprint != item.fingerprint { continue; }
                let saved = prior.and_then(|(index, old_rows, old_catalog, membership)| {
                    let entry = index.entries.get(&track.id)?;
                    let old_row = entry.row?;
                    let old_item = &old_rows[old_row];
                    let old_track = &old_catalog.tracks[entry.track];
                    (item.title == old_item.title && item.artist == old_item.artist && item.key == old_item.key
                        && item.bpm == old_item.bpm && item.length == old_item.length && item.last_play.is_some() == old_item.last_play.is_some()
                        && track.annotations == old_track.annotations).then_some(membership.mask[old_row])
                });
                let matches = saved.unwrap_or_else(|| {
                    #[cfg(test)] { membership.evaluated += 1; }
                    compiled.matches(crate::library::search::Row { title: &item.title, artist: &item.artist, key: &item.key,
                        bpm: item.bpm.value(), seconds: item.length, played: item.last_play.is_some(), annotations: &track.annotations })
                });
                membership.mask[row] = matches;
                if matches { membership.indices.push(row); }
            }
            smart.insert(node.id.clone(), membership);
        }
        let mut manual: HashMap<TrackId, Vec<usize>> = HashMap::new();
        for (index, node) in catalog.crates.nodes().iter().enumerate() {
            for member in &node.members { manual.entry(member.clone()).or_default().push(index); }
        }
        Self {
            rows: Arc::downgrade(rows), catalog: Arc::downgrade(catalog), entries, smart, manual,
            #[cfg(test)]
            dropped: std::sync::Mutex::new(None),
        }
    }

    /// Reveal direct manual and automatic memberships for one saved track.
    /// Takes its stable identity and exact publication; returns node indices, or None while that publication is unavailable.
    pub fn crates_containing(&self, id: &TrackId, rows: &Arc<Vec<LibItem>>, catalog: &Arc<Catalog>) -> Option<Vec<usize>> {
        if !self.is_for(rows, catalog) { return None; }
        let entry = self.entries.get(id)?;
        let annotations = &catalog.tracks[entry.track].annotations;
        let mut result = self.manual.get(id).cloned().unwrap_or_default();
        for (index, node) in catalog.crates.nodes().iter().enumerate() {
            let automatic = node.annotation_rule.as_ref().is_some_and(|rule|rule.matches(annotations))
                || entry.row.is_some_and(|row|self.smart.get(&node.id).is_some_and(|membership|membership.mask[row]));
            if automatic { result.push(index); }
        }
        Some(result)
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

    /// Read prepared automatic membership for the exact visible publication.
    /// Takes crate identity and immutable rows/catalog; mismatched publications expose no stale result.
    pub fn smart_rows(&self, id: &CrateId, rows: &Arc<Vec<LibItem>>, catalog: &Arc<Catalog>) -> Option<&[usize]> {
        self.is_for(rows, catalog).then(||self.smart.get(id).map(|membership|membership.indices.as_slice())).flatten()
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
