//! Prepared sampler drafts: decoding and reusable-file IO stay on their owners.
//! A draft always retains its original project epoch and bank revision target.
use super::*;
use crate::engine::{
    media_load::SamplerToken,
    sampler::{Ack, Audition, Bank, Edit, EditState, Target},
};
use crate::sampler_bank::{
    self,
    manager::Manager,
    prepare::{Assignment, Operation, Request},
    resident::Settings,
    BankId, Factory, Source, SourceRef,
};

pub(super) struct Editor {
    open: bool,
    draft: Option<Draft>,
    slot: usize,
    name: String,
    definition: Option<BankId>,
    overwrite: bool,
    store: Option<Manager>,
    store_path: PathBuf,
    loading: Option<Loading>,
    applying: Option<Pending>,
    awaiting_snapshot: Option<u64>,
    audition: Option<(u64, Ack)>,
    stop_requested: bool,
    next_audition: u64,
    message: String,
    error: Option<String>,
}
struct Draft {
    bank: Bank,
    settings: Settings,
    epoch: u64,
    target: Target,
    prepared: bool,
    applied: bool,
    proofs: Vec<SourceRef>,
    token: Option<SamplerToken>,
    starts: [String; 16],
    ends: [String; 16],
}
struct Loading {
    token: SamplerToken,
    keep_target: Option<(u64, Target)>,
}
struct Pending {
    ack: Ack,
    bank: Bank,
    proofs: Vec<SourceRef>,
}

impl Default for Editor {
    fn default() -> Self {
        Self {
            open: false,
            draft: None,
            slot: 0,
            name: "My bank".into(),
            definition: None,
            overwrite: false,
            store: None,
            store_path: sampler_bank::default_path(),
            loading: None,
            applying: None,
            awaiting_snapshot: None,
            audition: None,
            stop_requested: false,
            next_audition: 1,
            message:
                "Prepare a draft, audition it, then explicitly Apply or save a reusable definition."
                    .into(),
            error: None,
        }
    }
}
impl Draft {
    fn captured(bank: Bank, epoch: u64) -> Self {
        Self {
            target: Target::Replace {
                id: bank.id,
                revision: bank.revision,
            },
            epoch,
            settings: (*bank.data.settings).clone(),
            starts: std::array::from_fn(|i| {
                bank.data.settings.slots[i]
                    .controls
                    .start_seconds
                    .to_string()
            }),
            ends: std::array::from_fn(|i| {
                bank.data.settings.slots[i]
                    .controls
                    .end_seconds
                    .map_or_else(String::new, |v| v.to_string())
            }),
            bank,
            prepared: true,
            applied: false,
            proofs: Vec::new(),
            token: None,
        }
    }
    fn current(&self, snap: &Snapshot) -> bool {
        self.epoch == snap.sampler_epoch
            && match self.target {
                Target::Append { revision } => revision == snap.sampler_revision,
                Target::Replace { id, revision } => snap
                    .sampler_instances
                    .iter()
                    .any(|b| b.id == id && b.revision == revision),
            }
    }
    fn applicable(&self) -> bool {
        self.prepared
            && self.editable()
            && self.token.as_ref().is_some_and(SamplerToken::is_current)
    }
    fn editable(&self) -> bool {
        self.bank.factory.is_none() && !self.applied
    }
}
impl Editor {
    pub(super) fn stop_for_close(&mut self, engine: &Engine) {
        self.stop(engine);
    }
    pub(super) fn blocks_close(&self) -> bool {
        self.busy() || self.stop_requested || self.store.as_ref().is_some_and(|store| store.busy)
    }
    pub(super) fn close_pending(&mut self) {
        self.open = true;
        self.message = "Close postponed while sampler work is pending. Wait for its actual result or Cancel / close this editor, then close the application again. A completed save or renderer Apply remains committed.".into();
    }
    #[cfg(test)]
    pub(super) fn set_store_path(&mut self, path: PathBuf) {
        self.store_path = path;
    }
    #[cfg(test)]
    pub(super) fn evidence(&self) -> serde_json::Value {
        serde_json::json!({"open": self.open, "loading": self.loading.is_some(), "applying": self.applying.is_some(),
            "message": self.message, "error": self.error, "slot": self.slot, "audition": self.audition.as_ref().map(|(id, _)| id),
            "saved": self.store.as_ref().and_then(|store| store.saved.as_ref()).is_some(),
            "durable": self.store.as_ref().and_then(|store| store.saved.as_ref()).is_some_and(|saved| saved.commit.durable),
            "draft": self.draft.as_ref().map(|d| serde_json::json!({"id":d.bank.id,"prepared":d.prepared,"applied":d.applied,
                "settings":d.settings,"frames":d.bank.data.audio.iter().map(|s|s.as_ref().map(|s|s.frames())).collect::<Vec<_>>(),
                "ranges":d.bank.data.ranges,"issues":d.bank.data.issues}))})
    }
    fn busy(&self) -> bool {
        self.loading.is_some() || self.applying.is_some() || self.awaiting_snapshot.is_some()
    }
    fn stop(&mut self, engine: &Engine) -> bool {
        let Some((id, _)) = &self.audition else {
            self.stop_requested = false;
            return true;
        };
        match engine.send(Command::SamplerAuditionStop { id: *id }) {
            Ok(_) => {
                self.audition = None;
                self.stop_requested = false;
                true
            }
            Err(crate::engine::SubmissionError::Disconnected) => {
                // No future dequeue can acknowledge this release. Preserve the
                // unknown outcome visibly, but do not deadlock explicit shutdown.
                self.audition = None;
                self.stop_requested = false;
                self.error = Some("Engine disconnected before audition stop could be confirmed. Its output state is unknown; application shutdown remains available.".into());
                true
            }
            Err(error) => {
                self.stop_requested = true;
                self.error = Some(format!(
                    "Audition stop not accepted: {error}. Retrying the reserved release."
                ));
                false
            }
        }
    }
    fn cancel(&mut self, engine: &Engine) {
        self.stop(engine);
        if let Some(loading) = self.loading.take() {
            loading.token.cancel();
        }
        if let Some(pending) = &self.applying {
            if pending.ack.cancel() {
                self.message = "Apply cancelled before renderer ownership.".into();
            } else {
                self.message =
                    "Apply already belongs to the renderer; waiting for its actual outcome.".into();
            }
        } else {
            self.message =
                "Unapplied draft discarded. Applied banks and completed saves are retained.".into();
        }
        if let Some(store) = &self.store {
            if store.busy {
                store.cancel();
            }
        }
        self.open = false;
        self.awaiting_snapshot = None;
        if let Some(draft) = &self.draft {
            if let Some(token) = &draft.token {
                token.cancel();
            }
        }
        self.draft = None;
    }
    fn start(&mut self, app: &App, operation: Operation, keep_target: bool) {
        if self.busy() {
            return;
        }
        if !self.stop(&app.engine) {
            return;
        }
        let target = keep_target
            .then(|| self.draft.as_ref().map(|d| (d.epoch, d.target)))
            .flatten();
        let request = Request {
            epoch: target.map_or(app.snap.sampler_epoch, |v| v.0),
            revision: app.snap.sampler_revision,
            sample_rate: app.engine.sr(),
            operation,
            catalog: app.library_metadata.catalog.clone(),
        };
        let result = app
            .loader
            .as_ref()
            .ok_or_else(|| "Media decoder is unavailable.".to_string())
            .and_then(|loader| loader.request_sampler(request, app.engine.sampler_assets.clone()));
        match result {
            Ok(token) => {
                self.loading = Some(Loading {
                    token,
                    keep_target: target,
                });
                self.error = None;
                self.message =
                    "Preparing preview on the decoder worker… Current pads are unchanged.".into();
            }
            Err(error) => self.error = Some(error),
        }
    }
    fn change(
        &mut self,
        app: &App,
        assignment: Option<Assignment>,
        retry: Option<u8>,
        clear: Option<u8>,
    ) {
        let Some(draft) = &self.draft else {
            return;
        };
        if !draft.editable() {
            self.error = Some(
                "Copy a factory bank or reopen the current applied bank before editing.".into(),
            );
            return;
        }
        let mut settings = draft.settings.clone();
        for i in 0..16 {
            settings.slots[i].controls.start_seconds = draft.starts[i].parse().unwrap_or(f64::NAN);
            settings.slots[i].controls.end_seconds = if draft.ends[i].trim().is_empty() {
                None
            } else {
                Some(draft.ends[i].parse().unwrap_or(f64::NAN))
            };
        }
        self.start(
            app,
            Operation::Change {
                bank: draft.bank.clone(),
                settings,
                assignment,
                retry,
                clear,
            },
            true,
        );
    }
    fn apply(&mut self, app: &App) {
        if self.busy() || !self.stop(&app.engine) {
            return;
        }
        let Some(draft) = &self.draft else {
            return;
        };
        if !draft.applicable() || !draft.current(&app.snap) {
            return;
        }
        let ack = draft.token.as_ref().unwrap().ack.clone();
        let command = Command::SamplerEdit(Edit {
            epoch: draft.epoch,
            target: draft.target,
            bank: draft.bank.clone(),
            select: true,
            ack: ack.clone(),
        });
        match app.engine.send(command) {
            Ok(_) => {
                self.applying = Some(Pending {
                    ack,
                    bank: draft.bank.clone(),
                    proofs: draft.proofs.clone(),
                });
                self.error = None;
                self.message = "Apply queued; waiting for the renderer.".into();
            }
            Err(error) => self.error = Some(format!("Apply not accepted: {error}")),
        }
    }
    fn audition(&mut self, app: &App) {
        if !self.stop(&app.engine) {
            return;
        }
        let Some(draft) = &self.draft else {
            return;
        };
        if !draft.prepared || draft.bank.data.audio[self.slot].is_none() {
            return;
        }
        let Some(next) = self.next_audition.checked_add(1) else {
            self.error = Some("Audition identities exhausted; reopen the application.".into());
            return;
        };
        let id = self.next_audition;
        self.next_audition = next;
        let ack = Ack::new();
        match app.engine.send(Command::SamplerAudition(Audition {
            id,
            bank: draft.bank.data.clone(),
            slot: self.slot as u8,
            track: app.snap.selected_track as u8,
            ack: ack.clone(),
        })) {
            Ok(_) => {
                self.audition = Some((id, ack));
                self.error = None;
            }
            Err(error) => self.error = Some(format!("Audition not accepted: {error}")),
        }
    }
}

impl App {
    pub(super) fn open_sampler_editor(&mut self) {
        let mut editor = std::mem::take(&mut self.sampler_editor);
        if !editor.open && !editor.busy() {
            editor.draft = self
                .snap
                .sampler_instances
                .get(self.snap.sampler_bank)
                .cloned()
                .map(|bank| Draft::captured(bank, self.snap.sampler_epoch));
            editor.name = editor
                .draft
                .as_ref()
                .map_or_else(|| "My bank".into(), |d| format!("{} copy", d.bank.name()));
        }
        editor.open = true;
        if editor.store.is_none() {
            match Manager::start(
                editor.store_path.clone(),
                self.engine.cmd.performance().clone(),
            ) {
                Ok(store) => editor.store = Some(store),
                Err(error) => {
                    editor.error = Some(format!("Reusable sampler store unavailable: {error}"))
                }
            }
        }
        self.sampler_editor = editor;
    }
    pub(super) fn poll_sampler_editor(&mut self) {
        let mut editor = std::mem::take(&mut self.sampler_editor);
        if editor.stop_requested {
            editor.stop(&self.engine);
        }
        if let Some(store) = &mut editor.store {
            if store.poll() {
                if let Some(saved) = &store.saved {
                    editor.definition = Some(saved.definition);
                    editor.message = if saved.commit.durable {
                        "Reusable bank saved. This does not Apply a working bank or save the project.".into()
                    } else {
                        format!(
                            "Reusable bank committed; durability warning: {}",
                            saved
                                .commit
                                .warning
                                .as_deref()
                                .unwrap_or("directory sync unconfirmed")
                        )
                    };
                }
            }
        }
        if let Some(completion) = self.loader.as_ref().and_then(Loader::take_sampler_ready) {
            let selected = editor
                .loading
                .as_ref()
                .is_some_and(|p| p.token.id == completion.token.id);
            if selected {
                let pending = editor.loading.take().unwrap();
                match completion.work.commit().map_err(|e| e.to_string()).and_then(|_guard| {
                    if !completion.token.is_current() { return Err("Preview cancelled; current bank retained.".into()); }
                    let mut prepared = completion.result?;
                    if let Some((epoch, target)) = pending.keep_target { prepared.epoch = epoch; prepared.target = target; }
                    let mut proofs = if pending.keep_target.is_some() { editor.draft.as_ref().map(|d| d.proofs.clone()).unwrap_or_default() } else { Vec::new() };
                    proofs.retain(|proof| prepared.bank.data.settings.slots.iter().any(|slot| matches!(&slot.source, Some(Source::Library { reference }) if reference == proof)));
                    for proof in prepared.verified_sources { if !proofs.contains(&proof) { proofs.push(proof); } }
                    debug_assert!(proofs.len() <= 16);
                    let mut draft = Draft::captured(prepared.bank, prepared.epoch);
                    draft.proofs = proofs;
                    draft.token = Some(completion.token.clone());
                    draft.target = prepared.target;
                    editor.draft = Some(draft);
                    Ok(())
                }) {
                    Ok(()) => { editor.error = None; editor.message = "Preview ready. Audition uses this draft; pads change only after Apply.".into(); }
                    Err(error) => editor.error = Some(error),
                }
            }
        }
        if let Some(pending) = &editor.applying {
            match pending.ack.state() {
                EditState::Pending => {
                    if !self.engine.cmd.is_connected() {
                        editor.error = Some(
                            "Engine disconnected before confirming Apply; outcome unknown.".into(),
                        );
                        editor.applying = None;
                    }
                }
                EditState::Applied => {
                    // Acquire the request outcome first, then wait for a snapshot
                    // at least as new as the renderer revision observed here.
                    // This also terminates after a later Undo/project replacement;
                    // it does not assume this bank is still current.
                    editor.awaiting_snapshot = Some(self.engine.project.revision());
                    for proof in &pending.proofs {
                        if let Err(error) = self.library_metadata.qualify_sampler(proof.clone()) {
                            self.status = format!(
                                "Sampler applied; source qualification was not accepted: {error}"
                            );
                            editor.error = Some(self.status.clone());
                        }
                    }
                    if let Some(draft) = &mut editor.draft {
                        if draft.bank.id == pending.bank.id {
                            draft.applied = true;
                            draft.proofs.clear();
                        }
                    }
                    editor.message = "Bank applied by renderer. Use Edit current bank for another revision. Save reusable and project Save are separate.".into();
                    editor.applying = None;
                }
                EditState::Rejected => {
                    editor.error = Some("Renderer rejected or cancelled this bank edit. The original target was not redirected; reopen the current bank to retry.".into());
                    editor.applying = None;
                }
            }
        }
        if editor
            .awaiting_snapshot
            .is_some_and(|revision| self.snap.project_revision >= revision)
        {
            editor.awaiting_snapshot = None;
        }
        if let Some((_, ack)) = &editor.audition {
            if ack.state() == EditState::Rejected {
                editor.error = Some("Renderer rejected the audition.".into());
                editor.stop(&self.engine);
            } else if ack.ended() {
                editor.stop(&self.engine);
            }
        }
        self.sampler_editor = editor;
    }
    pub(super) fn sampler_editor_ui(&mut self, ctx: &egui::Context) {
        if !self.sampler_editor.open {
            return;
        }
        let mut editor = std::mem::take(&mut self.sampler_editor);
        keyboard::block_for_dialog(ctx);
        let mut close = ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Escape));
        let mut open = true;
        let available = ctx.screen_rect().shrink(8.0);
        let mut action = None;
        egui::Window::new("Sampler bank editor").id(egui::Id::new("sampler-bank-editor"))
            .open(&mut open).collapsible(false).default_width(630.0).min_width(250.0_f32.min(available.width()))
            .max_width(available.width()).max_height(available.height()).constrain_to(available)
            .show(ctx, |ui| accessibility::scope(ui, "Sampler editor", |ui| {
                ui.push_id("sampler-status", |ui| {
                    ui.label(&editor.message);
                    if editor.awaiting_snapshot.is_some() { ui.label("Renderer Applied; waiting for the current bank snapshot before another edit."); }
                    if let Some(error) = &editor.error { ui.colored_label(self.theme.red, error); }
                });
                let allowed = !editor.busy() && !self.project.committing() && self.project.dialog_is_closed();
                let scroll = egui::ScrollArea::both().id_salt("sampler-editor-body")
                    .max_height((available.height() - 155.0).max(60.0)).show(ui, |ui| {
                    ui.label("Working project banks (factory originals are read-only)");
                    ui.horizontal_wrapped(|ui| {
                        for bank in &self.snap.sampler_instances {
                            let id = bank.id.to_string();
                            ui.push_id(bank.id, |ui| {
                                if button(ui, &format!("{} · {}", bank.name(), &id[..8]), &format!("Edit working bank {} ({id})", bank.name()), HelpControl::SamplerEdit, allowed).clicked() {
                                    action = Some(Action::Select(bank.clone()));
                                }
                            });
                        }
                    });
                    if button(ui, "Edit current bank", "Edit current bank", HelpControl::SamplerEdit, allowed).clicked() {
                        if let Some(bank) = self.snap.sampler_instances.get(self.snap.sampler_bank) { action = Some(Action::Select(bank.clone())); }
                    }
                    text(ui, &mut editor.name, "New or reusable bank name", HelpControl::SamplerName, 128);
                    ui.horizontal_wrapped(|ui| {
                        if button(ui, "Create empty", "Create empty bank", HelpControl::SamplerCreate, allowed).clicked() { action = Some(Action::Create); }
                        if button(ui, "Copy draft", "Copy draft bank", HelpControl::SamplerCopy, allowed && editor.draft.as_ref().is_some_and(|d| d.prepared)).clicked() { action = Some(Action::Copy); }
                    });
                    ui.horizontal_wrapped(|ui| {
                        for bank in Factory::ALL {
                            if button(ui, &format!("Original {}", bank.name()), &format!("Copy original {}", bank.name()), HelpControl::SamplerCopy, allowed).clicked() { action = Some(Action::Factory(bank)); }
                        }
                    });
                    ui.push_id("sampler-draft", |ui| {
                    if let Some(draft) = &mut editor.draft {
                        ui.separator();
                        ui.push_id("draft-target-status", |ui| {
                        ui.label(format!("Draft: {} · {}", draft.bank.name(), if draft.bank.factory.is_some() { "factory: copy to edit" } else { "captured working identity" }));
                        if !draft.current(&self.snap) && !draft.applied { ui.colored_label(self.theme.yellow, "Target changed since this draft began. Apply is disabled; reopen the current bank."); }
                        });
                        let editable = allowed && draft.editable();
                        ui.add_enabled_ui(editable, |ui| {
                            if text(ui, &mut draft.settings.name, "Working bank name", HelpControl::SamplerName, 128).changed() { draft.prepared = false; }
                        });
                        ui.horizontal_wrapped(|ui| {
                            for slot in 0..16 { if button(ui, &format!("{}{}", slot + 1, if draft.bank.data.issues[slot].is_some() { " !" } else { "" }),
                                &format!("Slot {}", slot + 1), HelpControl::SamplerSlot, allowed).clicked() { action = Some(Action::Slot(slot)); } }
                        });
                        let slot = editor.slot;
                        ui.push_id("slot-source-status", |ui| {
                        ui.label(format!("Slot {} source: {}", slot + 1, source_label(&draft.bank, slot)));
                        if let Some(issue) = &draft.bank.data.issues[slot] { ui.colored_label(self.theme.red, format!("Missing/unavailable: {issue}. No replacement sound is substituted.")); }
                        });
                        let picked = self.selected_library_item();
                        ui.label(format!("Selected crate source: {}", picked.map_or("none", |item| item.title.as_str())));
                        ui.horizontal_wrapped(|ui| {
                            let local = picked.is_some_and(|item| matches!(item.source, LibSource::File(_)) && item.fingerprint.is_some());
                            if button(ui, "Assign selected local source", "Assign selected local source", HelpControl::SamplerAssign, editable && local).clicked() { action = Some(Action::Assign); }
                            if button(ui, "Clear slot", "Clear selected slot", HelpControl::SamplerClear, editable).clicked() { action = Some(Action::Clear); }
                            if button(ui, "Retry missing source", "Retry selected source", HelpControl::SamplerRetry, editable && draft.settings.slots[slot].source.is_some()).clicked() { action = Some(Action::Retry); }
                        });
                        ui.add_enabled_ui(editable, |ui| {
                            let controls = &mut draft.settings.slots[slot].controls;
                            let gain = ui.add(egui::Slider::new(&mut controls.gain, 0.0..=2.0).text("Slot gain"));
                            if let Some(value) = accessibility::numeric(ui, &gain, "Slot gain", controls.gain, 0.0, 2.0, 0.01, "linear") { controls.gain = value; draft.prepared = false; }
                            accessibility::focus(ui, &gain); help::annotate(ui, &gain, HelpControl::SamplerGain);
                            if gain.changed() { draft.prepared = false; }
                            if text(ui, &mut draft.starts[slot], "Start seconds", HelpControl::SamplerTrim, 32).changed() { draft.prepared = false; }
                            if text(ui, &mut draft.ends[slot], "End seconds (empty means source end)", HelpControl::SamplerTrim, 32).changed() { draft.prepared = false; }
                        });
                        ui.push_id("slot-waveform", |ui| waveform(ui, &draft.bank, slot, &self.theme));
                        ui.push_id("preview-status", |ui| {
                        if draft.prepared && !draft.applied && draft.token.as_ref().is_some_and(|t| !t.is_current()) { ui.label("This preview request was invalidated. Prepare again before Apply; the installed bank is unchanged."); }
                        if !draft.prepared { ui.label("Values changed: prepare the preview before audition, Apply or reusable Save."); }
                        });
                        ui.horizontal_wrapped(|ui| {
                            if button(ui, "Prepare preview", "Prepare slot preview", HelpControl::SamplerPreview, editable).clicked() { action = Some(Action::Prepare); }
                            if button(ui, "Audition", "Audition selected slot", HelpControl::SamplerAudition, allowed && draft.prepared && draft.bank.data.audio[slot].is_some()).clicked() { action = Some(Action::Audition); }
                            if button(ui, "Stop audition", "Stop audition", HelpControl::SamplerAuditionStop, editor.audition.is_some()).clicked() { action = Some(Action::Stop); }
                        });
                    }
                    });
                    ui.push_id("audition-status", |ui| {
                    if let Some((id, ack)) = &editor.audition { ui.label(match ack.state() {
                        EditState::Pending => "Audition queued; waiting for renderer.", EditState::Rejected => "Audition rejected.",
                        EditState::Applied if self.snap.sampler_audition == Some(*id) => "Audition is active on the captured track.",
                        EditState::Applied => "Audition was accepted; awaiting current playback observation.",
                    }); }
                    });
                    ui.separator();
                    ui.label("Reusable definitions reference local files. Project Save embeds playable audio.");
                    if let Some(store) = &editor.store {
                        ui.push_id("store-status", |ui| {
                        if store.busy { ui.label("Reusable store operation pending…"); }
                        if let Some(error) = &store.error { ui.colored_label(self.theme.red, error); }
                        });
                        ui.push_id("store-definitions", |ui| {
                        if let Some(collection) = &store.collection {
                            for definition in &collection.banks {
                                ui.push_id(definition.id, |ui| {
                                    let id = definition.id.to_string();
                                    let r = ui.add_enabled(!store.busy, egui::Button::new(format!("{} · {}", definition.name, &id[..8])).selected(editor.definition == Some(definition.id)));
                                    accessibility::button(ui, &r, &format!("Reusable definition {} ({id})", definition.name), Some(editor.definition == Some(definition.id)));
                                    help::annotate(ui, &r, HelpControl::SamplerDefinition);
                                    if r.clicked() { editor.definition = Some(definition.id); editor.overwrite = false; }
                                });
                            }
                        }
                        });
                        ui.horizontal_wrapped(|ui| {
                            if button(ui, "Import selected definition", "Import reusable definition", HelpControl::SamplerImport, allowed && !store.busy && editor.definition.is_some()).clicked() { action = Some(Action::Import); }
                            if button(ui, "Retry store", "Retry reusable store", HelpControl::SamplerRetry, !store.busy).clicked() { action = Some(Action::StoreRetry); }
                        });
                        let overwrite = ui.add_enabled(!store.busy && editor.definition.is_some(), egui::Checkbox::new(&mut editor.overwrite, "Replace selected reusable definition"));
                        help::annotate(ui, &overwrite, HelpControl::SamplerOverwrite);
                        if button(ui, "Save reusable", "Save reusable bank", HelpControl::SamplerSave,
                            allowed && !store.busy && store.collection.is_some() && editor.draft.as_ref().is_some_and(|d| d.prepared)).clicked() { action = Some(Action::Save); }
                    }
                });
                accessibility::scrollbars(ui, "Sampler editor", &scroll);
                ui.horizontal_wrapped(|ui| {
                    if button(ui, "Apply bank", "Apply bank", HelpControl::SamplerApply, allowed && editor.draft.as_ref().is_some_and(|d| d.applicable() && d.current(&self.snap))).clicked() { action = Some(Action::Apply); }
                    if button(ui, "Cancel / close", "Cancel or close sampler editor", HelpControl::SamplerCancel, true).clicked() { close = true; }
                });
            }));
        if !open || close {
            editor.cancel(&self.engine);
        } else if let Some(action) = action {
            match action {
                Action::Select(bank) => {
                    if editor.stop(&self.engine) {
                        editor.draft = Some(Draft::captured(bank, self.snap.sampler_epoch));
                        editor.error = None;
                    }
                }
                Action::Slot(slot) => {
                    if editor.stop(&self.engine) {
                        editor.slot = slot;
                    }
                }
                Action::Factory(bank) => editor.start(
                    self,
                    Operation::Factory {
                        bank,
                        name: editor.name.clone(),
                    },
                    false,
                ),
                Action::Create => editor.start(
                    self,
                    Operation::Empty {
                        name: editor.name.clone(),
                    },
                    false,
                ),
                Action::Copy => {
                    if let Some(draft) = &editor.draft {
                        editor.start(
                            self,
                            Operation::Copy {
                                bank: draft.bank.clone(),
                                name: editor.name.clone(),
                            },
                            false,
                        );
                    }
                }
                Action::Prepare => editor.change(self, None, None, None),
                Action::Clear => {
                    let slot = editor.slot as u8;
                    editor.change(self, None, None, Some(slot));
                }
                Action::Retry => {
                    let slot = editor.slot as u8;
                    editor.change(self, None, Some(slot), None);
                }
                Action::Assign => {
                    let assignment = self.selected_library_item().and_then(|item| {
                        item.fingerprint.map(|fingerprint| Assignment {
                            slot: editor.slot as u8,
                            source: item.source.clone(),
                            fingerprint,
                        })
                    });
                    if let Some(assignment) = assignment {
                        editor.change(self, Some(assignment), None, None);
                    }
                }
                Action::Apply => editor.apply(self),
                Action::Audition => editor.audition(self),
                Action::Stop => {
                    editor.stop(&self.engine);
                }
                Action::Import => {
                    if let Some(definition) = editor
                        .store
                        .as_ref()
                        .and_then(|s| s.collection.as_ref())
                        .and_then(|c| c.banks.iter().find(|d| Some(d.id) == editor.definition))
                        .cloned()
                    {
                        editor.start(self, Operation::Definition(definition), false);
                    }
                }
                Action::Save => {
                    if let (Some(store), Some(draft)) = (&mut editor.store, &editor.draft) {
                        if let Err(error) = store.save(
                            draft.bank.clone(),
                            editor.name.clone(),
                            editor.overwrite.then_some(editor.definition).flatten(),
                        ) {
                            editor.error = Some(error);
                        }
                    }
                }
                Action::StoreRetry => {
                    if let Some(store) = &mut editor.store {
                        if let Err(error) = store.retry() {
                            editor.error = Some(error);
                        }
                    }
                }
            }
        }
        if editor.busy() || editor.stop_requested || editor.store.as_ref().is_some_and(|s| s.busy) {
            ctx.request_repaint_after(std::time::Duration::from_millis(30));
        }
        self.sampler_editor = editor;
    }
}

enum Action {
    Select(Bank),
    Slot(usize),
    Factory(Factory),
    Create,
    Copy,
    Prepare,
    Clear,
    Retry,
    Assign,
    Apply,
    Audition,
    Stop,
    Import,
    Save,
    StoreRetry,
}
fn button(
    ui: &mut Ui,
    text: &str,
    name: &str,
    control: HelpControl,
    enabled: bool,
) -> egui::Response {
    let response = ui.add_enabled(enabled, egui::Button::new(text));
    accessibility::button(ui, &response, name, None);
    help::annotate(ui, &response, control);
    response
}
fn text(
    ui: &mut Ui,
    value: &mut String,
    label: &str,
    control: HelpControl,
    limit: usize,
) -> egui::Response {
    ui.label(label);
    let response = ui.add(
        egui::TextEdit::singleline(value)
            .id_salt(label)
            .char_limit(limit)
            .desired_width(260.0),
    );
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::TextEdit,
            response.enabled(),
            format!("Sampler editor: {label}"),
        )
    });
    accessibility::focus(ui, &response);
    help::annotate(ui, &response, control);
    response
}
fn source_label(bank: &Bank, slot: usize) -> String {
    match &bank.data.settings.slots[slot].source {
        Some(Source::Library { reference }) => reference
            .path()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|e| e),
        Some(Source::Factory { bank, slot }) => {
            format!("Original {} / slot {}", bank.name(), slot + 1)
        }
        None if bank.data.audio[slot].is_some() => {
            "Project-embedded audio (assign a local source to save reusable)".into()
        }
        None => "Empty".into(),
    }
}
fn waveform(ui: &mut Ui, bank: &Bank, slot: usize, theme: &Theme) {
    ui.label("Sampled PCM waveform: left above / right below (narrow peaks may be missed)");
    let (rect, response) = ui.allocate_exact_size(
        Vec2::new(ui.available_width().clamp(160.0, 570.0), 82.0),
        Sense::hover(),
    );
    help::annotate(ui, &response, HelpControl::SamplerWaveform);
    ui.painter().rect_filled(rect, 2.0, theme.bg);
    let Some(sample) = &bank.data.audio[slot] else {
        return;
    };
    let frames = sample.frames();
    if frames == 0 {
        return;
    }
    for channel in 0..2 {
        let center = rect.top() + if channel == 0 { 20.0 } else { 61.0 };
        let mut previous = None;
        for i in 0..256 {
            let frame = i * (frames - 1) / 255;
            let value = sample.data
                [frame * sample.ch as usize + channel.min(sample.ch as usize - 1)]
            .clamp(-1.0, 1.0);
            let point = Pos2::new(
                egui::lerp(rect.x_range(), i as f32 / 255.0),
                center - value * 18.0,
            );
            if let Some(previous) = previous {
                ui.painter()
                    .line_segment([previous, point], Stroke::new(1.0_f32, theme.accent));
            }
            previous = Some(point);
        }
    }
    if let Some((start, end)) = bank.data.ranges[slot] {
        for value in [start, end] {
            let x = egui::lerp(rect.x_range(), (value / frames as f64) as f32);
            ui.painter()
                .vline(x, rect.y_range(), Stroke::new(2.0_f32, theme.yellow));
        }
    }
    ui.label(format!(
        "{} frames · {} Hz · {} channels · {:.6} s",
        frames,
        sample.sr,
        sample.ch,
        frames as f64 / sample.sr as f64
    ));
}

#[cfg(test)]
mod tests;
