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
            "omat-native-arrangement-{}",
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
            if !self.app.arrangement.busy()
                && !self.app.project_pending_for_test()
                && !self.app.library_metadata.active()
            {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "{}; arrangement_busy={}, project_pending={}, library_active={}, library={}",
                self.app.arrangement.message,
                self.app.arrangement.busy(),
                self.app.project_pending_for_test(),
                self.app.library_metadata.active(),
                self.app.library_metadata.label()
            );
            std::thread::sleep(Duration::from_millis(2));
        }
        self.frame(vec![]);
    }
}
impl Drop for Gui {
    fn drop(&mut self) {
        self.app.arrangement.cancel();
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
#[test]
fn native_shared_source_placement_edit_apply_and_hour_navigation_are_real_handlers() {
    let mut g = Gui::new();
    let data = (0..144000)
        .map(|i| (i as f32 * 0.057).sin() * 0.1)
        .collect::<Vec<_>>();
    let spectrum = crate::engine::waveform::Waveform::analyze(&data, 1, 48000, || false).unwrap();
    let audio = Arc::new(crate::engine::dsp::Sample {
        name: "native timeline spectrum".into(),
        sr: 48000,
        ch: 1,
        data,
        peaks: Default::default(),
        spectrum: Some(Arc::new(spectrum)),
        bpm: 120.0,
        path: String::new(),
    });
    let region = crate::engine::audio_clip::Region::full(&audio, 120.0)
        .unwrap()
        .prepare(&audio)
        .unwrap();
    let mut clip = crate::engine::Clip::empty();
    clip.kind = crate::engine::ClipKind::Audio;
    clip.name = audio.name.clone();
    clip.bars = (region.duration_beats / 4.0) as f32;
    clip.audio = Some(audio.clone());
    clip.audio_region = Some(region);
    g.rt.tracks[2].clips[7] = clip;
    g.click("Arrangement timeline");
    g.wait();
    let preview = g.app.arrangement.preview.as_ref().unwrap();
    let index = preview
        .choices
        .iter()
        .position(|c| c.0 == 2 && c.1 == 7)
        .unwrap();
    let target = preview.choices[index].2.clone();
    g.click("Session source");
    g.click(&target);
    g.click("Retain Session source");
    g.click("Place source at range start");
    g.set("Instance start beat", 4.0);
    g.set("Source offset beats", 1.0);
    g.set("Instance duration beats", 2.0);
    g.click("Copy instance");
    assert_eq!(g.app.arrangement.model.instances.len(), 2);
    assert_eq!(g.app.arrangement.model.sources.len(), 1);
    assert_eq!(g.app.arrangement.model.instances[1].start, 6.0);
    let destination = g.rt.session.tracks[1].name.clone();
    g.click("Destination track");
    g.click(&destination);
    g.click("Move instance to destination track");
    assert_eq!(
        g.app.arrangement.model.instances[1].track,
        g.rt.session.reference(Axis::Track, 1).unwrap()
    );
    let output = g.frame(vec![]);
    let mesh_vertices = output
        .shapes
        .iter()
        .filter_map(|s| {
            if let egui::Shape::Mesh(m) = &s.shape {
                Some(m.vertices.len())
            } else {
                None
            }
        })
        .sum::<usize>();
    assert!(mesh_vertices > 10000, "{mesh_vertices} vertices");
    g.click("Use Arrangement playback");
    g.click("Apply song");
    g.wait();
    assert!(g.rt.arrangement.enabled());
    assert_eq!(
        g.rt.arrangement
            .plan
            .as_ref()
            .unwrap()
            .model
            .instances
            .len(),
        2
    );
    g.set("Range start", 7196.0);
    g.set("Range end", 7204.0);
    g.click("Show selected range");
    assert_eq!(g.app.arrangement.start, 7196.0);
    assert_eq!(g.app.arrangement.width, 8.0);
    g.click("Seek range start");
    assert!((g.app.snap.beat - 7196.0).abs() < 1e-10);
    g.click("Play song");
    assert!(g.rt.playing);
    g.click("Pause song");
    assert!(!g.rt.playing);
    g.click("Rewind song");
    assert_eq!(g.app.snap.beat, 0.0);
    g.set("First visible beat", 4.0);
    g.set("Visible beats", 8.0);
    let song = g.rt.arrangement.plan.as_ref().unwrap();
    assert_eq!(song.model.instances[0].offset, 1.0);
    assert_eq!(song.model.sources[0].clip.audio_region.unwrap().start, 0);
    g.rt.apply(Command::Undo);
    assert!(!g.rt.arrangement.enabled());
    g.rt.apply(Command::Redo);
    assert!(g.rt.arrangement.enabled());
}

#[test]
fn midi_preview_clips_held_offsets_and_repeated_inner_loop_occurrences_to_visible_song_range() {
    let mut g = Gui::new();
    g.click("Arrangement timeline");
    g.wait();
    let preview = g.app.arrangement.preview.as_ref().unwrap();
    let mut clip = preview.captured.state.tracks[1].clips[0].clone();
    clip.bars = 2.0;
    let mut region = crate::engine::midi_edit::Region::full(2.0);
    region.start = 1.0;
    region.end = 7.0;
    region.loop_start = 3.0;
    region.loop_end = 5.0;
    clip.region = Some(region);
    clip.notes = vec![crate::engine::MidiNote { variation: None,
        id: crate::engine::midi_edit::NoteId::new(),
        muted: false,
        pitch: 64,
        vel: 100,
        start: 3.0,
        len: 1.5,
        channel: 0,
        release_vel: 64,
        source_timing: None,
    }];
    let instance = Instance {
        id: 2,
        source: 1,
        track: g.rt.session.reference(Axis::Track, 1).unwrap(),
        start: 10.0,
        offset: 2.5,
        duration: 8.0,
        repeating: true,
        gain: 1.0,
        fades: None,
        fade_link: 0,
        crossfade: None,
    };
    let mut spans = Vec::new();
    midi_preview(&clip, instance, [10.0, 18.0], |pitch, a, b| {
        spans.push((pitch, a, b))
    });
    assert_eq!(
        spans,
        vec![
            (64, 10.0, 11.0),
            (64, 11.5, 13.0),
            (64, 13.5, 15.0),
            (64, 15.5, 17.0),
            (64, 17.5, 18.0)
        ]
    );
    spans.clear();
    midi_preview(&clip, instance, [14.0, 14.75], |pitch, a, b| {
        spans.push((pitch, a, b))
    });
    assert_eq!(spans, vec![(64, 14.0, 14.75)]);
}

#[test]
fn native_arrangement_crossfade_and_aligned_fade_links_use_actual_widgets_and_one_apply_undo() {
    let mut g = Gui::new();
    let audio = Arc::new(crate::engine::dsp::Sample {
        name: "native fade source".into(),
        sr: 48000,
        ch: 2,
        data: (0..192000)
            .flat_map(|i| {
                let a = (i as f32 * 0.041 + 0.5).sin() * 0.1;
                [a, -a * 0.6]
            })
            .collect(),
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
    clip.name = audio.name.clone();
    clip.bars = (region.duration_beats / 4.0) as f32;
    clip.audio = Some(audio.clone());
    clip.audio_region = Some(region);
    g.rt.tracks[2].clips[7] = clip;
    g.click("Arrangement timeline");
    g.wait();
    let preview = g.app.arrangement.preview.as_ref().unwrap();
    let choice = preview
        .choices
        .iter()
        .find(|c| c.0 == 2 && c.1 == 7)
        .unwrap()
        .2
        .clone();
    g.click("Session source");
    g.click(&choice);
    g.click("Retain Session source");
    g.click("Place source at range start");
    g.set("Instance start beat", 1.0);
    g.set("Source offset beats", 1.0);
    g.set("Instance duration beats", 2.0);
    g.set("Fade in beats", 0.1);
    g.set("Fade in curve", -0.5);
    g.set("Fade out curve", 0.5);
    let outgoing = g.app.arrangement.selected.unwrap();
    g.click("Copy instance");
    let incoming = g.app.arrangement.selected.unwrap();
    g.set("Source offset beats", 3.0);
    g.set("Fade in beats", 0.0);
    g.app.arrangement.selected = Some(outgoing);
    g.frame(vec![]);
    g.click("Crossfade partner");
    g.click(&format!("Crossfade into instance {incoming} at beat 3"));
    g.set("Crossfade length beats", 0.5);
    g.set("Crossfade curve", 0.4);
    g.click("Create or resize crossfade");
    let a = g
        .app
        .arrangement
        .model
        .instances
        .iter()
        .find(|i| i.id == outgoing)
        .unwrap();
    let b = g
        .app
        .arrangement
        .model
        .instances
        .iter()
        .find(|i| i.id == incoming)
        .unwrap();
    assert_eq!(a.crossfade, Some(incoming));
    assert_eq!(a.fades.unwrap().fade_out, 0.5);
    assert_eq!(b.fades.unwrap().fade_in, 0.5);
    assert_eq!(a.fades.unwrap().out_curve, 0.4);
    g.click("Unlink crossfade");
    assert_eq!(
        g.app
            .arrangement
            .model
            .instances
            .iter()
            .find(|i| i.id == outgoing)
            .unwrap()
            .crossfade,
        None
    );
    g.click("Copy instance");
    let linked = g.app.arrangement.selected.unwrap();
    g.set("Instance start beat", 1.0);
    let destination = g.rt.session.tracks[1].name.clone();
    g.click("Destination track");
    g.click(&destination);
    g.click("Move instance to destination track");
    g.app.arrangement.selected = Some(outgoing);
    g.frame(vec![]);
    g.click("Fade link partner");
    g.click(&format!("Link instance {linked} at beat 1"));
    g.click("Link aligned fades");
    g.set("Fade in beats", 0.2);
    g.set("Fade in curve", 0.6);
    let a = g
        .app
        .arrangement
        .model
        .instances
        .iter()
        .find(|i| i.id == outgoing)
        .unwrap();
    let linked = g
        .app
        .arrangement
        .model
        .instances
        .iter()
        .find(|i| i.id == linked)
        .unwrap();
    assert!(a.fade_link != 0);
    assert_eq!(a.fade_link, linked.fade_link);
    assert_eq!(a.fades, linked.fades);
    let reviewed = g.app.arrangement.model.instances.clone();
    let cursor = g.app.engine.undo.view().cursor;
    g.click("Use Arrangement playback");
    g.click("Apply song");
    g.wait();
    assert_eq!(g.app.engine.undo.view().cursor, cursor + 1);
    assert_eq!(
        g.rt.arrangement.plan.as_ref().unwrap().model.instances,
        reviewed
    );
    g.rt.apply(Command::Undo);
    assert!(!g.rt.arrangement.enabled());
    g.rt.apply(Command::Redo);
    assert!(g.rt.arrangement.enabled());
}
