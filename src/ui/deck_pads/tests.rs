use super::*;
use crate::ui::test_support::Fixture;
use egui::accesskit::{Action, ActionData, ActionRequest, Node, NodeId};

struct Gui {
    fixture: Fixture,
    ctx: egui::Context,
    time: f64,
    nodes: Vec<(NodeId, Node)>,
    focused: bool,
}
impl Gui {
    fn new() -> Self {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut gui = Self {
            fixture: Fixture::new(256),
            ctx,
            time: 0.0,
            nodes: Vec::new(),
            focused: true,
        };
        gui.frame(vec![]);
        gui.frame(vec![]);
        gui
    }
    fn frame(&mut self, events: Vec<egui::Event>) {
        self.time += 0.02;
        let output = self.ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1800.0, 1600.0))),
                time: Some(self.time),
                focused: self.focused,
                events,
                ..Default::default()
            },
            |ctx| self.fixture.app.update_frame(ctx),
        );
        self.nodes = output.platform_output.accesskit_update.unwrap().nodes;
        self.fixture.rt.process(&mut []);
        self.fixture.rt.publish_for_test();
    }
    fn action(&mut self, label: &str, action: Action, data: Option<ActionData>) {
        self.frame(vec![]);
        let target = self
            .nodes
            .iter()
            .find(|(_, n)| n.label() == Some(label))
            .map(|(id, _)| *id)
            .unwrap_or_else(|| {
                panic!(
                    "missing {label}: {:?}",
                    self.nodes
                        .iter()
                        .filter_map(|(_, n)| n.label())
                        .collect::<Vec<_>>()
                )
            });
        self.frame(vec![egui::Event::AccessKitActionRequest(ActionRequest {
            target,
            action,
            data,
        })]);
        self.frame(vec![]);
    }
    fn mode(&mut self, deck: u8, mode: Mode) {
        let label = format!("Deck {}: Pad mode", (b'A' + deck) as char);
        self.action(&label, Action::Click, None);
        self.action(
            &format!("Deck {}: Pad mode {}", (b'A' + deck) as char, mode.label()),
            Action::Click,
            None,
        );
    }
}

#[test]
fn native_deck_pad_mode_chooser_parameters_and_fixed_slot_actions_reach_only_the_named_deck() {
    let mut gui = Gui::new();
    gui.mode(0, Mode::Roll);
    assert_eq!(
        gui.fixture.app.engine.snapshot().decks[0].controls.pad_mode,
        1
    );
    assert_eq!(
        gui.fixture.app.engine.snapshot().decks[1].controls.pad_mode,
        0
    );
    gui.action("Deck A: Pad parameter right", Action::Click, None);
    assert_eq!(
        gui.fixture.app.engine.snapshot().decks[0]
            .controls
            .roll_scale,
        1
    );
    let label = "Deck A: Roll pad 1: 0.0625 beats";
    gui.action(
        label,
        Action::CustomAction,
        Some(ActionData::CustomAction(1)),
    );
    assert_eq!(
        gui.fixture.app.engine.snapshot().decks[0].controls.roll,
        Some(0)
    );
    gui.mode(0, Mode::Slice);
    assert!(gui.fixture.app.engine.snapshot().decks[0]
        .controls
        .roll
        .is_none());
    gui.action(
        "Deck A: Slice pad 1: Slice 1",
        Action::CustomAction,
        Some(ActionData::CustomAction(2)),
    );
    assert!(gui.fixture.app.engine.snapshot().decks[0]
        .controls
        .slice
        .is_none());
    gui.action(
        "Deck A: Slice pad 1: Slice 1",
        Action::CustomAction,
        Some(ActionData::CustomAction(1)),
    );
    assert_eq!(
        gui.fixture.app.engine.snapshot().decks[0].controls.slice,
        Some(0)
    );
    gui.focused = false;
    gui.frame(vec![]);
    assert!(gui.fixture.app.engine.snapshot().decks[0]
        .controls
        .slice
        .is_none());
}

#[test]
fn native_deck_pad_keyboard_and_touch_owners_release_independently_outside_the_pad() {
    let mut gui = Gui::new();
    gui.mode(1, Mode::Roll);
    let label = "Deck B: Roll pad 2: 0.0625 beats";
    gui.action(label, Action::Focus, None);
    gui.frame(vec![egui::Event::Key {
        key: Key::Space,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    }]);
    assert_eq!(
        gui.fixture.app.engine.snapshot().decks[1].controls.roll,
        Some(1)
    );
    let viewport = gui.ctx.viewport_id();
    gui.fixture
        .app
        .set_deck_pad_input(1, 1, 4, true, 0.4, false, viewport);
    gui.fixture.rt.process(&mut []);
    gui.frame(vec![egui::Event::Key {
        key: Key::Space,
        physical_key: None,
        pressed: false,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    }]);
    assert_eq!(
        gui.fixture.app.engine.snapshot().decks[1].controls.roll,
        Some(1)
    );
    gui.fixture
        .app
        .set_deck_pad_input(1, 1, 4, false, 0.0, false, viewport);
    gui.fixture.rt.process(&mut []);
    gui.fixture.rt.publish_for_test();
    assert!(gui.fixture.app.engine.snapshot().decks[1]
        .controls
        .roll
        .is_none());
    assert!(gui.fixture.app.engine.snapshot().decks[0]
        .controls
        .roll
        .is_none());
    gui.action(label, Action::Focus, None);
    gui.frame(vec![egui::Event::Key {
        key: Key::Space,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    }]);
    assert_eq!(
        gui.fixture.app.engine.snapshot().decks[1].controls.roll,
        Some(1)
    );
    gui.mode(1, Mode::Slice);
    gui.action("Deck B: Slice pad 2: Slice 2", Action::Focus, None);
    gui.frame(vec![egui::Event::Key {
        key: Key::Space,
        physical_key: None,
        pressed: true,
        repeat: true,
        modifiers: egui::Modifiers::NONE,
    }]);
    assert!(gui.fixture.app.engine.snapshot().decks[1]
        .controls
        .slice
        .is_none());
    gui.frame(vec![egui::Event::Key {
        key: Key::Space,
        physical_key: None,
        pressed: false,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    }]);
    gui.frame(vec![egui::Event::Key {
        key: Key::Space,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    }]);
    assert_eq!(
        gui.fixture.app.engine.snapshot().decks[1].controls.slice,
        Some(1)
    );
    gui.focused = false;
    gui.frame(vec![]);
    assert!(gui.fixture.app.engine.snapshot().decks[1]
        .controls
        .slice
        .is_none());
}

#[test]
fn native_deck_pad_presentation_uses_right_deck_sample_names_and_stable_saved_loop_ids() {
    let mut gui = Gui::new();
    gui.mode(1, Mode::Sampler);
    let (name, _, _) = presentation(
        Mode::Sampler,
        1,
        0,
        &gui.fixture.app.snap.decks[1],
        &gui.fixture.app.snap,
    );
    assert_eq!(
        name,
        gui.fixture.app.snap.sampler_instances[gui.fixture.app.snap.sampler_bank]
            .data
            .audio[8]
            .as_ref()
            .unwrap()
            .name
    );
    gui.mode(0, Mode::SavedLoop);
    gui.action("Deck A: Saved Loop pad 3: Save loop", Action::Click, None);
    assert!(gui.fixture.app.engine.snapshot().decks[0].saved_loops.slots[2].is_some());
    assert!(gui.fixture.app.engine.snapshot().decks[1]
        .saved_loops
        .is_default());
}

#[test]
fn native_chromatic_root_ranges_reset_and_focus_release_use_actual_deck_pad_controls() {
    use crate::engine::{deck_controls::Control, key_shift::Request};
    let mut gui = Gui::new();
    gui.fixture.rt.apply(Command::DeckAudio { deck: 0, audio: std::sync::Arc::new(crate::engine::dsp::Sample {
        name: "Native chromatic cue source".into(), path: String::new(), sr: 48000, ch: 2, bpm: 120.0,
        data: (0..144000).flat_map(|frame| { let value = (std::f64::consts::TAU * 440.0 * frame as f64 / 48000.0).sin() as f32 * 0.25; [value, -value] }).collect(), peaks: Vec::new().into(), spectrum: None,
    }) });
    gui.fixture.rt.publish_for_test();
    gui.fixture.rt.apply(Command::DeckHotCue { deck: 0, pad: 2, del: false });
    gui.fixture.rt.publish_for_test();
    let shift = Request::new(0, &gui.fixture.app.engine.snapshot().decks[0], 3, false).unwrap();
    gui.fixture.rt.apply(Command::DeckKeyShift(shift));
    gui.fixture.rt.publish_for_test();
    gui.frame(vec![]);
    gui.mode(0, Mode::PitchCue);
    gui.action("Deck A: Pitch root", Action::Click, None);
    gui.action("Deck A: Pitch root cue 3", Action::Click, None);
    assert_eq!(gui.fixture.app.engine.snapshot().decks[0].controls.pitch_cue, 2);
    gui.action("Deck A: Pitch range high", Action::Click, None);
    assert_eq!(gui.fixture.app.engine.snapshot().decks[0].controls.pitch_range, 2);
    let label = "Deck A: Pitch Cue pad 8: +6 st";
    gui.action(label, Action::CustomAction, Some(ActionData::CustomAction(1)));
    assert_eq!(gui.fixture.app.engine.snapshot().decks[0].controls.pitch_semitones, Some(6));
    assert!(gui.fixture.app.engine.snapshot().decks[0].previewing);
    assert_eq!(gui.fixture.rt.decks[0].key_shift, 3);
    assert!(gui.fixture.app.engine.snapshot().decks[1].controls.pitch_pad.is_none());
    gui.action("Deck A: Pitch range low", Action::Click, None);
    assert_eq!(gui.fixture.app.engine.snapshot().decks[0].controls.pitch_range, 0);
    assert!(gui.fixture.app.engine.snapshot().decks[0].controls.pitch_pad.is_none());
    assert!(!gui.fixture.app.engine.snapshot().decks[0].previewing);
    gui.action("Deck A: Pitch Cue pad 8: +1 st", Action::CustomAction, Some(ActionData::CustomAction(2)));
    gui.action("Deck A: Pitch Cue pad 1: -6 st", Action::CustomAction, Some(ActionData::CustomAction(1)));
    assert_eq!(gui.fixture.app.engine.snapshot().decks[0].controls.pitch_semitones, Some(-6));
    gui.focused = false; gui.frame(vec![]);
    assert!(gui.fixture.app.engine.snapshot().decks[0].controls.pitch_pad.is_none());
    assert!(!gui.fixture.app.engine.snapshot().decks[0].previewing);
    assert_eq!(gui.fixture.rt.decks[0].key_shift, 3);
    gui.focused = true; gui.frame(vec![]);
    gui.action("Deck A: Pitch original key", Action::Click, None);
    assert_eq!(gui.fixture.rt.decks[0].key_shift, 0);
    gui.fixture.rt.apply(Command::Undo); gui.fixture.rt.publish_for_test(); gui.frame(vec![]);
    assert_eq!(gui.fixture.rt.decks[0].key_shift, 3);
    gui.fixture.rt.apply(Command::Redo); gui.fixture.rt.publish_for_test(); gui.frame(vec![]);
    assert_eq!(gui.fixture.rt.decks[0].key_shift, 0);
    gui.fixture.rt.apply(Command::DeckControl { source: 247, deck: 0, control: Control::Quantize { enabled: true, division: 3 } });
    gui.fixture.rt.publish_for_test(); gui.frame(vec![]);
    gui.action("Deck A: Pitch Cue pad 4: -3 st", Action::CustomAction, Some(ActionData::CustomAction(1)));
    assert!(gui.fixture.app.engine.snapshot().decks[0].controls.pending.is_none());
    gui.mode(0, Mode::Sampler);
    assert!(gui.fixture.app.engine.snapshot().decks[0].controls.pitch_pad.is_none());
    assert!(gui.fixture.app.engine.snapshot().decks[0].controls.quantize);
    println!("CHROMATIC_CUE_NATIVE {{\"native_egui_accesskit\":true,\"root_range_labels\":true,\"original_reset_undo_redo\":true,\"range_mode_focus_cancel\":true,\"wrong_deck_mutations\":0,\"physical_devices_opened\":false}}");
}
