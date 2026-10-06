use super::*;
use egui::accesskit::{Action, ActionData, ActionRequest, Node, NodeId};
use std::time::Duration;
struct Gui {
    app: App,
    rt: Box<crate::engine::RtEngine>,
    ctx: egui::Context,
    nodes: Vec<(NodeId, Node)>,
    time: f64,
    root: PathBuf,
}
impl Gui {
    fn new() -> Self {
        let fixture = test_support::Fixture::new(256);
        let mut rt = fixture.rt;
        rt.apply(Command::Stop);
        rt.apply(Command::Select { track: 0, scene: 7 });
        let root = std::env::temp_dir().join(format!(
            "omat-native-clip_manager-{}",
            crate::sampler_bank::BankId::new().unwrap()
        ));
        std::fs::create_dir(&root).unwrap();
        let mut app = fixture.app;
        app.library_metadata =
            library_metadata::Metadata::with_hook(root.join("catalog.json"), || {});
        app.library_metadata
            .set_performance(app.engine.cmd.performance().clone());
        app.library_initialized = false;
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut g = Self {
            app,
            rt,
            ctx,
            nodes: vec![],
            time: 0.0,
            root,
        };
        for _ in 0..4 {
            g.frame(vec![]);
        }
        g
    }
    fn frame(&mut self, events: Vec<egui::Event>) -> egui::FullOutput {
        self.time += 0.02;
        self.rt.publish_for_test();
        let output = self.ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1800.0, 1800.0))),
                time: Some(self.time),
                events,
                focused: true,
                ..Default::default()
            },
            |ctx| self.app.update_frame(ctx),
        );
        self.nodes = output
            .platform_output
            .accesskit_update
            .as_ref()
            .unwrap()
            .nodes
            .clone();
        self.rt.process(&mut [0.0; 256]);
        output
    }
    fn action(&mut self, label: &str, action: Action, data: Option<ActionData>) {
        let target = self
            .nodes
            .iter()
            .find(|(_, n)| n.label() == Some(label) && n.supports_action(action))
            .map(|(id, _)| *id)
            .unwrap_or_else(|| {
                panic!(
                    "Missing {label}: {:?}",
                    self.nodes
                        .iter()
                        .filter_map(|(_, n)| n.label())
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
    fn set(&mut self, label: &str, value: f64) {
        self.action(
            label,
            Action::SetValue,
            Some(ActionData::NumericValue(value)),
        );
    }
    fn wait(&mut self) {
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            self.frame(vec![]);
            if !self.app.clip_manager.busy()
                && !self.app.project_pending_for_test()
                && !self.app.library_metadata.active()
            {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "{}",
                self.app.clip_manager.message
            );
            std::thread::sleep(Duration::from_millis(2));
        }
        self.frame(vec![]);
    }
}
impl Drop for Gui {
    fn drop(&mut self) {
        self.app.clip_manager.cancel();
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
impl Gui {
    fn key(&mut self, key: egui::Key, modifiers: egui::Modifiers) {
        self.frame(vec![egui::Event::Key {
            key,
            physical_key: Some(key),
            pressed: true,
            repeat: false,
            modifiers,
        }]);
        self.frame(vec![egui::Event::Key {
            key,
            physical_key: Some(key),
            pressed: false,
            repeat: false,
            modifiers,
        }]);
    }
    fn choose(&mut self, label: &str, track: usize, scene: usize) {
        let slot = Slot::at(&self.rt.session, track, scene).unwrap();
        let name = self
            .app
            .clip_manager
            .preview
            .as_ref()
            .unwrap()
            .slots
            .iter()
            .find(|(s, _)| *s == slot)
            .unwrap()
            .1
            .clone();
        self.click(label);
        self.frame(vec![]);
        self.click(&name);
    }
}
#[test]
fn native_create_copy_move_rename_disable_delete_and_keyboard_actions_use_atomic_handlers() {
    let mut g = Gui::new();
    g.click("Session clips");
    g.wait();
    g.set("New MIDI clip bars", 2.0);
    g.click("Custom clip color");
    g.set("Clip color red", 20.0);
    g.set("Clip color green", 240.0);
    g.set("Clip color blue", 80.0);
    g.click("Disable clip");
    g.key(egui::Key::N, egui::Modifiers::CTRL);
    g.wait();
    assert_eq!(g.rt.tracks[0].clips[7].kind, crate::engine::ClipKind::Midi);
    assert_eq!(g.rt.tracks[0].clips[7].bars, 2.0);
    assert!(g.rt.tracks[0].clips[7].properties.disabled);
    assert_eq!(g.rt.tracks[0].clips[7].properties.color, Some([20, 240, 80]));
    g.action("Clip name", Action::Focus, None);
    g.key(egui::Key::A, egui::Modifiers::CTRL);
    g.frame(vec![egui::Event::Text("native renamed phrase".into())]);
    g.key(egui::Key::Enter, egui::Modifiers::CTRL);
    g.wait();
    assert_eq!(g.rt.tracks[0].clips[7].name, "native renamed phrase");
    assert!(g.rt.tracks[0].clips[7].properties.disabled);
    assert_eq!(
        g.rt.tracks[0].clips[7].properties.color,
        Some([20, 240, 80])
    );
    g.choose("Destination slot", 0, 6);
    g.key(egui::Key::D, egui::Modifiers::CTRL);
    g.wait();
    assert_eq!(g.rt.tracks[0].clips[6].name, "native renamed phrase");
    assert_eq!(
        g.rt.tracks[0].clips[6].properties,
        g.rt.tracks[0].clips[7].properties
    );
    g.choose("Destination slot", 0, 5);
    g.key(egui::Key::M, egui::Modifiers::CTRL);
    g.wait();
    assert_eq!(g.rt.tracks[0].clips[7].kind, crate::engine::ClipKind::Empty);
    assert_eq!(g.rt.tracks[0].clips[5].name, "native renamed phrase");
    g.key(
        egui::Key::Backspace,
        egui::Modifiers::CTRL | egui::Modifiers::SHIFT,
    );
    g.wait();
    assert_eq!(g.rt.tracks[0].clips[5].kind, crate::engine::ClipKind::Empty);
    g.rt.apply(Command::Undo);
    assert_eq!(g.rt.tracks[0].clips[5].name, "native renamed phrase");
}
#[test]
fn native_preset_save_inspect_insert_and_audition_produce_real_audio() {
    let mut g = Gui::new();
    let audio = Arc::new(crate::engine::dsp::Sample {
        name: "preset audition".into(),
        sr: 48000,
        ch: 2,
        data: vec![0.1; 48000 * 2],
        peaks: Default::default(),
        spectrum: None,
        bpm: 120.0,
        path: String::new(),
    });
    let region = crate::engine::audio_clip::Region::full(&audio, 120.0)
        .unwrap()
        .prepare(&audio)
        .unwrap();
    let mut clip = crate::engine::Clip::empty();
    clip.kind = crate::engine::ClipKind::Audio;
    clip.name = "source stays intact".into();
    clip.bars = (region.duration_beats / 4.0) as f32;
    clip.audio = Some(audio.clone());
    clip.audio_region = Some(region);
    g.rt.tracks[0].clips[7] = clip;
    g.click("Session clips");
    g.wait();
    let path = g.root.join("audition.omatclip");
    g.app.clip_manager.path = path.to_string_lossy().into_owned();
    g.key(egui::Key::S, egui::Modifiers::CTRL | egui::Modifiers::SHIFT);
    g.wait();
    assert!(path.is_file());
    let source_bytes = std::fs::read(&path).unwrap();
    g.key(egui::Key::O, egui::Modifiers::CTRL);
    g.wait();
    assert!(g
        .app
        .clip_manager
        .preview
        .as_ref()
        .unwrap()
        .preset
        .is_some());
    g.choose("Destination slot", 0, 6);
    g.click("Insert preset in destination");
    g.wait();
    assert_eq!(g.rt.tracks[0].clips[6].name, "source stays intact");
    assert!(Arc::ptr_eq(
        g.rt.tracks[0].clips[6].audio.as_ref().unwrap(),
        &audio
    ));
    assert_eq!(source_bytes, std::fs::read(&path).unwrap());
    g.rt.apply(Command::ToggleQuant);
    g.rt.apply(Command::Master(0.5));
    g.click("Refresh clips");
    g.wait();
    g.click("Audition destination");
    let mut out = vec![0.0; 2048];
    g.rt.process(&mut out);
    assert_eq!(g.rt.tracks[0].playing.unwrap().scene, 6);
    assert!(out.iter().any(|v| v.abs() > 0.0001));
    g.click("Stop destination track");
    assert!(g.rt.tracks[0].playing.is_none());
}
