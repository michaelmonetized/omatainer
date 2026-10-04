use super::test_support::{label_center, Fixture};
use super::*;
use crate::engine::load_receipt::Media;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

struct Files(PathBuf);
impl Files {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "omatainer-load-status-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn wave(&self, name: &str) -> PathBuf {
        let path = self.0.join(name);
        let mut bytes = Vec::new();
        let frames = 480u32;
        bytes.extend(b"RIFF");
        bytes.extend((36 + frames * 2).to_le_bytes());
        bytes.extend(b"WAVEfmt ");
        bytes.extend(16u32.to_le_bytes());
        bytes.extend(1u16.to_le_bytes());
        bytes.extend(1u16.to_le_bytes());
        bytes.extend(48000u32.to_le_bytes());
        bytes.extend(96000u32.to_le_bytes());
        bytes.extend(2u16.to_le_bytes());
        bytes.extend(16u16.to_le_bytes());
        bytes.extend(b"data");
        bytes.extend((frames * 2).to_le_bytes());
        for _ in 0..frames {
            bytes.extend(12000i16.to_le_bytes());
        }
        std::fs::write(&path, bytes).unwrap();
        path
    }
}
impl Drop for Files {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn frame(
    app: &mut App,
    ctx: &egui::Context,
    time: f64,
    events: Vec<egui::Event>,
) -> egui::FullOutput {
    ctx.run(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1440.0, 900.0))),
            time: Some(time),
            events,
            ..Default::default()
        },
        |ctx| app.update_frame(ctx),
    )
}
fn visible(output: &egui::FullOutput, text: &str) -> bool {
    output.shapes.iter().any(|shape| matches!(&shape.shape,
        egui::epaint::Shape::Text(label) if label.galley.text().contains(text)
        && label.visual_bounding_rect().intersects(Rect::from_min_size(Pos2::ZERO, egui::vec2(1440.0, 900.0)))))
}
fn assert_visible(output: &egui::FullOutput, text: &str) {
    assert!(visible(output, text), "missing painted {text}");
}
fn click(app: &mut App, ctx: &egui::Context, pos: Pos2, time: f64) {
    for (pressed, time) in [(true, time), (false, time + 0.01)] {
        frame(
            app,
            ctx,
            time,
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                },
            ],
        );
    }
}

#[test]
fn production_missing_corrupt_and_successful_loads_paint_identity_and_acknowledged_state() {
    let files = Files::new();
    let missing = files.0.join("missing.wav");
    let corrupt = files.0.join("corrupt.wav");
    std::fs::write(&corrupt, b"this is not an audio container").unwrap();
    let valid = files.wave("valid.wav");
    let mut f = Fixture::new(256);
    f.app.loader = Some(Loader::start().unwrap());
    let ctx = egui::Context::default();
    let original = f.rt.decks[0].audio.clone().unwrap();
    for (i, (path, title)) in [(&missing, "missing media"), (&corrupt, "corrupt media")]
        .into_iter()
        .enumerate()
    {
        f.app.load_file(0, path.clone(), title);
        f.poll_loads();
        let output = frame(&mut f.app, &ctx, 1.0 + i as f64, vec![]);
        assert_visible(&output, &format!("load failed on A: {title}:"));
        assert_visible(&output, &path.to_string_lossy());
        assert_visible(&output, "Retry");
        assert_visible(&output, "Dismiss");
        assert!(f.rt.cmd_rx.is_empty());
        assert!(Arc::ptr_eq(
            f.rt.decks[0].audio.as_ref().unwrap(),
            &original
        ));
    }
    f.app.load_file(1, valid.clone(), "successful media");
    f.poll_loads();
    let output = frame(&mut f.app, &ctx, 4.0, vec![]);
    assert_visible(&output, "queued successful media → B");
    assert_visible(&output, "load failed on A: corrupt media:"); // the other deck cannot erase the error
    assert!(!visible(&output, "loaded successful media"));
    let command = f.rt.cmd_rx.try_recv().unwrap();
    assert!(
        matches!(&command, Command::DeckLoadRequested { deck: 1, media: Media::Decoded { audio, .. }, .. } if audio.frames() == 480)
    );
    f.rt.apply(command);
    let output = frame(&mut f.app, &ctx, 5.0, vec![]);
    assert_visible(&output, "loaded successful media → B");
    assert_eq!(
        f.rt.decks[1].audio.as_ref().unwrap().path,
        valid.to_string_lossy()
    );
    assert_visible(&output, "load failed on A: corrupt media:");
}

#[test]
fn retry_uses_original_source_after_browsing_and_errors_persist_until_dismissed() {
    let files = Files::new();
    let path = files.0.join("retry.wav");
    let mut f = Fixture::new(256);
    f.app.loader = Some(Loader::start().unwrap());
    f.app.load_file(0, path.clone(), "retry this file");
    f.poll_loads();
    let ctx = egui::Context::default();
    let output = frame(&mut f.app, &ctx, 3600.0, vec![]); // no timeout-based disappearance
    assert_visible(&output, "load failed on A: retry this file:");
    let retry = label_center(&output, "Retry");
    f.app.lib_filter = "Harmony".into();
    files.wave("retry.wav");
    click(&mut f.app, &ctx, retry, 3601.0);
    // Completion may arrive during the release frame; poll only while decoding.
    if matches!(
        f.app.loads[0].as_ref().unwrap().phase,
        load_status::Phase::Loading
    ) {
        f.poll_loads();
    }
    let command = f.rt.cmd_rx.try_recv().unwrap();
    assert!(
        matches!(&command, Command::DeckLoadRequested { deck: 0, media: Media::Decoded { audio, .. }, .. } if audio.path == path.to_string_lossy())
    );
    f.rt.apply(command);
    let output = frame(&mut f.app, &ctx, 3602.0, vec![]);
    assert_visible(&output, "loaded retry this file → A");

    f.app
        .load_file(1, files.0.join("still missing.wav"), "dismiss this error");
    f.poll_loads();
    frame(&mut f.app, &ctx, 7199.0, vec![]); // settle the bottom panel after adding deck B
    let output = frame(&mut f.app, &ctx, 7200.0, vec![]);
    assert_visible(&output, "load failed on B: dismiss this error:");
    // Last matching label belongs to B, after the already successful A row.
    let dismiss = label_center(&output, "Dismiss");
    click(&mut f.app, &ctx, dismiss, 7201.0);
    let output = frame(&mut f.app, &ctx, 7202.0, vec![]);
    assert!(!visible(&output, "dismiss this error"));
    assert_visible(&output, "loaded retry this file → A");
    assert!(f.app.loads[1].is_none());
}

#[test]
fn builtins_require_actual_application_even_when_same_title_is_already_current() {
    let mut f = Fixture::new(256);
    let ctx = egui::Context::default();
    f.app.lib_filter = "Drums".into();
    f.app.load_sel(0);
    let output = frame(&mut f.app, &ctx, 1.0, vec![]);
    assert_visible(&output, "queued Drums (session) → A");
    assert!(!visible(&output, "loaded Drums (session)"));
    f.rt.process(&mut [0.0; 2]);
    assert_visible(
        &frame(&mut f.app, &ctx, 2.0, vec![]),
        "loaded Drums (session) → A",
    );
    f.app.load_sel(0);
    assert_visible(
        &frame(&mut f.app, &ctx, 3.0, vec![]),
        "queued Drums (session) → A",
    );
    f.rt.process(&mut [0.0; 2]);
    assert_visible(
        &frame(&mut f.app, &ctx, 4.0, vec![]),
        "loaded Drums (session) → A",
    );
    f.rt.apply(Command::LoadBuiltin { deck: 0, stem: 1 });
    let output = frame(&mut f.app, &ctx, 5.0, vec![]);
    assert_visible(&output, "replaced or unloaded");
    assert!(!visible(&output, "loaded Drums (session)"));
    f.app.load_sel(0);
    f.rt.process(&mut [0.0; 2]);
    assert_visible(
        &frame(&mut f.app, &ctx, 6.0, vec![]),
        "loaded Drums (session) → A",
    );
    f.rt.apply(Command::DeckUnload { deck: 0 });
    let output = frame(&mut f.app, &ctx, 7.0, vec![]);
    assert_visible(&output, "replaced or unloaded");
    assert!(!visible(&output, "loaded Drums (session)"));
}

#[test]
fn unavailable_builtin_and_disconnected_renderer_produce_retryable_failure() {
    let mut f = Fixture::new(256);
    let ctx = egui::Context::default();
    let builtin = f.rt.builtin[0].take();
    let original = f.rt.decks[1].audio.clone().unwrap();
    f.app.lib_filter = "Drums".into();
    f.app.load_sel(1);
    f.rt.process(&mut [0.0; 2]);
    let output = frame(&mut f.app, &ctx, 1.0, vec![]);
    assert_visible(
        &output,
        "load failed on B: Drums (session): built-in media is unavailable",
    );
    assert_visible(&output, "Retry");
    assert!(Arc::ptr_eq(
        f.rt.decks[1].audio.as_ref().unwrap(),
        &original
    ));
    f.rt.builtin[0] = builtin;
    let output = frame(&mut f.app, &ctx, 1.1, vec![]);
    click(&mut f.app, &ctx, label_center(&output, "Retry"), 1.2);
    f.rt.process(&mut [0.0; 2]);
    assert_visible(
        &frame(&mut f.app, &ctx, 1.3, vec![]),
        "loaded Drums (session) → B",
    );
    f.app.lib_filter = "Harmony".into();
    f.app.load_sel(0);
    drop(f.rt);
    let output = frame(&mut f.app, &ctx, 2.0, vec![]);
    assert_visible(
        &output,
        "load failed on A: Harmony (session): audio engine disconnected",
    );
    assert!(!visible(&output, "loaded Harmony"));
}

#[test]
fn decode_finishing_after_an_independent_renderer_eject_cannot_reload_the_deck() {
    let files=Files::new();let path=files.wave("pending.wav");let mut f=Fixture::new(256);
    f.app.load_file(0,path.clone(),"pending file");
    f.decoder_jobs.recv_timeout(Duration::from_secs(3)).unwrap();
    f.app.engine.send(Command::DeckUnload{deck:0}).unwrap();
    f.rt.process(&mut [0.0;256]);
    assert!(f.rt.decks[0].audio.is_none());
    f.decoder_results.send((0,crate::engine::dsp::decode_audio(&path))).unwrap();f.poll_loads();
    let receipt=f.app.loads[0].as_ref().unwrap().receipt.as_ref().unwrap().clone();
    let counts=crate::engine::test_alloc::measure(||f.rt.process(&mut [0.0;256]));
    assert_eq!((counts.allocations,counts.frees),(0,0));
    assert_eq!(receipt.state(),crate::engine::load_receipt::State::Superseded);
    assert!(f.rt.decks[0].audio.is_none());f.app.poll_load_receipts();
    assert!(matches!(f.app.loads[0].as_ref().unwrap().phase,Phase::Superseded));
}

#[test]
fn superseded_queued_load_never_revives_success_and_old_completion_cannot_replace_newer_media() {
    let files = Files::new();
    let path = files.wave("old.wav");
    let mut f = Fixture::new(256);
    f.app.load_file(0, path.clone(), "old file");
    f.decoder_jobs.recv_timeout(Duration::from_secs(3)).unwrap();
    f.decoder_results
        .send((0, crate::engine::dsp::decode_audio(&path)))
        .unwrap();
    f.poll_loads();
    let old = f.rt.cmd_rx.try_recv().unwrap();
    f.app.lib_filter = "Harmony".into();
    f.app.load_sel(0);
    let new = f.rt.cmd_rx.try_recv().unwrap();
    f.rt.apply(new);
    f.rt.apply(old); // adversarial delayed stale completion
    assert_eq!(f.rt.decks[0].title, "Harmony (session)");
    let output = frame(&mut f.app, &egui::Context::default(), 1.0, vec![]);
    assert_visible(&output, "loaded Harmony (session) → A");
    assert!(!visible(&output, "old file"));
}

#[test]
fn rejected_or_still_queued_unload_does_not_claim_current_media_was_replaced() {
    let files = Files::new();
    let path = files.wave("current.wav");
    for decoded in [false, true] {
        let mut f = Fixture::new(32);
        let name = if decoded {
            f.app.load_file(0, path.clone(), "current file");
            f.decoder_jobs.recv_timeout(Duration::from_secs(3)).unwrap();
            f.decoder_results
                .send((0, crate::engine::dsp::decode_audio(&path)))
                .unwrap();
            f.poll_loads();
            "current file"
        } else {
            f.app.lib_filter = "Drums".into();
            f.app.load_sel(0);
            "Drums (session)"
        };
        f.rt.process(&mut [0.0; 2]);
        let ctx = egui::Context::default();
        assert_visible(
            &frame(&mut f.app, &ctx, 1.0, vec![]),
            &format!("loaded {name} → A"),
        );
        let audio = f.rt.decks[0].audio.clone().unwrap();
        while f.app.engine.send(Command::Tap(Instant::now())).is_ok() {}
        assert!(!f.app.submit(Command::DeckUnload { deck: 0 }));
        let output = frame(&mut f.app, &ctx, 2.0, vec![]);
        assert_visible(&output, &format!("loaded {name} → A"));
        assert!(!visible(&output, "replaced or unloaded"));
        assert!(Arc::ptr_eq(f.rt.decks[0].audio.as_ref().unwrap(), &audio));
        f.rt.process(&mut [0.0; 2]);
        assert!(f.app.submit(Command::DeckUnload { deck: 0 }));
        let output = frame(&mut f.app, &ctx, 3.0, vec![]);
        assert_visible(&output, &format!("loaded {name} → A"));
        assert!(!visible(&output, "replaced or unloaded"));
        f.rt.process(&mut [0.0; 2]);
        assert!(f.rt.decks[0].audio.is_none());
        let output = frame(&mut f.app, &ctx, 4.0, vec![]);
        assert_visible(&output, "replaced or unloaded");
        assert!(!visible(&output, &format!("loaded {name}")));
    }
}

#[test]
fn cancellation_and_application_have_one_winner_at_controlled_interleavings() {
    use crate::engine::load_receipt::State;
    use std::sync::mpsc;
    let files = Files::new();
    let path = files.wave("racing.wav");
    for decoded in [false, true] {
        for after_claim in [false, true] {
            let mut f = Fixture::new(48);
            if decoded {
                f.app.load_file(0, path.clone(), "racing file");
                f.decoder_jobs.recv_timeout(Duration::from_secs(3)).unwrap();
                f.decoder_results
                    .send((0, crate::engine::dsp::decode_audio(&path)))
                    .unwrap();
                f.poll_loads();
            } else {
                f.app.lib_filter = "Harmony".into();
                f.app.load_sel(0);
            }
            let before = f.rt.decks[0].audio.clone().unwrap();
            let command = f.rt.cmd_rx.try_recv().unwrap();
            let receipt = f.app.loads[0].as_ref().unwrap().receipt.clone().unwrap();
            let (paused, at_pause) = mpsc::channel();
            let (resume, resumed) = mpsc::channel();
            f.rt.load_test_hooks[usize::from(after_claim)] = Some(Box::new(move || {
                paused.send(()).unwrap();
                resumed.recv().unwrap();
            }));
            let mut rt = f.rt;
            let renderer = std::thread::spawn(move || {
                rt.apply(command);
                rt
            });
            at_pause.recv_timeout(Duration::from_secs(3)).unwrap();
            assert_eq!(
                receipt.state(),
                if after_claim {
                    State::Applying
                } else {
                    State::Pending
                }
            );
            assert!(f.app.submit(Command::DeckUnload { deck: 0 }));
            let output = frame(&mut f.app, &egui::Context::default(), 1.0, vec![]);
            if after_claim {
                assert_visible(&output, "queued ");
                assert!(!visible(&output, "replaced or unloaded"));
                assert_eq!(receipt.state(), State::Applying);
            } else {
                assert_eq!(receipt.state(), State::Superseded);
            }
            resume.send(()).unwrap();
            let mut rt = renderer.join().unwrap();
            if after_claim {
                assert_eq!(receipt.state(), State::Current);
                assert!(!Arc::ptr_eq(rt.decks[0].audio.as_ref().unwrap(), &before));
            } else {
                assert_eq!(receipt.state(), State::Superseded);
                assert!(Arc::ptr_eq(rt.decks[0].audio.as_ref().unwrap(), &before));
            }
            rt.process(&mut [0.0; 2]); // execute the later unload
            assert!(rt.decks[0].audio.is_none());
            assert_eq!(receipt.state(), State::Superseded);
            assert!(!visible(
                &frame(&mut f.app, &egui::Context::default(), 2.0, vec![]),
                "loaded racing file"
            ));
        }
    }
}
