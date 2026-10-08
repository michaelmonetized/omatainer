use super::*;
use egui::accesskit::{Action, ActionData, ActionRequest, Node, NodeId};

struct Editor {
    ctx: egui::Context,
    config: Config,
    status: Status,
    nodes: Vec<(NodeId, Node)>,
    time: f64,
}
impl Editor {
    fn new() -> Self {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut editor = Self {
            ctx,
            config: Config::default(),
            status: Status::default(),
            nodes: Vec::new(),
            time: 0.0,
        };
        editor.status.outputs.push(Endpoint {
            name: "Fixture synth".into(),
            id: Some("74:0".into()),
        });
        editor.frame(vec![]);
        editor.frame(vec![]);
        editor
    }
    fn frame(&mut self, events: Vec<egui::Event>) {
        self.time += 0.02;
        let output = self.ctx.run(
            egui::RawInput {
                events,
                time: Some(self.time),
                focused: true,
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::Vec2::new(1400.0, 1600.0),
                )),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default()
                    .show(ctx, |ui| edit(ui, &mut self.config, Some(&self.status)));
            },
        );
        self.nodes = output.platform_output.accesskit_update.unwrap().nodes;
    }
    fn action(&mut self, label: &str, action: Action, data: Option<ActionData>) {
        let target = self
            .nodes
            .iter()
            .find(|(_, node)| node.label() == Some(label))
            .map(|(id, _)| *id)
            .unwrap_or_else(|| {
                panic!(
                    "missing {label}: {:?}",
                    self.nodes
                        .iter()
                        .filter_map(|(_, node)| node.label())
                        .collect::<Vec<_>>()
                )
            });
        self.frame(vec![egui::Event::AccessKitActionRequest(ActionRequest {
            target,
            action,
            data,
        })]);
        self.frame(vec![]);
    }
    fn click(&mut self, label: &str) {
        self.action(label, Action::Click, None);
    }
}
#[test]
fn native_clock_destination_selection_compensation_and_profile_reopening_use_real_handlers() {
    let mut editor = Editor::new();
    editor.click("MIDI clock output");
    editor.click("Send song MIDI clock");
    editor.click("Add clock output");
    editor.click("Choose Clock output 1");
    editor.click("Fixture synth [74:0]");
    editor.action(
        "MIDI clock compensation",
        Action::SetValue,
        Some(ActionData::NumericValue(-18.0)),
    );
    assert_eq!(
        editor.config,
        Config {
            enabled: true,
            ports: vec![Endpoint {
                name: "Fixture synth".into(),
                id: Some("74:0".into())
            }],
            compensation_ms: -18
        }
    );
    let folder = std::env::temp_dir().join(format!(
        "omatainer-clock-native-{}",
        crate::sampler_bank::BankId::new().unwrap()
    ));
    std::fs::create_dir(&folder).unwrap();
    let path = folder.join("preferences.json");
    let mut preferences = crate::preferences::Preferences::defaults(&folder);
    preferences.profiles.get_mut("Studio").unwrap().midi_clock = editor.config.clone();
    crate::preferences::storage::save(
        &path,
        &preferences,
        crate::preferences::storage::Overwrite::New,
        &Default::default(),
    )
    .unwrap();
    let reopened = crate::preferences::storage::load(&path, &Default::default()).unwrap();
    assert_eq!(
        reopened.preferences.current().unwrap().midi_clock,
        editor.config
    );
    editor.click("Remove clock output 1");
    assert!(editor.config.ports.is_empty());
    assert!(editor.config.validate().is_err());
    editor.click("Send song MIDI clock");
    assert!(editor.config.validate().is_ok());
    std::fs::remove_dir_all(folder).unwrap();
}
