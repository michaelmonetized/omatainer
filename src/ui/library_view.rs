//! Filtered indices are rebuilt only when the immutable crate or query changes.
//! Formatted cells live only for the viewport (including egui's bounded overscan).
use super::*;
use std::sync::Weak;

#[derive(Default)]
pub(super) struct LibraryView {
    library: Weak<Vec<LibItem>>,
    query: String,
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

pub(super) struct Cells {
    pub bpm: String,
    pub length: String,
    pub played: String,
    pub played_at: Option<SystemTime>,
    pub played_tooltip: String,
    pub played_refresh_at: Option<SystemTime>,
    played_clock: SystemTime,
}

impl Cells {
    pub fn new(item: &LibItem, played_at: Option<SystemTime>, now: SystemTime) -> Self {
        let played = play_time::format(played_at, now);
        Self {
            bpm: item.bpm.cell(),
            length: fmt_len(item.length),
            played: played.label,
            played_tooltip: played.tooltip,
            played_refresh_at: played.next_change,
            played_clock: now,
            played_at,
        }
    }

    pub fn refresh_play(&mut self, played_at: Option<SystemTime>, now: SystemTime) -> bool {
        let expired = played_at.is_some() && (now < self.played_clock
            || self.played_refresh_at.is_some_and(|deadline| now >= deadline));
        if self.played_at == played_at && !expired { return false; }
        let played = play_time::format(played_at, now);
        self.played_at = played_at;
        self.played = played.label;
        self.played_tooltip = played.tooltip;
        self.played_refresh_at = played.next_change;
        self.played_clock = now;
        true
    }
}

#[cfg(test)]
#[derive(Default, Clone, Copy, Debug)]
pub(super) struct ViewStats {
    pub rebuilds: usize,
    pub examined: usize,
    pub rendered: usize,
    pub formatted: usize,
}

impl App {
    pub(super) fn refresh_library_view(&mut self) {
        let view = &mut self.library_view;
        // The weak reference prevents allocation-address reuse and makes
        // Arc::make_mut detach as well. It never owns the old crate's elements,
        // so the scan worker remains responsible for retiring the large Vec.
        let library_changed = view.library.as_ptr() != Arc::as_ptr(&self.library);
        let query_changed = view.query != self.lib_filter;
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
        if library_changed || query_changed {
            let q = self.lib_filter.to_lowercase();
            let indices = Arc::make_mut(&mut view.indices);
            indices.clear();
            indices.extend(self.library.iter().enumerate().filter_map(|(index, item)| {
                (q.is_empty()
                    || item.title.to_lowercase().contains(&q)
                    || item.artist.to_lowercase().contains(&q))
                .then_some(index)
            }));
            // Keep the worker's BPM/key/title order; never sort on a steady frame.
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
                    .position(|&i| identity.matches(&self.library[i])).unwrap_or(0);
            }
            let stride = view.stride.max(18.0);
            view.pending_offset = Some(if query_changed {
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
                view.stats.rebuilds += 1;
                view.stats.examined += self.library.len();
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
            title: item.title.clone(),
        }))
    }
}
