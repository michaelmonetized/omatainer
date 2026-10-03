use super::*;
use crate::ui::test_support::{label_center, Fixture};

fn fixture() -> Fixture {
    let mut fixture = Fixture::new(256);
    fixture.rt.apply(Command::DeckLoadLock {
        deck: 0,
        enabled: true,
    });
    fixture.rt.apply(Command::DeckPlay { deck: 0 });
    sync(&mut fixture);
    fixture
}
fn sync(fixture: &mut Fixture) {
    let deadline = Instant::now() + std::time::Duration::from_secs(3);
    loop {
        fixture.rt.publish();
        let snapshot = fixture.app.engine.snapshot();
        if snapshot.decks[0].load_locked == fixture.rt.decks[0].load_locked
            && snapshot.decks[0].playing == fixture.rt.decks[0].playing
            && snapshot.decks[0].title == fixture.rt.decks[0].title
        {
            fixture.app.snap = snapshot;
            return;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
}
fn frame(
    ctx: &egui::Context,
    app: &mut App,
    events: Vec<egui::Event>,
    time: f64,
) -> egui::FullOutput {
    ctx.run(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1440.0, 900.0))),
            time: Some(time),
            events,
            ..Default::default()
        },
        |ctx| {
            app.deck_load_confirmation(ctx);
        },
    )
}
fn click(ctx: &egui::Context, app: &mut App, label: &str, time: &mut f64) {
    *time += 0.1;
    let output = frame(ctx, app, vec![], *time);
    let pos = label_center(&output, label);
    for pressed in [true, false] {
        *time += 0.1;
        frame(
            ctx,
            app,
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
            *time,
        );
    }
}
fn selection() -> Selection {
    Selection {
        title: "Harmony (session)".into(),
        source: LibSource::Builtin(BuiltinStem::Harmony),
    }
}
#[test]
fn actual_native_checkbox_has_deck_identity_and_toggles_only_its_renderer_lock() {
    use egui::accesskit::{Action, ActionRequest, Role};
    let mut fixture = Fixture::new(256);
    sync(&mut fixture);
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let mut time = 1.0;
    let mut run = |fixture: &mut Fixture, events| {
        time += 0.1;
        ctx.run(egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1440.0, 900.0))),
            time: Some(time), events, ..Default::default()
        }, |ctx|fixture.app.update_frame(ctx))
    };
    let _ = run(&mut fixture, vec![]);
    let output = run(&mut fixture, vec![]);
    let (id, node) = output.platform_output.accesskit_update.as_ref().unwrap().nodes.iter()
        .find(|(_, node)|node.label() == Some("Deck A: Lock playing deck")).unwrap();
    assert_eq!(node.role(), Role::CheckBox);
    let id = *id;
    for expected in [true, false] {
        let _ = run(&mut fixture, vec![egui::Event::AccessKitActionRequest(ActionRequest { target: id, action: Action::Click, data: None })]);
        fixture.rt.process(&mut [0.0; 512]);
        sync(&mut fixture);
        assert_eq!(fixture.rt.decks[0].load_locked, expected);
        assert!(!fixture.rt.decks[1].load_locked);
        let _ = run(&mut fixture, vec![]);
    }
}
#[test]
fn actual_mouse_keyboard_drop_and_factory_midi_load_routes_keep_the_locked_deck_playing() {
    use crate::ui::test_support::{crate_frame, click as crate_click};
    let mut fixture = fixture();
    let original = fixture.rt.decks[0].audio.clone().unwrap();
    fixture.app.lib_filter = "Harmony".into();
    let ctx = egui::Context::default();
    let output = crate_frame(&ctx, &mut fixture.app, 0.0, vec![]);
    crate_click(&ctx, &mut fixture.app, label_center(&output, "→ A"), 0.1);
    assert!(fixture.app.deck_load_review.is_some());
    fixture.app.deck_load_review = None;
    let output = crate_frame(&ctx, &mut fixture.app, 0.2, vec![]);
    crate_click(&ctx, &mut fixture.app, label_center(&output, "Harmony (session)"), 0.3);
    crate_frame(&ctx, &mut fixture.app, 0.4, vec![egui::Event::Key {
        key: Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE,
    }]);
    assert!(fixture.app.deck_load_review.is_some());
    fixture.app.deck_load_review = None;
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/audio/tone.flac");
    let _ = ctx.run(egui::RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1440.0, 900.0))),
        time: Some(1.0), events: vec![egui::Event::PointerMoved(Pos2::new(100.0, 100.0))],
        dropped_files: vec![egui::DroppedFile { path: Some(path), ..Default::default() }],
        ..Default::default()
    }, |ctx|fixture.app.update_frame(ctx));
    assert!(fixture.app.deck_load_review.is_some());
    assert!(fixture.decoder_jobs.try_recv().is_err());
    fixture.app.deck_load_review = None;
    fixture.app.publish_library_selection();
    fixture.app.engine.midi.receive_for_test(&fixture.app.engine.cmd, 41, "Pioneer DDJ-FLX4", &[0x90, 0x02, 0x7f]);
    assert_eq!(fixture.app.engine.cmd.ui_request_stats().pending, 0);
    fixture.rt.process(&mut [0.0; 512]);
    assert!(Arc::ptr_eq(&original, fixture.rt.decks[0].audio.as_ref().unwrap()));
    assert!(fixture.rt.decks[0].playing);
    assert!(fixture.rt.decks[0].pos > 0.0);
    assert!(fixture.decoder_jobs.try_recv().is_err());
}

#[test]
fn approved_file_decode_keeps_old_audio_until_ready_and_failure_preserves_it() {
    use crate::engine::decode::DecodeFailure;
    let mut fixture = fixture();
    let original = fixture.rt.decks[0].audio.clone().unwrap();
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/audio/tone.flac");
    let selected = Selection { title: "Own fixture tone".into(), source: LibSource::File(path.clone()) };
    let ctx = egui::Context::default();
    let mut time = 1.0;
    for success in [false, true] {
        fixture.app.load_source(0, Some(&selected));
        frame(&ctx, &mut fixture.app, vec![], time);
        click(&ctx, &mut fixture.app, "I intend to replace this playing track", &mut time);
        click(&ctx, &mut fixture.app, "Confirm deck replacement", &mut time);
        let (deck, captured) = fixture.decoder_jobs.recv_timeout(std::time::Duration::from_secs(3)).unwrap();
        assert_eq!((deck, captured), (0, path.clone()));
        fixture.rt.process(&mut [0.0; 512]);
        assert!(Arc::ptr_eq(&original, fixture.rt.decks[0].audio.as_ref().unwrap()));
        assert!(fixture.rt.decks[0].playing);
        let result = if success { crate::engine::dsp::decode_audio(&path) } else { Err(DecodeFailure { kind: crate::engine::decode::DecodeFailureKind::Unsupported, stage: crate::engine::decode::DecodeStage::Open, diagnostics: Default::default(), detail: "Controlled unsupported fixture".into() }) };
        fixture.decoder_results.send((0, result)).unwrap();
        fixture.poll_loads();
        if success {
            fixture.rt.apply(fixture.rt.cmd_rx.try_recv().unwrap());
            assert!(!fixture.rt.decks[0].playing);
            assert!(!Arc::ptr_eq(&original, fixture.rt.decks[0].audio.as_ref().unwrap()));
        } else {
            assert!(fixture.rt.cmd_rx.is_empty());
            assert!(Arc::ptr_eq(&original, fixture.rt.decks[0].audio.as_ref().unwrap()));
            assert!(fixture.rt.decks[0].playing);
        }
    }
}
#[test]
fn native_review_cancel_and_confirmation_preserve_current_audio_until_renderer_application() {
    let mut fixture = fixture();
    let original = fixture.rt.decks[0].audio.clone().unwrap();
    let ctx = egui::Context::default();
    let mut time = 1.0;
    fixture.app.load_source(0, Some(&selection()));
    assert!(fixture.rt.cmd_rx.is_empty());
    frame(&ctx, &mut fixture.app, vec![], time);
    click(&ctx, &mut fixture.app, "Keep playing track", &mut time);
    assert!(fixture.app.deck_load_review.is_none());
    assert!(fixture.rt.cmd_rx.is_empty());
    fixture.app.load_source(0, Some(&selection()));
    frame(&ctx, &mut fixture.app, vec![], time);
    click(
        &ctx,
        &mut fixture.app,
        "I intend to replace this playing track",
        &mut time,
    );
    click(
        &ctx,
        &mut fixture.app,
        "Confirm deck replacement",
        &mut time,
    );
    assert!(fixture.app.deck_load_review.is_none());
    assert!(Arc::ptr_eq(
        &original,
        fixture.rt.decks[0].audio.as_ref().unwrap()
    ));
    assert!(fixture.rt.decks[0].playing);
    let command = fixture.rt.cmd_rx.try_recv().unwrap();
    fixture.rt.apply(command);
    fixture.app.poll_load_receipts();
    assert!(matches!(
        fixture.app.loads[0].as_ref().unwrap().phase,
        Phase::Loaded
    ));
    assert_eq!(fixture.rt.decks[0].title, "Harmony (session)");
    assert!(!fixture.rt.decks[0].playing);
    assert!(fixture.rt.decks[0].load_locked);
}
#[test]
fn stale_native_review_and_global_performance_mode_never_authorize_another_track() {
    let mut fixture = fixture();
    let ctx = egui::Context::default();
    fixture.app.load_source(0, Some(&selection()));
    assert!(fixture.app.deck_load_review.is_some());
    fixture.rt.apply(Command::DeckPlay { deck: 0 });
    fixture.rt.apply(Command::LoadBuiltin { deck: 0, stem: 1 });
    sync(&mut fixture);
    frame(&ctx, &mut fixture.app, vec![], 1.0);
    assert!(fixture.app.deck_load_review.is_none());
    assert!(fixture.rt.cmd_rx.is_empty());
    fixture.rt.apply(Command::DeckPlay { deck: 0 });
    sync(&mut fixture);
    fixture
        .app
        .engine
        .cmd
        .performance()
        .set_enabled(true)
        .unwrap();
    fixture.app.load_source(0, Some(&selection()));
    assert!(fixture.app.deck_load_review.is_none());
    assert!(fixture.rt.cmd_rx.is_empty());
}
#[test]
fn native_eject_requires_review_and_exact_media_identity() {
    let mut fixture = fixture();
    assert!(fixture.app.review_locked_eject(0));
    let ctx = egui::Context::default();
    let mut time = 1.0;
    frame(&ctx, &mut fixture.app, vec![], time);
    click(
        &ctx,
        &mut fixture.app,
        "I intend to replace this playing track",
        &mut time,
    );
    click(
        &ctx,
        &mut fixture.app,
        "Confirm deck replacement",
        &mut time,
    );
    fixture.rt.apply(fixture.rt.cmd_rx.try_recv().unwrap());
    assert!(fixture.rt.decks[0].audio.is_none());
    assert!(!fixture.rt.decks[0].playing);
}
