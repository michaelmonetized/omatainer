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
        rt.apply(Command::Select { track: 2, scene: 7 });
        let root = std::env::temp_dir().join(format!(
            "omat-native-audio-{}",
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
            if !self.app.audio_clips.busy()
                && !self.app.project_pending_for_test()
                && !self.app.library_metadata.active()
            {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "{}",
                self.app.audio_clips.message
            );
            std::thread::sleep(Duration::from_millis(2));
        }
        self.frame(vec![]);
    }
    fn wav(&self) -> PathBuf {
        let path = self.root.join("rainbow.wav");
        let mut writer = crate::audio_delivery::wav::Writer::new(
            &path,
            2,
            48000,
            crate::audio_delivery::wav::Encoding::Float32,
            false,
        )
        .unwrap();
        let data: Vec<f32> = (0..48000 * 7)
            .flat_map(|i| {
                let t = i as f32 / 48000.0;
                let mix = [
                    35.0_f32, 100.0, 250.0, 700.0, 1700.0, 4000.0, 9000.0, 16000.0,
                ]
                .into_iter()
                .map(|hz| (std::f32::consts::TAU * hz * t).sin() * 0.025)
                .sum::<f32>();
                [mix, mix * 0.75]
            })
            .collect();
        writer.append(&data, 1.0).unwrap();
        writer.finish().unwrap();
        path
    }
    fn load(&mut self, path: &std::path::Path) {
        self.app.audio_clips.path = path.display().to_string();
        self.frame(vec![]);
        self.click("Inspect audio file");
        self.wait();
        assert!(
            self.app.audio_clips.source.is_some(),
            "{}",
            self.app.audio_clips.message
        );
    }
}
impl Drop for Gui {
    fn drop(&mut self) {
        self.app.audio_clips.cancel();
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
#[test]
fn native_audio_import_rainbow_trim_loop_reverse_pitch_gain_apply_and_shared_reuse_work() {
    let mut g = Gui::new();
    g.click("Audio clip");
    g.wait();
    assert!(
        g.app.audio_clips.preview.is_some(),
        "{}",
        g.app.audio_clips.message
    );
    assert!(g.app.audio_clips.source.is_none());
    let path = g.wav();
    g.load(&path);
    let source = g
        .app
        .audio_clips
        .preview
        .as_ref()
        .unwrap()
        .source
        .as_ref()
        .unwrap()
        .clone();
    assert!(source.spectrum.is_some());
    let output = g.frame(vec![]);
    assert!(output
        .shapes
        .iter()
        .any(|s| matches!(&s.shape, egui::Shape::Mesh(m) if m.vertices.len()>20000)));
    g.set("Source start frame", 12000.0);
    g.set("Source end frame (exclusive)", 120000.0);
    g.set("Loop start frame", 24000.0);
    g.set("Loop end frame (exclusive)", 96000.0);
    g.click("Use inner loop on repeating launches");
    g.click("Reverse audio");
    g.set("Audio transpose", 7.0);
    g.set("Audio source tempo", 128.0);
    g.set("Audio clip gain", 0.42);
    g.click("Show trim");
    g.click("Zoom in audio");
    assert!(g.app.audio_clips.view_width < 0.4);
    let region = g.app.audio_clips.region.unwrap();
    let cursor = g.app.engine.undo.view().cursor;
    g.click("Apply audio clip");
    g.wait();
    assert_eq!(
        g.rt.tracks[2].clips[7].audio_region.unwrap().region,
        region,
        "{}",
        g.app.audio_clips.message
    );
    assert_eq!(g.rt.tracks[2].clips[7].gain, 0.42);
    assert_eq!(g.app.engine.undo.view().cursor, cursor + 1);
    assert!(Arc::ptr_eq(
        g.rt.tracks[2].clips[7].audio.as_ref().unwrap(),
        &source
    ));
    g.app.audio_clips.open = false;
    g.rt.apply(Command::Select { track: 3, scene: 7 });
    g.frame(vec![]);
    g.click("Audio clip");
    g.wait();
    g.load(&path);
    let reused = g
        .app
        .audio_clips
        .preview
        .as_ref()
        .unwrap()
        .source
        .as_ref()
        .unwrap();
    assert!(Arc::ptr_eq(reused, &source));
    g.click("Apply audio clip");
    g.wait();
    assert!(Arc::ptr_eq(
        g.rt.tracks[2].clips[7].audio.as_ref().unwrap(),
        g.rt.tracks[3].clips[7].audio.as_ref().unwrap()
    ));
    assert_ne!(
        g.rt.tracks[2].clips[7].audio_region.unwrap().region,
        g.rt.tracks[3].clips[7].audio_region.unwrap().region
    );
    g.rt.apply(Command::Undo);
    assert_eq!(g.rt.tracks[3].clips[7].kind, crate::engine::ClipKind::Empty);
    g.rt.apply(Command::Redo);
    assert!(g.rt.tracks[3].clips[7].audio_region.is_some());
}
#[test]
fn actual_mp3_and_damaged_file_import_have_explicit_results_and_midi_targets_are_preserved() {
    let mut g = Gui::new();
    g.click("Audio clip");
    g.wait();
    let wav = g.wav();
    let mp3 = g.root.join("source.mp3");
    let result = std::process::Command::new("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(&wav)
        .args(["-codec:a", "libmp3lame", "-b:a", "320k"])
        .arg(&mp3)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    g.load(&mp3);
    assert_eq!(
        g.app
            .audio_clips
            .preview
            .as_ref()
            .unwrap()
            .source
            .as_ref()
            .unwrap()
            .ch,
        2
    );
    g.click("Apply audio clip");
    g.wait();
    assert!(g.rt.tracks[2].clips[7].audio_region.is_some());
    let previous = g.rt.tracks[2].clips[7].audio.clone().unwrap();
    let broken = g.root.join("broken.wav");
    let bytes = std::fs::read(&wav).unwrap();
    std::fs::write(&broken, &bytes[..bytes.len() - 1000]).unwrap();
    g.app.audio_clips.path = broken.display().to_string();
    g.frame(vec![]);
    g.click("Inspect audio file");
    g.wait();
    assert!(
        g.app.audio_clips.message.contains("Incomplete")
            || g.app.audio_clips.message.contains("Damaged"),
        "{}",
        g.app.audio_clips.message
    );
    assert!(Arc::ptr_eq(
        g.rt.tracks[2].clips[7].audio.as_ref().unwrap(),
        &previous
    ));
    g.app.open_audio_clip(2, 0);
    g.wait();
    assert!(g.app.audio_clips.preview.is_none());
    assert!(
        g.app.audio_clips.message.contains("contains MIDI"),
        "{}",
        g.app.audio_clips.message
    );
}
