//! Explicit, immutable match review; filesystem and persistence remain workers.
use super::*;
use crate::library::relocation_search;
use std::sync::atomic::Ordering;

#[derive(Default)]
pub(super) struct Search {
    roots: String,
    handle: Option<library_scan::SearchHandle>,
    receipt: Option<Arc<relocation_search::Receipt>>,
    selected: Option<usize>,
    proof_queued: bool,
    finished: bool,
}

impl Search {
    pub fn discard(&mut self) {
        self.handle = None;
        self.receipt = None;
        self.selected = None;
        self.proof_queued = false;
        self.finished = false;
    }
}

pub(super) fn panel(app: &mut App, relocation: &mut Relocation, ui: &mut egui::Ui) {
    let search = &mut relocation.search;
    if let Some(handle) = &search.handle {
        if !search.finished {
            if let Some(result) = app.library_scan.replacement_result(handle.id) {
                search.finished = true;
                match result {
                    Ok(receipt) if !handle.cancelled() => {
                        relocation.message = format!("Found {} byte-identical replacements. Select a location to review; no association has changed.", receipt.matches.len());
                        search.receipt = Some(receipt);
                    }
                    Ok(_) => {
                        relocation.message =
                            "Search cancelled by Performance protection; search again in Studio"
                                .into()
                    }
                    Err(error) => relocation.message = error,
                }
            }
        }
    }
    ui.separator();
    ui.label("Or search replacement folders (one absolute folder per line)");
    let busy = search.handle.is_some() && !search.finished;
    let editable = !relocation.pending && !relocation.saved && !busy;
    let roots = ui.add_enabled(
        editable,
        egui::TextEdit::multiline(&mut search.roots)
            .char_limit(64 * 4096)
            .desired_rows(2)
            .desired_width(420.0),
    );
    roots.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::TextEdit,
            editable,
            "Replacement search folders",
        )
    });
    help::annotate(ui, &roots, HelpControl::RelocateRoots);
    if roots.changed() {
        search.discard();
    }
    let start = ui
        .push_id((&relocation.request.id, &search.roots), |ui| {
            ui.add_enabled(
                editable && !app.library_scan.active() && !app.library_metadata.active(),
                egui::Button::new("Search replacement folders"),
            )
        })
        .inner;
    help::annotate(ui, &start, HelpControl::RelocateSearch);
    if start.clicked() {
        let paths = search
            .roots
            .lines()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .take(65)
            .map(PathBuf::from)
            .collect();
        match app.library_scan.search_replacement(
            app.library_metadata.catalog.clone(),
            relocation.request.clone(),
            paths,
        ) {
            Ok(handle) => {
                search.handle = Some(handle);
                search.receipt = None;
                search.selected = None;
                search.proof_queued = false;
                search.finished = false;
                relocation.message = "Searching and comparing complete file content…".into();
            }
            Err(error) => relocation.message = error,
        }
    }
    let Some(handle) = &search.handle else {
        return;
    };
    if !search.finished {
        let p = &handle.progress;
        let phase = match p.stage.load(Ordering::Acquire) {
            0 => "Verifying original",
            1 => "Searching",
            _ => "Checking discovered locations",
        };
        ui.label(format!(
            "{phase}: {} entries · {} files · {} bytes hashed · {} skipped",
            p.entries.load(Ordering::Relaxed),
            p.files.load(Ordering::Relaxed),
            p.bytes.load(Ordering::Relaxed),
            p.skipped.load(Ordering::Relaxed)
        ));
        if ui
            .button("Cancel replacement search")
            .help(ui, HelpControl::RelocateCancel)
            .clicked()
        {
            handle.cancel();
        }
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(50));
        return;
    }
    if handle.cancelled() {
        ui.label("This search was cancelled. Search again before choosing a replacement.");
        return;
    }
    let Some(receipt) = &search.receipt else {
        return;
    };
    ui.label(format!(
        "{} byte-identical matches · {} files inspected · {} bytes hashed",
        receipt.matches.len(),
        receipt.files,
        receipt.bytes
    ));
    if !receipt.complete {
        ui.colored_label(Color32::YELLOW, "Incomplete search: other matches may exist in skipped folders or beyond search limits.");
    }
    if !receipt.samples.is_empty() {
        ui.collapsing("Skipped search entries", |ui| {
            for reason in [
                relocation_search::Reason::Unavailable,
                relocation_search::Reason::Unreadable,
                relocation_search::Reason::Symlink,
                relocation_search::Reason::ForeignMount,
                relocation_search::Reason::Invalid,
                relocation_search::Reason::Changed,
                relocation_search::Reason::Limit,
            ] {
                let count = receipt.skipped[reason as usize];
                if count > 0 {
                    ui.label(format!("{count} {}", reason.label()));
                }
            }
            for sample in &receipt.samples {
                ui.label(format!(
                    "{}: {} — {}",
                    sample.reason.label(),
                    sample.path,
                    sample.detail
                ));
            }
        });
    }
    let current = app
        .library_metadata
        .catalog
        .track(&receipt.target.source)
        .filter(|t| {
            t.id == receipt.target.id
                && t.versions[t.current].fingerprint == Some(receipt.target.fingerprint)
        });
    let qualified =
        current.is_some_and(|t| t.versions[t.current].content_hash == Some(receipt.original_hash));
    if current.is_none() {
        ui.label("The original library version changed. Search again.");
        return;
    }
    if !qualified && !search.proof_queued {
        match app
            .library_metadata
            .qualify_measured_content(crate::sampler_bank::SourceRef {
                track: receipt.target.id.clone(),
                source: receipt.target.source.clone(),
                fingerprint: receipt.target.fingerprint,
                content_hash: Some(receipt.original_hash),
            }) {
            Ok(()) => search.proof_queued = true,
            Err(error) => {
                ui.label(error);
            }
        }
    }
    if !qualified || app.library_metadata.active() || !app.library_metadata.durable {
        ui.label("Waiting for verified source identity and current library edits to be saved…");
    }
    let selectable = !relocation.pending && !relocation.saved;
    ui.push_id((&receipt.target.id, handle.id), |ui| {
        egui::ScrollArea::vertical().id_salt("replacement-matches").max_height(180.0).show_rows(ui, ui.text_style_height(&egui::TextStyle::Body) + 6.0, receipt.matches.len(), |ui, rows| {
            for index in rows {
                let candidate = &receipt.matches[index];
                let row = ui.add_enabled(selectable, egui::Button::selectable(search.selected == Some(index), candidate.location.path.display().to_string()));
                help::annotate(ui, &row, HelpControl::RelocateChoice);
                if row.clicked() { search.selected = Some(index); }
            }
        });
        if let Some(index) = search.selected {
            let candidate = &receipt.matches[index];
            ui.label(format!("Selected replacement: {}", candidate.location.path.display()));
            if receipt.matches.len() > 1 { ui.label("Several copies contain identical bytes. Only the selected location will be associated with this track."); }
            let ready = selectable && qualified && app.library_metadata.durable && !app.library_metadata.active();
            let commit = ui.push_id(index, |ui| ui.add_enabled(ready, egui::Button::new("Verify and use selected replacement"))).inner;
            help::annotate(ui, &commit, HelpControl::RelocateUse);
            if commit.clicked() {
                relocation.request.destination = candidate.location.path.clone();
                if app.library_metadata.relocate_reviewed(relocation.request.clone(), candidate.clone()) {
                    relocation.pending = true;
                    relocation.message = "Rechecking the reviewed file and saving its association…".into();
                } else { relocation.message = format!("Replacement was not queued. {}", app.library_metadata.label()); }
            }
        } else if !receipt.matches.is_empty() { ui.label("Select a replacement to enable verification and save."); }
    });
}
