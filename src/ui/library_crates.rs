//! Stable catalog identities and worker receipts back every collection edit.
//! View selection is local; collection persistence never runs on the GUI thread.
use super::*;
use crate::library::{crates::{CrateId, Edit}, TrackId};
use library_metadata::{CollectionAction, CollectionOutcome, CollectionToken};
use std::collections::{BTreeMap, HashMap, HashSet};

const MAX_SELECTION: usize = 4096;
#[derive(Default)]
pub(super) struct Crates {
    pub open: bool,
    pub selected: Option<CrateId>,
    revision: Option<u64>,
    tree: Vec<(usize, usize)>,
    parents: HashMap<CrateId, Option<CrateId>>,
    destination: Option<CrateId>,
    members: BTreeMap<usize, TrackId>,
    name: String,
    position: usize,
    member_cursor: usize,
    pub(super) message: String,
    pub(super) pending: Option<(CollectionToken, CollectionAction)>,
    retry: Option<(u64, CollectionAction)>,
    delete: Option<Deletion>,
    #[cfg(test)]
    tree_rebuilds: usize,
}
struct Deletion {
    revision: u64,
    id: CrateId,
    name: String,
    crates: usize,
    memberships: usize,
}
impl App {
    #[cfg(test)]
    pub(super) fn crate_evidence(&self) -> serde_json::Value {
        serde_json::json!({"open":self.library_crates.open,"selected":self.library_crates.selected,
            "pending":self.library_crates.pending.is_some(),"message":self.library_crates.message,
            "insert_position":self.library_crates.position,
            "durable":self.library_metadata.durable,"owner_active":self.library_metadata.active(),
            "forest":self.library_metadata.catalog.crates,
            "visible":self.library_view.indices.iter().map(|&row|self.library_metadata.catalog.track(&self.library[row].source).map(|track|track.id.clone())).collect::<Vec<_>>()})
    }

    pub(super) fn refresh_named_crates(&mut self) {
        let forest = &self.library_metadata.catalog.crates;
        let state = &mut self.library_crates;
        if state.revision == Some(forest.revision()) { return; }
        let index: HashMap<_, _> = forest.nodes().iter().enumerate().map(|(i, n)| (&n.id, i)).collect();
        let mut selected = state.selected.clone();
        while selected.as_ref().is_some_and(|id| !index.contains_key(id)) {
            selected = selected.as_ref().and_then(|id| state.parents.get(id)).cloned().flatten();
        }
        if state.selected != selected {
            state.message = "Selected crate no longer exists; showing its nearest surviving parent or All tracks.".into();
            state.name = selected.as_ref().and_then(|id| forest.node(id)).map(|node| node.name.clone()).unwrap_or_else(|| "New crate".into());
            state.selected = selected;
            state.members.clear();
            state.member_cursor = 1;
            state.position = 1;
        }
        state.tree.clear();
        state.parents.clear();
        let mut stack: Vec<(&CrateId, Option<&CrateId>, usize)> = forest.roots().iter().rev().map(|id| (id, None, 0usize)).collect();
        while let Some((id, parent, depth)) = stack.pop() {
            let Some(&row) = index.get(id) else { continue };
            state.tree.push((row, depth));
            state.parents.insert(id.clone(), parent.cloned());
            stack.extend(forest.nodes()[row].children.iter().rev().map(|child| (child, Some(id), depth + 1)));
        }
        if let Some(node) = state.selected.as_ref().and_then(|id| forest.node(id)) {
            let retained: HashSet<_> = state.members.values().collect();
            let refreshed = node.members.iter().enumerate().filter(|(_, id)| retained.contains(id)).map(|(row, id)| (row, id.clone())).collect();
            state.members = refreshed;
        } else { state.members.clear(); }
        if state.destination.as_ref().is_some_and(|id| !index.contains_key(id)) { state.destination = None; }
        if state.delete.as_ref().is_some_and(|request| request.revision != forest.revision()) { state.delete = None; }
        state.revision = Some(forest.revision());
        #[cfg(test)] { state.tree_rebuilds += 1; }
    }

    pub(super) fn choose_named_crate(&mut self, id: Option<CrateId>) {
        self.refresh_named_crates();
        if id.as_ref().is_some_and(|id| self.library_metadata.catalog.crates.node(id).is_none()) {
            self.library_crates.message = "This crate is unavailable; showing All tracks. The project does not recreate catalog collections.".into();
            self.library_crates.selected = None;
        } else { self.library_crates.selected = id; }
        self.library_crates.members.clear();
        self.library_crates.delete = None;
        self.library_crates.name = self.library_crates.selected.as_ref()
            .and_then(|id| self.library_metadata.catalog.crates.node(id)).map(|n| n.name.clone()).unwrap_or_else(|| "New crate".into());
        self.library_crates.position = 1;
        self.refresh_library_view();
        self.publish_library_selection();
    }

    pub(super) fn poll_named_crates(&mut self) {
        if let Some(receipt) = self.library_metadata.take_collection_result() {
            if self.library_crates.pending.as_ref().is_some_and(|(token, _)| token.id == receipt.id) {
                let (_, action) = self.library_crates.pending.take().unwrap();
                self.library_crates.retry = None;
                let annotation = matches!(&action, CollectionAction::Annotate { .. });
                self.library_crates.message = match receipt.outcome {
                    CollectionOutcome::Read => "Crates refreshed from the catalog owner.".into(),
                    CollectionOutcome::Durable { changed } if annotation => if changed { "Annotations saved.".into() } else { "Annotations already match; no changes needed.".into() },
                    CollectionOutcome::Durable { changed } => if changed { "Crate changes saved.".into() } else { "Crates already match; no changes needed.".into() },
                    CollectionOutcome::CommittedUnconfirmed(error) => format!("Crate changes committed; durability unconfirmed: {error}. Do not repeat this edit."),
                    CollectionOutcome::Unknown(error) => format!("Crate outcome unknown: {error}. Reopen the catalog before repeating this edit."),
                    CollectionOutcome::Rejected(error) => {
                        self.library_crates.retry = Some((receipt.revision, action));
                        format!("Crate edit was not applied: {error}")
                    }
                };
                if let Some(created) = receipt.created { self.choose_named_crate(Some(created)); }
            }
        }
        self.refresh_named_crates();
    }

    pub(super) fn submit_crate_edit(&mut self, revision: u64, action: CollectionAction) {
        if self.project.committing() || !self.project.dialog_is_closed() || self.library_closing() {
            self.library_crates.message = "Finish or cancel the pending project/library decision before editing crates.".into();
            return;
        }
        if self.library_crates.pending.is_some() { return; }
        match self.library_metadata.edit_crates(revision, action.clone()) {
            Ok(token) => {
                self.library_crates.pending = Some((token, action));
                self.library_crates.retry = None;
                self.library_crates.delete = None;
                self.library_crates.message = "Crate edit queued; waiting for the catalog save receipt.".into();
            }
            Err(error) => self.library_crates.message = format!("Crate edit not queued: {error}"),
        }
    }

    fn captured_crate_rows(&mut self, all: bool) -> Result<Vec<TrackId>, String> {
        self.refresh_library_view();
        if all && self.library_view.indices.len() > MAX_SELECTION {
            return Err("At most 4096 filtered tracks can be added at once; narrow the search. Nothing was truncated.".into());
        }
        let rows: &[usize] = if all { &self.library_view.indices } else {
            self.library_view.indices.get(self.lib_sel).map(std::slice::from_ref).unwrap_or(&[])
        };
        if rows.is_empty() { return Err("Select a saved catalog track first.".into()); }
        rows.iter().map(|&row| {
            let item = &self.library[row];
            self.library_metadata.catalog.track_for_version(&item.source, item.fingerprint)
                .map(|track| track.id.clone()).ok_or_else(|| "A selected row is not saved in the catalog yet; no memberships changed.".into())
        }).collect()
    }

    fn selected_crate_members(&self) -> Vec<TrackId> {
        self.library_crates.members.values().cloned().collect()
    }

    fn request_crate_delete(&mut self) {
        let forest = &self.library_metadata.catalog.crates;
        let Some(id) = self.library_crates.selected.clone() else { return };
        let Some(node) = forest.node(&id) else { return };
        let mut request = Deletion { revision: forest.revision(), id: id.clone(), name: node.name.clone(), crates: 0, memberships: 0 };
        let by_id: HashMap<_, _> = forest.nodes().iter().map(|node| (&node.id, node)).collect();
        let mut stack = vec![&id];
        while let Some(id) = stack.pop() {
            if let Some(node) = by_id.get(id) {
                request.crates += 1;
                request.memberships += node.members.len();
                stack.extend(&node.children);
            }
        }
        self.library_crates.delete = Some(request);
    }

    pub(super) fn named_crate_selector(&mut self, ui: &mut Ui) {
        self.refresh_named_crates();
        let title = self.library_crates.selected.as_ref().and_then(|id| self.library_metadata.catalog.crates.node(id))
            .map(|node| node.name.as_str()).unwrap_or("All tracks");
        let response = ui.button(format!("{title} ▾"));
        accessibility::button(ui, &response, "Choose or edit named crates", None);
        help::annotate(ui, &response, HelpControl::NamedCrates);
        if response.clicked() {
            if self.library_crates.name.is_empty() { self.library_crates.name = "New crate".into(); }
            self.library_crates.open = true;
        }
    }

    pub(super) fn named_crates_ui(&mut self, ctx: &egui::Context) {
        if !self.library_crates.open { return; }
        self.refresh_named_crates();
        let mut open = true;
        egui::Window::new("Named crates").id(egui::Id::new("named-crates-window"))
            .open(&mut open).default_size(Vec2::new(700.0, 650.0)).show(ctx, |ui| {
                keyboard::block_for_dialog(ctx);
                let revision = self.library_metadata.catalog.crates.revision();
                // Every changing section consumes one stable parent slot. An old
                // native action cannot acquire a new meaning after a notice/tree changes.
                ui.scope(|ui| {
                    ui.label("Crates store ordered references only. Source audio and sampler banks are never moved or deleted.");
                    ui.label(&self.library_crates.message);
                });
                ui.push_id("crate-tree", |ui| {
                    let count = self.library_crates.tree.len();
                    let current = self.library_crates.selected.as_ref().and_then(|id| self.library_crates.tree.iter().position(|(index, _)| &self.library_metadata.catalog.crates.nodes()[*index].id == id)).map(|index| index + 1).unwrap_or(0);
                    let mut number = current as f32;
                    preferences::float_control(ui, "Crate tree row (0 = All tracks)", &mut number, 0.0, count as f32, 1.0, " row", HelpControl::NamedCrates);
                    if number.round() as usize != current {
                        let chosen = (number.round() as usize).checked_sub(1).and_then(|index| self.library_crates.tree.get(index)).map(|(index, _)| self.library_metadata.catalog.crates.nodes()[*index].id.clone());
                        self.choose_named_crate(chosen);
                    }
                    if ui.selectable_label(self.library_crates.selected.is_none(), "All tracks").help(ui, HelpControl::NamedCrates).clicked() { self.choose_named_crate(None); }
                    let rows = self.library_crates.tree.len();
                    let mut chosen = None;
                    let output = egui::ScrollArea::vertical().id_salt("named-crate-tree").max_height(150.0).show_rows(ui, 20.0, rows, |ui, range| {
                        for row in range {
                            let (index, depth) = self.library_crates.tree[row];
                            let node = &self.library_metadata.catalog.crates.nodes()[index];
                            ui.push_id((&node.id, "tree-row"), |ui| {
                                ui.horizontal(|ui| {
                                    ui.add_space(depth as f32 * 12.0);
                                    if ui.selectable_label(self.library_crates.selected.as_ref() == Some(&node.id), format!("{} · {} tracks", node.name, node.members.len()))
                                        .help(ui, HelpControl::NamedCrates).clicked() { chosen = Some(node.id.clone()); }
                                });
                            });
                        }
                    });
                    accessibility::scrollbars(ui, "Named crate tree", &output);
                    if let Some(id) = chosen { self.choose_named_crate(Some(id)); }
                });
                let id = self.library_crates.selected.clone();
                let available = self.library_crates.pending.is_none() && !self.project.committing() && self.project.dialog_is_closed() && !self.library_closing();
                ui.push_id(("crate-edit", revision, &id), |ui| {
                    ui.horizontal(|ui| {
                        let label = ui.label("Name");
                        let name = ui.add(egui::TextEdit::singleline(&mut self.library_crates.name).char_limit(256).desired_width(190.0)).labelled_by(label.id).help(ui, HelpControl::CrateName);
                        name.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, "Crate name"));
                    });
                    ui.horizontal(|ui| {
                        if ui.add_enabled(available, egui::Button::new("New root crate")).help(ui, HelpControl::CrateCreate).clicked() {
                            self.submit_crate_edit(revision, CollectionAction::Create { name: self.library_crates.name.trim().into(), parent: None, before: None });
                        }
                        if ui.add_enabled(available && id.is_some(), egui::Button::new("New child crate")).help(ui, HelpControl::CrateCreate).clicked() {
                            self.submit_crate_edit(revision, CollectionAction::Create { name: self.library_crates.name.trim().into(), parent: id.clone(), before: None });
                        }
                        if ui.add_enabled(available && id.is_some(), egui::Button::new("Rename crate")).help(ui, HelpControl::CrateName).clicked() {
                            self.submit_crate_edit(revision, CollectionAction::Edit(Edit::Rename { id: id.clone().unwrap(), name: self.library_crates.name.trim().into() }));
                        }
                        if ui.add_enabled(available && id.is_some(), egui::Button::new("Delete crate…")).help(ui, HelpControl::CrateDelete).clicked() { self.request_crate_delete(); }
                    });
                    self.crate_order_buttons(ui, revision, id.as_ref(), available);
                    ui.horizontal(|ui| {
                        if ui.add_enabled(id.is_some(), egui::Button::new("Use as destination")).help(ui, HelpControl::CrateDestination).clicked() { self.library_crates.destination = id.clone(); }
                        let destination = self.library_crates.destination.as_ref().and_then(|id| self.library_metadata.catalog.crates.node(id)).map(|node| node.name.as_str()).unwrap_or("none");
                        ui.label(format!("Destination: {destination}"));
                    });
                    self.refresh_library_view();
                    let target = self.library_crates.destination.clone();
                    let selected_source = self.library_view.indices.get(self.lib_sel).map(|&i| (self.library[i].source.clone(), self.library[i].fingerprint));
                    ui.push_id(("add-members", &target, self.library_view.generation, selected_source), |ui| {
                        ui.horizontal(|ui| {
                            for (label, all) in [("Add selected track", false), ("Add filtered tracks", true)] {
                                if ui.add_enabled(available && target.is_some(), egui::Button::new(label)).help(ui, HelpControl::CrateAdd).clicked() {
                                    match self.captured_crate_rows(all) {
                                        Ok(members) => self.submit_crate_edit(revision, CollectionAction::Edit(Edit::AddMembers { id: target.clone().unwrap(), members, before: None })),
                                        Err(error) => self.library_crates.message = error,
                                    }
                                }
                            }
                        });
                    });
                });
                ui.scope(|ui| self.crate_members_ui(ui, revision, id.as_ref(), available));
                ui.scope(|ui| self.crate_delete_ui(ui));
                ui.scope(|ui| {
                    ui.horizontal(|ui| {
                        ui.push_id("cancel-crate-request", |ui| {
                            if ui.add_enabled(self.library_crates.pending.is_some(), egui::Button::new("Cancel crate edit")).help(ui, HelpControl::CrateCancelEdit).clicked() {
                                if let Some((token, _)) = &self.library_crates.pending {
                                    self.library_crates.message = if token.cancel() { "Cancellation requested; waiting for the catalog owner." } else { "Publication has already begun; waiting for its actual outcome." }.into();
                                }
                            }
                        });
                        ui.push_id("retry-crate-request", |ui| {
                            if ui.add_enabled(available && self.library_crates.retry.is_some(), egui::Button::new("Retry rejected crate edit")).help(ui, HelpControl::CrateCancelEdit).clicked() {
                                let (expected, action) = self.library_crates.retry.take().unwrap();
                                self.submit_crate_edit(expected, action);
                            }
                        });
                        if ui.button("Close crates").help(ui, HelpControl::NamedCrates).clicked() { self.library_crates.open = false; }
                    });
                });
            });
        self.library_crates.open &= open;
    }
}

impl App {
    fn crate_order_buttons(&mut self, ui: &mut Ui, revision: u64, id: Option<&CrateId>, available: bool) {
        let parent = id.and_then(|id| self.library_crates.parents.get(id)).cloned().flatten();
        let forest = &self.library_metadata.catalog.crates;
        let siblings = parent.as_ref().and_then(|id| forest.node(id)).map(|node| node.children.as_slice()).unwrap_or(forest.roots());
        let index = id.and_then(|id| siblings.iter().position(|other| other == id));
        let up = index.filter(|&index| index > 0).map(|index| siblings[index - 1].clone());
        let down = index.filter(|&index| index + 1 < siblings.len()).map(|index| siblings.get(index + 2).cloned());
        let out = parent.as_ref().and_then(|parent| self.library_crates.parents.get(parent)).cloned().flatten();
        let destination = self.library_crates.destination.clone();
        ui.horizontal(|ui| {
            if ui.add_enabled(available && up.is_some(), egui::Button::new("Crate up")).help(ui, HelpControl::CrateOrder).clicked() {
                self.submit_crate_edit(revision, CollectionAction::Edit(Edit::MoveCrate { id: id.unwrap().clone(), parent: parent.clone(), before: up }));
            }
            if ui.add_enabled(available && down.is_some(), egui::Button::new("Crate down")).help(ui, HelpControl::CrateOrder).clicked() {
                self.submit_crate_edit(revision, CollectionAction::Edit(Edit::MoveCrate { id: id.unwrap().clone(), parent: parent.clone(), before: down.flatten() }));
            }
            if ui.add_enabled(available && parent.is_some(), egui::Button::new("Move crate out")).help(ui, HelpControl::CrateOrder).clicked() {
                self.submit_crate_edit(revision, CollectionAction::Edit(Edit::MoveCrate { id: id.unwrap().clone(), parent: out, before: None }));
            }
            ui.push_id(&destination, |ui| {
                if ui.add_enabled(available && id.is_some() && destination.is_some() && id != destination.as_ref(), egui::Button::new("Nest in destination")).help(ui, HelpControl::CrateDestination).clicked() {
                    self.submit_crate_edit(revision, CollectionAction::Edit(Edit::MoveCrate { id: id.unwrap().clone(), parent: destination.clone(), before: None }));
                }
            });
        });
    }

    fn crate_members_ui(&mut self, ui: &mut Ui, revision: u64, id: Option<&CrateId>, available: bool) {
        let catalog = self.library_metadata.catalog.clone();
        let Some(node) = id.and_then(|id| catalog.crates.node(id)) else {
            ui.label("All tracks keeps library sorting. Select a named crate to reorder or remove its direct members.");
            return;
        };
        ui.push_id(("crate-members", revision, id), |ui| {
            ui.label(format!("{} direct members; {} selected (maximum 4096). Child crates are separate views.", node.members.len(), self.library_crates.members.len()));
            let mut cursor = self.library_crates.member_cursor.max(1).min(node.members.len().max(1)) as f32;
            preferences::float_control(ui, "Member row", &mut cursor, 1.0, node.members.len().max(1) as f32, 1.0, " row", HelpControl::CrateMembership);
            self.library_crates.member_cursor = cursor.round() as usize;
            let member = node.members.get(self.library_crates.member_cursor.saturating_sub(1));
            ui.push_id(("toggle-cursor", member), |ui| {
                if ui.add_enabled(member.is_some(), egui::Button::new("Toggle member at row")).help(ui, HelpControl::CrateMembership).clicked() {
                    let member = member.unwrap();
                    let row = self.library_crates.member_cursor.saturating_sub(1);
                    if self.library_crates.members.remove(&row).is_none() {
                        if self.library_crates.members.len() < MAX_SELECTION { self.library_crates.members.insert(row, member.clone()); }
                        else { self.library_crates.message = "At most 4096 members can be selected for one edit.".into(); }
                    }
                }
            });
            // Names are read only for visible rows; stable IDs, not display
            // positions, own checkboxes and every subsequent mutation.
            let output = egui::ScrollArea::vertical().id_salt("crate-members-list").max_height(150.0).show_rows(ui, 20.0, node.members.len(), |ui, rows| {
                for row in rows {
                    let member = &node.members[row];
                    let track = self.library_metadata.collection_rows().track_index(member, &catalog).and_then(|index| catalog.tracks.get(index));
                    let title = track.map(|track| track.versions[track.current].metadata.title.as_str()).unwrap_or("Unavailable catalog identity");
                    let unavailable = self.library_metadata.collection_rows().row(member, &self.library, &catalog).is_none();
                    let title = if unavailable { format!("{title} · unavailable in current view · {}", member.0) } else { title.to_owned() };
                    let mut selected = self.library_crates.members.contains_key(&row);
                    ui.push_id(member, |ui| {
                        let response = ui.checkbox(&mut selected, format!("Member {}: {title}", row + 1)).help(ui, HelpControl::CrateMembership);
                        if response.changed() {
                            if selected && self.library_crates.members.len() < MAX_SELECTION { self.library_crates.members.insert(row, member.clone()); }
                            else if !selected { self.library_crates.members.remove(&row); }
                            else { self.library_crates.message = "At most 4096 members can be selected for one edit.".into(); }
                        }
                    });
                }
            });
            accessibility::scrollbars(ui, "Crate members", &output);
            let selected = self.selected_crate_members();
            // Salt actions with the exact bounded member selection. An exposed
            // Remove cannot later remove a different checkbox selection.
            ui.push_id(&selected, |ui| {
                ui.horizontal(|ui| {
                    if ui.button("Clear member selection").help(ui, HelpControl::CrateMembership).clicked() { self.library_crates.members.clear(); }
                    if ui.add_enabled(available && !selected.is_empty(), egui::Button::new("Remove selected memberships")).help(ui, HelpControl::CrateMembership).clicked() {
                        self.submit_crate_edit(revision, CollectionAction::Edit(Edit::RemoveMembers { id: node.id.clone(), members: selected.clone() }));
                    }
                });
                ui.horizontal(|ui| {
                    let mut position = self.library_crates.position.max(1) as f32;
                    preferences::float_control(ui, "Insert before row (last + 1 appends)", &mut position, 1.0, (node.members.len() + 1) as f32, 1.0, " row", HelpControl::CrateOrder);
                    self.library_crates.position = position.round() as usize;
                    ui.push_id(self.library_crates.position, |ui| {
                        if ui.add_enabled(available && !selected.is_empty(), egui::Button::new("Reorder selected members")).help(ui, HelpControl::CrateOrder).clicked() {
                            self.submit_crate_edit(revision, CollectionAction::Edit(Edit::MoveMembers {
                                source: node.id.clone(), destination: node.id.clone(), members: selected.clone(),
                                before: node.members.get(self.library_crates.position.saturating_sub(1)).cloned(),
                            }));
                        }
                    });
                });
                let destination = self.library_crates.destination.clone();
                ui.push_id(&destination, |ui| {
                    ui.horizontal(|ui| {
                        for (label, copying) in [("Copy selected to destination", true), ("Move selected to destination", false)] {
                            if ui.add_enabled(available && !selected.is_empty() && destination.is_some() && destination.as_ref() != id, egui::Button::new(label)).help(ui, HelpControl::CrateDestination).clicked() {
                                let destination = destination.clone().unwrap();
                                let edit = if copying { Edit::AddMembers { id: destination, members: selected.clone(), before: None } }
                                    else { Edit::MoveMembers { source: node.id.clone(), destination, members: selected.clone(), before: None } };
                                self.submit_crate_edit(revision, CollectionAction::Edit(edit));
                            }
                        }
                    });
                });
            });
        });
    }

    fn crate_delete_ui(&mut self, ui: &mut Ui) {
        let Some(request) = &self.library_crates.delete else { return };
        let revision = request.revision;
        let id = request.id.clone();
        let message = format!("Delete '{}' and its {} crate(s), removing {} memberships? Audio, library tracks and sampler banks are preserved.", request.name, request.crates, request.memberships);
        ui.push_id(("confirm-delete-crate", revision, &id), |ui| {
            ui.label(message);
            let mut confirm = false;
            let mut cancel = false;
            ui.horizontal(|ui| {
                confirm = ui.button("Confirm delete crate subtree").help(ui, HelpControl::CrateDelete).clicked();
                cancel = ui.button("Keep crate").help(ui, HelpControl::CrateDelete).clicked();
            });
            if confirm { self.submit_crate_edit(revision, CollectionAction::Edit(Edit::DeleteSubtree { id: id.clone() })); }
            else if cancel { self.library_crates.delete = None; }
        });
    }
}

#[cfg(test)]
mod tests;
