use super::*;
use crate::library::{TrackId, crates::CrateId};
use library_metadata::CollectionAction;
use std::collections::HashSet;

const MAX_TRACKS: usize = 4096;
const MAX_BYTES: usize = 2 * 1024 * 1024;
struct Entry {
    id: TrackId,
    selection: Arc<Selection>,
    queued_at: u64,
}
#[derive(Default)]
pub(super) struct Panel {
    pub open: bool,
    entries: Vec<Entry>,
    selected: Option<TrackId>,
    retain: bool,
    revision: u64,
    name: String,
    pub message: String,
    pub saved: Option<CrateId>,
    preview: Option<(u8, u64)>,
}
impl App {
    fn prepare_available(&self) -> bool {
        !self.project.committing() && self.project.dialog_is_closed() && !self.library_closing()
    }

    /// Capture upcoming tracks in published browser order.
    /// Takes a selected/filtered flag; either appends every new identity or leaves the queue unchanged.
    pub(super) fn prepare_browser_rows(&mut self, all: bool) {
        self.refresh_library_view();
        let rows: &[usize] = if all {
            &self.library_view.indices
        } else {
            self.library_view
                .indices
                .get(self.lib_sel)
                .map(std::slice::from_ref)
                .unwrap_or(&[])
        };
        if rows.is_empty() || rows.len() > MAX_TRACKS {
            self.library_prepare.message =
                "Select 1–4096 saved tracks; narrow the crate first. Nothing was truncated.".into();
            return;
        }
        let selections = rows
            .iter()
            .map(|&row| {
                let item = &self.library[row];
                Arc::new(Selection {
                    source: item.source.clone(),
                    title: item.title.clone(),
                    fingerprint: item.fingerprint,
                })
            })
            .collect();
        self.prepare_selections(selections);
    }

    /// Append version-bound selections from a native or hardware request.
    /// Takes captured selections; rejects stale identities, capacity and closed decisions before changing the queue.
    pub(super) fn prepare_selections(&mut self, selections: Vec<Arc<Selection>>) {
        let result = (|| {
            if !self.prepare_available() {
                return Err(
                    "Finish the project/library decision before changing the prepare queue".into(),
                );
            }
            if selections.is_empty() || selections.len() > MAX_TRACKS {
                return Err("Select 1–4096 saved tracks; nothing was truncated".into());
            }
            let now = self
                .engine
                .performance_history
                .as_ref()
                .and_then(|handle| handle.clock())
                .unwrap_or(0);
            let existing: HashMap<_, _> = self
                .library_prepare
                .entries
                .iter()
                .map(|entry| (&entry.id, &entry.selection))
                .collect();
            let mut seen: HashSet<_> = existing.keys().map(|id| (*id).clone()).collect();
            let mut additions = Vec::new();
            let mut bytes: usize = self
                .library_prepare
                .entries
                .iter()
                .map(|entry| {
                    entry.selection.bytes() + entry.id.0.capacity() + std::mem::size_of::<Entry>()
                })
                .sum();
            for selection in selections {
                let track = self
                    .library_metadata
                    .catalog
                    .track(&selection.source)
                    .filter(|track| {
                        track.versions[track.current].fingerprint == selection.fingerprint
                    })
                    .ok_or_else(|| {
                        "A captured track changed or is not saved yet; no queue entries were added"
                            .to_string()
                    })?;
                if matches!(
                    selection.source,
                    LibSource::File(_) | LibSource::Removable { .. }
                ) && selection.fingerprint.is_none()
                {
                    return Err(
                        "This file has no verified source version; rescan before preparing it"
                            .into(),
                    );
                }
                if let Some(existing) = existing.get(&track.id) {
                    if existing.source != selection.source
                        || existing.fingerprint != selection.fingerprint
                    {
                        return Err("An older version of this track is queued; remove it or explicitly restore its crate".into());
                    }
                }
                if !seen.insert(track.id.clone()) {
                    continue;
                }
                bytes = bytes
                    .saturating_add(selection.bytes())
                    .saturating_add(track.id.0.capacity() + std::mem::size_of::<Entry>());
                if seen.len() > MAX_TRACKS || bytes > MAX_BYTES {
                    return Err("Prepare queue accepts at most 4096 tracks and 2 MiB of references; nothing was added".into());
                }
                additions.push(Entry {
                    id: track.id.clone(),
                    selection,
                    queued_at: now,
                });
            }
            let next = self
                .library_prepare
                .revision
                .checked_add(1)
                .ok_or("Prepare queue revision exhausted")?;
            let count = additions.len();
            self.library_prepare.entries.extend(additions);
            self.library_prepare.revision = next;
            if self.library_prepare.selected.is_none() {
                self.library_prepare.selected = self
                    .library_prepare
                    .entries
                    .first()
                    .map(|entry| entry.id.clone());
            }
            Ok(count)
        })();
        self.library_prepare.message = match result {
            Ok(count) => format!(
                "Added {count} upcoming tracks; {} queued.",
                self.library_prepare.entries.len()
            ),
            Err(error) => error,
        };
        self.library_prepare.open = true;
        self.poll_prepare_queue();
    }

    fn prepare_selection(&self) -> Option<Arc<Selection>> {
        self.library_prepare
            .entries
            .iter()
            .find(|entry| Some(&entry.id) == self.library_prepare.selected.as_ref())
            .map(|entry| entry.selection.clone())
    }

    fn remove_prepared(&mut self, id: &TrackId) {
        if !self.prepare_available() {
            return;
        }
        let Some(next) = self.library_prepare.revision.checked_add(1) else {
            self.library_prepare.message = "Prepare queue revision exhausted".into();
            return;
        };
        let Some(index) = self
            .library_prepare
            .entries
            .iter()
            .position(|entry| &entry.id == id)
        else {
            return;
        };
        self.library_prepare.entries.remove(index);
        self.library_prepare.revision = next;
        if self.library_prepare.selected.as_ref() == Some(id) {
            self.library_prepare.selected = self
                .library_prepare
                .entries
                .get(index)
                .or_else(|| self.library_prepare.entries.last())
                .map(|entry| entry.id.clone());
        }
    }

    fn move_prepared(&mut self, id: &TrackId, up: bool) {
        if !self.prepare_available() {
            return;
        }
        let Some(index) = self
            .library_prepare
            .entries
            .iter()
            .position(|entry| &entry.id == id)
        else {
            return;
        };
        let destination = if up {
            index.checked_sub(1)
        } else {
            index
                .checked_add(1)
                .filter(|next| *next < self.library_prepare.entries.len())
        };
        let Some(destination) = destination else {
            return;
        };
        let Some(next) = self.library_prepare.revision.checked_add(1) else {
            self.library_prepare.message = "Prepare queue revision exhausted".into();
            return;
        };
        self.library_prepare.entries.swap(index, destination);
        self.library_prepare.revision = next;
    }

    /// Restore the current versions of every saved manual-crate member.
    /// Takes the crate identity; atomically replaces the queue in stored order, never substituting missing identities.
    fn restore_prepare_crate(&mut self, id: &CrateId) {
        let result = (|| {
            if !self.prepare_available() {
                return Err("Finish the project/library decision before restoring a queue".into());
            }
            let catalog = &self.library_metadata.catalog;
            let node = catalog
                .crates
                .node(id)
                .ok_or("The saved crate is unavailable")?;
            if node.smart_rule.is_some()
                || node.members.is_empty()
                || node.members.len() > MAX_TRACKS
            {
                return Err("Restore a manual crate containing 1–4096 ordered tracks".into());
            }
            let now = self
                .engine
                .performance_history
                .as_ref()
                .and_then(|handle| handle.clock())
                .unwrap_or(0);
            let mut entries = Vec::with_capacity(node.members.len());
            let mut bytes = 0usize;
            for id in &node.members {
                let index = self.library_metadata.collection_rows().track_index(id,catalog).ok_or("A saved track or its current catalog publication is unavailable; queue unchanged")?;
                let track = &catalog.tracks[index];
                let version = &track.versions[track.current];
                if matches!(
                    track.source,
                    LibSource::File(_) | LibSource::Removable { .. }
                ) && version.fingerprint.is_none()
                {
                    return Err("A saved file has no verified version; rescan first".into());
                }
                let selection = Arc::new(Selection {
                    source: track.source.clone(),
                    title: version.metadata.title.clone(),
                    fingerprint: version.fingerprint,
                });
                bytes = bytes
                    .saturating_add(selection.bytes())
                    .saturating_add(id.0.capacity() + std::mem::size_of::<Entry>());
                if bytes > MAX_BYTES {
                    return Err(
                        "Saved crate exceeds the 2 MiB prepare-reference limit; queue unchanged"
                            .into(),
                    );
                }
                entries.push(Entry {
                    id: id.clone(),
                    selection,
                    queued_at: now,
                });
            }
            let next = self
                .library_prepare
                .revision
                .checked_add(1)
                .ok_or("Prepare queue revision exhausted")?;
            self.library_prepare.entries = entries;
            self.library_prepare.selected = self
                .library_prepare
                .entries
                .first()
                .map(|entry| entry.id.clone());
            self.library_prepare.revision = next;
            Ok(node.name.clone())
        })();
        self.library_prepare.message = match result {
            Ok(name) => format!("Restored current saved versions from {name}."),
            Err(error) => error,
        };
        self.poll_prepare_queue();
    }

    fn save_prepare_crate(&mut self) {
        if self.library_prepare.entries.is_empty() {
            self.library_prepare.message = "Queue at least one saved track first.".into();
            return;
        }
        if self.library_crates.pending.is_some() {
            self.library_prepare.message = "Wait for the pending catalog edit receipt.".into();
            return;
        }
        self.submit_crate_edit(
            self.library_metadata.catalog.crates.revision(),
            CollectionAction::CreatePrepared {
                name: self.library_prepare.name.trim().into(),
                members: self
                    .library_prepare
                    .entries
                    .iter()
                    .map(|entry| entry.id.clone())
                    .collect(),
            },
        );
        self.library_prepare.message = self.library_crates.message.clone();
    }

    fn stop_prepare_preview(&mut self) {
        if let Some((deck, expected)) = self.library_prepare.preview {
            if self.submit(Command::DeckPreview {
                deck,
                expected,
                on: false,
            }) {
                self.library_prepare.preview = None;
            }
        }
    }

    fn preview_prepared(&mut self, deck: u8) {
        let Some(selection) = self.prepare_selection() else {
            return;
        };
        let Some(receipt) = self.prepared_receipt(deck, &selection) else {
            self.library_prepare.message =
                "Load this captured track on a paused deck before previewing it.".into();
            return;
        };
        if !self.prepare_available() {
            return;
        }
        self.stop_prepare_preview();
        let expected = receipt.history_key();
        if self.submit(Command::DeckPreview {
            deck,
            expected,
            on: true,
        }) {
            self.library_prepare.preview = Some((deck, expected));
            self.library_prepare.message = "Preview queued on the existing deck output. Stop preview restores its original position; playback stays paused.".into();
        }
    }

    fn prepared_receipt(&self, deck: u8, selection: &Selection) -> Option<Receipt> {
        let target = self.snap.decks.get(usize::from(deck))?;
        if target.playing || target.touching {
            return None;
        }
        let receipt = self.cue_receipt(target.receipt_key)?;
        self.playback_watches
            .iter()
            .any(|watch| {
                watch.matches_load(
                    &selection.source,
                    selection.fingerprint,
                    receipt.history_key(),
                )
            })
            .then_some(receipt)
    }

    /// Remove only queued versions with a complete confirmed playing-output window.
    /// Takes no arguments; loading, paused preview, mute, stale receipts and uncertain output retain the entry.
    pub(super) fn poll_prepare_queue(&mut self) {
        if !self.prepare_available() || !self.library_prepare.open {
            self.stop_prepare_preview();
        }
        let Some(handle) = &self.engine.performance_history else {
            return;
        };
        let enabled = !self.library_prepare.retain && !self.library_prepare.entries.is_empty();
        if let Err(error) = handle.set_prepare_monitor(enabled) {
            self.library_prepare.message = error.into();
            return;
        }
        if !enabled || !handle.can_measure() || !self.prepare_available() {
            return;
        }
        let events = [handle.digital_play(0), handle.digital_play(1)];
        let identities = &self.playback_watches;
        let removed: Vec<_> = self
            .library_prepare
            .entries
            .iter()
            .filter(|entry| {
                events.iter().flatten().any(|event| {
                    entry.queued_at != 0
                        && event.wall_ns > entry.queued_at
                        && identities.iter().any(|watch| {
                            watch.matches_load(
                                &entry.selection.source,
                                entry.selection.fingerprint,
                                event.load,
                            )
                        })
                })
            })
            .map(|entry| entry.id.clone())
            .collect();
        for id in &removed {
            self.remove_prepared(id);
        }
        if !removed.is_empty() {
            self.library_prepare.message = format!(
                "Removed {} tracks after confirmed playing output.",
                removed.len()
            );
        }
    }

    pub(super) fn prepare_queue_ui(&mut self, ctx: &egui::Context) {
        if !self.library_prepare.open {
            return;
        }
        if self.library_prepare.name.is_empty() {
            self.library_prepare.name = "Prepared set".into();
        }
        let mut open = true;
        egui::Window::new(tr!("Prepare queue")).id(egui::Id::new("prepare-queue-window"))
            .open(&mut open).default_size(Vec2::new(700.0,530.0)).show(ctx, |ui| {
                keyboard::block_for_dialog(ctx);
                ui.scope(|ui| {
                    ui.label(tr!("Upcoming tracks keep their captured source version. Saving creates an ordered manual crate."));
                    ui.label(&self.library_prepare.message);
                });
                let available = self.prepare_available();
                ui.push_id("prepare-add",|ui| {
                    ui.horizontal_wrapped(|ui| {
                        if ui.add_enabled(available,egui::Button::new(tr!("Queue selected track"))).help(ui,HelpControl::PrepareQueue).clicked() { self.prepare_browser_rows(false); }
                        if ui.add_enabled(available,egui::Button::new(tr!("Queue filtered crate"))).help(ui,HelpControl::PrepareQueue).clicked() { self.prepare_browser_rows(true); }
                        let response = ui.checkbox(&mut self.library_prepare.retain,tr!("Retain after play"));
                        help::annotate(ui,&response,HelpControl::PrepareQueue);
                    });
                    if !self.library_prepare.retain && !self.engine.performance_history.as_ref().is_some_and(|handle|handle.can_measure()) {
                        ui.label(tr!("Automatic removal unavailable for this output route; queued tracks retained."));
                    }
                    ui.label(tr!("Remove after play requires a complete 10 ms window of digital main output while playing. Loading and paused preview retain the track."));
                });
                let revision = self.library_prepare.revision;
                ui.push_id(("prepare-list",revision),|ui| {
                    let cursor = self.library_prepare.entries.iter().position(|entry|Some(&entry.id) == self.library_prepare.selected.as_ref()).unwrap_or(0);
                    let mut number = (cursor+1) as f32;
                    if !self.library_prepare.entries.is_empty() {
                        preferences::float_control(ui,"Prepare queue row",&mut number,1.0,self.library_prepare.entries.len() as f32,1.0," row",HelpControl::PrepareQueue);
                        self.library_prepare.selected = self.library_prepare.entries.get(number.round() as usize - 1).map(|entry|entry.id.clone());
                    }
                    let mut selected = None;
                    let output = egui::ScrollArea::vertical().id_salt("prepare-tracks").max_height(200.0)
                        .show_rows(ui,22.0,self.library_prepare.entries.len(),|ui,range| {
                            for index in range {
                                let entry = &self.library_prepare.entries[index];
                                ui.push_id(&entry.id,|ui| {
                                    let label = format!("{}. {}",index+1,entry.selection.title);
                                    let response = ui.selectable_label(Some(&entry.id)==self.library_prepare.selected.as_ref(),label).help(ui,HelpControl::PrepareQueue);
                                    if response.clicked() { selected = Some(entry.id.clone()); }
                                });
                            }
                        });
                    accessibility::scrollbars(ui,"Prepare queue tracks",&output);
                    if let Some(id) = selected { self.library_prepare.selected = Some(id); }
                    let id = self.library_prepare.selected.clone();
                    ui.horizontal_wrapped(|ui| {
                        if ui.add_enabled(available && id.is_some(),egui::Button::new(tr!("Move prepared track up"))).help(ui,HelpControl::PrepareQueue).clicked() { self.move_prepared(id.as_ref().unwrap(),true); }
                        if ui.add_enabled(available && id.is_some(),egui::Button::new(tr!("Move prepared track down"))).help(ui,HelpControl::PrepareQueue).clicked() { self.move_prepared(id.as_ref().unwrap(),false); }
                        if ui.add_enabled(available && id.is_some(),egui::Button::new(tr!("Remove prepared track"))).help(ui,HelpControl::PrepareQueue).clicked() { self.remove_prepared(id.as_ref().unwrap()); }
                    });
                });
                ui.push_id(("prepare-decks",revision,self.library_prepare.selected.clone()),|ui| {
                    let selection = self.prepare_selection();
                    ui.horizontal_wrapped(|ui| {
                        for deck in 0..DECKS as u8 {
                            let name = (b'A'+deck) as char;
                            if ui.add_enabled(available && selection.is_some(),egui::Button::new(crate::localization::format("Load prepared track on deck {0}",&[name.to_string()]))).help(ui,HelpControl::PrepareQueue).clicked() {
                                self.stop_prepare_preview(); self.load_source(deck,selection.as_deref());
                            }
                            let ready = selection.as_ref().is_some_and(|selection|self.prepared_receipt(deck,selection).is_some());
                            if ui.add_enabled(available && ready,egui::Button::new(crate::localization::format("Preview prepared track on deck {0}",&[name.to_string()]))).help(ui,HelpControl::PrepareQueue).clicked() { self.preview_prepared(deck); }
                        }
                    });
                    if ui.add_enabled(self.library_prepare.preview.is_some(),egui::Button::new(tr!("Stop prepared preview"))).help(ui,HelpControl::PrepareQueue).clicked() { self.stop_prepare_preview(); }
                    if let Some(selection) = selection {
                        if let Some(track) = self.library_metadata.catalog.track(&selection.source).filter(|track|track.versions[track.current].fingerprint == selection.fingerprint) {
                            let data = &track.versions[track.current].metadata;
                            ui.label(format!("{} · {} · {}",data.artist,data.bpm.label(),data.key));
                        } else { ui.label(tr!("Captured version changed in the catalog. Restore a saved crate explicitly to use current versions.")); }
                    }
                });
                ui.push_id("prepare-save",|ui| {
                    let label = ui.label(tr!("Crate name"));
                    let response = ui.add(egui::TextEdit::singleline(&mut self.library_prepare.name).char_limit(256)).labelled_by(label.id).help(ui,HelpControl::PrepareQueue);
                    response.widget_info(||egui::WidgetInfo::labeled(egui::WidgetType::TextEdit,true,"Saved queue name"));
                    ui.horizontal_wrapped(|ui| {
                        if ui.add_enabled(available && !self.library_prepare.entries.is_empty() && self.library_crates.pending.is_none(),egui::Button::new(tr!("Save prepare queue as crate"))).help(ui,HelpControl::PrepareQueue).clicked() { self.save_prepare_crate(); }
                        let selected = self.library_crates.selected.clone();
                        if ui.add_enabled(available && selected.is_some(),egui::Button::new(tr!("Restore selected named crate"))).help(ui,HelpControl::PrepareQueue).clicked() { self.stop_prepare_preview();self.restore_prepare_crate(selected.as_ref().unwrap()); }
                        let saved = self.library_prepare.saved.clone();
                        if ui.add_enabled(available && saved.is_some(),egui::Button::new(tr!("Restore last saved queue"))).help(ui,HelpControl::PrepareQueue).clicked() { self.stop_prepare_preview();self.restore_prepare_crate(saved.as_ref().unwrap()); }
                    });
                });
            });
        self.library_prepare.open = open;
        self.poll_prepare_queue();
    }
}

#[cfg(test)]
mod tests;
