use super::*;
use crate::library::{
    file_management::{transfer::journal::Recovery, Duplicate},
    TrackId,
};
use library_metadata::{
    files::{Action, Report, Selection},
    CollectionAction, CollectionOutcome,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Operation {
    #[default]
    Copy,
    Move,
    Remove,
    Merge,
}
#[derive(Default)]
pub(super) struct Panel {
    pub open: bool,
    selection: Option<Selection>,
    labels: Vec<String>,
    expected: u64,
    destination: String,
    operation: Operation,
    duplicates: Vec<Duplicate>,
    duplicate: Option<TrackId>,
    preparation_from_duplicate: bool,
    recovery: Option<Recovery>,
    saved_moves: Vec<Recovery>,
    reviewed: Option<Action>,
    next_review: u64,
    pub message: String,
}
impl Panel {
    fn draft(&self) -> Result<Action, String> {
        let selection = self
            .selection
            .clone()
            .ok_or("Capture saved library rows first")?;
        Ok(match self.operation {
            Operation::Copy | Operation::Move => Action::Transfer {
                selection,
                destination: PathBuf::from(self.destination.trim()),
                move_files: self.operation == Operation::Move,
            },
            Operation::Remove => Action::Remove(selection),
            Operation::Merge => {
                if selection.ids.len() != 1 {
                    return Err("Capture one saved track before reviewing a duplicate merge".into());
                }
                let duplicate = self
                    .duplicate
                    .clone()
                    .ok_or("Find and select a likely duplicate first")?;
                let retain = selection.ids[0].clone();
                let preparation = if self.preparation_from_duplicate {
                    duplicate.clone()
                } else {
                    retain.clone()
                };
                Action::Merge {
                    selection: Selection {
                        catalog: selection.catalog,
                        ids: vec![retain.clone(), duplicate],
                    },
                    retain,
                    preparation,
                }
            }
        })
    }
    pub(super) fn accept(&mut self, report: Report, outcome: &CollectionOutcome) {
        if matches!(
            outcome,
            CollectionOutcome::Read
                | CollectionOutcome::Durable { .. }
                | CollectionOutcome::CommittedUnconfirmed(_)
        ) {
            if !report.duplicates.is_empty() || matches!(outcome, CollectionOutcome::Read) {
                self.duplicates = report.duplicates;
                self.duplicate = None;
            }
            self.recovery = report.recovery;
            self.saved_moves = report.archives;
            if !report.message.is_empty() {
                self.message = report.message;
            }
            self.reviewed = None;
        }
    }
}
impl App {
    fn capture_library_files(&mut self, filtered: bool) {
        if let Some(previous) = self.library_files.selection.take() {
            let Selection { catalog, ids } = previous;
            if let Err(catalog) = self.library_metadata.retire_file_review(catalog) {
                self.library_files.selection = Some(Selection { catalog, ids });
                self.library_files.message =
                    "Wait for the catalog worker to retire the previous file review.".into();
                return;
            }
        }
        self.refresh_library_view();
        let rows: &[usize] = if filtered {
            &self.library_view.indices
        } else {
            self.library_view
                .indices
                .get(self.lib_sel)
                .map(std::slice::from_ref)
                .unwrap_or(&[])
        };
        let panel = &mut self.library_files;
        panel.selection = None;
        panel.reviewed = None;
        panel.duplicates.clear();
        panel.duplicate = None;
        panel.labels.clear();
        if rows.is_empty() || rows.len() > crate::library::file_management::MAX_SELECTION {
            panel.message="Capture 1–128 saved rows; narrow the filter for larger batches. Nothing is truncated.".into();
            return;
        }
        let ids: Option<Vec<_>> = rows
            .iter()
            .map(|&row| {
                let item = self.library.get(row)?;
                self.library_metadata
                    .catalog
                    .track_for_version(&item.source, item.fingerprint)
                    .filter(|track| track.versions[track.current].fingerprint == item.fingerprint)
                    .map(|track| track.id.clone())
            })
            .collect();
        let Some(ids) = ids else {
            panel.message = "Finish the library scan and save before reviewing files.".into();
            return;
        };
        panel.expected = self.library_metadata.catalog.crates.revision();
        panel.labels = rows
            .iter()
            .take(8)
            .map(|&row| {
                format!(
                    "{} · {:?}",
                    self.library[row].title, self.library[row].source
                )
            })
            .collect();
        panel.message=format!("Captured {} saved track identities and prepared versions. Review before applying a file operation.",ids.len());
        panel.selection = Some(Selection {
            catalog: self.library_metadata.catalog.clone(),
            ids,
        });
    }
    pub(super) fn library_files_ui(&mut self, ctx: &egui::Context) {
        if !self.library_files.open {
            return;
        }
        let available = self.library_crates.pending.is_none()
            && !self.library_metadata.active()
            && !self.library_closing()
            && !self.project.committing()
            && self.project.dialog_is_closed();
        let mut open = true;
        let mut capture = None;
        let mut submit = None;
        egui::Window::new("Music files").id(egui::Id::new("library-music-files")).open(&mut open).default_size(Vec2::new(770.0,650.0)).vscroll(true).show(ctx,|ui|{
            keyboard::block_for_dialog(ctx);
            let panel=&mut self.library_files;
            ui.label("Copy saves the new copy as the library location and keeps the original. Move saves the new location, then keeps the original in recoverable storage. Remove reference keeps source audio in place.");
            ui.horizontal(|ui|{
                if ui.add_enabled(available,egui::Button::new("Capture selected music file")).clicked(){capture=Some(false);}
                if ui.add_enabled(available,egui::Button::new("Capture filtered music files")).clicked(){capture=Some(true);}
                if ui.add_enabled(available,egui::Button::new("Inspect file recovery")).clicked(){submit=Some((panel.expected,Action::Inspect));}
            });
            ui.label(&panel.message);
            if let Some(selection)=&panel.selection {
                ui.label(format!("{} captured track(s)",selection.ids.len()));
                for label in &panel.labels {ui.label(label);}
            }
            let previous=(panel.operation,panel.destination.clone(),panel.duplicate.clone(),panel.preparation_from_duplicate);
            ui.add_enabled_ui(available,|ui|{
                ui.horizontal(|ui|{ui.selectable_value(&mut panel.operation,Operation::Copy,"Copy");ui.selectable_value(&mut panel.operation,Operation::Move,"Move with recovery");ui.selectable_value(&mut panel.operation,Operation::Remove,"Remove reference");ui.selectable_value(&mut panel.operation,Operation::Merge,"Merge duplicates");});
                if matches!(panel.operation,Operation::Copy|Operation::Move) {
                    let label=ui.label("Existing destination folder");let response=ui.add(egui::TextEdit::singleline(&mut panel.destination).char_limit(4096).desired_width(620.0)).labelled_by(label.id);response.widget_info(||egui::WidgetInfo::labeled(egui::WidgetType::TextEdit,true,"Music file destination folder"));
                }
                if panel.operation==Operation::Merge {
                    if ui.add_enabled(panel.selection.as_ref().is_some_and(|s|s.ids.len()==1),egui::Button::new("Find likely duplicates")).clicked(){submit=Some((panel.expected,Action::Duplicates(panel.selection.clone().unwrap())));}
                    ui.label("Matching names are candidates only. Merge requires identical complete bytes. The captured track keeps its stable identity; choose which prepared version is current.");
                    for duplicate in &panel.duplicates {ui.selectable_value(&mut panel.duplicate,Some(duplicate.second.clone()),format!("{} · {}",duplicate.label,duplicate.reason));}
                    ui.checkbox(&mut panel.preparation_from_duplicate,"Use duplicate's current preparation");
                }
            });
            if previous!=(panel.operation,panel.destination.clone(),panel.duplicate.clone(),panel.preparation_from_duplicate){panel.reviewed=None;}
            if ui.add_enabled(available && panel.selection.is_some() && panel.recovery.is_none(),egui::Button::new("Review music file operation")).clicked(){match panel.draft().and_then(|action|{action.validate()?;Ok(action)}) {Ok(action)=>{if let Some(id)=panel.next_review.checked_add(1){panel.next_review=id;panel.reviewed=Some(action);panel.message="Review captured rows, destination and preparation choice. Apply rechecks the saved catalog and every changed file.".into();}else{panel.message="File review identity exhausted; reopen the app.".into();}},Err(error)=>panel.message=error}}
            if ui.push_id(("apply-music-file-review",panel.next_review),|ui|ui.add_enabled(available && panel.reviewed.is_some(),egui::Button::new("Apply reviewed music file operation"))).inner.clicked(){submit=Some((panel.expected,panel.reviewed.take().unwrap()));}
            if let Some(recovery)=&panel.recovery {
                ui.separator();ui.label(format!("Recoverable move {} · {} file(s)",recovery.id,recovery.files));ui.label(&recovery.description);
                if ui.add_enabled(available,egui::Button::new("Restore reviewed original music files")).clicked(){submit=Some((panel.expected,Action::Restore{id:recovery.id.clone()}));}
                if ui.add_enabled(available && recovery.active,egui::Button::new("Keep reviewed moved music locations")).clicked(){submit=Some((panel.expected,Action::Keep{id:recovery.id.clone()}));}
            }
            for recovery in &panel.saved_moves {
                ui.push_id((&recovery.id,"saved-move"),|ui|{
                    ui.label(format!("Saved move {} · {} file(s)",recovery.id,recovery.files));
                    if ui.add_enabled(available && panel.recovery.is_none(),egui::Button::new(format!("Restore saved move {}",recovery.id))).clicked(){submit=Some((panel.expected,Action::Restore{id:recovery.id.clone()}));}
                });
            }
            if let Some((token,CollectionAction::Files(_)))=&self.library_crates.pending {if ui.button("Cancel pending music file operation").clicked(){token.cancel();}}
            ui.label(&self.library_crates.message);
        });
        self.library_files.open = open;
        if !open {
            self.library_files.reviewed = None;
        }
        if let Some(filtered) = capture {
            self.capture_library_files(filtered);
        }
        if let Some((expected, action)) = submit {
            self.submit_crate_edit(expected, CollectionAction::Files(action));
        }
    }
}

#[cfg(test)]
mod tests;
