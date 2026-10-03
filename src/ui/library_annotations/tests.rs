use super::*;
use crate::engine::RtEngine;
use egui::accesskit::{Action, ActionData, ActionRequest, Node, NodeId};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

pub(in crate::ui) struct Files(pub(in crate::ui) PathBuf);
impl Files {
    pub(in crate::ui) fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "omatainer-annotations-gui-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        for name in ["One.flac", "Two.flac", "Three.flac"] {
            std::fs::write(
                path.join(name),
                include_bytes!("../../../tests/fixtures/audio/tone.flac"),
            )
            .unwrap();
        }
        Self(path)
    }
}
impl Drop for Files {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
pub(in crate::ui) struct Gui {
    pub(in crate::ui) app: App,
    pub(in crate::ui) rt: RtEngine,
    ctx: egui::Context,
    nodes: Vec<(NodeId, Node)>,
    time: f64,
}
impl Gui {
    pub(in crate::ui) fn new(files: &Files) -> Self {
        let (engine, mut rt) = Engine::headless_for_test(48000, 256);
        rt.publish_for_test();
        let loader = Loader::start_with_performance(engine.cmd.performance().clone()).unwrap();
        let mut app = App::with_loader(engine, Theme::default(), Some(loader));
        app.library_metadata =
            library_metadata::Metadata::with_hook(files.0.join("catalog.json"), || {});
        app.library_metadata
            .set_performance(app.engine.cmd.performance().clone());
        app.library_initialized = false;
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut gui = Self {
            app,
            rt,
            ctx,
            nodes: vec![],
            time: 0.0,
        };
        gui.wait(|gui| !gui.app.library_metadata.active());
        gui.app
            .library_scan
            .start(vec![files.0.clone()], gui.app.library.clone());
        gui.wait(|gui| !gui.app.library_scan.active() && !gui.app.library_metadata.active());
        gui.click("annotations…");
        gui
    }
    pub(in crate::ui) fn frame(&mut self, events: Vec<egui::Event>) {
        self.time += 0.02;
        let out = self.ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1800.0, 1600.0))),
                time: Some(self.time),
                events,
                focused: true,
                ..Default::default()
            },
            |ctx| self.app.update_frame(ctx),
        );
        self.nodes = out.platform_output.accesskit_update.unwrap().nodes;
        self.rt.process(&mut [0.0; 256]);
        self.rt.publish_for_test();
    }
    pub(in crate::ui) fn wait(&mut self, mut condition: impl FnMut(&Self) -> bool) {
        let until = Instant::now() + Duration::from_secs(10);
        loop {
            self.frame(vec![]);
            if condition(self) {
                break;
            }
            assert!(
                Instant::now() < until,
                "{} {:?}",
                self.app.library_crates.message,
                self.app.library_metadata.storage_error
            );
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    pub(in crate::ui) fn node(&self, label: &str) -> NodeId {
        self.nodes
            .iter()
            .find(|(_, node)| node.label() == Some(label))
            .map(|(id, _)| *id)
            .unwrap_or_else(|| {
                panic!(
                    "missing {label}: {:?}",
                    self.nodes
                        .iter()
                        .filter_map(|(_, n)| n.label())
                        .collect::<Vec<_>>()
                )
            })
    }
    pub(in crate::ui) fn action(&mut self, label: &str, action: Action, data: Option<ActionData>) {
        let target = self.node(label);
        self.frame(vec![egui::Event::AccessKitActionRequest(ActionRequest {
            target,
            action,
            data,
        })]);
        self.frame(vec![]);
    }
    pub(in crate::ui) fn click(&mut self, label: &str) {
        self.action(label, Action::Click, None);
    }
    pub(in crate::ui) fn text(&mut self, label: &str, value: &str) {
        self.action(label, Action::Focus, None);
        self.frame(vec![
            egui::Event::Key {
                key: Key::A,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers {
                    ctrl: true,
                    command: true,
                    ..Default::default()
                },
            },
            egui::Event::Text(value.into()),
        ]);
        self.frame(vec![]);
    }
    pub(in crate::ui) fn finish(&mut self) {
        self.wait(|gui| {
            gui.app.library_crates.pending.is_none() && !gui.app.library_metadata.active()
        });
    }
}

#[test]
fn native_reviewed_batch_persists_and_filters_without_changing_audio_or_loaded_deck() {
    let files = Files::new();
    let before: Vec<_> = ["One.flac", "Two.flac", "Three.flac"]
        .into_iter()
        .map(|name| std::fs::read(files.0.join(name)).unwrap())
        .collect();
    let mut gui = Gui::new(&files);
    let loaded = gui.rt.decks[0].audio.clone();
    gui.click("Capture filtered annotation batch");
    let captured = gui.app.library_annotations.ids.len();
    assert!(captured >= 3);
    gui.click("Change rating");
    gui.action(
        "Rating (0 = unrated)",
        Action::SetValue,
        Some(ActionData::NumericValue(4.0)),
    );
    gui.click("Change Performance tags (one per line)");
    gui.text("Performance tags (one per line)", "clean\nrequest");
    gui.click("Change Performance notes");
    gui.text("Performance notes", "日本語 request\nDinner set");
    gui.click("Review annotation changes");
    assert!(gui.app.library_annotations.reviewed.is_some());
    gui.click("Apply reviewed annotations");
    gui.finish();
    assert!(gui.app.library_crates.message.contains("Annotations saved"));
    for track in &gui.app.library_metadata.catalog.tracks {
        assert_eq!(track.annotations.rating, 4);
        assert_eq!(track.annotations.tags, vec!["clean", "request"]);
    }
    assert!(match (&loaded, &gui.rt.decks[0].audio) {
        (Some(a), Some(b)) => Arc::ptr_eq(a, b),
        (None, None) => true,
        _ => false,
    });
    gui.app.library_annotations.open = false;
    gui.text("Search crate", "rating>=4 tag:clean note:Dinner");
    gui.app.refresh_library_view();
    assert_eq!(gui.app.library_view.indices.len(), captured);
    let saved = crate::library::read(&files.0.join("catalog.json")).unwrap();
    assert!(saved
        .tracks
        .iter()
        .all(|track| track.annotations.notes == "日本語 request\nDinner set"));
    for (index, name) in ["One.flac", "Two.flac", "Three.flac"]
        .into_iter()
        .enumerate()
    {
        assert_eq!(std::fs::read(files.0.join(name)).unwrap(), before[index]);
    }
}

#[test]
fn native_saved_annotation_rule_updates_membership_and_review_edits_do_not_auto_apply() {
    let files = Files::new();
    let mut gui = Gui::new(&files);
    gui.click("Capture selected annotations");
    gui.click("Change rating");
    gui.action(
        "Rating (0 = unrated)",
        Action::SetValue,
        Some(ActionData::NumericValue(5.0)),
    );
    gui.click("Review annotation changes");
    assert!(gui
        .app
        .library_metadata
        .catalog
        .tracks
        .iter()
        .all(|track| track.annotations.rating == 0));
    gui.click("Apply reviewed annotations");
    gui.finish();
    gui.app.library_annotations.open = false;
    gui.app.submit_crate_edit(
        gui.app.library_metadata.catalog.crates.revision(),
        CollectionAction::Create {
            name: "Top".into(),
            parent: None,
            before: None,
        },
    );
    gui.finish();
    let id = gui.app.library_crates.selected.clone().unwrap();
    gui.app.library_annotations.open = true;
    gui.frame(vec![]);
    gui.action(
        "Minimum crate rating",
        Action::SetValue,
        Some(ActionData::NumericValue(5.0)),
    );
    gui.click("Save annotation rule on selected crate");
    gui.finish();
    gui.app.refresh_library_view();
    assert_eq!(gui.app.library_view.indices.len(), 1);
    assert!(gui
        .app
        .library_metadata
        .catalog
        .crates
        .node(&id)
        .unwrap()
        .annotation_rule
        .is_some());
    gui.app.library_crates.selected = None;
    gui.frame(vec![]);
    gui.app.library_crates.selected = Some(id.clone());
    gui.frame(vec![]);
    assert_eq!(gui.app.library_annotations.rule.minimum_rating, 5);
    gui.click("Capture filtered annotation batch");
    gui.click("Change rating");
    gui.action(
        "Rating (0 = unrated)",
        Action::SetValue,
        Some(ActionData::NumericValue(0.0)),
    );
    gui.click("Review annotation changes");
    gui.click("Apply reviewed annotations");
    gui.finish();
    gui.app.refresh_library_view();
    assert!(gui.app.library_view.indices.is_empty());
    gui.app.engine.cmd.performance().set_enabled(true).unwrap();
    gui.frame(vec![]);
    gui.click("Capture selected annotations");
    assert!(gui.app.library_crates.pending.is_none());
}
