use super::*;
use crate::ui::test_support::Fixture;
use egui::accesskit::{Action, ActionRequest, Node, NodeId};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Gui {
    fixture: Fixture,
    ctx: egui::Context,
    nodes: Vec<(NodeId, Node)>,
    time: f64,
    dir: PathBuf,
}
impl Drop for Gui {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}
impl Gui {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "omatainer-prefs-ui-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let mut fixture = Fixture::new(256);
        let mut startup = Startup::read(dir.join("preferences.json"), dir.clone());
        for profile in startup.preferences.profiles.values_mut() {
            profile.startup.scan_library = false;
        }
        fixture.app.settings = Settings::from_startup(startup, false, None);
        fixture.app.settings.worker = Some(
            Worker::with_discovery(dir.join("preferences.json"), || {
                Ok(crate::engine::audio::config::tests::inventory())
            })
            .unwrap(),
        );
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut gui = Self {
            fixture,
            ctx,
            nodes: vec![],
            time: 0.0,
            dir,
        };
        gui.frame(vec![]);
        gui.frame(vec![]);
        gui
    }
    fn frame(&mut self, events: Vec<egui::Event>) -> egui::FullOutput {
        self.raw_frame(egui::RawInput {
            events,
            ..Default::default()
        })
    }
    fn raw_frame(&mut self, mut raw: egui::RawInput) -> egui::FullOutput {
        self.time += 0.02;
        let out = self.ctx.run(
            {
                raw.screen_rect = Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1600.0, 1200.0)));
                raw.time = Some(self.time);
                raw
            },
            |ctx| self.fixture.app.update_frame(ctx),
        );
        self.nodes = out
            .platform_output
            .accesskit_update
            .as_ref()
            .unwrap()
            .nodes
            .clone();
        self.fixture.rt.process(&mut [0.0; 128]);
        self.fixture.rt.publish_for_test();
        out
    }
    fn node(&self, name: &str) -> NodeId {
        self.nodes
            .iter()
            .find(|(_, node)| node.label() == Some(name))
            .map(|(id, _)| *id)
            .unwrap_or_else(|| {
                panic!(
                    "missing {name}; {:?}",
                    self.nodes
                        .iter()
                        .filter_map(|(_, n)| n.label())
                        .collect::<Vec<_>>()
                )
            })
    }
    fn click(&mut self, name: &str) {
        let target = self.node(name);
        self.frame(vec![egui::Event::AccessKitActionRequest(ActionRequest {
            target,
            action: Action::Click,
            data: None,
        })]);
        self.frame(vec![]);
    }
    fn wait(&mut self) {
        let end = Instant::now() + std::time::Duration::from_secs(3);
        while self.fixture.app.settings.busy() {
            self.frame(vec![]);
            assert!(Instant::now() < end);
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        self.frame(vec![]);
    }
    fn open(&mut self) {
        self.click("Preferences");
        assert!(self.fixture.app.settings.open);
    }
    fn preview_apply(&mut self) {
        self.click("Preview changes");
        self.wait();
        assert!(self
            .fixture
            .app
            .settings
            .preview
            .as_ref()
            .unwrap()
            .1
            .is_ok());
        self.click("Apply and save");
        self.wait();
    }
}
#[test]
fn real_window_previews_persists_applies_and_reopens_supported_profile_settings() {
    let mut gui = Gui::new();
    gui.open();
    gui.click("Follow desktop theme and font");
    gui.click("Open help on startup");
    let state = &mut gui.fixture.app.settings;
    let profile = state.draft.profiles.get_mut("Studio").unwrap();
    // Numeric widget state then native SetValue is covered by accessibility;
    // here the UI's committed worker path carries all validated settings.
    profile.audio.sample_rate = Some(96000);
    profile.audio.buffer_frames = Some(256);
    profile.appearance.font_size = Some(18.0);
    profile.appearance.scale = 1.25;
    profile.shortcuts.insert(
        "transport".into(),
        Some(model::Shortcut {
            key: "T".into(),
            ctrl: false,
            shift: false,
            alt: false,
        }),
    );
    gui.preview_apply();
    assert!(gui.fixture.app.settings.pending_restart());
    assert_eq!(gui.fixture.app.theme.font_size, 18.0);
    assert_eq!(gui.ctx.zoom_factor(), 1.25);
    let loaded = storage::load(&gui.dir.join("preferences.json"), &AtomicBool::new(false)).unwrap();
    assert_eq!(loaded.preferences, gui.fixture.app.settings.applied);
    let startup = Startup::read(gui.dir.join("preferences.json"), gui.dir.clone());
    let mut reopened = Fixture::new(64);
    reopened.app.initialize_preferences(
        &gui.ctx,
        startup,
        loaded.preferences.current().unwrap().audio.clone(),
    );
    assert!(reopened.app.keys_open);
    assert!(!reopened.app.settings.pending_restart());
    assert!(!reopened.app.settings.profile().appearance.follow_theme);
    gui.click("Cancel changes");
    assert!(!gui.fixture.app.settings.open);
    gui.ctx
        .memory_mut(|memory| memory.surrender_focus(memory.focused().unwrap_or(egui::Id::NULL)));
    gui.frame(vec![]);
    gui.frame(vec![egui::Event::Key {
        key: Key::T,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    }]);
    assert!(
        gui.fixture.rt.playing,
        "remapped shortcut reaches real engine"
    );
}
#[test]
fn delayed_preview_and_save_cancel_leave_file_live_values_and_held_gate_unchanged() {
    let mut gui = Gui::new();
    gui.open();
    gui.preview_apply();
    let before = std::fs::read(gui.dir.join("preferences.json")).unwrap();
    let previous = gui.fixture.app.settings.applied.clone();
    gui.fixture
        .app
        .settings
        .draft
        .profiles
        .get_mut("Studio")
        .unwrap()
        .appearance
        .scale = 1.4;
    gui.fixture
        .app
        .settings
        .worker
        .as_ref()
        .unwrap()
        .delay
        .store(100, Ordering::Release);
    gui.click("Preview changes");
    gui.click("Cancel pending preferences operation");
    gui.wait();
    assert_eq!(gui.fixture.app.settings.applied, previous);
    assert!(gui.fixture.app.settings.message.contains("Cancelled"));
    gui.fixture
        .app
        .settings
        .worker
        .as_ref()
        .unwrap()
        .delay
        .store(0, Ordering::Release);
    gui.click("Preview changes");
    gui.wait();
    gui.fixture
        .app
        .settings
        .worker
        .as_ref()
        .unwrap()
        .delay
        .store(100, Ordering::Release);
    gui.click("Apply and save");
    gui.click("Cancel pending preferences operation");
    gui.wait();
    assert_eq!(
        std::fs::read(gui.dir.join("preferences.json")).unwrap(),
        before
    );
    assert_eq!(gui.fixture.app.settings.applied, previous);
}
#[test]
fn invalid_import_and_unavailable_route_keep_current_state_and_show_failure() {
    let mut gui = Gui::new();
    gui.open();
    let previous = gui.fixture.app.settings.applied.clone();
    gui.fixture
        .app
        .settings
        .draft
        .profiles
        .get_mut("Studio")
        .unwrap()
        .audio
        .device = Some("Missing interface".into());
    gui.click("Preview changes");
    gui.wait();
    assert!(gui
        .fixture
        .app
        .settings
        .preview
        .as_ref()
        .unwrap()
        .1
        .is_err());
    let node = gui
        .nodes
        .iter()
        .find(|(_, n)| n.label() == Some("Apply and save"))
        .unwrap();
    assert!(node.1.is_disabled());
    let bad = gui.dir.join("newer.json");
    std::fs::write(&bad, b"{\"version\":999}").unwrap();
    gui.fixture.app.settings.file_path = bad.to_string_lossy().into();
    gui.click("Import into draft");
    gui.wait();
    assert!(gui.fixture.app.settings.message.contains("version"));
    assert_eq!(gui.fixture.app.settings.applied, previous);
    assert_eq!(std::fs::read(&bad).unwrap(), b"{\"version\":999}");
}
#[test]
fn explicit_backup_reset_recovers_invalid_cache_and_export_never_overwrites() {
    let mut gui = Gui::new();
    let path = gui.dir.join("preferences.json");
    std::fs::write(&path, b"{\"version\":999}").unwrap();
    let startup = Startup::read(path.clone(), gui.dir.clone());
    assert!(startup.blocked);
    gui.fixture.app.settings = Settings::from_startup(startup, false, None);
    gui.fixture.app.settings.worker = Some(
        Worker::with_discovery(path.clone(), || {
            Ok(crate::engine::audio::config::tests::inventory())
        })
        .unwrap(),
    );
    gui.open();
    gui.click("Preserve old file and reset all preferences");
    gui.wait();
    assert!(!gui.fixture.app.settings.blocked);
    let backup = std::fs::read_dir(&gui.dir)
        .unwrap()
        .filter_map(Result::ok)
        .find(|e| e.file_name().to_string_lossy().contains("preserved-"))
        .unwrap();
    assert_eq!(std::fs::read(backup.path()).unwrap(), b"{\"version\":999}");
    let export = gui.dir.join("portable.json");
    gui.fixture.app.settings.file_path = export.to_string_lossy().into();
    gui.click("Export draft");
    gui.wait();
    let bytes = std::fs::read(&export).unwrap();
    gui.click("Export draft");
    gui.wait();
    assert!(gui.fixture.app.settings.message.contains("failed"));
    assert_eq!(std::fs::read(&export).unwrap(), bytes);
    gui.click("Import into draft");
    gui.wait();
    assert!(gui.fixture.app.settings.message.contains("Imported"));
}

#[test]
fn profile_preview_cancel_preserves_live_midi_ownership_and_committed_switch_releases_only_excluded_source(
) {
    use crate::engine::midi::connection_test_support as midi;
    let mut gui = Gui::new();
    gui.fixture.rt.selected_track = 1;
    let control = midi::install(&mut gui.fixture.app.engine);
    let ports = [("a", "Keyboard A"), ("b", "Keyboard B")];
    control.discover(&ports);
    let a = control.connect("a", Ok(()));
    let b = control.connect("b", Ok(()));
    midi::until(|| gui.fixture.app.engine.midi.policy_status().unwrap().applied == Some(1));
    a.push(&[0x90, 60, 100]);
    b.push(&[0x90, 60, 100]);
    midi::until(|| gui.fixture.app.engine.midi.input_stats().dispatched == 2);
    gui.frame(vec![]);
    let held = |gui: &Gui| {
        gui.fixture.rt.tracks[1]
            .poly
            .voices
            .iter()
            .filter(|voice| voice.input.is_some() && matches!(voice.env.stage, 1..=3))
            .count()
    };
    assert_eq!(held(&gui), 2);
    gui.open();
    gui.fixture
        .app
        .settings
        .draft
        .profiles
        .get_mut("Studio")
        .unwrap()
        .midi_inputs = model::MidiInputs::Selected(vec!["Keyboard A".into()]);
    gui.fixture.app.settings.editor_strings();
    gui.click("Preview changes");
    gui.wait();
    gui.click("Cancel changes");
    assert_eq!(held(&gui), 2);
    assert_eq!(
        gui.fixture
            .app
            .engine
            .midi
            .policy_status()
            .unwrap()
            .requested,
        1
    );
    assert!(!a.is_closed() && !b.is_closed());
    gui.open();
    gui.fixture
        .app
        .settings
        .draft
        .profiles
        .get_mut("Performance")
        .unwrap()
        .midi_inputs = model::MidiInputs::Selected(vec!["Keyboard A".into()]);
    gui.fixture.app.settings.edited = "Performance".into();
    gui.fixture.app.settings.editor_strings();
    gui.frame(vec![]);
    gui.click("Use this profile");
    gui.preview_apply();
    assert!(gui
        .fixture
        .app
        .engine
        .midi
        .policy_status()
        .unwrap()
        .pending());
    control.discover(&ports);
    midi::until(|| {
        !gui.fixture
            .app
            .engine
            .midi
            .policy_status()
            .unwrap()
            .pending()
    });
    gui.frame(vec![]);
    assert_eq!(held(&gui), 1);
    assert!(!a.is_closed() && b.is_closed());
    assert_eq!(gui.fixture.app.settings.applied.active, "Performance");
    a.push(&[0x80, 60, 0]);
    midi::until(|| gui.fixture.app.engine.midi.input_stats().dispatched == 3);
    gui.frame(vec![]);
    assert_eq!(held(&gui), 0);
}

#[test]
fn reset_result_rebuilds_editor_buffers_before_next_frame_can_write_old_roots_back() {
    let mut gui = Gui::new();
    gui.open();
    let state = &mut gui.fixture.app.settings;
    let mut custom = state.profile().clone();
    custom.library_roots = vec![gui.dir.join("old-root")];
    custom.midi_inputs = model::MidiInputs::Selected(vec!["Old keyboard".into()]);
    state
        .draft
        .profiles
        .insert("Old renamed profile".into(), custom);
    state.edited = "Old renamed profile".into();
    state.editor_strings();
    state.blocked = true;
    gui.frame(vec![]);
    gui.click("Preserve old file and reset all preferences");
    gui.wait();
    let state = &gui.fixture.app.settings;
    assert_eq!(state.edited, "Studio");
    assert_eq!(state.draft, state.applied);
    assert!(state.midi_names.is_empty());
    assert!(!state.roots.contains("old-root"));
    assert_eq!(
        state.draft.current().unwrap().library_roots,
        model::Profile::defaults(&gui.dir).library_roots
    );
}

#[test]
fn applied_library_roots_drive_the_real_scan_and_failed_save_keeps_previous_roots() {
    let mut gui = Gui::new();
    let folder = gui.dir.join("new-music");
    std::fs::create_dir(&folder).unwrap();
    std::fs::write(folder.join("Only new root.wav"), b"fixture metadata scan").unwrap();
    gui.open();
    gui.fixture.app.settings.roots = folder.to_string_lossy().into();
    gui.frame(vec![]);
    gui.preview_apply();
    let end = Instant::now() + std::time::Duration::from_secs(3);
    while !gui
        .fixture
        .app
        .library
        .iter()
        .any(|item| item.source == LibSource::File(folder.join("Only new root.wav")))
    {
        gui.frame(vec![]);
        std::thread::sleep(std::time::Duration::from_millis(2));
        assert!(Instant::now() < end);
    }
    let old = gui.fixture.app.settings.applied.clone();
    // An external edit after the preview may not be clobbered by Apply.
    gui.fixture.app.settings.roots = gui.dir.join("other").to_string_lossy().into();
    gui.frame(vec![]);
    gui.click("Preview changes");
    gui.wait();
    let path = gui.dir.join("preferences.json");
    std::fs::write(&path, b"external editor change").unwrap();
    gui.click("Apply and save");
    gui.wait();
    assert_eq!(gui.fixture.app.settings.applied, old);
    assert_eq!(std::fs::read(&path).unwrap(), b"external editor change");
    assert!(gui.fixture.app.settings.message.contains("reload"));
}

#[test]
fn native_close_preserves_pending_preferences_work_until_explicit_cancel_or_completion() {
    let mut gui = Gui::new();
    gui.open();
    gui.preview_apply();
    let path = gui.dir.join("preferences.json");
    let before = std::fs::read(&path).unwrap();
    gui.fixture
        .app
        .settings
        .draft
        .profiles
        .get_mut("Studio")
        .unwrap()
        .appearance
        .scale = 1.4;
    gui.click("Preview changes");
    gui.wait();
    gui.fixture
        .app
        .settings
        .worker
        .as_ref()
        .unwrap()
        .delay
        .store(1000, Ordering::Release);
    gui.click("Apply and save");
    assert!(gui.fixture.app.settings.busy());
    let mut raw = egui::RawInput::default();
    raw.viewports
        .get_mut(&egui::ViewportId::ROOT)
        .unwrap()
        .events
        .push(egui::ViewportEvent::Close);
    let output = gui.raw_frame(raw);
    let commands = &output.viewport_output[&egui::ViewportId::ROOT].commands;
    assert!(commands
        .iter()
        .any(|command| matches!(command, egui::ViewportCommand::CancelClose)));
    assert!(!commands
        .iter()
        .any(|command| matches!(command, egui::ViewportCommand::Close)));
    assert!(gui.fixture.app.settings.message.contains("Close cancelled"));
    assert!(!gui.fixture.app.project.committing());
    assert!(gui.fixture.app.engine.send(Command::Master(0.37)).is_ok());
    gui.click("Cancel pending preferences operation");
    gui.wait();
    assert_eq!(std::fs::read(&path).unwrap(), before);
    assert!(!gui.fixture.app.project.committing());
    assert_eq!(gui.fixture.rt.master, 0.37);
}

#[test]
fn persisted_undo_remap_controls_the_renderer_and_edit_menu_hint() {
    let mut gui = Gui::new();
    gui.open();
    gui.fixture
        .app
        .settings
        .draft
        .profiles
        .get_mut("Studio")
        .unwrap()
        .shortcuts
        .insert(
            "undo".into(),
            Some(model::Shortcut {
                key: "U".into(),
                ctrl: true,
                shift: false,
                alt: false,
            }),
        );
    gui.preview_apply();
    let loaded = storage::load(&gui.dir.join("preferences.json"), &AtomicBool::new(false)).unwrap();
    assert_eq!(
        loaded.preferences.current().unwrap().shortcuts["undo"]
            .as_ref()
            .unwrap()
            .key,
        "U"
    );
    gui.click("Cancel changes");
    gui.ctx
        .memory_mut(|memory| memory.surrender_focus(memory.focused().unwrap_or(egui::Id::NULL)));
    let before = gui.fixture.rt.master;
    gui.fixture.app.engine.send(Command::Master(0.31)).unwrap();
    gui.frame(vec![]);
    gui.frame(vec![]);
    assert_eq!(gui.fixture.rt.master, 0.31);
    gui.frame(vec![egui::Event::Key {
        key: Key::U,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::CTRL,
    }]);
    assert_eq!(gui.fixture.rt.master, before);
    gui.fixture.app.engine.send(Command::Master(0.42)).unwrap();
    gui.frame(vec![]);
    gui.frame(vec![]);
    gui.click("Edit");
    let output = gui.frame(vec![]);
    assert!(
        output.shapes.iter().any(|shape| matches!(&shape.shape,
        egui::epaint::Shape::Text(text) if text.galley.text() == "Ctrl+U")),
        "menu must display the saved effective shortcut"
    );
}

#[test]
fn recovery_limits_are_real_numeric_controls_persist_reopen_and_cancel() {
    use egui::accesskit::ActionData;
    let mut gui = Gui::new();
    gui.open();
    gui.click("Autosave and recovery limits");
    for (label, value) in [("Recovery checkpoint interval", 75.0), ("Recovery generations per session", 5.0), ("Recovery storage limit", 512.0)] {
        let target = gui.node(label);
        gui.frame(vec![egui::Event::AccessKitActionRequest(ActionRequest { target, action: Action::SetValue, data: Some(ActionData::NumericValue(value)) })]);
        gui.frame(vec![]);
    }
    let draft = gui.fixture.app.settings.draft.profiles["Studio"].recovery.clone();
    assert_eq!((draft.checkpoint_seconds, draft.retention, draft.max_bytes), (75, 5, 512 * crate::recovery::MIB));
    gui.preview_apply();
    let loaded = storage::load(&gui.dir.join("preferences.json"), &AtomicBool::new(false)).unwrap();
    assert_eq!(loaded.preferences.current().unwrap().recovery, draft);
    let previous = loaded.preferences.current().unwrap().recovery.clone();
    gui.click("Cancel changes"); gui.open();
    if !gui.nodes.iter().any(|(_, node)| node.label() == Some("Recovery checkpoint interval")) { gui.click("Autosave and recovery limits"); }
    let target = gui.node("Recovery checkpoint interval");
    gui.frame(vec![egui::Event::AccessKitActionRequest(ActionRequest { target, action: Action::SetValue, data: Some(ActionData::NumericValue(100.0)) })]);
    gui.click("Cancel changes");
    assert_eq!(gui.fixture.app.settings.profile().recovery, previous);
    assert_eq!(storage::load(&gui.dir.join("preferences.json"), &AtomicBool::new(false)).unwrap().preferences.current().unwrap().recovery, previous);
}

#[test]
fn protected_pending_root_scan_waits_for_studio_without_losing_the_request() {
    let mut gui = Gui::new();
    gui.fixture.app.library_metadata = crate::ui::library_metadata::Metadata::new(Some(gui.dir.join("library.json")));
    gui.fixture.app.library_metadata.set_performance(gui.fixture.app.engine.cmd.performance().clone());
    let end = Instant::now() + std::time::Duration::from_secs(3);
    while !gui.fixture.app.library_metadata.ready() {
        gui.frame(vec![]);
        assert!(Instant::now() < end);
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    let active = gui.fixture.app.settings.applied.active.clone();
    gui.fixture.app.settings.applied.profiles.get_mut(&active).unwrap().library_roots = vec![gui.dir.clone()];
    gui.fixture.app.engine.cmd.send(Command::PerformanceMode(true)).unwrap();
    gui.fixture.rt.process(&mut [0.0; 128]);
    assert!(gui.fixture.app.engine.cmd.performance().protected());
    gui.fixture.app.scan_library();
    assert!(gui.fixture.app.settings.rescan, "protected scan must retain the pending request");
    for _ in 0..3 { gui.frame(vec![]); }
    assert!(gui.fixture.app.settings.rescan);
    assert!(!gui.fixture.app.library_scan.active());
    gui.fixture.app.engine.cmd.send(Command::PerformanceMode(false)).unwrap();
    gui.fixture.rt.process(&mut [0.0; 128]);
    gui.frame(vec![]);
    assert!(!gui.fixture.app.settings.rescan);
    assert!(gui.fixture.app.library_scan.active());
    let end = Instant::now() + std::time::Duration::from_secs(3);
    while gui.fixture.app.library_scan.active() || gui.fixture.app.library_metadata.active() {
        gui.frame(vec![]);
        assert!(Instant::now() < end);
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    assert!(gui.fixture.app.library_metadata.catalog.watched_roots.binding(&active, &gui.dir).is_some());
}
