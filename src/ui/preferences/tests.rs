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
    height:f32,
}
impl Drop for Gui {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}
impl Gui {
    fn click_in(&mut self, group: &str, label: &str) {
        let mut pending = self.nodes.iter().find(|(_,node)| node.role()==egui::accesskit::Role::Group && node.label()==Some(group)).unwrap().1.children().to_vec();
        let target = loop {
            let id = pending.pop().expect("named control in its command group");
            if let Some((_,node)) = self.nodes.iter().find(|(current,_)| *current==id) {
                if node.label()==Some(label) { break id; }
                pending.extend_from_slice(node.children());
            }
        };
        self.frame(vec![egui::Event::AccessKitActionRequest(ActionRequest { target,action:Action::Click,data:None })]);
        self.frame(vec![]);
    }
    fn routing_text(&mut self,name:&str,value:&str){
        let target=self.node(name);
        self.frame(vec![egui::Event::AccessKitActionRequest(ActionRequest{target,action:Action::Focus,data:None})]);
        let modifiers=egui::Modifiers{ctrl:true,command:true,..Default::default()};
        self.frame(vec![egui::Event::Key{key:Key::A,physical_key:None,pressed:true,repeat:false,modifiers}]);
        self.frame(vec![egui::Event::Key{key:Key::A,physical_key:None,pressed:false,repeat:false,modifiers},egui::Event::Text(value.into())]);
        self.frame(vec![]);
    }
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
            height:1200.0,
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
                raw.screen_rect = Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1600.0, self.height)));
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
    let mut reopened = Fixture::new(80);
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
#[test]
fn native_midi_routing_controls_preview_cancel_persist_and_reopen_exact_profile() {
    let mut gui = Gui::new();
    crate::engine::midi::routing::install_for_test(&mut gui.fixture.app.engine, Default::default());
    gui.open();
    gui.click("Use explicit track MIDI routing");
    gui.click("Route MIDI track 3");
    gui.click("Track 3 MIDI ports, channels and filters");
    gui.click("Track 3: external MIDI output");
    gui.routing_text("Track 3 output exact name", "Synth");
    gui.click("Track 3 output: require exact backend port id");
    gui.routing_text("Track 3 output exact id", "300:0");
    gui.click("Track 3 output channel: preserve source channel");
    let target = gui.node("Track 3 output channel");
    gui.frame(vec![egui::Event::AccessKitActionRequest(ActionRequest {
        target,
        action: Action::SetValue,
        data: Some(egui::accesskit::ActionData::NumericValue(8.0)),
    })]);
    gui.click("Add track 3 MIDI input");
    gui.click("Track 3 input 1");
    gui.routing_text("Track 3 input 1 exact name", "Keyboard A");
    gui.click("Track 3 input 1: require exact backend port id");
    gui.routing_text("Track 3 input 1 exact id", "100:0");
    gui.click("Track 3 input 1: all channels");
    gui.click("T3 I1 channel 2");
    gui.click("T3 I1 channel 1");
    gui.click("Track 3: Live thru to external output");
    gui.click("Track 3: Complete SysEx ≤256 bytes");
    let expected = gui
        .fixture
        .app
        .settings
        .draft
        .current()
        .unwrap()
        .midi_routing
        .clone();
    assert!(expected.enabled);
    assert_eq!(expected.routes.len(), 1);
    let route = &expected.routes[0];
    assert_eq!(route.track, 2);
    assert_eq!(route.inputs[0].channels, 2);
    assert_eq!(route.inputs[0].port.name, "Keyboard A");
    assert_eq!(route.output.as_ref().unwrap().name, "Synth");
    assert_eq!(route.output_channel, Some(7));
    assert!(route.thru && route.filter.sysex);
    assert!(expected.validate().is_ok());
    gui.click("Preview changes");
    gui.wait();
    assert!(
        !gui.fixture
            .app
            .engine
            .midi
            .routing_status()
            .unwrap()
            .applied
            .enabled
    );
    gui.click("Cancel changes");
    assert!(!gui.dir.join("preferences.json").exists());
    gui.open();
    gui.fixture
        .app
        .settings
        .draft
        .profiles
        .get_mut("Studio")
        .unwrap()
        .midi_routing = expected.clone();
    gui.preview_apply();
    let deadline = Instant::now() + std::time::Duration::from_secs(5);
    while gui
        .fixture
        .app
        .engine
        .midi
        .routing_status()
        .unwrap()
        .pending
    {
        gui.frame(vec![]);
        assert!(Instant::now() < deadline);
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    assert_eq!(
        gui.fixture
            .app
            .engine
            .midi
            .routing_status()
            .unwrap()
            .applied
            .as_ref(),
        &expected
    );
    let file = gui.dir.join("preferences.json");
    let bytes = std::fs::read(&file).unwrap();
    let loaded = storage::load(&file, &AtomicBool::new(false)).unwrap();
    assert_eq!(loaded.preferences.current().unwrap().midi_routing, expected);
    let startup = Startup::read(file, gui.dir.clone());
    assert_eq!(
        startup.preferences.current().unwrap().midi_routing,
        expected
    );
    gui.fixture
        .app
        .settings
        .draft
        .profiles
        .get_mut("Studio")
        .unwrap()
        .midi_routing
        .routes[0]
        .output_channel = Some(12);
    gui.click("Cancel changes");
    assert_eq!(
        std::fs::read(gui.dir.join("preferences.json")).unwrap(),
        bytes
    );
    assert_eq!(
        gui.fixture
            .app
            .engine
            .midi
            .routing_status()
            .unwrap()
            .applied
            .as_ref(),
        &expected
    );
}

#[test]
fn native_missing_output_receipt_preserves_saved_profile_and_midi_panel_resets() {
    let mut gui = Gui::new();
    crate::engine::midi::routing::install_for_test(&mut gui.fixture.app.engine, Default::default());
    gui.open();
    gui.fixture
        .app
        .settings
        .draft
        .profiles
        .get_mut("Studio")
        .unwrap()
        .midi_routing = crate::engine::midi::routing::Routing {
        enabled: true,
        routes: vec![crate::engine::midi::routing::Route {
            track: 2,
            inputs: vec![],
            output: Some(crate::engine::midi::routing::Endpoint {
                name: "Missing synthesizer".into(),
                id: None,
            }),
            output_channel: Some(7),
            monitor: true,
            thru: false,
            filter: Default::default(),
        }],
    };
    gui.preview_apply();
    let end = Instant::now() + std::time::Duration::from_secs(5);
    while gui
        .fixture
        .app
        .engine
        .midi
        .routing_status()
        .unwrap()
        .pending
    {
        gui.frame(vec![]);
        assert!(Instant::now() < end);
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    gui.frame(vec![]);
    let status = gui.fixture.app.engine.midi.routing_status().unwrap();
    assert!(status.error.as_ref().unwrap().contains("missing"));
    assert!(!status.applied.enabled);
    let path = gui.dir.join("preferences.json");
    let bytes = std::fs::read(&path).unwrap();
    assert!(
        storage::load(&path, &AtomicBool::new(false))
            .unwrap()
            .preferences
            .current()
            .unwrap()
            .midi_routing
            .enabled
    );
    gui.click("Cancel changes");
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    gui.height = 720.0;
    gui.fixture.app.midi_open = true;
    gui.frame(vec![]);
    gui.frame(vec![]);
    let output = gui.frame(vec![]);
    assert!(output.shapes.iter().any(|shape|matches!(&shape.shape,egui::epaint::Shape::Text(text) if text.galley.text().contains("missing"))),"missing destination receipt was not painted in native MIDI panel");
    let epoch = gui.fixture.app.engine.cmd.midi_routing().output_state().2;
    gui.click("All notes off / reset MIDI outputs");
    assert!(gui.fixture.app.engine.cmd.midi_routing().output_state().2 > epoch);
    gui.click("Edit MIDI routing in Preferences");
    assert!(gui.fixture.app.settings.open);
    gui.click("Retry saved MIDI routing");
    gui.frame(vec![]);
    while gui
        .fixture
        .app
        .engine
        .midi
        .routing_status()
        .unwrap()
        .pending
    {
        gui.frame(vec![]);
        assert!(Instant::now() < end);
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    assert!(gui
        .fixture
        .app
        .engine
        .midi
        .routing_status()
        .unwrap()
        .error
        .as_ref()
        .unwrap()
        .contains("missing"));
    gui.click("Cancel changes");
    assert_eq!(std::fs::read(path).unwrap(), bytes);
}

#[test]
fn startup_empty_session_is_selected_in_native_preferences_and_persisted() {
    let mut gui = Gui::new(); gui.height = 1900.0; gui.open();
    gui.click("Empty session");
    assert_eq!(gui.fixture.app.settings.draft.current().unwrap().startup.session, crate::project_template::Startup::Empty);
    gui.preview_apply();
    let loaded = storage::load(&gui.dir.join("preferences.json"), &AtomicBool::new(false)).unwrap();
    assert_eq!(loaded.preferences.current().unwrap().startup.session, crate::project_template::Startup::Empty);
    let target = gui.node("Project template");
    gui.frame(vec![egui::Event::AccessKitActionRequest(ActionRequest { target, action: Action::Focus, data: None })]);
    gui.click("Project template");
    assert!(matches!(gui.fixture.app.settings.draft.current().unwrap().startup.session, crate::project_template::Startup::Template { .. }));
    let missing = gui.dir.join("missing.omtemplate"); gui.routing_text("Startup template file", missing.to_str().unwrap());
    gui.click("Preview changes"); gui.wait(); assert!(gui.fixture.app.settings.message.contains("unavailable"));
    assert_eq!(storage::load(&gui.dir.join("preferences.json"), &AtomicBool::new(false)).unwrap().preferences, loaded.preferences);
    gui.click("Cancel changes"); assert_eq!(gui.fixture.app.settings.draft, gui.fixture.app.settings.applied);
}
#[test]
fn template_hardware_draft_keeps_exact_missing_ports_and_rejects_reused_track_target() {
    use crate::project_template as template;
    use crate::engine::midi::routing::{Endpoint, Route, Routing, Filter};
    let mut gui = Gui::new(); gui.height = 1900.0;
    crate::engine::midi::routing::install_for_test(&mut gui.fixture.app.engine, Routing::default());
    let original = gui.fixture.app.settings.applied.clone();
    let mut hardware = template::Hardware::capture(gui.fixture.app.settings.profile());
    hardware.routing = Routing { enabled: true, routes: vec![Route { track: 0, inputs: vec![], output: Some(Endpoint { name: "Unavailable synthesizer".into(), id: Some("999:0".into()) }), output_channel: Some(3), monitor: false, thru: false, filter: Filter::default() }] };
    let metadata = template::Metadata { schema: template::VERSION, name: "External synth".into(), kind: template::Kind::Track { bus: gui.fixture.rt.session.scenes[0].name.clone() }, hardware };
    let target = template::Target::capture(&gui.fixture.rt.session, 2).unwrap();
    gui.fixture.app.review_template_hardware(&metadata, Some(target)); gui.frame(vec![]);
    assert!(gui.fixture.app.settings.open); assert_eq!(gui.fixture.app.settings.applied, original);
    assert_eq!(gui.fixture.app.settings.draft.current().unwrap().midi_routing.routes[0].track, 2);
    gui.click("Cancel changes"); assert_eq!(gui.fixture.app.settings.applied, original); assert_eq!(gui.fixture.app.settings.draft, original);
    gui.fixture.rt.session.namespace[1] += 1; gui.fixture.rt.publish_for_test(); gui.frame(vec![]);
    gui.fixture.app.review_template_hardware(&metadata, Some(target));
    assert_eq!(gui.fixture.app.settings.draft, original); assert!(gui.fixture.app.templates.error.as_deref().unwrap().contains("replaced or deleted"));
}

#[test]
fn display_controls_apply_persist_reopen_and_cancel_without_audio_changes() {
    let mut gui = Gui::new(); gui.open();
    let audio = gui.fixture.app.settings.profile().audio.clone();
    let routing = gui.fixture.app.settings.profile().midi_routing.clone();
    gui.click("High contrast light"); gui.click("Reduce decorative motion");
    let display = &mut gui.fixture.app.settings.draft.profiles.get_mut("Studio").unwrap().appearance;
    display.waveform_contrast = 2.5; display.level_contrast = 3.0;
    gui.preview_apply();
    assert_eq!(gui.fixture.app.theme.contrast, crate::theme::Contrast::Light);
    assert!(gui.fixture.app.theme.reduced_motion);
    assert_eq!(gui.ctx.style().animation_time, 0.0);
    assert_eq!(gui.fixture.app.settings.profile().audio, audio);
    assert_eq!(gui.fixture.app.settings.profile().midi_routing, routing);
    assert!(!gui.fixture.app.settings.pending_restart());
    let bytes = std::fs::read(gui.dir.join("preferences.json")).unwrap();
    let startup = Startup::read(gui.dir.join("preferences.json"), gui.dir.clone());
    let mut reopened = Fixture::new(80);
    reopened.app.initialize_preferences(&gui.ctx, startup, audio.clone());
    assert_eq!(reopened.app.theme, gui.fixture.app.theme);
    gui.click("High contrast dark"); gui.click("Cancel changes");
    assert_eq!(std::fs::read(gui.dir.join("preferences.json")).unwrap(), bytes);
    assert_eq!(gui.fixture.app.theme.contrast, crate::theme::Contrast::Light);
    gui.open();
    gui.click("High contrast dark");
    gui.click("Preview changes"); gui.wait();
    let mut externally_changed = bytes.clone(); externally_changed.push(b'\n');
    std::fs::write(gui.dir.join("preferences.json"), &externally_changed).unwrap();
    gui.click("Apply and save"); gui.wait();
    assert_eq!(std::fs::read(gui.dir.join("preferences.json")).unwrap(), externally_changed);
    assert_eq!(gui.fixture.app.theme.contrast, crate::theme::Contrast::Light);
    assert_eq!(gui.fixture.app.settings.profile().audio, audio);
}

#[test]
fn light_contrast_keeps_actual_pending_audio_warning_readable() {
    let mut gui = Gui::new(); gui.open();
    let profile = gui.fixture.app.settings.applied.profiles.get_mut("Studio").unwrap();
    profile.appearance.contrast = crate::theme::Contrast::Light;
    profile.audio.sample_rate = Some(96000);
    gui.fixture.app.apply_appearance(&gui.ctx);
    assert!(gui.fixture.app.settings.pending_restart());
    let output = gui.frame(vec![]);
    let color = output.shapes.iter().find_map(|shape| match &shape.shape {
        egui::Shape::Text(text) if text.galley.text().starts_with("Saved audio differs") => Some(text.galley.job.sections[0].format.color),
        _ => None,
    }).expect("pending audio warning is actually painted");
    assert_eq!(color, gui.fixture.app.theme.yellow);
    assert!(crate::theme::contrast_ratio(color,gui.fixture.app.theme.bg)>=4.5);
}

#[test]
fn osc_preferences_apply_reopen_cancel_and_conflict_preserve_the_actual_listener() {
    let mut gui = Gui::new();
    gui.open();
    gui.click("Enable loopback OSC");
    let target = gui.node("OSC port (0 = automatic)");
    gui.frame(vec![egui::Event::AccessKitActionRequest(ActionRequest {
        target,
        action: Action::SetValue,
        data: Some(egui::accesskit::ActionData::NumericValue(0.0)),
    })]);
    gui.preview_apply();
    let config = crate::automation::osc::Config {
        enabled: true,
        port: 0,
    };
    let deadline = Instant::now() + std::time::Duration::from_secs(3);
    let applied = loop {
        gui.frame(vec![]);
        let status = gui.fixture.app.automation_network.status();
        if status.applied == Some(config) && !status.pending {
            break status;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(std::time::Duration::from_millis(1));
    };
    assert!(applied.port.is_some());
    assert!(applied.error.is_none());
    let path = gui.dir.join("preferences.json");
    let bytes = std::fs::read(&path).unwrap();
    let saved = storage::load(&path, &AtomicBool::new(false)).unwrap();
    assert_eq!(saved.preferences.current().unwrap().automation, config);
    assert!(!String::from_utf8_lossy(&bytes).contains(applied.token.as_ref().unwrap()));
    gui.click("Enable loopback OSC");
    gui.click("Cancel changes");
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    assert_eq!(
        gui.fixture.app.automation_network.status().port,
        applied.port
    );
    assert_eq!(
        gui.fixture.app.automation_network.status().token,
        applied.token
    );
    let mut reopened = Gui::new();
    reopened.fixture.app.initialize_preferences(
        &reopened.ctx,
        Startup::read(path.clone(), gui.dir.clone()),
        model::Audio::default(),
    );
    let deadline = Instant::now() + std::time::Duration::from_secs(3);
    let restarted = loop {
        reopened.frame(vec![]);
        let status = reopened.fixture.app.automation_network.status();
        if status.applied == Some(config) && !status.pending {
            break status;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(std::time::Duration::from_millis(1));
    };
    assert_ne!(restarted.token, applied.token);
    gui.open();
    gui.click("Enable loopback OSC");
    gui.click("Preview changes");
    gui.wait();
    let mut external = bytes.clone();
    external.push(b'\n');
    std::fs::write(&path, &external).unwrap();
    gui.click("Apply and save");
    gui.wait();
    assert!(gui.fixture.app.settings.message.contains("changed on disk"));
    assert_eq!(std::fs::read(&path).unwrap(), external);
    assert_eq!(gui.fixture.app.settings.profile().automation, config);
    assert_eq!(
        gui.fixture.app.automation_network.status().token,
        applied.token
    );
}

#[test]
fn native_shortcut_capture_cancel_persist_export_import_and_failure_preserve_other_settings() {
    let mut gui = Gui::new();
    gui.open();
    gui.click("Keyboard shortcuts");
    gui.click_in("Play / stop session", "Capture key");
    let before = gui.fixture.app.settings.applied.clone();
    let key = |pressed| egui::Event::Key { key:Key::G,physical_key:Some(Key::Q),pressed,repeat:false,modifiers:Default::default() };
    gui.frame(vec![key(true)]); gui.frame(vec![key(false)]);
    assert_eq!(gui.fixture.app.settings.draft.current().unwrap().shortcuts["transport"].as_ref().unwrap().key,"G");
    assert_eq!(gui.fixture.app.settings.applied,before);
    gui.click_in("Play / stop session", "Capture key");
    let escape = |pressed| egui::Event::Key { key:Key::Escape,physical_key:None,pressed,repeat:false,modifiers:Default::default() };
    gui.frame(vec![escape(true)]); gui.frame(vec![escape(false)]);
    assert_eq!(gui.fixture.app.settings.draft.current().unwrap().shortcuts["transport"].as_ref().unwrap().key,"G");
    gui.click_in("Play / stop session", "Capture key");
    let collision = |pressed| egui::Event::Key { key:Key::Q,physical_key:Some(Key::A),pressed,repeat:false,modifiers:Default::default() };
    gui.frame(vec![collision(true)]); gui.frame(vec![collision(false)]);
    gui.click("Preview changes"); gui.wait();
    assert!(gui.fixture.app.settings.message.contains("both use"));
    assert_eq!(gui.fixture.app.settings.applied,before);
    gui.click("Cancel changes");
    assert_eq!(gui.fixture.app.settings.draft,before);
    gui.open();
    gui.click_in("Play / stop session", "Capture key");
    gui.frame(vec![key(true)]); gui.frame(vec![key(false)]);
    gui.preview_apply();
    let saved = storage::load(&gui.dir.join("preferences.json"),&AtomicBool::new(false)).unwrap();
    assert_eq!(saved.preferences.current().unwrap().shortcuts["transport"].as_ref().unwrap().key,"G");
    assert_eq!(saved.preferences.current().unwrap().audio,before.current().unwrap().audio);
    assert_eq!(saved.preferences.current().unwrap().library_roots,before.current().unwrap().library_roots);
    let mut reopened = Fixture::new(80);
    let startup = Startup::read(gui.dir.join("preferences.json"),gui.dir.clone());
    let context = egui::Context::default();
    reopened.app.initialize_preferences(&context,startup,saved.preferences.current().unwrap().audio.clone());
    let _ = context.run(egui::RawInput { screen_rect:Some(Rect::from_min_size(Pos2::ZERO,Vec2::new(1600.0,1200.0))),events:vec![key(true)],..Default::default() },|ctx| reopened.app.update_frame(ctx));
    reopened.rt.process(&mut []);
    assert!(reopened.rt.playing,"reopened App must dispatch the persisted logical G binding");
    let export = gui.dir.join("bindings.json");
    gui.fixture.app.settings.roots.push_str("\nunrelated-relative-draft");
    gui.frame(vec![]);
    assert!(gui.fixture.app.settings.draft.current().unwrap().validate().is_err());
    gui.fixture.app.settings.file_path = export.to_string_lossy().into();
    gui.click("Export bindings only"); gui.wait();
    let bytes = std::fs::read(&export).unwrap();
    let bundle = model::shortcuts::Bundle::decode(&bytes).unwrap();
    gui.click("Export bindings only"); gui.wait();
    assert!(gui.fixture.app.settings.message.contains("Destination exists"));
    assert_eq!(std::fs::read(&export).unwrap(),bytes);
    gui.click("Reset all shortcut bindings");
    assert!(gui.fixture.app.settings.draft.current().unwrap().shortcuts.is_empty());
    gui.click("Import bindings into draft"); gui.wait();
    let mut expected = before.current().unwrap().clone(); bundle.apply(&mut expected).unwrap();
    expected.library_roots.push("unrelated-relative-draft".into());
    assert_eq!(gui.fixture.app.settings.draft.current().unwrap(),&expected);
    gui.click("Cancel changes");
    assert_eq!(gui.fixture.app.settings.draft,gui.fixture.app.settings.applied);
    gui.open();
    gui.fixture.app.settings.file_path = export.to_string_lossy().into();
    gui.fixture.app.settings.worker.as_ref().unwrap().delay.store(100,Ordering::Release);
    gui.click("Import bindings into draft");
    gui.click("Cancel pending preferences operation"); gui.wait();
    assert!(gui.fixture.app.settings.message.contains("Cancelled"));
    assert_eq!(gui.fixture.app.settings.draft,gui.fixture.app.settings.applied);
    gui.fixture.app.settings.worker.as_ref().unwrap().delay.store(0,Ordering::Release);
    std::fs::write(&export,b"{\"version\":999}").unwrap();
    let prior = gui.fixture.app.settings.draft.clone();
    gui.click("Import bindings into draft"); gui.wait();
    assert!(gui.fixture.app.settings.message.contains("failed"));
    assert_eq!(gui.fixture.app.settings.draft,prior);
    assert_eq!(std::fs::read(&export).unwrap(),b"{\"version\":999}");
}

#[test]
fn native_language_preview_cancel_apply_reopen_and_translated_layout_preserve_music() {
    use crate::localization::Locale;
    for (locale, name, menu, cancel) in [(Locale::Spanish,"Español","Proyecto","Cancelar cambios"),(Locale::German,"Deutsch","Projekt","Änderungen verwerfen")] {
        let mut gui = Gui::new(); gui.open();
        let musical = (gui.fixture.rt.bpm, gui.fixture.rt.master, gui.fixture.rt.quant, gui.fixture.rt.xfader);
        gui.click(name);
        assert_eq!(gui.fixture.app.settings.profile().appearance.locale, Locale::English);
        gui.click("Cancel changes");
        assert_eq!(gui.fixture.app.settings.profile().appearance.locale, Locale::English);
        gui.open(); gui.click(name); gui.preview_apply();
        assert_eq!(gui.fixture.app.settings.profile().appearance.locale, locale);
        assert_eq!((gui.fixture.rt.bpm, gui.fixture.rt.master, gui.fixture.rt.quant, gui.fixture.rt.xfader), musical);
        assert!(gui.nodes.iter().any(|(_,n)| n.label()==Some(menu)));
        assert!(gui.nodes.iter().any(|(_,n)| n.label()==Some(cancel)));
        let loaded=storage::load(&gui.dir.join("preferences.json"),&AtomicBool::new(false)).unwrap();
        assert_eq!(loaded.preferences.current().unwrap().appearance.locale,locale);
        let startup=Startup::read(gui.dir.join("preferences.json"),gui.dir.clone());
        let mut reopened=Fixture::new(80);reopened.app.initialize_preferences(&gui.ctx,startup,loaded.preferences.current().unwrap().audio.clone());
        assert_eq!(reopened.app.settings.profile().appearance.locale,locale);
        let out=gui.frame(vec![]);
        for shape in &out.shapes {
            if let egui::Shape::Text(text)=&shape.shape { assert!(text.galley.rect.width().is_finite());assert!(text.galley.rect.height().is_finite()); }
        }
        gui.click(cancel);
    }
}


#[test]
fn real_workspace_controls_reorder_hide_resize_detach_persist_and_reopen_unicode_names() {
    use crate::preferences::workspaces::Panel;
    let mut gui=Gui::new(); gui.height=5000.0; gui.frame(vec![]); gui.open();
    gui.click("Configure panel layout");
    gui.routing_text("New workspace name","Mix 窗"); gui.click("Copy workspace");
    gui.click("Move Sampler up"); gui.click("Show Library");
    gui.click("Automatic Sampler height"); gui.click("Separate Sampler window");
    let expected=gui.fixture.app.settings.draft.current().unwrap().workspaces.clone();
    assert_eq!(expected.active,"Mix 窗");
    let layout=expected.current().unwrap();
    assert_eq!(layout.panels[0].panel,Panel::Sampler);
    assert_eq!(layout.panels[0].height,320.0); assert!(layout.panels[0].detached);
    assert!(!layout.panels.iter().find(|entry|entry.panel==Panel::Library).unwrap().visible);
    gui.preview_apply();
    assert_eq!(gui.fixture.app.settings.profile().workspaces,expected);
    let loaded=storage::load(&gui.dir.join("preferences.json"),&AtomicBool::new(false)).unwrap();
    assert_eq!(loaded.preferences.current().unwrap().workspaces,expected);
    let mut reopened=Fixture::new(80);
    reopened.app.initialize_preferences(&gui.ctx,Startup::read(gui.dir.join("preferences.json"),gui.dir.clone()),loaded.preferences.current().unwrap().audio.clone());
    assert_eq!(reopened.app.settings.profile().workspaces,expected);
    gui.click("Cancel changes"); gui.open(); gui.click("Reset workspace layouts"); gui.click("Cancel changes");
    assert_eq!(gui.fixture.app.settings.profile().workspaces,expected);
}

#[test]
fn workspace_preview_cancel_and_invalid_layout_keep_saved_and_applied_state() {
    let mut gui=Gui::new(); gui.height=5000.0; gui.frame(vec![]); gui.open(); gui.preview_apply();
    let bytes=std::fs::read(gui.dir.join("preferences.json")).unwrap();
    let previous=gui.fixture.app.settings.applied.clone();
    gui.click("Configure panel layout"); gui.click("Show Library");
    gui.fixture.app.settings.worker.as_ref().unwrap().delay.store(100,Ordering::Release);
    gui.click("Preview changes"); gui.click("Cancel pending preferences operation"); gui.wait();
    assert_eq!(gui.fixture.app.settings.applied,previous);
    assert_eq!(std::fs::read(gui.dir.join("preferences.json")).unwrap(),bytes);
    gui.fixture.app.settings.worker.as_ref().unwrap().delay.store(0,Ordering::Release);
    for label in ["Show Decks","Show Sampler","Show Session and mixer"] {gui.click(label);}
    gui.click("Preview changes"); gui.wait();
    assert!(gui.fixture.app.settings.preview.is_none());
    assert!(gui.fixture.app.settings.message.contains("Keep at least one workspace panel visible"));
    assert_eq!(gui.fixture.app.settings.applied,previous);
    assert_eq!(std::fs::read(gui.dir.join("preferences.json")).unwrap(),bytes);
    gui.click("Cancel changes");
    assert_eq!(gui.fixture.app.settings.draft,previous);
}
