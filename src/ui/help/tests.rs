use super::*;
use crate::engine::RtEngine;
use egui::accesskit::{Action, ActionData, ActionRequest, Node, NodeId};

struct Gui {
    app: App,
    rt: RtEngine,
    ctx: egui::Context,
    time: f64,
    nodes: Vec<(NodeId, Node)>,
    painted: Vec<String>,
}
impl Gui {
    fn new() -> Self {
        let (engine, mut rt) = Engine::headless_for_test(48_000, 256);
        rt.publish_for_test();
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut g = Self {
            app: App::with_loader(engine, Theme::default(), None),
            rt,
            ctx,
            time: 0.0,
            nodes: Vec::new(),
            painted: Vec::new(),
        };
        g.frame(vec![]);
        g.frame(vec![]);
        g
    }
    fn frame(&mut self, events: Vec<egui::Event>) {
        self.rt.process(&mut [0.0; 256]);
        self.rt.publish_for_test();
        self.time += 0.03;
        let modifiers = events
            .iter()
            .find_map(|event| match event {
                egui::Event::Key { modifiers, .. } => Some(*modifiers),
                _ => None,
            })
            .unwrap_or_default();
        let out = self.ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1800.0, 1400.0))),
                time: Some(self.time),
                events,
                modifiers,
                ..Default::default()
            },
            |ctx| self.app.update_frame(ctx),
        );
        self.painted = out
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::epaint::Shape::Text(text) => Some(text.galley.text().to_owned()),
                _ => None,
            })
            .collect();
        self.nodes = out.platform_output.accesskit_update.unwrap().nodes;
    }
    fn node(&self, name: &str) -> &(NodeId, Node) {
        self.nodes
            .iter()
            .find(|(_, node)| node.label() == Some(name))
            .unwrap_or_else(|| {
                panic!(
                    "missing {name}; {:?}",
                    self.nodes
                        .iter()
                        .filter_map(|(_, node)| node.label())
                        .collect::<Vec<_>>()
                )
            })
    }
    fn action(&mut self, name: &str, action: Action, data: Option<ActionData>) {
        let target = self.node(name).0;
        self.frame(vec![egui::Event::AccessKitActionRequest(ActionRequest {
            action,
            target,
            data,
        })]);
        self.frame(vec![]);
    }
    fn click(&mut self, name: &str) {
        self.action(name, Action::Click, None);
    }
    fn key(&mut self, key: Key) {
        for pressed in [true, false] {
            self.frame(vec![egui::Event::Key {
                key,
                physical_key: Some(key),
                pressed,
                repeat: false,
                modifiers: Default::default(),
            }]);
        }
    }
    fn start(&mut self, topic: Topic) {
        if !self.app.keys_open {
            self.click("Help");
        }
        self.click(topic.title());
        self.click("Start this lesson");
        assert_eq!(self.app.help.lesson.as_ref().unwrap().topic, topic);
    }
    fn ready(&self) -> bool {
        self.app.help.lesson.as_ref().unwrap().ready
    }
    fn next(&mut self) {
        assert!(
            self.ready(),
            "not ready at step {}",
            self.app.help.lesson.as_ref().unwrap().step
        );
        let before = self.app.help.lesson.as_ref().unwrap().step;
        self.click("Next lesson step");
        assert_eq!(
            self.app.help.lesson.as_ref().unwrap().step,
            before + 1,
            "Next did not activate"
        );
    }
    fn done(&self) {
        assert!(self.app.help.lesson.as_ref().unwrap().complete());
    }
    fn menu(&mut self, action: &str) {
        self.click("Project");
        self.click(action);
    }
    fn enter_path(&mut self, path: &std::path::Path) {
        self.action("Project file path", Action::Focus, None);
        self.frame(vec![
            egui::Event::Key {
                key: Key::A,
                physical_key: Some(Key::A),
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::CTRL | egui::Modifiers::COMMAND,
            },
            egui::Event::Text(path.display().to_string()),
        ]);
        self.frame(vec![egui::Event::Key {
            key: Key::A,
            physical_key: Some(Key::A),
            pressed: false,
            repeat: false,
            modifiers: egui::Modifiers::CTRL | egui::Modifiers::COMMAND,
        }]);
        assert_eq!(
            self.node("Project file path").1.value(),
            Some(path.to_str().unwrap()),
            "path typing"
        );
    }
    fn settle_project(&mut self) {
        let deadline = Instant::now() + std::time::Duration::from_secs(5);
        while self.app.project_help_state().2 {
            self.frame(vec![]);
            assert!(Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        self.frame(vec![]);
    }
}

#[test]
fn focused_controls_have_units_context_and_f1_does_not_consume_text() {
    let mut g = Gui::new();
    for (name, expected) in [
        ("Deck A: Pitch", "±8"),
        ("Deck A: Bass EQ", "0–340%"),
        ("Track 1: Gain", "0–120%"),
        ("Sampler bank", "sixteen"),
        ("Load selected crate item to deck A", "Queued/decoding"),
        ("Master effect 1: Wet", "0–100%"),
        ("Search crate", "Text filter"),
    ] {
        let description = g.node(name).1.description().unwrap();
        assert!(description.contains(expected), "{name}: {description}");
    }
    let numeric = g.node("Deck A: Pitch").1.description().unwrap();
    assert!(
        numeric.contains("F2"),
        "existing keyboard instructions were lost"
    );
    g.action("Search crate", Action::Focus, None);
    g.key(Key::F1);
    assert!(g.app.keys_open);
    assert_eq!(g.app.help.context, Some(Control::CrateSearch));
    assert!(!g.rt.playing);
    g.frame(vec![egui::Event::Text("harmony".into())]);
    assert_eq!(g.app.lib_filter, "harmony");
    g.key(Key::F1);
    assert!(!g.app.keys_open);
}

#[test]
fn recording_lesson_observes_original_target_hold_release_and_real_launch() {
    let mut g = Gui::new();
    g.app.send(Command::Select { track: 4, scene: 3 });
    g.app.send(Command::SamplerInst(SamplerInstrument::Synth(
        crate::engine::SynthInstrument::Keys,
    )));
    g.frame(vec![]);
    g.start(Topic::Recording);
    assert!(!g.ready());
    g.click("Arm selected cell");
    g.next();
    let pad = "Sampler: Pad 1: A MIDI note 57";
    g.click(pad); // AccessKit click deliberately toggles a held pad.
    assert!(g.app.snap.tracks[4].clips[3].recording_held);
    assert_eq!(g.app.snap.tracks[4].clips[3].note_count, 1);
    g.next();
    assert!(!g.ready(), "held capture is not a released recording");
    g.app.send(Command::Select { track: 6, scene: 6 });
    g.frame(vec![]);
    g.click(pad);
    assert!(!g.app.snap.tracks[4].clips[3].recording_held);
    assert_eq!(g.rt.tracks[4].clips[3].notes.len(), 1);
    assert!(g.rt.tracks[6].clips[6].notes.is_empty());
    g.next();
    g.click("Disarm compose");
    g.next();
    let name = format!("Clip track 5 scene 4: {}", g.rt.tracks[4].clips[3].name);
    g.click(&name);
    g.next();
    g.done();
    let before = g.rt.tracks[4].clips[3].notes.len();
    g.click("Cancel lesson");
    assert!(g.app.help.lesson.is_none());
    assert_eq!(g.rt.tracks[4].clips[3].notes.len(), before);
    assert!(g.rt.playing, "cancelling the guide must not stop music");
}

#[test]
fn lesson_rejects_capacity_failure_and_project_identity_change() {
    let mut g = Gui::new();
    g.app.send(Command::Select { track: 4, scene: 3 });
    g.frame(vec![]);
    g.start(Topic::Recording);
    g.app.send(Command::ComposeArm { track: 4, scene: 3 });
    g.frame(vec![]);
    g.next();
    // The live voice remains playable when a recording write is refused. A
    // held gate alone must never certify a new note or completed recording.
    g.rt.tracks[4].clips[3].notes = vec![
        crate::engine::MidiNote {
            id: crate::engine::midi_edit::NoteId::new(), muted: false,
            pitch: 60,
            vel: 100,
            start: 0.0,
            len: 1.0
        };
        8192
    ];
    g.app.send(Command::SamplerInst(SamplerInstrument::Synth(
        crate::engine::SynthInstrument::Keys,
    )));
    g.app.send(Command::SamplerPad { pad: 0, on: true });
    g.frame(vec![]);
    assert!(!g.app.snap.tracks[4].clips[3].recording_held);
    assert!(!g.ready());
    g.app.send(Command::SamplerPad { pad: 0, on: false });
    g.frame(vec![]);
    // A different epoch cannot inherit the prior guide's captured destination.
    g.click("Project");
    g.click("New project");
    g.click("Discard changes");
    let deadline = Instant::now() + std::time::Duration::from_secs(5);
    while !g.app.help.lesson.as_ref().unwrap().invalidated {
        g.frame(vec![]);
        assert!(Instant::now() < deadline);
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    assert!(g.app.help.lesson.as_ref().unwrap().invalidated);
    assert!(!g.ready());
}

#[test]
fn dj_lesson_uses_current_receipt_and_rendered_playback_not_load_admission() {
    let mut g = Gui::new();
    g.start(Topic::Dj);
    g.app.load_file(
        0,
        PathBuf::from("/unavailable/no-decoder.wav"),
        "unavailable",
    );
    g.frame(vec![]);
    assert!(!g.ready());
    g.click("Load selected crate item to deck A");
    g.next();
    assert!(!g.ready(), "loaded media was not played yet");
    g.click("Deck A: Platter play or pause");
    g.next();
    // Ensure a real cue action changes the cue state from the start baseline.
    g.click("Deck A: Hot cue 1");
    g.next();
    g.click("Deck A: Platter play or pause");
    g.next();
    g.done();
}

#[test]
fn restored_cues_do_not_complete_a_new_cue_edit_and_replacement_invalidates_the_target() {
    let mut g = Gui::new();
    g.start(Topic::Dj);
    let mut prep = crate::engine::preparation::Preparation::default();
    prep.hotcues[0] = Some(0.2);
    let receipt = Receipt::with_preparation(Some(prep));
    let mut state = LoadState::new(None, Phase::Queued);
    state.receipt = Some(receipt.clone());
    g.app.loads[0] = Some(state);
    g.app.send(Command::DeckLoadRequested {
        deck: 0,
        media: Media::Builtin(BuiltinStem::Drums.index()),
        receipt,
    });
    // Current is atomic and may be visible while App still has the prior
    // media's snapshot. Observe and advance at exactly that boundary.
    g.rt.process(&mut [0.0; 256]);
    assert!(!g.app.snap.decks[0].hotcues[0]);
    let _ = g.ctx.run(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1800.0, 1400.0))),
            ..Default::default()
        },
        |ctx| g.app.help_panel(ctx),
    );
    assert!(g.ready());
    g.app.help.lesson.as_mut().unwrap().next();
    g.frame(vec![]);
    g.click("Deck A: Platter play or pause");
    g.next();
    assert!(g.rt.decks[0].hotcues[0].set);
    assert!(!g.ready(), "restored preparation is not a new cue edit");
    g.click("Load selected crate item to deck A");
    assert!(g.app.help.lesson.as_ref().unwrap().invalidated);
    assert!(!g.ready());
}

#[test]
fn mixing_and_emergency_guides_observe_separate_transport_and_deck_states() {
    let mut g = Gui::new();
    g.start(Topic::Mixing);
    assert!(!g.ready());
    g.click("Scene 1: Toggle playback");
    g.next();
    g.action(
        "Track 1: Gain",
        Action::SetValue,
        Some(ActionData::NumericValue(43.0)),
    );
    g.next();
    let track = format!("Track 1 {}: Mute", g.rt.tracks[0].name);
    g.click(&track);
    g.next();
    g.click(&track);
    g.next();
    g.done();
    g.start(Topic::Emergency);
    g.next();
    g.click("Deck A: Platter play or pause");
    g.app.send(Command::Stop);
    g.frame(vec![]);
    assert!(
        !g.ready(),
        "session stop does not pause the independent deck"
    );
    g.click("Deck A: Platter play or pause");
    g.next();
    assert!(!g.ready());
    g.click("Diagnostics");
    g.click("I personally verified this physical check");
    g.frame(vec![]);
    g.next();
    g.done();
}

#[test]
fn hardware_lessons_stay_unverified_without_audio_or_midi_evidence() {
    let mut g = Gui::new();
    g.start(Topic::Setup);
    g.click("Diagnostics");
    assert!(
        !g.ready(),
        "headless renderer is not a running physical output"
    );
    g.start(Topic::Controllers);
    g.click("MIDI");
    g.next();
    assert!(!g.ready());
    for _ in 0..5 {
        g.frame(vec![]);
    }
    assert!(
        !g.ready(),
        "opening/retrying a window is not handled device input"
    );
    g.click("Cancel lesson");
    assert!(g.app.help.lesson.is_none());
}

#[test]
fn editing_lesson_tracks_target_gain_and_actual_undo_redo_positions() {
    let mut g = Gui::new();
    g.start(Topic::Editing);
    let clip = format!("Clip track 1 scene 1: {}", g.rt.tracks[0].clips[0].name);
    g.action(
        &clip,
        Action::CustomAction,
        Some(ActionData::CustomAction(3)),
    );
    g.next();
    let original = g.rt.tracks[0].clips[0].gain;
    g.action(
        "Clip track 1 scene 1: Gain",
        Action::SetValue,
        Some(ActionData::NumericValue(53.0)),
    );
    g.next();
    assert!(!g.ready());
    // Real Edit menu buttons use the same renderer history as keyboard/MIDI.
    g.click("Edit");
    let undo = g
        .nodes
        .iter()
        .find_map(|(_, node)| {
            node.label()
                .filter(|label| label.starts_with("Undo Set clip gain"))
                .map(str::to_owned)
        })
        .expect("named undo menu");
    g.click(&undo);
    assert_eq!(
        g.rt.tracks[0].clips[0].gain,
        original,
        "cursor {}, items {:?}, editor {:?}, error {:?}",
        g.app.engine.undo.view().cursor,
        g.app
            .engine
            .undo
            .view()
            .items
            .iter()
            .flatten()
            .map(|i| i.label())
            .collect::<Vec<_>>(),
        g.app.clip_gain_edit.map(|e| e.value.to_bits()),
        g.app.submission_error.get()
    );
    g.next();
    g.click("Edit");
    let redo = g
        .nodes
        .iter()
        .find_map(|(_, node)| {
            node.label()
                .filter(|label| label.starts_with("Redo Set clip gain"))
                .map(str::to_owned)
        })
        .expect("named redo menu");
    g.click(&redo);
    g.next();
    g.done();
    assert_eq!(g.rt.tracks[0].clips[0].gain, 0.53);
}

#[test]
fn project_lesson_requires_real_atomic_save_new_reopen_and_survives_cancel_failure() {
    struct Files(PathBuf);
    impl Drop for Files {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let files = Files(std::env::temp_dir().join(format!(
            "omatainer-help-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        )));
    std::fs::create_dir(&files.0).unwrap();
    let path = files.0.join("Lesson with spaces.omat");
    let mut g = Gui::new();
    g.app.send(Command::ClipGain {
        track: 0,
        scene: 0,
        value: 0.37,
    });
    g.frame(vec![]);
    g.start(Topic::Projects);
    g.menu("Save project as…");
    g.click("Cancel");
    assert!(!g.ready());
    g.menu("Save project as…");
    g.enter_path(&files.0.join("absent/destination.omat"));
    g.click("Save");
    g.settle_project();
    assert!(!g.ready(), "failed publication cannot certify save");
    g.menu("Save project as…");
    g.enter_path(&path);
    g.click("Save");
    g.settle_project();
    assert!(path.exists());
    g.next();
    g.menu("New project");
    if g.nodes
        .iter()
        .any(|(_, n)| n.label() == Some("Discard changes"))
    {
        g.click("Discard changes");
    }
    g.settle_project();
    assert_eq!(g.rt.tracks[0].clips[0].notes.len(), 0);
    if !g.app.keys_open {
        g.click("Help");
    }
    g.next();
    g.menu("Open project…");
    if g.nodes
        .iter()
        .any(|(_, n)| n.label() == Some("Discard changes"))
    {
        g.click("Discard changes");
    }
    g.enter_path(&path);
    g.click("Open");
    g.settle_project();
    g.next();
    g.done();
    assert_eq!(g.rt.tracks[0].clips[0].gain, 0.37);
    assert!(!g.rt.playing && g.rt.decks.iter().all(|d| !d.playing));
    let mut opened = Gui::new();
    opened.start(Topic::Projects);
    opened.menu("Open project…");
    if opened
        .nodes
        .iter()
        .any(|(_, n)| n.label() == Some("Discard changes"))
    {
        opened.click("Discard changes");
    }
    opened.enter_path(&path);
    opened.click("Open");
    opened.settle_project();
    assert!(
        !opened.ready(),
        "opening an already saved file is not evidence of performing Save As"
    );
}

#[test]
fn effective_shortcuts_and_disabled_next_remain_accessibly_explained() {
    let mut g = Gui::new();
    g.app
        .settings
        .applied
        .profiles
        .get_mut("Studio")
        .unwrap()
        .shortcuts
        .insert(
            "play_a".into(),
            Some(crate::preferences::Shortcut {
                key: "F6".into(),
                ctrl: false,
                shift: false,
                alt: false,
            }),
        );
    g.start(Topic::Setup);
    let next = &g.node("Next lesson step").1;
    assert!(next.is_disabled());
    assert!(next
        .description()
        .unwrap()
        .contains("Queue acceptance is insufficient"));
    g.click("Active shortcuts and accessible input");
    for _ in 0..6 {
        g.frame(vec![]);
    }
    g.action("Help content", Action::Focus, None);
    for _ in 0..4 {
        if g.painted.iter().any(|label| label.contains("F6")) {
            return;
        }
        g.key(Key::PageDown);
    }
    panic!(
        "effective shortcut not reachable through accessible help scroll: {:?}",
        g.painted
    );
}

#[test]
fn topic_titles_remain_short_and_emergency_instructions_are_in_the_body() {
    for topic in Topic::ALL { assert!(topic.title().chars().count() <= 40, "{}", topic.title()); }
    assert_eq!(Topic::Emergency.title(), "Stop and recover");
    assert!(Topic::Emergency.text().starts_with("Enable performance mode"));
    assert!(Topic::Emergency.text().contains("Emergency mute remains"));
}

#[test]
fn manual_and_catalogue_are_synchronized() {
    let generated = catalogue::manual();
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/manual.md");
    assert!(std::fs::read_to_string(path).unwrap() == generated, "Regenerate docs/manual.md with the ignored regenerate_offline_manual test after catalogue/binding changes");
    for control in Control::ALL {
        let d = control.definition();
        assert!(!d.title.is_empty() && !d.units.is_empty() && !d.purpose.is_empty());
        assert!(generated.contains(d.title));
    }
}

#[test]
#[ignore = "Maintainer command: regenerate the checked offline manual after catalogue/binding edits"]
fn regenerate_offline_manual() {
    std::fs::write(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/manual.md"),
        catalogue::manual(),
    )
    .unwrap();
}

#[test]
fn disabled_help_is_visible_and_text_paint_does_not_claim_an_input_layer() {
    let ctx = egui::Context::default();
    ctx.style_mut(|s| {
        s.interaction.tooltip_delay = 0.0;
        s.interaction.show_tooltips_only_when_still = false;
    });
    let mut point = Pos2::ZERO;
    let mut layer = egui::LayerId::background();
    let mut output = None;
    for frame in 0..4 {
        let out = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 400.0))),
                time: Some(frame as f64 * 0.1),
                events: if frame == 1 {
                    vec![egui::Event::PointerMoved(point)]
                } else {
                    vec![]
                },
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let response = ui.add_enabled(false, egui::Button::new("Next lesson step"));
                    point = response.rect.center();
                    layer = response.layer_id;
                    annotate(ui, &response, Control::LessonNext);
                });
            },
        );
        output = Some(out);
    }
    let out = output.unwrap();
    let text = out
        .shapes
        .iter()
        .find_map(|s| match &s.shape {
            egui::epaint::Shape::Text(t)
                if t.galley.text().contains("Queue acceptance is insufficient") =>
            {
                Some(t)
            }
            _ => None,
        })
        .expect("disabled control must still display its explanation");
    // The pointer may traverse the painted text to a neighboring real control.
    assert_eq!(
        ctx.layer_id_at(text.pos + Vec2::new(8.0, 12.0)),
        Some(layer)
    );
}

#[test]
fn offline_contract_is_visible_reference_without_a_fabricated_lesson() {
    let mut g = Gui::new();
    g.app.keys_open = true;
    g.app.help.topic = Topic::Offline;
    g.frame(vec![]);
    g.frame(vec![]);
    assert!(g
        .painted
        .iter()
        .any(|text| text.contains("without an account or cloud session")));
    assert!(!g
        .nodes
        .iter()
        .any(|(_, node)| node.label() == Some("Start this lesson")));
    assert!(g.app.help.lesson.is_none());
    assert!(Topic::Offline.text().contains("not implemented"));
    assert!(Topic::Offline.text().contains("external browser"));
}
