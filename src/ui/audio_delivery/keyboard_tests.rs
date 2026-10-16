use super::*;
use crate::engine::RtEngine;
use egui::accesskit::{Action, Invalid, Live, Node, NodeId, Role};
use std::time::Duration;

pub(in crate::ui) struct Gui {
    pub app: App,
    pub rt: Box<RtEngine>,
    pub ctx: egui::Context,
    pub root: PathBuf,
    nodes: Vec<(NodeId, Node)>,
    focus: NodeId,
    shapes: Vec<egui::epaint::ClippedShape>,
    time: f64,
    pub pause_renderer: bool,
}
impl Gui {
    pub fn new() -> Self {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join(".artifacts/keyboard-audio-fields")
            .join(crate::sampler_bank::BankId::new().unwrap().to_string());
        std::fs::create_dir_all(&root).unwrap();
        let (engine, rt) = Engine::headless_for_test(48000, 256);
        let mut app = App::with_loader(engine, Theme::default(), None);
        app.settings = preferences::Settings::with_worker_for_test(root.join("preferences.json"));
        app.library_metadata =
            library_metadata::Metadata::with_hook(root.join("library.json"), || {});
        app.library_metadata
            .set_performance(app.engine.cmd.performance().clone());
        app.library_initialized = true;
        app.plugins
            .set_performance(app.engine.cmd.performance().clone());
        app.plugins.initialize(root.join("plugins.json"));
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut gui = Self {
            app,
            rt: Box::new(rt),
            ctx,
            root,
            nodes: vec![],
            focus: NodeId::from(0_u64),
            shapes: vec![],
            time: 0.0,
            pause_renderer: false,
        };
        gui.until(|gui| !gui.app.plugins.busy());
        gui
    }
    pub fn frame(&mut self, events: Vec<egui::Event>, modifiers: egui::Modifiers) {
        self.rt.publish_for_test();
        self.time += 0.02;
        let output = self.ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(2200.0, 2400.0))),
                time: Some(self.time),
                focused: true,
                modifiers,
                events,
                ..Default::default()
            },
            |ctx| self.app.update_frame(ctx),
        );
        let tree = output.platform_output.accesskit_update.unwrap();
        self.nodes = tree.nodes;
        self.focus = tree.focus;
        self.shapes = output.shapes;
        if !self.pause_renderer {
            self.rt.process(&mut [0.0; 128]);
        }
        self.rt.publish_for_test();
    }
    pub fn idle(&mut self) {
        self.frame(vec![], egui::Modifiers::NONE);
    }
    pub fn key(&mut self, key: Key, modifiers: egui::Modifiers) {
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
    pub fn node(&self, name: &str) -> (NodeId, &Node) {
        self.nodes
            .iter()
            .find(|(_, node)| {
                node.label() == Some(name)
                    && node.role() != Role::Window
                    && (node.supports_action(Action::Focus)
                        || node.is_disabled() && node.role() != Role::Label)
            })
            .map(|(id, node)| (*id, node))
            .unwrap_or_else(|| {
                panic!(
                    "Missing native widget {name}; names {:?}",
                    self.nodes
                        .iter()
                        .filter_map(|(_, node)| node.label())
                        .collect::<Vec<_>>()
                )
            })
    }
    #[track_caller]
    pub fn focused(&self, name: &str) {
        let id = self.node(name).0;
        assert_eq!(
            self.ctx
                .memory(|memory| memory.focused())
                .map(|id| NodeId::from(id.value())),
            Some(id),
            "native Memory focus for {name}"
        );
        assert_eq!(self.focus, id, "native AccessKit focus for {name}");
    }
    #[track_caller]
    pub fn painted_focus(&self, name: &str) {
        self.focused(name);
        let response = self
            .ctx
            .read_response(self.ctx.memory(|memory| memory.focused()).unwrap())
            .unwrap();
        assert!(self.shapes.iter().any(|shape| matches!(&shape.shape, egui::epaint::Shape::Rect(rect) if rect.stroke.width == 2.0 && rect.rect == response.rect.expand(2.0))), "actual visible focus for {name}; native response {response:?}; actual 2px paint {:?}", self.shapes.iter().filter_map(|shape| match &shape.shape { egui::epaint::Shape::Rect(rect) if rect.stroke.width == 2.0 => Some((shape.clip_rect, rect)), _ => None }).collect::<Vec<_>>());
    }
    #[track_caller]
    pub fn tab_to(&mut self, name: &str) {
        for _ in 0..700 {
            let id = self.node(name).0;
            if self
                .ctx
                .memory(|memory| memory.focused())
                .map(|id| NodeId::from(id.value()))
                == Some(id)
            {
                self.focused(name);
                return;
            }
            self.key(Key::Tab, egui::Modifiers::NONE);
        }
        panic!("Tab did not reach {name}");
    }
    pub fn described_node(&self, name: &str, description: &str) -> NodeId {
        let labels = self
            .nodes
            .iter()
            .filter(|(_, node)| {
                node.label() == Some(description) || node.value() == Some(description)
            })
            .map(|(id, _)| *id)
            .collect::<Vec<_>>();
        self.nodes
            .iter()
            .find(|(_, node)| {
                node.label() == Some(name)
                    && node.described_by().iter().any(|id| labels.contains(id))
            })
            .map(|(id, _)| *id)
            .unwrap_or_else(|| panic!("Missing {name} described by {description}"))
    }
    pub fn tab_to_node(&mut self, target: NodeId) {
        for _ in 0..700 {
            if self
                .ctx
                .memory(|memory| memory.focused())
                .map(|id| NodeId::from(id.value()))
                == Some(target)
            {
                assert_eq!(self.focus, target, "actual native tree focus");
                return;
            }
            self.key(Key::Tab, egui::Modifiers::NONE);
        }
        panic!("Tab did not reach native {target:?}");
    }
    pub fn press(&mut self, name: &str) {
        self.tab_to(name);
        self.key(Key::Enter, egui::Modifiers::NONE);
    }
    pub fn toggle(&mut self, name: &str) {
        self.tab_to(name);
        self.key(Key::Space, egui::Modifiers::NONE);
    }
    #[track_caller]
    fn export_format(&mut self, format: Format) {
        self.tab_to("Export format");
        self.focused("Export format");
        let opener = self.ctx.memory(|memory| memory.focused()).unwrap();
        assert!(!egui::ComboBox::is_open(&self.ctx, opener));
        self.key(Key::Space, egui::Modifiers::NONE);
        assert!(egui::ComboBox::is_open(&self.ctx, opener));
        self.tab_to(format.title());
        self.painted_focus(format.title());
        self.key(Key::Enter, egui::Modifiers::NONE);
        assert_eq!(self.app.audio_delivery.request.options.format, format);
        assert!(!egui::ComboBox::is_open(&self.ctx, opener));
        self.painted_focus("Export format");
    }
    pub fn text(&mut self, name: &str, value: &str) {
        self.tab_to(name);
        self.key(Key::A, egui::Modifiers::CTRL | egui::Modifiers::COMMAND);
        self.frame(vec![egui::Event::Text(value.into())], egui::Modifiers::NONE);
        self.idle();
    }
    pub fn number(&mut self, name: &str, value: &str) {
        self.text(name, value);
        self.key(Key::Enter, egui::Modifiers::NONE);
    }
    #[track_caller]
    pub fn until(&mut self, mut ready: impl FnMut(&Self) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(15);
        while !ready(self) {
            self.idle();
            assert!(
                Instant::now() < deadline,
                "native worker did not settle: {}",
                self.app.audio_delivery.message
            );
            std::thread::sleep(Duration::from_millis(1));
        }
        self.idle();
        self.idle();
    }
    pub fn menu(&mut self, menu: &str, item: &str) {
        self.toggle(menu);
        self.press(item);
    }
    pub fn delivery(&mut self) {
        let key = command_palette::chord(self.app.settings.profile()).unwrap();
        self.key(key, egui::Modifiers::CTRL | egui::Modifiers::SHIFT);
        self.focused("Search commands");
        self.frame(
            vec![egui::Event::Text("Open audio export and recording".into())],
            egui::Modifiers::NONE,
        );
        self.key(Key::Enter, egui::Modifiers::NONE);
        assert!(
            self.app.audio_delivery.open,
            "ordinary palette delivery route"
        );
    }
    pub fn plugins(&mut self) {
        self.menu("Tools", "Preferences");
        self.press("VST3 plugins");
        assert!(self.app.plugins.open, "ordinary preferences plugin route");
    }
    pub fn associated_error(&self, name: &str) {
        let field = self.node(name).1;
        assert_eq!(field.invalid(), Some(Invalid::True));
        let error = field.error_message().unwrap();
        assert!(field.described_by().contains(&error));
        let node = &self.nodes.iter().find(|(id, _)| *id == error).unwrap().1;
        assert_eq!(node.role(), Role::Alert);
        assert_eq!(node.live(), Some(Live::Polite));
        assert_eq!(
            self.nodes
                .iter()
                .filter(|(_, candidate)| candidate.label() == node.label())
                .count(),
            1,
            "one visible named error"
        );
    }
}
impl Drop for Gui {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn keyboard_access_delivery_rows_native_keys_validation_conditional_ids_and_fresh_reopen() {
    let mut gui = Gui::new();
    gui.delivery();
    gui.press("Review export source");
    gui.until(|gui| gui.app.audio_delivery.job.is_none() && gui.app.audio_delivery.plan.is_some());
    let revision = gui.app.engine.project.revision();
    let range_id = gui.node("Range start (seconds)").0;
    gui.press("Current session launches");
    assert_eq!(
        gui.node("Range start (seconds)").0,
        range_id,
        "hiding the scene chooser preserves the actual native number"
    );
    gui.press("Selected scene");
    assert_eq!(
        gui.node("Range start (seconds)").0,
        range_id,
        "scene chooser never steals the native range identity"
    );
    gui.export_format(Format::Pcm16);
    assert_eq!(gui.app.audio_delivery.request.options.format, Format::Pcm16);
    gui.toggle("Triangular dither for integer quantization");
    assert!(gui.app.audio_delivery.request.options.dither);
    gui.export_format(Format::Float32);
    assert!(!gui.app.audio_delivery.request.options.dither);
    assert!(gui
        .node("Triangular dither for integer quantization")
        .1
        .is_disabled());
    gui.text("Range start (seconds)", "NaN");
    gui.associated_error("Range start (seconds)");
    assert!(gui.node("Render master audio").1.is_disabled());
    gui.key(Key::Enter, egui::Modifiers::NONE);
    assert!(gui.app.audio_delivery.job.is_none());
    gui.tab_to("Range start (seconds)");
    gui.key(Key::Escape, egui::Modifiers::NONE);
    gui.number("Range start (seconds)", "0.25");
    gui.tab_to("Range start (seconds)");
    gui.key(Key::ArrowDown, egui::Modifiers::NONE);
    gui.painted_focus("Range start (seconds)");
    assert!((gui.app.audio_delivery.request.start - 0.15).abs() < 1e-9);
    gui.key(Key::Tab, egui::Modifiers::NONE);
    gui.focused("Range end (seconds)");
    gui.key(Key::Tab, egui::Modifiers::SHIFT);
    gui.focused("Range start (seconds)");
    gui.press("Delivery metadata & final-file QC");
    let peak_id = gui.node("Maximum true peak (dBTP)").0;
    gui.toggle("Check integrated loudness target");
    assert_eq!(
        gui.node("Maximum true peak (dBTP)").0,
        peak_id,
        "optional preceding target rows preserve native peak identity"
    );
    gui.tab_to("Maximum true peak (dBTP)");
    gui.key(Key::F2, egui::Modifiers::NONE);
    gui.focused("Maximum true peak (dBTP)");
    assert_eq!(
        gui.node("Maximum true peak (dBTP)").1.role(),
        Role::SpinButton,
        "native DragValue retains its own AccessKit role"
    );
    assert!(
        keyboard::text_is_focused(&gui.ctx),
        "the actual native TextEdit owns keys after Tab/F2"
    );
    gui.number("Maximum true peak (dBTP)", "-2.5");
    assert_eq!(gui.app.audio_delivery.config.targets.max_true_peak, -2.5);
    gui.text("ISRC", "bad");
    gui.associated_error("ISRC");
    assert!(gui.node("Render master audio").1.is_disabled());
    gui.text("ISRC", "USAAA2600001");
    gui.text("Title", "Keyboard delivery");
    gui.press("Delivery metadata & final-file QC");
    gui.press("Aligned stems & versions");
    gui.text("New pass name", "keyboard-version");
    gui.press("Version");
    gui.toggle("Return · Reverb");
    let next = gui.app.audio_delivery.batch.as_ref().unwrap().next_id;
    gui.press("Add selected contribution pass");
    let pass = gui
        .app
        .audio_delivery
        .batch
        .as_ref()
        .unwrap()
        .items
        .iter()
        .find(|item| item.id == next)
        .unwrap();
    assert_eq!(pass.name, "keyboard-version");
    assert_eq!(pass.kind, data::batch::Kind::Version);
    assert_eq!(
        pass.selection.targets,
        vec![crate::engine::audio::routing::selection::Target::FixedReturn(0)]
    );
    let name = format!("Pass {next} name");
    let pass_id = gui.node(&name).0;
    gui.text(&name, "keyboard-renamed");
    let first = gui.app.audio_delivery.batch.as_ref().unwrap().items[0].id;
    gui.press(&format!("Remove pass {first}"));
    assert_eq!(
        gui.node(&name).0,
        pass_id,
        "removing a preceding pass preserves saved pass identity"
    );
    let recipe_path = gui.root.join("keyboard-recipe.json");
    gui.text("Batch recipe JSON path", recipe_path.to_str().unwrap());
    gui.press("Save batch recipe to new file");
    gui.until(|gui| gui.app.audio_delivery.job.is_none() && recipe_path.exists());
    let saved = data::batch::Recipe::load(&recipe_path, &AtomicBool::new(false)).unwrap();
    assert_eq!(saved.config.metadata.title, "Keyboard delivery");
    assert_eq!(saved.config.metadata.isrc, "USAAA2600001");
    assert_eq!(
        saved
            .items
            .iter()
            .find(|item| item.id == next)
            .unwrap()
            .name,
        "keyboard-renamed"
    );
    gui.text("Maximum recording seconds", "NaN");
    gui.associated_error("Maximum recording seconds");
    assert!(gui.node("Start performance recording").1.is_disabled());
    gui.key(Key::Enter, egui::Modifiers::NONE);
    assert!(gui.app.audio_delivery.job.is_none());
    gui.tab_to("Maximum recording seconds");
    gui.key(Key::Escape, egui::Modifiers::NONE);
    gui.number("Maximum recording seconds", "60");
    assert_eq!(gui.app.audio_delivery.record_seconds, 60);
    gui.text("Range start (seconds)", "-");
    gui.associated_error("Range start (seconds)");
    gui.press("Close audio export & recording");
    assert!(!gui.app.audio_delivery.open);
    gui.delivery();
    assert_ne!(
        gui.node("Range start (seconds)").0,
        range_id,
        "reopened dialog has a fresh native draft owner"
    );
    assert!(gui.node("Range start (seconds)").1.invalid().is_none());
    gui.number("Range start (seconds)", "0.5");
    assert_eq!(gui.app.audio_delivery.request.start, 0.5);
    gui.press("Aligned stems & versions");
    gui.press("Reopen batch recipe");
    gui.until(|gui| {
        gui.app.audio_delivery.job.is_none()
            && gui.app.audio_delivery.request.start == saved.export.start
    });
    assert_eq!(
        gui.app.engine.project.revision(),
        revision,
        "delivery draft/recipe work leaves artwork history untouched"
    );
    assert!(!gui.rt.playing);
}

#[test]
fn keyboard_access_delivery_metadata_rows_associate_existing_validation_and_recover() {
    let mut gui = Gui::new();
    gui.delivery();
    gui.press("Review export source");
    gui.until(|gui| gui.app.audio_delivery.job.is_none() && gui.app.audio_delivery.plan.is_some());
    gui.press("Delivery metadata & final-file QC");
    let revision = gui.app.engine.project.revision();
    let oversized = "x".repeat(1025);
    for (name, invalid, corrected) in [
        ("Title", oversized.as_str(), "Native delivery"),
        ("Artist", oversized.as_str(), "Keyboard artist"),
        ("ISRC", "bad", "USAAA2600001"),
        ("Description", oversized.as_str(), "Native metadata"),
        ("BWF originator", oversized.as_str(), "Omatainer"),
        (
            "BWF originator reference",
            oversized.as_str(),
            "keyboard-reference",
        ),
        ("BWF date YYYY-MM-DD", oversized.as_str(), "2026-10-15"),
        ("BWF time HH:MM:SS", oversized.as_str(), "12:34:56"),
    ] {
        let id = gui.node(name).0;
        gui.text(name, invalid);
        gui.painted_focus(name);
        gui.associated_error(name);
        for other in [
            "Title",
            "Artist",
            "ISRC",
            "Description",
            "BWF originator",
            "BWF originator reference",
            "BWF date YYYY-MM-DD",
            "BWF time HH:MM:SS",
        ] {
            if other != name {
                assert!(
                    gui.node(other).1.invalid().is_none(),
                    "{name} validation cannot mark the {other} native row"
                );
            }
        }
        assert!(gui.node("Render master audio").1.is_disabled());
        gui.key(Key::Enter, egui::Modifiers::NONE);
        assert!(gui.app.audio_delivery.job.is_none());
        gui.text(name, corrected);
        gui.painted_focus(name);
        assert_eq!(gui.node(name).0, id, "correction retains the native owner");
        assert!(gui.node(name).1.invalid().is_none());
        assert!(gui.node(name).1.error_message().is_none());
        assert!(!gui.node("Render master audio").1.is_disabled());
    }
    let metadata = &gui.app.audio_delivery.config.metadata;
    assert_eq!(metadata.title, "Native delivery");
    assert_eq!(metadata.artist, "Keyboard artist");
    assert_eq!(metadata.isrc, "USAAA2600001");
    assert_eq!(metadata.description, "Native metadata");
    assert_eq!(metadata.originator, "Omatainer");
    assert_eq!(metadata.originator_reference, "keyboard-reference");
    assert_eq!(metadata.date, "2026-10-15");
    assert_eq!(metadata.time, "12:34:56");
    assert_eq!(gui.app.engine.project.revision(), revision);
    assert!(!gui.rt.playing);
}

#[test]
fn keyboard_access_delivery_broadcast_metadata_native_errors_follow_format_and_recover() {
    let mut gui = Gui::new();
    gui.delivery();
    gui.press("Review export source");
    gui.until(|gui| gui.app.audio_delivery.job.is_none() && gui.app.audio_delivery.plan.is_some());
    gui.export_format(Format::Bwf24);
    gui.press("Delivery metadata & final-file QC");
    gui.until(|gui| {
        gui.nodes.iter().any(|(_, node)| {
            node.label() == Some("BWF date YYYY-MM-DD") && node.supports_action(Action::Focus)
        })
    });
    let revision = gui.app.engine.project.revision();
    let originator_id = gui.node("BWF originator").0;
    let date_id = gui.node("BWF date YYYY-MM-DD").0;
    for (name, malformed, corrected, message) in [
        (
            "Description",
            "d".repeat(257),
            "d".repeat(256),
            "Broadcast metadata requires ASCII text fitting256 bytes",
        ),
        (
            "BWF originator",
            "o".repeat(33),
            "o".repeat(32),
            "Broadcast metadata requires ASCII text fitting32 bytes",
        ),
        (
            "BWF originator reference",
            "r".repeat(33),
            "r".repeat(32),
            "Broadcast metadata requires ASCII text fitting32 bytes",
        ),
        (
            "BWF date YYYY-MM-DD",
            "2026/10/15".into(),
            "2026-10-15".into(),
            "BWF date/time require YYYY-MM-DD and HH:MM:SS",
        ),
        (
            "BWF time HH:MM:SS",
            "12-34-56".into(),
            "12:34:56".into(),
            "BWF date/time require YYYY-MM-DD and HH:MM:SS",
        ),
    ] {
        let id = gui.node(name).0;
        gui.text(name, &malformed);
        gui.painted_focus(name);
        gui.associated_error(name);
        let error = gui.node(name).1.error_message().unwrap();
        assert_eq!(
            gui.nodes
                .iter()
                .find(|(id, _)| *id == error)
                .unwrap()
                .1
                .label(),
            Some(message),
            "the actual row uses the existing container validator's error"
        );
        assert!(gui.node("Render master audio").1.is_disabled());
        gui.key(Key::Enter, egui::Modifiers::NONE);
        assert!(gui.app.audio_delivery.job.is_none());
        gui.text(name, &corrected);
        gui.painted_focus(name);
        assert_eq!(gui.node(name).0, id);
        assert!(gui.node(name).1.invalid().is_none());
        assert!(gui.node(name).1.error_message().is_none());
        assert!(!gui.node("Render master audio").1.is_disabled());
    }
    gui.text("BWF originator", &"o".repeat(33));
    gui.text("BWF date YYYY-MM-DD", "2026/10/15");
    gui.press("Delivery metadata & final-file QC");
    gui.until(|gui| {
        gui.node("Delivery metadata & final-file QC").1.invalid() == Some(Invalid::True)
    });
    gui.associated_error("Delivery metadata & final-file QC");
    for format in [
        Format::Float32,
        Format::Pcm16,
        Format::Pcm24,
        Format::Flac16,
        Format::Flac24,
        Format::Mp3,
        Format::Aiff16,
        Format::Aiff24,
    ] {
        gui.export_format(format);
        assert_eq!(gui.app.audio_delivery.request.options.format, format);
        assert!(gui
            .node("Delivery metadata & final-file QC")
            .1
            .invalid()
            .is_none());
        assert!(!gui.node("Render master audio").1.is_disabled());
    }
    for format in [Format::Bwf24, Format::Rf64Float32, Format::Rf64Pcm24] {
        gui.export_format(format);
        gui.associated_error("Delivery metadata & final-file QC");
        assert!(gui.node("Render master audio").1.is_disabled());
    }
    gui.press("Delivery metadata & final-file QC");
    assert_eq!(gui.node("BWF originator").0, originator_id);
    assert_eq!(gui.node("BWF date YYYY-MM-DD").0, date_id);
    gui.associated_error("BWF originator");
    gui.associated_error("BWF date YYYY-MM-DD");
    gui.text("BWF originator", "Omatainer");
    gui.text("BWF date YYYY-MM-DD", "2026-10-15");
    assert!(!gui.node("Render master audio").1.is_disabled());
    gui.text("Maximum true peak (dBTP)", "-");
    gui.associated_error("Maximum true peak (dBTP)");
    gui.press("Delivery metadata & final-file QC");
    gui.until(|gui| {
        gui.node("Delivery metadata & final-file QC").1.invalid() == Some(Invalid::True)
    });
    gui.export_format(Format::Float32);
    gui.associated_error("Delivery metadata & final-file QC");
    assert!(gui.node("Render master audio").1.is_disabled());
    gui.press("Delivery metadata & final-file QC");
    gui.tab_to("Maximum true peak (dBTP)");
    gui.key(Key::Escape, egui::Modifiers::NONE);
    gui.number("Maximum true peak (dBTP)", "-1.5");
    assert!(!gui.node("Render master audio").1.is_disabled());
    assert_eq!(
        gui.app.audio_delivery.config.metadata.originator,
        "Omatainer"
    );
    assert_eq!(gui.app.audio_delivery.config.metadata.date, "2026-10-15");
    assert_eq!(gui.app.engine.project.revision(), revision);
    assert!(gui.app.audio_delivery.job.is_none());
    assert!(!gui.rt.playing);
}

#[test]
fn keyboard_access_delivery_busy_and_stale_review_preserve_worker_admission() {
    let mut gui = Gui::new();
    gui.delivery();
    gui.press("Review export source");
    gui.until(|gui| gui.app.audio_delivery.job.is_none() && gui.app.audio_delivery.plan.is_some());
    gui.tab_to("Range start (seconds)");
    let before = gui.app.audio_delivery.request.start;
    gui.pause_renderer = true;
    gui.app.start_audio_delivery(Operation::Inspect);
    gui.idle();
    assert!(
        gui.app.audio_delivery.job.is_some(),
        "real capture waits for renderer admission"
    );
    assert!(gui.node("Range start (seconds)").1.is_disabled());
    gui.frame(vec![egui::Event::Text("99".into())], egui::Modifiers::NONE);
    gui.key(Key::ArrowUp, egui::Modifiers::NONE);
    assert_eq!(gui.app.audio_delivery.request.start, before);
    gui.press("Cancel audio work");
    gui.pause_renderer = false;
    gui.until(|gui| gui.app.audio_delivery.job.is_none());
    gui.press("Review export source");
    gui.until(|gui| gui.app.audio_delivery.job.is_none());
    let namespace = gui.app.audio_delivery.plan.as_ref().unwrap().namespace;
    let destination = gui.root.join("must-not-publish");
    gui.text("New export folder", destination.to_str().unwrap());
    gui.menu("Project", "New project");
    gui.until(|gui| !gui.app.project_pending_for_test() && gui.rt.session.namespace != namespace);
    gui.press("Render master audio");
    gui.until(|gui| gui.app.audio_delivery.job.is_none());
    assert!(
        gui.app
            .audio_delivery
            .message
            .contains("changed since export review"),
        "{}",
        gui.app.audio_delivery.message
    );
    assert!(
        !destination.exists(),
        "stale source never reaches encoded output or publication"
    );
    assert!(!gui.rt.playing);
}

#[test]
fn keyboard_access_delivery_native_configs_and_typed_roundtrip_survive_render_focus_blur() {
    let mut gui = Gui::new();
    gui.delivery();
    gui.press("Review export source");
    gui.until(|gui| gui.app.audio_delivery.job.is_none() && gui.app.audio_delivery.plan.is_some());
    let repeats = gui.app.audio_delivery.request.repeats;
    gui.tab_to("Repeat range (times)");
    assert_eq!(
        gui.node("Repeat range (times)").1.numeric_value_step(),
        Some(0.25)
    );
    gui.key(Key::ArrowUp, egui::Modifiers::NONE);
    gui.painted_focus("Repeat range (times)");
    assert_eq!(
        gui.app.audio_delivery.request.repeats, repeats,
        "native .25 integral step rounds to the original integer; it must not become a forced +1"
    );
    let seconds = gui.app.audio_delivery.record_seconds;
    gui.tab_to("Maximum recording seconds");
    assert_eq!(
        gui.node("Maximum recording seconds").1.numeric_value_step(),
        Some(0.25)
    );
    gui.key(Key::ArrowUp, egui::Modifiers::NONE);
    gui.focused("Maximum recording seconds");
    assert_eq!(gui.app.audio_delivery.record_seconds, seconds);
    let rate = gui.app.audio_delivery.request.options.rate;
    gui.tab_to("Sample rate (Hz)");
    assert_eq!(
        gui.node("Sample rate (Hz)").1.numeric_value_step(),
        Some(100.0)
    );
    gui.key(Key::ArrowUp, egui::Modifiers::NONE);
    gui.painted_focus("Sample rate (Hz)");
    assert_eq!(
        gui.app.audio_delivery.request.options.rate,
        rate + 100,
        "explicit native sample-rate step remains 100 Hz"
    );
    gui.number("Sample rate (Hz)", &rate.to_string());

    let start = 0.12345678901234567_f64;
    let peak = -1.2345678901234567_f64;
    gui.number("Range start (seconds)", &start.to_string());
    gui.press("Delivery metadata & final-file QC");
    gui.number("Maximum true peak (dBTP)", &peak.to_string());
    let reference = 9_007_199_254_740_993_u64;
    let reference_text = reference.to_string();
    gui.app.audio_delivery.config.metadata.time_reference = reference;
    gui.idle();
    assert_eq!(
        gui.node("BWF source-origin sample reference (samples)")
            .1
            .numeric_value_step(),
        Some(0.25)
    );
    for _ in 0..3 {
        for name in [
            "Range start (seconds)",
            "Maximum true peak (dBTP)",
            "BWF source-origin sample reference (samples)",
        ] {
            for _ in 0..3 {
                gui.idle();
            }
            gui.tab_to(name);
            gui.painted_focus(name);
            gui.key(Key::F2, egui::Modifiers::NONE);
            gui.focused(name);
            gui.key(Key::Tab, egui::Modifiers::NONE);
            gui.idle();
            assert_eq!(
                gui.app.audio_delivery.request.start.to_bits(),
                start.to_bits(),
                "render/focus/blur keeps the authored f64 range"
            );
            assert_eq!(
                gui.app
                    .audio_delivery
                    .config
                    .targets
                    .max_true_peak
                    .to_bits(),
                peak.to_bits(),
                "render/focus/blur keeps the authored f64 QC target"
            );
            assert_eq!(
                gui.app.audio_delivery.config.metadata.time_reference, reference,
                "native presentation cannot narrow a loaded u64 through f64"
            );
            assert_eq!(
                gui.node("Range start (seconds)")
                    .1
                    .numeric_value()
                    .unwrap()
                    .to_bits(),
                start.to_bits(),
                "native numeric metadata keeps f64 precision"
            );
            assert_eq!(
                gui.node("Maximum true peak (dBTP)")
                    .1
                    .numeric_value()
                    .unwrap()
                    .to_bits(),
                peak.to_bits(),
                "native QC metadata keeps f64 precision"
            );
            assert_eq!(
                gui.node("BWF source-origin sample reference (samples)")
                    .1
                    .value(),
                Some(reference_text.as_str()),
                "the native text represents the original integer exactly"
            );
        }
    }
    gui.number("Range start (seconds)", "− 1");
    assert_eq!(
        gui.app.audio_delivery.request.start, 0.0,
        "accepted native edits retain their original range clamp"
    );
    assert!(gui.app.audio_delivery.job.is_none());
    assert!(!gui.rt.playing);
}
