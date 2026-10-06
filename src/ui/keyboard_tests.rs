use super::test_support::{label_center, Fixture};
use super::*;

fn key(key: Key, pressed: bool, modifiers: egui::Modifiers) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: Some(key),
        pressed,
        repeat: false,
        modifiers,
    }
}

struct Gui {
    fixture: Fixture,
    ctx: egui::Context,
    time: f64,
}

impl Gui {
    fn new() -> Self {
        Self {
            fixture: Fixture::new(144),
            ctx: egui::Context::default(),
            time: 0.0,
        }
    }

    fn frame(&mut self, events: Vec<egui::Event>, modifiers: egui::Modifiers) -> egui::FullOutput {
        self.time += 0.02;
        let output = self.ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1440.0, 1000.0))),
                time: Some(self.time),
                events,
                modifiers,
                ..Default::default()
            },
            |ctx| self.fixture.app.update_frame(ctx),
        );
        self.fixture.rt.process(&mut []);
        output
    }

    fn click(&mut self, position: Pos2) {
        for pressed in [true, false] {
            self.frame(
                vec![
                    egui::Event::PointerMoved(position),
                    egui::Event::PointerButton {
                        pos: position,
                        button: PointerButton::Primary,
                        pressed,
                        modifiers: Default::default(),
                    },
                ],
                Default::default(),
            );
        }
    }

    fn focus_search(&mut self) {
        self.fixture.app.lib_filter.clear();
        self.frame(vec![], Default::default());
        let output = self.frame(vec![], Default::default());
        self.click(label_center(&output, "search"));
        let id = self
            .ctx
            .memory(|memory| memory.focused())
            .expect("real search focus");
        assert!(egui::TextEdit::load_state(&self.ctx, id).is_some());
    }

    fn stroke(&mut self, pressed: Key, modifiers: egui::Modifiers, text: Option<&str>) {
        let mut events = vec![key(pressed, true, modifiers)];
        if let Some(text) = text {
            events.push(egui::Event::Text(text.into()));
        }
        self.frame(events, modifiers);
        self.frame(vec![key(pressed, false, modifiers)], modifiers);
    }

    fn command_count(&self) -> u64 {
        self.fixture.rt.command_stats.received
    }
}

#[test]
fn keyboard_only_palette_search_cancel_and_run_never_dispatch_typing_as_music() {
    let mut gui = Gui::new();
    gui.frame(vec![], Default::default());
    let before = gui.command_count();
    let chord = egui::Modifiers { ctrl: true, command: true, shift: true, ..Default::default() };
    gui.stroke(Key::P, chord, None);
    assert!(gui.fixture.app.command_palette.open);
    for (key, text) in [(Key::Q,"q"), (Key::Space," "), (Key::Num1,"1"), (Key::A,"a")] {
        gui.stroke(key, Default::default(), Some(text));
        assert_eq!(gui.command_count(), before);
    }
    gui.stroke(Key::Escape, Default::default(), None);
    assert!(!gui.fixture.app.command_palette.open);
    assert_eq!(gui.command_count(), before);
    gui.frame(vec![], Default::default());
    gui.frame(vec![], Default::default());
    gui.stroke(Key::P, chord, None);
    gui.frame(vec![egui::Event::Text("Play / stop session".into())], Default::default());
    assert_eq!(gui.command_count(), before);
    gui.stroke(Key::Enter, Default::default(), None);
    assert_eq!(gui.command_count(), before + 1);
    assert!(gui.fixture.rt.playing);
    assert!(!gui.fixture.app.command_palette.open);
}

#[test]
fn palette_respects_saved_chord_collisions_and_keyboard_layout_logical_keys() {
    let mut gui = Gui::new();
    let active = gui.fixture.app.settings.applied.active.clone();
    gui.fixture.app.settings.applied.profiles.get_mut(&active).unwrap().shortcuts.insert("play_a".into(), Some(crate::preferences::Shortcut { key:"P".into(),ctrl:true,shift:true,alt:false }));
    assert_eq!(command_palette::chord(gui.fixture.app.settings.profile()), Some(Key::K));
    gui.frame(vec![], Default::default());
    let before = gui.command_count();
    let modifiers = egui::Modifiers { ctrl: true, command: true, shift: true, ..Default::default() };
    gui.frame(vec![egui::Event::Key { key:Key::K, physical_key:Some(Key::Q), pressed:true, repeat:false, modifiers }], modifiers);
    gui.frame(vec![egui::Event::Key { key:Key::K, physical_key:Some(Key::Q), pressed:false, repeat:false, modifiers }], modifiers);
    assert!(gui.fixture.app.command_palette.open);
    assert_eq!(gui.command_count(), before);
    gui.frame(vec![egui::Event::Ime(egui::ImeEvent::Preedit("東京".into()))], Default::default());
    gui.stroke(Key::Enter, Default::default(), None);
    assert!(gui.fixture.app.command_palette.open);
    assert_eq!(gui.command_count(), before);
    gui.stroke(Key::Escape, Default::default(), None);
}

#[test]
fn actual_focused_search_types_all_bound_characters_without_global_commands() {
    let mut gui = Gui::new();
    gui.focus_search();
    let before = gui.command_count();
    let mut expected = String::new();
    for (pressed, text, modifiers) in [
        (Key::Q, "q", egui::Modifiers::NONE),
        (Key::P, "p", egui::Modifiers::NONE),
        (Key::A, "a", egui::Modifiers::NONE),
        (Key::L, "l", egui::Modifiers::NONE),
        (Key::M, "m", egui::Modifiers::NONE),
        (Key::W, "w", egui::Modifiers::NONE),
        (Key::O, "o", egui::Modifiers::NONE),
        (Key::F, "f", egui::Modifiers::NONE),
        (Key::Num1, "1", egui::Modifiers::NONE),
        (Key::Num2, "2", egui::Modifiers::NONE),
        (Key::Num3, "3", egui::Modifiers::NONE),
        (Key::Num4, "4", egui::Modifiers::NONE),
        (Key::Num5, "5", egui::Modifiers::NONE),
        (Key::Num6, "6", egui::Modifiers::NONE),
        (Key::Num7, "7", egui::Modifiers::NONE),
        (Key::Num8, "8", egui::Modifiers::NONE),
        (Key::OpenBracket, "[", egui::Modifiers::NONE),
        (Key::CloseBracket, "]", egui::Modifiers::NONE),
        (Key::Space, " ", egui::Modifiers::NONE),
        (Key::Slash, "/", egui::Modifiers::NONE),
        (Key::Slash, "?", egui::Modifiers::SHIFT),
        (Key::Q, "Q", egui::Modifiers::SHIFT),
        (Key::P, "P", egui::Modifiers::SHIFT),
        (Key::A, "A", egui::Modifiers::SHIFT),
        (Key::L, "L", egui::Modifiers::SHIFT),
    ] {
        gui.stroke(pressed, modifiers, Some(text));
        expected.push_str(text);
        assert_eq!(gui.fixture.app.lib_filter, expected);
        assert_eq!(gui.command_count(), before);
        assert!(!gui.fixture.app.keys_open && !gui.fixture.app.midi_open);
    }
    assert!(!gui.fixture.rt.playing);
    assert!(gui.fixture.rt.decks.iter().all(|deck| !deck.playing));
}

#[test]
fn restored_scene_sync_and_crossfader_keys_reach_exact_renderer_targets() {
    for (scene, pressed) in [Key::Num1, Key::Num2, Key::Num3, Key::Num4,
        Key::Num5, Key::Num6, Key::Num7, Key::Num8].into_iter().enumerate() {
        let mut gui = Gui::new();
        let clip = gui.fixture.rt.tracks[0].clips[0].clone();
        gui.fixture.rt.tracks[0].clips.fill(clip);
        gui.frame(vec![], Default::default());
        let before = gui.command_count();
        gui.stroke(pressed, Default::default(), None);
        assert_eq!(gui.command_count(), before + 1);
        assert_eq!(gui.fixture.rt.tracks[0].playing.as_ref().unwrap().scene as usize, scene);
    }
    let mut gui = Gui::new();
    gui.frame(vec![], Default::default());
    for (pressed, deck) in [(Key::W, 0), (Key::O, 1)] {
        let original = gui.fixture.rt.decks[deck].sync;
        let other = gui.fixture.rt.decks[1 - deck].sync;
        let before = gui.command_count();
        gui.stroke(pressed, Default::default(), None);
        assert_eq!(gui.command_count(), before + 1);
        assert_eq!(gui.fixture.rt.decks[deck].sync, !original);
        assert_eq!(gui.fixture.rt.decks[1 - deck].sync, other);
        gui.stroke(pressed, Default::default(), None);
        assert_eq!(gui.fixture.rt.decks[deck].sync, original);
    }
    for (pressed, expected) in [(Key::OpenBracket, 0.0), (Key::CloseBracket, 1.0)] {
        let before = gui.command_count();
        gui.stroke(pressed, Default::default(), None);
        assert_eq!(gui.command_count(), before + 1);
        assert_eq!(gui.fixture.rt.xfader, expected);
    }
}

#[test]
fn restored_load_key_uses_filtered_selection_and_selected_deck() {
    let mut gui = Gui::new();
    gui.fixture.rt.apply(Command::SelectDeck(1));
    gui.fixture.rt.apply(Command::DeckUnload { deck: 1 });
    gui.fixture.rt.publish_for_test();
    gui.fixture.app.lib_filter = "Harmony".into();
    gui.frame(vec![], Default::default());
    let a = gui.fixture.rt.decks[0].audio.clone();
    let before = gui.command_count();
    gui.stroke(Key::F, Default::default(), None);
    assert_eq!(gui.command_count(), before + 1);
    assert!(gui.fixture.rt.decks[1].title.contains("Harmony"));
    assert!(Arc::ptr_eq(gui.fixture.rt.decks[0].audio.as_ref().unwrap(), a.as_ref().unwrap()));
    assert!(!gui.fixture.rt.decks[1].playing);
    assert!(gui.fixture.decoder_jobs.try_recv().is_err(), "builtins do not decode files");
}

#[test]
fn unbound_modifiers_and_repeats_are_rejected_while_modified_brackets_dispatch_jump_controls() {
    let mut gui = Gui::new();
    gui.frame(vec![], Default::default());
    let before = gui.command_count();
    for pressed in [Key::Num1, Key::Num8, Key::W, Key::O, Key::F,
        Key::OpenBracket, Key::CloseBracket] {
        for bits in 1..32 {
            let mods = egui::Modifiers {
                alt: bits & 1 != 0, ctrl: bits & 2 != 0, shift: bits & 4 != 0,
                mac_cmd: bits & 8 != 0, command: bits & 16 != 0,
            };
            let previous = gui.command_count();
            let jump = matches!(pressed, Key::OpenBracket | Key::CloseBracket)
                && (mods == egui::Modifiers::SHIFT || mods == egui::Modifiers::ALT);
            gui.stroke(pressed, mods, None);
            assert_eq!(gui.command_count(), previous + u64::from(jump));
        }
        // egui derives repeat from held-key state, overriding the input flag.
        // Hold a modified (suppressed) press, then release Ctrl while the key
        // keeps repeating; that repeat must not trigger a new global action.
        let previous = gui.command_count();
        gui.frame(vec![key(pressed, true, egui::Modifiers::CTRL)], egui::Modifiers::CTRL);
        let mut repeated = key(pressed, true, Default::default());
        if let egui::Event::Key { repeat, .. } = &mut repeated { *repeat = true; }
        gui.frame(vec![repeated], Default::default());
        gui.frame(vec![key(pressed, false, Default::default())], Default::default());
        assert_eq!(gui.command_count(), previous);
    }
    assert_eq!(gui.command_count(), before + 4);
    assert!(gui.fixture.decoder_jobs.try_recv().is_err());
}

#[test]
fn focused_search_owns_modifiers_selection_cursor_and_deletion_keys() {
    let mut gui = Gui::new();
    gui.focus_search();
    gui.frame(
        vec![egui::Event::Text("editable text".into())],
        Default::default(),
    );
    let before = gui.command_count();
    // Every combination of physical/logical modifier flags: none may turn a
    // focused typing gesture into transport, deck, punctuation-help or MIDI-window actions.
    // Bound F1 is the deliberate contextual-help exception, tested in help/tests.rs.
    for bits in 0..32 {
        let modifiers = egui::Modifiers {
            alt: bits & 1 != 0,
            ctrl: bits & 2 != 0,
            shift: bits & 4 != 0,
            mac_cmd: bits & 8 != 0,
            command: bits & 16 != 0,
        };
        for pressed in [
            Key::Q,
            Key::P,
            Key::A,
            Key::L,
            Key::M,
            Key::Space,
            Key::Slash,
        ] {
            gui.stroke(pressed, modifiers, None);
            assert_eq!(gui.fixture.app.lib_filter, "editable text");
            assert_eq!(gui.command_count(), before);
            assert!(!gui.fixture.app.keys_open && !gui.fixture.app.midi_open);
        }
    }
    let command = egui::Modifiers::CTRL | egui::Modifiers::COMMAND;
    gui.stroke(Key::A, command, None);
    gui.stroke(Key::Backspace, egui::Modifiers::NONE, None);
    assert_eq!(gui.fixture.app.lib_filter, "");
    gui.frame(vec![egui::Event::Paste("qpa l".into())], Default::default());
    gui.stroke(Key::Home, egui::Modifiers::NONE, None);
    gui.stroke(Key::Delete, egui::Modifiers::NONE, None);
    gui.stroke(Key::End, egui::Modifiers::NONE, None);
    gui.stroke(Key::Backspace, egui::Modifiers::NONE, None);
    assert_eq!(gui.fixture.app.lib_filter, "pa ");
    assert_eq!(gui.command_count(), before);
}

#[test]
fn ending_text_focus_does_not_leak_escape_or_same_frame_transport() {
    for ending in [Key::Escape, Key::Enter, Key::Tab] {
        let mut gui = Gui::new();
        gui.focus_search();
        gui.fixture.rt.fx_view = 3;
        let before = gui.command_count();
        gui.frame(
            vec![
                key(ending, true, Default::default()),
                key(Key::Q, true, Default::default()),
            ],
            Default::default(),
        );
        assert_eq!(gui.command_count(), before);
        assert_eq!(gui.fixture.rt.fx_view, 3);
        assert!(!gui.fixture.rt.decks[0].playing);
    }
}

#[test]
fn unfocused_global_bindings_work_and_respect_each_events_modifiers() {
    let mut gui = Gui::new();
    gui.frame(vec![], Default::default());
    let before = gui.command_count();
    for pressed in [Key::Q, Key::P, Key::A, Key::L, Key::Space, Key::Escape] {
        gui.stroke(pressed, Default::default(), None);
    }
    assert_eq!(gui.command_count(), before + 8);
    gui.stroke(Key::F1, Default::default(), None);
    assert!(gui.fixture.app.keys_open);
    gui.stroke(Key::Slash, egui::Modifiers::SHIFT, None);
    assert!(!gui.fixture.app.keys_open);
    gui.stroke(Key::M, egui::Modifiers::CTRL, None);
    assert!(gui.fixture.app.midi_open);

    let before = gui.command_count();
    for modifiers in [
        egui::Modifiers::CTRL,
        egui::Modifiers::ALT,
        egui::Modifiers::SHIFT,
        egui::Modifiers::COMMAND,
    ] {
        for pressed in [Key::Q, Key::P, Key::A, Key::L, Key::Space, Key::Escape] {
            gui.stroke(pressed, modifiers, None);
        }
    }
    assert_eq!(gui.command_count(), before);
    // RawInput's final modifier state cannot rewrite an earlier key event.
    gui.frame(
        vec![key(Key::Q, true, egui::Modifiers::NONE)],
        egui::Modifiers::CTRL,
    );
    assert_eq!(gui.command_count(), before + 1);
    let mut repeat = key(Key::Q, true, Default::default());
    if let egui::Event::Key { repeat, .. } = &mut repeat {
        *repeat = true;
    }
    gui.frame(vec![repeat], Default::default());
    assert_eq!(gui.command_count(), before + 1);
}

#[test]
fn error_dialog_blocks_first_and_dismissal_frame_shortcuts() {
    let mut gui = Gui::new();
    gui.fixture
        .app
        .submission_error
        .set(Some(crate::engine::SubmissionError::Full));
    let before = gui.command_count();
    gui.frame(
        vec![key(Key::Space, true, Default::default())],
        Default::default(),
    );
    assert_eq!(gui.command_count(), before);
    let output = gui.frame(vec![], Default::default());
    let dismiss = label_center(&output, "Dismiss");
    for pressed in [true, false] {
        let mut events = vec![
            egui::Event::PointerMoved(dismiss),
            egui::Event::PointerButton {
                pos: dismiss,
                button: PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            },
        ];
        if !pressed {
            events.push(key(Key::Q, true, Default::default()));
        }
        gui.frame(events, Default::default());
    }
    assert!(gui.fixture.app.submission_error.get().is_none());
    assert_eq!(gui.command_count(), before);
    gui.frame(
        vec![key(Key::Q, false, Default::default())],
        Default::default(),
    );
    gui.stroke(Key::Q, Default::default(), None);
    assert_eq!(gui.command_count(), before + 1);
}

#[test]
fn future_text_fields_and_modals_share_the_same_dispatch_guard() {
    let mut fixture = Fixture::new(32);
    let ctx = egui::Context::default();
    let id = egui::Id::new("future-rename-field");
    let mut text = String::new();
    for (index, events) in [
        vec![],
        vec![
            key(Key::Q, true, Default::default()),
            egui::Event::Text("q".into()),
        ],
    ]
    .into_iter()
    .enumerate()
    {
        let _ = ctx.run(
            egui::RawInput {
                events,
                time: Some(index as f64),
                ..Default::default()
            },
            |ctx| {
                fixture.app.shortcut_focus.begin_frame(ctx);
                egui::Window::new("Rename fixture").show(ctx, |ui| {
                    let response = ui.add(egui::TextEdit::singleline(&mut text).id(id));
                    if index == 0 {
                        response.request_focus();
                    }
                });
                fixture.app.handle_keys(ctx);
            },
        );
    }
    fixture.rt.process(&mut []);
    assert_eq!(text, "q");
    assert_eq!(fixture.rt.command_stats.received, 0);
    // The first frame of a future modal is guarded explicitly; egui only
    // reports its modal layer on the following frame.
    ctx.memory_mut(|memory| memory.stop_text_input());
    fixture.app.shortcut_focus = Default::default();
    let _ = ctx.run(
        egui::RawInput {
            events: vec![key(Key::Space, true, Default::default())],
            time: Some(2.0),
            ..Default::default()
        },
        |ctx| {
            fixture.app.shortcut_focus.begin_frame(ctx);
            keyboard::block_for_dialog(ctx);
            egui::Modal::new(egui::Id::new("future-modal")).show(ctx, |ui| {
                ui.label("Modal fixture");
            });
            fixture.app.handle_keys(ctx);
        },
    );
    fixture.rt.process(&mut []);
    assert_eq!(fixture.rt.command_stats.received, 0);
}
