use super::*;
use crate::engine::RtEngine;
use egui::accesskit::{Action as AtAction, ActionData, ActionRequest, Node, NodeId};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

struct Files(PathBuf);
impl Files {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "omat-sampler-ui-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        use std::os::unix::fs::DirBuilderExt;
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&root)
            .unwrap();
        Self(root)
    }
    fn wave(&self) -> PathBuf {
        let path = self.0.join("sample.wav");
        let frames = 24_000u32;
        let channels = 2u16;
        let sr = 48_000u32;
        let size = frames * channels as u32 * 2;
        let mut bytes = b"RIFF".to_vec();
        bytes.extend((36 + size).to_le_bytes());
        bytes.extend(b"WAVEfmt ");
        bytes.extend(16u32.to_le_bytes());
        bytes.extend(1u16.to_le_bytes());
        bytes.extend(channels.to_le_bytes());
        bytes.extend(sr.to_le_bytes());
        bytes.extend((sr * channels as u32 * 2).to_le_bytes());
        bytes.extend((channels * 2).to_le_bytes());
        bytes.extend(16u16.to_le_bytes());
        bytes.extend(b"data");
        bytes.extend(size.to_le_bytes());
        for i in 0..frames {
            for scale in [0.5, 0.25] {
                bytes.extend(
                    (((i as f64 * 440.0 * std::f64::consts::TAU / sr as f64).sin()
                        * scale
                        * i16::MAX as f64)
                        .round() as i16)
                        .to_le_bytes(),
                );
            }
        }
        std::fs::write(&path, bytes).unwrap();
        path
    }
}
impl Drop for Files {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
struct Gui {
    app: App,
    rt: RtEngine,
    ctx: egui::Context,
    time: f64,
    nodes: Vec<(NodeId, Node)>,
    render: bool,
    size: Vec2,
}
impl Gui {
    fn new(files: &Files) -> Self {
        let (engine, mut rt) = Engine::headless_for_test(48_000, 256);
        rt.publish_for_test();
        let loader = Loader::start_with_performance(engine.cmd.performance().clone()).unwrap();
        let mut app = App::with_loader(engine, Theme::default(), Some(loader));
        app.sampler_editor.store_path = files.0.join("banks.json");
        app.start_library_store(files.0.join("catalog.json"));
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut value = Self {
            app,
            rt,
            ctx,
            time: 0.0,
            nodes: vec![],
            render: true,
            size: Vec2::new(1600.0, 1600.0),
        };
        value.wait(|g| !g.app.library_metadata.active());
        value.frame(vec![]);
        value
    }
    fn frame(&mut self, events: Vec<egui::Event>) -> egui::FullOutput {
        self.time += 0.02;
        let modifiers = events
            .iter()
            .rev()
            .find_map(|e| {
                if let egui::Event::Key { modifiers, .. } = e {
                    Some(*modifiers)
                } else {
                    None
                }
            })
            .unwrap_or_default();
        let result = self.ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, self.size)),
                time: Some(self.time),
                focused: true,
                events,
                modifiers,
                ..Default::default()
            },
            |ctx| self.app.update_frame(ctx),
        );
        self.nodes = result
            .platform_output
            .accesskit_update
            .as_ref()
            .unwrap()
            .nodes
            .clone();
        if self.render {
            self.rt.process(&mut [0.0; 128]);
            self.rt.publish_for_test();
        }
        result
    }
    fn wait(&mut self, mut done: impl FnMut(&Self) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            self.frame(vec![]);
            if done(self) {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "timeout: {} {:?}; store={:?}",
                self.app.sampler_editor.message,
                self.app.sampler_editor.error,
                self.app.sampler_editor.store.as_ref().map(|s| (
                    &s.error,
                    s.busy,
                    s.saved.is_some()
                ))
            );
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    fn node(&self, name: &str) -> (NodeId, &Node) {
        self.nodes
            .iter()
            .find(|(_, n)| n.label() == Some(name))
            .map(|(id, n)| (*id, n))
            .unwrap_or_else(|| {
                panic!(
                    "missing {name}: {:?}",
                    self.nodes
                        .iter()
                        .filter_map(|(_, n)| n.label())
                        .collect::<Vec<_>>()
                )
            })
    }
    fn action(&mut self, name: &str, action: AtAction, data: Option<ActionData>) {
        let target = self.node(name).0;
        self.frame(vec![egui::Event::AccessKitActionRequest(ActionRequest {
            action,
            target,
            data,
        })]);
        self.frame(vec![]);
    }
    fn click(&mut self, name: &str) {
        self.action(name, AtAction::Click, None);
    }
    fn key(&mut self, key: Key, modifiers: egui::Modifiers) {
        for pressed in [true, false] {
            self.frame(vec![egui::Event::Key {
                key,
                physical_key: None,
                pressed,
                repeat: false,
                modifiers,
            }]);
        }
    }
    fn text(&mut self, name: &str, value: &str) {
        self.action(name, AtAction::Focus, None);
        self.key(
            Key::A,
            egui::Modifiers {
                ctrl: true,
                command: true,
                ..Default::default()
            },
        );
        if value.is_empty() {
            self.key(Key::Backspace, Default::default());
        } else {
            self.frame(vec![egui::Event::Text(value.into())]);
        }
        self.frame(vec![]);
    }
    fn open(&mut self) {
        self.click("Sampler: Edit sampler banks");
        self.wait(|g| g.app.sampler_editor.store.as_ref().is_some_and(|s| !s.busy));
    }
    fn create(&mut self) {
        self.click("Sampler editor: Create empty bank");
        self.ready();
    }
    fn ready(&mut self) {
        self.wait(|g| g.app.sampler_editor.loading.is_none());
        assert!(
            self.app.sampler_editor.error.is_none(),
            "{:?}",
            self.app.sampler_editor.error
        );
    }
    fn add_source(&mut self, path: PathBuf) {
        let source = LibSource::File(path.clone());
        let fingerprint = FileFingerprint::read(&path).unwrap();
        let mut catalog = (*self.app.library_metadata.catalog).clone();
        catalog
            .upsert(
                source.clone(),
                Some(fingerprint),
                crate::library::Metadata {
                    title: "Sampler fixture".into(),
                    artist: String::new(),
                    bpm: Bpm::UNKNOWN,
                    key: String::new(),
                    duration: None,
                    last_play: None,
                },
            )
            .unwrap();
        self.app.library_metadata.catalog = Arc::new(catalog);
        self.app.library = Arc::new(vec![LibItem {
            title: "Sampler fixture".into(),
            artist: String::new(),
            bpm: Bpm::UNKNOWN,
            fingerprint: Some(fingerprint),
            key: String::new(),
            length: None,
            last_play: None,
            source,
        }]);
        self.app.lib_sel = 0;
        self.app.refresh_library_view();
        self.app.library_metadata.rebase();
        self.wait(|g| !g.app.library_metadata.active());
        self.frame(vec![]);
    }
    fn assign(&mut self) {
        self.click("Sampler editor: Assign selected local source");
        self.ready();
    }
    fn apply(&mut self) {
        self.click("Sampler editor: Apply bank");
        self.wait(|g| g.app.sampler_editor.applying.is_none());
    }
}

#[test]
fn real_editor_creates_assigns_trims_auditions_applies_and_undoes_captured_bank() {
    let files = Files::new();
    let mut g = Gui::new(&files);
    g.add_source(files.wave());
    g.open();
    let originals: Vec<_> = g.rt.sampler_banks.iter().map(|b| b.id).collect();
    g.create();
    g.assign();
    let source = g.app.sampler_editor.draft.as_ref().unwrap().bank.data.audio[0]
        .clone()
        .unwrap();
    assert_eq!(source.frames(), 24_000);
    g.action(
        "Sampler editor: Slot gain",
        AtAction::SetValue,
        Some(ActionData::NumericValue(0.35)),
    );
    g.text("Sampler editor: Start seconds", "0.05");
    g.text(
        "Sampler editor: End seconds (empty means source end)",
        "0.2",
    );
    assert!(!g.app.sampler_editor.draft.as_ref().unwrap().prepared);
    assert_eq!(g.rt.sampler_banks.len(), 3);
    g.click("Sampler editor: Prepare slot preview");
    g.ready();
    let draft = &g.app.sampler_editor.draft.as_ref().unwrap().bank;
    assert_eq!(draft.data.ranges[0], Some((2400.0, 9600.0)));
    assert!((draft.data.settings.slots[0].controls.gain - 0.35).abs() < 1e-6);
    let notes_before = g.rt.tracks[0].clips[0].notes.len();
    g.click("Sampler editor: Audition selected slot");
    let mut audio = vec![0.0; 512];
    g.rt.process(&mut audio);
    assert!(audio.iter().any(|v| v.abs() > 1e-5));
    assert!(g.rt.sampler_audition.is_some());
    assert!(g.rt.pad_voices.iter().all(Option::is_none));
    assert_eq!(g.rt.tracks[0].clips[0].notes.len(), notes_before);
    g.click("Sampler editor: Stop audition");
    assert!(g.rt.sampler_audition.is_none());
    let bank_id = g.app.sampler_editor.draft.as_ref().unwrap().bank.id;
    g.apply();
    assert_eq!(g.rt.sampler_banks.len(), 4);
    assert_eq!(g.rt.sampler_banks[3].id, bank_id);
    assert_eq!(
        g.rt.sampler_banks[..3]
            .iter()
            .map(|b| b.id)
            .collect::<Vec<_>>(),
        originals
    );
    g.app.engine.send(Command::Undo).unwrap();
    g.frame(vec![]);
    assert_eq!(g.rt.sampler_banks.len(), 3);
    g.app.engine.send(Command::Redo).unwrap();
    g.frame(vec![]);
    assert_eq!(g.rt.sampler_banks[3].id, bank_id);
}

#[test]
fn all_sixteen_slots_and_factory_copy_keep_exact_original_target() {
    let files = Files::new();
    let mut g = Gui::new(&files);
    g.open();
    assert!(g.node("Sampler editor: Apply bank").1.is_disabled());
    let original = g.rt.sampler_banks[0].data.audio.clone();
    g.click("Sampler editor: Copy draft bank");
    g.ready();
    let target = g.app.sampler_editor.draft.as_ref().unwrap().target;
    for slot in 0..16 {
        g.click(&format!("Sampler editor: Slot {}", slot + 1));
        g.action(
            "Sampler editor: Slot gain",
            AtAction::SetValue,
            Some(ActionData::NumericValue((slot + 1) as f64 / 16.0)),
        );
        g.click("Sampler editor: Prepare slot preview");
        g.ready();
        assert_eq!(g.app.sampler_editor.draft.as_ref().unwrap().target, target);
    }
    g.app.engine.send(Command::SamplerBank(1)).unwrap();
    g.frame(vec![]);
    g.apply();
    let bank = &g.rt.sampler_banks[3];
    for slot in 0..16 {
        assert!(Arc::ptr_eq(
            bank.data.audio[slot].as_ref().unwrap(),
            original[slot].as_ref().unwrap()
        ));
        assert_eq!(
            bank.data.settings.slots[slot].controls.gain,
            (slot + 1) as f32 / 16.0
        );
    }
}

#[test]
fn actual_store_reopens_mixed_format_definition_and_missing_is_not_substituted() {
    let files = Files::new();
    let wave = files.wave();
    let mut g = Gui::new(&files);
    g.add_source(wave.clone());
    g.open();
    g.create();
    g.assign();
    let flac = files.0.join("second.flac");
    std::fs::copy(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/audio/tone.flac"),
        &flac,
    )
    .unwrap();
    g.add_source(flac);
    g.click("Sampler editor: Slot 2");
    g.assign();
    g.text(
        "Sampler editor: New or reusable bank name",
        "Mixed reusable",
    );
    g.click("Sampler editor: Save reusable bank");
    g.wait(|g| g.app.sampler_editor.store.as_ref().unwrap().saved.is_some());
    assert_eq!(g.rt.sampler_banks.len(), 3, "Save is not Apply");
    let saved = g
        .app
        .sampler_editor
        .store
        .as_ref()
        .unwrap()
        .collection
        .clone()
        .unwrap();
    let definition = saved.banks[0].clone();
    assert_eq!(definition.name, "Mixed reusable");
    std::fs::rename(&wave, files.0.join("moved.wav")).unwrap();
    let catalog = g.app.library_metadata.catalog.clone();
    drop(g);
    let mut restarted = Gui::new(&files);
    restarted.app.library_metadata.catalog = catalog;
    restarted.open();
    restarted.click(&format!(
        "Sampler editor: Reusable definition Mixed reusable ({})",
        definition.id
    ));
    restarted.click("Sampler editor: Import reusable definition");
    restarted.ready();
    let draft = &restarted.app.sampler_editor.draft.as_ref().unwrap().bank;
    assert!(draft.data.audio[0].is_none());
    assert!(draft.data.issues[0].is_some());
    assert!(draft.data.audio[1].is_some());
    assert!(draft.data.issues[1].is_none());
    restarted.apply();
    assert!(restarted.rt.sampler_banks[3].data.audio[0].is_none());
    assert_eq!(
        restarted.rt.sampler_banks[3].data.settings.definition,
        Some(definition.id)
    );
}

#[test]
fn cancel_wins_before_claim_but_late_close_does_not_misreport_apply() {
    let files = Files::new();
    let mut g = Gui::new(&files);
    g.open();
    g.create();
    g.render = false;
    g.click("Sampler editor: Apply bank");
    assert!(g.app.sampler_editor.applying.is_some());
    g.click("Sampler editor: Cancel or close sampler editor");
    g.rt.process(&mut []);
    g.rt.publish_for_test();
    g.frame(vec![]);
    assert_eq!(g.rt.sampler_banks.len(), 3);
    assert!(!g.app.sampler_editor.open);
    g.render = true;
    g.open();
    g.create();
    g.render = false;
    g.click("Sampler editor: Apply bank");
    g.rt.process(&mut []);
    g.click("Sampler editor: Cancel or close sampler editor");
    g.frame(vec![]);
    assert_eq!(g.rt.sampler_banks.len(), 4);
    assert!(
        g.app.sampler_editor.message.contains("applied")
            || g.app.sampler_editor.message.contains("Applied")
    );
}

#[test]
fn invalid_trim_and_stale_target_are_not_silently_retargeted() {
    let files = Files::new();
    let mut g = Gui::new(&files);
    g.add_source(files.wave());
    g.open();
    g.create();
    g.assign();
    g.text("Sampler editor: Start seconds", "not a number");
    g.frame(vec![]);
    assert_eq!(
        g.app.sampler_editor.draft.as_ref().unwrap().starts[0],
        "not a number"
    );
    g.click("Sampler editor: Prepare slot preview");
    g.wait(|g| g.app.sampler_editor.loading.is_none());
    assert!(g.app.sampler_editor.error.is_some());
    assert_eq!(g.rt.sampler_banks.len(), 3);
    g.text("Sampler editor: Start seconds", "0");
    g.click("Sampler editor: Prepare slot preview");
    g.ready();
    let stale = g.app.sampler_editor.draft.as_ref().unwrap().target;
    let prepared = crate::sampler_bank::prepare::run(
        Request {
            epoch: g.app.snap.sampler_epoch,
            revision: g.app.snap.sampler_revision,
            sample_rate: 48_000,
            origins: Vec::new(), catalog: g.app.library_metadata.catalog.clone(),
            operation: Operation::Empty {
                name: "External bank".into(),
            },
        },
        &g.app.engine.sampler_assets,
        || false,
    )
    .unwrap();
    g.app.engine.send(prepared.command(Ack::new())).unwrap();
    g.frame(vec![]);
    g.frame(vec![]);
    assert_eq!(g.app.sampler_editor.draft.as_ref().unwrap().target, stale);
    assert!(g.node("Sampler editor: Apply bank").1.is_disabled());
}

#[test]
fn small_window_focus_help_and_escape_stop_audition() {
    let files = Files::new();
    let mut g = Gui::new(&files);
    g.open();
    g.click("Sampler editor: Copy draft bank");
    g.ready();
    g.size = Vec2::new(520.0, 400.0);
    g.frame(vec![]);
    for label in [
        "Slot 16",
        "Start seconds",
        "End seconds (empty means source end)",
        "Slot gain",
        "Prepare slot preview",
        "Audition selected slot",
        "Save reusable bank",
        "Apply bank",
        "Cancel or close sampler editor",
    ] {
        let name = format!("Sampler editor: {label}");
        g.action(&name, AtAction::Focus, None);
        for _ in 0..3 {
            g.frame(vec![]);
        }
        let bounds = g.node(&name).1.bounds().unwrap();
        assert!(
            bounds.x0 >= 0.0 && bounds.x1 <= 520.0 && bounds.y0 >= 0.0 && bounds.y1 <= 400.0,
            "{name}: {bounds:?}"
        );
    }
    g.action("Sampler editor: Slot 16", AtAction::Focus, None);
    g.frame(vec![]);
    let node = g.node("Sampler editor: Slot 16").1;
    assert!(node.description().unwrap().contains("1–16"));
    g.key(Key::F1, Default::default());
    assert!(g.app.keys_open);
    g.app.keys_open = false;
    g.size = Vec2::new(1600.0, 1600.0);
    g.frame(vec![]);
    g.click("Sampler editor: Audition selected slot");
    assert!(g.app.sampler_editor.audition.is_some());
    g.key(Key::Escape, Default::default());
    assert!(!g.app.sampler_editor.open);
    assert!(g.rt.sampler_audition.is_none());
    assert!(g.app.sampler_editor.audition.is_none());
}

#[test]
fn fresh_proofs_wait_for_applied_and_failed_source_replacement_preserves_preview() {
    let files = Files::new();
    let path = files.wave();
    let mut g = Gui::new(&files);
    g.add_source(path.clone());
    g.open();
    g.create();
    g.assign();
    let source = LibSource::File(path.clone());
    let fp = FileFingerprint::read(&path).unwrap();
    assert_eq!(g.app.sampler_editor.draft.as_ref().unwrap().proofs.len(), 1);
    assert!(
        g.app
            .library_metadata
            .catalog
            .version(&source, Some(fp))
            .unwrap()
            .content_hash
            .is_none(),
        "preview must not qualify"
    );
    g.action(
        "Sampler editor: Slot gain",
        AtAction::SetValue,
        Some(ActionData::NumericValue(0.75)),
    );
    g.click("Sampler editor: Prepare slot preview");
    g.ready();
    assert_eq!(
        g.app.sampler_editor.draft.as_ref().unwrap().proofs.len(),
        1,
        "controls retain the same fresh decoded source proof"
    );
    g.render = false;
    g.click("Sampler editor: Apply bank");
    assert!(
        g.app
            .library_metadata
            .catalog
            .version(&source, Some(fp))
            .unwrap()
            .content_hash
            .is_none(),
        "admission is not Applied"
    );
    g.rt.process(&mut []);
    g.rt.publish_for_test();
    g.render = true;
    g.wait(|g| g.app.sampler_editor.applying.is_none() && !g.app.library_metadata.active());
    assert!(g
        .app
        .library_metadata
        .catalog
        .version(&source, Some(fp))
        .unwrap()
        .content_hash
        .is_some());
    g.click("Sampler editor: Edit current bank");
    let original = g.app.sampler_editor.draft.as_ref().unwrap().bank.data.audio[0]
        .as_ref()
        .unwrap()
        .clone();
    std::fs::write(&path, b"changed source bytes").unwrap();
    g.click("Sampler editor: Assign selected local source");
    g.wait(|g| g.app.sampler_editor.loading.is_none());
    assert!(g.app.sampler_editor.error.is_some());
    assert!(Arc::ptr_eq(
        &original,
        g.app.sampler_editor.draft.as_ref().unwrap().bank.data.audio[0]
            .as_ref()
            .unwrap()
    ));
    assert!(Arc::ptr_eq(
        &original,
        g.rt.sampler_banks[3].data.audio[0].as_ref().unwrap()
    ));
}

#[test]
fn same_frame_apply_cancel_never_submits_and_pending_native_close_preserves_commit_truth() {
    let files = Files::new();
    let mut g = Gui::new(&files);
    g.open();
    g.create();
    let apply = g.node("Sampler editor: Apply bank").0;
    let cancel = g.node("Sampler editor: Cancel or close sampler editor").0;
    g.frame(
        vec![apply, cancel]
            .into_iter()
            .map(|target| {
                egui::Event::AccessKitActionRequest(ActionRequest {
                    action: AtAction::Click,
                    target,
                    data: None,
                })
            })
            .collect(),
    );
    g.frame(vec![]);
    assert_eq!(g.rt.sampler_banks.len(), 3);
    assert!(!g.app.sampler_editor.open);
    g.open();
    g.create();
    g.render = false;
    g.click("Sampler editor: Apply bank");
    let out = g.ctx.run(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, g.size)),
            viewports: [(
                egui::ViewportId::ROOT,
                egui::ViewportInfo {
                    events: vec![egui::ViewportEvent::Close],
                    ..Default::default()
                },
            )]
            .into_iter()
            .collect(),
            ..Default::default()
        },
        |ctx| g.app.update_frame(ctx),
    );
    assert!(out.viewport_output[&egui::ViewportId::ROOT]
        .commands
        .iter()
        .any(|c| matches!(c, egui::ViewportCommand::CancelClose)));
    assert!(g.app.sampler_editor.message.contains("Close postponed"));
    assert!(g.app.sampler_editor.applying.is_some());
    g.rt.process(&mut []);
    g.rt.publish_for_test();
    g.render = true;
    g.frame(vec![]);
    assert_eq!(g.rt.sampler_banks.len(), 4);
    assert!(g.app.sampler_editor.message.contains("applied by renderer"));
}

#[test]
fn close_during_real_store_job_can_wait_or_cancel_without_losing_committed_truth() {
    use std::sync::mpsc;
    for cancel in [false, true] {
        let files = Files::new();
        let mut g = Gui::new(&files);
        g.open();
        g.create();
        let path = files.0.join("held-banks.json");
        let (entered, seen) = mpsc::sync_channel(1);
        let (release, proceed) = mpsc::sync_channel(1);
        g.app.sampler_editor.store = Some(
            Manager::with_hook(
                path.clone(),
                g.app.engine.cmd.performance().clone(),
                move |save| {
                    if save {
                        entered.send(()).unwrap();
                        proceed.recv().unwrap();
                    }
                },
            )
            .unwrap(),
        );
        g.wait(|g| !g.app.sampler_editor.store.as_ref().unwrap().busy);
        g.click("Sampler editor: Save reusable bank");
        seen.recv_timeout(Duration::from_secs(3)).unwrap();
        let out = g.ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, g.size)),
                viewports: [(
                    egui::ViewportId::ROOT,
                    egui::ViewportInfo {
                        events: vec![egui::ViewportEvent::Close],
                        ..Default::default()
                    },
                )]
                .into_iter()
                .collect(),
                ..Default::default()
            },
            |ctx| g.app.update_frame(ctx),
        );
        assert!(out.viewport_output[&egui::ViewportId::ROOT]
            .commands
            .iter()
            .any(|c| matches!(c, egui::ViewportCommand::CancelClose)));
        assert!(g.app.sampler_editor.message.contains("Close postponed"));
        assert!(!path.exists());
        if cancel {
            g.frame(vec![]);
            g.click("Sampler editor: Cancel or close sampler editor");
        }
        release.send(()).unwrap();
        g.wait(|g| !g.app.sampler_editor.store.as_ref().unwrap().busy);
        assert_eq!(path.exists(), !cancel);
        if cancel {
            assert!(g
                .app
                .sampler_editor
                .store
                .as_ref()
                .unwrap()
                .error
                .as_ref()
                .unwrap()
                .contains("cancel"));
        } else {
            assert!(
                g.app
                    .sampler_editor
                    .store
                    .as_ref()
                    .unwrap()
                    .saved
                    .as_ref()
                    .unwrap()
                    .commit
                    .durable
            );
            assert!(g.app.sampler_editor.message.contains("saved"));
        }
        assert!(!g.app.sampler_editor.blocks_close());
        assert_eq!(g.rt.sampler_banks.len(), 3);
    }
}

#[test]
fn factory_buttons_and_duplicate_reusable_names_have_distinct_native_targets() {
    let files = Files::new();
    let mut g = Gui::new(&files);
    g.open();
    for factory in Factory::ALL {
        g.click(&format!("Sampler editor: Copy original {}", factory.name()));
        g.ready();
        let draft = &g.app.sampler_editor.draft.as_ref().unwrap().bank;
        assert!(draft.factory.is_none());
        assert!(draft.data.audio.iter().all(Option::is_some));
        assert_ne!(draft.id, g.rt.sampler_banks[factory.index()].id);
        for slot in 0..16 {
            assert!(
                matches!(draft.data.settings.slots[slot].source, Some(Source::Factory { bank, slot: actual }) if bank == factory && actual == slot as u8)
            );
        }
    }
    g.text("Sampler editor: New or reusable bank name", "Same name");
    g.click("Sampler editor: Save reusable bank");
    g.wait(|g| g.app.sampler_editor.store.as_ref().unwrap().saved.is_some());
    let first = g
        .app
        .sampler_editor
        .store
        .as_ref()
        .unwrap()
        .saved
        .as_ref()
        .unwrap()
        .definition;
    g.click("Sampler editor: Save reusable bank");
    g.wait(|g| {
        g.app
            .sampler_editor
            .store
            .as_ref()
            .unwrap()
            .collection
            .as_ref()
            .unwrap()
            .banks
            .len()
            == 2
    });
    let second = g
        .app
        .sampler_editor
        .store
        .as_ref()
        .unwrap()
        .saved
        .as_ref()
        .unwrap()
        .definition;
    assert_ne!(first, second);
    let first_name = format!("Sampler editor: Reusable definition Same name ({first})");
    let second_name = format!("Sampler editor: Reusable definition Same name ({second})");
    assert_ne!(g.node(&first_name).0, g.node(&second_name).0);
    g.click(&first_name);
    assert_eq!(g.app.sampler_editor.definition, Some(first));
    g.click("Replace selected reusable definition");
    g.text("Sampler editor: New or reusable bank name", "First renamed");
    g.click("Sampler editor: Save reusable bank");
    g.wait(|g| g.app.sampler_editor.store.as_ref().unwrap().saved.is_some());
    let collection = g
        .app
        .sampler_editor
        .store
        .as_ref()
        .unwrap()
        .collection
        .as_ref()
        .unwrap();
    assert_eq!(collection.banks.len(), 2);
    assert_eq!(
        collection
            .banks
            .iter()
            .find(|d| d.id == first)
            .unwrap()
            .name,
        "First renamed"
    );
    assert_eq!(
        collection
            .banks
            .iter()
            .find(|d| d.id == second)
            .unwrap()
            .name,
        "Same name"
    );
}

#[test]
fn queued_accessibility_actions_keep_identity_across_async_error_and_store_publication() {
    let files = Files::new();
    let mut g = Gui::new(&files);
    g.add_source(files.wave());
    g.open();
    g.create();
    g.assign();
    let prepare = g.node("Sampler editor: Prepare slot preview").0;
    let apply = g.node("Sampler editor: Apply bank").0;
    let save = g.node("Sampler editor: Save reusable bank").0;
    g.text("Sampler editor: Start seconds", "bad");
    g.click("Sampler editor: Prepare slot preview");
    g.wait(|g| g.app.sampler_editor.loading.is_none());
    assert!(g.app.sampler_editor.error.is_some());
    assert_eq!(g.node("Sampler editor: Prepare slot preview").0, prepare);
    assert_eq!(g.node("Sampler editor: Apply bank").0, apply);
    assert_eq!(g.node("Sampler editor: Save reusable bank").0, save);
    g.text("Sampler editor: Start seconds", "0");
    g.frame(vec![egui::Event::AccessKitActionRequest(ActionRequest {
        action: AtAction::Click,
        target: prepare,
        data: None,
    })]);
    g.ready();
    assert!(g.app.sampler_editor.draft.as_ref().unwrap().applicable());
    g.frame(vec![egui::Event::AccessKitActionRequest(ActionRequest {
        action: AtAction::Click,
        target: save,
        data: None,
    })]);
    g.wait(|g| g.app.sampler_editor.store.as_ref().unwrap().saved.is_some());
    assert_eq!(g.node("Sampler editor: Save reusable bank").0, save);
    assert_eq!(g.node("Sampler editor: Apply bank").0, apply);
    g.frame(vec![egui::Event::AccessKitActionRequest(ActionRequest {
        action: AtAction::Click,
        target: apply,
        data: None,
    })]);
    g.wait(|g| g.app.sampler_editor.applying.is_none());
    assert_eq!(g.rt.sampler_banks.len(), 4);
}

#[test]
fn applied_ack_waits_for_snapshot_before_edit_current_can_capture_a_stale_bank() {
    let files = Files::new();
    let mut g = Gui::new(&files);
    g.open();
    g.create();
    let old_revision = g.app.snap.project_revision;
    g.render = false;
    g.click("Sampler editor: Apply bank");
    g.rt.process(&mut []);
    g.app.poll_sampler_editor();
    assert!(
        g.app.sampler_editor.applying.is_none(),
        "Applied is acknowledged, not inferred from a snapshot"
    );
    assert_eq!(g.app.snap.project_revision, old_revision);
    assert!(g.app.sampler_editor.awaiting_snapshot.is_some());
    assert!(
        g.app.sampler_editor.busy(),
        "new editor actions cannot capture the old selected bank"
    );
    g.app.engine.send(Command::Undo).unwrap();
    g.rt.process(&mut []);
    g.rt.publish_for_test();
    g.app.snap = g.app.engine.snapshot();
    g.app.poll_sampler_editor();
    assert!(
        g.app.sampler_editor.awaiting_snapshot.is_none(),
        "a later published Undo also finishes the barrier"
    );
    assert_eq!(g.rt.sampler_banks.len(), 3);
    assert!(g.app.sampler_editor.draft.as_ref().unwrap().applied);
}

#[test]
fn disconnected_audition_retains_unknown_notice_without_trapping_native_close() {
    let files = Files::new();
    let mut g = Gui::new(&files);
    g.open();
    g.click("Sampler editor: Audition selected slot");
    assert!(g.app.sampler_editor.audition.is_some());
    assert_eq!(
        g.app.sampler_editor.audition.as_ref().unwrap().1.state(),
        EditState::Applied
    );
    let (_replacement_engine, replacement) = Engine::headless_for_test(48_000, 256);
    drop(std::mem::replace(&mut g.rt, replacement));
    g.render = false;
    assert!(!g.app.engine.cmd.is_connected());
    g.click("Sampler editor: Cancel or close sampler editor");
    assert!(!g.app.sampler_editor.blocks_close());
    assert!(!g.app.sampler_editor.stop_requested);
    assert!(g
        .app
        .sampler_editor
        .error
        .as_ref()
        .unwrap()
        .contains("unknown"));
    let out = g.ctx.run(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, g.size)),
            viewports: [(
                egui::ViewportId::ROOT,
                egui::ViewportInfo {
                    events: vec![egui::ViewportEvent::Close],
                    ..Default::default()
                },
            )]
            .into_iter()
            .collect(),
            ..Default::default()
        },
        |ctx| g.app.update_frame(ctx),
    );
    assert!(out.viewport_output[&egui::ViewportId::ROOT]
        .commands
        .iter()
        .any(|c| matches!(c, egui::ViewportCommand::CancelClose)));
    g.wait(|g| !g.app.project.dialog_is_closed());
    g.frame(vec![]);
    g.frame(vec![]); // Let the native modal finish its sizing pass.
    assert!(
        !g.node("Discard changes").1.is_disabled(),
        "existing explicit unavailable-renderer shutdown path remains usable"
    );
}

#[test]
fn applied_source_proof_allows_real_relocation_and_reopened_definition_recovers_moved_pcm() {
    let files = Files::new();
    let original = files.wave();
    let moved = files.0.join("moved.wav");
    let mut g = Gui::new(&files);
    g.add_source(original.clone());
    g.open();
    g.create();
    g.assign();
    let source = LibSource::File(original.clone());
    let fp = FileFingerprint::read(&original).unwrap();
    let expected = g.app.sampler_editor.draft.as_ref().unwrap().bank.data.audio[0]
        .as_ref()
        .unwrap()
        .data
        .clone();
    g.apply();
    g.wait(|g| !g.app.library_metadata.active());
    assert!(g
        .app
        .library_metadata
        .catalog
        .version(&source, Some(fp))
        .unwrap()
        .content_hash
        .is_some());
    g.text(
        "Sampler editor: New or reusable bank name",
        "Moved source recovery",
    );
    g.click("Sampler editor: Save reusable bank");
    g.wait(|g| g.app.sampler_editor.store.as_ref().unwrap().saved.is_some());
    let definition = g
        .app
        .sampler_editor
        .store
        .as_ref()
        .unwrap()
        .saved
        .as_ref()
        .unwrap()
        .definition;
    g.click("Sampler editor: Cancel or close sampler editor");
    let identity = g
        .app
        .library_metadata
        .catalog
        .track(&source)
        .unwrap()
        .id
        .clone();
    std::fs::rename(&original, &moved).unwrap();
    g.app.open_cue_relocation();
    g.frame(vec![]);
    g.frame(vec![]);
    g.text("Relocated track path", moved.to_str().unwrap());
    g.click("Verify and relocate track");
    let destination = LibSource::File(moved.clone());
    g.wait(|g| {
        !g.app.library_metadata.active()
            && g.app.library_metadata.catalog.track(&destination).is_some()
    });
    assert_eq!(
        g.app
            .library_metadata
            .catalog
            .track(&destination)
            .unwrap()
            .id,
        identity
    );
    assert!(g.app.library_metadata.durable);
    drop(g);
    let mut restarted = Gui::new(&files);
    restarted.open();
    restarted.click(&format!(
        "Sampler editor: Reusable definition Moved source recovery ({definition})"
    ));
    restarted.click("Sampler editor: Import reusable definition");
    restarted.ready();
    let draft = &restarted.app.sampler_editor.draft.as_ref().unwrap().bank;
    assert!(draft.data.issues[0].is_none());
    assert_eq!(draft.data.audio[0].as_ref().unwrap().data, expected);
    assert!(
        matches!(&draft.data.settings.slots[0].source, Some(Source::Library { reference }) if reference.source == destination && reference.track == identity)
    );
    restarted.apply();
    assert_eq!(
        restarted.rt.sampler_banks[3].data.audio[0]
            .as_ref()
            .unwrap()
            .data,
        expected
    );
}

mod offline;
