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
    pub(super) fn from_stored(source: LibSource, version: &crate::library::Version) -> Self {
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

pub(super) struct Reconciled {
    pub notice: Option<String>,
    pub qualification_pending: bool,
    pub relocation: Option<Result<(), String>>,
}

pub(super) fn reconcile_optional(
    store: &mut Store,
    items: &mut Vec<LibItem>,
    captures: &[Capture],
    proofs:&[crate::sampler_bank::SourceRef],
    import: Option<&std::path::Path>,
    import_work: Option<&crate::engine::performance::WorkPermit>,
    scan_work: Option<&crate::engine::performance::WorkPermit>,
    roots: Option<&library_scan::ScanRoots>,
    fallback: &[LibItem],
    relocation: Option<(&crate::library::Relocate, &crate::engine::performance::WorkPermit)>,
    qualification_work: Option<&crate::engine::performance::WorkPermit>,
    qualification_sources: &[(LibSource, Option<FileFingerprint>)],
) -> Result<Reconciled, String> {
    const PROTECTED: &str =
        "Performance protection cancelled the optional catalog import before commit";
    let cancelled =
        |work: &crate::engine::performance::WorkPermit| work.cancelled();
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
    let mut use_scan = scan_work.is_some_and(|work| !cancelled(work));
    let mut roots_error=None;
    if use_scan {
        if let Some(roots)=roots {
            let enrollment=(|| {
                if let Some(book)=&roots.book {if !catalog.watched_roots.matches(book) {catalog.watched_roots.apply(book)?;}}
                catalog.adopt_volumes(&roots.adoptions)?;
                Ok::<(),String>(())
            })();
            if let Err(error)=enrollment {
                roots_error=Some(format!("watched-root scan rejected: {error}"));use_scan=false;
                catalog=store.catalog.clone();
            }
        }
    }
    if scan_work.is_some() && !use_scan {
        *items = fallback.to_vec();
    }
    reconcile_items(&mut catalog, items, captures)?;
    let mut proof_error=qualify_proofs(&mut catalog,proofs);
    let needs_qualification = !qualification_sources.is_empty();
    let mut qualification_pending = needs_qualification && qualification_work.is_none();
    if let Some(work) = qualification_work.filter(|_| needs_qualification) {
        for (source, fingerprint) in qualification_sources {
            if cancelled(work) { break; }
            catalog.qualify_cues_cancellable(source, *fingerprint, &work.cancel());
        }
        qualification_pending = cancelled(work);
    }
    let mut relocated = false;
    let mut relocation_error = None;
    if let Some((request, work)) = relocation {
        match catalog.relocate_cancellable(request, &work.cancel()) {
            Ok(()) => relocated = true,
            Err(error) => relocation_error = Some(format!("relocation rejected: {error}")),
        }
    }
    // A single short Studio-only permit orders mode entry against catalog
    // replacement/rename/fsync. A mode request during an already committed save
    // reports Changing; it cannot retroactively label the saved import cancelled.
    let work = if use_scan {
        scan_work
    } else if imported {
        import_work
    } else if relocated {
        relocation.map(|(_, work)| work)
    } else if needs_qualification {
        qualification_work
    } else {
        None
    };
    let commit = work.map(|work| work.commit());
    let import_cancelled = imported && import_work.is_some_and(&cancelled);
    let scan_cancelled = use_scan && scan_work.is_some_and(&cancelled);
    let relocation_cancelled = relocated && relocation.is_some_and(|(_, work)| cancelled(work));
    let qualification_cancelled = needs_qualification && qualification_work.is_some_and(&cancelled);
    let mut guard = None;
    if import_cancelled || scan_cancelled || relocation_cancelled || qualification_cancelled || commit.as_ref().is_some_and(|commit| commit.is_err()) {
        // Optional candidates never replace essential loaded-media metadata,
        // preparation or play history. Rebuild only that essential transaction.
        catalog = store.catalog.clone();
        if scan_work.is_some() {
            *items = fallback.to_vec();
        }
        reconcile_items(&mut catalog, items, captures)?;
        proof_error=qualify_proofs(&mut catalog,proofs);
        if imported { import_error = Some(PROTECTED.into()); }
        if relocated { relocation_error = Some("relocation rejected: Performance protection cancelled verification before commit".into()); }
        qualification_pending |= needs_qualification;
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
    let relocation = relocation.map(|_| match &relocation_error {
        Some(error) => Err(error.clone()),
        None if relocated => Ok(()),
        None => Err("relocation rejected: verification did not complete".into()),
    });
    Ok(Reconciled {
        notice: roots_error.or(proof_error).or(relocation_error).or_else(|| import_error.map(|error| format!("import rejected: {error}"))),
        qualification_pending,
        relocation,
    })
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

fn qualify_proofs(catalog:&mut crate::library::Catalog,proofs:&[crate::sampler_bank::SourceRef])->Option<String> {
    let mut error=None;
    for proof in proofs {
        let result=proof.content_hash.ok_or("measured content proof has no digest".to_string()).and_then(|hash|catalog.qualify_verified_content(&proof.track,&proof.source,proof.fingerprint,hash));
        if let Err(detail)=result {error.get_or_insert(format!("content proof rejected: {detail}"));}
    }
    error
}
fn reconcile_items(
    catalog: &mut crate::library::Catalog,
    items: &[LibItem],
    captures: &[Capture],
) -> Result<(), String> {
    let changed=|source:&LibSource,fp:Option<FileFingerprint>|catalog.track(source).is_some_and(|t|t.versions[t.current].fingerprint!=fp);
    let needs_mounts=items.iter().any(|i|matches!(i.source,LibSource::Removable {..}) && changed(&i.source,i.fingerprint))
        || captures.iter().any(|c|matches!(c.source,LibSource::Removable {..}) && changed(&c.source,c.fingerprint));
    let mounts=needs_mounts.then(crate::media_location::Snapshot::discover).and_then(Result::ok);
    let present=|source:&LibSource,fp:Option<FileFingerprint>|match source {
        LibSource::File(path)=>FileFingerprint::read(path)==fp,
        LibSource::Removable {..}=>mounts.as_ref().and_then(|s|s.resolve(source).ok().and_then(|l|s.inspect(&l).ok()))
            .map(|m|FileFingerprint::from_metadata(&m))==fp,
        _=>true,
    };
    for item in items.iter() {
        let current = catalog.track(&item.source).map(|t| (t.current,t.versions[t.current].fingerprint));
        catalog.upsert(
            item.source.clone(),
            item.fingerprint,
            item.stored_metadata(),
        )?;
        if let Some((current,old))=current {
            if old!=item.fingerprint && !present(&item.source,item.fingerprint) {
                catalog.restore_current(&item.source,current);
            }
        }
    }
    for capture in captures {
        // A retired load can update its archived version, never change which
        // bytes are current at a path after a newer scan/load.
        let current = catalog.track(&capture.source).map(|t| (t.current,t.versions[t.current].fingerprint));
        let version = catalog.upsert(
            capture.source.clone(),
            capture.fingerprint,
            capture.metadata.clone(),
        )?;
        let _ = version;
        catalog.update_preparation(&capture.source, capture.fingerprint, capture.preparation, capture.played);
        if let Some((current,_))=current.filter(|(_,old)|*old!=capture.fingerprint && !present(&capture.source,capture.fingerprint)) {
            catalog.restore_current(&capture.source,current);
        }
    }
    Ok(())
}

impl App {
    fn import_media_paths(&mut self) {
        if !self.library_metadata.ready() || self.library_metadata.active() {self.status="Wait for the current catalog save before importing music".into();return;}
        let paths: Vec<PathBuf> = self.library_media_paths.lines().filter(|line| !line.is_empty()).map(PathBuf::from).collect();
        if paths.is_empty() || paths.len() > 64 || paths.iter().any(|p| !p.is_absolute() || p.as_os_str().len() > 4096) {
            self.status = "Enter 1–64 absolute file or folder paths, one per line (at most 4096 bytes each)".into();
        } else if self.library_scan.import_with_catalog(paths, self.library.clone(),self.library_metadata.catalog.clone()) {
            self.status = "Music import queued; current decks keep playing".into();
        } else { self.status = self.library_scan.label(); }
    }
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
        let preparation = if matches!(source, LibSource::File(_) | LibSource::Removable {..}) && fingerprint.is_none() {
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
            ui.label("Import music files or folders. Enter one absolute path per line; existing library tracks and playing decks are preserved.");
            let paths = ui.add(egui::TextEdit::multiline(&mut self.library_media_paths).id_salt("music-import-paths").char_limit(65_536).desired_rows(3).desired_width(420.0));
            paths.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, "Music file and folder paths"));
            help::annotate(ui, &paths, help::Control::MusicImportPaths);
            if paths.changed() { self.library_media_revision = self.library_media_revision.checked_add(1).unwrap_or(u64::MAX); }
            let busy = self.library_scan.active();
            ui.push_id(("music-import", self.library_media_revision), |ui| {
                let response = ui.add_enabled(!busy && !self.library_metadata.active() && self.library_metadata.ready() && self.library_media_revision != u64::MAX, egui::Button::new("Import music files/folders"));
                help::annotate(ui, &response, help::Control::MusicImport);
                if response.clicked() { self.import_media_paths(); }
            });
            if busy && ui.button("Cancel music import/scan").clicked() { self.library_scan.cancel(); }
            if ui.button("Manage music folders in Preferences").help(ui, help::Control::MusicRoots).clicked() { self.settings.open = true; }
            ui.label(self.library_scan.label());
            if let Some(summary) = &self.library_scan.summary {
                if !summary.samples.is_empty() {
                    ui.label(format!("First {} skipped entries of {}:", summary.samples.len(), summary.skipped_count()));
                    let scroll = egui::ScrollArea::vertical().id_salt("music-import-skipped").max_height(140.0).show(ui, |ui| {
                        for sample in &summary.samples { ui.label(format!("{}: {} — {}", sample.reason.label(), sample.path, sample.detail)); }
                    });
                    accessibility::scrollbars(ui, "Music import skipped entries", &scroll);
                }
            }
            ui.separator();
            ui.label("Import an Omatainer catalog JSON. Existing identities and preparation are preserved; conflicting imports are rejected.");
            ui.label("Local files and uniquely identified mounted removable libraries can play. Offline, ambiguous or changed volumes fail explicitly. Provider references remain unavailable locally.");
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
#[cfg(test)]
mod import_tests;

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
    pub(super) fn library_closing(&self) -> bool { self.library_close.requested || self.library_close.allow }
    pub(super) fn prepare_library_close(&mut self) -> CloseState {
        self.library_metadata.set_collections_closing(true);
        if !self.stop_analysis_for_close() { return CloseState::Pending; }
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
        self.library_metadata.set_collections_closing(false);
        self.library_close = Close::default();
    }
    pub(super) fn request_library_close(&mut self, ctx: &egui::Context) {
        self.library_metadata.set_collections_closing(true);
        self.library_close.requested = true;
        self.stop_analysis_for_close();
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
