use super::*;
use crate::ui::audio_delivery::keyboard_tests::Gui;
use egui::accesskit::Invalid;

#[test]
fn keyboard_access_plugin_roots_catalog_native_keys_stable_owner_and_protection() {
    let mut gui = Gui::new();
    gui.plugins();
    let roots_id = gui.node("Plugin folders").0;
    gui.text("Plugin folders", "relative/plugins");
    gui.associated_error("Plugin folders");
    assert!(gui.node("Scan changed plugins").1.is_disabled());
    assert!(gui.node("Retry quarantined plugins").1.is_disabled());
    gui.key(Key::Enter, egui::Modifiers::NONE);
    assert!(
        !gui.app.plugins.busy(),
        "invalid folder text cannot start a host scan"
    );
    let root = gui.root.clone();
    gui.text("Plugin folders", "   ");
    assert!(
        !gui.node("Scan changed plugins").1.is_disabled(),
        "scanner trim/filter policy allows an empty roots list"
    );
    let missing = root.join("not-installed");
    gui.text("Plugin folders", missing.to_str().unwrap());
    assert!(
        !gui.node("Scan changed plugins").1.is_disabled(),
        "scanner catalog policy does not require a root to exist"
    );
    let duplicates = std::iter::repeat(missing.to_str().unwrap())
        .take(64)
        .collect::<Vec<_>>()
        .join("\n");
    gui.text("Plugin folders", &duplicates);
    assert!(
        !gui.node("Scan changed plugins").1.is_disabled(),
        "64 roots and repeated paths are allowed by the existing catalog policy"
    );
    gui.text(
        "Plugin folders",
        &(duplicates + "\n" + missing.to_str().unwrap()),
    );
    assert!(
        gui.node("Scan changed plugins").1.is_disabled(),
        "the existing catalog policy caps roots at64"
    );
    gui.text("Plugin folders", &format!("{}/../other", root.display()));
    assert!(
        gui.node("Scan changed plugins").1.is_disabled(),
        "the existing catalog policy rejects ParentDir components"
    );
    gui.text("Plugin folders", root.to_str().unwrap());
    assert!(gui.node("Plugin folders").1.invalid().is_none());
    gui.focused("Plugin folders");
    let focused = gui.ctx.memory(|memory| memory.focused()).unwrap();
    gui.idle();
    let rect = gui.ctx.read_response(focused).unwrap().rect;
    gui.idle();
    assert_eq!(
        gui.ctx.read_response(focused).unwrap().rect,
        rect,
        "native multiline layout settles after replacing 65 plugin folders"
    );
    gui.painted_focus("Plugin folders");
    assert_eq!(gui.app.plugins.roots, root.to_str().unwrap());
    gui.key(Key::Tab, egui::Modifiers::NONE);
    gui.focused("Reload plugin catalog");
    gui.key(Key::Tab, egui::Modifiers::SHIFT);
    gui.focused("Plugin folders");
    gui.app.engine.cmd.performance().set_enabled(true).unwrap();
    gui.idle();
    assert_eq!(
        gui.node("Plugin folders").0,
        roots_id,
        "protection notice does not replace the native text owner"
    );
    assert!(gui.node("Plugin folders").1.is_disabled());
    assert!(gui.node("Scan changed plugins").1.is_disabled());
    gui.frame(
        vec![egui::Event::Text("must not replace folders".into())],
        egui::Modifiers::NONE,
    );
    gui.key(Key::Enter, egui::Modifiers::NONE);
    assert_eq!(gui.app.plugins.roots, root.to_str().unwrap());
    assert!(!gui.app.plugins.busy());
    gui.app.engine.cmd.performance().set_enabled(false).unwrap();
    gui.idle();
    gui.press("Close native plugins");
    assert!(!gui.app.plugins.open);
    gui.plugins();
    assert_eq!(gui.node("Plugin folders").0, roots_id);
    assert_eq!(gui.app.plugins.roots, root.to_str().unwrap());

    let first = gui.root.join("First.vst3");
    let second = gui.root.join("Second.vst3");
    gui.app.plugins.catalog.records = [&first, &second]
        .into_iter()
        .map(|path| crate::plugin_host::scanner::Record {
            path: path.clone(),
            binary: None,
            classes: vec![],
            failure: Some("Inert UI metadata fixture; no native code".into()),
        })
        .collect();
    gui.idle();
    let second_id = gui.described_node("Blacklist plugin", second.to_str().unwrap());
    gui.app.plugins.catalog.records.rotate_left(1);
    gui.idle();
    assert_eq!(
        gui.described_node("Blacklist plugin", second.to_str().unwrap()),
        second_id,
        "saved bundle path survives catalog reorder"
    );
    gui.tab_to_node(second_id);
    gui.key(Key::Enter, egui::Modifiers::NONE);
    gui.until(|gui| !gui.app.plugins.busy() && gui.app.plugins.catalog.blacklist.contains(&second));
    assert_eq!(
        gui.described_node("Allow plugin", second.to_str().unwrap()),
        second_id,
        "blacklist state changes the name, not the native target"
    );
    assert!(!gui.rt.playing);
}

#[test]
fn keyboard_access_plugin_return_selector_native_keys_retained_id_and_stale_refusal() {
    let mut gui = Gui::new();
    let path = gui.root.join("Inert.vst3");
    let bus = vst3_host::AudioBusConfig {
        channel_count: 2,
        active: true,
    };
    let class = crate::plugin_host::Class {
        info: vst3_host::PluginInfo {
            path: path.clone(),
            name: "Fixture effect".into(),
            vendor: "UI fixture".into(),
            version: "1".into(),
            category: "Fx".into(),
            uid: "0123456789abcdef0123456789abcdef".into(),
            audio_inputs: 1,
            audio_outputs: 1,
            has_midi_input: false,
            has_midi_output: false,
            has_gui: false,
        },
        layout: vst3_host::AudioBusLayout {
            inputs: vec![bus],
            outputs: vec![bus],
        },
        parameters: vec![],
        latency: 0,
        tail: 0,
        native: None,
        lv2: None,
    };
    gui.app.plugins.catalog.records = vec![crate::plugin_host::scanner::Record {
        path: path.clone(),
        binary: Some(crate::plugin_host::BinaryIdentity {
            bundle: path.clone(),
            binary: path.join("inert.so"),
            sha256: "0".repeat(64),
            arch: std::env::consts::ARCH.into(),
        }),
        classes: vec![class],
        failure: None,
    }];
    let mut model = crate::engine::audio::routing::model::Model::default();
    model.next_id = 3;
    model.buses.push(crate::engine::audio::routing::model::Bus {
        id: 2,
        alias: "Fixture return".into(),
        channels: 2,
        gain: 1.0,
        mute: false,
        mixer: Default::default(),
    });
    assert!(
        model.plugins.is_empty(),
        "the typed return fixture contains no processors"
    );
    gui.rt.routing = Some(Box::new(
        crate::engine::audio::routing::prepared::Prepared::new(Arc::new(model), &gui.rt.session)
            .unwrap(),
    ));
    gui.plugins();
    assert!(
        gui.app.plugins.selected.is_none(),
        "metadata fixture does not preselect a plugin"
    );
    gui.press("Fixture effect · UI fixture · 1");
    assert_eq!(gui.app.plugins.selected, Some((0, 0)));
    let target_id = gui.node("Return for effect").0;
    assert!(gui.node("Append return effect").1.is_disabled());
    gui.toggle("Return for effect");
    gui.press("Fixture return");
    gui.tab_to("Append return effect");
    gui.painted_focus("Append return effect");
    assert!(!gui.node("Append return effect").1.is_disabled());
    let key = egui::Id::new("native-return-target");
    assert_eq!(
        gui.ctx.data(|data| data.get_temp::<([u64; 2], u64)>(key)),
        Some((gui.rt.session.namespace, 2))
    );
    gui.app.engine.cmd.performance().set_enabled(true).unwrap();
    gui.idle();
    assert!(gui.node("Append return effect").1.is_disabled());
    gui.key(Key::Enter, egui::Modifiers::NONE);
    assert!(
        !gui.app.audio_routing.open,
        "protected native Enter cannot start attach review or a host"
    );
    gui.app.engine.cmd.performance().set_enabled(false).unwrap();
    gui.rt.routing = None;
    gui.idle();
    assert_eq!(gui.node("Return for effect").0, target_id);
    assert!(gui.node("Append return effect").1.is_disabled());
    assert!(
        gui.ctx
            .data(|data| data.get_temp::<([u64; 2], u64)>(key))
            .is_none(),
        "removed retained return is cleared"
    );
    gui.ctx
        .data_mut(|data| data.insert_temp(key, ([u64::MAX; 2], 2_u64)));
    gui.idle();
    assert!(
        gui.ctx
            .data(|data| data.get_temp::<([u64; 2], u64)>(key))
            .is_none(),
        "foreign namespace cannot target a current return"
    );
    assert!(gui.node("Append return effect").1.is_disabled());
    assert_ne!(
        gui.node("Return for effect").1.invalid(),
        Some(Invalid::True),
        "an unchosen return stays a normal native chooser"
    );
    assert!(!gui.app.audio_routing.open);
    assert!(!gui.rt.playing);
}
