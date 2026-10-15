use super::*;
use crate::engine::RtEngine;
use egui::accesskit::{
    Action, ActionData, ActionRequest, Invalid, Live, Node, NodeId, Role, Toggled,
};
use std::sync::atomic::AtomicBool;
use std::time::Duration;

struct Native {
    ctx: egui::Context,
    time: f64,
    nodes: Vec<(NodeId, Node)>,
    focus: Option<NodeId>,
    shapes: Vec<egui::epaint::ClippedShape>,
}
impl Native {
    fn new() -> Self {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        Self {
            ctx,
            time: 0.0,
            nodes: vec![],
            focus: None,
            shapes: vec![],
        }
    }
    fn frame(
        &mut self,
        events: Vec<egui::Event>,
        modifiers: egui::Modifiers,
        draw: impl FnMut(&egui::Context),
    ) {
        self.time += 0.02;
        let output = self.ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1800.0, 1800.0))),
                time: Some(self.time),
                focused: true,
                modifiers,
                events,
                ..Default::default()
            },
            draw,
        );
        let update = output.platform_output.accesskit_update.unwrap();
        self.nodes = update.nodes;
        self.focus = Some(update.focus);
        self.shapes = output.shapes;
    }
    #[track_caller]
    fn node(&self, label: &str) -> (NodeId, &Node) {
        self.nodes
            .iter()
            .find(|(_, node)| {
                node.label() == Some(label)
                    && node.role() != Role::Window
                    && (node.supports_action(Action::Focus) || node.is_disabled())
            })
            .map(|(id, node)| (*id, node))
            .unwrap_or_else(|| {
                panic!(
                    "Missing native widget {label:?}; focus {:?}; names {:?}",
                    self.focus,
                    self.nodes
                        .iter()
                        .filter_map(|(_, node)| node.label())
                        .collect::<Vec<_>>()
                )
            })
    }
    #[track_caller]
    fn focused(&self, label: &str) {
        let id = self.node(label).0;
        assert_eq!(
            self.ctx
                .memory(|memory| memory.focused())
                .map(|id| NodeId::from(id.value())),
            Some(id),
            "native Memory focus for {label}"
        );
        assert_eq!(self.focus, Some(id), "native AccessKit focus for {label}");
    }
    #[track_caller]
    fn focused_panel(&self, label: &str) {
        assert_eq!(self.node(label).1.role(), Role::Button);
        self.focused(label);
        let id = self.ctx.memory(|memory| memory.focused()).unwrap();
        let response = self.ctx.read_response(id).unwrap();
        assert!(
            self.shapes.iter().any(|shape| matches!(&shape.shape,
            egui::epaint::Shape::Rect(rect) if rect.stroke.width == 2.0
                && rect.rect == response.rect.expand(2.0))),
            "visible native panel focus ring for {label}"
        );
    }
    fn label_containing(&self, needle: &str) -> String {
        self.nodes
            .iter()
            .filter(|(_, node)| node.supports_action(Action::Focus))
            .filter_map(|(_, node)| node.label())
            .find(|label| label.contains(needle))
            .unwrap_or_else(|| panic!("Missing native {needle}"))
            .to_owned()
    }
}
trait Keyboard {
    fn native(&self) -> &Native;
    fn frame(&mut self, events: Vec<egui::Event>, modifiers: egui::Modifiers);
    fn idle(&mut self) {
        self.frame(vec![], egui::Modifiers::NONE);
    }
    fn key(&mut self, key: Key, modifiers: egui::Modifiers) {
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
        self.idle();
    }
    #[track_caller]
    fn tab_to(&mut self, label: &str) {
        for _ in 0..600 {
            let focused = self
                .native()
                .ctx
                .memory(|memory| memory.focused())
                .map(|id| NodeId::from(id.value()));
            if self
                .native()
                .nodes
                .iter()
                .any(|(id, node)| Some(*id) == focused && node.label() == Some(label))
            {
                self.native().focused(label);
                return;
            }
            self.key(Key::Tab, egui::Modifiers::NONE);
        }
        panic!(
            "Tab could not reach {label:?}; native focus {:?}",
            self.native().focus
        );
    }
    #[track_caller]
    fn press(&mut self, label: &str) {
        self.tab_to(label);
        self.key(Key::Enter, egui::Modifiers::NONE);
    }
    #[track_caller]
    fn text(&mut self, label: &str, text: &str) {
        self.tab_to(label);
        self.key(Key::A, egui::Modifiers::CTRL | egui::Modifiers::COMMAND);
        self.frame(vec![egui::Event::Text(text.into())], egui::Modifiers::NONE);
    }
    #[track_caller]
    fn number(&mut self, label: &str, text: &str) {
        self.text(label, text);
        self.key(Key::Enter, egui::Modifiers::NONE);
    }
}
struct Gui {
    native: Native,
    app: App,
    rt: Box<RtEngine>,
    root: PathBuf,
}
impl Gui {
    fn new() -> Self {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join(".artifacts/keyboard-access")
            .join(crate::sampler_bank::BankId::new().unwrap().to_string());
        std::fs::create_dir_all(&root).unwrap();
        let (engine, rt) = Engine::headless_for_test(48000, 256);
        let mut app = App::with_loader(engine, Theme::default(), None);
        app.settings = preferences::Settings::with_worker_for_test(root.join("preferences.json"));
        app.library_metadata =
            library_metadata::Metadata::with_hook(root.join("catalog.json"), || {});
        app.library_metadata
            .set_performance(app.engine.cmd.performance().clone());
        app.library_initialized = true;
        let mut gui = Self {
            native: Native::new(),
            app,
            rt: Box::new(rt),
            root,
        };
        for _ in 0..4 {
            gui.idle();
        }
        gui
    }
    #[track_caller]
    fn until(&mut self, mut ready: impl FnMut(&Self) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(15);
        while !ready(self) {
            self.idle();
            assert!(
                Instant::now() < deadline,
                "native worker/route did not settle; {:?}",
                self.app.project_result_for_test()
            );
            std::thread::sleep(Duration::from_millis(1));
        }
        self.idle();
        self.idle();
    }
    fn has(&self, label: &str) -> bool {
        self.native
            .nodes
            .iter()
            .any(|(_, node)| node.label() == Some(label))
    }
    fn enabled(&self, label: &str) -> bool {
        self.has(label) && !self.native.node(label).1.is_disabled()
    }
    fn cursor_values(&self) -> (u8, f32, f32, u8) {
        let cursor = self.app.piano_roll.draft.as_ref().unwrap().cursor;
        (cursor.pitch, cursor.start, cursor.length, cursor.velocity)
    }
    fn route(&mut self, label: &str) {
        self.idle();
        self.idle();
        let key = command_palette::chord(self.app.settings.profile()).unwrap();
        self.key(key, egui::Modifiers::CTRL | egui::Modifiers::SHIFT);
        assert!(self.app.command_palette.open, "ordinary palette chord");
        self.native.focused("Search commands");
        self.frame(vec![egui::Event::Text(label.into())], egui::Modifiers::NONE);
        self.key(Key::Enter, egui::Modifiers::NONE);
        assert!(!self.app.command_palette.open);
    }
    fn menu(&mut self, label: &str) {
        self.tab_to("Project");
        self.key(Key::Space, egui::Modifiers::NONE);
        self.press(label);
    }
    fn new_project(&mut self) {
        let namespace = self.rt.session.namespace;
        self.menu("New project");
        if self.has("Discard changes") {
            self.press("Discard changes");
        }
        self.until(|gui| {
            !gui.app.project_pending_for_test() && gui.rt.session.namespace != namespace
        });
        assert!(self
            .rt
            .tracks
            .iter()
            .flat_map(|track| &track.clips)
            .all(|clip| clip.kind == crate::engine::ClipKind::Empty));
    }
    fn save(&mut self, path: &std::path::Path) {
        self.menu("Save project as…");
        self.text("Project file path", path.to_str().unwrap());
        self.press("Save");
        self.until(|gui| path.exists() && !gui.app.project_pending_for_test());
        assert_eq!(self.app.project_result_for_test().0.as_deref(), Some(path));
    }
    fn reopen(&mut self, path: &std::path::Path) {
        self.new_project();
        self.menu("Open project…");
        self.text("Project file path", path.to_str().unwrap());
        self.press("Open");
        self.until(|gui| {
            !gui.app.project_pending_for_test()
                && gui.app.project_result_for_test().0.as_deref() == Some(path)
        });
    }
    fn floating_rect(&self, panel: &str) -> Rect {
        let book = &self.app.settings.profile().workspaces.studio;
        let layout = &book.tasks[&book.active];
        let (index, group) = layout
            .groups
            .iter()
            .enumerate()
            .find(|(_, group)| {
                group.place == crate::panel_workspace::Place::Floating
                    && group.panels.iter().any(|id| id == panel)
            })
            .unwrap();
        let id = egui::Id::new("omatainer-studio")
            .with(index)
            .with(group.panels.first());
        let actual = self
            .native
            .ctx
            .memory(|memory| memory.area_rect(id))
            .expect("actual rendered native floating window");
        assert!(
            (actual.width() - group.rect[2]).abs() < 1.0
                && (actual.height() - group.rect[3]).abs() < 1.0
        );
        actual
    }
}
impl Keyboard for Gui {
    fn native(&self) -> &Native {
        &self.native
    }
    fn frame(&mut self, events: Vec<egui::Event>, modifiers: egui::Modifiers) {
        self.rt.publish_for_test();
        self.native
            .frame(events, modifiers, |ctx| self.app.update_frame(ctx));
        self.rt.process(&mut [0.0; 128]);
        self.rt.publish_for_test();
    }
}
impl Drop for Gui {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn keyboard_access_empty_project_tracks_clips_notes_controllers_song_timing_save_reopen() {
    let mut gui = Gui::new();
    assert_eq!((gui.rt.selected_track, gui.rt.selected_scene), (0, 0));
    gui.new_project();
    let tracks = gui.rt.session.track_order.len();
    let scenes = gui.rt.session.scene_order.len();
    gui.press("+ MIDI track");
    gui.until(|gui| gui.rt.session.track_order.len() == tracks + 1 && gui.enabled("+ MIDI track"));
    gui.number("Go to track", &(tracks + 1).to_string());
    gui.press("+ Scene");
    gui.until(|gui| gui.rt.session.scene_order.len() == scenes + 1 && gui.enabled("+ Scene"));
    gui.number("Go to scene", &(scenes + 1).to_string());
    let track = gui.rt.selected_track;
    let scene = gui.rt.selected_scene;
    assert_eq!(
        track,
        usize::from(*gui.rt.session.track_order.last().unwrap())
    );
    assert_eq!(
        scene,
        usize::from(*gui.rt.session.scene_order.last().unwrap())
    );
    assert_eq!(
        gui.rt.tracks[track].clips[scene].kind,
        crate::engine::ClipKind::Empty
    );
    gui.route("Edit tracks and scenes");
    gui.text("Session track name", "Keyboard track");
    gui.press("Rename track");
    gui.until(|gui| gui.rt.tracks[track].name == "Keyboard track" && gui.enabled("Rename track"));
    let position_id = gui.native.node("Session position").0;
    let rename_id = gui.native.node("Rename track").0;
    gui.tab_to("Use custom color");
    gui.key(Key::Space, egui::Modifiers::NONE);
    assert_eq!(
        gui.native.node("Session position").0,
        position_id,
        "RGB row preserves native property identity"
    );
    assert_eq!(
        gui.native.node("Rename track").0,
        rename_id,
        "RGB row preserves native command identity"
    );
    gui.key(Key::Space, egui::Modifiers::NONE);
    gui.press("Close Session editor");
    gui.route("Create and edit Session clips");
    gui.until(|gui| gui.enabled("Create MIDI clip"));
    gui.text("Clip name", "Keyboard phrase");
    gui.number("New MIDI clip bars", "4");
    gui.press("Create MIDI clip");
    gui.until(|gui| {
        gui.rt.tracks[track].clips[scene].kind == crate::engine::ClipKind::Midi
            && gui.enabled("Apply clip properties")
    });
    gui.press("Close Session clips");
    gui.route("Create and edit MIDI notes and controller lanes");
    gui.until(|gui| gui.enabled("MIDI piano roll: Add note"));
    for (pitch, start, length, velocity) in [
        ("67", "1.250001", "0.500001", "91"),
        ("72", "4", "0.75", "103"),
    ] {
        gui.number("MIDI piano roll: Note pitch", pitch);
        gui.number("MIDI piano roll: Note start (beats)", start);
        gui.number("MIDI piano roll: Note length (beats)", length);
        gui.number("MIDI piano roll: Note velocity", velocity);
        let expected = (
            pitch.parse::<u8>().unwrap(),
            start.parse::<f32>().unwrap(),
            length.parse::<f32>().unwrap(),
            velocity.parse::<u8>().unwrap(),
        );
        assert_eq!(
            gui.cursor_values(),
            expected,
            "native note inputs before Add"
        );
        gui.press("MIDI piano roll: Add note");
        let added = gui
            .app
            .piano_roll
            .draft
            .as_ref()
            .unwrap()
            .notes
            .last()
            .unwrap();
        assert_eq!(
            (added.pitch, added.start, added.len, added.vel),
            expected,
            "Add retains exact native cursor values: {added:?}"
        );
    }
    gui.press("MIDI piano roll: Select all notes");
    assert_eq!(
        gui.native
            .node("MIDI piano roll: Note pitch")
            .1
            .state_description(),
        Some("Mixed")
    );
    assert!(gui
        .native
        .node("MIDI piano roll: Note pitch")
        .1
        .numeric_value()
        .is_none());
    gui.press("MIDI piano roll: Clear selection");
    let note = gui.native.label_containing("Note 1:");
    let note_id = gui.native.node(&note).0;
    gui.press(&note);
    assert_eq!(gui.native.node(&note).1.toggled(), Some(Toggled::True));
    assert_eq!(
        gui.cursor_values(),
        (67, 1.250001_f32, 0.500001_f32, 91),
        "native note selection restores its cursor values"
    );
    gui.number("MIDI piano roll: Note start (beats)", "9.125001");
    assert_eq!(
        gui.cursor_values(),
        (67, 9.125001_f32, 0.500001_f32, 91),
        "precise native start edit preserves other cursor values"
    );
    gui.press("MIDI piano roll: Set selected note values");
    let changed = &gui.app.piano_roll.draft.as_ref().unwrap().notes[0];
    assert_eq!(
        (changed.pitch, changed.start, changed.len, changed.vel),
        (67, 9.125001_f32, 0.500001_f32, 91),
        "Set retains selected values: {changed:?}"
    );
    let moved = gui.native.label_containing("Note 1:");
    assert_eq!(
        gui.native.node(&moved).0,
        note_id,
        "saved NoteId survives time edits"
    );
    gui.tab_to("MIDI controller lanes");
    gui.key(Key::Space, egui::Modifiers::NONE);
    for (beat, value) in [("2", "80"), ("1", "60")] {
        gui.number("MIDI piano roll: Controller beat (beats)", beat);
        gui.number("MIDI piano roll: Controller value", value);
        gui.press("MIDI piano roll: Insert controller point");
    }
    let point = gui.native.label_containing("2.000000000 beats · value 80");
    let point_id = gui.native.node(&point).0;
    gui.press(&point);
    gui.number("MIDI piano roll: Controller beat (beats)", "0.5");
    gui.press("MIDI piano roll: Set selected controller point");
    let point = gui.native.label_containing("0.500000000 beats · value 80");
    assert_eq!(
        gui.native.node(&point).0,
        point_id,
        "wire order keeps native identity after sorting"
    );
    let unchanged = &gui.app.piano_roll.draft.as_ref().unwrap().notes[0];
    assert_eq!(
        (
            unchanged.pitch,
            unchanged.start,
            unchanged.len,
            unchanged.vel
        ),
        (67, 9.125001_f32, 0.500001_f32, 91),
        "controller edits preserve notes: {unchanged:?}"
    );
    gui.press("MIDI piano roll: Apply MIDI edit");
    gui.until(|gui| {
        gui.rt.tracks[track].clips[scene].notes.len() == 2
            && gui.enabled("MIDI piano roll: Apply MIDI edit")
    });
    let notes = gui.rt.tracks[track].clips[scene].notes.clone();
    assert!(
        notes.iter().any(|note| note.pitch == 67
            && note.start == 9.125001_f32
            && note.len == 0.500001_f32
            && note.vel == 91),
        "renderer retains exact authored notes: {notes:?}"
    );
    assert_eq!(
        gui.rt.tracks[track].clips[scene]
            .lanes
            .as_ref()
            .unwrap()
            .messages
            .len(),
        2
    );
    gui.press("MIDI piano roll: Cancel / close MIDI editor");
    gui.route("Create and edit arrangement instances");
    gui.until(|gui| gui.enabled("Retain Session source"));
    gui.press("Retain Session source");
    gui.number("Range start", "8");
    gui.press("Place source at range start");
    gui.number("Instance start beat", "12.125001");
    assert_eq!(
        gui.native.node("Instance start beat").1.numeric_value(),
        Some(12.125001)
    );
    gui.number("Instance duration beats", "4.5");
    gui.press("Apply song");
    gui.until(|gui| {
        gui.rt
            .arrangement
            .plan
            .as_ref()
            .is_some_and(|plan| plan.model.instances.len() == 1)
            && gui.enabled("Apply song")
    });
    gui.press("Close arrangement editor");
    gui.route("Edit tempo and meter automation");
    gui.text("Tempo points: beat BPM step/ramp", "0 123 step\n8 138 step");
    gui.press("Apply timing");
    gui.until(|gui| {
        gui.rt
            .conductor
            .as_ref()
            .is_some_and(|map| map.tempos.len() == 2)
            && gui.enabled("Apply timing")
    });
    gui.press("Close timing editor");
    let path = gui.root.join("keyboard-authored.omat");
    gui.save(&path);
    let saved = crate::project_file::load::<project::Document>(
        &path,
        &crate::project_file::Limits::default(),
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(saved.state.engine.tracks[track].clips[scene].notes, notes);
    assert_eq!(
        saved.state.engine.arrangement.as_ref().unwrap().instances[0].start,
        12.125001
    );
    gui.reopen(&path);
    assert_eq!(gui.rt.tracks[track].name, "Keyboard track");
    assert_eq!(gui.rt.tracks[track].clips[scene].notes, notes);
    assert_eq!(
        gui.rt.tracks[track].clips[scene]
            .lanes
            .as_ref()
            .unwrap()
            .messages
            .len(),
        2
    );
    assert_eq!(
        gui.rt.arrangement.plan.as_ref().unwrap().model.instances[0].duration,
        4.5
    );
    assert_eq!(gui.rt.conductor.as_ref().unwrap().tempos.len(), 2);
    assert!(!gui.rt.playing && gui.rt.decks.iter().all(|deck| !deck.playing));
}

struct Fields {
    native: Native,
    preceding: bool,
    amount: f64,
    accepted: usize,
    locked: bool,
    live: bool,
    owner: u64,
    mixed: bool,
    second: bool,
    second_value: f64,
}
impl Fields {
    fn new() -> Self {
        let mut fields = Self {
            native: Native::new(),
            preceding: false,
            amount: 1.0,
            accepted: 0,
            locked: false,
            live: true,
            owner: 17,
            mixed: false,
            second: false,
            second_value: 2.0,
        };
        fields.idle();
        fields
    }
}
impl Keyboard for Fields {
    fn native(&self) -> &Native {
        &self.native
    }
    fn frame(&mut self, events: Vec<egui::Event>, modifiers: egui::Modifiers) {
        self.native.frame(events, modifiers, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                form(
                    ui,
                    egui::Id::new(("native-typed-fixture", [3_u64, 7], self.owner)),
                    |ui| {
                        ui.checkbox(&mut self.preceding, "Show preceding row");
                        if self.preceding {
                            let mut before = 3.0;
                            number(ui, "Earlier", &mut before, 0.0, 10.0, 0.01, 6, "");
                        }
                        ui.add_enabled_ui(!self.locked && self.live, |ui| {
                            mixed(ui, "Amount", self.mixed);
                            number(ui, "Amount", &mut self.amount, 0.0, 10.0, 0.01, 6, " beats");
                            if self.second {
                                ui.push_id(("persisted-owner", 99_u64), |ui| {
                                    accessibility::scope(ui, "Second owner", |ui| {
                                        number(
                                            ui,
                                            "Amount",
                                            &mut self.second_value,
                                            0.0,
                                            10.0,
                                            0.01,
                                            6,
                                            " beats",
                                        );
                                    })
                                });
                            }
                            if action(ui, "Apply draft", !invalid(ui)).clicked() {
                                self.accepted += 1;
                            }
                        });
                    },
                );
            });
        });
    }
}

#[test]
fn keyboard_access_repeated_property_keeps_existing_native_owner_scopes_distinct() {
    let mut fields = Fields::new();
    fields.second = true;
    fields.idle();
    let first = fields.native.node("Amount (beats)").0;
    let second = fields.native.node("Second owner: Amount (beats)").0;
    assert_ne!(
        first, second,
        "existing persisted owner scopes survive the numeric decorator"
    );
    fields.number("Amount (beats)", "1.75");
    assert_eq!((fields.amount, fields.second_value), (1.75, 2.0));
    fields.number("Second owner: Amount (beats)", "8.25");
    assert_eq!((fields.amount, fields.second_value), (1.75, 8.25));
    fields.tab_to("Show preceding row");
    fields.key(Key::Space, egui::Modifiers::NONE);
    assert_eq!(fields.native.node("Amount (beats)").0, first);
    assert_eq!(fields.native.node("Second owner: Amount (beats)").0, second);
}

#[test]
fn keyboard_access_conditional_row_keeps_actual_native_id_arrow_focus_and_tab_order() {
    let mut fields = Fields::new();
    let id = fields.native.node("Amount (beats)").0;
    for _ in 0..4 {
        fields.tab_to("Show preceding row");
        fields.key(Key::Space, egui::Modifiers::NONE);
        assert_eq!(fields.native.node("Amount (beats)").0, id);
        fields.tab_to("Amount (beats)");
        fields.native.focused("Amount (beats)");
        let response = fields
            .native
            .ctx
            .read_response(fields.native.ctx.memory(|memory| memory.focused()).unwrap())
            .unwrap();
        assert!(fields.native.shapes.iter().any(|shape|matches!(&shape.shape,egui::epaint::Shape::Rect(rect) if rect.stroke.width==2.0 && rect.rect==response.rect.expand(2.0))),"visible native focus ring");
        fields.key(Key::ArrowDown, egui::Modifiers::NONE);
        fields.native.focused("Amount (beats)");
        fields.key(Key::Tab, egui::Modifiers::NONE);
        fields.native.focused("Apply draft");
        fields.key(Key::Tab, egui::Modifiers::SHIFT);
        fields.native.focused("Amount (beats)");
    }
    assert!((fields.amount - 0.96).abs() < 0.000001);
}

#[test]
fn keyboard_access_precise_native_invalid_mixed_escape_and_disabled_target_semantics() {
    let mut fields = Fields::new();
    fields.mixed = true;
    fields.idle();
    assert_eq!(
        fields.native.node("Amount (beats)").1.state_description(),
        Some("Mixed")
    );
    assert!(fields
        .native
        .node("Amount (beats)")
        .1
        .numeric_value()
        .is_none());
    fields.text("Amount (beats)", "NaN");
    let error = fields
        .native
        .node("Amount (beats)")
        .1
        .error_message()
        .unwrap();
    assert_eq!(
        fields.native.node("Amount (beats)").1.invalid(),
        Some(Invalid::True)
    );
    assert!(fields
        .native
        .node("Amount (beats)")
        .1
        .described_by()
        .contains(&error));
    assert_eq!(
        fields
            .native
            .nodes
            .iter()
            .filter(|(_, node)| node.label() == Some("Enter a finite number"))
            .count(),
        1
    );
    assert_eq!(
        fields
            .native
            .nodes
            .iter()
            .find(|(id, _)| *id == error)
            .unwrap()
            .1
            .live(),
        Some(Live::Polite)
    );
    assert!(fields.native.node("Apply draft").1.is_disabled());
    assert_eq!(fields.amount, 1.0);
    fields.key(Key::Enter, egui::Modifiers::NONE);
    assert_eq!(fields.accepted, 0);
    fields.tab_to("Amount (beats)");
    fields.key(Key::Escape, egui::Modifiers::NONE);
    fields.idle();
    assert!(fields.native.node("Amount (beats)").1.invalid().is_none());
    fields.mixed = false;
    fields.number("Amount (beats)", " 1 . 2 5 0 0 0 1 ");
    assert_eq!(fields.amount, 1.250001);
    assert_eq!(
        fields.native.node("Amount (beats)").1.numeric_value(),
        Some(1.250001)
    );
    fields.press("Apply draft");
    assert_eq!(fields.accepted, 1);
    fields.locked = true;
    fields.idle();
    let target = fields.native.node("Amount (beats)").0;
    fields.frame(
        vec![egui::Event::AccessKitActionRequest(ActionRequest {
            target,
            action: Action::SetValue,
            data: Some(ActionData::NumericValue(9.0)),
        })],
        egui::Modifiers::NONE,
    );
    assert!(fields.native.node("Amount (beats)").1.is_disabled());
    assert_eq!(fields.amount, 1.250001);
    fields.locked = false;
    fields.live = false;
    fields.idle();
    assert!(fields.native.node("Apply draft").1.is_disabled());
    fields.live = true;
    fields.owner = 18;
    fields.idle();
    assert_ne!(
        fields.native.node("Amount (beats)").0,
        target,
        "new persisted owner gets a distinct actual widget"
    );
    fields.number("Amount (beats)", "− 0.5");
    assert_eq!(fields.amount, 0.0, "native finite range clamp is retained");
}

#[test]
fn keyboard_access_native_panel_size_invalid_refusal_repeated_focus_and_rendered_footprint() {
    let mut gui = Gui::new();
    gui.route("Arrange workspace panels");
    gui.press("Float panel");
    gui.idle();
    gui.idle();
    gui.native.focused_panel("MIDI");
    gui.number("Floating panel width (px)", "720");
    gui.number("Floating panel height (px)", "480");
    gui.press("Apply floating panel size");
    for _ in 0..6 {
        gui.idle();
        let rect = gui.floating_rect("midi");
        assert!(
            (rect.width() - 720.0).abs() < 1.0 && (rect.height() - 480.0).abs() < 1.0,
            "actual footprint {rect:?}"
        );
    }
    let before = gui.floating_rect("midi");
    gui.text("Floating panel width (px)", "not a size");
    let node = gui.native.node("Floating panel width (px)").1;
    assert_eq!(node.invalid(), Some(Invalid::True));
    assert!(node.error_message().is_some());
    assert!(gui.native.node("Apply floating panel size").1.is_disabled());
    gui.key(Key::Enter, egui::Modifiers::NONE);
    gui.idle();
    assert_eq!(gui.floating_rect("midi"), before);
    gui.number("Floating panel width (px)", "720");
    for _ in 0..2 {
        gui.press("Focus panel");
        gui.idle();
        gui.idle();
        gui.native.focused_panel("MIDI");
    }
    gui.press("Cancel size draft");
    gui.tab_to("Workspace: Decks");
    gui.key(Key::Space, egui::Modifiers::NONE);
    gui.press("Save workspace layouts");
    let path = gui.root.join("preferences.json");
    gui.until(|gui| path.exists() && !gui.app.settings.busy());
    let saved = crate::preferences::storage::load(&path, &AtomicBool::new(false)).unwrap();
    let group = saved.preferences.current().unwrap().workspaces.studio.tasks["Decks"]
        .groups
        .iter()
        .find(|group| group.panels == ["midi"])
        .unwrap();
    assert!((group.rect[2] - 720.0).abs() < 1.0 && (group.rect[3] - 480.0).abs() < 1.0);
}

#[test]
fn keyboard_access_floating_window_native_json_reopen_allows_later_manual_resize() {
    let mut gui = Gui::new();
    gui.route("Arrange workspace panels");
    gui.press("Float panel");
    gui.idle();
    gui.idle();
    gui.number("Floating panel width (px)", "680");
    gui.number("Floating panel height (px)", "440");
    gui.press("Apply floating panel size");
    gui.press("Cancel size draft");
    gui.tab_to("Workspace: Decks");
    gui.key(Key::Space, egui::Modifiers::NONE);
    gui.press("Save workspace layouts");
    let path = gui.root.join("preferences.json");
    gui.until(|gui| path.exists() && !gui.app.settings.busy());
    let loaded = crate::preferences::storage::load(&path, &AtomicBool::new(false)).unwrap();
    let mut reopened = Gui::new();
    reopened.app.settings.applied = loaded.preferences.clone();
    reopened.app.settings.draft = loaded.preferences;
    for _ in 0..5 {
        reopened.idle();
        let actual = reopened.floating_rect("midi");
        assert!(
            (actual.width() - 680.0).abs() < 1.0 && (actual.height() - 440.0).abs() < 1.0,
            "reopened native footprint {actual:?}"
        );
    }
    let before = reopened.floating_rect("midi");
    let start = before.right_bottom() - Vec2::splat(2.0);
    let end = start + Vec2::new(96.0, 64.0);
    reopened.frame(
        vec![egui::Event::PointerMoved(start)],
        egui::Modifiers::NONE,
    );
    reopened.frame(
        vec![egui::Event::PointerButton {
            pos: start,
            button: PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        }],
        egui::Modifiers::NONE,
    );
    reopened.frame(vec![egui::Event::PointerMoved(end)], egui::Modifiers::NONE);
    reopened.frame(
        vec![egui::Event::PointerButton {
            pos: end,
            button: PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        }],
        egui::Modifiers::NONE,
    );
    for _ in 0..4 {
        reopened.idle();
    }
    let after = reopened.floating_rect("midi");
    assert!(
        after.width() > before.width() + 40.0 && after.height() > before.height() + 25.0,
        "manual native resize remains available: {before:?} -> {after:?}"
    );
}

#[test]
fn keyboard_access_cancelled_invalid_size_draft_reopens_with_fresh_native_owner_and_applies() {
    let mut gui = Gui::new();
    gui.route("Arrange workspace panels");
    gui.press("Float panel");
    gui.idle();
    gui.idle();
    let before = gui.floating_rect("midi");
    gui.text("Floating panel width (px)", "-");
    let invalid_owner = gui.native.node("Floating panel width (px)").0;
    assert_eq!(
        gui.native.node("Floating panel width (px)").1.invalid(),
        Some(Invalid::True)
    );
    assert!(gui.native.node("Apply floating panel size").1.is_disabled());
    gui.press("Cancel size draft");
    gui.idle();
    assert_eq!(gui.floating_rect("midi"), before);
    gui.route("Arrange workspace panels");
    assert_ne!(
        gui.native.node("Floating panel width (px)").0,
        invalid_owner,
        "a reopened native edit is a new draft owner"
    );
    assert!(gui
        .native
        .node("Floating panel width (px)")
        .1
        .invalid()
        .is_none());
    gui.number("Floating panel width (px)", "760");
    gui.number("Floating panel height (px)", "500");
    assert!(!gui.native.node("Apply floating panel size").1.is_disabled());
    gui.press("Apply floating panel size");
    for _ in 0..5 {
        gui.idle();
        let rect = gui.floating_rect("midi");
        assert!((rect.width() - 760.0).abs() < 1.0 && (rect.height() - 500.0).abs() < 1.0);
    }
}
