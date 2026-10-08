use super::*;
use crate::engine::{dsp::InputKey, RtEngine, SamplerInstrument, SynthInstrument};
use egui::accesskit::{Action, ActionRequest, Node, NodeId};
use std::{cell::RefCell, rc::Rc};

#[derive(Clone)]
struct Window {
    events: Vec<egui::Event>,
    size: Vec2,
    monitor: Option<Vec2>,
    ppp: f32,
    focused: bool,
    close: bool,
    nodes: Vec<(NodeId, Node)>,
    builder: egui::ViewportBuilder,
    commands: Vec<egui::ViewportCommand>,
}
impl Default for Window {
    fn default() -> Self {
        Self {
            events: vec![],
            size: egui::vec2(960.0, 720.0),
            monitor: Some(egui::vec2(1920.0, 1080.0)),
            ppp: 1.0,
            focused: true,
            close: false,
            nodes: vec![],
            builder: Default::default(),
            commands: vec![],
        }
    }
}
#[derive(Default)]
struct Native {
    windows: BTreeMap<Panel, Window>,
    time: f64,
}
struct Gui {
    app: App,
    rt: Box<RtEngine>,
    ctx: egui::Context,
    native: Rc<RefCell<Native>>,
    time: f64,
    root_focused: bool,
    root_nodes: Vec<(NodeId, Node)>,
}
fn window_id(panel: Panel) -> egui::ViewportId {
    egui::ViewportId::from_hash_of(("workspace-window", panel))
}
impl Gui {
    fn new(panels: &[Panel]) -> Self {
        let (engine, mut rt) = Engine::headless_for_test(48_000, 256);
        rt.apply(Command::SamplerInst(SamplerInstrument::Synth(
            SynthInstrument::Keys,
        )));
        rt.publish_for_test();
        let mut app = App::with_loader(engine, Theme::default(), None);
        let config = &mut app
            .settings
            .applied
            .profiles
            .get_mut("Studio")
            .unwrap()
            .workspaces;
        for entry in &mut config.saved.get_mut("Production").unwrap().panels {
            entry.visible = panels.contains(&entry.panel);
            entry.detached = entry.visible;
        }
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        ctx.set_embed_viewports(false);
        let native = Rc::new(RefCell::new(Native::default()));
        let state = native.clone();
        egui::Context::set_immediate_viewport_renderer(move |ctx, mut viewport| {
            let panel = Panel::ALL
                .into_iter()
                .find(|panel| window_id(*panel) == viewport.ids.this)
                .unwrap();
            let (window, time) = {
                let mut state = state.borrow_mut();
                let time = state.time;
                let window = state.windows.entry(panel).or_default();
                let out = window.clone();
                window.events.clear();
                window.close = false;
                (out, time)
            };
            let mut raw = egui::RawInput {
                viewport_id: viewport.ids.this,
                focused: window.focused,
                events: window.events,
                time: Some(time),
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, window.size)),
                ..Default::default()
            };
            raw.viewports.insert(
                viewport.ids.this,
                egui::ViewportInfo {
                    parent: Some(egui::ViewportId::ROOT),
                    native_pixels_per_point: Some(window.ppp),
                    monitor_size: window.monitor,
                    focused: Some(window.focused),
                    events: if window.close {
                        vec![egui::ViewportEvent::Close]
                    } else {
                        vec![]
                    },
                    ..Default::default()
                },
            );
            let output = ctx.run(raw, |ctx| (viewport.viewport_ui_cb)(ctx));
            let mut state = state.borrow_mut();
            let window = state.windows.get_mut(&panel).unwrap();
            window.nodes = output.platform_output.accesskit_update.unwrap().nodes;
            window.builder = viewport.builder;
            window.commands.extend(
                output
                    .viewport_output
                    .get(&viewport.ids.this)
                    .unwrap()
                    .commands
                    .clone(),
            );
        });
        let mut gui = Self {
            app,
            rt: Box::new(rt),
            ctx,
            native,
            time: 0.0,
            root_focused: false,
            root_nodes: vec![],
        };
        gui.frame(vec![]);
        gui.frame(vec![]);
        gui
    }
    fn frame(&mut self, events: Vec<egui::Event>) {
        self.time += 0.02;
        {
            let mut native = self.native.borrow_mut();
            native.time = self.time;
            for window in native.windows.values_mut() {
                window.commands.clear();
            }
        }
        let mut raw = egui::RawInput {
            focused: self.root_focused,
            events,
            time: Some(self.time),
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1600.0, 1200.0))),
            ..Default::default()
        };
        raw.viewports
            .get_mut(&egui::ViewportId::ROOT)
            .unwrap()
            .monitor_size = Some(egui::vec2(1600.0, 1200.0));
        for (panel, window) in &self.native.borrow().windows {
            raw.viewports.insert(
                window_id(*panel),
                egui::ViewportInfo {
                    parent: Some(egui::ViewportId::ROOT),
                    focused: Some(window.focused),
                    ..Default::default()
                },
            );
        }
        let output = self.ctx.run(raw, |ctx| self.app.update_frame(ctx));
        self.root_nodes = output.platform_output.accesskit_update.unwrap().nodes;
        for (id, viewport) in output.viewport_output {
            if let Some(panel) = Panel::ALL.into_iter().find(|panel| window_id(*panel) == id) {
                self.native
                    .borrow_mut()
                    .windows
                    .get_mut(&panel)
                    .unwrap()
                    .commands
                    .extend(viewport.commands);
            }
        }
        self.rt.process(&mut [0.0; 128]);
        self.rt.publish_for_test();
    }
    fn input(&mut self, panel: Panel, events: Vec<egui::Event>) {
        self.native
            .borrow_mut()
            .windows
            .entry(panel)
            .or_default()
            .events = events;
        self.frame(vec![]);
    }
    fn click(&mut self, panel: Panel, label: &str) {
        let target = self.native.borrow().windows[&panel]
            .nodes
            .iter()
            .find(|(_, node)| node.label() == Some(label))
            .unwrap_or_else(|| panic!("missing {label}"))
            .0;
        self.input(
            panel,
            vec![egui::Event::AccessKitActionRequest(ActionRequest {
                target,
                action: Action::Click,
                data: None,
            })],
        );
        self.frame(vec![]);
    }
    fn pad(&self, pad: u8) -> Pos2 {
        touch::target_rect(
            &self.ctx,
            window_id(Panel::Sampler),
            touch::Target::Pad(pad),
        )
        .unwrap()
        .center()
    }
    fn held(&self, pad: u8) -> bool {
        self.rt.sampler_poly.voices.iter().any(|voice| {
            voice.input == Some(InputKey::Pad(pad)) && matches!(voice.env.stage, 1..=3)
        })
    }
    fn touch(&mut self, pad: u8, phase: egui::TouchPhase) {
        let pos = self.pad(pad);
        self.input(
            Panel::Sampler,
            vec![egui::Event::Touch {
                device_id: egui::TouchDeviceId(1),
                id: egui::TouchId(1),
                phase,
                pos,
                force: Some(0.3),
            }],
        );
    }
}

#[test]
fn native_secondary_sampler_releases_on_focus_dpi_monitor_change_and_close_without_resuming() {
    let mut gui = Gui::new(&[Panel::Sampler]);
    for mode in 0..3 {
        gui.touch(0, egui::TouchPhase::Start);
        assert!(gui.held(0));
        {
            let mut native = gui.native.borrow_mut();
            let window = native.windows.get_mut(&Panel::Sampler).unwrap();
            match mode {
                0 => window.focused = false,
                1 => window.ppp = 1.25,
                _ => window.monitor = Some(egui::vec2(1280.0, 900.0)),
            }
        }
        gui.frame(vec![]);
        assert!(!gui.held(0));
        gui.native
            .borrow_mut()
            .windows
            .get_mut(&Panel::Sampler)
            .unwrap()
            .focused = true;
        gui.frame(vec![]);
        gui.touch(0, egui::TouchPhase::Move);
        assert!(!gui.held(0));
    }
    gui.touch(0, egui::TouchPhase::Start);
    assert!(gui.held(0));
    gui.native
        .borrow_mut()
        .windows
        .get_mut(&Panel::Sampler)
        .unwrap()
        .close = true;
    gui.frame(vec![]);
    assert!(!gui.held(0));
    assert!(gui.app.workspace.closed.contains(&Panel::Sampler));
    gui.frame(vec![]);
    assert!(!gui.app.workspace.inputs.contains_key(&Panel::Sampler));
    assert!(
        touch::target_rect(&gui.ctx, egui::ViewportId::ROOT, touch::Target::Pad(0)).is_some(),
        "closing returns the actual sampler controls to the main window"
    );
    assert!(
        gui.app
            .settings
            .profile()
            .workspaces
            .current()
            .unwrap()
            .panels
            .iter()
            .find(|entry| entry.panel == Panel::Sampler)
            .unwrap()
            .detached,
        "closing does not overwrite the saved layout"
    );
}

#[test]
fn native_window_sizes_use_wayland_screen_dimensions_and_clamp_after_monitor_loss() {
    let mut gui = Gui::new(&[Panel::Sampler]);
    {
        let mut state = gui.native.borrow_mut();
        let window = state.windows.get_mut(&Panel::Sampler).unwrap();
        window.size = egui::vec2(1100.0, 800.0);
        window.monitor = None;
    }
    gui.frame(vec![]);
    assert_eq!(
        gui.app.workspace.sizes[&Panel::Sampler],
        egui::vec2(1100.0, 800.0),
        "no native inner_rect is required on Wayland"
    );
    {
        let mut state = gui.native.borrow_mut();
        let window = state.windows.get_mut(&Panel::Sampler).unwrap();
        window.size = egui::vec2(2400.0, 1600.0);
    }
    gui.frame(vec![]);
    assert_eq!(
        gui.app.workspace.sizes[&Panel::Sampler],
        egui::vec2(1600.0, 1200.0)
    );
    assert!(gui.native.borrow().windows[&Panel::Sampler].commands.iter().any(|command|matches!(command,egui::ViewportCommand::InnerSize(size) if *size==egui::vec2(1600.0,1200.0))));
    assert_eq!(
        gui.native.borrow().windows[&Panel::Sampler]
            .builder
            .clamp_size_to_monitor_size,
        Some(true)
    );
    assert_eq!(
        gui.native.borrow().windows[&Panel::Sampler]
            .builder
            .app_id
            .as_deref(),
        Some(crate::APPLICATION_ID)
    );
    gui.click(Panel::Sampler, "Return panel to main window");
    assert!(gui.app.workspace.closed.contains(&Panel::Sampler));
}

#[test]
fn native_windows_keep_transport_safety_warnings_and_one_shared_controller_target() {
    let mut gui = Gui::new(&[Panel::Sampler, Panel::Session]);
    gui.rt.apply(Command::SelectDeck(1));
    gui.rt.apply(Command::Select { track: 2, scene: 0 });
    gui.rt.publish_for_test();
    gui.frame(vec![]);
    for panel in [Panel::Sampler, Panel::Session] {
        let native = gui.native.borrow();
        let labels = native.windows[&panel]
            .nodes
            .iter()
            .filter_map(|(_, node)| node.label().or_else(|| node.value()))
            .collect::<Vec<_>>();
        for required in [
            "Play / stop session",
            "Stop session",
            "Emergency silence…",
            "Audio output unavailable",
            "Controller target: deck B, track 3",
        ] {
            assert!(
                labels.iter().any(|label| label.contains(required)),
                "missing {required}: {labels:?}"
            );
        }
    }
    gui.click(Panel::Session, "Play / stop session");
    assert!(gui.rt.playing);
    let selected = gui.rt.selected_track;
    gui.click(Panel::Sampler, "Stop session");
    assert!(!gui.rt.playing);
    assert_eq!(gui.rt.selected_deck, 1);
    assert_eq!(gui.rt.selected_track, selected);
    gui.click(Panel::Sampler, "Emergency silence…");
    gui.click(Panel::Sampler, "Confirm emergency silence");
    assert!(gui.app.engine.cmd.performance().status().output_muted);
    for panel in [Panel::Sampler, Panel::Session] {
        assert!(gui.native.borrow().windows[&panel]
            .nodes
            .iter()
            .any(|(_, node)| node
                .label()
                .or_else(|| node.value())
                .is_some_and(|label| label.contains("EMERGENCY OUTPUT MUTE"))));
    }
}

#[test]
fn native_child_touch_is_cancelled_by_root_preferences_on_its_first_frame() {
    let mut gui = Gui::new(&[Panel::Sampler]);
    gui.touch(0, egui::TouchPhase::Start);
    assert!(gui.held(0));
    gui.app.settings.open = true;
    gui.frame(vec![]);
    assert!(!gui.held(0));
    gui.app.settings.open = false;
    gui.frame(vec![]);
    gui.touch(0, egui::TouchPhase::Move);
    assert!(!gui.held(0));
    gui.touch(0, egui::TouchPhase::Start);
    assert!(gui.held(0));
    gui.app
        .settings
        .applied
        .profiles
        .get_mut("Studio")
        .unwrap()
        .workspaces
        .active = "Mix".into();
    gui.frame(vec![]);
    assert!(!gui.held(0));
    assert!(gui.app.workspace.inputs.is_empty());
}

#[test]
fn native_global_shortcut_uses_only_the_focused_secondary_window() {
    let mut gui = Gui::new(&[Panel::Sampler, Panel::Session]);
    gui.native
        .borrow_mut()
        .windows
        .get_mut(&Panel::Sampler)
        .unwrap()
        .focused = false;
    let key = |pressed| egui::Event::Key {
        key: Key::Space,
        physical_key: None,
        pressed,
        repeat: false,
        modifiers: Default::default(),
    };
    gui.input(Panel::Session, vec![key(true)]);
    assert!(gui.rt.playing);
    gui.input(Panel::Session, vec![key(false)]);
    assert!(gui.rt.playing);
    gui.input(Panel::Session, vec![key(true)]);
    assert!(!gui.rt.playing);
    gui.native
        .borrow_mut()
        .windows
        .get_mut(&Panel::Session)
        .unwrap()
        .focused = false;
    gui.input(Panel::Session, vec![key(false)]);
    gui.input(Panel::Session, vec![key(true)]);
    assert!(
        !gui.rt.playing,
        "an unfocused native window cannot own the transport shortcut"
    );
}

#[test]
fn manually_small_root_panel_clips_content_and_focus_scrolls_pads_back_into_reach() {
    let mut gui = Gui::new(&[Panel::Sampler]);
    gui.root_focused = true;
    let layout = gui
        .app
        .settings
        .applied
        .profiles
        .get_mut("Studio")
        .unwrap()
        .workspaces
        .saved
        .get_mut("Production")
        .unwrap();
    let sampler = layout
        .panels
        .iter_mut()
        .find(|entry| entry.panel == Panel::Sampler)
        .unwrap();
    sampler.detached = false;
    sampler.height = 64.0;
    gui.frame(vec![]);
    gui.frame(vec![]);
    let pad = gui
        .root_nodes
        .iter()
        .find(|(_, node)| {
            node.label()
                .is_some_and(|label| label.starts_with("Sampler: Pad 1:"))
        })
        .unwrap()
        .0;
    gui.frame(vec![egui::Event::AccessKitActionRequest(ActionRequest {
        target: pad,
        action: Action::Focus,
        data: None,
    })]);
    gui.frame(vec![]);
    gui.frame(vec![]);
    let hit = touch::target_rect(&gui.ctx, egui::ViewportId::ROOT, touch::Target::Pad(0))
        .expect("focused pad is actually reachable inside the resized panel");
    assert!(hit.height() > 0.0 && hit.height() <= 64.0);
    assert!(
        gui.root_nodes.iter().any(|(_, node)| node
            .label()
            .is_some_and(|label| label.contains("Sampler panel") && label.contains("scroll"))),
        "small panels expose named scroll controls"
    );
}

#[test]
fn moving_workspace_panels_releases_gui_deck_touch_and_preserves_controller_owners() {
    let mut gui = Gui::new(&[Panel::Decks]);
    gui.app.send(Command::DeckTouch { deck: 0, on: true });
    gui.app.send(Command::MidiDeckTouch {
        source: 99,
        deck: 0,
        on: true,
    });
    gui.frame(vec![]);
    assert!(gui.rt.decks[0].touching);
    gui.app
        .settings
        .applied
        .profiles
        .get_mut("Studio")
        .unwrap()
        .workspaces
        .active = "Mix".into();
    gui.frame(vec![]);
    assert!(gui.rt.decks[0].touching);
    gui.app.send(Command::MidiDeckTouch {
        source: 99,
        deck: 0,
        on: false,
    });
    gui.frame(vec![]);
    assert!(!gui.rt.decks[0].touching);
}

#[test]
fn closing_native_palette_owner_or_changing_workspace_restores_root_keyboard_controls() {
    for change_layout in [false, true] {
        let mut gui = Gui::new(&[Panel::Sampler, Panel::Session]);
        let chord = egui::Modifiers {
            ctrl: true,
            command: true,
            shift: true,
            ..Default::default()
        };
        gui.input(
            Panel::Session,
            vec![egui::Event::Key {
                key: Key::P,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: chord,
            }],
        );
        assert!(gui.app.command_palette.open);
        gui.native
            .borrow_mut()
            .windows
            .get_mut(&Panel::Sampler)
            .unwrap()
            .close = true;
        gui.frame(vec![]);
        assert!(
            gui.app.command_palette.open,
            "closing another window keeps commands open"
        );
        if change_layout {
            gui.app
                .settings
                .applied
                .profiles
                .get_mut("Studio")
                .unwrap()
                .workspaces
                .active = "DJ".into();
        } else {
            gui.native
                .borrow_mut()
                .windows
                .get_mut(&Panel::Session)
                .unwrap()
                .close = true;
        }
        gui.frame(vec![]);
        assert!(
            !gui.app.command_palette.open,
            "no command dialog survives its retired controls"
        );
        gui.root_focused = true;
        for window in gui.native.borrow_mut().windows.values_mut() {
            window.focused = false;
        }
        gui.frame(vec![egui::Event::Key {
            key: Key::Space,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Default::default(),
        }]);
        assert!(
            gui.rt.playing,
            "the actual root transport shortcut is usable again"
        );
    }
}
