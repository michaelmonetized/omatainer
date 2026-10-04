use super::*;
use crate::engine::{test_alloc, RtEngine};
use egui::accesskit::{Action, ActionData, ActionRequest, Node, NodeId, Role};

struct Gui {
    app: App,
    rt: RtEngine,
    ctx: egui::Context,
    time: f64,
    size: Vec2,
    nodes: Vec<(NodeId, Node)>,
    render: bool,
}
impl Gui {
    fn new(sr: u32) -> Self {
        let (engine, mut rt) = Engine::headless_for_test(sr, 256);
        rt.publish_for_test();
        let app = App::with_loader(engine, Theme::default(), None);
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut value = Self {
            app,
            rt,
            ctx,
            time: 0.0,
            size: Vec2::new(1440.0, 1000.0),
            nodes: Vec::new(),
            render: true,
        };
        value.frame(vec![]);
        value.frame(vec![]);
        value
    }
    fn frame(&mut self, events: Vec<egui::Event>) -> egui::FullOutput {
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
        let out = self.ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, self.size)),
                time: Some(self.time),
                focused: true,
                modifiers,
                events,
                ..Default::default()
            },
            |ctx| self.app.update_frame(ctx),
        );
        self.nodes = out
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
        out
    }
    fn node(&self, name: &str) -> (NodeId, &Node) {
        self.nodes
            .iter()
            .find(|(_, n)| n.label() == Some(name))
            .map(|(id, n)| (*id, n))
            .unwrap_or_else(|| {
                panic!(
                    "missing {name}; {:?}",
                    self.nodes
                        .iter()
                        .filter_map(|(_, n)| n.label())
                        .collect::<Vec<_>>()
                )
            })
    }
    fn action(&mut self, name: &str, action: Action, data: Option<ActionData>) -> egui::FullOutput {
        let target = self.node(name).0;
        self.frame(vec![egui::Event::AccessKitActionRequest(ActionRequest {
            action,
            target,
            data,
        })]);
        self.frame(vec![])
    }
    fn click(&mut self, name: &str) -> egui::FullOutput {
        self.action(name, Action::Click, None)
    }
    fn key(&mut self, key: Key, modifiers: egui::Modifiers) {
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
    fn text(&mut self, name: &str, text: &str) {
        self.action(name, Action::Focus, None);
        self.key(
            Key::A,
            egui::Modifiers {
                ctrl: true,
                command: true,
                ..Default::default()
            },
        );
        if text.is_empty() {
            self.key(Key::Backspace, Default::default());
        } else {
            self.frame(vec![egui::Event::Text(text.into())]);
        }
        self.frame(vec![]);
    }
    fn open(&mut self) {
        self.click("Deck A: Beatgrid editor");
        self.frame(vec![]);
    }
    fn apply(&mut self) {
        self.click("Deck A beatgrid: Apply grid");
        self.frame(vec![]);
    }
    fn close(&mut self) {
        self.click("Deck A beatgrid: Cancel or close grid editor");
        self.frame(vec![]);
    }
}

#[test]
fn tempo_anchor_widgets_validate_replace_delete_cancel_and_commit_one_receipt_qualified_edit() {
    let mut gui = Gui::new(48_000);
    let audio = gui.rt.decks[0].audio.clone().unwrap();
    let before = gui.app.engine.undo.view().cursor;
    gui.open();
    gui.text("Deck A beatgrid: Downbeat seconds", "0");
    gui.text("Deck A beatgrid: Stretch tempo BPM", "120");
    for (beat, seconds) in [("4", "2"), ("8", "3.5")] {
        gui.text("Deck A beatgrid: Tempo anchor beat", beat);
        gui.text("Deck A beatgrid: Tempo anchor source seconds", seconds);
        gui.click("Deck A beatgrid: Insert or replace tempo anchor");
    }
    let initial = gui.app.grid_editor.as_ref().unwrap().draft.unwrap();
    assert_eq!(initial.anchors().len(), 2);
    assert_eq!(initial.bpm_at(2.0), Some(160.0));
    assert_eq!(gui.rt.decks[0].grid, None);
    let old_delete = gui.node("Deck A beatgrid: Delete tempo anchor 2").0;
    gui.text("Deck A beatgrid: Tempo anchor source seconds", "3.6");
    gui.click("Deck A beatgrid: Insert or replace tempo anchor");
    let replaced = gui.app.grid_editor.as_ref().unwrap().draft.unwrap();
    assert_eq!(replaced.anchors().len(), 2);
    assert_eq!(replaced.seconds_at(8.0), Some(3.6));
    gui.frame(vec![egui::Event::AccessKitActionRequest(ActionRequest {
        action: Action::Click, target: old_delete, data: None,
    })]);
    assert_eq!(gui.app.grid_editor.as_ref().unwrap().draft, Some(replaced));
    for seconds in ["nan", "99", "2", "2.0001"] {
        gui.text("Deck A beatgrid: Tempo anchor source seconds", seconds);
        gui.click("Deck A beatgrid: Insert or replace tempo anchor");
        assert_eq!(gui.app.grid_editor.as_ref().unwrap().draft, Some(replaced));
        assert!(gui.app.grid_editor.as_ref().unwrap().error.is_some());
        assert!(gui.node("Deck A beatgrid: Apply grid").1.is_disabled());
    }
    gui.text("Deck A beatgrid: Tempo anchor source seconds", "3.6");
    gui.click("Deck A beatgrid: Insert or replace tempo anchor");
    gui.click("Deck A beatgrid: Delete tempo anchor 1");
    let draft = gui.app.grid_editor.as_ref().unwrap().draft.unwrap();
    assert_eq!(draft.anchors().len(), 1);
    assert_eq!(draft.seconds_at(8.0), Some(3.6));
    gui.apply();
    assert_eq!(gui.rt.decks[0].grid, Some(draft));
    assert_eq!(gui.app.engine.undo.view().cursor, before + 1);
    assert_eq!(gui.app.grid_editor.as_ref().unwrap().receipt.preparation().unwrap().1.grid, Some(draft));
    assert!(std::sync::Arc::ptr_eq(&audio, gui.rt.decks[0].audio.as_ref().unwrap()));
    gui.close(); gui.open();
    assert_eq!(gui.app.grid_editor.as_ref().unwrap().draft, Some(draft));
    gui.click("Deck A beatgrid: Delete tempo anchor 1");
    gui.close();
    assert_eq!(gui.rt.decks[0].grid, Some(draft));
    gui.open();
    gui.rt.apply(Command::DeckUnload { deck: 0 });
    gui.rt.publish_for_test();
    gui.frame(vec![]); gui.frame(vec![]);
    assert!(gui.app.grid_editor.is_none());
}

#[test]
fn real_grid_ui_previews_set_slip_stretch_half_double_and_commits_one_undo() {
    for sr in [44_100, 48_000, 96_000] {
        let mut gui = Gui::new(sr);
        gui.action(
            "Deck A: Waveform position",
            Action::SetValue,
            Some(ActionData::NumericValue(0.75)),
        );
        gui.click("Deck A: Hot cue 1");
        let cue = gui.rt.decks[0].hotcues[0].pos;
        gui.action(
            "Deck A: Waveform position",
            Action::SetValue,
            Some(ActionData::NumericValue(1.25)),
        );
        let seconds = gui.rt.decks[0].pos / gui.rt.decks[0].audio.as_ref().unwrap().sr as f64;
        let before = gui.app.engine.undo.view().cursor;
        let analyzed = gui.rt.decks[0].audio.as_ref().unwrap().bpm;
        gui.open();
        assert!(gui
            .app
            .grid_editor
            .as_ref()
            .unwrap()
            .message
            .contains("Unverified"));
        gui.click("Deck A beatgrid: Set downbeat at playhead");
        gui.click("Deck A beatgrid: Slip +10 ms");
        gui.click("Deck A beatgrid: Slip −1 ms");
        let origin = seconds + 0.009;
        assert!(
            (gui.app
                .grid_editor
                .as_ref()
                .unwrap()
                .draft
                .unwrap()
                .downbeat()
                - origin)
                .abs()
                < 1e-10
        );
        gui.text("Deck A beatgrid: Stretch tempo BPM", "120");
        gui.click("Deck A beatgrid: Half tempo");
        assert_eq!(
            gui.app.grid_editor.as_ref().unwrap().draft.unwrap().bpm(),
            60.0
        );
        gui.click("Deck A beatgrid: Double tempo");
        assert_eq!(
            gui.app.grid_editor.as_ref().unwrap().draft.unwrap().bpm(),
            120.0
        );
        assert!(
            gui.app
                .grid_editor
                .as_ref()
                .unwrap()
                .draft
                .unwrap()
                .beat_at(0.0)
                .unwrap()
                < 0.0
        );
        assert_eq!(
            gui.rt.decks[0].grid, None,
            "preview has no audio-side change"
        );
        assert_eq!(gui.app.engine.undo.view().cursor, before);
        gui.apply();
        let applied = gui.rt.decks[0].grid.unwrap();
        assert!((applied.downbeat() - origin).abs() < 1e-10);
        assert_eq!(applied.bpm(), 120.0);
        assert!(gui.app.grid_editor.as_ref().unwrap().pending.is_none());
        assert!(gui
            .app
            .grid_editor
            .as_ref()
            .unwrap()
            .message
            .contains("applied"));
        assert_eq!(gui.app.engine.undo.view().cursor, before + 1);
        for _ in 0..8 {
            gui.frame(vec![]);
        }
        assert_eq!(
            gui.app.engine.undo.view().cursor,
            before + 1,
            "idle editor adds no undo entries"
        );
        assert_eq!(gui.rt.decks[0].hotcues[0].pos, cue);
        assert_eq!(gui.rt.decks[0].audio.as_ref().unwrap().bpm, analyzed);
        gui.close();
        gui.key(
            Key::Z,
            egui::Modifiers {
                ctrl: true,
                command: true,
                ..Default::default()
            },
        );
        for _ in 0..5 {
            gui.frame(vec![]);
        }
        assert_eq!(gui.rt.decks[0].grid, None);
        gui.key(
            Key::Z,
            egui::Modifiers {
                ctrl: true,
                command: true,
                shift: true,
                ..Default::default()
            },
        );
        for _ in 0..5 {
            gui.frame(vec![]);
        }
        assert_eq!(gui.rt.decks[0].grid, Some(applied));
        assert_eq!(gui.rt.decks[0].hotcues[0].pos, cue);
    }
}

#[test]
fn draft_cancel_escape_and_track_replacement_leave_original_preparation_unchanged() {
    let mut gui = Gui::new(48_000);
    let before = gui.app.engine.undo.view().cursor;
    gui.open();
    gui.text("Deck A beatgrid: Downbeat seconds", "-0.25");
    gui.text("Deck A beatgrid: Stretch tempo BPM", "60");
    assert_eq!(
        gui.app
            .grid_editor
            .as_ref()
            .unwrap()
            .draft
            .unwrap()
            .downbeat(),
        -0.25
    );
    gui.close();
    assert!(gui.app.grid_editor.is_none());
    assert_eq!(gui.rt.decks[0].grid, None);
    assert_eq!(gui.app.engine.undo.view().cursor, before);
    gui.open();
    gui.text("Deck A beatgrid: Stretch tempo BPM", "NaN");
    assert!(gui.app.grid_editor.as_ref().unwrap().error.is_some());
    let apply = gui.node("Deck A beatgrid: Apply grid").0;
    gui.frame(vec![
        egui::Event::Key {
            key: Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Default::default(),
        },
        egui::Event::AccessKitActionRequest(ActionRequest {
            action: Action::Click,
            target: apply,
            data: None,
        }),
    ]);
    assert!(gui.app.grid_editor.is_none());
    assert_eq!(
        gui.rt.decks[0].grid, None,
        "Escape wins the same frame as Apply"
    );
    gui.key(Key::Escape, Default::default());
    gui.open();
    gui.click("Deck A beatgrid: Slip +1 ms");
    let apply = gui.node("Deck A beatgrid: Apply grid").0;
    let cancel = gui.node("Deck A beatgrid: Cancel or close grid editor").0;
    gui.frame(
        vec![apply, cancel]
            .into_iter()
            .map(|target| {
                egui::Event::AccessKitActionRequest(ActionRequest {
                    action: Action::Click,
                    target,
                    data: None,
                })
            })
            .collect(),
    );
    assert!(gui.app.grid_editor.is_none());
    assert_eq!(gui.rt.decks[0].grid, None, "Cancel wins simultaneous Apply");
    gui.open();
    gui.click("Deck A beatgrid: Slip +1 ms");
    gui.app
        .engine
        .send(Command::LoadBuiltin { deck: 0, stem: 1 })
        .unwrap();
    gui.frame(vec![]);
    gui.frame(vec![]);
    assert!(gui.app.grid_editor.is_none());
    assert!(gui.app.status.contains("track changed"));
    assert_eq!(gui.rt.decks[0].grid, None);
}

#[test]
fn invalid_unknown_and_out_of_range_tempos_never_invent_or_submit_a_grid() {
    let mut gui = Gui::new(48_000);
    gui.open();
    for invalid in ["", "0", "NaN", "inf", "19.99", "400.01"] {
        gui.text("Deck A beatgrid: Stretch tempo BPM", invalid);
        assert!(gui.app.grid_editor.as_ref().unwrap().error.is_some());
        assert!(gui.node("Deck A beatgrid: Apply grid").1.is_disabled());
        gui.click("Deck A beatgrid: Apply grid");
        assert_eq!(gui.rt.decks[0].grid, None);
    }
    gui.text("Deck A beatgrid: Stretch tempo BPM", "20");
    assert!(gui.node("Deck A beatgrid: Half tempo").1.is_disabled());
    assert!(!gui.node("Deck A beatgrid: Double tempo").1.is_disabled());
    gui.text("Deck A beatgrid: Stretch tempo BPM", "400");
    assert!(gui.node("Deck A beatgrid: Double tempo").1.is_disabled());
    gui.text("Deck A beatgrid: Downbeat seconds", "1e99");
    assert!(gui.app.grid_editor.as_ref().unwrap().error.is_some());
    gui.close();
    // A valid loaded identity with an unknown analysis result must ask for BPM.
    gui.app.snap.decks[0].source_bpm = 0.0;
    gui.app.open_grid_editor(0);
    assert!(gui.app.grid_editor.as_ref().unwrap().draft.is_none());
    assert!(gui.app.grid_editor.as_ref().unwrap().tempo.is_empty());
    gui.frame(vec![]);
    gui.click("Deck A beatgrid: Set downbeat at playhead");
    assert!(gui.app.grid_editor.as_ref().unwrap().error.is_some());
    assert_eq!(gui.rt.decks[0].grid, None);
}

#[test]
fn reset_is_a_draft_until_apply_and_queue_rejection_preserves_it() {
    let mut gui = Gui::new(48_000);
    gui.open();
    gui.text("Deck A beatgrid: Stretch tempo BPM", "120");
    gui.apply();
    let applied = gui.rt.decks[0].grid;
    gui.click("Deck A beatgrid: Reset manual grid");
    assert_eq!(gui.app.grid_editor.as_ref().unwrap().draft, None);
    assert_eq!(gui.rt.decks[0].grid, applied);
    gui.close();
    gui.open();
    gui.click("Deck A beatgrid: Reset manual grid");
    gui.apply();
    assert_eq!(gui.rt.decks[0].grid, None);
    gui.text("Deck A beatgrid: Stretch tempo BPM", "125");
    gui.render = false;
    while gui.app.engine.send(Command::Xfader(0.25)).is_ok() {}
    gui.click("Deck A beatgrid: Apply grid");
    assert!(gui
        .app
        .grid_editor
        .as_ref()
        .unwrap()
        .message
        .contains("not accepted"));
    assert!(gui.app.grid_editor.as_ref().unwrap().pending.is_none());
    assert!(gui.app.grid_editor.as_ref().unwrap().draft.is_some());
    gui.render = true;
    for _ in 0..12 {
        gui.frame(vec![]);
    }
    assert_eq!(gui.rt.decks[0].grid, None);
}

#[test]
fn small_window_scroll_controls_are_named_focusable_and_f1_describes_the_actual_grid_action() {
    let mut gui = Gui::new(48_000);
    gui.size = Vec2::new(640.0, 360.0);
    gui.frame(vec![]);
    // Open from the actual button even when the performance surface scrolls.
    gui.action("Deck A: Beatgrid editor", Action::Focus, None);
    gui.open();
    for name in [
        "Downbeat seconds",
        "Stretch tempo BPM",
        "Set downbeat at playhead",
        "Slip +10 ms",
        "Half tempo",
        "Double tempo",
        "Reset manual grid",
        "Apply grid",
        "Cancel or close grid editor",
    ] {
        let label = format!("Deck A beatgrid: {name}");
        gui.action(&label, Action::Focus, None);
        for _ in 0..3 {
            gui.frame(vec![]);
        }
        let node = gui.node(&label).1;
        assert!(matches!(node.role(), Role::Button | Role::TextInput));
        let bounds = node.bounds().expect("real control bounds");
        assert!(
            bounds.y0 >= 0.0 && bounds.y1 <= 360.0 && bounds.x0 >= 0.0 && bounds.x1 <= 640.0,
            "{label}: {bounds:?}"
        );
        assert!(node.description().is_some(), "{label} has contextual help");
    }
    gui.action("Deck A beatgrid: Half tempo", Action::Focus, None);
    gui.key(Key::F1, Default::default());
    assert!(gui.app.keys_open);
    let out = gui.frame(vec![]);
    assert!(out.shapes.iter().any(|s| matches!(&s.shape, egui::epaint::Shape::Text(t) if t.galley.text().contains("Half grid tempo"))));
    assert_eq!(gui.rt.decks[0].grid, None, "help changes no preparation");
}

#[test]
fn visible_grid_markers_cover_pickups_and_bound_extreme_spans_without_rt_work() {
    let extreme = Grid::new(0.0, 400.0).unwrap();
    assert!(markers(extreme, -1e300, 1e300).count() <= 128);
    let coarse: Vec<_> = markers(extreme, 0.01, 1000.0).collect();
    assert!(coarse.len() <= 128 && !coarse.is_empty());
    assert!(coarse.iter().all(|m| m.beat.rem_euclid(4) == 0));

    let grid = Grid::new(1.25, 120.0).unwrap();
    let found: Vec<_> = markers(grid, 0.0, 2.0).collect();
    assert_eq!(
        found
            .iter()
            .map(|m| (m.beat, m.seconds))
            .collect::<Vec<_>>(),
        vec![(-2, 0.25), (-1, 0.75), (0, 1.25), (1, 1.75)]
    );
    assert!(markers(grid, f64::NAN, 10.0).next().is_none());
    assert!(markers(grid, 1.0, 1.0).next().is_none());
    assert!(markers(Grid::new(-1e9, 400.0).unwrap(), 0.0, 1e9).count() <= 128);
    let mut gui = Gui::new(48_000);
    gui.open();
    gui.text("Deck A beatgrid: Downbeat seconds", "1.25");
    gui.text("Deck A beatgrid: Stretch tempo BPM", "120");
    let out = gui.frame(vec![]);
    let rect = gui
        .node("Beatgrid band-magnitude preview")
        .1
        .bounds()
        .unwrap();
    let target_x = rect.x0 as f32 + (1.25 / 7.0) * (rect.x1 - rect.x0) as f32;
    let dashes = out.shapes.iter().filter(|s| matches!(&s.shape, egui::epaint::Shape::LineSegment { points, stroke } if stroke.color==gui.app.theme.yellow && (points[0].x-target_x).abs()<0.01 && points[0].x==points[1].x)).count();
    assert_eq!(
        dashes, 8,
        "actual painter shows the draft downbeat at source 1.25s"
    );
    assert_eq!(
        test_alloc::measure(|| gui.rt.process(&mut [0.0; 128])),
        test_alloc::Counts::default()
    );
}

struct Files(PathBuf);
impl Files {
    fn new() -> Self {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "omatainer-grid99-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&dir).unwrap();
        Self(dir)
    }
    fn wave(&self, name: &str, leading_silence: bool) -> PathBuf {
        let path = self.0.join(format!("{name}.wav"));
        let sr = 16_000u32;
        let frames = sr * 4;
        let size = frames * 2;
        let mut bytes = b"RIFF".to_vec();
        bytes.extend((36 + size).to_le_bytes());
        bytes.extend(b"WAVEfmt ");
        bytes.extend(16u32.to_le_bytes());
        bytes.extend(1u16.to_le_bytes());
        bytes.extend(1u16.to_le_bytes());
        bytes.extend(sr.to_le_bytes());
        bytes.extend((sr * 2).to_le_bytes());
        bytes.extend(2u16.to_le_bytes());
        bytes.extend(16u16.to_le_bytes());
        bytes.extend(b"data");
        bytes.extend(size.to_le_bytes());
        for frame in 0..frames {
            let silent = leading_silence && frame < sr;
            let t = frame as f64 / sr as f64;
            let pulse = (frame % (sr / 2)) < sr / 100;
            let signal = if silent {
                0
            } else {
                ((t * 440.0 * std::f64::consts::TAU).sin() * if pulse { 18_000.0 } else { 2_000.0 })
                    as i16
            };
            bytes.extend(signal.to_le_bytes());
        }
        std::fs::write(&path, bytes).unwrap();
        path
    }
}
impl Drop for Files {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
impl Gui {
    #[track_caller]
    fn until(&mut self, mut condition: impl FnMut(&Self) -> bool) {
        let deadline = Instant::now() + std::time::Duration::from_secs(8);
        loop {
            self.frame(vec![]);
            if condition(self) {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "grid library timeout: status={} metadata={} scan={} durable={} sources={:?}",
                self.app.status,
                self.app.library_metadata.label(),
                self.app.library_scan.label(),
                self.app.library_metadata.durable,
                self.app
                    .library
                    .iter()
                    .map(|i| &i.source)
                    .collect::<Vec<_>>()
            );
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }
    fn library(&mut self, files: &Files, seed: f32) {
        self.app.loader = Some(
            crate::engine::media_load::Loader::with_decoder(move |path, _| {
                // Real PCM decoder and source fingerprint; only the ambiguous analysis
                // hint is controlled. The production estimator's 70–180 BPM search
                // does not itself establish a detected 60/240 BPM result.
                let mut decoded = crate::engine::decode::decode_audio(path)?;
                decoded.sample.bpm = seed;
                Ok(decoded)
            })
            .unwrap(),
        );
        self.app
            .start_library_store(files.0.join("catalog/library.json"));
        self.until(|g| !g.app.library_metadata.active() && g.app.library_metadata.durable);
    }
    fn file(&mut self, path: &std::path::Path) {
        self.text("Search crate", path.file_stem().unwrap().to_str().unwrap());
        assert_eq!(self.app.filtered().len(), 1);
        self.click("Load selected crate item to deck A");
        self.until(|g| {
            g.app.loads[0].as_ref().is_some_and(|l| {
                l.receipt
                    .as_ref()
                    .is_some_and(|r| r.state() == State::Current)
            }) && !g.app.library_metadata.active()
        });
        self.frame(vec![]);
    }
}

#[test]
fn recorded_human_drum_grid_is_edited_saved_reopened_and_auditioned_through_native_widgets() {
    let files = Files::new();
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/beatgrid/11_rock_100_beat_4-4.wav");
    let path = files.0.join("human-drum.wav");
    std::fs::copy(&source, &path).unwrap();
    let mut gui = Gui::new(48_000);
    gui.library(&files, 100.0);
    gui.app.loader = Some(crate::engine::media_load::Loader::with_decoder(|path, _| crate::engine::decode::decode_audio(path)).unwrap());
    assert!(gui.app.library_scan.start(vec![files.0.clone()], gui.app.library.clone()));
    gui.until(|g| !g.app.library_scan.active() && !g.app.library_metadata.active()
        && g.app.library.iter().any(|item| item.source == LibSource::File(path.clone())));
    gui.file(&path);
    let original = gui.rt.decks[0].audio.clone().unwrap();
    let analyzed = original.bpm;
    gui.action("Deck A: Waveform position", Action::SetValue, Some(ActionData::NumericValue(3.0)));
    gui.click("Deck A: Hot cue 1");
    let cue = gui.rt.decks[0].hotcues[0].pos;
    gui.open();
    gui.text("Deck A beatgrid: Downbeat seconds", "0");
    gui.text("Deck A beatgrid: Stretch tempo BPM", "100");
    for (beat, seconds) in [("4", "2.3925"), ("8", "4.7925"), ("12", "7.20125"), ("15", "8.99875")] {
        gui.text("Deck A beatgrid: Tempo anchor beat", beat);
        gui.text("Deck A beatgrid: Tempo anchor source seconds", seconds);
        gui.click("Deck A beatgrid: Insert or replace tempo anchor");
    }
    gui.apply();
    let grid = gui.rt.decks[0].grid.unwrap();
    assert_eq!(grid.anchors().len(), 4);
    gui.until(|g| !g.app.library_metadata.active()
        && g.app.cue_storage_status(&g.app.grid_editor.as_ref().unwrap().receipt) == "Saved in DJ library");
    assert!(std::sync::Arc::ptr_eq(&original, gui.rt.decks[0].audio.as_ref().unwrap()));
    assert_eq!(gui.rt.decks[0].hotcues[0].pos, cue);
    gui.close();
    let catalog = files.0.join("catalog/library.json");
    drop(gui);
    let deadline = Instant::now() + std::time::Duration::from_secs(5);
    while crate::library::Store::open(catalog.clone()).is_err() {
        assert!(Instant::now() < deadline); std::thread::sleep(std::time::Duration::from_millis(1));
    }
    let mut reopened = Gui::new(96_000);
    reopened.library(&files, analyzed);
    reopened.app.loader = Some(crate::engine::media_load::Loader::with_decoder(|path, _| crate::engine::decode::decode_audio(path)).unwrap());
    reopened.file(&path); reopened.open();
    assert_eq!(reopened.app.grid_editor.as_ref().unwrap().draft, Some(grid));
    assert_eq!(reopened.rt.decks[0].grid, Some(grid));
    assert_eq!(reopened.rt.decks[0].audio.as_ref().unwrap().bpm, analyzed);
    assert_eq!(reopened.rt.decks[0].hotcues[0].pos, cue);
    assert!(!reopened.rt.decks[0].playing);
    reopened.close();
    reopened.click("Deck A: Platter play or pause");
    let mut audio = [0.0; 8192];
    assert_eq!(test_alloc::measure(|| reopened.rt.process(&mut audio)), test_alloc::Counts::default());
    assert!(audio.iter().any(|sample| sample.abs() > 1e-4));
}

#[test]
fn actual_decoded_pickups_and_leading_silence_correct_ambiguous_seeds_and_reload_all_cues() {
    for (name, silence, seed, correction) in [
        ("pickup", false, 60.0, "Double tempo"),
        ("leading", true, 240.0, "Half tempo"),
    ] {
        let files = Files::new();
        let path = files.wave(name, silence);
        let mut gui = Gui::new(48_000);
        gui.library(&files, seed);
        assert!(gui
            .app
            .library_scan
            .start(vec![files.0.clone()], gui.app.library.clone()));
        gui.until(|g| {
            !g.app.library_scan.active()
                && !g.app.library_metadata.active()
                && g.app
                    .library
                    .iter()
                    .any(|i| i.source == LibSource::File(path.clone()))
        });
        gui.file(&path);
        assert_eq!(gui.rt.decks[0].audio.as_ref().unwrap().bpm, seed);
        assert_eq!(gui.rt.decks[0].audio.as_ref().unwrap().sr, 16_000);
        for cue in 0..8 {
            gui.action(
                "Deck A: Waveform position",
                Action::SetValue,
                Some(ActionData::NumericValue(0.25 + cue as f64 * 0.4)),
            );
            gui.click(&format!("Deck A: Hot cue {}", cue + 1));
        }
        let cues = gui.rt.decks[0].hotcues.clone();
        for (index, cue) in cues.iter().enumerate() {
            assert!(cue.set, "actual GUI must set all eight cue points");
            assert!((cue.pos / 16_000.0 - (0.25 + index as f64 * 0.4)).abs() < 1e-6);
        }
        gui.action(
            "Deck A: Waveform position",
            Action::SetValue,
            Some(ActionData::NumericValue(1.25)),
        );
        gui.open();
        assert_eq!(
            gui.app.grid_editor.as_ref().unwrap().draft.unwrap().bpm(),
            seed as f64
        );
        gui.click(&format!("Deck A beatgrid: {correction}"));
        gui.click("Deck A beatgrid: Set downbeat at playhead");
        gui.click("Deck A beatgrid: Slip +1 ms");
        gui.apply();
        let grid = gui.rt.decks[0].grid.unwrap();
        assert_eq!(grid.bpm(), 120.0);
        assert!((grid.downbeat() - 1.251).abs() < 1e-12);
        assert!(grid.beat_at(0.0).unwrap() < -2.0);
        gui.until(|g| {
            !g.app.library_metadata.active()
                && g.app
                    .cue_storage_status(&g.app.grid_editor.as_ref().unwrap().receipt)
                    == "Saved in DJ library"
        });
        for (old, current) in cues.iter().zip(&gui.rt.decks[0].hotcues) {
            assert_eq!((old.set, old.pos), (current.set, current.pos));
        }
        gui.close();
        drop(gui);
        // Worker-held lock retirement is asynchronous; retry only this private store.
        let deadline = Instant::now() + std::time::Duration::from_secs(5);
        while crate::library::Store::open(files.0.join("catalog/library.json")).is_err() {
            assert!(Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        let mut reopened = Gui::new(96_000);
        reopened.library(&files, seed);
        reopened.file(&path);
        assert_eq!(reopened.rt.decks[0].grid, Some(grid));
        assert_eq!(
            reopened.rt.decks[0].audio.as_ref().unwrap().bpm,
            seed,
            "manual preparation must not overwrite analysis"
        );
        assert_eq!(reopened.app.snap.decks[0].bpm, 120.0);
        for (old, current) in cues.iter().zip(&reopened.rt.decks[0].hotcues) {
            assert_eq!((old.set, old.pos), (current.set, current.pos));
            assert_eq!(
                grid.beat_at(old.pos / 16_000.0),
                grid.beat_at(current.pos / 16_000.0)
            );
        }
        reopened.open();
        assert_eq!(reopened.app.grid_editor.as_ref().unwrap().draft, Some(grid));
    }
}

#[test]
fn actual_preview_uses_all_three_mean_magnitude_bands_symmetrically() {
    let theme = Theme::default();
    for (bands, expected_height) in [
        ([0.0, 0.0, 0.5], 35.0),
        ([0.25, 0.25, 0.0], 35.0),
        ([0.0; 3], 0.0),
        ([0.8; 3], 70.0),
    ] {
        let mut snap = crate::engine::DeckSnap::default();
        snap.source_sample_rate = 48_000;
        snap.frames = 480_000.0;
        snap.peaks = Arc::new(vec![bands; 1024]);
        let ctx = egui::Context::default();
        let mut center = 0.0;
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(900.0, 400.0))),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    center = ui.cursor().min.y + 65.0;
                    preview(ui, &theme, &snap, None, None);
                });
            },
        );
        let bands: Vec<_> = output
            .shapes
            .iter()
            .filter_map(|s| match &s.shape {
                egui::epaint::Shape::LineSegment { points, stroke }
                    if stroke.color == theme.fg_dim.gamma_multiply(0.45) =>
                {
                    Some(points)
                }
                _ => None,
            })
            .collect();
        assert_eq!(
            bands.len(),
            512,
            "preview work remains bounded by displayed columns"
        );
        for points in bands {
            assert!((points[1].y - points[0].y - expected_height).abs() < 0.001);
            assert!(((points[1].y + points[0].y) * 0.5 - center).abs() < 0.001);
        }
    }
}

#[test]
fn request_ack_finishes_after_same_batch_replacement_or_undo_without_confusing_other_preparation() {
    for undo in [false, true] {
        let mut gui = Gui::new(48_000);
        gui.open();
        gui.text("Deck A beatgrid: Stretch tempo BPM", "150");
        gui.render = false;
        gui.apply();
        let editor = gui.app.grid_editor.as_ref().unwrap();
        let ack = editor.pending.as_ref().unwrap().ack.clone();
        let requested = editor.draft;
        assert_eq!(ack.state(), GridEditState::Pending);
        let later = Grid::new(0.125, 90.0).unwrap();
        if undo {
            gui.app.engine.send(Command::Undo).unwrap();
        } else {
            gui.app
                .engine
                .send(Command::DeckGrid {
                    deck: 0,
                    grid: Some(later),
                    receipt: editor.receipt.clone(),
                    ack: GridEditAck::new(),
                })
                .unwrap();
        }
        gui.rt.process(&mut [0.0; 128]);
        gui.rt.publish_for_test();
        gui.render = true;
        gui.frame(vec![]);
        assert_eq!(ack.state(), GridEditState::Applied);
        assert_eq!(gui.rt.decks[0].grid, if undo { None } else { Some(later) });
        let editor = gui.app.grid_editor.as_ref().unwrap();
        assert!(
            editor.pending.is_none(),
            "later preparation must not strand the request"
        );
        assert!(editor.message.contains("applied, then changed"));
        assert_eq!(editor.draft, requested, "keep the explicit draft available");
        assert!(!gui.node("Deck A beatgrid: Apply grid").1.is_disabled());
    }
    // An earlier edit can publish before this command reaches the bounded
    // renderer drain. Its newer preparation revision is not our completion.
    let mut gui = Gui::new(48_000);
    gui.open();
    gui.text("Deck A beatgrid: Stretch tempo BPM", "150");
    gui.render = false;
    let receipt = gui.app.grid_editor.as_ref().unwrap().receipt.clone();
    gui.app
        .engine
        .send(Command::DeckGrid {
            deck: 0,
            grid: Some(Grid::new(1.0, 90.0).unwrap()),
            receipt,
            ack: GridEditAck::new(),
        })
        .unwrap();
    for i in 0..31 {
        gui.app
            .engine
            .send(Command::TrackGain {
                track: 0,
                value: if i % 2 == 0 { 0.6 } else { 0.7 },
            })
            .unwrap();
    }
    gui.apply();
    let ack = gui
        .app
        .grid_editor
        .as_ref()
        .unwrap()
        .pending
        .as_ref()
        .unwrap()
        .ack
        .clone();
    gui.rt.process(&mut [0.0; 128]);
    gui.rt.publish_for_test();
    gui.frame(vec![]);
    assert_eq!(ack.state(), GridEditState::Pending);
    assert!(gui.app.grid_editor.as_ref().unwrap().pending.is_some());
    gui.rt.process(&mut [0.0; 128]);
    gui.rt.publish_for_test();
    gui.frame(vec![]);
    assert_eq!(ack.state(), GridEditState::Applied);
    assert!(gui.app.grid_editor.as_ref().unwrap().pending.is_none());
    assert_eq!(gui.rt.decks[0].grid.unwrap().bpm(), 150.0);
}

#[test]
fn reset_reopen_uses_source_hint_during_snapshot_lag_and_disconnected_request_is_unknown() {
    let mut gui = Gui::new(48_000);
    let source = gui.app.snap.decks[0].source_bpm;
    gui.open();
    gui.text("Deck A beatgrid: Stretch tempo BPM", "150");
    gui.apply();
    gui.close();
    assert_eq!(gui.app.snap.decks[0].bpm, 150.0);
    let stale = gui.app.snap.decks[0].clone();
    let receipt = gui.app.cue_receipt(stale.receipt_key).unwrap();
    gui.app
        .engine
        .send(Command::DeckGrid {
            deck: 0,
            grid: None,
            receipt: receipt.clone(),
            ack: GridEditAck::new(),
        })
        .unwrap();
    gui.rt.process(&mut []);
    // Keep the prior published effective BPM, but use the coherent new
    // preparation plus immutable source hint when reopening the editor.
    assert_eq!(gui.app.snap.decks[0].bpm, 150.0);
    gui.app.open_grid_editor(0);
    assert_eq!(
        gui.app.grid_editor.as_ref().unwrap().draft.unwrap().bpm(),
        source as f64
    );
    gui.rt.publish_for_test();
    gui.frame(vec![]);
    gui.text("Deck A beatgrid: Stretch tempo BPM", "160");
    gui.render = false;
    gui.apply();
    assert!(gui.app.grid_editor.as_ref().unwrap().pending.is_some());
    let (_, other) = Engine::headless_for_test(48_000, 256);
    drop(std::mem::replace(&mut gui.rt, other));
    gui.frame(vec![]);
    let editor = gui.app.grid_editor.as_ref().unwrap();
    assert!(editor.pending.is_none());
    assert!(editor.message.contains("outcome is unknown"));
    assert_eq!(editor.draft.unwrap().bpm(), 160.0);
}

#[test]
fn newly_valid_tempo_does_not_retarget_an_already_exposed_native_grid_action() {
    let mut gui = Gui::new(48_000);
    gui.action(
        "Deck A: Waveform position",
        Action::SetValue,
        Some(ActionData::NumericValue(1.25)),
    );
    let expected_playhead = source_seconds(&gui.app.snap.decks[0]);
    gui.app.snap.decks[0].source_bpm = 0.0;
    gui.app.open_grid_editor(0);
    gui.frame(vec![]);
    assert!(gui.app.grid_editor.as_ref().unwrap().draft.is_none());
    gui.action("Deck A beatgrid: Stretch tempo BPM", Action::Focus, None);
    gui.frame(vec![egui::Event::Text("120".into())]);
    let target = gui.node("Deck A beatgrid: Set downbeat at playhead").0;
    assert!(gui.app.grid_editor.as_ref().unwrap().draft.is_some());
    // That frame exposed Set before the new draft's playhead beat label was
    // rendered. Its native identity must survive the label appearing now.
    gui.frame(vec![egui::Event::AccessKitActionRequest(ActionRequest {
        action: Action::Click,
        target,
        data: None,
    })]);
    assert_eq!(
        gui.app
            .grid_editor
            .as_ref()
            .unwrap()
            .draft
            .unwrap()
            .downbeat(),
        expected_playhead
    );
    assert_eq!(gui.rt.decks[0].grid, None);
}

#[test]
fn grid_only_capture_defers_hashing_under_protection_then_relocates_after_studio_qualification() {
    let files = Files::new();
    let original = files.wave("grid-only", false);
    let source = LibSource::File(original.clone());
    let fingerprint = FileFingerprint::read(&original);
    let mut gui = Gui::new(48_000);
    gui.library(&files, 60.0);
    assert!(gui
        .app
        .library_scan
        .start(vec![files.0.clone()], gui.app.library.clone()));
    gui.until(|g| {
        !g.app.library_scan.active()
            && !g.app.library_metadata.active()
            && g.app.library.iter().any(|i| i.source == source)
    });
    gui.file(&original);
    gui.open();
    gui.text("Deck A beatgrid: Downbeat seconds", "1.25");
    gui.click("Deck A beatgrid: Double tempo");
    gui.render = false;
    gui.apply();
    gui.rt.process(&mut [0.0; 128]);
    gui.rt.publish_for_test();
    // The renderer committed the edit, but GUI capture/optional qualification
    // has not run yet. Essential preparation saving must continue under protection.
    gui.app.engine.cmd.performance().set_enabled(true).unwrap();
    gui.render = true;
    gui.until(|g| {
        !g.app.library_metadata.active()
            && g.app
                .cue_storage_status(&g.app.grid_editor.as_ref().unwrap().receipt)
                .starts_with("Saved in DJ library")
    });
    assert!(gui.app.cue_storage_status(&gui.app.grid_editor.as_ref().unwrap().receipt).contains("move verification pending"));
    let stored = gui
        .app
        .library_metadata
        .catalog
        .version(&source, fingerprint)
        .unwrap();
    let grid = stored.preparation.grid.unwrap();
    assert!(stored.preparation.hotcues.iter().all(Option::is_none));
    assert_eq!(
        stored.content_hash, None,
        "optional hashing must remain deferred"
    );
    assert_eq!(
        crate::library::read(&files.0.join("catalog/library.json"))
            .unwrap()
            .version(&source, fingerprint)
            .unwrap()
            .preparation
            .grid,
        Some(grid)
    );
    gui.app.engine.cmd.performance().set_enabled(false).unwrap();
    gui.until(|g| {
        !g.app.library_metadata.active()
            && g.app
                .library_metadata
                .catalog
                .version(&source, fingerprint)
                .is_some_and(|v| v.content_hash.is_some())
    });
    let id = gui
        .app
        .library_metadata
        .catalog
        .track(&source)
        .unwrap()
        .id
        .clone();
    let moved = files.0.join("relocated-grid.wav");
    std::fs::rename(&original, &moved).unwrap();
    gui.close();
    gui.app.open_cue_relocation();
    gui.frame(vec![]);
    gui.frame(vec![]);
    gui.text("Relocated track path", moved.to_str().unwrap());
    gui.click("Verify and relocate track");
    gui.until(|g| {
        !g.app.library_metadata.active()
            && g.app
                .library_metadata
                .catalog
                .track(&LibSource::File(moved.clone()))
                .is_some_and(|track| track.id == id)
    });
    // The old bytes no longer exist at their original path, so this successful
    // association necessarily used the earlier grid-only content qualification.
    assert!(!original.exists());
    let old_receipt = gui.app.snap.decks[0].receipt_key;
    gui.text("Search crate", "grid-only");
    gui.click("Load selected crate item to deck A");
    gui.until(|g| {
        g.app.snap.decks[0].receipt_key != old_receipt && g.app.snap.decks[0].grid == Some(grid)
    });
    assert_eq!(gui.rt.decks[0].grid, Some(grid));
    assert!(gui.rt.decks[0].hotcues.iter().all(|cue| !cue.set));
    assert_eq!(gui.rt.decks[0].audio.as_ref().unwrap().bpm, 60.0);
}
