use super::*;
use crate::ui::test_support::Fixture;
use egui::accesskit::{Action, ActionData, ActionRequest, Node, NodeId};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

struct Files(PathBuf);
impl Files {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "omatainer-cues98-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn store(&self) -> PathBuf {
        self.0.join("saved/library.json")
    }
    fn wave(&self, name: &str) -> PathBuf {
        let path = self.0.join(name);
        let size = 48_000u32 * 2 * 2;
        let mut bytes = b"RIFF".to_vec();
        bytes.extend((36 + size).to_le_bytes());
        bytes.extend(b"WAVEfmt ");
        bytes.extend(16u32.to_le_bytes());
        bytes.extend(1u16.to_le_bytes());
        bytes.extend(1u16.to_le_bytes());
        bytes.extend(48_000u32.to_le_bytes());
        bytes.extend(96_000u32.to_le_bytes());
        bytes.extend(2u16.to_le_bytes());
        bytes.extend(16u16.to_le_bytes());
        bytes.extend(b"data");
        bytes.extend(size.to_le_bytes());
        bytes.resize(bytes.len() + size as usize, 0);
        std::fs::write(&path, bytes).unwrap();
        path
    }
}
impl Drop for Files {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct Gui {
    f: Fixture,
    ctx: egui::Context,
    nodes: Vec<(NodeId, Node)>,
    time: f64,
    size: Vec2,
    render: bool,
}
impl Gui {
    fn new() -> Self {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut gui = Self {
            f: Fixture::new(144),
            ctx,
            nodes: vec![],
            time: 0.0,
            size: Vec2::new(1600.0, 1200.0),
            render: true,
        };
        gui.f.rt.publish_for_test();
        gui.frame(vec![]);
        gui.frame(vec![]);
        gui
    }
    fn frame(&mut self, events: Vec<egui::Event>) -> egui::FullOutput {
        self.time += 0.02;
        let modifiers = events
            .iter()
            .rev()
            .find_map(|e| match e {
                egui::Event::Key { modifiers, .. } => Some(*modifiers),
                _ => None,
            })
            .unwrap_or_default();
        let out = self.ctx.run(
            egui::RawInput {
                events,
                modifiers,
                focused: true,
                time: Some(self.time),
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, self.size)),
                ..Default::default()
            },
            |ctx| self.f.app.update_frame(ctx),
        );
        self.nodes = out
            .platform_output
            .accesskit_update
            .as_ref()
            .unwrap()
            .nodes
            .clone();
        if self.render {
            self.f.rt.process(&mut [0.0; 128]);
            self.f.rt.publish_for_test();
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
    fn action(&mut self, name: &str, action: Action, data: Option<ActionData>) {
        let target = self.node(name).0;
        self.frame(vec![egui::Event::AccessKitActionRequest(ActionRequest {
            target,
            action,
            data,
        })]);
        self.frame(vec![]);
    }
    fn click(&mut self, name: &str) {
        self.action(name, Action::Click, None);
    }
    fn text(&mut self, name: &str, value: &str) {
        self.action(name, Action::Focus, None);
        let modifiers = egui::Modifiers {
            ctrl: true,
            command: true,
            ..Default::default()
        };
        for pressed in [true, false] {
            self.frame(vec![egui::Event::Key {
                key: Key::A,
                physical_key: None,
                pressed,
                repeat: false,
                modifiers,
            }]);
        }
        self.frame(vec![egui::Event::Text(value.into())]);
        self.frame(vec![]);
    }
    fn settle(&mut self) {
        let end = Instant::now() + Duration::from_secs(5);
        loop {
            self.frame(vec![]);
            if !self.f.app.library_metadata.active() && !self.f.app.library_scan.active() {
                break;
            }
            assert!(
                Instant::now() < end,
                "{}",
                self.f.app.library_metadata.label()
            );
            std::thread::sleep(Duration::from_millis(1));
        }
        self.frame(vec![]);
    }
    fn select(&mut self, source: &LibSource) {
        self.f.app.refresh_library_view();
        self.f.app.lib_sel = self
            .f
            .app
            .library_view
            .indices
            .iter()
            .position(|&i| self.f.app.library[i].source == *source)
            .unwrap();
        self.f.app.refresh_library_view();
    }
    fn open_editor(&mut self) {
        self.action(
            "Deck A: Hot cue 1",
            Action::CustomAction,
            Some(ActionData::CustomAction(2)),
        );
        assert!(self.f.app.cue_editor.editor.is_some());
    }
    fn close_editor(&mut self) {
        self.click("Close cue editor");
        assert!(self.f.app.cue_editor.editor.is_none());
    }
}

#[test]
fn actual_eight_cue_editor_fields_apply_persist_reload_and_match_waveform_and_pads() {
    let files = Files::new();
    let mut gui = Gui::new();
    gui.f.app.start_library_store(files.store());
    gui.settle();
    gui.open_editor();
    for i in 0..8 {
        gui.f
            .app
            .engine
            .send(Command::DeckSeek {
                deck: 0,
                frac: 0.02 * i as f32,
            })
            .unwrap();
        gui.f.rt.process(&mut []);
        gui.f.rt.publish_for_test();
        gui.frame(vec![]);
        gui.click(&format!("Set {}", i + 1));
        gui.text(
            &format!("Cue {} name", i + 1),
            &format!("Part {} 東京", i + 1),
        );
        gui.text(
            &format!("Cue {} color", i + 1),
            &format!("#{:02X}A4E1", i * 29),
        );
        gui.click(&format!("Apply {}", i + 1));
        let applied = gui.f.rt.decks[0].cue_styles[i];
        assert_eq!(applied.name.as_str(), format!("Part {} 東京", i + 1));
        assert_eq!(applied.color, Some([i as u8 * 29, 164, 225]));
        assert!(gui.f.rt.decks[0].hotcues[i].set);
    }
    gui.settle();
    let saved = gui.f.app.engine.initial_playback[0].as_ref().unwrap()
        .preparation()
        .unwrap()
        .1;
    assert_eq!(
        gui.f
            .app
            .cue_storage_status(gui.f.app.engine.initial_playback[0].as_ref().unwrap()),
        "Saved in DJ library"
    );
    gui.close_editor();
    let rendered = gui.frame(vec![]);
    for i in 0..8 {
        let rgb = Color32::from_rgb(i as u8 * 29, 164, 225);
        assert!(
            rendered.shapes.iter().any(|shape| matches!(&shape.shape,
            egui::epaint::Shape::LineSegment { stroke, .. } if stroke.color == rgb)),
            "waveform cue {i} color absent"
        );
        assert!(
            rendered.shapes.iter().any(|shape| matches!(&shape.shape,
            egui::epaint::Shape::Rect(rect) if rect.stroke.color == rgb)),
            "pad cue {i} color absent"
        );
        let name = format!("Deck A: Hot cue {}: Part {} 東京", i + 1, i + 1);
        let description = gui.node(&name).1.description().unwrap();
        assert!(
            description.contains(&format!("#{:02X}A4E1", i * 29)),
            "{description}"
        );
        assert!(
            description.contains(&format!("{:.3} seconds", saved.hotcues[i].unwrap())),
            "{description}"
        );
        assert!(gui
            .node("Deck A: Waveform position")
            .1
            .description()
            .unwrap()
            .contains(&format!("Part {} 東京", i + 1)));
    }
    drop(gui);
    let end = Instant::now() + Duration::from_secs(5);
    while crate::library::Store::open(files.store()).is_err() {
        assert!(Instant::now() < end);
        std::thread::sleep(Duration::from_millis(1));
    }
    let mut reopened = Gui::new();
    reopened.f.app.start_library_store(files.store());
    reopened.settle();
    assert_eq!(
        reopened.f.app.engine.initial_playback[0].as_ref().unwrap()
            .preparation()
            .unwrap()
            .1,
        saved
    );
    reopened.action(
        "Deck A: Hot cue 1: Part 1 東京",
        Action::CustomAction,
        Some(ActionData::CustomAction(2)),
    );
    for i in 0..8 {
        assert_eq!(
            reopened.node(&format!("Cue {} name", i + 1)).1.value(),
            Some(format!("Part {} 東京", i + 1).as_str())
        );
    }
}

#[test]
fn actual_editor_rejects_invalid_drafts_and_stale_queued_cue_actions_without_retargeting() {
    let mut gui = Gui::new();
    gui.open_editor();
    gui.click("Set 1");
    gui.text("Cue 1 name", &"🎵".repeat(17));
    gui.click("Apply 1");
    assert!(gui
        .f
        .app
        .cue_editor
        .editor
        .as_ref()
        .unwrap()
        .message
        .contains("64 UTF-8 bytes"));
    assert_eq!(gui.f.rt.decks[0].cue_styles[0], Style::default());
    gui.text("Cue 1 name", "Chorus");
    gui.text("Cue 1 color", "#xyz123");
    gui.click("Apply 1");
    assert!(gui
        .f
        .app
        .cue_editor
        .editor
        .as_ref()
        .unwrap()
        .message
        .contains("hexadecimal"));
    gui.text("Cue 1 color", "#FF0088");
    gui.render = false;
    gui.click("Apply 1");
    gui.click("Set 2");
    gui.click("Delete 1");
    let receipt = crate::engine::load_receipt::Receipt::new();
    gui.f.rt.apply(Command::DeckLoadRequested {
        deck: 0,
        media: crate::engine::load_receipt::Media::Builtin(1),
        receipt,
    });
    gui.f.rt.apply(Command::DeckHotCue {
        deck: 0,
        pad: 0,
        del: false,
    });
    let before = gui.f.app.engine.undo.checkpoint();
    gui.render = true;
    gui.frame(vec![]);
    gui.frame(vec![]);
    assert!(gui.f.app.cue_editor.editor.is_none());
    assert!(gui.f.rt.decks[0].hotcues[0].set);
    assert!(!gui.f.rt.decks[0].hotcues[1].set);
    assert_eq!(gui.f.rt.decks[0].cue_styles, [Style::default(); 8]);
    assert_eq!(gui.f.app.engine.undo.checkpoint(), before);
}

#[test]
fn actual_relocation_worker_reports_completion_follows_identity_and_preserves_manual_browse() {
    for (browse_away, cancel_for_show) in [(false, false), (true, false), (false, true)] {
        let files = Files::new();
        let original = files.wave("original.wav");
        let moved = files.0.join("moved.wav");
        let source = LibSource::File(original.clone());
        let fingerprint = FileFingerprint::read(&original);
        let mut gui = Gui::new();
        let pause = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let worker_pause = pause.clone();
        let (started_tx, started_rx) = std::sync::mpsc::sync_channel(1);
        let (resume_tx, resume_rx) = std::sync::mpsc::sync_channel(1);
        gui.f.app.library_metadata =
            library_metadata::Metadata::with_hook(files.store(), move || {
                if worker_pause.swap(false, Ordering::AcqRel) {
                    started_tx.send(()).unwrap();
                    resume_rx.recv().unwrap();
                }
            });
        gui.f
            .app
            .library_metadata
            .set_performance(gui.f.app.engine.cmd.performance().clone());
        gui.settle();
        gui.f
            .app
            .library_scan
            .start(vec![files.0.clone()], gui.f.app.library.clone());
        gui.settle();
        gui.select(&source);
        gui.f.app.load_sel(0);
        let (_, path) = gui
            .f
            .decoder_jobs
            .recv_timeout(Duration::from_secs(2))
            .unwrap();
        assert_eq!(path, original);
        gui.f
            .decoder_results
            .send((0, crate::engine::decode::decode_audio(&path)))
            .unwrap();
        gui.f.poll_loads();
        gui.frame(vec![]);
        gui.frame(vec![]);
        gui.open_editor();
        gui.click("Set 1");
        gui.text("Cue 1 name", "Saved drop");
        gui.text("Cue 1 color", "#112233");
        gui.click("Apply 1");
        gui.settle();
        gui.close_editor();
        // Credit actual rendered playback, then stop it before file relocation.
        gui.f
            .app
            .engine
            .send(Command::DeckPlay { deck: 0 })
            .unwrap();
        gui.frame(vec![]);
        gui.frame(vec![]);
        gui.f
            .app
            .engine
            .send(Command::DeckPlay { deck: 0 })
            .unwrap();
        gui.frame(vec![]);
        gui.settle();
        assert!(gui.f.app.last_played.latest_identity().is_some());
        let id = gui
            .f
            .app
            .library_metadata
            .catalog
            .track(&source)
            .unwrap()
            .id
            .clone();
        std::fs::rename(&original, &moved).unwrap();
        gui.f.app.open_cue_relocation();
        gui.frame(vec![]);
        gui.frame(vec![]);
        gui.text("Relocated track path", moved.to_str().unwrap());
        pause.store(true, Ordering::Release);
        gui.click("Verify and relocate track");
        started_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        let other = LibSource::Builtin(BuiltinStem::Harmony);
        if browse_away {
            gui.select(&other);
        }
        if cancel_for_show {
            gui.f
                .app
                .engine
                .send(Command::PerformanceMode(true))
                .unwrap();
        }
        // Force a stale worker publication and a reorder while relocation is
        // accepted, so index clamping cannot accidentally masquerade as identity.
        let mut metadata = gui.f.app.capture_metadata(&source, fingerprint);
        metadata.bpm = Bpm::new(200.0, Origin::User);
        gui.f
            .app
            .library_metadata
            .capture(super::super::library_store::Capture {
                source: source.clone(),
                fingerprint,
                metadata,
                preparation: None,
                played: None,
            });
        resume_tx.send(()).unwrap();
        gui.settle();
        let relocation = gui.f.app.cue_editor.relocation.as_ref().unwrap();
        assert!(!relocation.pending);
        if cancel_for_show {
            assert!(!relocation.saved);
            assert!(
                relocation.message.contains("cancel"),
                "{}",
                relocation.message
            );
            let current = gui.f.app.library_metadata.catalog.track(&source).unwrap();
            assert_eq!(current.id, id);
            assert_eq!(
                current.versions[current.current].metadata.bpm.value(),
                Some(200.0)
            );
            assert_eq!(
                current.versions[current.current].preparation.hotcue_styles[0]
                    .name
                    .as_str(),
                "Saved drop"
            );
            assert!(gui
                .f
                .app
                .library_metadata
                .catalog
                .track(&LibSource::File(moved))
                .is_none());
            continue;
        }
        assert!(relocation.saved, "{}", relocation.message);
        assert!(relocation.message.starts_with("Relocation saved:"));
        let target = LibSource::File(moved.clone());
        assert_eq!(
            gui.f
                .app
                .library_metadata
                .catalog
                .track(&target)
                .unwrap()
                .id,
            id
        );
        assert_eq!(
            gui.f.app.selected_library_item().unwrap().source,
            if browse_away { other } else { target.clone() }
        );
        let history_row = gui.f.app.library_view.indices[gui.f.app.last_play_idx];
        assert_eq!(
            gui.f.app.library[history_row].source, target,
            "old receipt history did not follow verified relocation"
        );
        gui.f.app.lib_filter = "hidden by this query".into();
        gui.f.app.refresh_library_view();
        assert!(gui.f.app.library_view.indices.is_empty());
        gui.f.app.lib_filter.clear();
        gui.f.app.refresh_library_view();
        let history_row = gui.f.app.library_view.indices[gui.f.app.last_play_idx];
        assert_eq!(gui.f.app.library[history_row].source, target);
        let version = gui
            .f
            .app
            .library_metadata
            .catalog
            .version(&target, FileFingerprint::read(&moved))
            .unwrap();
        assert_eq!(
            version.preparation.hotcue_styles[0].name.as_str(),
            "Saved drop"
        );
        assert_eq!(
            version.preparation.hotcue_styles[0].color,
            Some([17, 34, 51])
        );
        // A stale caller retrying this already-associated destination must see
        // verification rejection, never infer success from its existing TrackId.
        let stale_request = gui
            .f
            .app
            .cue_editor
            .relocation
            .as_ref()
            .unwrap()
            .request
            .clone();
        assert!(gui.f.app.library_metadata.relocate(stale_request.clone()));
        gui.settle();
        assert!(gui
            .f
            .app
            .library_metadata
            .relocation_result(&stale_request)
            .unwrap()
            .as_ref()
            .unwrap_err()
            .contains("original track is no longer"));
        gui.f.app.cue_editor.relocation = None;
        gui.select(&target);
        gui.f.app.load_sel(0);
        let (_, path) = gui
            .f
            .decoder_jobs
            .recv_timeout(Duration::from_secs(2))
            .unwrap();
        assert_eq!(path, moved);
        gui.f
            .decoder_results
            .send((0, crate::engine::decode::decode_audio(&path)))
            .unwrap();
        gui.f.poll_loads();
        gui.frame(vec![]);
        gui.frame(vec![]);
        assert_eq!(gui.f.rt.decks[0].cue_styles[0].name.as_str(), "Saved drop");
        assert!(gui.f.rt.decks[0].hotcues[0].set);
    }
}

#[test]
fn small_cue_window_scrolls_focused_last_row_into_reach_without_leaking_global_keys() {
    let mut gui = Gui::new();
    gui.size = Vec2::new(740.0, 500.0);
    gui.open_editor();
    gui.click("Set 8");
    gui.action("Cue 8 name", Action::Focus, None);
    gui.frame(vec![]);
    let bounds = gui.node("Cue 8 name").1.bounds().unwrap();
    assert!(
        bounds.y0 >= 0.0 && bounds.y1 <= 500.0,
        "last-row field remained outside the window: {bounds:?}"
    );
    let was_playing = gui.f.rt.playing;
    gui.text("Cue 8 name", "Last part");
    gui.frame(vec![egui::Event::Key {
        key: Key::Space,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Default::default(),
    }]);
    gui.frame(vec![egui::Event::Key {
        key: Key::Space,
        physical_key: None,
        pressed: false,
        repeat: false,
        modifiers: Default::default(),
    }]);
    assert_eq!(
        gui.f.rt.playing, was_playing,
        "cue text editing toggled global transport"
    );
    gui.click("Apply 8");
    assert_eq!(gui.f.rt.decks[0].cue_styles[7].name.as_str(), "Last part");
}

#[test]
fn protected_cue_edits_are_durable_while_hashing_and_relocation_wait_for_studio() {
    let files = Files::new();
    let original = files.wave("protected.wav");
    let moved = files.0.join("moved.wav");
    let source = LibSource::File(original.clone());
    let fingerprint = FileFingerprint::read(&original);
    let mut gui = Gui::new();
    gui.f.app.start_library_store(files.store());
    gui.settle();
    gui.f
        .app
        .library_scan
        .start(vec![files.0.clone()], gui.f.app.library.clone());
    gui.settle();
    gui.select(&source);
    gui.f.app.load_sel(0);
    let (_, path) = gui
        .f
        .decoder_jobs
        .recv_timeout(Duration::from_secs(2))
        .unwrap();
    gui.f
        .decoder_results
        .send((0, crate::engine::decode::decode_audio(&path)))
        .unwrap();
    gui.f.poll_loads();
    gui.frame(vec![]);
    gui.frame(vec![]);
    gui.settle();
    gui.f
        .app
        .engine
        .send(Command::PerformanceMode(true))
        .unwrap();
    gui.open_editor();
    gui.click("Set 1");
    gui.text("Cue 1 name", "Protected take");
    gui.text("Cue 1 color", "#09F0A2");
    gui.click("Apply 1");
    gui.settle();
    let version = gui
        .f
        .app
        .library_metadata
        .catalog
        .version(&source, fingerprint)
        .unwrap();
    assert_eq!(
        version.preparation.hotcue_styles[0].name.as_str(),
        "Protected take"
    );
    assert!(version.content_hash.is_none());
    assert!(gui.f.app.library_metadata.durable);
    assert!(gui
        .f
        .app
        .cue_storage_status(
            &gui.f.app.loads[0]
                .as_ref()
                .unwrap()
                .receipt
                .clone()
                .unwrap()
        )
        .contains("move verification pending"));
    gui.click("Delete 1");
    assert!(
        gui.f.rt.decks[0].hotcues[0].set,
        "protected editor deletion was admitted"
    );
    gui.close_editor();
    gui.f.app.open_cue_relocation();
    gui.frame(vec![]);
    gui.frame(vec![]);
    gui.text("Relocated track path", moved.to_str().unwrap());
    gui.click("Verify and relocate track");
    assert!(!gui.f.app.cue_editor.relocation.as_ref().unwrap().pending);
    assert!(gui
        .f
        .app
        .cue_editor
        .relocation
        .as_ref()
        .unwrap()
        .message
        .contains("Performance"));
    assert!(gui.f.app.library_metadata.catalog.track(&source).is_some());
    gui.f
        .app
        .engine
        .send(Command::PerformanceMode(false))
        .unwrap();
    gui.settle();
    assert!(
        gui.f
            .app
            .library_metadata
            .catalog
            .version(&source, fingerprint)
            .unwrap()
            .content_hash
            .is_some(),
        "deferred qualification did not resume"
    );
}

#[test]
fn native_cue_editor_displays_saved_region_and_explicit_cue_only_override_preserves_its_link() {
    use crate::engine::deck_controls::{Control, SavedLoopAction};
    let mut gui = Gui::new();
    let rate = f64::from(gui.f.rt.decks[0].audio.as_ref().unwrap().sr);
    gui.f.rt.decks[0].loop_start = rate * 0.5;
    gui.f.rt.decks[0].loop_len = rate * 0.25;
    let media_key = gui.f.app.engine.snapshot().decks[0].media_key;
    for action in [SavedLoopAction::Save, SavedLoopAction::Cue { pad: 4 }] {
        gui.f.rt.apply(Command::DeckControl { source: 0, deck: 0, control: Control::SavedLoop { media_key, id: 3, action } });
    }
    gui.f.rt.publish_for_test();gui.frame(vec![]);
    gui.click("Deck A: Cue editor");
    gui.click("Cue 5 only");
    assert!(gui.f.rt.decks[0].playing);
    assert!(!gui.f.rt.decks[0].loop_on);
    assert_eq!(gui.f.app.engine.snapshot().decks[0].saved_loops.cue_loops[4], Some(3));
    assert!((gui.f.rt.decks[0].pos / rate - 0.5).abs() < 0.01);
    gui.click("Jump 5");
    assert!(gui.f.rt.decks[0].loop_on);
    let snap = gui.f.app.engine.snapshot().decks[0].clone();
    assert!(super::description(&snap, 4).contains("saved loop 3"));
}
