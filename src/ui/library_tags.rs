//! Reviewed tag edits. The GUI freezes identities and changed fields; its two
//! existing workers own all media I/O, catalog persistence and large retirement.
use super::*;
use crate::library::tags::Patch;
use crate::media_tags::write::{Record, Recovery};
use library_scan::tag_jobs::{Handle, Reply, Task};

#[derive(Default)]
struct Field {
    change: bool,
    value: String,
}
struct Preview {
    reply: Arc<Reply>,
    handle: Handle,
}
struct Queue {
    patch: Patch,
    embedded: bool,
    cursor: usize,
    saved: usize,
    skipped: usize,
    unconfirmed: usize,
    cancelled: bool,
}
enum Pending {
    Inspect(Handle),
    Apply(Handle),
    Save(Handle),
    Recover(Handle),
    Cleanup(Handle),
}
impl Pending {
    fn handle(&self) -> &Handle {
        match self {
            Self::Inspect(h)
            | Self::Apply(h)
            | Self::Save(h)
            | Self::Recover(h)
            | Self::Cleanup(h) => h,
        }
    }
}
struct RecoveryQueue {
    reply: Arc<Reply>,
    cursor: usize,
    work: Arc<crate::engine::performance::WorkPermit>,
}
pub(super) struct Panel {
    pub open: bool,
    fields: [Field; 4],
    embedded: bool,
    preview: Option<Preview>,
    reviewed: Option<Patch>,
    queue: Option<Queue>,
    pending: Option<Pending>,
    cleanup: Option<(Record, bool, Arc<crate::engine::performance::WorkPermit>)>,
    recovery: Option<RecoveryQueue>,
    recovery_checked: bool,
    message: String,
    outcomes: Vec<String>,
    generation: u64,
    detail_row: usize,
}
impl Default for Panel {
    fn default() -> Self {
        Self {
            open: false,
            fields: std::array::from_fn(|_| Field::default()),
            embedded: true,
            preview: None,
            reviewed: None,
            queue: None,
            pending: None,
            cleanup: None,
            recovery: None,
            recovery_checked: false,
            message: String::new(),
            outcomes: Vec::new(),
            generation: 0,
            detail_row: 0,
        }
    }
}
impl Panel {
    fn busy(&self) -> bool {
        self.pending.is_some()
            || self.queue.is_some()
            || self.recovery.is_some()
            || self.cleanup.is_some()
    }
    fn patch(&self) -> Patch {
        let values: Vec<_> = self
            .fields
            .iter()
            .map(|field| field.change.then(|| field.value.clone()))
            .collect();
        Patch {
            title: values[0].clone(),
            artist: values[1].clone(),
            bpm: values[2].clone(),
            key: values[3].clone(),
        }
    }
    fn note(&mut self, mut text: String) {
        if text.len() > 2048 {
            let mut end = 2048;
            while !text.is_char_boundary(end) {
                end -= 1;
            }
            text.truncate(end);
            text.push_str("…");
        }
        // Per-row results are bounded independently of total library size.
        if self.outcomes.len() < 32 {
            self.outcomes.push(text.clone());
        }
        self.message = text;
    }
    fn cancel(&mut self) {
        if let Some(queue) = &mut self.queue {
            queue.cancelled = true;
        }
        if let Some(preview) = &self.preview {
            preview.handle.cancel();
        }
        if let Some(pending) = &self.pending {
            if !matches!(pending, Pending::Cleanup(_)) {
                pending.handle().cancel();
            }
        }
        self.message = "Cancellation requested. Already written tags still finish their library save; earlier saved tracks remain saved.".into();
    }
}

impl App {
    pub(super) fn stop_tags_for_close(&mut self) -> bool {
        if self.library_tags.busy() {
            self.library_tags.cancel();
        }
        self.poll_library_tags();
        !self.library_tags.busy()
    }
    fn inspect_tags(&mut self, batch: bool) {
        if self.library_tags.busy()
            || self.library_scan.active()
            || self.library_metadata.active()
            || !self.library_metadata.ready()
            || self.project.committing()
            || self.library_closing()
        {
            return;
        }
        self.refresh_library_view();
        let begin = if batch { 0 } else { self.lib_sel };
        let end = if batch {
            self.library_view.indices.len()
        } else {
            begin.saturating_add(1)
        };
        if begin >= end
            || end > self.library_view.indices.len()
            || end - begin > library_scan::tag_jobs::MAX_REVIEW
        {
            self.library_tags.message =
                "Select 1–4,096 rows; narrow the crate filter for a larger library.".into();
            return;
        }
        if !batch {
            if let Some(item) = self.selected_library_item() {
                let values = [
                    item.title.clone(),
                    item.artist.clone(),
                    item.bpm.value().map_or_else(String::new, |v| v.to_string()),
                    item.key.clone(),
                ];
                self.library_tags.fields = values.map(|value| Field {
                    change: false,
                    value,
                });
            }
        } else {
            self.library_tags.fields = std::array::from_fn(|_| Field::default());
        }
        self.library_tags.preview = None;
        self.library_tags.detail_row = 0;
        self.library_tags.reviewed = None;
        self.library_tags.outcomes.clear();
        self.library_tags.generation = self.library_tags.generation.saturating_add(1);
        let task = Task::Inspect {
            catalog: self.library_metadata.catalog.clone(),
            rows: self.library.clone(),
            indices: self.library_view.indices.clone(),
            begin,
            end,
        };
        match self.library_scan.tag_task(task, None) {
            Ok(handle) => {
                self.library_tags.pending = Some(Pending::Inspect(handle));
                self.library_tags.message = "Reading captured tracks' actual embedded tags…".into();
            }
            Err(error) => self.library_tags.message = error,
        }
    }
    fn commit_reviewed_tags(&mut self) {
        if self.library_tags.busy()
            || self.project.committing()
            || self.library_closing()
            || self.library_tags.generation == u64::MAX
        {
            return;
        }
        let Some(preview) = &self.library_tags.preview else {
            return;
        };
        if preview.handle.cancelled() {
            self.library_tags.message =
                "Review expired after cancellation or Performance protection; inspect again."
                    .into();
            return;
        }
        let Some(patch) = self.library_tags.reviewed.clone() else {
            return;
        };
        if patch.validate().is_err() || patch.is_empty() || !self.library_metadata.reserve_tags() {
            return;
        }
        self.library_tags.queue = Some(Queue {
            patch,
            embedded: self.library_tags.embedded,
            cursor: 0,
            saved: 0,
            skipped: 0,
            unconfirmed: 0,
            cancelled: false,
        });
        self.library_tags.outcomes.clear();
        self.library_tags.message =
            "Applying the reviewed fields to the captured tracks, one at a time…".into();
    }
    fn finish_tag_queue(&mut self) {
        if let Some(queue) = self.library_tags.queue.take() {
            let last = self.library_tags.message.clone();
            self.library_tags.message = format!("{}: {} saved, {} skipped unchanged, {} failed or awaiting recovery. Earlier saved tracks remain saved.{}",
                if queue.cancelled { "Tag batch cancelled" } else { "Tag batch finished" }, queue.saved, queue.skipped, queue.unconfirmed,
                if queue.unconfirmed == 0 { String::new() } else { format!(" Latest result: {last}") });
        }
        self.library_tags.reviewed = None;
        self.library_tags.preview = None;
        self.library_metadata.release_tags();
    }
    pub(super) fn poll_library_tags(&mut self) {
        if self.project.committing() || self.library_closing() {
            if let Some(queue) = &mut self.library_tags.queue {
                queue.cancelled = true;
            }
        }
        // Project close still needs the filesystem result and sole-writer save.
        if self.project.committing() && self.library_tags.pending.is_some() {
            if let Some(publication) = self.library_scan.poll() {
                publication.discard();
            }
            self.poll_library_metadata();
        }
        if let Some(Pending::Save(handle)) = &self.library_tags.pending {
            let id = handle.id;
            let work = handle.work.clone();
            if let Some(receipt) = self.library_metadata.take_tag_result() {
                self.library_tags.pending = None;
                if receipt.id != id {
                    self.library_tags.note(
                        "Tag save receipt does not match the pending operation; recovery required."
                            .into(),
                    );
                } else {
                    match receipt.outcome {
                        Ok(message) => {
                            if let Some(queue) = &mut self.library_tags.queue {
                                queue.saved += 1;
                            }
                            self.library_tags.note(message);
                        }
                        Err(error) => {
                            if let Some(queue) = &mut self.library_tags.queue {
                                queue.unconfirmed += 1;
                                queue.cancelled = true;
                            }
                            self.library_tags.note(error);
                        }
                    }
                    if let Some(record) = receipt.cleanup {
                        self.library_tags.cleanup = Some((record, false, work));
                    }
                }
            }
        }
        let result = self
            .library_tags
            .pending
            .as_ref()
            .filter(|pending| !matches!(pending, Pending::Save(_)))
            .and_then(|pending| self.library_scan.tag_result(pending.handle().id));
        if let Some(reply) = result {
            let pending = self.library_tags.pending.take().unwrap();
            let was_recovery = matches!(pending, Pending::Recover(_));
            let handle = match pending {
                Pending::Inspect(handle) => {
                    match reply.as_ref() {
                        Reply::Preview(preview) if !handle.cancelled() => {
                            self.library_tags.message = format!("{} captured rows inspected. Choose changed fields, then review the edits.", preview.rows.len());
                            self.library_tags.preview = Some(Preview { reply, handle });
                        }
                        Reply::Failed { message, .. } => self.library_tags.note(message.clone()),
                        _ => self
                            .library_tags
                            .note("Tag inspection cancelled; no edits were made.".into()),
                    }
                    return;
                }
                Pending::Apply(handle) | Pending::Recover(handle) | Pending::Cleanup(handle) => {
                    handle
                }
                Pending::Save(_) => unreachable!(),
            };
            match reply.as_ref() {
                Reply::Applied(_) | Reply::Sidecar { .. } => {
                    let save = library_metadata::tags::Save {
                        id: handle.id,
                        result: reply.clone(),
                        recovery_index: None,
                        work: handle.work.clone(),
                    };
                    match self.library_metadata.save_tags(save) {
                        Ok(()) => self.library_tags.pending = Some(Pending::Save(handle)),
                        Err(_) => {
                            self.library_tags.note("Catalog owner unavailable. Any installed media remains journaled for recovery.".into());
                            if let Some(queue) = &mut self.library_tags.queue {
                                queue.cancelled = true;
                            }
                        }
                    }
                }
                Reply::Recover(_) => {
                    self.library_tags.recovery = Some(RecoveryQueue {
                        reply,
                        cursor: 0,
                        work: handle.work,
                    });
                    self.library_tags.recovery_checked = true;
                }
                Reply::Cleaned => {}
                Reply::Failed { message, record } => {
                    self.library_tags.note(message.clone());
                    if let Some(queue) = &mut self.library_tags.queue {
                        queue.unconfirmed += 1;
                        if record.is_some() || handle.cancelled() {
                            queue.cancelled = true;
                        }
                    }
                    if record.is_some() {
                        self.library_tags.recovery_checked = false;
                    }
                    if was_recovery {
                        self.library_metadata.release_tags();
                        self.library_tags.recovery_checked = true;
                    }
                    if matches!(self.library_scan.state, library_scan::ScanState::Failed(_)) {
                        self.library_tags.recovery_checked = false;
                        if let Some(queue) = &mut self.library_tags.queue {
                            queue.cancelled = true;
                        }
                    }
                }
                _ => self
                    .library_tags
                    .note("Unexpected filesystem tag reply; no catalog edit was submitted.".into()),
            }
        }
        if self.library_tags.pending.is_some() {
            return;
        }
        if self.library_scan.active() {
            return;
        }
        if let Some((record, unchanged, work)) = self.library_tags.cleanup.take() {
            match self.library_scan.tag_task(
                Task::Cleanup {
                    record: record.clone(),
                    unchanged,
                },
                Some(work.clone()),
            ) {
                Ok(handle) => self.library_tags.pending = Some(Pending::Cleanup(handle)),
                Err(error) => {
                    self.library_tags.cleanup = Some((record, unchanged, work));
                    self.library_tags.message = error;
                }
            }
            return;
        }
        if self.library_tags.recovery.is_some() {
            self.advance_tag_recovery();
            return;
        }
        if let Some(queue) = &self.library_tags.queue {
            let Some(preview) = &self.library_tags.preview else {
                self.finish_tag_queue();
                return;
            };
            let Reply::Preview(rows) = preview.reply.as_ref() else {
                self.finish_tag_queue();
                return;
            };
            if queue.cancelled || preview.handle.cancelled() || queue.cursor >= rows.rows.len() {
                if preview.handle.cancelled() {
                    self.library_tags.queue.as_mut().unwrap().cancelled = true;
                }
                self.finish_tag_queue();
                return;
            }
            let row = &rows.rows[queue.cursor];
            let target = row.target.clone();
            let work = preview.handle.work.clone();
            let patch = queue.patch.clone();
            let embedded = queue.embedded;
            self.library_tags.queue.as_mut().unwrap().cursor += 1;
            let Some(target) = target else {
                self.library_tags.queue.as_mut().unwrap().skipped += 1;
                self.library_tags.note(format!(
                    "{}: skipped; no current local media identity",
                    row.metadata.title
                ));
                return;
            };
            let current = self
                .library_metadata
                .catalog
                .track(&target.source)
                .is_some_and(|track| {
                    track.id == target.id
                        && track.versions[track.current].fingerprint == Some(target.fingerprint)
                });
            if !current {
                self.library_tags.queue.as_mut().unwrap().skipped += 1;
                self.library_tags
                    .note("Track changed since review; skipped without editing media.".into());
                return;
            }
            let Some(root) = self.library_metadata.tag_recovery_root.clone() else {
                self.finish_tag_queue();
                return;
            };
            match self.library_scan.tag_task(
                Task::Apply {
                    target,
                    patch,
                    root,
                    embedded,
                },
                Some(work),
            ) {
                Ok(handle) => self.library_tags.pending = Some(Pending::Apply(handle)),
                Err(error) => {
                    self.library_tags.queue.as_mut().unwrap().cursor -= 1;
                    self.library_tags.message = error;
                }
            }
            return;
        }
        // Startup/retry recovery is admitted only after the catalog lock is held.
        if !self.library_tags.recovery_checked
            && self.library_metadata.ready()
            && !self.library_metadata.active()
            && !self.engine.cmd.performance().protected()
            && !self.project.committing()
            && !self.library_closing()
        {
            if let Some(root) = self.library_metadata.tag_recovery_root.clone() {
                if self.library_metadata.reserve_tags() {
                    match self.library_scan.tag_task(Task::Recover { root }, None) {
                        Ok(handle) => self.library_tags.pending = Some(Pending::Recover(handle)),
                        Err(error) => {
                            self.library_metadata.release_tags();
                            self.library_tags.message = error;
                        }
                    }
                }
            } else {
                self.library_tags.recovery_checked = true;
            }
        }
    }
    fn advance_tag_recovery(&mut self) {
        let recovery = self.library_tags.recovery.as_mut().unwrap();
        let Reply::Recover(records) = recovery.reply.as_ref() else {
            unreachable!()
        };
        if recovery.cursor >= records.len() {
            self.library_tags.recovery = None;
            self.library_metadata.release_tags();
            return;
        }
        let index = recovery.cursor;
        recovery.cursor += 1;
        match &records[index] {
            Recovery::Applied(_) => {
                let id = self
                    .library_tags
                    .generation
                    .checked_add(1)
                    .unwrap_or(u64::MAX);
                self.library_tags.generation = id;
                let save = library_metadata::tags::Save {
                    id,
                    result: recovery.reply.clone(),
                    recovery_index: Some(index),
                    work: recovery.work.clone(),
                };
                let handle = Handle {
                    id,
                    work: recovery.work.clone(),
                };
                if self.library_metadata.save_tags(save).is_ok() {
                    self.library_tags.pending = Some(Pending::Save(handle));
                } else {
                    recovery.cursor -= 1;
                }
            }
            Recovery::Unchanged(record) => {
                self.library_tags.cleanup = Some((record.clone(), true, recovery.work.clone()))
            }
            Recovery::Conflict { message, .. } => {
                let message = message.clone();
                self.library_tags
                    .note(format!("Tag recovery needs attention: {message}"));
            }
        }
    }
    pub(super) fn library_tags_ui(&mut self, ctx: &egui::Context) {
        if !self.library_tags.open {
            return;
        }
        keyboard::block_for_dialog(ctx);
        let mut open = true;
        let mut inspect = None;
        let mut commit = false;
        egui::Window::new("Audio metadata").open(&mut open).default_width(680.0).show(ctx, |ui| {
            let busy = self.library_tags.busy();
            let available = self.library_tags.generation != u64::MAX && !busy && !self.project.committing() && !self.library_closing() && !self.engine.cmd.performance().protected();
            ui.label("Read actual embedded tags. Missing fields use filename hints; user sidecars take precedence. Changes do not retune loaded audio or replace your beat grid.");
            ui.horizontal(|ui| {
                if ui.add_enabled(available, egui::Button::new("Inspect selected track")).help(ui, help::Control::TagInspect).clicked() { inspect = Some(false); }
                if ui.add_enabled(available, egui::Button::new("Inspect filtered crate (batch)")).help(ui, help::Control::TagInspect).clicked() { inspect = Some(true); }
                if ui.add_enabled(busy, egui::Button::new("Cancel tag work")).help(ui, help::Control::TagCancel).clicked() { self.library_tags.cancel(); }
            });
            ui.add_enabled_ui(available, |ui| {
                for (index, name) in ["Title", "Artist", "BPM", "Key"].into_iter().enumerate() {
                    ui.horizontal(|ui| {
                        let field = &mut self.library_tags.fields[index];
                        let check = ui.checkbox(&mut field.change, format!("Change {name}")).help(ui, help::Control::TagField);
                        let input = ui.add_enabled(field.change, egui::TextEdit::singleline(&mut field.value).id_salt(("tag-field", index)).char_limit(4096).desired_width(380.0));
                        help::annotate(ui, &input, help::Control::TagField);
                        input.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, format!("New {name}")));
                        if check.changed() || input.changed() { self.library_tags.reviewed = None; self.library_tags.generation = self.library_tags.generation.saturating_add(1); }
                    });
                }
                ui.label("Unchecked = keep existing value. Checked and empty = clear that field on every captured track.");
                if ui.checkbox(&mut self.library_tags.embedded, "Write supported embedded tags; use a library sidecar when writing is unavailable").help(ui, help::Control::TagStorage).changed() { self.library_tags.reviewed = None; }
                ui.small("Embedded rewrites support MP3, FLAC, WAV and AIFF up to 128 MiB, after audio and metadata preservation checks. Read-only, larger, unsupported or imperfect files use sidecars. Fractional BPM that a tag cannot store precisely stays in the sidecar.");
            });
            if let Some(preview) = &self.library_tags.preview {
                if let Reply::Preview(preview) = preview.reply.as_ref() {
                    ui.label(format!("Captured targets: {} (browsing or filtering now will not change this list)", preview.rows.len()));
                    let output = egui::ScrollArea::vertical().id_salt("tag-target-review").max_height(220.0).show_rows(ui, 84.0, preview.rows.len(), |ui, range| {
                        for index in range {
                            let row = &preview.rows[index];
                            ui.push_id(("tag-review-row", index), |ui| {
                                if ui.selectable_label(self.library_tags.detail_row == index, format!("{} · saved: {} — {}", index + 1, row.metadata.artist, row.metadata.title)).clicked() { self.library_tags.detail_row = index; }
                                if let Some(target) = &row.target { ui.add(egui::Label::new(format!("{:?}", target.source)).truncate()).on_hover_text(format!("{:?}", target.source)); }
                                match &row.observation {
                                    Ok(observation) => {
                                        let source = |field: &Option<crate::media_tags::Field>| field.as_ref().map_or("filename fallback", |field| field.source.label());
                                        ui.small(format!("Title: {} · artist: {} · BPM: {} · key: {}", source(&observation.fields.title), source(&observation.fields.artist), source(&observation.fields.bpm), source(&observation.fields.key)));
                                        ui.small(if observation.supports_write() { "Embedded write will be checked; sidecar fallback enabled" } else { "Sidecar only: embedded rewrite unavailable" });
                                    }
                                    Err(error) => { ui.small(error); }
                                }
                            });
                        }
                    });
                    accessibility::scrollbars(ui, "Tag review targets", &output);
                    if let Some(row) = preview.rows.get(self.library_tags.detail_row) {
                        ui.label(format!("Actual embedded values for captured row {}", self.library_tags.detail_row + 1));
                        match &row.observation {
                            Ok(observation) => {
                                for (name, field) in [("Title", &observation.fields.title), ("Artist", &observation.fields.artist), ("BPM", &observation.fields.bpm), ("Key", &observation.fields.key)] {
                                    let value = field.as_ref().map_or_else(|| "[missing; filename fallback when no user sidecar]".into(), |field| format!("{} ({})", field.value, field.source.label()));
                                    ui.add(egui::Label::new(format!("{name}: {value}")).truncate()).on_hover_text(value);
                                }
                                for notice in &observation.notices { ui.add(egui::Label::new(notice).truncate()).on_hover_text(notice); }
                            }
                            Err(error) => { ui.label(error); }
                        }
                        if let Some(target) = &row.target {
                            if let Some(tags) = self.library_metadata.catalog.version(&target.source, Some(target.fingerprint)).and_then(|version| version.tags.as_ref()) {
                                ui.small(format!("Effective saved values: {}", tags.describe()));
                            }
                        }
                    }

                    let patch = self.library_tags.patch();
                    if let Err(error) = patch.validate() { ui.label(error); }
                    if ui.add_enabled(available && !patch.is_empty() && patch.validate().is_ok(), egui::Button::new("Review these field changes")).help(ui, help::Control::TagReview).clicked() {
                        self.library_tags.reviewed = Some(patch);
                        self.library_tags.generation = self.library_tags.generation.saturating_add(1);
                    }
                }
            }
            if let Some(patch) = &self.library_tags.reviewed {
                ui.separator();
                for (name, value) in [("Title", &patch.title), ("Artist", &patch.artist), ("BPM", &patch.bpm), ("Key", &patch.key)] {
                    if let Some(value) = value { ui.label(format!("{name} → {}", if value.is_empty() { "[clear]" } else { value })); }
                }
                ui.label("Each track is saved separately. Cancelling leaves earlier completed tracks saved.");
                ui.push_id(("tag-commit", self.library_tags.generation), |ui| {
                    commit = ui.add_enabled(available, egui::Button::new("Apply reviewed edits to captured tracks")).help(ui, help::Control::TagApply).clicked();
                });
            }
            ui.separator();
            ui.label(&self.library_tags.message);
            if !self.library_tags.outcomes.is_empty() {
                ui.small("First 32 per-track results; the summary includes the full captured batch.");
                let output = egui::ScrollArea::vertical().id_salt("tag-outcomes").max_height(110.0).show_rows(ui, 22.0, self.library_tags.outcomes.len(), |ui, rows| {
                    for index in rows { ui.label(&self.library_tags.outcomes[index]); }
                });
                accessibility::scrollbars(ui, "Tag save outcomes", &output);
            }
            if ui.add_enabled(!busy, egui::Button::new("Retry tag recovery")).help(ui, help::Control::TagRecovery).clicked() { self.library_tags.recovery_checked = false; self.library_metadata.retry_save(); }
        });
        self.library_tags.open = open;
        if !open {
            self.library_tags.cancel();
            self.library_tags.preview = None;
            self.library_tags.reviewed = None;
        }
        if let Some(batch) = inspect {
            self.inspect_tags(batch);
        }
        if commit {
            self.commit_reviewed_tags();
        }
        if self.library_tags.busy() {
            ctx.request_repaint_after(std::time::Duration::from_millis(30));
        }
    }
}

#[cfg(test)]
mod tests;
