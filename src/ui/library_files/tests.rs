use super::*;
use crate::engine::{
    media_analysis::tests::{wav, Files},
    RtEngine,
};
use egui::accesskit::{Action as NativeAction, ActionRequest, Node, NodeId};
use std::{fs, time::Duration};

struct Gui {
    app: App,
    rt: Box<RtEngine>,
    ctx: egui::Context,
    nodes: Vec<(NodeId, Node)>,
    time: f64,
    reference: Option<(Engine, Box<RtEngine>)>,
    frames: u64,
}
impl Gui {
    fn new(files: &Files) -> Self {
        let (engine, mut rt) = Engine::headless_for_test(48000, 256);
        rt.publish_for_test();
        let loader = Loader::start_with_performance(engine.cmd.performance().clone()).unwrap();
        let mut app = App::with_loader(engine, Theme::default(), Some(loader));
        app.start_library_store(files.0.join("catalog.json"));
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut gui = Self {
            app,
            rt: Box::new(rt),
            ctx,
            nodes: Vec::new(),
            time: 0.0,
            reference: None,
            frames: 0,
        };
        gui.settle();
        gui
    }
    fn frame(&mut self, events: Vec<egui::Event>) {
        self.time += 0.02;
        let modifiers = events
            .iter()
            .rev()
            .find_map(|event| {
                if let egui::Event::Key { modifiers, .. } = event {
                    Some(*modifiers)
                } else {
                    None
                }
            })
            .unwrap_or_default();
        let output = self.ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1800.0, 1800.0))),
                time: Some(self.time),
                focused: true,
                events,
                modifiers,
                ..Default::default()
            },
            |ctx| self.app.update_frame(ctx),
        );
        self.nodes = output.platform_output.accesskit_update.unwrap().nodes;
        let mut pcm = vec![0.0; 256];
        self.rt.process(&mut pcm);
        self.rt.publish_for_test();
        if let Some((_, reference)) = &mut self.reference {
            let mut expected = vec![0.0; 256];
            reference.process(&mut expected);
            assert_eq!(pcm, expected, "File management changed playing deck PCM");
        }
        self.frames += 128;
    }
    fn wait(&mut self, mut ready: impl FnMut(&Self) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            self.frame(Vec::new());
            if ready(self) {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "Native file workflow timed out: {} / {} / {}",
                self.app.library_files.message,
                self.app.library_crates.message,
                self.app.library_metadata.label()
            );
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    fn settle(&mut self) {
        let mut quiet = 0;
        self.wait(|gui| {
            if !gui.app.library_metadata.active()
                && !gui.app.library_scan.active()
                && gui.app.library_crates.pending.is_none()
            {
                quiet += 1;
            } else {
                quiet = 0;
            }
            quiet >= 3
        });
    }
    fn node(&self, label: &str) -> NodeId {
        self.nodes
            .iter()
            .find(|(_, node)| node.label() == Some(label))
            .map(|(id, _)| *id)
            .unwrap_or_else(|| {
                panic!(
                    "Missing {label}: {:?}",
                    self.nodes
                        .iter()
                        .filter_map(|(_, node)| node.label())
                        .collect::<Vec<_>>()
                )
            })
    }
    fn action(&mut self, label: &str, action: NativeAction) {
        let target = self.node(label);
        self.frame(vec![egui::Event::AccessKitActionRequest(ActionRequest {
            target,
            action,
            data: None,
        })]);
        self.frame(Vec::new());
    }
    fn click(&mut self, label: &str) {
        self.action(label, NativeAction::Click);
    }
    fn text(&mut self, label: &str, value: &str) {
        self.action(label, NativeAction::Focus);
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
        self.frame(Vec::new());
    }
    fn select(&mut self, path: &std::path::Path) {
        self.app.lib_filter.clear();
        self.app.refresh_library_view();
        self.app.lib_sel = self
            .app
            .library_view
            .indices
            .iter()
            .position(|&index| {
                self.app.library[index].source == LibSource::File(path.to_path_buf())
            })
            .unwrap();
        self.app.refresh_library_view();
    }
    fn playing(&mut self) {
        self.app.load_sel(0);
        self.wait(|gui| {
            gui.app.loads[0]
                .as_ref()
                .is_some_and(|load| matches!(load.phase, load_status::Phase::Loaded))
        });
        self.settle();
        self.app.engine.send(Command::DeckPlay { deck: 0 }).unwrap();
        self.frame(Vec::new());
        self.settle();
        let (engine, mut rt) = Engine::headless_for_test(48000, 256);
        rt.decks = self.rt.decks.clone();
        self.reference = Some((engine, Box::new(rt)));
    }
    fn open(&mut self) {
        self.click("files…");
        self.frame(Vec::new());
        self.frame(Vec::new());
    }
    fn apply(&mut self) {
        self.click("Review music file operation");
        self.frame(Vec::new());
        self.click("Apply reviewed music file operation");
        self.settle();
    }
}
fn seed(files: &Files) -> Vec<crate::library::Track> {
    let mut store = crate::library::Store::open(files.0.join("catalog.json")).unwrap();
    for index in 0..3 {
        let proof = files.source(&format!("track{index}.wav"), &wav(80000, 8000, 1, false));
        let LibSource::File(path) = &proof.source else {
            unreachable!()
        };
        let hash = crate::library::hash_project_source(path, proof.fingerprint, || true).unwrap();
        let version = store
            .catalog
            .upsert(
                proof.source,
                Some(proof.fingerprint),
                crate::library::Metadata {
                    title: format!("File {index}"),
                    artist: "Native files".into(),
                    bpm: Bpm::new(128.0, bpm::Origin::User),
                    key: "Am".into(),
                    duration: Some(10.0),
                    last_play: None,
                },
            )
            .unwrap();
        version.preparation.cue = index as f64 / 10.0;
        version.preparation.hotcues[7] = Some(3.0);
        version.content_hash = Some(hash);
    }
    store.save().unwrap();
    store.catalog.tracks.clone()
}
fn track_path(track: &crate::library::Track) -> PathBuf {
    let LibSource::File(path) = &track.source else {
        unreachable!()
    };
    path.clone()
}
fn wait_closed(path: PathBuf) {
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        if let Ok(store) = crate::library::Store::open(path.clone()) {
            drop(store);
            break;
        }
        assert!(
            Instant::now() < deadline,
            "Native file worker did not close"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn native_accessible_copy_move_restore_and_reference_removal_keep_playing_pcm_and_saved_preparation(
) {
    let files = Files::new();
    let baseline = seed(&files);
    let mut gui = Gui::new(&files);
    gui.select(&track_path(&baseline[0]));
    gui.playing();
    gui.open();
    gui.click("Capture selected music file");
    gui.settle();
    let copies = files.0.join("copies");
    fs::create_dir(&copies).unwrap();
    gui.text("Music file destination folder", copies.to_str().unwrap());
    gui.apply();
    let id = baseline[0].id.clone();
    let current = gui
        .app
        .library_metadata
        .catalog
        .tracks
        .iter()
        .find(|track| track.id == id)
        .unwrap();
    assert_eq!(track_path(current), copies.join("track0.wav"));
    assert!(track_path(&baseline[0]).exists());
    assert_eq!(
        current.versions[current.current].preparation.hotcues,
        baseline[0].versions[baseline[0].current]
            .preparation
            .hotcues
    );
    gui.select(&copies.join("track0.wav"));
    gui.click("Capture selected music file");
    gui.settle();
    gui.click("Move with recovery");
    let moved = files.0.join("moved");
    fs::create_dir(&moved).unwrap();
    let sentinel = moved.join("foreign.wav");
    fs::write(&sentinel, b"leave me alone").unwrap();
    gui.text("Music file destination folder", moved.to_str().unwrap());
    gui.apply();
    assert!(!copies.join("track0.wav").exists());
    assert!(moved.join("track0.wav").exists());
    assert!(gui.app.library_files.recovery.is_some());
    gui.click("Inspect file recovery");
    gui.settle();
    gui.click("Restore reviewed original music files");
    gui.settle();
    assert!(copies.join("track0.wav").exists());
    assert!(!moved.join("track0.wav").exists());
    assert!(gui.app.library_files.recovery.is_none());
    assert_eq!(fs::read(&sentinel).unwrap(), b"leave me alone");
    gui.select(&copies.join("track0.wav"));
    gui.click("Capture selected music file");
    gui.settle();
    gui.click("Remove reference");
    gui.apply();
    assert!(!gui
        .app
        .library_metadata
        .catalog
        .tracks
        .iter()
        .any(|track| track.id == id));
    assert!(!gui
        .app
        .library
        .iter()
        .any(|row| row.source == LibSource::File(copies.join("track0.wav"))));
    assert!(copies.join("track0.wav").exists());
    assert!(track_path(&baseline[0]).exists());
    assert!(gui.frames > 0);
    let saved = gui.app.library_metadata.catalog.tracks.clone();
    let frames = gui.frames;
    drop(gui);
    wait_closed(files.0.join("catalog.json"));
    let reopened = crate::library::read(&files.0.join("catalog.json")).unwrap();
    assert_eq!(reopened.tracks, saved);
    println!(
        "LIBRARY_FILES_NATIVE {}",
        serde_json::json!({"actual_egui_accesskit":true,"copy_move_restore_remove":true,"saved_preparation":true,"playing_pcm_unchanged":true,"rendered_frames":frames,"save_reopen":true,"unselected_sentinel_untouched":true,"os_gui_opened":false,"physical_devices_opened":false})
    );
}

#[test]
fn native_duplicate_review_keeps_chosen_preparation_and_stale_apply_cannot_change_meaning() {
    let files = Files::new();
    let baseline = seed(&files);
    let mut gui = Gui::new(&files);
    gui.select(&track_path(&baseline[0]));
    gui.open();
    gui.click("Capture selected music file");
    gui.settle();
    gui.click("Merge duplicates");
    gui.click("Find likely duplicates");
    gui.settle();
    assert_eq!(gui.app.library_files.duplicates.len(), 2);
    let duplicate = baseline[1].id.clone();
    let label = gui
        .nodes
        .iter()
        .filter_map(|(_, node)| node.label())
        .find(|label| label.starts_with("File 1 ·"))
        .unwrap()
        .to_string();
    gui.click(&label);
    gui.click("Use duplicate's current preparation");
    let unchanged = gui.app.library_metadata.catalog.tracks.clone();
    gui.click("Review music file operation");
    let stale = gui.node("Apply reviewed music file operation");
    gui.click("Use duplicate's current preparation");
    gui.frame(vec![egui::Event::AccessKitActionRequest(ActionRequest {
        target: stale,
        action: NativeAction::Click,
        data: None,
    })]);
    gui.settle();
    assert_eq!(gui.app.library_metadata.catalog.tracks, unchanged);
    assert!(gui.app.library_files.reviewed.is_none());
    gui.click("Use duplicate's current preparation");
    gui.apply();
    assert_eq!(
        gui.app.library_metadata.catalog.tracks.len(),
        unchanged.len() - 1
    );
    assert!(!gui
        .app
        .library_metadata
        .catalog
        .tracks
        .iter()
        .any(|track| track.id == duplicate));
    let retained = gui
        .app
        .library_metadata
        .catalog
        .tracks
        .iter()
        .find(|track| track.id == baseline[0].id)
        .unwrap();
    assert_eq!(retained.source, baseline[1].source);
    assert_eq!(
        retained.versions[retained.current].preparation,
        baseline[1].versions[baseline[1].current].preparation
    );
    assert!(retained.versions.iter().any(
        |version| version.preparation == baseline[0].versions[baseline[0].current].preparation
    ));
    for track in &baseline {
        assert!(track_path(track).exists());
    }
    let saved = gui.app.library_metadata.catalog.tracks.clone();
    drop(gui);
    wait_closed(files.0.join("catalog.json"));
    assert_eq!(
        crate::library::read(&files.0.join("catalog.json"))
            .unwrap()
            .tracks,
        saved
    );
    println!(
        "LIBRARY_FILES_NATIVE_DUPLICATE {}",
        serde_json::json!({"actual_egui_accesskit":true,"reviewed_pair":true,"chosen_preparation":true,"both_prepared_histories":true,"stale_apply_refused":true,"source_audio_kept":true,"save_reopen":true,"physical_devices_opened":false})
    );
}

#[test]
fn native_keep_moved_locations_then_new_batch_and_reopened_saved_move_restoration() {
    let files = Files::new();
    let before = seed(&files);
    let destination = files.0.join("kept");
    fs::create_dir(&destination).unwrap();
    let mut gui = Gui::new(&files);
    gui.select(&track_path(&before[0]));
    gui.open();
    gui.click("Capture selected music file");
    gui.settle();
    gui.click("Move with recovery");
    gui.text(
        "Music file destination folder",
        destination.to_str().unwrap(),
    );
    gui.apply();
    let id = gui.app.library_files.recovery.as_ref().unwrap().id.clone();
    gui.click("Keep reviewed moved music locations");
    gui.settle();
    assert!(gui.app.library_files.recovery.is_none());
    assert_eq!(gui.app.library_files.saved_moves.len(), 1);
    assert!(!track_path(&before[0]).exists());
    gui.select(&track_path(&before[1]));
    gui.click("Capture selected music file");
    gui.settle();
    gui.click("Copy");
    gui.apply();
    assert!(destination.join("track1.wav").exists());
    assert!(track_path(&before[1]).exists());
    drop(gui);
    wait_closed(files.0.join("catalog.json"));
    let mut gui = Gui::new(&files);
    gui.open();
    gui.click("Inspect file recovery");
    gui.settle();
    assert_eq!(gui.app.library_files.saved_moves.len(), 1);
    gui.click(&format!("Restore saved move {id}"));
    gui.settle();
    assert!(gui.app.library_files.saved_moves.is_empty());
    assert!(track_path(&before[0]).exists());
    assert!(!destination.join("track0.wav").exists());
    assert!(destination.join("track1.wav").exists());
    let restored = gui
        .app
        .library_metadata
        .catalog
        .tracks
        .iter()
        .find(|track| track.id == before[0].id)
        .unwrap();
    assert_eq!(restored.source, before[0].source);
    assert_eq!(
        restored.versions[restored.current].preparation,
        before[0].versions[before[0].current].preparation
    );
    println!(
        "LIBRARY_FILES_NATIVE_RETAINED {}",
        serde_json::json!({"actual_egui_accesskit":true,"keep_moved_locations":true,"next_batch":true,"app_reopened":true,"saved_move_restore":true,"unselected_copy_preserved":true,"physical_devices_opened":false})
    );
}
