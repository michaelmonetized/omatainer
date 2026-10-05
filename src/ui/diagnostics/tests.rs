use super::*;
use crate::engine::audio::OutputCallback;
use capture::{Report, DURATION, INTERVAL, MAX_BYTES, MAX_SAMPLES};
use std::os::unix::fs::PermissionsExt;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!(
            "omatainer-diagnostics-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&p).unwrap();
        Self(p)
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
struct Gui {
    f: test_support::Fixture,
    ctx: egui::Context,
    time: f64,
}
impl Gui {
    fn new() -> Self {
        let mut f = test_support::Fixture::new(80);
        f.rt.publish_for_test();
        Self {
            f,
            ctx: egui::Context::default(),
            time: 0.0,
        }
    }
    fn frame(&mut self, events: Vec<egui::Event>) -> egui::FullOutput {
        self.time += 0.02;
        self.ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1800.0, 1400.0))),
                time: Some(self.time),
                events,
                ..Default::default()
            },
            |ctx| self.f.app.update_frame(ctx),
        )
    }
    fn click(&mut self, label: &str) {
        self.frame(vec![]);
        let output = self.frame(vec![]);
        let pos = test_support::label_center(&output, label);
        for pressed in [true, false] {
            self.frame(vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                },
            ]);
        }
    }
    fn wait_file(&mut self) {
        let end = Instant::now() + Duration::from_secs(3);
        while self.f.app.diagnostics.worker.is_some() {
            self.frame(vec![]);
            assert!(Instant::now() < end, "file worker stuck");
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    fn capture(&mut self) {
        self.click("Diagnostics");
        self.click("Start capture");
        self.frame(vec![]);
        self.click("Stop capture");
    }
}
fn visible(output: &egui::FullOutput, text: &str) -> bool {
    output.shapes.iter().any(|shape|matches!(&shape.shape,egui::epaint::Shape::Text(t) if t.galley.text().contains(text)))
}

#[test]
fn real_ui_captures_redacted_metadata_exports_reopens_and_does_not_modify_project() {
    let root = Directory::new();
    let mut gui = Gui::new();
    let secret = "/home/private-user/Secret Song.wav";
    {
        let mut snap = gui.f.app.engine.snap.lock();
        snap.decks[0].title = secret.into();
        snap.tracks[0].name = secret.into();
        snap.tracks[0].clips[0].name = secret.into();
        snap.midi = vec![secret.into()];
    }
    gui.capture();
    assert!(!gui.f.app.diagnostics.capture.running());
    assert_eq!(
        gui.f
            .app
            .diagnostics
            .capture
            .report
            .as_ref()
            .unwrap()
            .samples
            .len(),
        1
    );
    gui.f.app.diagnostics.path = root.0.join("capture.json").display().to_string();
    gui.click("Export redacted");
    gui.wait_file();
    assert_eq!(gui.f.app.diagnostics.status, "Redacted capture exported");
    let bytes = std::fs::read(root.0.join("capture.json")).unwrap();
    let text = std::str::from_utf8(&bytes).unwrap();
    assert!(
        !text.contains(secret) && !text.contains("private-user") && !text.contains("Secret Song")
    );
    assert_eq!(
        std::fs::metadata(root.0.join("capture.json"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    let report: Report = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(report.metadata.tracks, TRACKS);
    assert_eq!(report.metadata.midi_status_entries, 1);
    assert_eq!(report.metadata.app_version, env!("CARGO_PKG_VERSION"));
    assert!(report.metadata.logical_cpus > 0);
    gui.f.app.diagnostics.capture.report = None;
    let beat = gui.f.rt.beat;
    gui.click("Reopen capture");
    gui.wait_file();
    assert_eq!(
        gui.f
            .app
            .diagnostics
            .capture
            .report
            .as_ref()
            .unwrap()
            .samples
            .len(),
        1
    );
    assert_eq!(gui.f.rt.beat, beat);
    assert!(visible(
        &gui.frame(vec![]),
        "Capture reopened; live engine unchanged"
    ));
    // Existing target is a real failure, never silent overwrite.
    gui.click("Export redacted");
    gui.wait_file();
    assert!(gui
        .f
        .app
        .diagnostics
        .status
        .starts_with("Diagnostic file failed:"));
    assert_eq!(std::fs::read(root.0.join("capture.json")).unwrap(), bytes);
    std::fs::write(root.0.join("malformed.json"), b"bad json").unwrap();
    gui.f.app.diagnostics.path = root.0.join("malformed.json").display().to_string();
    gui.click("Reopen capture");
    gui.wait_file();
    assert!(gui
        .f
        .app
        .diagnostics
        .status
        .starts_with("Diagnostic file failed:"));
    assert_eq!(
        gui.f
            .app
            .diagnostics
            .capture
            .report
            .as_ref()
            .unwrap()
            .samples
            .len(),
        1
    );
}

#[test]
fn actual_ui_cancellation_keeps_no_capture_or_partial_file_and_remains_responsive() {
    let root = Directory::new();
    let mut gui = Gui::new();
    gui.click("Diagnostics");
    gui.click("Start capture");
    gui.click("Cancel capture");
    assert!(gui.f.app.diagnostics.capture.report.is_none());
    gui.click("Start capture");
    gui.frame(vec![]);
    gui.click("Stop capture");
    gui.f.app.diagnostics.path = root.0.join("cancelled.json").display().to_string();
    let gate = Arc::new(std::sync::Barrier::new(2));
    gui.f.app.diagnostics.file_gate = Some(gate.clone());
    gui.click("Export redacted");
    assert!(gui.f.app.diagnostics.worker.is_some());
    gui.click("Cancel file operation");
    for _ in 0..3 {
        gui.frame(vec![]);
    }
    gate.wait();
    gui.wait_file();
    assert_eq!(gui.f.app.diagnostics.status, "File operation cancelled");
    assert!(!root.0.join("cancelled.json").exists());
    assert_eq!(std::fs::read_dir(&root.0).unwrap().count(), 0);
}

#[test]
fn display_uses_real_callback_queue_and_separately_measured_ui_update() {
    let (engine, rt) = Engine::headless_for_test(48000, 80);
    let mut callback = OutputCallback::new(rt, 2);
    engine.cmd.set_profiling(true);
    callback.render_timed(&mut [0f32; 256], Some(Duration::from_millis(9)));
    engine.send(Command::Tap(Instant::now())).unwrap();
    let measured = engine.cmd.audio_metrics();
    let mut app = App::with_loader(engine, Theme::default(), None);
    app.diagnostics.open = true;
    app.diagnostics.ui_delay = Duration::from_millis(6);
    let ctx = egui::Context::default();
    let outside = Instant::now();
    let _ = ctx.run(egui::RawInput::default(), |ctx| app.update_frame(ctx));
    let ui_ns = app.diagnostics.ui_update_ns.unwrap();
    assert!(ui_ns >= 6_000_000 && ui_ns <= outside.elapsed().as_nanos() as u64);
    assert_eq!(app.engine.cmd.audio_metrics(), measured);
    let sample = app.diagnostic_sample();
    let output = ctx.run(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1800.0, 1400.0))),
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| show_sample(ui, &sample));
        },
    );
    for label in [
        "Predicted output latency 9.000 ms",
        "Audio queue 1/80",
        "Output 48000 Hz · 2 channels · 128 frames",
        "XRUNs unavailable",
        &format!("UI update wall {:.3} ms", ui_ns as f64 / 1e6),
    ] {
        assert!(visible(&output, label), "missing {label}");
    }
    ctx.style_mut(|style| style.animation_time = 0.0);
    let render = |events| {
        ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1800.0, 1400.0))),
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| show_sample(ui, &sample));
            },
        )
    };
    let panel = render(vec![]);
    let pos = test_support::label_center(&panel, "Track and device samples");
    for pressed in [true, false] {
        render(vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            },
        ]);
    }
    let panel = render(vec![]);
    let profile = sample.profile.as_ref().unwrap();
    assert_eq!(profile.frame, 0);
    let track = profile.costs.iter().find(|cost| cost.point == 2).unwrap();
    let device = profile
        .costs
        .iter()
        .find(|cost| cost.effect.is_some())
        .unwrap();
    for cost in [track, device] {
        assert!(visible(&panel, &cost.label()));
        if cost.elapsed_ns > 0 {
            assert!(visible(&panel, &format!("{} ns/frame", cost.elapsed_ns)));
        }
        assert!(visible(
            &panel,
            &format!(
                "{:.3}% estimated",
                cost.elapsed_ns as f64 * profile.sample_rate as f64 / 1e7
            )
        ));
    }
}

#[test]
fn capture_bounds_do_not_reconstruct_stalled_gui_time_and_files_validate_before_reopen() {
    let root = Directory::new();
    let gui = Gui::new();
    let start = Instant::now();
    let mut capture = Capture::default();
    capture.start(gui.f.app.diagnostic_metadata(), start);
    for i in 0..MAX_SAMPLES + 3 {
        capture.record(gui.f.app.diagnostic_sample(), start + INTERVAL * i as u32);
    }
    assert!(!capture.running());
    assert_eq!(capture.report.as_ref().unwrap().samples.len(), MAX_SAMPLES);
    capture.start(gui.f.app.diagnostic_metadata(), start);
    capture.record(gui.f.app.diagnostic_sample(), start);
    capture.record(
        gui.f.app.diagnostic_sample(),
        start + DURATION + Duration::from_secs(1),
    );
    assert!(!capture.running());
    assert_eq!(capture.report.as_ref().unwrap().samples.len(), 1);
    let flag = AtomicBool::new(false);
    let file = root.0.join("bad.json");
    for bytes in [b"not-json".to_vec(), vec![b'x'; MAX_BYTES + 1]] {
        std::fs::write(&file, bytes).unwrap();
        assert!(capture::reopen(&file, &flag).is_err());
    }
    let mut report = capture.report.unwrap();
    report.schema = 999;
    std::fs::write(&file, serde_json::to_vec(&report).unwrap()).unwrap();
    assert!(capture::reopen(&file, &flag)
        .unwrap_err()
        .contains("Unsupported"));
    report.schema = 1;
    report.samples[0].profile = Some(crate::engine::diagnostics::Profile {
        frame: 0,
        sample_rate: 48000,
        omitted_devices: 0,
        costs: vec![crate::engine::diagnostics::Cost {
            point: usize::MAX,
            effect: None,
            elapsed_ns: 0,
        }],
    });
    assert!(report.validate().unwrap_err().contains("identity"));
    report.samples[0].profile = None;
    flag.store(true, Ordering::Release);
    assert_eq!(
        capture::export(&root.0.join("never.json"), &report, &flag),
        Err("Cancelled".into())
    );
    assert!(!root.0.join("never.json").exists());
}

#[test]
fn performance_cancels_optional_export_and_reopen_but_preserves_committed_capture() {
    let root = Directory::new();
    let mut gui = Gui::new(); gui.capture();
    let path = root.0.join("show.json");
    let report = gui.f.app.diagnostics.capture.report.clone().unwrap();
    let handle = gui.f.app.engine.cmd.performance().clone();
    let gate = Arc::new(std::sync::Barrier::new(2));
    let worker = capture::Worker::start_for_show(path.clone(), Some(report.clone()), &handle, Some(gate.clone())).unwrap();
    handle.set_enabled(true).unwrap(); gate.wait();
    let end = Instant::now() + Duration::from_secs(3);
    loop { if let Some(result) = worker.poll() { assert!(matches!(result, capture::Completed::Cancelled)); break; } assert!(Instant::now() < end); std::thread::yield_now(); }
    assert!(!path.exists());
    assert!(capture::Worker::start_for_show(path.clone(), Some(report.clone()), &handle, None).is_err());
    assert!(capture::Worker::start_for_show(path.clone(), None, &handle, None).is_err());
    handle.set_enabled(false).unwrap();
    let worker = capture::Worker::start_for_show(path.clone(), Some(report), &handle, None).unwrap();
    loop { if let Some(result) = worker.poll() { assert!(matches!(result, capture::Completed::Exported)); break; } assert!(Instant::now() < end); std::thread::yield_now(); }
    let committed = std::fs::read(&path).unwrap();
    handle.set_enabled(true).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), committed);
    assert!(gui.f.app.diagnostics.capture.report.is_some());
}
