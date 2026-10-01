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

pub(super) fn reconcile(
    store: &mut Store,
    items: &mut Vec<LibItem>,
    captures: Vec<Capture>,
    import: Option<PathBuf>,
) -> Result<Option<String>, String> {
    // Work on a candidate so invalid imports/limits never partly mutate memory.
    let mut catalog = store.catalog.clone();
    let import_error = import.and_then(|path| {
        crate::library::read(&path)
            .and_then(|other| catalog.merge_import(other))
            .err()
    });
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
            capture.metadata,
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
    store.catalog = catalog;
    store.save()?;
    // A scan discovers local media; it never deletes persisted entries or
    // remote namespaces. Missing/unmounted/provider entries remain in the crate.
    *items = store
        .catalog
        .tracks
        .iter()
        .map(|track| LibItem::from_stored(track.source.clone(), &track.versions[track.current]))
        .collect();
    Ok(import_error)
}

impl App {
    pub(super) fn start_library_store(&mut self, path: PathBuf) {
        self.library_metadata = library_metadata::Metadata::new(Some(path));
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
            ui.add(egui::TextEdit::singleline(&mut self.library_import_path).hint_text("/path/to/library.json").desired_width(420.0));
            if ui.button("Import catalog").clicked() {
                if self.library_import_path.trim().is_empty() { self.status = "Choose a catalog path".into(); }
                else if self.library_metadata.import(PathBuf::from(self.library_import_path.trim())) {
                    self.status = "DJ library import queued".into();
                } else { self.status = "DJ library import unavailable or already pending".into(); }
            }
            ui.label(self.library_metadata.label());
            if ui.button("Retry library save").clicked() { self.library_metadata.retry_save(); }
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
                    if ui.button("Retry library save").clicked() {
                        self.library_metadata.retry_save();
                    }
                    let keep = ui.button("Keep working");
                    keep_working = keep.clicked() || keep.is_pointer_button_down_on();
                    discard = ui.button("Close without saving").clicked();
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
