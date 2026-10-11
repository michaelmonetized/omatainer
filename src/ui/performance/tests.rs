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
        if !self.nodes.iter().any(|(_, node)| node.label() == Some(label) && node.supports_action(Action::Click)) {
            let status = self.fixture.app.engine.cmd.performance().status();
            let menu = if status.output_muted { "Output muted" } else if status.recovery { "Controls paused" } else { "Performance" };
            if label != menu { self.click_once(menu); self.frame(vec![]); }
        }
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
    fn custom(&mut self, label: &str, action: i32) {
        let target = self.nodes.iter().find_map(|(id, node)| (node.label() == Some(label)).then_some(*id)).unwrap();
        self.frame(vec![egui::Event::AccessKitActionRequest(ActionRequest {
            target, action: Action::CustomAction, data: Some(egui::accesskit::ActionData::CustomAction(action)),
        })]);
        self.frame(vec![]);
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
        gui.click(translated("Turn on performance mode"));
        gui.click(translated("Turn off performance mode…"));
        assert_ne!(translated("Turn off performance mode? Playing decks and device settings can be changed again. Muted output stays muted."), "Turn off performance mode? Playing decks and device settings can be changed again. Muted output stays muted.");
        assert!(gui.has(translated("Turn off performance mode? Playing decks and device settings can be changed again. Muted output stays muted.")));
        gui.click(translated("Cancel"));
        assert!(gui.fixture.app.engine.cmd.performance().status().protected);
        gui.click(translated("Turn off performance mode…"));
        gui.click(translated("Turn off performance mode"));
        assert!(!gui.fixture.app.engine.cmd.performance().status().protected);
        gui.click(translated("Stop everything…"));
        assert_ne!(translated("Stop both decks and the session, finish recording and release held notes. Effect tails may continue. Release your controls before using them again."), "Stop both decks and the session, finish recording and release held notes. Effect tails may continue. Release your controls before using them again.");
        assert!(gui.has(translated("Stop both decks and the session, finish recording and release held notes. Effect tails may continue. Release your controls before using them again.")));
        gui.click(translated("Stop everything"));
        assert!(gui.fixture.app.engine.cmd.performance().status().recovery);
        gui.frame(vec![]);
        gui.click(translated("Resume controls…"));
        assert_ne!(translated("Release all keys, pads and platter touches, then resume controls. Playback stays stopped. Muted output stays muted."), "Release all keys, pads and platter touches, then resume controls. Playback stays stopped. Muted output stays muted.");
        assert!(gui.has(translated("Release all keys, pads and platter touches, then resume controls. Playback stays stopped. Muted output stays muted.")));
        let label = translated("Resume controls");
        assert!(gui.nodes.iter().find(|(_,node)| node.label()==Some(label)).unwrap().1.is_disabled());
        gui.click(translated("All controls are released"));
        gui.click(label);
        assert!(!gui.fixture.app.engine.cmd.performance().status().recovery);
        assert!(!gui.fixture.rt.playing);
        gui.click(translated("Mute output…"));
        assert_ne!(translated("Stop playback and recording, release held notes and mute output. Reset audio when you are ready to unmute."), "Stop playback and recording, release held notes and mute output. Reset audio when you are ready to unmute.");
        assert!(gui.has(translated("Stop playback and recording, release held notes and mute output. Reset audio when you are ready to unmute.")));
        gui.click(translated("Cancel"));
        assert!(!gui.fixture.app.engine.cmd.performance().status().output_muted);
        gui.click(translated("Mute output…"));
        gui.click(translated("Mute output"));
        gui.frame(vec![]);
        assert!(gui.fixture.app.engine.cmd.performance().status().output_muted);
        gui.click(translated("Resume controls…"));
        let label = translated("Resume controls");
        assert!(gui.nodes.iter().find(|(_,node)| node.label()==Some(label)).unwrap().1.is_disabled());
        gui.click(translated("All controls are released"));
        gui.click(label);
        let status = gui.fixture.app.engine.cmd.performance().status();
        assert!(!status.recovery && status.output_muted);
        assert!(!gui.fixture.rt.playing);
    }
}

#[test]
fn actual_safety_controls_require_deliberate_decisions_and_keep_emergency_mute() {
    let mut gui = Gui::new();
    gui.click("Turn on performance mode");
    assert!(gui.fixture.app.engine.cmd.performance().status().protected);
    gui.click("Mute output…");
    assert!(!gui.fixture.app.engine.cmd.performance().status().recovery);
    gui.click("Cancel");
    assert!(!gui.fixture.app.engine.cmd.performance().status().recovery);
    gui.click("Mute output…");
    gui.click("Mute output");
    let status = gui.fixture.app.engine.cmd.performance().status();
    assert!(status.recovery && status.stopped && status.output_muted);
    gui.frame(vec![]); // let egui resize the newly expanded safety panel
    assert!(gui.has("Output muted"));
    gui.click("Resume controls…");
    let recovery = gui
        .nodes
        .iter()
        .find(|(_, node)| node.label() == Some("Resume controls"))
        .unwrap();
    assert!(recovery.1.is_disabled());
    gui.click("All controls are released");
    gui.click("Resume controls");
    let status = gui.fixture.app.engine.cmd.performance().status();
    assert!(!status.recovery && status.output_muted && status.protected);
    gui.click("Turn off performance mode…");
    assert!(gui.fixture.app.engine.cmd.performance().status().protected);
    gui.click("Turn off performance mode");
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
    gui.click("Turn on performance mode");
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
                sample: crate::engine::dsp::Sample { spectrum: None,
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
    gui.click("Turn on performance mode");
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
    gui.click("Turn on performance mode");
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
    gui.click("Deck A: Lock playing deck");
    let original = gui.fixture.rt.decks[0].audio.clone().unwrap();
    let other = gui.fixture.rt.decks[1].audio.clone().unwrap();
    gui.click("Load selected crate item to deck A");
    gui.click("Keep playing track");
    assert!(Arc::ptr_eq(gui.fixture.rt.decks[0].audio.as_ref().unwrap(), &original));
    assert!(gui.fixture.rt.decks[0].playing);
    gui.click("Load selected crate item to deck A");
    gui.click("I intend to replace this playing track");
    gui.click_once("Confirm deck replacement");
    assert!(Arc::ptr_eq(gui.fixture.rt.decks[0].audio.as_ref().unwrap(), &original));
    gui.frame(vec![]); gui.frame(vec![]);
    assert!(!Arc::ptr_eq(gui.fixture.rt.decks[0].audio.as_ref().unwrap(), &original));
    assert!(matches!(gui.fixture.app.loads[0].as_ref().unwrap().phase, Phase::Loaded));
    gui.fixture.rt.apply(Command::DeckPlay { deck: 0 }); gui.frame(vec![]);
    gui.custom("Deck A: Platter play or pause", 2);
    gui.click("Keep playing track");
    assert!(gui.fixture.rt.decks[0].audio.is_some());
    gui.custom("Deck A: Platter play or pause", 2);
    gui.click("I intend to replace this playing track");
    gui.click_once("Confirm deck replacement");
    assert!(gui.fixture.rt.decks[0].audio.is_some());
    gui.frame(vec![]); gui.frame(vec![]);
    assert!(gui.fixture.rt.decks[0].audio.is_none());
    assert!(Arc::ptr_eq(gui.fixture.rt.decks[1].audio.as_ref().unwrap(), &other));
    assert!(gui.fixture.app.status.contains("unloaded"));
}

#[test]
fn a_reviewed_delayed_decode_keeps_audio_until_ready_and_refuses_a_changed_target() {
    let mut gui = Gui::new();
    gui.fixture.rt.apply(Command::DeckPlay { deck: 0 }); gui.frame(vec![]);
    gui.click("Deck A: Lock playing deck"); gui.frame(vec![]);
    let original = gui.fixture.rt.decks[0].audio.clone().unwrap();
    let handle = gui.fixture.app.engine.cmd.performance().clone();
    let approval = handle.approve_deck_load(0, gui.fixture.app.snap.decks[0].load_gate_word).unwrap();
    gui.fixture.app.load_reference_approved(0, LibSource::File(PathBuf::from("/private/reviewed.wav")), "reviewed", Some(approval), None);
    gui.fixture.decoder_jobs.recv_timeout(std::time::Duration::from_secs(2)).unwrap();
    gui.frame(vec![]);
    assert!(gui.fixture.rt.decks[0].playing);
    assert!(Arc::ptr_eq(gui.fixture.rt.decks[0].audio.as_ref().unwrap(), &original));
    handle.set_deck_load_lock(0, false).unwrap();
    gui.fixture.rt.apply(Command::LoadBuiltin { deck: 0, stem: 1 });
    let replacement = gui.fixture.rt.decks[0].audio.clone().unwrap();
    handle.set_deck_load_lock(0, true).unwrap();
    gui.fixture.decoder_results.send((0, Ok(crate::engine::decode::DecodedAudio { sample: crate::engine::dsp::Sample { spectrum: None,
        name: "reviewed".into(), path: String::new(), sr: 48000, ch: 2, data: vec![0.25; 4096],
        peaks: Arc::new(Vec::new()), bpm: 120.0 }, diagnostics: Default::default() }))).unwrap();
    gui.fixture.poll_loads(); gui.frame(vec![]);
    assert!(Arc::ptr_eq(gui.fixture.rt.decks[0].audio.as_ref().unwrap(), &replacement));
    assert!(matches!(gui.fixture.app.loads[0].as_ref().unwrap().phase, Phase::Failed(_)));
}

#[test]
fn studio_deck_lock_reviews_keyboard_and_drop_and_rejects_midi_before_decoding() {
    let mut gui = Gui::new();
    gui.fixture.rt.apply(Command::DeckPlay { deck: 0 }); gui.frame(vec![]);
    gui.click("Deck A: Lock playing deck");
    assert!(!gui.fixture.app.engine.cmd.performance().protected());
    let original = gui.fixture.rt.decks[0].audio.clone().unwrap();
    let position = gui.fixture.rt.decks[0].pos;
    gui.click("Load selected crate item to deck A");
    assert!(gui.fixture.app.deck_load_review.is_some());
    assert!(gui.fixture.decoder_jobs.try_recv().is_err());
    gui.click("Keep playing track");
    gui.frame(vec![]);
    assert!(!keyboard::dialogs_block_input(&gui.ctx));
    gui.frame(vec![egui::Event::Key { key: Key::F, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() }]);
    assert!(gui.fixture.app.deck_load_review.is_some());
    assert!(gui.fixture.decoder_jobs.try_recv().is_err());
    gui.frame(vec![egui::Event::Key { key: Key::F, physical_key: None, pressed: false, repeat: false, modifiers: Default::default() }]);
    gui.click("Keep playing track");
    gui.frame_input(vec![], vec![egui::DroppedFile { path: Some(PathBuf::from("/private/locked-drop.wav")), ..Default::default() }]);
    assert!(gui.fixture.app.deck_load_review.is_some());
    assert!(gui.fixture.decoder_jobs.try_recv().is_err());
    gui.click("Keep playing track");
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

#[test]
fn resting_workspace_keeps_recovery_actions_in_menus() {
    let gui = Gui::new();
    for label in ["Performance", "Tools", "FX", "Project"] { assert!(gui.has(label), "missing {label}"); }
    for label in ["Stop everything…", "Mute output…", "Resume controls…", "Journal current edits now", "Render CPU:"] {
        assert!(!gui.has(label), "routine workspace exposes {label}");
    }
}
