use super::*;
use crate::engine::RtEngine;

struct Gui {
    app: App,
    rt: RtEngine,
    ctx: egui::Context,
    time: f64,
}
impl Gui {
    fn new() -> Self {
        let (engine, rt) = Engine::headless_for_test(48_000, 256);
        Self {
            app: App::with_loader(engine, Theme::default(), None),
            rt,
            ctx: egui::Context::default(),
            time: 0.0,
        }
    }
    fn frame(&mut self, events: Vec<egui::Event>, modifiers: egui::Modifiers) -> egui::FullOutput {
        self.time += 0.02;
        let out = self.ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1440.0, 1000.0))),
                time: Some(self.time),
                events,
                modifiers,
                ..Default::default()
            },
            |ctx| self.app.update_frame(ctx),
        );
        self.rt.process(&mut [0.0; 128]);
        self.rt.publish();
        out
    }
    fn idle(&mut self) -> egui::FullOutput {
        self.frame(vec![], Default::default())
    }
    fn stroke(&mut self, key: Key, modifiers: egui::Modifiers) {
        for pressed in [true, false] {
            self.frame(
                vec![egui::Event::Key {
                    key,
                    physical_key: Some(key),
                    pressed,
                    repeat: false,
                    modifiers,
                }],
                modifiers,
            );
        }
    }
    fn point(&mut self, pos: Pos2, pressed: Option<bool>) -> egui::FullOutput {
        let mut events = vec![egui::Event::PointerMoved(pos)];
        if let Some(pressed) = pressed {
            events.push(egui::Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            });
        }
        self.frame(events, Default::default())
    }
    fn click(&mut self, label: &str) {
        self.idle();
        let out = self.idle();
        let pos = test_support::label_center(&out, label);
        self.point(pos, Some(true));
        self.point(pos, Some(false));
    }
    fn refresh(&mut self) {
        for _ in 0..3 {
            self.idle();
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }
}

#[test]
fn actual_keyboard_undo_redo_and_edit_menu_follow_renderer_history() {
    let mut gui = Gui::new();
    let original = gui.rt.master;
    gui.app.send(Command::Master(0.27));
    gui.refresh();
    assert_eq!(gui.app.engine.undo.view().cursor, 1);
    gui.stroke(Key::Z, egui::Modifiers::CTRL);
    assert_eq!(gui.rt.master, original);
    assert_eq!(gui.app.engine.undo.view().cursor, 0);
    gui.stroke(Key::Z, egui::Modifiers::CTRL | egui::Modifiers::SHIFT);
    assert_eq!(gui.rt.master, 0.27);
    gui.refresh();
    gui.click("Edit");
    gui.click("Undo Set master gain");
    assert_eq!(gui.rt.master, original);
    gui.refresh();
    gui.stroke(Key::Y, egui::Modifiers::CTRL);
    assert_eq!(gui.rt.master, 0.27);
    gui.refresh();
    gui.click("Edit");
    gui.click("History…");
    let out = gui.idle();
    test_support::label_center(&out, "Applied 1. Set master gain");
    assert!(gui.undo_message().contains("queued"));
}
impl Gui {
    fn undo_message(&self) -> &str {
        self.app.undo_history.queued.unwrap_or_default()
    }
}

#[test]
fn text_focus_keeps_ctrl_z_inside_search_and_modal_replacement_blocks_history() {
    let mut gui = Gui::new();
    gui.app.send(Command::Master(0.31));
    gui.refresh();
    gui.click("search");
    gui.frame(
        vec![egui::Event::Text("Harmony".into())],
        Default::default(),
    );
    let before = gui.app.engine.undo.checkpoint();
    gui.stroke(Key::Z, egui::Modifiers::CTRL);
    assert_eq!(gui.rt.master, 0.31);
    assert_eq!(gui.app.engine.undo.checkpoint(), before);
    gui.stroke(Key::Escape, Default::default());
    gui.refresh();
    gui.click("Project");
    gui.click("New project");
    gui.stroke(Key::Z, egui::Modifiers::CTRL);
    assert_eq!(gui.rt.master, 0.31);
    gui.click("Cancel");
    gui.refresh();
    gui.stroke(Key::Z, egui::Modifiers::CTRL);
    assert_ne!(gui.rt.master, 0.31);
}

fn crossfader_thumb(output: &egui::FullOutput) -> Pos2 {
    output
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::epaint::Shape::Rect(rect)
                if (rect.rect.width() - 12.0).abs() < 0.01
                    && (rect.rect.height() - 14.0).abs() < 0.01 =>
            {
                Some(rect.rect.center())
            }
            _ => None,
        })
        .expect("actual painted crossfader thumb")
}

#[test]
fn actual_pointer_drag_is_one_transaction_and_intervening_external_edit_breaks_group() {
    let mut gui = Gui::new();
    gui.refresh();
    let original = gui.rt.xfader;
    let start = crossfader_thumb(&gui.idle());
    gui.point(start, Some(true));
    for x in [20.0, 40.0, 60.0] {
        gui.point(start + Vec2::new(x, 0.0), None);
    }
    gui.point(start + Vec2::new(60.0, 0.0), Some(false));
    let end = gui.rt.xfader;
    assert!(end > original);
    assert_eq!(gui.app.engine.undo.view().cursor, 1);
    gui.stroke(Key::Z, egui::Modifiers::CTRL);
    assert_eq!(gui.rt.xfader, original);
    gui.stroke(Key::Y, egui::Modifiers::CTRL);
    assert_eq!(gui.rt.xfader, end);
    gui.refresh();
    let start = crossfader_thumb(&gui.idle());
    gui.point(start, Some(true));
    gui.point(start - Vec2::new(10.0, 0.0), None);
    let first = gui.rt.xfader;
    gui.app.engine.send(Command::Master(0.61)).unwrap();
    gui.rt.process(&mut []);
    gui.point(start - Vec2::new(35.0, 0.0), None);
    gui.point(start - Vec2::new(35.0, 0.0), Some(false));
    assert_eq!(gui.app.engine.undo.view().cursor, 4);
    gui.stroke(Key::Z, egui::Modifiers::CTRL);
    assert_eq!(gui.rt.xfader, first);
    assert_eq!(
        gui.rt.master, 0.61,
        "unrelated admitted edit remains applied"
    );
}

#[test]
fn rejected_admission_keeps_history_and_reports_failure_in_actual_gui() {
    let mut gui = Gui::new();
    gui.app.send(Command::Master(0.29));
    gui.refresh();
    let before = gui.app.engine.undo.checkpoint();
    while gui.app.engine.send(Command::Master(0.4)).is_ok() {}
    let _ = gui.ctx.run(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1440.0, 1000.0))),
            ..Default::default()
        },
        |ctx| {
            gui.app.history_action(false);
            gui.app.update_frame(ctx);
        },
    );
    assert!(gui.app.submission_error.get().is_some());
    assert_eq!(gui.app.engine.undo.checkpoint(), before);
    assert!(gui.app.undo_history.queued.is_none());
}

#[test]
fn renderer_rejected_note_edit_is_visible_without_altering_history_or_notes() {
    let mut gui = Gui::new();
    gui.refresh();
    let original = gui.rt.tracks[0].clips[0].notes.clone();
    let before = gui.app.engine.undo.checkpoint();
    gui.app.send(Command::SetNotes {
        track: 0,
        scene: 0,
        notes: vec![
            crate::engine::MidiNote {
                id: crate::engine::midi_edit::NoteId::new(), muted: false,
                pitch: 60,
                start: 0.0,
                len: 0.25,
                vel: 100
            };
            8193
        ],
    });
    gui.refresh();
    assert_eq!(gui.rt.tracks[0].clips[0].notes.len(), original.len());
    for (current, old) in gui.rt.tracks[0].clips[0].notes.iter().zip(original) {
        assert_eq!(
            (current.pitch, current.start, current.len, current.vel),
            (old.pitch, old.start, old.len, old.vel)
        );
    }
    assert_eq!(gui.app.engine.undo.checkpoint(), before);
    let error = gui
        .app
        .undo_history
        .failure
        .clone()
        .expect("renderer rejection is retained");
    assert!(error.contains("8192"));
    assert!(
        !gui.app.undo_history.open,
        "automatic error display does not alter saved panel view"
    );
    gui.click("Dismiss history message");
    assert!(gui.app.undo_history.failure.is_none());
    assert!(!gui.app.undo_history.open);
}
