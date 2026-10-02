use super::*;
use crate::ui::piano_roll::tests::Gui;
use egui::accesskit::{Action, ActionData};

struct Files(PathBuf);
impl Files {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "omat-midi-files-{}",
            crate::sampler_bank::BankId::new().unwrap()
        ));
        std::fs::create_dir(&root).unwrap();
        Self(root)
    }
    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}
impl Drop for Files {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/midi/sixteen-bars-ppqn960.mid")
}
fn settled(gui: &mut Gui) {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        gui.frame(vec![]);
        if !gui.app.midi_files.busy() && !gui.app.project_pending_for_test() {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "{:?} / {:?}",
            gui.app.midi_files.error,
            gui.app.project_result_for_test().1
        );
        std::thread::sleep(Duration::from_millis(1));
    }
    gui.frame(vec![]);
}
fn path(gui: &mut Gui, value: &std::path::Path) {
    gui.action("MIDI file path", Action::Focus, None);
    gui.key(
        Key::A,
        egui::Modifiers {
            ctrl: true,
            command: true,
            ..Default::default()
        },
    );
    gui.frame(vec![egui::Event::Text(value.display().to_string())]);
    assert_eq!(gui.app.midi_files.path, value.display().to_string());
}
fn open(gui: &mut Gui, exporting: bool) {
    gui.click_text("Project");
    gui.click_text(if exporting {
        "Export MIDI file…"
    } else {
        "Import MIDI file…"
    });
    gui.frame(vec![]);
    assert!(gui.app.midi_files.open);
}
fn inspect(gui: &mut Gui, value: &std::path::Path) {
    open(gui, false);
    path(gui, value);
    gui.click("Inspect MIDI file");
    settled(gui);
    assert!(
        gui.app.midi_files.error.is_none(),
        "{:?}",
        gui.app.midi_files.error
    );
    assert!(gui.app.midi_files.preview.is_some());
}
fn musical(mut file: crate::midi_file::File) -> crate::midi_file::File {
    for track in &mut file.tracks {
        for note in &mut track.notes {
            note.start_order = 0;
            note.end_order = 0;
        }
        for m in &mut track.messages {
            m.order = 0;
        }
        for m in &mut track.meta {
            m.order = 0;
        }
    }
    file
}

#[test]
fn real_project_menu_import_review_save_reopen_export_preserves_all_fixture_events() {
    let files = Files::new();
    let mut gui = Gui::new();
    inspect(&mut gui, &fixture_path());
    assert_eq!(
        gui.app
            .midi_files
            .mappings
            .iter()
            .map(|m| m.destination)
            .collect::<Vec<_>>(),
        vec![Some((2, 7)), Some((3, 0))]
    );
    gui.click_text("Keep session tempo and meter");
    gui.click_text("Use complete file conductor");
    let cursor = gui.app.engine.undo.view().cursor;
    gui.click("Import inspected MIDI");
    settled(&mut gui);
    assert!(
        gui.app.midi_files.error.is_none(),
        "{:?}",
        gui.app.midi_files.error
    );
    assert_eq!(gui.app.engine.undo.view().cursor, cursor + 1);
    let notes = gui.rt.tracks[3].clips[0].notes.clone();
    assert_eq!(notes.len(), 64);
    assert_eq!(
        gui.rt.tracks[3].clips[0]
            .lanes
            .as_ref()
            .unwrap()
            .messages
            .len(),
        11
    );
    assert_eq!(gui.rt.conductor.as_ref().unwrap().tempos[1].micros, 666667);
    gui.app.midi_files.open = false;
    gui.frame(vec![]);
    gui.click_text("Project");
    gui.click_text("Save project as…");
    gui.path(&files.path("session.omat"));
    gui.click_text("Save");
    settled(&mut gui);
    assert_eq!(
        gui.app.project_result_for_test().0,
        Some(files.path("session.omat"))
    );
    let mut reopened = Gui::new();
    reopened.click_text("Project");
    reopened.click_text("Open project…");
    reopened.path(&files.path("session.omat"));
    reopened.click_text("Open");
    settled(&mut reopened);
    assert_eq!(reopened.rt.tracks[3].clips[0].notes, notes);
    assert_eq!(reopened.rt.conductor, gui.rt.conductor);
    open(&mut reopened, true);
    path(&mut reopened, &files.path("export.mid"));
    assert_eq!(reopened.app.midi_files.cells, BTreeSet::from([(2, 7)]));
    reopened.click("Track 4 scene 1");
    reopened.click_text("Export current session tempo and meter");
    reopened.click("Export new MIDI file");
    settled(&mut reopened);
    assert!(
        reopened.app.midi_files.error.is_none(),
        "{:?}",
        reopened.app.midi_files.error
    );
    let exported =
        crate::midi_file::decode(&std::fs::read(files.path("export.mid")).unwrap()).unwrap();
    let original = crate::midi_file::decode(&std::fs::read(fixture_path()).unwrap()).unwrap();
    assert_eq!(musical(exported), musical(original));
    let bytes = std::fs::read(files.path("export.mid")).unwrap();
    reopened.click("Export new MIDI file");
    settled(&mut reopened);
    assert!(reopened
        .app
        .midi_files
        .error
        .as_ref()
        .unwrap()
        .contains("unused path"));
    assert_eq!(std::fs::read(files.path("export.mid")).unwrap(), bytes);
    if let Some(root) = std::env::var_os("OMAT_MIDI_FILE_EVIDENCE") {
        let root = PathBuf::from(root);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::copy(files.path("export.mid"), root.join("native-export.mid")).unwrap();
        std::fs::copy(files.path("session.omat"), root.join("native-session.omat")).unwrap();
    }
}

#[test]
fn independent_ardour_region_returns_through_ui_with_review_and_kept_conductor() {
    let files = Files::new();
    let mut gui = Gui::new();
    inspect(&mut gui, &fixture_path());
    gui.click_text("Keep session tempo and meter");
    gui.click_text("Use complete file conductor");
    gui.click("Import inspected MIDI");
    settled(&mut gui);
    let conductor = gui.rt.conductor.clone();
    let returned = fixture_path().with_file_name("ardour-9.8-region-ppqn19200.mid");
    path(&mut gui, &returned);
    gui.click("Inspect MIDI file");
    settled(&mut gui);
    assert_eq!(
        gui.app.midi_files.preview.as_ref().unwrap().file.ppqn,
        19200
    );
    assert_eq!(
        gui.app
            .midi_files
            .preview
            .as_ref()
            .unwrap()
            .file
            .warnings
            .len(),
        134
    );
    let before = gui.app.engine.undo.checkpoint();
    gui.click("Import inspected MIDI");
    settled(&mut gui);
    assert_eq!(
        gui.app.engine.undo.checkpoint(),
        before,
        "Unreviewed proprietary DAW data was not admitted"
    );
    gui.action(
        "File track 1 destination track",
        Action::SetValue,
        Some(ActionData::NumericValue(4.0)),
    );
    gui.action(
        "File track 1 destination scene",
        Action::SetValue,
        Some(ActionData::NumericValue(1.0)),
    );
    gui.click_text("Use complete file conductor");
    gui.click_text("Keep session tempo and meter");
    gui.click_text("Accept listed unsupported omissions");
    gui.click("Import inspected MIDI");
    settled(&mut gui);
    assert!(
        gui.app.midi_files.error.is_none(),
        "{:?}",
        gui.app.midi_files.error
    );
    assert_eq!(gui.rt.conductor, conductor);
    let clip = &gui.rt.tracks[3].clips[0];
    assert_eq!(clip.notes.len(), 64);
    let mut notes = clip.notes.iter().collect::<Vec<_>>();
    notes.sort_by_key(|n| n.source_timing.unwrap().start);
    for (i, note) in notes.iter().enumerate() {
        let timing = note.source_timing.unwrap();
        assert_eq!(
            (timing.ppqn, timing.start, timing.duration),
            (19200, i as u64 * 19200 + 20, 14380)
        );
        assert_eq!(
            (note.channel, note.pitch, note.vel, note.release_vel),
            (
                (i % 2 * 2) as u8,
                60 + (i % 8) as u8,
                80 + (i % 32) as u8,
                i as u8
            )
        );
    }
    let lanes = clip.lanes.as_ref().unwrap();
    let cc1 = lanes
        .messages
        .iter()
        .filter(|m| m.bytes[0] == 0xb0 && m.bytes[1] == 1)
        .collect::<Vec<_>>();
    assert_eq!(cc1.len(), 9);
    for (i, message) in cc1.iter().enumerate() {
        assert_eq!(
            (message.tick, message.bytes[2]),
            (i as u64 * 8 * 19200, i as u8 * 15)
        );
    }
    assert_eq!(
        lanes
            .messages
            .iter()
            .filter(|m| m.bytes[0] & 0xf0 == 0xc0)
            .count(),
        2
    );
    assert_eq!(lanes.end_tick, 65 * 19200);
    open(&mut gui, true);
    // Export the returned note cell, with the conductor retained in this session.
    for cell in gui.app.midi_files.cells.clone() {
        if cell != (3, 0) {
            gui.click(&format!("Track {} scene {}", cell.0 + 1, cell.1 + 1));
        }
    }
    if !gui.app.midi_files.cells.contains(&(3, 0)) {
        gui.click("Track 4 scene 1");
    }
    gui.action(
        "Export PPQN",
        Action::SetValue,
        Some(ActionData::NumericValue(19200.0)),
    );
    path(&mut gui, &files.path("returned.mid"));
    gui.click("Export new MIDI file");
    settled(&mut gui);
    assert!(
        gui.app.midi_files.error.is_none(),
        "{:?}",
        gui.app.midi_files.error
    );
    let exported =
        crate::midi_file::decode(&std::fs::read(files.path("returned.mid")).unwrap()).unwrap();
    assert!(exported.warnings.is_empty());
    assert_eq!(exported.tracks[1].notes.len(), 64);
    assert_eq!(exported.tracks[1].messages.len(), 15);
    assert!(exported.tracks[0]
        .meta
        .iter()
        .any(|m| m.tick == 32 * 19200 && m.value == crate::midi_file::MetaValue::Tempo(666667)));
    assert!(exported.tracks[0].meta.iter().any(|m| m.tick == 32 * 19200
        && matches!(
            m.value,
            crate::midi_file::MetaValue::Meter {
                numerator: 7,
                denominator_power: 3,
                ..
            }
        )));
    if let Some(root) = std::env::var_os("OMAT_MIDI_FILE_EVIDENCE") {
        let root = PathBuf::from(root);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::copy(
            files.path("returned.mid"),
            root.join("native-ardour-return.mid"),
        )
        .unwrap();
    }
}

#[test]
fn native_merge_choice_combines_existing_contents_and_undo_restores_both_cells() {
    let mut gui = Gui::new();
    inspect(&mut gui, &fixture_path());
    gui.click("Import inspected MIDI");
    settled(&mut gui);
    let notes = gui.rt.tracks[3].clips[0].notes.clone();
    let lanes = gui.rt.tracks[3].clips[0].lanes.clone();
    let before = gui.app.engine.undo.view().cursor;
    gui.click_text("Merge with existing mapped MIDI clips");
    gui.click("Import inspected MIDI");
    settled(&mut gui);
    assert!(
        gui.app.midi_files.error.is_none(),
        "{:?}",
        gui.app.midi_files.error
    );
    assert_eq!(gui.rt.tracks[3].clips[0].notes.len(), 128);
    assert_eq!(
        gui.rt.tracks[3].clips[0]
            .lanes
            .as_ref()
            .unwrap()
            .messages
            .len(),
        22
    );
    assert_eq!(gui.app.engine.undo.view().cursor, before + 1);
    gui.app.engine.send(crate::engine::Command::Undo).unwrap();
    gui.frame(vec![]);
    assert_eq!(gui.rt.tracks[3].clips[0].notes, notes);
    assert_eq!(gui.rt.tracks[3].clips[0].lanes, lanes);
}

#[test]
fn file_review_on_a_720_line_window_scrolls_to_real_import_controls() {
    let mut gui = Gui::new();
    gui.screen = Vec2::new(1280.0, 720.0);
    inspect(
        &mut gui,
        &fixture_path().with_file_name("ardour-9.8-region-ppqn19200.mid"),
    );
    let nodes = &gui.nodes;
    let button = nodes
        .iter()
        .find_map(|(_, n)| (n.label() == Some("Import inspected MIDI")).then_some(n))
        .unwrap();
    // A bounded outer scroll area makes the long review reachable without
    // allowing the native window itself to grow beyond the available screen.
    let original = button.bounds().unwrap();
    for _ in 0..12 {
        gui.frame(vec![
            egui::Event::PointerMoved(Pos2::new(800.0, 600.0)),
            egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: Vec2::new(0.0, -100.0),
                modifiers: egui::Modifiers::NONE,
            },
        ]);
    }
    let bounds = gui.node("Import inspected MIDI").1.bounds().unwrap();
    assert!(
        bounds.y1 < 720.0 && bounds.y0 >= 0.0,
        "{original:?} -> {bounds:?}"
    );
    gui.click_text("Accept listed unsupported omissions");
    gui.click("Import inspected MIDI");
    settled(&mut gui);
    assert!(
        gui.app.midi_files.error.is_none(),
        "{:?}",
        gui.app.midi_files.error
    );
    assert_eq!(gui.rt.tracks[2].clips[7].notes.len(), 64);
}

#[test]
fn actual_mapping_controls_and_source_replacement_fail_without_session_changes() {
    let files = Files::new();
    let source = files.path("source.mid");
    std::fs::copy(fixture_path(), &source).unwrap();
    let mut gui = Gui::new();
    inspect(&mut gui, &source);
    gui.action(
        "File track 2 destination track",
        Action::SetValue,
        Some(ActionData::NumericValue(5.0)),
    );
    gui.action(
        "File track 2 destination scene",
        Action::SetValue,
        Some(ActionData::NumericValue(3.0)),
    );
    assert_eq!(gui.app.midi_files.mappings[1].destination, Some((4, 2)));
    let before = gui.app.engine.undo.checkpoint();
    std::fs::write(&source, b"changed after review").unwrap();
    gui.click("Import inspected MIDI");
    settled(&mut gui);
    assert!(gui
        .app
        .midi_files
        .error
        .as_ref()
        .unwrap()
        .contains("changed"));
    assert_eq!(gui.app.engine.undo.checkpoint(), before);
    assert!(gui.rt.tracks[4].clips[2].lanes.is_none());
    path(&mut gui, &source);
    gui.click("Inspect MIDI file");
    settled(&mut gui);
    assert!(gui.app.midi_files.error.is_some());
}

#[test]
fn native_close_cancels_queued_import_before_claim_and_preserves_session() {
    let mut gui = Gui::new();
    inspect(&mut gui, &fixture_path());
    let (entered, resume) = gui.app.midi_files.worker.as_ref().unwrap().pause_import();
    gui.render = false;
    gui.click("Import inspected MIDI");
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        gui.rt.process(&mut []);
        gui.frame(vec![]);
        if entered.try_recv().is_ok() {
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(1));
    }
    resume.send(()).unwrap();
    while gui.app.midi_files.pending.is_none() {
        gui.frame(vec![]);
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(1));
    }
    let original_notes = gui.rt.tracks[3].clips[0].notes.clone();
    let before = gui.app.engine.undo.checkpoint();
    gui.native_close = true;
    gui.frame(vec![]);
    gui.rt.process(&mut []);
    gui.render = true;
    settled(&mut gui);
    assert_eq!(gui.app.engine.undo.checkpoint(), before);
    assert_eq!(gui.rt.tracks[3].clips[0].notes, original_notes);
    assert!(gui.rt.conductor.is_none());
    assert!(gui.app.midi_files.message.contains("cancelled"));
}

fn retire_destination_and_create_replacement(gui: &mut Gui, track: usize) {
    use crate::engine::session::{Action as Edit, Axis, Request};
    let id = gui.rt.session.tracks[track].id;
    let (request, ack) = Request::metadata(
        &gui.rt.session, gui.app.engine.undo.checkpoint().epoch,
        Edit::Delete { axis: Axis::Track, id },
    ).unwrap();
    gui.app.engine.send(Command::SessionEdit(request)).unwrap();
    gui.frame(vec![]);
    assert_eq!(ack.state(), crate::engine::midi_edit::Outcome::Applied);
    gui.frame(vec![]);
    gui.click("+ MIDI track");
    let deadline = Instant::now() + Duration::from_secs(15);
    while gui.rt.session.tracks[track].id == id || !gui.rt.session.tracks[track].active {
        gui.frame(vec![]);
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(1));
    }
    for _ in 0..4 { gui.frame(vec![]); }
    assert_eq!(gui.rt.tracks[track].kind, 2);
    assert!(gui.rt.session.tracks[track].active);
    assert_ne!(gui.rt.session.tracks[track].id, id);
}

#[test]
fn inspected_destination_is_unmapped_when_its_storage_slot_is_reused() {
    let mut gui = Gui::new();
    inspect(&mut gui, &fixture_path());
    assert_eq!(gui.app.midi_files.mappings[0].destination, Some((2, 7)));
    retire_destination_and_create_replacement(&mut gui, 2);
    assert_eq!(gui.app.midi_files.mappings[0].destination, None);
    assert!(gui.app.midi_files.error.as_ref().unwrap().contains("replacement explicitly"));
    assert!(!gui.app.midi_files.reviewed);
    assert!(gui.rt.tracks[2].clips[7].notes.is_empty());
}

#[test]
fn export_rejects_a_reused_selected_cell_without_creating_a_file_or_history() {
    let files = Files::new();
    let mut gui = Gui::new();
    open(&mut gui, true);
    assert_eq!(gui.app.midi_files.cells, BTreeSet::from([(2, 7)]));
    path(&mut gui, &files.path("stale.mid"));
    retire_destination_and_create_replacement(&mut gui, 2);
    let before = gui.app.engine.undo.checkpoint();
    gui.click("Export new MIDI file");
    settled(&mut gui);
    assert!(gui.app.midi_files.error.as_ref().unwrap().contains("explicitly select its replacement"));
    assert!(!files.path("stale.mid").exists());
    assert_eq!(gui.app.engine.undo.checkpoint(), before);
}
