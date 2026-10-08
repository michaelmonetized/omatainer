use super::*;
use crate::ui::test_support::Fixture;
use egui::accesskit::{Action, ActionRequest, Node, NodeId};
#[test]
fn native_plugin_browser_controls_reload_persist_and_respect_protection() {
    let root = std::env::temp_dir().join(format!(
        "omatainer-plugin-ui-{}",
        crate::sampler_bank::BankId::new().unwrap()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("plugins.json");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let mut fixture = Fixture::new(256);
    fixture
        .app
        .plugins
        .set_performance(fixture.app.engine.cmd.performance().clone());
    fixture.app.plugins.initialize(path.clone());
    fixture.app.plugins.open = true;
    let mut nodes = Vec::<(NodeId, Node)>::new();
    let mut frame = |fixture: &mut Fixture, events: Vec<egui::Event>| {
        fixture.app.poll_plugins();
        let out = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1600., 1400.))),
                focused: true,
                events,
                ..Default::default()
            },
            |ctx| fixture.app.plugins_ui(ctx),
        );
        nodes = out.platform_output.accesskit_update.unwrap().nodes;
    };
    let until = Instant::now() + std::time::Duration::from_secs(3);
    while fixture.app.plugins.busy() {
        frame(&mut fixture, vec![]);
        assert!(Instant::now() < until);
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    fixture.app.plugins.roots = root.display().to_string();
    frame(&mut fixture, vec![]);
    drop(frame);
    assert!(nodes
        .iter()
        .any(|(_, n)| n.label() == Some("Plugin folders")));
    let scan = nodes
        .iter()
        .find(|(_, n)| n.label() == Some("Scan changed plugins"))
        .unwrap()
        .0;
    fixture
        .app
        .engine
        .cmd
        .performance()
        .set_enabled(true)
        .unwrap();
    let out = ctx.run(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1600., 1400.))),
            focused: true,
            events: vec![egui::Event::AccessKitActionRequest(ActionRequest {
                target: scan,
                action: Action::Click,
                data: None,
            })],
            ..Default::default()
        },
        |ctx| fixture.app.plugins_ui(ctx),
    );
    assert!(!fixture.app.plugins.busy());
    assert!(!path.exists());
    assert!(out
        .platform_output
        .accesskit_update
        .unwrap()
        .nodes
        .iter()
        .find(|(_, n)| n.label() == Some("Scan changed plugins"))
        .unwrap()
        .1
        .is_disabled());
    fixture
        .app
        .engine
        .cmd
        .performance()
        .set_enabled(false)
        .unwrap();
    let out = ctx.run(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1600., 1400.))),
            focused: true,
            ..Default::default()
        },
        |ctx| fixture.app.plugins_ui(ctx),
    );
    assert!(out
        .platform_output
        .accesskit_update
        .unwrap()
        .nodes
        .iter()
        .any(|(_, n)| n.label() == Some("Reload plugin catalog")));
    fixture
        .app
        .plugins
        .catalog
        .records
        .push(crate::plugin_host::scanner::Record {
            path: root.join("Unknown.vst3"),
            binary: None,
            classes: vec![],
            failure: Some("Missing native binary".into()),
        });
    let output = ctx.run(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1600., 1400.))),
            focused: true,
            ..Default::default()
        },
        |ctx| fixture.app.plugins_ui(ctx),
    );
    let button = output
        .platform_output
        .accesskit_update
        .unwrap()
        .nodes
        .iter()
        .find(|(_, n)| n.label() == Some("Blacklist plugin"))
        .unwrap()
        .0;
    let _ = ctx.run(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1600., 1400.))),
            focused: true,
            events: vec![egui::Event::AccessKitActionRequest(ActionRequest {
                target: button,
                action: Action::Click,
                data: None,
            })],
            ..Default::default()
        },
        |ctx| fixture.app.plugins_ui(ctx),
    );
    let deadline = Instant::now() + std::time::Duration::from_secs(3);
    while fixture.app.plugins.busy() {
        fixture.app.poll_plugins();
        assert!(Instant::now() < deadline);
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    assert!(path.exists());
    assert!(fixture
        .app
        .plugins
        .catalog
        .blacklist
        .contains(&root.join("Unknown.vst3")));
    let mut loaded = crate::plugin_host::scanner::Browser::default();
    loaded.initialize(path);
    while loaded.busy() {
        loaded.poll();
        assert!(Instant::now() < deadline);
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    assert!(loaded
        .catalog
        .blacklist
        .contains(&root.join("Unknown.vst3")));
    std::fs::remove_dir_all(root).unwrap();
}
