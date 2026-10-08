//! Immutable identity lookup built beside each exact row/catalog publication.
//! No filesystem access, alias substitution, or table construction on the GUI.
use super::{Arc, LibItem, Weak};
use crate::library::{Catalog, TrackId, crates::CrateId, smart_crates::Rule};
use std::collections::HashMap;
mod search_index;

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
    search:Option<search_index::Index>,
    search_error:Option<String>,
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
        let prepared_search=search_index::Index::build(rows,catalog,old.as_ref().and_then(|(index,old_rows,old_catalog)|Some((index.search.as_ref()?,old_rows,old_catalog))));
        let (search,search_error)=match prepared_search {Ok(index)=>(Some(index),None),Err(error)=>(None,Some(error))};
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
                    let key_record = |version: &crate::library::Version| version.analysis.as_ref().and_then(|record|record.key.as_ref()).cloned();
                    (item.title == old_item.title && item.artist == old_item.artist && item.key == old_item.key
                        && item.bpm == old_item.bpm && item.length == old_item.length && item.last_play.is_some() == old_item.last_play.is_some()
                        && track.annotations == old_track.annotations && track.locks.metadata == old_track.locks.metadata
                        && track.versions[track.current].tags == old_track.versions[old_track.current].tags
                        && key_record(&track.versions[track.current]) == key_record(&old_track.versions[old_track.current])).then_some(membership.mask[old_row])
                });
                let matches = saved.unwrap_or_else(|| {
                    #[cfg(test)] { membership.evaluated += 1; }
                    let key = crate::musical_key::effective(Some(&track.versions[track.current]),&item.key,track.locks.metadata).0;
                    compiled.matches(crate::library::search::Row { title: &item.title, artist: &item.artist, key: &key,
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
            rows: Arc::downgrade(rows), catalog: Arc::downgrade(catalog), entries, smart, manual, search, search_error,
            #[cfg(test)]
            dropped: std::sync::Mutex::new(None),
        }
    }

    /// Use normalized metadata only for its exact row/catalog publication.
    /// Takes a compiled query, exact row and live play state; returns None while a matching worker publication is unavailable.
    pub fn matches_query(&self,query:&crate::library::search::Query,index:usize,played:bool,rows:&Arc<Vec<LibItem>>,catalog:&Arc<Catalog>)->Option<bool> {
        self.is_for(rows,catalog).then(||self.search.as_ref().map(|search|search.matches(query,index,played))).flatten()
    }
    /// Preserve membership after a bounded metadata-only worker update.
    /// Takes old view and current exact publications; returns changed row positions for an unchanged identity/order pair only.
    pub fn search_delta(&self,old_rows:&Weak<Vec<LibItem>>,old_catalog:&Weak<Catalog>,rows:&Arc<Vec<LibItem>>,catalog:&Arc<Catalog>)->Option<&[usize]> {
        self.is_for(rows,catalog).then(||self.search.as_ref()?.delta_for(old_rows,old_catalog)).flatten()
    }
    /// Sort exact current rows using full worker-prepared metadata.
    /// Takes mutable membership and current publications/history/columns; returns false when that exact index is unavailable.
    pub fn sort_query(&self,indices:&mut [usize],rows:&Arc<Vec<LibItem>>,catalog:&Arc<Catalog>,history:&crate::ui::play_history::History,sorts:[Option<crate::preferences::library_layout::Sort>;2])->bool {
        let Some(index)=self.search.as_ref().filter(|_|self.is_for(rows,catalog)) else {return false;};
        if sorts[0].is_some() {indices.sort_by(|a,b|index.compare(*a,*b,rows,history,sorts));}true
    }
    /// Insert changed whole-library rows in the same stable metadata order.
    /// Takes two exact row positions and current publications/history/columns; returns their value order with original row order for equal values.
    pub fn compare_rows(&self,a:usize,b:usize,rows:&Arc<Vec<LibItem>>,catalog:&Arc<Catalog>,history:&crate::ui::play_history::History,sorts:[Option<crate::preferences::library_layout::Sort>;2])->Option<std::cmp::Ordering> {self.is_for(rows,catalog).then(||self.search.as_ref().map(|index|index.compare(a,b,rows,history,sorts).then_with(||a.cmp(&b)))).flatten()}

    /// Report an explicit retained search index limit for the current publication.
    /// Takes exact current rows/catalog; exposes an error only for the matching worker result.
    pub fn search_error(&self,rows:&Arc<Vec<LibItem>>,catalog:&Arc<Catalog>)->Option<&str> {self.is_for(rows,catalog).then(||self.search_error.as_deref()).flatten()}
    /// Account for the extra prepared metadata owned by this publication.
    /// Takes no arguments; returns normalized search record and owner bytes, excluding existing identity and crate tables.
    pub fn search_bytes(&self)->usize {self.search.as_ref().map_or(0,|index|index.bytes())}

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
