//! Review captured versions before changing preparation protection on the catalog owner.
use super::*;
use crate::library::protection::{Locks, Patch, Target};
use library_metadata::CollectionAction;

#[derive(Default)]
pub(super) struct Panel {
    pub open: bool,
    targets: Vec<Target>,
    expected: u64,
    locks: Locks,
    change: [bool; 3],
    reviewed: Option<Patch>,
    next_review: u64,
    message: String,
}
impl Panel {
    /// Build selected lock changes.
    /// Takes this draft; returns explicit grid, tempo and metadata lock choices or an empty-review error.
    fn patch(&self) -> Result<Patch, String> {
        let patch = Patch {
            grid: self.change[0].then_some(self.locks.grid),
            bpm: self.change[1].then_some(self.locks.bpm),
            metadata: self.change[2].then_some(self.locks.metadata),
        };
        if patch.is_empty() {
            Err("Choose at least one lock field".into())
        } else {
            Ok(patch)
        }
    }
}
impl App {
    /// Freeze the selected preparation versions.
    /// Takes selected-row or filtered-crate choice; captures up to 4096 exact current versions without changing them.
    fn capture_preparation_protection(&mut self, filtered: bool) {
        self.refresh_library_view();
        let indices: Vec<_> = if filtered {
            self.library_view.indices.iter().copied().collect()
        } else {
            self.library_view
                .indices
                .get(self.lib_sel)
                .copied()
                .into_iter()
                .collect()
        };
        let panel = &mut self.library_protection;
        panel.targets.clear();
        panel.reviewed = None;
        if indices.is_empty() || indices.len() > 4096 {
            panel.message = "Select 1–4096 saved library rows".into();
            return;
        }
        let captured: Option<Vec<_>> = indices
            .iter()
            .map(|&row| {
                let item = self.library.get(row)?;
                let track = self.library_metadata.catalog.track(&item.source)?;
                (track.versions[track.current].fingerprint == item.fingerprint)
                    .then(|| Target::capture(track))
            })
            .collect();
        let Some(targets) = captured else {
            panel.message =
                "Finish the current library scan/save before reviewing preparation locks".into();
            return;
        };
        panel.locks = targets[0].locks;
        panel.targets = targets;
        panel.change = [false; 3];
        panel.expected = self.library_metadata.catalog.crates.revision();
        panel.message = format!(
            "Captured {} current versions. Unselected lock fields keep each track's own setting.",
            panel.targets.len()
        );
    }
    pub(super) fn library_protection_ui(&mut self, ctx: &egui::Context) {
        if !self.library_protection.open {
            return;
        }
        let mut open = true;
        let mut capture = None;
        let mut action = None;
        let available = self.library_crates.pending.is_none()
            && !self.library_metadata.active()
            && !self.library_closing();
        egui::Window::new(tr!("Preparation locks")).id(egui::Id::new("library-preparation-locks")).open(&mut open).default_width(690.0).vscroll(true).show(ctx,|ui| {
            let panel=&mut self.library_protection;
            ui.label(tr!("Grid locks also block manual grid edits and Undo until unlocked. BPM and metadata locks keep saved values during automatic analysis, scans and tag refresh. Reviewed manual tag changes remain available. Locks and preparation are sidecar data; audio files stay separate."));
            ui.horizontal(|ui| {
                if ui.add_enabled(available,egui::Button::new(tr!("Capture selected preparation"))).help(ui,HelpControl::PreparationLocks).clicked() {capture=Some(false);}
                if ui.add_enabled(available,egui::Button::new(tr!("Capture filtered preparation"))).help(ui,HelpControl::PreparationLocks).clicked() {capture=Some(true);}
            });
            ui.label(&panel.message);
            let previous=(panel.locks,panel.change);
            ui.add_enabled_ui(available && !panel.targets.is_empty(),|ui| {
                ui.horizontal(|ui| {ui.checkbox(&mut panel.change[0],tr!("Change grid lock"));ui.checkbox(&mut panel.locks.grid,tr!("Lock grid")).help(ui,HelpControl::PreparationLocks);});
                ui.horizontal(|ui| {ui.checkbox(&mut panel.change[1],tr!("Change BPM lock"));ui.checkbox(&mut panel.locks.bpm,tr!("Lock BPM")).help(ui,HelpControl::PreparationLocks);});
                ui.horizontal(|ui| {ui.checkbox(&mut panel.change[2],tr!("Change metadata lock"));ui.checkbox(&mut panel.locks.metadata,tr!("Lock metadata")).help(ui,HelpControl::PreparationLocks);});
            });
            if previous!=(panel.locks,panel.change) {panel.reviewed=None;}
            if ui.add_enabled(available && !panel.targets.is_empty(),egui::Button::new(tr!("Review preparation locks"))).help(ui,HelpControl::PreparationLocks).clicked() {
                match panel.patch() {Ok(patch)=>{if let Some(id)=panel.next_review.checked_add(1) {panel.next_review=id;panel.reviewed=Some(patch);} else {panel.message="Preparation review identity exhausted; reopen the app before saving".into();return;}panel.message=format!("Reviewed {} exact current versions. Save rechecks the captured metadata, grid and previous lock settings.",panel.targets.len());},Err(error)=>panel.message=error}
            }
            if let Some(patch)=panel.reviewed {
                for target in panel.targets.iter().take(8) {
                    ui.label(format!("{} · {} → {}",target.metadata.title,target.locks.description(),patch.apply(target.locks).description()));
                }
            }
            if ui.push_id(("save-preparation-review",panel.next_review),|ui|ui.add_enabled(available && panel.reviewed.is_some(),egui::Button::new(tr!("Save reviewed preparation locks"))).help(ui,HelpControl::PreparationLocks)).inner.clicked() {
                action=Some((panel.expected,CollectionAction::Protect {targets:panel.targets.clone(),patch:panel.reviewed.take().unwrap()}));
            }
            if let Some((token,_))=&self.library_crates.pending {
                if ui.button(tr!("Cancel pending preparation edit")).clicked() {token.cancel();}
            }
            ui.label(&self.library_crates.message);
            ui.label(tr!("Replacement bytes start with fresh preparation. The track keeps its lock choices; unlocking never restores unverified markers from old audio."));
        });
        self.library_protection.open = open;
        if !open {
            self.library_protection.reviewed = None;
        }
        if let Some(filtered) = capture {
            self.capture_preparation_protection(filtered);
        }
        if let Some((revision, action)) = action {
            self.submit_crate_edit(revision, action);
        }
    }
}

#[cfg(test)]
mod tests;
