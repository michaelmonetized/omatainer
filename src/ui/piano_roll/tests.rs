use super::*;
use crate::engine::RtEngine;
use egui::accesskit::{Action, ActionData, ActionRequest, Node, NodeId};
use std::time::Duration;

pub(crate) struct Gui {
    pub(crate) app: App,
    pub(crate) rt: RtEngine,
    pub(crate) ctx: egui::Context,
    pub(crate) nodes: Vec<(NodeId, Node)>,
    time: f64,
    pub(crate) render: bool,
    focused: bool,
    pub(crate) native_close: bool,
    pub(crate) screen: Vec2,
}
impl Gui {
    pub(crate) fn new() -> Self {
        let (engine, mut rt) = Engine::headless_for_test(48_000, 256);
        engine.send(Command::Select { track: 2, scene: 7 }).unwrap();
        rt.process(&mut []);
        rt.publish_for_test();
        let app = App::with_loader(engine, Theme::default(), None);
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut gui = Self {
            app,
            rt,
            ctx,
            nodes: Vec::new(),
            time: 0.0,
            render: true,
            focused: true,
            native_close: false,
            screen: Vec2::new(1440.0, 1400.0),
        };
        for _ in 0..4 {
            gui.frame(vec![]);
        }
        gui.settle();
        gui
    }
    pub(crate) fn frame(&mut self, events: Vec<egui::Event>) -> egui::FullOutput {
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
        let mut input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, self.screen)),
            time: Some(self.time),
            focused: self.focused,
            events,
            modifiers,
            ..Default::default()
        };
        if self.native_close {
            input
                .viewports
                .get_mut(&egui::ViewportId::ROOT)
                .unwrap()
                .events
                .push(egui::ViewportEvent::Close);
            self.native_close = false;
        }
        let output = self.ctx.run(input, |ctx| self.app.update_frame(ctx));
        self.nodes = output
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
        output
    }
    pub(crate) fn settle(&mut self) {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            self.frame(vec![]);
            if !self.app.piano_roll.busy() && !self.app.project_pending_for_test() {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "MIDI/project worker did not settle: {:?} / {:?}",
                self.app.piano_roll.error,
                self.app.project_result_for_test().1
            );
            std::thread::sleep(Duration::from_millis(1));
        }
        self.frame(vec![]);
    }
    pub(crate) fn node(&self, label: &str) -> (NodeId, &Node) {
        self.nodes
            .iter()
            .find(|(_, n)| n.label() == Some(label))
            .map(|(id, n)| (*id, n))
            .unwrap_or_else(|| {
                panic!(
                    "missing {label}; labels {:?}",
                    self.nodes
                        .iter()
                        .filter_map(|(_, n)| n.label())
                        .collect::<Vec<_>>()
                )
            })
    }
    pub(crate) fn action(&mut self, label: &str, action: Action, data: Option<ActionData>) {
        let target = self.node(label).0;
        self.frame(vec![egui::Event::AccessKitActionRequest(ActionRequest {
            action,
            target,
            data,
        })]);
        self.frame(vec![]);
    }
    pub(crate) fn click(&mut self, label: &str) {
        self.action(label, Action::Click, None);
    }
    fn number(&mut self, label: &str, value: f64) {
        self.action(
            &format!("MIDI piano roll: {label}"),
            Action::SetValue,
            Some(ActionData::NumericValue(value)),
        );
    }
    pub(crate) fn key(&mut self, key: Key, modifiers: egui::Modifiers) {
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
    fn pointer(&mut self, pos: Pos2, pressed: bool) {
        self.frame(vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            },
        ]);
    }
    pub(crate) fn click_text(&mut self, text: &str) {
        let output = self.frame(vec![]);
        let pos = test_support::label_center(&output, text);
        self.pointer(pos, true);
        self.pointer(pos, false);
    }
    pub(crate) fn path(&mut self, path: &PathBuf) {
        let old = self
            .app
            .project_path_text_for_test()
            .expect("project path dialog");
        self.click_text(if old.is_empty() {
            "/path/to/session.omat"
        } else {
            &old
        });
        self.key(
            Key::A,
            egui::Modifiers {
                ctrl: true,
                command: true,
                ..Default::default()
            },
        );
        self.frame(vec![egui::Event::Text(path.display().to_string())]);
    }
    fn open_editor(&mut self) {
        self.click("Sampler: Edit selected MIDI clip");
        self.settle();
        assert!(
            self.app.piano_roll.draft.is_some(),
            "{:?}",
            self.app.piano_roll.error
        );
    }
    fn apply(&mut self) {
        self.click("MIDI piano roll: Apply MIDI edit");
        self.settle();
        assert!(
            self.app.piano_roll.error.is_none(),
            "{:?}",
            self.app.piano_roll.error
        );
        assert!(!self.app.piano_roll.draft.as_ref().unwrap().dirty);
    }
}
struct File(PathBuf);
impl File {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!(
            "omat-piano-{}.omat",
            crate::sampler_bank::BankId::new().unwrap()
        )))
    }
}
impl Drop for File {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

#[test]
fn real_accessible_editor_composes_revises_saves_reopens_sixteen_bars_and_matches_emitted_notes() {
    let file = File::new();
    let mut gui = Gui::new();
    gui.open_editor();
    gui.number("Clip end", 64.0);
    gui.number("Loop end", 64.0);
    for index in 0..64 {
        gui.number("Note pitch", [60.0, 64.0, 67.0, 72.0][index % 4]);
        gui.number("Note start", index as f64);
        gui.number("Note length", 0.5);
        gui.number("Note velocity", 80.0 + (index % 16) as f64);
        gui.click("MIDI piano roll: Add note");
    }
    let original_ids: BTreeSet<_> = gui
        .app
        .piano_roll
        .draft
        .as_ref()
        .unwrap()
        .notes
        .iter()
        .map(|n| n.id)
        .collect();
    assert_eq!(original_ids.len(), 64);
    gui.click("MIDI piano roll: Select all notes");
    gui.click("MIDI piano roll: Transpose up");
    gui.click("MIDI piano roll: Lengthen notes");
    assert_eq!(
        gui.app.piano_roll.draft.as_ref().unwrap().selected,
        original_ids
    );
    gui.apply();
    let expected = gui.rt.tracks[2].clips[7].notes.clone();
    assert_eq!(expected.len(), 64);
    assert_eq!(
        expected.iter().map(|n| n.id).collect::<BTreeSet<_>>(),
        original_ids
    );
    gui.click("MIDI piano roll: Cancel / close MIDI editor");
    gui.click_text("Project");
    gui.click_text("Save project as…");
    gui.path(&file.0);
    gui.click_text("Save");
    gui.settle();
    assert_eq!(
        gui.app.project_result_for_test().0.as_ref(),
        Some(&file.0),
        "{:?}",
        gui.app.project_result_for_test().1
    );
    let mut reopened = Gui::new();
    reopened.click_text("Project");
    reopened.click_text("Open project…");
    reopened.path(&file.0);
    reopened.click_text("Open");
    reopened.settle();
    assert_eq!(reopened.rt.tracks[2].clips[7].notes, expected);
    assert_eq!(reopened.rt.tracks[2].clips[7].region.unwrap().end, 64.0);
    reopened.open_editor();
    assert_eq!(
        reopened.app.piano_roll.draft.as_ref().unwrap().notes,
        expected
    );
    reopened.click("MIDI piano roll: Cancel / close MIDI editor");
    reopened.app.engine.send(Command::SetBpm(120.0)).unwrap();
    reopened.app.engine.send(Command::Quant(0.0)).unwrap();
    reopened.rt.process(&mut []);
    reopened.rt.begin_midi_trace_for_test(2);
    reopened
        .app
        .engine
        .send(Command::FireClip {
            track: 2,
            scene: 7,
            looping: false,
        })
        .unwrap();
    for _ in 0..3001 {
        reopened.rt.process(&mut [0.0; 1024]);
    }
    let emitted = reopened.rt.take_midi_trace_for_test(2);
    let mut reference: Vec<_> = expected
        .iter()
        .filter(|n| !n.muted)
        .flat_map(|n| {
            [
                (n.start as f64 * 24_000.0, true, n.pitch, n.vel),
                ((n.start + n.len) as f64 * 24_000.0, false, n.pitch, 0),
            ]
        })
        .map(|(frame, on, pitch, velocity)| ((frame + 1e-7).floor() as u64, on, pitch, velocity))
        .collect();
    reference.sort_by_key(|event| (event.0, event.1, event.2));
    assert_eq!(emitted, reference);
    assert!(reopened.rt.tracks[2].playing.is_none());
}

#[test]
fn real_pointer_and_keyboard_roll_share_stable_selection_and_triplet_free_edits() {
    let mut gui = Gui::new();
    gui.open_editor();
    let bounds = gui
        .node("MIDI piano roll: MIDI piano roll grid")
        .1
        .bounds()
        .unwrap();
    let origin = Pos2::new(bounds.x0 as f32 + 94.0, bounds.y0 as f32 + 52.0);
    let click = origin + Vec2::new(40.0, 8.0 * 18.0 + 9.0);
    gui.pointer(click, true);
    gui.pointer(click, false);
    gui.frame(vec![]); // Register the newly painted note's hit rectangle.
    let draft = gui.app.piano_roll.draft.as_ref().unwrap();
    assert_eq!(draft.notes.len(), 1);
    assert_eq!(
        (
            draft.notes[0].pitch,
            draft.notes[0].start,
            draft.notes[0].len
        ),
        (64, 1.0, 0.25)
    );
    let id = draft.notes[0].id;
    let body = click + Vec2::new(4.0, 0.0);
    gui.pointer(body, true);
    gui.frame(vec![egui::Event::PointerMoved(
        body + Vec2::new(40.0, -18.0),
    )]);
    assert!(
        gui.app.piano_roll.draft.as_ref().unwrap().drag.is_some(),
        "note drag did not start; grid bounds {:?}",
        gui.node("MIDI piano roll: MIDI piano roll grid").1.bounds()
    );
    gui.pointer(body + Vec2::new(40.0, -18.0), false);
    gui.frame(vec![]); // Paint and register the note at its new coordinates.
    let note = &gui.app.piano_roll.draft.as_ref().unwrap().notes[0];
    assert_eq!(
        (note.id, note.pitch, note.start, note.len),
        (id, 65, 2.0, 0.25)
    );
    let edge = origin + Vec2::new(89.0, 7.0 * 18.0 + 9.0);
    gui.pointer(edge, true);
    gui.frame(vec![egui::Event::PointerMoved(edge + Vec2::new(20.0, 0.0))]);
    gui.pointer(edge + Vec2::new(20.0, 0.0), false);
    assert_eq!(
        gui.app.piano_roll.draft.as_ref().unwrap().notes[0].len,
        0.75
    );
    gui.action("MIDI piano roll: MIDI piano roll grid", Action::Focus, None);
    gui.key(Key::ArrowUp, egui::Modifiers::NONE);
    gui.key(Key::ArrowRight, egui::Modifiers::SHIFT);
    gui.key(
        Key::D,
        egui::Modifiers {
            ctrl: true,
            command: true,
            ..Default::default()
        },
    );
    let draft = gui.app.piano_roll.draft.as_ref().unwrap();
    assert_eq!(draft.notes[0].id, id);
    assert_eq!(draft.notes[0].pitch, 66);
    assert_eq!(draft.notes[0].len, 1.0);
    assert_ne!(draft.notes[1].id, id);
    assert_eq!(draft.notes[1].start, 3.0);
    gui.key(Key::M, egui::Modifiers::NONE);
    assert!(gui.app.piano_roll.draft.as_ref().unwrap().notes[1].muted);
    gui.key(Key::Delete, egui::Modifiers::NONE);
    assert_eq!(gui.app.piano_roll.draft.as_ref().unwrap().notes.len(), 1);
    gui.click_text("Sixteenth notes");
    gui.click_text("Eighth triplets");
    gui.click("MIDI piano roll: Select all notes");
    gui.click("MIDI piano roll: Move later");
    assert!(
        (gui.app.piano_roll.draft.as_ref().unwrap().notes[0].start - (2.0 + 1.0 / 3.0)).abs()
            < 1e-6
    );
    gui.click_text("Eighth triplets");
    gui.click_text("Free");
    gui.number("Note start", 2.123456);
    gui.number("Note length", 0.456789);
    gui.number("Note velocity", 73.0);
    gui.click("MIDI piano roll: Set selected note values");
    let note = &gui.app.piano_roll.draft.as_ref().unwrap().notes[0];
    assert_eq!(note.id, id);
    assert!((note.start - 2.123456).abs() < 1e-6);
    assert!((note.len - 0.456789).abs() < 1e-6);
    assert_eq!(note.vel, 73);
    gui.apply();
    assert_eq!(gui.rt.tracks[2].clips[7].notes[0].id, id);
}

#[test]
fn real_editor_keeps_captured_destination_and_preserves_unrelated_gain_then_rejects_stale_content()
{
    let mut gui = Gui::new();
    gui.open_editor();
    gui.app
        .engine
        .send(Command::Select { track: 3, scene: 1 })
        .unwrap();
    gui.app
        .engine
        .send(Command::ClipGain {
            track: 2,
            scene: 7,
            value: 0.42,
        })
        .unwrap();
    gui.frame(vec![]);
    gui.click("MIDI piano roll: Add note");
    gui.apply();
    assert_eq!(gui.rt.tracks[2].clips[7].notes.len(), 1);
    assert!(gui.rt.tracks[3].clips[1].notes.is_empty());
    assert_eq!(gui.rt.tracks[2].clips[7].gain, 0.42);
    let mut replacement = gui.rt.tracks[2].clips[7].notes[0].clone();
    replacement.pitch = 71;
    replacement.id = NoteId::new();
    gui.app
        .engine
        .send(Command::SetNotes {
            track: 2,
            scene: 7,
            notes: vec![replacement.clone()],
        })
        .unwrap();
    gui.frame(vec![]);
    gui.click("MIDI piano roll: Transpose up");
    gui.click("MIDI piano roll: Apply MIDI edit");
    gui.settle();
    assert!(gui
        .app
        .piano_roll
        .error
        .as_ref()
        .unwrap()
        .contains("rejected"));
    assert_eq!(gui.rt.tracks[2].clips[7].notes, vec![replacement]);
    assert!(gui.app.piano_roll.draft.as_ref().unwrap().dirty);
}

#[test]
fn native_close_and_explicit_discard_cancel_a_queued_edit_before_renderer_ownership() {
    let mut gui = Gui::new();
    gui.open_editor();
    gui.click("MIDI piano roll: Add note");
    let before = gui.app.engine.undo.checkpoint();
    gui.render = false;
    gui.click("MIDI piano roll: Apply MIDI edit");
    let ack = gui.app.piano_roll.pending.as_ref().unwrap().ack.clone();
    assert_eq!(ack.state(), Outcome::Pending);
    gui.native_close = true;
    let output = gui.frame(vec![]);
    gui.frame(vec![]);
    assert!(output.viewport_output[&egui::ViewportId::ROOT]
        .commands
        .iter()
        .any(|c| matches!(c, egui::ViewportCommand::CancelClose)));
    assert!(gui.app.piano_roll.blocks_close());
    gui.click("MIDI piano roll: Discard MIDI draft");
    assert_eq!(ack.state(), Outcome::Cancelled);
    gui.render = true;
    gui.settle();
    assert!(gui.rt.tracks[2].clips[7].notes.is_empty());
    assert_eq!(gui.app.engine.undo.checkpoint(), before);
    assert!(!gui.app.piano_roll.blocks_close());
}

#[test]
fn real_audition_focus_loss_preserves_physical_gate_and_never_records_editor_notes() {
    use crate::engine::dsp::InputKey;
    let mut gui = Gui::new();
    gui.open_editor();
    gui.app
        .engine
        .send(Command::LiveNoteOn {
            source: 999,
            ch: 1,
            note: 60,
            vel: 80,
        })
        .unwrap();
    gui.frame(vec![]);
    gui.rt.recording = true;
    gui.rt.playing = true;
    gui.click("MIDI piano roll: Audition note");
    assert!(gui.app.piano_roll.audition.is_some());
    assert!(gui.rt.tracks[2].clips[7].notes.is_empty());
    assert!(gui.rt.tracks[2]
        .poly
        .voices
        .iter()
        .any(|v| matches!(v.input, Some(InputKey::Preview(_))) && matches!(v.env.stage, 1..=3)));
    gui.focused = false;
    gui.frame(vec![]);
    gui.frame(vec![]);
    assert!(gui.app.piano_roll.audition.is_none());
    assert!(gui.rt.tracks[2]
        .poly
        .voices
        .iter()
        .all(|v| !matches!(v.input, Some(InputKey::Preview(_))) || !matches!(v.env.stage, 1..=3)));
    assert!(gui.rt.tracks[2].poly.voices.iter().any(|v| v.input
        == Some(InputKey::Midi {
            source: 999,
            ch: 1,
            note: 60
        })
        && matches!(v.env.stage, 1..=3)));
    assert!(gui.rt.tracks[2].clips[7].notes.is_empty());
}

#[test]
fn clip_loop_markers_numeric_view_and_scale_folding_are_real_accessible_controls() {
    let mut gui = Gui::new();
    gui.open_editor();
    gui.number("Time zoom", 8.0);
    gui.number("Top pitch", 67.0);
    gui.click_text("All pitches");
    gui.click_text("Major scale");
    gui.number("Scale root", 2.0); // D major; existing off-scale notes remain visible.
    gui.number("Note pitch", 60.0);
    gui.click("MIDI piano roll: Add note");
    gui.frame(vec![]);
    assert!(canvas::pitches(gui.app.piano_roll.draft.as_ref().unwrap()).contains(&60));
    let bounds = gui
        .node("MIDI piano roll: Loop start marker")
        .1
        .bounds()
        .unwrap();
    let marker = Pos2::new(
        ((bounds.x0 + bounds.x1) / 2.0) as f32,
        ((bounds.y0 + bounds.y1) / 2.0) as f32,
    );
    gui.pointer(marker, true);
    gui.frame(vec![egui::Event::PointerMoved(
        marker + Vec2::new(32.0, 0.0),
    )]);
    gui.pointer(marker + Vec2::new(32.0, 0.0), false);
    gui.frame(vec![]);
    assert_eq!(
        gui.app.piano_roll.draft.as_ref().unwrap().region.loop_start,
        4.0
    );
    gui.number("Clip start", 2.0);
    gui.number("Loop end", 48.0);
    gui.number("Clip end", 56.0);
    gui.click("MIDI piano roll: Loop enabled");
    gui.apply();
    assert_eq!(
        gui.rt.tracks[2].clips[7].region,
        Some(Region {
            start: 2.0,
            end: 56.0,
            loop_start: 4.0,
            loop_end: 48.0,
            loop_enabled: false
        })
    );
    gui.number("Time scroll", 32.0);
    gui.number("Pitch zoom", 24.0);
    assert_eq!(gui.app.piano_roll.draft.as_ref().unwrap().view_beat, 32.0);
    assert_eq!(gui.app.piano_roll.draft.as_ref().unwrap().row_pixels, 24.0);
}

#[test]
fn draft_operations_keep_existing_identity_and_give_copies_fresh_identity() {
    let baseline = Arc::new(Document {
        track_identity: None, scene_identity: None,
        lanes: None,
        track: 2,
        scene: 7,
        epoch: 1,
        kind: crate::engine::ClipKind::Empty,
        name: String::new(),
        bars: 1.0,
        region: None,
        notes: Vec::new(),
    });
    let mut draft = Draft::new(baseline);
    draft.add().unwrap();
    let first = draft.notes[0].id;
    draft.transform(1.0, 2, 0.25, false).unwrap();
    assert_eq!(draft.notes[0].id, first);
    assert_eq!(
        (
            draft.notes[0].pitch,
            draft.notes[0].start,
            draft.notes[0].len
        ),
        (62, 1.0, 0.5)
    );
    draft.duplicate().unwrap();
    assert_ne!(draft.notes[1].id, first);
    assert_eq!(draft.notes[1].start, 1.5);
    draft.transform(0.0, 0, 0.0, true).unwrap();
    assert!(!draft.notes[0].muted);
    assert!(draft.notes[1].muted);
    draft.delete();
    assert_eq!(draft.notes.len(), 1);
    assert_eq!(draft.notes[0].id, first);
}
