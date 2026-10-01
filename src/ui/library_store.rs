//! GUI bridge to the existing bounded crate-metadata worker. Disk reads,
//! migrations, imports, merging, serialization and fsync stay on that worker.
use super::*;
use crate::engine::preparation::Preparation;
use crate::library::{Metadata, Store};

#[derive(Clone)]
pub(super) struct Capture {
    pub source: LibSource,
    pub fingerprint: Option<FileFingerprint>,
    pub metadata: Metadata,
    pub preparation: Option<Preparation>,
    pub played: Option<SystemTime>,
}
impl LibItem {
    pub(super) fn stored_metadata(&self) -> Metadata {
        Metadata {
            title: self.title.clone(),
            artist: self.artist.clone(),
            bpm: self.bpm,
            key: self.key.clone(),
            duration: self.length,
            last_play: self.last_play,
        }
    }
    fn from_stored(source: LibSource, version: &crate::library::Version) -> Self {
        let m = &version.metadata;
        Self {
            source,
            title: m.title.clone(),
            artist: m.artist.clone(),
            bpm: m.bpm,
            key: m.key.clone(),
            length: m.duration,
            last_play: m.last_play,
            fingerprint: version.fingerprint,
        }
    }
}

pub(super) fn reconcile_optional(
    store: &mut Store,
    items: &mut Vec<LibItem>,
    captures: &[Capture],
    import: Option<&std::path::Path>,
    import_work: Option<&crate::engine::performance::WorkPermit>,
    scan_work: Option<&crate::engine::performance::WorkPermit>,
    fallback: &[LibItem],
) -> Result<Option<String>, String> {
    use std::sync::atomic::Ordering;
    const PROTECTED: &str =
        "Performance protection cancelled the optional catalog import before commit";
    let cancelled =
        |work: &crate::engine::performance::WorkPermit| work.cancel().load(Ordering::Acquire);
    let mut catalog = store.catalog.clone();
    let mut import_error = None;
    let mut imported = false;
    if let Some(path) = import {
        if import_work.is_some_and(&cancelled) {
            import_error = Some(PROTECTED.into());
        } else {
            match crate::library::read(path).and_then(|other| catalog.merge_import(other)) {
                Ok(()) => imported = true,
                Err(error) => import_error = Some(error),
            }
        }
    }
    let use_scan = scan_work.is_some_and(|work| !cancelled(work));
    if scan_work.is_some() && !use_scan {
        *items = fallback.to_vec();
    }
    reconcile_items(&mut catalog, items, captures)?;
    // A single short Studio-only permit orders mode entry against catalog
    // replacement/rename/fsync. A mode request during an already committed save
    // reports Changing; it cannot retroactively label the saved import cancelled.
    let work = if use_scan {
        scan_work
    } else if imported {
        import_work
    } else {
        None
    };
    let commit = work.map(|work| work.commit());
    let import_cancelled = imported && import_work.is_some_and(&cancelled);
    let scan_cancelled = use_scan && scan_work.is_some_and(&cancelled);
    let mut guard = None;
    if import_cancelled || scan_cancelled || commit.as_ref().is_some_and(|commit| commit.is_err()) {
        // Optional candidates never replace essential loaded-media metadata,
        // preparation or play history. Rebuild only that essential transaction.
        catalog = store.catalog.clone();
        if scan_work.is_some() {
            *items = fallback.to_vec();
        }
        reconcile_items(&mut catalog, items, captures)?;
        if imported {
            import_error = Some(PROTECTED.into());
        }
    } else if let Some(Ok(commit)) = commit {
        guard = Some(commit);
    }
    store.catalog = catalog;
    store.save()?;
    *items = store
        .catalog
        .tracks
        .iter()
        .map(|track| LibItem::from_stored(track.source.clone(), &track.versions[track.current]))
        .collect();
    drop(guard);
    Ok(import_error)
}

/// A committed optional transaction can outlive the frame that enables
/// protection. Keep the old visible identities with essential version-qualified
/// updates until a later Studio rebase exposes the committed catalog additions.
pub(super) fn restricted_rows(base: &[LibItem], captures: &[Capture]) -> Vec<LibItem> {
    // Never overlay the full saved catalog here: a completed optional import
    // can change metadata for an already visible identity too. Reuse the same
    // merge rules with only the visible baseline and accepted essential edits.
    let mut catalog = crate::library::Catalog::default();
    if reconcile_items(&mut catalog, base, captures).is_err() {
        return base.to_vec();
    }
    base.iter()
        .map(|item| {
            catalog.version(&item.source, item.fingerprint).map_or_else(
                || item.clone(),
                |version| LibItem::from_stored(item.source.clone(), version),
            )
        })
        .collect()
}

fn reconcile_items(
    catalog: &mut crate::library::Catalog,
    items: &[LibItem],
    captures: &[Capture],
) -> Result<(), String> {
    for item in items.iter() {
        let current = catalog.track(&item.source).map(|t| t.current);
        catalog.upsert(
            item.source.clone(),
            item.fingerprint,
            item.stored_metadata(),
        )?;
        if let (Some(current), LibSource::File(path)) = (current, &item.source) {
            if FileFingerprint::read(path) != item.fingerprint {
                catalog
                    .tracks
                    .iter_mut()
                    .find(|t| t.source == item.source)
                    .unwrap()
                    .current = current;
            }
        }
    }
    for capture in captures {
        // A retired load can update its archived version, never change which
        // bytes are current at a path after a newer scan/load.
        let current = catalog.track(&capture.source).map(|t| t.current);
        let version = catalog.upsert(
            capture.source.clone(),
            capture.fingerprint,
            capture.metadata.clone(),
        )?;
        if let Some(preparation) = capture.preparation.filter(|p| p.valid()) {
            version.preparation = preparation;
        }
        version.metadata.last_play = version.metadata.last_play.max(capture.played);
        if let Some(current) = current.filter(|_| match &capture.source {
            LibSource::File(path) => FileFingerprint::read(path) != capture.fingerprint,
            _ => false,
        }) {
            let track = catalog
                .tracks
                .iter_mut()
                .find(|t| t.source == capture.source)
                .unwrap();
            track.current = current;
        }
    }
    Ok(())
}

impl App {
    pub(super) fn start_library_store(&mut self, path: PathBuf) {
        self.library_metadata = library_metadata::Metadata::new(Some(path));
        self.library_metadata.set_performance(self.engine.cmd.performance().clone());
        self.library_initialized = false;
    }
    pub(super) fn library_receipt(
        &self,
        source: &LibSource,
        fingerprint: Option<FileFingerprint>,
    ) -> Receipt {
        let preparation = if matches!(source, LibSource::File(_)) && fingerprint.is_none() {
            None
        } else {
            self.library_metadata
                .catalog
                .version(source, fingerprint)
                .map(|v| v.preparation)
        };
        Receipt::with_preparation(preparation)
    }
    pub(super) fn restore_initial_library_preparation(&mut self) {
        if self.library_initialized
            || self.library_metadata.storage.is_none()
            || !self.library_metadata.durable
        {
            return;
        }
        let mut admitted = true;
        for (deck, stem) in [BuiltinStem::Drums, BuiltinStem::Harmony]
            .into_iter()
            .enumerate()
        {
            let Some(version) = self
                .library_metadata
                .catalog
                .version(&LibSource::Builtin(stem), None)
            else {
                continue;
            };
            if version.preparation != Preparation::default() {
                admitted &= self.submit(Command::DeckRestorePreparation {
                    deck: deck as u8,
                    receipt: self.engine.initial_playback[deck].clone(),
                    preparation: version.preparation,
                });
            }
        }
        self.library_initialized = admitted;
    }
    pub(super) fn capture_metadata(
        &self,
        source: &LibSource,
        fingerprint: Option<FileFingerprint>,
    ) -> Metadata {
        self.library
            .iter()
            .find(|item| &item.source == source && item.fingerprint == fingerprint)
            .map(LibItem::stored_metadata)
            .or_else(|| {
                self.library_metadata
                    .catalog
                    .version(source, fingerprint)
                    .map(|v| v.metadata.clone())
            })
            .unwrap_or_else(|| Metadata {
                title: match source {
                    LibSource::File(path) => path
                        .file_stem()
                        .and_then(|p| p.to_str())
                        .unwrap_or("track")
                        .into(),
                    _ => "track".into(),
                },
                artist: String::new(),
                bpm: Bpm::UNKNOWN,
                key: String::new(),
                duration: None,
                last_play: None,
            })
    }
    pub(super) fn library_store_ui(&mut self, ctx: &egui::Context) {
        // Publication can arrive while the command port is temporarily full or
        // history preparation owns admission. Keep retrying the receipt-guarded
        // startup restore, without opening creative admission during close.
        if !self.project.committing() {
            self.restore_initial_library_preparation();
        }
        if !self.library_import_open {
            return;
        }
        keyboard::block_for_dialog(ctx);
        let mut open = true;
        egui::Window::new("Import DJ library").open(&mut open).show(ctx, |ui| {
            // A project CloseGuard seals renderer edits; the same close must
            // also exclude new catalog imports after its durability check.
            if self.project.committing() { ui.disable(); }
            ui.label("Import a version 1 or 2 Omatainer catalog JSON. Existing identities and preparation are preserved; conflicting imports are rejected.");
            ui.label("Local files can play. Removable-volume and provider references remain unavailable until a resolver is supported; no network request is made.");
            let path = ui.add(egui::TextEdit::singleline(&mut self.library_import_path).hint_text("/path/to/library.json").desired_width(420.0));
            path.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, "DJ library import path"));
            help::annotate(ui, &path, help::Control::LibraryImportPath);
            let response = ui.button("Import catalog");
            help::annotate(ui, &response, help::Control::LibraryImport);
            if response.clicked() {
                if self.library_import_path.trim().is_empty() { self.status = "Choose a catalog path".into(); }
                else if self.library_metadata.import(PathBuf::from(self.library_import_path.trim())) {
                    self.status = "DJ library import queued".into();
                } else { self.status = "DJ library import unavailable or already pending".into(); }
            }
            ui.label(self.library_metadata.label());
            let response = ui.button("Retry library save");
            help::annotate(ui, &response, help::Control::LibraryRetry);
            if response.clicked() { self.library_metadata.retry_save(); }
        });
        self.library_import_open = open;
    }
}
#[cfg(test)]
mod tests;

#[derive(Default)]
pub(super) struct Close {
    requested: bool,
    fence: Option<Arc<std::sync::atomic::AtomicBool>>,
    allow: bool,
    shown: bool,
}
pub(super) enum CloseState {
    Ready,
    Pending,
    Failed(String),
}
impl App {
    /// GUI-free progress API for the project/library close coordinator. A Ready
    /// result follows a renderer FIFO fence and the durable worker receipt.
    pub(super) fn prepare_library_close(&mut self) -> CloseState {
        use std::sync::atomic::{AtomicBool, Ordering};
        if self.library_metadata.storage.is_none() {
            return CloseState::Ready;
        }
        // A project CloseGuard already proves creative admission is sealed and
        // its prior command queue drained. Reuse it instead of submitting a
        // fence to the deliberately closed admission gate.
        let sealed = self.project_admission_sealed();
        if !sealed && self.library_close.fence.is_none() && self.engine.cmd.is_connected() {
            let acknowledged = Arc::new(AtomicBool::new(false));
            if self
                .engine
                .send(Command::LibraryFence {
                    acknowledged: acknowledged.clone(),
                })
                .is_ok()
            {
                self.library_close.fence = Some(acknowledged);
            }
        }
        let acknowledged = self
            .library_close
            .fence
            .as_ref()
            .is_some_and(|a| a.load(Ordering::Acquire));
        if !sealed && !acknowledged && self.engine.cmd.is_connected() {
            return CloseState::Pending;
        }
        self.poll_play_history();
        self.poll_library_metadata();
        if self.library_metadata.active() {
            CloseState::Pending
        } else if !self.library_metadata.durable {
            CloseState::Failed(self.library_metadata.label().to_owned())
        } else {
            CloseState::Ready
        }
    }
    pub(super) fn cancel_library_close(&mut self) {
        self.library_close = Close::default();
    }
    pub(super) fn request_library_close(&mut self, ctx: &egui::Context) {
        self.library_close.requested = true;
        if self.library_metadata.storage.is_none() {
            self.library_close.allow = true;
            self.allow_project_close(ctx);
        }
    }
    pub(super) fn library_close_ui(&mut self, ctx: &egui::Context) {
        if self.library_close.allow || !self.library_close.requested {
            return;
        }
        keyboard::block_for_dialog(ctx);
        let state = self.prepare_library_close();
        let ready = matches!(state, CloseState::Ready);
        let mut keep_working = false;
        let mut discard = false;
        // A pending dialog's current-frame input wins even if its save has
        // become Ready since the previous frame.
        if !ready || self.library_close.shown {
            self.library_close.shown = true;
            egui::Window::new("Saving DJ library before exit")
                .collapsible(false)
                .show(ctx, |ui| {
                    match &state {
                        CloseState::Failed(error) => {
                            ui.label(error);
                        }
                        CloseState::Ready => {
                            ui.label("DJ library saved.");
                        }
                        CloseState::Pending => {
                            ui.label(
                                "Waiting for earlier deck edits and the background library save…",
                            );
                        }
                    }
                    let retry = ui.button("Retry library save");
                    help::annotate(ui, &retry, help::Control::LibraryRetry);
                    if retry.clicked() { self.library_metadata.retry_save(); }
                    let keep = ui.button("Keep working");
                    help::annotate(ui, &keep, help::Control::LibraryKeepWorking);
                    keep_working = keep.clicked() || keep.is_pointer_button_down_on();
                    let close = ui.button("Close without saving");
                    help::annotate(ui, &close, help::Control::LibraryCloseWithoutSaving);
                    discard = close.clicked();
                });
        }
        if keep_working {
            self.cancel_project_close();
        } else if ready || discard {
            self.library_close.allow = true;
            self.allow_project_close(ctx);
        }
        ctx.request_repaint_after(std::time::Duration::from_millis(16));
    }
}
