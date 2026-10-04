use super::*;
use crate::engine::load_receipt::State;
use egui::accesskit::{Action, ActionRequest, Node, NodeId};
struct Gui {
    fixture: test_support::Fixture,
    ctx: egui::Context,
    nodes: Vec<(NodeId, Node)>,
    painted: Vec<String>,
    time: f64,
}
impl Gui {
    fn new() -> Self {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut gui = Self {
            fixture: test_support::Fixture::new(256),
            ctx,
            nodes: Vec::new(),
            painted: Vec::new(),
            time: 0.0,
        };
        gui.frame(Vec::new());
        gui.frame(Vec::new());
        gui
    }
    fn frame(&mut self, events: Vec<egui::Event>) { self.frame_input(events, Vec::new()); }
    fn frame_input(&mut self, events: Vec<egui::Event>, dropped_files: Vec<egui::DroppedFile>) {
        self.fixture.rt.process(&mut [0.0; 256]);
        self.fixture.rt.publish_for_test();
        self.time += 0.03;
        let modifiers = events
            .iter()
            .find_map(|event| {
                if let egui::Event::Key { modifiers, .. } = event {
                    Some(*modifiers)
                } else {
                    None
                }
            })
            .unwrap_or_default();
        let output = self.ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1800.0, 1400.0))),
                time: Some(self.time),
                events,
                dropped_files,
                modifiers,
                ..Default::default()
            },
            |ctx| self.fixture.app.update_frame(ctx),
        );
        self.painted = output
            .shapes
            .iter()
            .filter_map(|shape| {
                if let egui::epaint::Shape::Text(text) = &shape.shape {
                    Some(text.galley.text().to_owned())
                } else {
                    None
                }
            })
            .collect();
        self.nodes = output.platform_output.accesskit_update.unwrap().nodes;
    }
    fn click(&mut self, label: &str) {
        self.click_once(label);
        self.frame(vec![]);
    }
    fn click_once(&mut self, label: &str) {
        let target = self
            .nodes
            .iter()
            .find_map(|(id, node)| {
                (node.label() == Some(label) && node.supports_action(Action::Click)).then_some(*id)
            })
            .unwrap_or_else(|| panic!("missing {label}"));
        self.frame(vec![egui::Event::AccessKitActionRequest(ActionRequest {
            target,
            action: Action::Click,
            data: None,
        })]);
    }
    fn has(&self, label: &str) -> bool {
        self.painted.iter().any(|text| text.contains(label))
            || self
                .nodes
                .iter()
                .any(|(_, node)| node.label().is_some_and(|text| text.contains(label)))
    }
}
#[test]
fn localized_safety_decisions_keep_cancel_acknowledgment_and_latched_mute() {
    use crate::localization::{self, Locale};
    for locale in [Locale::Spanish,Locale::German] {
        let mut gui = Gui::new();
        let active = gui.fixture.app.settings.applied.active.clone();
        gui.fixture.app.settings.applied.profiles.get_mut(&active).unwrap().appearance.locale = locale;
        gui.frame(vec![]);
        let translated = |key| { let _locale = localization::scope(locale); localization::text(key) };
        gui.click(translated("Enable performance mode"));
        gui.click(translated("Leave performance mode…"));
        assert_ne!(translated("Leave protection deliberately. Playing deck replacement, destructive edits and optional background work will become available. Emergency mute, if present, stays latched."), "Leave protection deliberately. Playing deck replacement, destructive edits and optional background work will become available. Emergency mute, if present, stays latched.");
        assert!(gui.has(translated("Leave protection deliberately. Playing deck replacement, destructive edits and optional background work will become available. Emergency mute, if present, stays latched.")));
        gui.click(translated("Keep current safety state"));
        assert!(gui.fixture.app.engine.cmd.performance().status().protected);
        gui.click(translated("Leave performance mode…"));
        gui.click(translated("Leave protection"));
        assert!(!gui.fixture.app.engine.cmd.performance().status().protected);
        gui.click(translated("Safe stop…"));
        assert_ne!(translated("Stop session and both decks, finalize recorded holds, disarm compose and release all input-owned synth gates. Finite hits and effect tails continue naturally. Recovery needs your explicit input-release acknowledgment."), "Stop session and both decks, finalize recorded holds, disarm compose and release all input-owned synth gates. Finite hits and effect tails continue naturally. Recovery needs your explicit input-release acknowledgment.");
        assert!(gui.has(translated("Stop session and both decks, finalize recorded holds, disarm compose and release all input-owned synth gates. Finite hits and effect tails continue naturally. Recovery needs your explicit input-release acknowledgment.")));
        gui.click(translated("Stop all transports and release notes"));
        assert!(gui.fixture.app.engine.cmd.performance().status().recovery);
        gui.frame(vec![]);
        gui.click(translated("Recover inputs…"));
        assert_ne!(translated("Release physical keys, pads and platter touch controls first. Your confirmation is a user report, not a hardware check. This only reopens controls after queued pre-stop work drains; playback stays stopped and emergency output mute stays latched."), "Release physical keys, pads and platter touch controls first. Your confirmation is a user report, not a hardware check. This only reopens controls after queued pre-stop work drains; playback stays stopped and emergency output mute stays latched.");
        assert!(gui.has(translated("Release physical keys, pads and platter touch controls first. Your confirmation is a user report, not a hardware check. This only reopens controls after queued pre-stop work drains; playback stays stopped and emergency output mute stays latched.")));
        let label = translated("Inputs released — keep playback stopped");
        assert!(gui.nodes.iter().find(|(_,node)| node.label()==Some(label)).unwrap().1.is_disabled());
        gui.click(translated("I have released the physical inputs"));
        gui.click(label);
        assert!(!gui.fixture.app.engine.cmd.performance().status().recovery);
        assert!(!gui.fixture.rt.playing);
        gui.click(translated("Emergency silence…"));
        assert_ne!(translated("Finalize captured holds, stop session and both decks, release all notes and ramp output to silence over 2 ms. Output stays muted until a deliberate stopped DSP reset. This does not claim the device or physical controls are healthy."), "Finalize captured holds, stop session and both decks, release all notes and ramp output to silence over 2 ms. Output stays muted until a deliberate stopped DSP reset. This does not claim the device or physical controls are healthy.");
        assert!(gui.has(translated("Finalize captured holds, stop session and both decks, release all notes and ramp output to silence over 2 ms. Output stays muted until a deliberate stopped DSP reset. This does not claim the device or physical controls are healthy.")));
        gui.click(translated("Keep current safety state"));
        assert!(!gui.fixture.app.engine.cmd.performance().status().output_muted);
        gui.click(translated("Emergency silence…"));
        gui.click(translated("Confirm emergency silence"));
        gui.frame(vec![]);
        assert!(gui.fixture.app.engine.cmd.performance().status().output_muted);
        gui.click(translated("Recover inputs…"));
        let label = translated("Inputs released — keep output muted");
        assert!(gui.nodes.iter().find(|(_,node)| node.label()==Some(label)).unwrap().1.is_disabled());
        gui.click(translated("I have released the physical inputs"));
        gui.click(label);
        let status = gui.fixture.app.engine.cmd.performance().status();
        assert!(!status.recovery && status.output_muted);
        assert!(!gui.fixture.rt.playing);
    }
}

#[test]
fn actual_safety_controls_require_deliberate_decisions_and_keep_emergency_mute() {
    let mut gui = Gui::new();
    gui.click("Enable performance mode");
    assert!(gui.fixture.app.engine.cmd.performance().status().protected);
    gui.click("Emergency silence…");
    assert!(!gui.fixture.app.engine.cmd.performance().status().recovery);
    gui.click("Keep current safety state");
    assert!(!gui.fixture.app.engine.cmd.performance().status().recovery);
    gui.click("Emergency silence…");
    gui.click("Confirm emergency silence");
    let status = gui.fixture.app.engine.cmd.performance().status();
    assert!(status.recovery && status.stopped && status.output_muted);
    gui.frame(vec![]); // let egui resize the newly expanded safety panel
    assert!(gui.has("EMERGENCY OUTPUT MUTE"));
    gui.click("Recover inputs…");
    let recovery = gui
        .nodes
        .iter()
        .find(|(_, node)| node.label() == Some("Inputs released — keep output muted"))
        .unwrap();
    assert!(recovery.1.is_disabled());
    gui.click("I have released the physical inputs");
    gui.click("Inputs released — keep output muted");
    let status = gui.fixture.app.engine.cmd.performance().status();
    assert!(!status.recovery && status.output_muted && status.protected);
    gui.click("Leave performance mode…");
    assert!(gui.fixture.app.engine.cmd.performance().status().protected);
    gui.click("Leave protection");
    assert!(!gui.fixture.app.engine.cmd.performance().status().protected);
    assert!(
        gui.fixture
            .app
            .engine
            .cmd
            .performance()
            .status()
            .output_muted
    );
}
#[test]
fn midi_load_and_late_decode_cannot_replace_playing_media_but_stopped_deck_load_remains_available()
{
    let mut gui = Gui::new();
    let original = gui.fixture.rt.decks[0].audio.clone().unwrap();
    gui.fixture
        .app
        .load_file(0, PathBuf::from("/private/delayed.wav"), "delayed");
    gui.fixture
        .decoder_jobs
        .recv_timeout(std::time::Duration::from_secs(2))
        .unwrap();
    gui.fixture
        .app
        .engine
        .send(Command::DeckPlay { deck: 0 })
        .unwrap();
    gui.click("Enable performance mode");
    gui.fixture.app.engine.midi.receive_for_test(
        &gui.fixture.app.engine.cmd,
        41,
        "Pioneer DDJ-FLX4",
        &[0x90, 0x02, 0x7f],
    );
    assert_eq!(gui.fixture.app.engine.cmd.ui_request_stats().pending, 0);
    gui.fixture
        .decoder_results
        .send((
            0,
            Ok(crate::engine::decode::DecodedAudio {
                sample: crate::engine::dsp::Sample {
                    name: "delayed".into(),
                    path: String::new(),
                    sr: 48000,
                    ch: 2,
                    data: vec![0.0; 1024],
                    peaks: Arc::new(Vec::new()),
                    bpm: 0.0,
                },
                diagnostics: Default::default(),
            }),
        ))
        .unwrap();
    gui.fixture.poll_loads();
    gui.frame(vec![]);
    assert!(Arc::ptr_eq(
        gui.fixture.rt.decks[0].audio.as_ref().unwrap(),
        &original
    ));
    assert!(matches!(
        gui.fixture.app.loads[0].as_ref().unwrap().phase,
        Phase::Failed(_)
    ));
    gui.fixture.app.load_sel(1);
    gui.frame(vec![]);
    gui.frame(vec![]);
    assert_eq!(
        gui.fixture.app.loads[1]
            .as_ref()
            .unwrap()
            .receipt
            .as_ref()
            .unwrap()
            .state(),
        State::Current
    );
    assert!(gui.fixture.decoder_jobs.try_recv().is_err());
}
#[test]
fn destructive_shortcuts_and_project_open_are_guarded_while_recording_and_save_capture_remain_available(
) {
    let mut gui = Gui::new();
    gui.click("Enable performance mode");
    let before = gui.fixture.app.engine.undo.checkpoint();
    let modifiers = egui::Modifiers {
        ctrl: true,
        ..Default::default()
    };
    gui.frame(vec![egui::Event::Key {
        key: Key::Z,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers,
    }]);
    assert_eq!(before, gui.fixture.app.engine.undo.checkpoint());
    gui.frame(vec![egui::Event::Key {
        key: Key::N,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers,
    }]);
    assert_eq!(before, gui.fixture.app.engine.undo.checkpoint());
    gui.fixture
        .app
        .engine
        .send(Command::ComposeArm { track: 2, scene: 7 })
        .unwrap();
    gui.fixture
        .app
        .engine
        .send(Command::SamplerPad { pad: 0, on: true })
        .unwrap();
    gui.frame(vec![]);
    assert!(gui.fixture.app.snap.tracks[2].clips[7].recording_held);
    gui.fixture
        .app
        .engine
        .send(Command::SamplerPad { pad: 0, on: false })
        .unwrap();
    gui.frame(vec![]);
    assert_eq!(gui.fixture.rt.tracks[2].clips[7].notes.len(), 1);
    let project = gui.fixture.app.engine.project.clone();
    let worker = std::thread::spawn(move || {
        project
            .capture(&std::sync::atomic::AtomicBool::new(false))
            .unwrap()
    });
    while !worker.is_finished() {
        gui.fixture.rt.process(&mut [0.0; 256]);
        std::thread::yield_now();
    }
    let saved = worker.join().unwrap();
    assert_eq!(saved.state.tracks[2].clips[7].notes.len(), 1);
}

#[test]
fn renderer_rejected_load_retires_its_playback_watch_without_adding_history() {
    let mut gui = Gui::new();
    gui.click("Enable performance mode");
    let watches = gui.fixture.app.playback_watches.len();
    let original = gui.fixture.rt.decks[0].audio.clone().unwrap();
    gui.fixture
        .app
        .engine
        .send(Command::DeckPlay { deck: 0 })
        .unwrap();
    // Producer still observes the stopped deck; the renderer sees the preceding
    // Play and rejects this accepted intent without replacing current media.
    gui.fixture.app.load_sel(0);
    let receipt = gui.fixture.app.loads[0]
        .as_ref()
        .unwrap()
        .receipt
        .clone()
        .unwrap();
    assert_eq!(receipt.state(), State::Pending);
    gui.frame(vec![]);
    assert_eq!(receipt.state(), State::Protected);
    assert!(receipt.last_play().is_none());
    assert!(!receipt.retained_by_history());
    assert_eq!(gui.fixture.app.playback_watches.len(), watches);
    assert!(Arc::ptr_eq(
        &original,
        gui.fixture.rt.decks[0].audio.as_ref().unwrap()
    ));
}

#[test]
fn native_deck_review_cancel_load_and_eject_keep_the_exact_target() {
    let mut gui = Gui::new();
    gui.fixture.rt.apply(Command::DeckPlay { deck: 0 });
    gui.fixture.app.lib_sel = 1;
    gui.frame(vec![]);
    gui.click("Lock playing deck A");
    let original = gui.fixture.rt.decks[0].audio.clone().unwrap();
    gui.click("Review load override…");
    gui.click("Keep current deck audio");
    assert!(Arc::ptr_eq(gui.fixture.rt.decks[0].audio.as_ref().unwrap(), &original));
    assert!(gui.fixture.rt.decks[0].playing);
    gui.click("Review load override…");
    gui.click_once("Confirm load on reviewed deck");
    assert!(Arc::ptr_eq(gui.fixture.rt.decks[0].audio.as_ref().unwrap(), &original));
    gui.frame(vec![]); gui.frame(vec![]);
    assert!(!Arc::ptr_eq(gui.fixture.rt.decks[0].audio.as_ref().unwrap(), &original));
    assert!(matches!(gui.fixture.app.loads[0].as_ref().unwrap().phase, Phase::Loaded));
    gui.fixture.rt.apply(Command::DeckPlay { deck: 0 }); gui.frame(vec![]);
    gui.click("Review eject…"); gui.click("Keep current deck audio");
    assert!(gui.fixture.rt.decks[0].audio.is_some());
    gui.click("Review eject…"); gui.click_once("Confirm eject on reviewed deck");
    assert!(gui.fixture.rt.decks[0].audio.is_some());
    gui.frame(vec![]); gui.frame(vec![]);
    assert!(gui.fixture.rt.decks[0].audio.is_none());
    assert!(gui.fixture.app.status.contains("ejected"));
}

#[test]
fn a_reviewed_delayed_decode_keeps_audio_until_ready_and_refuses_a_changed_target() {
    let mut gui = Gui::new();
    gui.fixture.rt.apply(Command::DeckPlay { deck: 0 }); gui.frame(vec![]);
    gui.click("Lock playing deck A"); gui.frame(vec![]);
    let original = gui.fixture.rt.decks[0].audio.clone().unwrap();
    let handle = gui.fixture.app.engine.cmd.performance().clone();
    let approval = handle.approve_deck_load(0, gui.fixture.app.snap.decks[0].load_gate_word).unwrap();
    gui.fixture.app.load_reference_approved(0, LibSource::File(PathBuf::from("/private/reviewed.wav")), "reviewed", Some(approval));
    gui.fixture.decoder_jobs.recv_timeout(std::time::Duration::from_secs(2)).unwrap();
    gui.frame(vec![]);
    assert!(gui.fixture.rt.decks[0].playing);
    assert!(Arc::ptr_eq(gui.fixture.rt.decks[0].audio.as_ref().unwrap(), &original));
    handle.set_deck_load_lock(0, false).unwrap();
    gui.fixture.rt.apply(Command::LoadBuiltin { deck: 0, stem: 1 });
    let replacement = gui.fixture.rt.decks[0].audio.clone().unwrap();
    handle.set_deck_load_lock(0, true).unwrap();
    gui.fixture.decoder_results.send((0, Ok(crate::engine::decode::DecodedAudio { sample: crate::engine::dsp::Sample {
        name: "reviewed".into(), path: String::new(), sr: 48000, ch: 2, data: vec![0.25; 4096],
        peaks: Arc::new(Vec::new()), bpm: 120.0 }, diagnostics: Default::default() }))).unwrap();
    gui.fixture.poll_loads(); gui.frame(vec![]);
    assert!(Arc::ptr_eq(gui.fixture.rt.decks[0].audio.as_ref().unwrap(), &replacement));
    assert!(matches!(gui.fixture.app.loads[0].as_ref().unwrap().phase, Phase::Failed(_)));
}

#[test]
fn studio_deck_lock_rejects_actual_keyboard_drop_and_midi_loads_before_decoding() {
    let mut gui = Gui::new();
    gui.fixture.rt.apply(Command::DeckPlay { deck: 0 }); gui.frame(vec![]);
    gui.click("Lock playing deck A");
    assert!(!gui.fixture.app.engine.cmd.performance().protected());
    let original = gui.fixture.rt.decks[0].audio.clone().unwrap();
    let position = gui.fixture.rt.decks[0].pos;
    let rejected = gui.fixture.app.engine.cmd.performance().status().rejected;
    gui.click("Load selected crate item to deck A");
    assert!(gui.fixture.app.engine.cmd.performance().status().rejected > rejected);
    gui.click("Dismiss");
    gui.frame(vec![]);
    assert!(!keyboard::dialogs_block_input(&gui.ctx));
    let rejected = gui.fixture.app.engine.cmd.performance().status().rejected;
    gui.frame(vec![egui::Event::Key { key: Key::F, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() }]);
    assert!(gui.fixture.app.engine.cmd.performance().status().rejected > rejected);
    gui.frame(vec![egui::Event::Key { key: Key::F, physical_key: None, pressed: false, repeat: false, modifiers: Default::default() }]);
    gui.click("Dismiss");
    let rejected = gui.fixture.app.engine.cmd.performance().status().rejected;
    gui.frame_input(vec![], vec![egui::DroppedFile { path: Some(PathBuf::from("/private/locked-drop.wav")), ..Default::default() }]);
    assert!(gui.fixture.app.engine.cmd.performance().status().rejected > rejected);
    gui.click("Dismiss");
    let rejected = gui.fixture.app.engine.cmd.performance().status().rejected;
    gui.fixture.app.engine.midi.receive_for_test(&gui.fixture.app.engine.cmd, 41, "Pioneer DDJ-FLX4", &[0x90,0x02,0x7f]);
    assert!(gui.fixture.app.engine.cmd.performance().status().rejected > rejected);
    assert_eq!(gui.fixture.app.engine.cmd.ui_request_stats().pending, 0);
    assert!(gui.fixture.decoder_jobs.try_recv().is_err());
    gui.frame(vec![]);
    assert!(gui.fixture.rt.decks[0].playing);
    assert!(gui.fixture.rt.decks[0].pos > position);
    assert!(Arc::ptr_eq(gui.fixture.rt.decks[0].audio.as_ref().unwrap(), &original));
}
