//! Filtered indices are rebuilt only when the immutable crate or query changes.
//! Formatted cells live only for the viewport (including egui's bounded overscan).
use super::*;
use std::sync::Weak;

#[derive(Default)]
pub(super) struct LibraryView {
    library: Weak<Vec<LibItem>>,
    catalog: Weak<crate::library::Catalog>,
    query: String,
    pub search_all: bool,
    searched_all: bool,
    played_filter: bool,
    played_revision: u64,
    sorts: [Option<crate::preferences::library_layout::Sort>; 2],
    pub horizontal_offset: f32,
    crate_return: Option<(Option<crate::library::crates::CrateId>, String, Option<LibSource>, f32)>,
    crate_id: Option<crate::library::crates::CrateId>,
    collection_revision: u64,
    pub generation: u64,
    pub unavailable: usize,
    pub annotation_error: String,
    pub indices: Arc<Vec<usize>>,
    selected: Option<LibSource>,
    selected_index: usize,
    last_played: Option<LibSource>,
    last_played_index: usize,
    pub cells: HashMap<usize, Cells>,
    pub offset: f32,
    pub height: f32,
    pub stride: f32,
    top: Option<LibSource>,
    top_fraction: f32,
    pub pending_offset: Option<f32>,
    #[cfg(test)]
    pub stats: ViewStats,
}

pub(super) struct Bookmark {
    crate_id: Option<crate::library::crates::CrateId>,
    query: String,
    source: Option<LibSource>,
    top: Option<LibSource>,
    top_fraction: f32,
    offset: f32,
    search_all: bool,
    crate_return: Option<(Option<crate::library::crates::CrateId>, String, Option<LibSource>, f32)>,
}
impl Bookmark {
    /// Follow an explicitly verified moved source in retained navigation.
    /// Takes old and new identities; updates only matching selection and viewport anchors.
    pub(super) fn relocate(&mut self, from: &LibSource, to: &LibSource) {
        for source in [&mut self.source, &mut self.top] { if source.as_ref() == Some(from) { *source = Some(to.clone()); } }
        if let Some((_,_,source,_)) = &mut self.crate_return { if source.as_ref() == Some(from) { *source = Some(to.clone()); } }
    }
}

impl LibraryView {
    /// Follow deliberate crate selection.
    /// Takes mutable view state; discards whole-library mode and its old return context.
    pub fn reset_search_scope(&mut self) { self.search_all = false; self.crate_return = None; }
}

pub(super) struct Cells {
    pub title: String,
    pub bpm: String,
    pub key: String,
    pub key_detail: String,
    pub length: String,
    pub played: String,
    pub played_at: Option<SystemTime>,
    pub played_tooltip: String,
    pub played_refresh_at: Option<SystemTime>,
    pub annotations: [String; 5],
    played_clock: SystemTime,
    locale: crate::localization::Locale,
}

impl Cells {
    pub fn new(item: &LibItem, played_at: Option<SystemTime>, now: SystemTime) -> Self {
        let played = play_time::format(played_at, now);
        let (key, key_detail) = crate::musical_key::display(None, &item.key, false);
        Self {
            title: item.title.clone(),
            bpm: item.bpm.cell(),
            key,
            key_detail,
            length: fmt_len(item.length),
            played: played.label,
            played_tooltip: played.tooltip,
            played_refresh_at: played.next_change,
            played_clock: now,
            locale: crate::localization::current(),
            annotations: std::array::from_fn(|_|String::new()),
            played_at,
        }
    }

    pub fn refresh_play(&mut self, played_at: Option<SystemTime>, now: SystemTime) -> bool {
        let expired = played_at.is_some() && (now < self.played_clock
            || self.played_refresh_at.is_some_and(|deadline| now >= deadline));
        if self.played_at == played_at && !expired && self.locale == crate::localization::current() { return false; }
        let played = play_time::format(played_at, now);
        self.played_at = played_at;
        self.played = played.label;
        self.played_tooltip = played.tooltip;
        self.played_refresh_at = played.next_change;
        self.played_clock = now;
        self.locale = crate::localization::current();
        true
    }
}

#[cfg(test)]
#[derive(Default, Clone, Copy, Debug)]
pub(super) struct ViewStats {
    pub rebuilds: usize,
    pub updates:usize,
    pub examined: usize,
    pub rendered: usize,
    pub formatted: usize,
}

impl App {
    /// Follow an explicitly verified relocation, preserving a user's current
    /// selection rather than restoring whichever row submitted the request.
    pub(super) fn follow_library_relocation(&mut self, from: &LibSource, to: &LibSource) {
        if let Some(bookmark) = &mut self.library_crates.discovery.return_to { bookmark.relocate(from, to); }
        let view = &mut self.library_view;
        if view.selected.as_ref() == Some(from) { view.selected = Some(to.clone()); }
        if view.top.as_ref() == Some(from) { view.top = Some(to.clone()); }
    }

    /// Capture a library return point before discovery changes crates.
    /// Takes the current view; returns its query, selection, scroll anchor and whole-library context.
    pub(super) fn library_bookmark(&mut self) -> Bookmark {
        self.refresh_library_view();
        Bookmark { crate_id: self.library_crates.selected.clone(), query: self.lib_filter.clone(),
            source: self.library_view.indices.get(self.lib_sel).map(|index|self.library[*index].source.clone()),
            top: self.library_view.top.clone(), top_fraction: self.library_view.top_fraction,
            offset: self.library_view.offset, search_all: self.library_view.search_all,
            crate_return: self.library_view.crate_return.clone() }
    }

    /// Return from discovered crates without replacing track navigation.
    /// Takes a captured bookmark; restores the surviving crate, query, selected source and viewport.
    pub(super) fn restore_library_bookmark(&mut self, bookmark: Bookmark) {
        self.choose_named_crate(bookmark.crate_id);
        self.lib_filter = bookmark.query;
        self.library_view.search_all = bookmark.search_all;
        self.library_view.crate_return = bookmark.crate_return;
        self.refresh_library_view();
        let find = |source: &LibSource|self.library_view.indices.iter().position(|index|&self.library[*index].source == source);
        if let Some(source) = bookmark.source {
            if let Some(index) = find(&source) { self.lib_sel = index; }
            else { self.library_crates.message = "The saved selected track is no longer in this view; its return context was retained.".into(); }
        }
        self.library_view.pending_offset = Some(bookmark.top.as_ref().and_then(find)
            .map(|index|index as f32 * self.library_view.stride.max(18.0) + bookmark.top_fraction).unwrap_or(bookmark.offset));
        self.publish_library_selection();
    }

    /// Change search scope while retaining the named crate's navigation.
    /// Takes the requested whole-library mode; restores the prior crate query, row and scroll when returning.
    pub(super) fn set_library_search_all(&mut self, enabled: bool) {
        if self.library_view.search_all == enabled { return; }
        self.refresh_library_view();
        if enabled {
            let source = self.library_view.indices.get(self.lib_sel).map(|index|self.library[*index].source.clone());
            self.library_view.crate_return = Some((self.library_crates.selected.clone(), self.lib_filter.clone(), source, self.library_view.offset));
            self.library_view.search_all = true;
            self.refresh_library_view();
        } else {
            self.library_view.search_all = false;
            let previous = self.library_view.crate_return.take().filter(|(crate_id, ..)|*crate_id == self.library_crates.selected);
            if let Some((_,query,_,_)) = &previous { self.lib_filter.clone_from(query); }
            self.refresh_library_view();
            if let Some((_,_,source,offset)) = previous {
                if let Some(index) = source.and_then(|source|self.library_view.indices.iter().position(|index|self.library[*index].source==source)) { self.lib_sel=index; }
                self.library_view.pending_offset=Some(offset);
            }
        }
    }

    pub(super) fn refresh_library_view(&mut self) {
        self.refresh_named_crates();
        let sorts = self.library_layout.live.current().sorts();
        let view = &mut self.library_view;
        let order_changed = sorts != view.sorts;
        let selected_scope = if view.search_all { None } else { self.library_crates.selected.clone() };
        let selected_crate = &selected_scope;
        let scope_changed = view.search_all != view.searched_all;
        let collection_revision = self.library_metadata.catalog.crates.revision();
        let selected_crate_changed = view.crate_id != *selected_crate;
        let crate_changed = selected_crate_changed
            || selected_crate.is_some() && view.catalog.as_ptr() != Arc::as_ptr(&self.library_metadata.catalog)
            || selected_crate.is_some() && view.collection_revision != collection_revision;
        // The weak reference prevents allocation-address reuse and makes
        // Arc::make_mut detach as well. It never owns the old crate's elements,
        // so the scan worker remains responsible for retiring the large Vec.
        let library_changed = view.library.as_ptr() != Arc::as_ptr(&self.library);
        let query_changed = view.query != self.lib_filter;
        let sort_played = sorts.iter().flatten().any(|sort|sort.column == crate::preferences::library_layout::Column::Played);
        let played_revision = if sort_played { self.last_played.revision() } else { self.last_played.membership_revision() };
        let played_changed = (view.played_filter || sort_played) && view.played_revision != played_revision;
        let annotations_changed = view.catalog.as_ptr() != Arc::as_ptr(&self.library_metadata.catalog);
        if !library_changed {
            let source = view
                .indices
                .get(self.lib_sel)
                .map(|&i| &self.library[i].source);
            if source != view.selected.as_ref() {
                view.selected = source.cloned();
            }
            view.selected_index = self.lib_sel;
            let source = view
                .indices
                .get(self.last_play_idx)
                .map(|&i| &self.library[i].source);
            if source != view.last_played.as_ref() {
                view.last_played = source.cloned();
            }
            view.last_played_index = self.last_play_idx;
        }
        if library_changed || query_changed || crate_changed || annotations_changed || scope_changed || played_changed || order_changed {
            let parsed = crate::library::search::Query::parse(&self.lib_filter);
            view.played_filter = parsed.as_ref().is_ok_and(|query|query.uses_play_history());
            view.played_revision = played_revision;
            view.annotation_error = parsed.as_ref().err().cloned().unwrap_or_default();
            let row_index=self.library_metadata.collection_rows();
            let incremental=(!query_changed && !scope_changed && !crate_changed && !played_changed && !order_changed && selected_crate.is_none())
                .then(||row_index.search_delta(&view.library,&view.catalog,&self.library,&self.library_metadata.catalog)).flatten();
            let index_error=row_index.search_error(&self.library,&self.library_metadata.catalog).filter(|_|!self.lib_filter.trim().is_empty());
            if view.annotation_error.is_empty() {if let Some(error)=index_error {view.annotation_error=error.into();}}
            let empty_annotations = crate::library::annotations::Annotations::default();
            let matches = |index:usize| {
                let item=&self.library[index];
                let played=self.last_played.get(item).or(item.last_play).is_some();
                parsed.as_ref().is_ok_and(|query|index_error.is_none() && row_index.matches_query(query,index,played,&self.library,&self.library_metadata.catalog).unwrap_or_else(||{
                    let track=self.library_metadata.catalog.track(&item.source);
                    let fields=track.map_or(&empty_annotations,|track|&track.annotations);
                    let version=track.and_then(|track|track.versions.iter().find(|version|version.fingerprint==item.fingerprint));
                    let key=crate::musical_key::effective(version,&item.key,track.is_some_and(|track|track.locks.metadata)).0;
                    query.matches(crate::library::search::Row {title:&item.title,artist:&item.artist,key:&key,bpm:item.bpm.value(),seconds:item.length,played,annotations:fields})
                }))
            };
            let indices=Arc::make_mut(&mut view.indices);
            view.unavailable=0;
            if let Some(changed)=incremental {
                for &index in changed {if let Some(position)=indices.iter().position(|row|*row==index) {indices.remove(position);}}
                for &index in changed {if matches(index) {let position=if sorts[0].is_none() {indices.binary_search(&index).unwrap_err()} else {indices.binary_search_by(|other|row_index.compare_rows(*other,index,&self.library,&self.library_metadata.catalog,&self.last_played,sorts).expect("exact index admitted the metadata delta")).unwrap_or_else(|position|position)};indices.insert(position,index);}}
            } else {
                indices.clear();
                if let Some(node)=selected_crate.as_ref().and_then(|id|self.library_metadata.catalog.crates.node(id)) {
                    if node.smart_rule.is_some() {
                        if let Some(members)=row_index.smart_rows(&node.id,&self.library,&self.library_metadata.catalog) {indices.extend(members.iter().copied().filter(|index|matches(*index)));}
                        else {view.annotation_error="Smart crate membership is preparing for the current library".into();}
                    } else if let Some(rule)=&node.annotation_rule {
                        indices.extend(self.library.iter().enumerate().filter_map(|(index,item)|
                            (matches(index)&&self.library_metadata.catalog.track(&item.source).is_some_and(|track|rule.matches(&track.annotations))).then_some(index)));
                    } else {
                        for member in &node.members {
                            if let Some(index)=row_index.row(member,&self.library,&self.library_metadata.catalog) {if matches(index){indices.push(index);}}
                            else {view.unavailable+=1;}
                        }
                    }
                } else {indices.extend((0..self.library.len()).filter(|index|matches(*index)));}
                if !row_index.sort_query(indices,&self.library,&self.library_metadata.catalog,&self.last_played,sorts) {library_layout::sort::order(indices,&self.library,&self.library_metadata.catalog,&self.last_played,sorts);}
            }
            view.sorts = sorts;
            view.generation = view.generation.checked_add(1).expect("library view generation exhausted");
            let find = |source: &LibSource| {
                view.indices
                    .iter()
                    .position(|&i| &self.library[i].source == source)
            };
            self.lib_sel = view
                .selected
                .as_ref()
                .filter(|_| view.selected_index == self.lib_sel)
                .and_then(find)
                .unwrap_or_else(|| self.lib_sel.min(view.indices.len().saturating_sub(1)));
            self.last_play_idx = view
                .last_played
                .as_ref()
                .filter(|_| view.last_played_index == self.last_play_idx)
                .and_then(find)
                .unwrap_or(0);
            // A hidden last-played source keeps its identity. Returning it to
            // the filter must not inherit the temporary row-zero fallback.
            if let Some(identity) = self.last_played.latest_identity() {
                self.last_play_idx = view.indices.iter()
                    .position(|&i| identity.matches_current(&self.library[i], &self.library_metadata.catalog)).unwrap_or(0);
            }
            let stride = view.stride.max(18.0);
            view.pending_offset = Some(if query_changed || selected_crate_changed {
                self.lib_sel as f32 * stride
            } else {
                view.top
                    .as_ref()
                    .and_then(find)
                    .map(|i| i as f32 * stride + view.top_fraction)
                    .unwrap_or(view.offset)
            });
            view.cells.clear();
            view.library = Arc::downgrade(&self.library);
            view.query.clone_from(&self.lib_filter);
            view.searched_all = view.search_all;
            view.crate_id.clone_from(selected_crate);
            view.collection_revision = collection_revision;
            view.catalog = Arc::downgrade(&self.library_metadata.catalog);
            view.selected = view
                .indices
                .get(self.lib_sel)
                .map(|&i| self.library[i].source.clone());
            view.selected_index = self.lib_sel;
            view.last_played = view
                .indices
                .get(self.last_play_idx)
                .map(|&i| self.library[i].source.clone());
            view.last_played_index = self.last_play_idx;
            #[cfg(test)]
            {
                view.stats.rebuilds += usize::from(incremental.is_none());
                view.stats.updates += usize::from(incremental.is_some());
                view.stats.examined += incremental.map_or(self.library.len(),|changed|changed.len());
            }
        }
    }

    pub(super) fn selected_library_item(&mut self) -> Option<&LibItem> {
        self.refresh_library_view();
        self.library_view
            .indices
            .get(self.lib_sel)
            .map(|&index| &self.library[index])
    }

    pub(super) fn remember_crate_viewport(&mut self, offset: f32, height: f32, stride: f32) {
        let view = &mut self.library_view;
        view.offset = offset;
        view.height = height;
        view.stride = stride;
        let top = (offset / stride).floor() as usize;
        let source = view
            .indices
            .get(top)
            .map(|&index| &self.library[index].source);
        if source != view.top.as_ref() {
            view.top = source.cloned();
        }
        view.top_fraction = offset % stride;
        self.refresh_library_view();
    }

    pub(super) fn crate_navigation(&mut self, ui: &mut Ui, id: egui::Id) {
        if !ui.memory(|memory| memory.has_focus(id)) || self.library_view.indices.is_empty() {
            return;
        }
        ui.memory_mut(|memory| {
            memory.set_focus_lock_filter(
                id,
                egui::EventFilter {
                    vertical_arrows: true,
                    ..Default::default()
                },
            )
        });
        let old = self.lib_sel;
        let last = self.library_view.indices.len() - 1;
        let page = (self.library_view.height / self.library_view.stride.max(18.0))
            .floor()
            .max(1.0) as usize;
        ui.input_mut(|input| {
            for (key, amount) in [(Key::ArrowUp, 1), (Key::PageUp, page)] {
                if input.consume_key(egui::Modifiers::NONE, key) {
                    self.lib_sel = self.lib_sel.saturating_sub(amount);
                }
            }
            for (key, amount) in [(Key::ArrowDown, 1), (Key::PageDown, page)] {
                if input.consume_key(egui::Modifiers::NONE, key) {
                    self.lib_sel = self.lib_sel.saturating_add(amount).min(last);
                }
            }
            if input.consume_key(egui::Modifiers::NONE, Key::Home) {
                self.lib_sel = 0;
            }
            if input.consume_key(egui::Modifiers::NONE, Key::End) {
                self.lib_sel = last;
            }
        });
        if ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, Key::Enter)) {
            self.load_sel(self.load_target() as u8);
        }
        if old != self.lib_sel {
            let top = self.lib_sel as f32 * self.library_view.stride;
            let bottom = top + self.library_view.stride;
            if top < self.library_view.offset {
                self.library_view.pending_offset = Some(top);
            } else if bottom > self.library_view.offset + self.library_view.height {
                self.library_view.pending_offset = Some(bottom - self.library_view.height);
            }
            self.refresh_library_view();
        }
    }
}

/// Weak references keep retired large crate/index allocations out of the
/// controller bridge. Resolution fails closed if a new GUI view replaced them.
pub(super) struct PublishedView {
    pub library: Weak<Vec<LibItem>>,
    pub indices: Weak<Vec<usize>>,
}
impl crate::engine::ui_requests::SelectionView for PublishedView {
    fn len(&self) -> Option<usize> {
        Some(self.indices.upgrade()?.len())
    }
    fn selection(&self, index: usize) -> Option<Arc<Selection>> {
        let library = self.library.upgrade()?;
        let indices = self.indices.upgrade()?;
        let item = library.get(*indices.get(index)?)?;
        Some(Arc::new(Selection {
            source: item.source.clone(),
            title: item.title.clone(), fingerprint: item.fingerprint, }))
    }
}
