use super::*;
use crate::engine::RtEngine;
use egui::accesskit::{ActionRequest, Node, NodeId};

struct Gui {
    app: App,
    rt: RtEngine,
    ctx: egui::Context,
    time: f64,
    nodes: Vec<(NodeId, Node)>,
    focus: NodeId,
    size: Vec2,
    window_focused: bool,
}
impl Gui {
    fn new() -> Self {
        let (engine, mut rt) = Engine::headless_for_test(48_000, 256);
        rt.publish_for_test();
        let app = App::with_loader(engine, Theme::default(), None);
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut gui = Self {
            app,
            rt,
            ctx,
            time: 0.0,
            nodes: Vec::new(),
            focus: NodeId(0),
            size: Vec2::new(1600.0, 1200.0),
            window_focused: true,
        };
        gui.frame(vec![]);
        gui.frame(vec![]);
        gui
    }
    fn frame(&mut self, events: Vec<egui::Event>) -> egui::FullOutput {
        self.time += 0.02;
        let modifiers = events
            .iter()
            .rev()
            .find_map(|event| match event {
                egui::Event::Key { modifiers, .. } => Some(*modifiers),
                _ => None,
            })
            .unwrap_or_default();
        let out = self.ctx.run(
            egui::RawInput {
                modifiers,
                focused: self.window_focused,
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, self.size)),
                time: Some(self.time),
                events,
                ..Default::default()
            },
            |ctx| self.app.update_frame(ctx),
        );
        let tree = out
            .platform_output
            .accesskit_update
            .as_ref()
            .expect("actual accesskit output");
        self.nodes = tree.nodes.clone();
        self.focus = tree.focus;
        self.rt.process(&mut [0.0; 128]);
        self.rt.publish_for_test();
        out
    }
    fn node(&self, name: &str) -> (NodeId, &Node) {
        self.nodes
            .iter()
            .find(|(_, node)| node.label() == Some(name))
            .map(|(id, node)| (*id, node))
            .unwrap_or_else(|| {
                panic!(
                    "missing {name}; labels {:?}",
                    self.nodes
                        .iter()
                        .filter_map(|(_, n)| n.label())
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
    fn key(&mut self, key: Key, mut modifiers: egui::Modifiers, pressed: bool) {
        // Linux winit reports Ctrl as the platform command modifier too.
        modifiers.command |= modifiers.ctrl;
        self.frame(vec![egui::Event::Key {
            key,
            physical_key: None,
            modifiers,
            pressed,
            repeat: false,
        }]);
    }
}

#[test]
fn waveform_zoom_native_actions_keep_playback_and_named_position_controls_live() {
    let mut gui = Gui::new();
    let audio = Arc::new(crate::engine::dsp::Sample { name: "One-hour zoom fixture".into(), sr: 8000, ch: 1,
        data: vec![0.0; 3600 * 8000], peaks: Arc::new(vec![[0.1, 0.2, 0.3]; 8192]),
        bpm: 120.0, path: String::new() });
    gui.rt.apply(Command::DeckAudio { deck: 0, audio });
    gui.rt.decks[0].grid = Some(crate::engine::beatgrid::Grid::new(0.0, 120.0).unwrap().set_anchor(1800.0, 90.0).unwrap());
    gui.rt.decks[0].pos = 3590.0 * 8000.0;
    gui.rt.apply(Command::DeckPlay { deck: 0 });
    for size in [Vec2::new(1600.0, 1200.0), Vec2::new(800.0, 600.0)] {
        gui.size = size; gui.frame(vec![]); gui.frame(vec![]);
        let before = gui.rt.decks[0].pos;
        gui.action("Deck A: Waveform zoom", Action::Click, None);
        gui.action("16 bars", Action::Click, None);
        assert_eq!(gui.app.waveform.zoom, [waveform::Zoom::SixteenBars; DECKS]);
        gui.action("Link zoom", Action::Click, None);
        assert!(!gui.app.waveform.linked);
        gui.action("Deck B: Waveform zoom", Action::Click, None);
        gui.action("2 bars", Action::Click, None);
        assert_eq!(gui.app.waveform.zoom, [waveform::Zoom::SixteenBars, waveform::Zoom::TwoBars]);
        gui.action("Link zoom", Action::Click, None);
        assert_eq!(gui.app.waveform.zoom, [waveform::Zoom::SixteenBars; DECKS]);
        assert!(gui.rt.decks[0].playing && gui.rt.decks[0].pos > before);
        for deck in ['A','B'] {
            let node = gui.node(&format!("Deck {deck}: Waveform position")).1;
            assert_eq!(node.role(), egui::accesskit::Role::Slider);
            assert!(node.description().unwrap().contains("renderer position"));
        }
    }
}

#[test]
fn pitch_lock_reports_renderer_bypass_and_armed_modes_without_changing_its_action_id() {
    use crate::engine::keylock::Mode;
    let mut gui = Gui::new();
    let name = "Deck A: Pitch lock";
    let id = gui.node(name).0;
    let check = |gui: &mut Gui, mode: Mode, detail: &str, mark: &str| {
        gui.rt.publish_for_test();
        gui.frame(vec![]);
        let output = gui.frame(vec![]);
        assert_eq!(gui.app.snap.decks[0].keylock_mode, mode);
        let (current, node) = gui.node(name);
        assert_eq!(current, id, "status changes must preserve the native action target");
        assert!(node.description().unwrap_or_default().contains(detail), "{detail}");
        assert!(output.shapes.iter().any(|shape| matches!(&shape.shape,
            egui::epaint::Shape::Text(text) if text.galley.text() == mark)), "missing {mark}");
    };
    check(&mut gui, Mode::Off, "Off: tempo and pitch", "L");
    gui.action(name, Action::Click, None);
    check(&mut gui, Mode::Stopped, "Armed: deck stopped", "L");
    assert!(gui.rt.decks[0].keylock, "the actual native action reached the renderer");

    gui.rt.apply(Command::DeckPlay { deck: 0 });
    // Natural playback start must actually reach the advertised unity bypass;
    // do not seed an exact rate or fake a snapshot to make this assertion pass.
    gui.rt.process(&mut [0.0; 1024]);
    check(&mut gui, Mode::Unity, "Original rate", "L");
    gui.rt.apply(Command::DeckPitch { deck: 0, value: 0.9 });
    gui.rt.process(&mut [0.0; 256]);
    check(&mut gui, Mode::Locked, "Pitch preservation active", "L");

    gui.rt.apply(Command::DeckTouch { deck: 0, on: true });
    gui.rt.apply(Command::DeckJog { deck: 0, delta: -0.05 });
    gui.rt.process(&mut [0.0; 256]);
    check(&mut gui, Mode::ScratchBypass, "Scratch bypass", "L~");
    gui.rt.apply(Command::DeckTouch { deck: 0, on: false });
    gui.rt.process(&mut [0.0; 16_384]);
    check(&mut gui, Mode::Locked, "Pitch preservation active", "L");

    // Match/Sync may ask for rates beyond the pitch fader's range. Exercise
    // the real sync renderer against that controlled target, not a fake snap.
    gui.rt.decks[0].sync_bpm = 400.0;
    gui.rt.apply(Command::DeckSync { deck: 0 });
    gui.rt.process(&mut [0.0; 16_384]);
    check(&mut gui, Mode::UnsupportedRate, "Rate outside pitch-lock range", "L!");
    gui.action(name, Action::Click, None);
    check(&mut gui, Mode::Off, "Off: tempo and pitch", "L");
    assert!(!gui.rt.decks[0].keylock);
    gui.action(name, Action::Click, None);
    gui.rt.apply(Command::DeckUnload { deck: 0 });
    check(&mut gui, Mode::NoMedia, "Armed: no media", "L");
}

#[test]
fn actual_surface_has_named_roles_values_actions_and_keyboard_focus() {
    let mut gui = Gui::new();
    for name in [
        "Deck A: Pitch",
        "Deck B: Pitch",
        "Deck A: Bass EQ",
        "Crossfader",
        "Track 1: Gain",
        "Crate selection",
    ] {
        let (_, node) = gui.node(name);
        assert_eq!(node.role(), Role::Slider, "{name}");
        assert!(node.numeric_value().is_some(), "{name}");
        for action in [
            Action::Focus,
            Action::SetValue,
            Action::Increment,
            Action::Decrement,
        ] {
            assert!(node.supports_action(action), "{name} {action:?}");
        }
    }
    for name in [
        "Deck A: Platter play or pause",
        "Deck B: Hot cue 1",
        "Scene 1: Toggle playback",
        "Sampler: Sample pad 1",
    ] {
        let (_, node) = gui.node(name);
        assert_eq!(node.role(), Role::Button);
        assert!(node.supports_action(Action::Click));
        assert!(node.supports_action(Action::CustomAction));
    }
    gui.action("Deck A: Pitch", Action::Focus, None);
    let id = gui.node("Deck A: Pitch").0;
    assert_eq!(gui.focus, id);
    gui.key(Key::ArrowUp, Default::default(), true);
    gui.key(Key::ArrowUp, Default::default(), false);
    assert!((gui.rt.decks[0].pitch - 0.50625).abs() < 0.0001);
    gui.key(Key::End, Default::default(), true);
    gui.key(Key::End, Default::default(), false);
    assert_eq!(gui.rt.decks[0].pitch, 1.0);
    gui.key(Key::Tab, Default::default(), true);
    gui.key(Key::Tab, Default::default(), false);
    assert_ne!(gui.focus, id, "Tab leaves numeric control");
}

#[test]
fn numeric_setvalue_f2_validation_cancel_and_apply_use_display_units() {
    let mut gui = Gui::new();
    gui.action(
        "Deck A: Pitch",
        Action::SetValue,
        Some(ActionData::NumericValue(-4.0)),
    );
    assert_eq!(gui.rt.decks[0].pitch, 0.25);
    gui.action(
        "Deck A: Pitch",
        Action::SetValue,
        Some(ActionData::NumericValue(f64::NAN)),
    );
    assert_eq!(gui.rt.decks[0].pitch, 0.25);
    gui.action("Deck A: Pitch", Action::Focus, None);
    gui.key(Key::F2, Default::default(), true);
    gui.key(Key::F2, Default::default(), false);
    assert!(
        gui.ctx
            .data(|data| data.get_temp::<NumericEdit>(egui::Id::new(NUMBER)))
            .is_some(),
        "F2 opens numeric dialog; focus {:?}, pitch {:?}",
        gui.focus,
        gui.node("Deck A: Pitch").0
    );
    gui.key(Key::A, egui::Modifiers::CTRL, true);
    gui.key(Key::A, egui::Modifiers::CTRL, false);
    gui.frame(vec![egui::Event::Text("999".into())]);
    gui.key(Key::Enter, Default::default(), true);
    gui.key(Key::Enter, Default::default(), false);
    assert_eq!(gui.rt.decks[0].pitch, 0.25);
    assert!(
        gui.ctx
            .data(|data| data.get_temp::<NumericEdit>(egui::Id::new(NUMBER)))
            .unwrap()
            .error
    );
    gui.key(Key::Escape, Default::default(), true);
    gui.key(Key::Escape, Default::default(), false);
    assert!(gui
        .ctx
        .data(|data| data.get_temp::<NumericEdit>(egui::Id::new(NUMBER)))
        .is_none());
    gui.key(Key::F2, Default::default(), true);
    gui.key(Key::F2, Default::default(), false);
    assert!(
        gui.ctx
            .data(|data| data.get_temp::<NumericEdit>(egui::Id::new(NUMBER)))
            .is_some(),
        "F2 opens numeric dialog; focus {:?}, pitch {:?}",
        gui.focus,
        gui.node("Deck A: Pitch").0
    );
    gui.key(Key::A, egui::Modifiers::CTRL, true);
    gui.key(Key::A, egui::Modifiers::CTRL, false);
    gui.frame(vec![egui::Event::Text("2".into())]);
    gui.key(Key::Enter, Default::default(), true);
    gui.key(Key::Enter, Default::default(), false);
    gui.frame(vec![]);
    assert!(
        (gui.rt.decks[0].pitch - 0.625).abs() < 0.0001,
        "pitch {} edit {:?}",
        gui.rt.decks[0].pitch,
        gui.ctx
            .data(|data| data.get_temp::<NumericEdit>(egui::Id::new(NUMBER)))
            .map(|edit| (edit.text, edit.error))
    );
}

#[test]
fn native_f2_decimal_comma_validation_cancel_and_apply_preserve_display_units() {
    use crate::localization::Locale;
    for locale in [Locale::Spanish, Locale::German] {
        let mut gui = Gui::new();
        let active = gui.app.settings.applied.active.clone();
        gui.app.settings.applied.profiles.get_mut(&active).unwrap().appearance.locale = locale;
        gui.frame(vec![]);
        gui.action("Deck A: Pitch", Action::Focus, None);
        for (input, commit) in [("NaN", false), ("999,5", false), ("2,5", false), ("2,5", true)] {
            gui.key(Key::F2, Default::default(), true);
            gui.key(Key::F2, Default::default(), false);
            gui.key(Key::A, egui::Modifiers::CTRL, true);
            gui.key(Key::A, egui::Modifiers::CTRL, false);
            gui.frame(vec![egui::Event::Text(input.into())]);
            if input != "2,5" || commit {
                gui.key(Key::Enter, Default::default(), true);
                gui.key(Key::Enter, Default::default(), false);
            }
            if !commit {
                assert_eq!(gui.rt.decks[0].pitch, 0.5);
                if input != "2,5" {
                    assert!(gui.ctx.data(|data| data.get_temp::<NumericEdit>(egui::Id::new(NUMBER))).unwrap().error);
                }
                gui.key(Key::Escape, Default::default(), true);
                gui.key(Key::Escape, Default::default(), false);
            } else {
                gui.frame(vec![]);
                assert!((gui.rt.decks[0].pitch - 0.65625).abs() < 0.0001);
            }
            assert!(gui.ctx.data(|data| data.get_temp::<NumericEdit>(egui::Id::new(NUMBER))).is_none());
        }
    }
}

#[test]
fn assistive_custom_actions_reach_cues_loops_compose_clip_gain_and_solo() {
    let mut gui = Gui::new();
    gui.action("Deck A: Hot cue 1", Action::Click, None);
    assert!(gui.rt.decks[0].hotcues[0].set);
    gui.action(
        "Deck A: Hot cue 1",
        Action::CustomAction,
        Some(ActionData::CustomAction(1)),
    );
    assert!(!gui.rt.decks[0].hotcues[0].set);
    gui.action(
        "Track 1: Gain",
        Action::CustomAction,
        Some(ActionData::CustomAction(1)),
    );
    assert!(gui.rt.tracks[0].solo);
    let clip = gui
        .nodes
        .iter()
        .filter_map(|(_, node)| node.label())
        .find(|name| name.starts_with("Clip track 1 scene 1:"))
        .unwrap()
        .to_owned();
    gui.action(
        &clip,
        Action::CustomAction,
        Some(ActionData::CustomAction(2)),
    );
    assert!(gui.rt.compose_target.is_some());
    gui.action(
        &clip,
        Action::CustomAction,
        Some(ActionData::CustomAction(3)),
    );
    assert!(gui.app.clip_gain_edit.is_some());
    gui.action(
        "Clip track 1 scene 1: Gain",
        Action::SetValue,
        Some(ActionData::NumericValue(37.0)),
    );
    assert!((gui.rt.tracks[0].clips[0].gain - 0.37).abs() < 0.0001);
}

#[test]
fn sampler_keyboard_and_assistive_holds_share_a_gate_and_release_independently() {
    let mut gui = Gui::new();
    let pad = "Sampler: Sample pad 1";
    gui.action(pad, Action::Focus, None);
    gui.key(Key::Space, Default::default(), true);
    assert!(gui.app.pad_held[0]);
    assert!(!gui.rt.playing, "pad key cannot toggle transport");
    gui.action(pad, Action::Click, None);
    gui.key(Key::Space, Default::default(), false);
    assert!(
        gui.app.pad_held[0],
        "assistive hold survives keyboard release"
    );
    gui.action(pad, Action::CustomAction, Some(ActionData::CustomAction(1)));
    assert!(!gui.app.pad_held[0]);
    gui.key(Key::Space, Default::default(), true);
    gui.key(Key::Enter, Default::default(), true);
    gui.key(Key::Space, Default::default(), false);
    assert!(gui.app.pad_held[0], "Enter remains held");
    gui.action("Deck A: Pitch", Action::Focus, None);
    assert!(!gui.app.pad_held[0], "focus loss releases the original pad");
}

#[test]
fn keyboard_context_menu_has_all_modifier_actions_and_shift_fine_adjustment() {
    let mut gui = Gui::new();
    gui.action("Track 1: Gain", Action::Focus, None);
    let before = gui.rt.tracks[0].gain;
    gui.key(Key::ArrowDown, egui::Modifiers::SHIFT, true);
    gui.key(Key::ArrowDown, egui::Modifiers::SHIFT, false);
    assert!((gui.rt.tracks[0].gain - (before - 0.001)).abs() < 0.00001);
    gui.key(Key::F10, egui::Modifiers::SHIFT, true);
    gui.key(Key::F10, egui::Modifiers::SHIFT, false);
    gui.node("Toggle mute");
    gui.node("Toggle solo");
    gui.node("Open track effects");
    gui.action("Toggle solo", Action::Click, None);
    assert!(gui.rt.tracks[0].solo);
    assert!(!egui::Popup::is_any_open(&gui.ctx));
}

#[test]
fn virtual_crate_keyboard_and_assistive_index_reach_all_rows_with_bounded_visible_work() {
    let mut gui = Gui::new();
    gui.app.library = Arc::new(
        (0..50_000)
            .map(|index| LibItem {
                source: if index == 49_999 {
                    LibSource::Builtin(BuiltinStem::Harmony)
                } else {
                    LibSource::File(format!("private/{index}.wav").into())
                },
                title: format!("Track {index:05}"),
                artist: "Artist".into(),
                bpm: Bpm::hint(120.0),
                fingerprint: None,
                key: "C".into(),
                length: Some(120.0),
                last_play: None,
            })
            .collect(),
    );
    gui.frame(vec![]);
    gui.frame(vec![]);
    let rebuilds = gui.app.library_view.stats.rebuilds;
    let examined = gui.app.library_view.stats.examined;
    gui.action("Crate selection", Action::Focus, None);
    gui.key(Key::End, Default::default(), true);
    gui.key(Key::End, Default::default(), false);
    assert_eq!(gui.app.lib_sel, 49_999);
    assert!(gui
        .node("Crate selection")
        .1
        .value()
        .unwrap()
        .contains("Track 49999"));
    gui.key(Key::Enter, Default::default(), true);
    gui.key(Key::Enter, Default::default(), false);
    assert!(Arc::ptr_eq(
        gui.rt.decks[gui.app.load_target()].audio.as_ref().unwrap(),
        gui.rt.builtin[1].as_ref().unwrap()
    ));
    gui.action(
        "Crate selection",
        Action::SetValue,
        Some(ActionData::NumericValue(25001.0)),
    );
    assert_eq!(gui.app.lib_sel, 25_000);
    gui.key(Key::PageUp, Default::default(), true);
    gui.key(Key::PageUp, Default::default(), false);
    assert!(gui.app.lib_sel < 25_000);
    assert_eq!(gui.app.library_view.stats.rebuilds, rebuilds);
    assert_eq!(gui.app.library_view.stats.examined, examined);
    assert!(gui.app.library_view.stats.rendered < 20);
    assert!(gui.app.library_view.cells.len() < 20);
}

#[test]
fn native_slider_arrows_and_assistive_steps_are_applied_once_in_display_units() {
    let mut gui = Gui::new();
    let name = "Master effect 1: Wet";
    gui.action(name, Action::SetValue, Some(ActionData::NumericValue(40.0)));
    for invalid in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        gui.action(
            name,
            Action::SetValue,
            Some(ActionData::NumericValue(invalid)),
        );
        assert!((gui.rt.fx_wet[0] - 0.40).abs() < 0.00001);
    }
    gui.action(name, Action::Increment, None);
    assert!((gui.rt.fx_wet[0] - 0.41).abs() < 0.00001);
    gui.action(name, Action::Focus, None);
    gui.key(Key::ArrowRight, Default::default(), true);
    gui.key(Key::ArrowRight, Default::default(), false);
    assert!((gui.rt.fx_wet[0] - 0.42).abs() < 0.00001);
    gui.key(Key::ArrowLeft, egui::Modifiers::SHIFT, true);
    gui.key(Key::ArrowLeft, egui::Modifiers::SHIFT, false);
    assert!((gui.rt.fx_wet[0] - 0.419).abs() < 0.00001);
    gui.key(Key::Home, Default::default(), true);
    gui.key(Key::Home, Default::default(), false);
    assert_eq!(gui.rt.fx_wet[0], 0.0);
}

#[test]
fn every_pad_keyboard_and_assistive_gate_retains_exact_note_and_original_instrument() {
    use crate::engine::{dsp::InputKey, sampler_pad::PadIdentity, SynthInstrument};
    let mut gui = Gui::new();
    crate::engine::sampler_identity_tests::prepare(&mut gui.rt);
    let notes = [
        Some(57),
        Some(59),
        Some(60),
        Some(62),
        Some(64),
        Some(65),
        Some(67),
        Some(69),
        Some(58),
        None,
        Some(61),
        Some(63),
        None,
        Some(66),
        Some(68),
        None,
    ];
    for instrument in SamplerInstrument::ALL {
        gui.rt.apply(Command::SamplerInst(instrument));
        gui.rt.publish_for_test();
        gui.frame(vec![]);
        for pad in 0..16 {
            let identity = PadIdentity::new(pad as u8);
            let name = if instrument.synth().is_some() {
                format!(
                    "Sampler: Pad {}: {} MIDI note {}",
                    pad + 1,
                    identity.piano_label(),
                    identity.midi_note(3)
                )
            } else {
                format!("Sampler: Sample pad {}", pad + 1)
            };
            for assistive in [false, true] {
                gui.action(&name, Action::Focus, None);
                if assistive {
                    gui.action(
                        &name,
                        Action::CustomAction,
                        Some(ActionData::CustomAction(0)),
                    );
                } else {
                    gui.key(Key::Space, Default::default(), true);
                }
                let enabled = instrument.synth().is_none() || notes[pad].is_some();
                assert_eq!(gui.app.pad_held[pad], enabled, "{instrument:?} pad {pad}");
                if let Some(kind) = instrument.synth() {
                    let held: Vec<_> = gui
                        .rt
                        .sampler_poly
                        .voices
                        .iter()
                        .filter(|voice| {
                            voice.input == Some(InputKey::Pad(pad as u8))
                                && matches!(voice.env.stage, 1..=3)
                        })
                        .map(|voice| (voice.note(), voice.kind))
                        .collect();
                    assert_eq!(
                        held,
                        notes[pad]
                            .map(|note| vec![(note, kind)])
                            .unwrap_or_default()
                    );
                } else {
                    assert!(Arc::ptr_eq(
                        &gui.rt.pad_voices[pad].as_ref().unwrap().audio,
                        &gui.rt.sampler_banks[gui.rt.sampler_bank].data.audio[pad].as_ref().unwrap()
                    ));
                }
                if assistive {
                    gui.action(
                        &name,
                        Action::CustomAction,
                        Some(ActionData::CustomAction(1)),
                    );
                } else {
                    gui.key(Key::Space, Default::default(), false);
                }
                assert!(!gui.app.pad_held[pad]);
                assert!(gui
                    .rt
                    .sampler_poly
                    .voices
                    .iter()
                    .all(|voice| voice.input != Some(InputKey::Pad(pad as u8))
                        || !matches!(voice.env.stage, 1..=3)));
            }
        }
    }
    // Mode changes rename the same identity; release must retire the old voice.
    gui.rt.apply(Command::SamplerInst(SamplerInstrument::Synth(
        SynthInstrument::Keys,
    )));
    gui.rt.publish_for_test();
    gui.frame(vec![]);
    let old = "Sampler: Pad 1: A MIDI note 57";
    gui.action(old, Action::CustomAction, Some(ActionData::CustomAction(0)));
    gui.rt
        .apply(Command::SamplerInst(SamplerInstrument::Samples));
    gui.rt.publish_for_test();
    gui.frame(vec![]);
    gui.action(
        "Sampler: Sample pad 1",
        Action::CustomAction,
        Some(ActionData::CustomAction(1)),
    );
    assert!(gui
        .rt
        .sampler_poly
        .voices
        .iter()
        .all(|voice| voice.input != Some(InputKey::Pad(0)) || !matches!(voice.env.stage, 1..=3)));
}

#[test]
fn focus_scrolls_clipped_controls_into_a_small_window_and_shift_tab_traverses_back() {
    let mut gui = Gui::new();
    gui.size = Vec2::new(800.0, 600.0);
    gui.ctx.style_mut(|style| style.animation_time = 0.0);
    gui.frame(vec![]);
    gui.action("Deck B: Pitch", Action::Focus, None);
    for _ in 0..4 {
        gui.frame(vec![]);
    }
    let id = gui.node("Deck B: Pitch").0;
    let bounds = gui.node("Deck B: Pitch").1.bounds().unwrap();
    assert!(
        bounds.x0 >= 0.0 && bounds.x1 <= 800.0 && bounds.y0 >= 0.0 && bounds.y1 <= 600.0,
        "{bounds:?}"
    );
    gui.key(Key::Tab, Default::default(), true);
    gui.key(Key::Tab, Default::default(), false);
    assert_ne!(gui.focus, id);
    gui.key(Key::Tab, egui::Modifiers::SHIFT, true);
    gui.key(Key::Tab, egui::Modifiers::SHIFT, false);
    assert_eq!(gui.focus, id);
    gui.action("Track 8: Gain", Action::Focus, None);
    for _ in 0..4 {
        gui.frame(vec![]);
    }
    let bounds = gui.node("Track 8: Gain").1.bounds().unwrap();
    assert!(
        bounds.x0 >= 0.0 && bounds.x1 <= 800.0 && bounds.y0 >= 0.0 && bounds.y1 <= 600.0,
        "{bounds:?}"
    );
}

#[test]
fn window_loss_releases_all_input_holds_and_rejected_press_never_creates_local_ownership() {
    let mut gui = Gui::new();
    gui.action("Sampler: Sample pad 1", Action::Focus, None);
    gui.key(Key::Space, Default::default(), true);
    gui.action(
        "Sampler: Sample pad 2",
        Action::CustomAction,
        Some(ActionData::CustomAction(0)),
    );
    assert!(gui.app.pad_held[0] && gui.app.pad_held[1]);
    gui.window_focused = false;
    gui.frame(vec![egui::Event::WindowFocused(false)]);
    assert!(gui.app.pad_held.iter().all(|held| !held));
    assert!(gui.app.pad_inputs.iter().all(|mask| *mask == 0));
    gui.window_focused = true;
    gui.frame(vec![egui::Event::WindowFocused(true)]);
    gui.key(Key::Space, Default::default(), false);
    // Saturate the real producer queue before GUI sees a press.
    while gui.app.engine.cmd.send(Command::SetBpm(123.0)).is_ok() {}
    let name = "Sampler: Sample pad 1";
    let target = gui.node(name).0;
    gui.frame(vec![egui::Event::AccessKitActionRequest(ActionRequest {
        action: Action::CustomAction,
        target,
        data: Some(ActionData::CustomAction(0)),
    })]);
    assert!(!gui.app.pad_held[0]);
    assert_eq!(gui.app.pad_inputs[0], 0);
    assert!(gui.app.submission_error.get().is_some());
}

#[test]
fn keyboard_compose_and_deck_preparation_use_actual_commands_without_pointer_events() {
    let mut gui = Gui::new();
    let clip = gui
        .nodes
        .iter()
        .filter_map(|(_, node)| node.label())
        .find(|name| name.starts_with("Clip track 1 scene 1:"))
        .unwrap()
        .to_owned();
    gui.action(&clip, Action::Focus, None);
    gui.key(Key::F10, egui::Modifiers::SHIFT, true);
    gui.key(Key::F10, egui::Modifiers::SHIFT, false);
    gui.action("Arm compose", Action::Click, None);
    let before = gui.rt.tracks[0].clips[0].notes.len();
    gui.action("Sampler: Sample pad 1", Action::Focus, None);
    gui.key(Key::Space, Default::default(), true);
    gui.key(Key::Space, Default::default(), false);
    assert_eq!(gui.rt.tracks[0].clips[0].notes.len(), before + 1);
    gui.action(
        "Deck B: Waveform position",
        Action::SetValue,
        Some(ActionData::NumericValue(2.0)),
    );
    let deck = &gui.rt.decks[1];
    assert!((deck.pos / deck.audio.as_ref().unwrap().sr as f64 - 2.0).abs() < 1e-4);
    gui.action("Deck B: Hot cue 2", Action::Click, None);
    assert!(gui.rt.decks[1].hotcues[1].set);
    gui.action("Deck B: Platter play or pause", Action::Focus, None);
    gui.key(Key::Enter, Default::default(), true);
    gui.key(Key::Enter, Default::default(), false);
    assert!(gui.rt.decks[1].playing);
    assert!(
        !gui.rt.playing,
        "deck activation cannot toggle global transport"
    );
}

#[test]
fn each_focusable_surface_control_has_a_semantic_role_name_or_native_label_association() {
    let gui = Gui::new();
    for (id, node) in &gui.nodes {
        if !node.supports_action(Action::Focus) {
            continue;
        }
        assert_ne!(
            node.role(),
            Role::Unknown,
            "unnamed role {id:?} bounds {:?} children {:?}",
            node.bounds(),
            node.children()
        );
        assert!(
            node.label().is_some_and(|label| !label.is_empty()) || !node.labelled_by().is_empty(),
            "missing name {id:?} {:?} value {:?}",
            node.role(),
            node.value()
        );
        assert!(node.bounds().is_some(), "missing focus geometry {id:?}");
    }
    assert!(gui.node("Sampler instrument").1.value().is_some());
    assert!(gui.node("Sampler bank").1.value().is_some());
    assert!(gui
        .node("Track 1: Gain")
        .1
        .description()
        .unwrap()
        .contains("Mute off; solo off"));
    assert!(gui
        .node("Deck A: Bass EQ")
        .1
        .description()
        .unwrap()
        .contains("Cut off; solo off"));
}

#[test]
fn native_click_menu_reaches_alternative_actions_without_custom_action_support() {
    let mut gui = Gui::new();
    gui.action("Deck A: Hot cue 1", Action::Click, None);
    gui.action("Deck A: Hot cue 1", Action::Focus, None);
    gui.action("Actions for Deck A: Hot cue 1", Action::Click, None);
    gui.action("Delete cue", Action::Click, None);
    assert!(!gui.rt.decks[0].hotcues[0].set);
    gui.action("Sampler: Sample pad 1", Action::Focus, None);
    gui.action("Actions for Sampler: Sample pad 1", Action::Click, None);
    gui.action("Press pad", Action::Click, None);
    assert!(gui.app.pad_held[0]);
    gui.action("Actions for Sampler: Sample pad 1", Action::Click, None);
    gui.action("Release pad", Action::Click, None);
    assert!(!gui.app.pad_held[0]);
}

#[test]
fn overflow_scrollbar_values_and_scroll_view_keys_move_the_actual_content() {
    let mut gui = Gui::new();
    gui.size = Vec2::new(800.0, 600.0);
    gui.ctx.style_mut(|style| style.animation_time = 0.0);
    gui.frame(vec![]);
    gui.frame(vec![]);
    let name = "Performance surface: Vertical scroll";
    assert_eq!(gui.node(name).1.role(), Role::ScrollBar);
    assert!(gui.node(name).1.max_numeric_value().unwrap() > 32.0);
    let start = gui.node("Deck A: Pitch").1.bounds().unwrap().y0;
    gui.action(name, Action::SetValue, Some(ActionData::NumericValue(32.0)));
    assert!((gui.node(name).1.numeric_value().unwrap() - 32.0).abs() < 0.1);
    assert!(gui.node("Deck A: Pitch").1.bounds().unwrap().y0 < start - 30.0);
    gui.action("Performance surface", Action::Focus, None);
    gui.key(Key::Home, Default::default(), true);
    gui.key(Key::Home, Default::default(), false);
    assert!(gui.node(name).1.numeric_value().unwrap() < 0.1);
    gui.key(Key::PageDown, Default::default(), true);
    gui.key(Key::PageDown, Default::default(), false);
    assert!(gui.node(name).1.numeric_value().unwrap() > 32.0);
}

#[test]
fn accessible_project_path_keyboard_edit_save_reopen_and_undo_preserve_composition() {
    let directory = std::env::temp_dir().join(format!(
        "omatainer-accessible-project-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("keyboard composition.omat");
    let _ = std::fs::remove_file(&path);
    let mut gui = Gui::new();
    let wait = |gui: &mut Gui, check: &dyn Fn(&Gui) -> bool| {
        let until = Instant::now() + std::time::Duration::from_secs(8);
        while !check(gui) {
            gui.frame(vec![]);
            assert!(Instant::now() < until, "accessible project did not settle: epoch {} notes {} labels {:?}", gui.app.engine.undo.view().epoch, gui.rt.tracks[0].clips[0].notes.len(), gui.nodes.iter().filter_map(|(_,n)| n.label()).filter(|n| n.contains("project") || n.contains(".omat") || n.contains("Saved") || n.contains("Open")).collect::<Vec<_>>());
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        gui.frame(vec![]);
    };
    let epoch = gui.app.engine.undo.view().epoch;
    gui.action("Project", Action::Click, None);
    gui.action("New project", Action::Click, None);
    wait(&mut gui, &|g| {
        g.app.engine.undo.view().epoch > epoch && !g.app.project.committing()
    });
    let clip = gui
        .nodes
        .iter()
        .filter_map(|(_, n)| n.label())
        .find(|n| n.starts_with("Clip track 1 scene 1:"))
        .unwrap()
        .to_owned();
    gui.action(&clip, Action::Focus, None);
    gui.key(Key::F10, egui::Modifiers::SHIFT, true);
    gui.key(Key::F10, egui::Modifiers::SHIFT, false);
    gui.action("Arm compose", Action::Focus, None);
    gui.key(Key::Enter, Default::default(), true);
    gui.key(Key::Enter, Default::default(), false);
    assert!(gui.rt.compose_target.is_some());
    let pad = gui
        .nodes
        .iter()
        .filter_map(|(_, n)| n.label())
        .find(|n| *n == "Sampler: Sample pad 1" || n.starts_with("Sampler: Pad 1:"))
        .unwrap()
        .to_owned();
    gui.action(&pad, Action::Focus, None);
    gui.key(Key::Space, Default::default(), true);
    gui.key(Key::Space, Default::default(), false);
    assert_eq!(gui.rt.tracks[0].clips[0].notes.len(), 1);
    gui.action("Project", Action::Click, None);
    gui.action("Save project as…", Action::Click, None);
    assert_eq!(gui.node("Project file path").1.role(), Role::TextInput);
    gui.action("Project file path", Action::Focus, None);
    gui.key(Key::A, egui::Modifiers::CTRL, true);
    gui.key(Key::A, egui::Modifiers::CTRL, false);
    gui.frame(vec![egui::Event::Text(path.display().to_string())]);
    gui.action("Save", Action::Focus, None);
    gui.key(Key::Enter, Default::default(), true);
    gui.key(Key::Enter, Default::default(), false);
    wait(&mut gui, &|g| {
        path.exists() && !g.node("Project").1.is_disabled()
    });
    gui.action(
        "Track 1: Gain",
        Action::SetValue,
        Some(ActionData::NumericValue(31.0)),
    );
    assert!((gui.rt.tracks[0].gain - 0.31).abs() < 1e-6);
    gui.key(Key::Z, egui::Modifiers::CTRL, true);
    gui.key(Key::Z, egui::Modifiers::CTRL, false);
    assert!(
        (gui.rt.tracks[0].gain - 0.31).abs() > 1e-4,
        "focused slider permits global creative Undo"
    );
    gui.action("Project", Action::Click, None);
    gui.action("New project", Action::Click, None);
    wait(&mut gui, &|g| {
        g.rt.tracks[0].clips[0].notes.is_empty() && !g.app.project.committing()
    });
    gui.action("Project", Action::Click, None);
    gui.action("Open project…", Action::Click, None);
    gui.action("Project file path", Action::Focus, None);
    gui.frame(vec![egui::Event::Text(path.display().to_string())]);
    gui.action("Open", Action::Focus, None);
    gui.key(Key::Enter, Default::default(), true);
    gui.key(Key::Enter, Default::default(), false);
    wait(&mut gui, &|g| {
        g.rt.tracks[0].clips[0].notes.len() == 1 && !g.app.project.committing()
    });
    assert_eq!(gui.app.engine.undo.view().cursor, 0);
    assert!(gui.app.pad_inputs.iter().all(|v| *v == 0));
    assert!(gui
        .ctx
        .data(|d| d.get_temp::<NumericEdit>(egui::Id::new(NUMBER)))
        .is_none());
    std::fs::remove_dir_all(directory).unwrap();
}
