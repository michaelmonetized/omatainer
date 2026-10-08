use super::*;
#[test]
fn protection_blocks_migration_worker_admission() {
    let mut fixture = crate::ui::test_support::Fixture::new(256);
    fixture
        .app
        .engine
        .cmd
        .performance()
        .set_enabled(true)
        .unwrap();
    fixture.app.ableton.path = "/absent.als".into();
    fixture.app.ableton.start(&fixture.app.engine, false);
    assert!(!fixture.app.ableton.busy());
    assert!(fixture
        .app
        .ableton
        .message
        .to_lowercase()
        .contains("protection"));
}

fn frame(
    ctx: &egui::Context,
    fixture: &mut crate::ui::test_support::Fixture,
    events: Vec<egui::Event>,
) -> Vec<(egui::accesskit::NodeId, egui::accesskit::Node)> {
    fixture.rt.process(&mut [0.; 256]);
    fixture.rt.publish_for_test();
    fixture.app.ableton.poll();
    fixture.app.snap = fixture.app.engine.snapshot();
    fixture.app.poll_projects(ctx);
    fixture.app.confirm_project_snapshot();
    let output = ctx.run(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1600., 1400.))),
            focused: true,
            events,
            ..Default::default()
        },
        |ctx| {
            fixture.app.project_toolbar(ctx);
            fixture.app.ableton_ui(ctx);
        },
    );
    output.platform_output.accesskit_update.unwrap().nodes
}
fn click(ctx: &egui::Context, fixture: &mut crate::ui::test_support::Fixture, label: &str) {
    let nodes = frame(ctx, fixture, vec![]);
    let node = nodes
        .iter()
        .find(|(_, node)| node.label() == Some(label))
        .unwrap_or_else(|| panic!("Missing actual migration widget {label}"));
    assert!(!node.1.is_disabled(), "{label}");
    frame(
        ctx,
        fixture,
        vec![egui::Event::AccessKitActionRequest(
            egui::accesskit::ActionRequest {
                target: node.0,
                action: egui::accesskit::Action::Click,
                data: None,
            },
        )],
    );
}
fn settle(ctx: &egui::Context, fixture: &mut crate::ui::test_support::Fixture) {
    let deadline = Instant::now() + std::time::Duration::from_secs(10);
    while fixture.app.ableton.busy() {
        frame(ctx, fixture, vec![]);
        assert!(Instant::now() < deadline, "{}", fixture.app.ableton.message);
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
}
#[test]
#[ignore = "Requires a separately built native app with the isolated migration entry point"]
fn actual_review_publish_source_change_cancellation_and_existing_destination_preserve_session() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!(
        "target/validation/ableton-ui-{}",
        crate::sampler_bank::BankId::new().unwrap()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let source = root.join("Original 東京.als");
    std::fs::write(&source, crate::ableton::tests::document(11)).unwrap();
    let output = root.join("Converted.omatainer");
    let mut fixture = crate::ui::test_support::Fixture::new(256);
    let namespace = fixture.rt.session.namespace;
    let original_master = fixture.rt.master;
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    fixture.app.ableton.open = true;
    fixture.app.ableton.path = source.display().to_string();
    fixture.app.ableton.destination = output.display().to_string();
    frame(&ctx, &mut fixture, vec![]);
    frame(&ctx, &mut fixture, vec![]);
    click(&ctx, &mut fixture, "Review Live Set");
    settle(&ctx, &mut fixture);
    assert!(fixture.app.ableton.draft.is_some());
    assert!(!fixture.app.ableton.accepted);
    let publish = frame(&ctx, &mut fixture, vec![])
        .into_iter()
        .find(|(_, node)| node.label() == Some("Publish native project"))
        .unwrap();
    assert!(publish.1.is_disabled());
    click(
        &ctx,
        &mut fixture,
        "I reviewed these playback differences and unresolved dependencies",
    );
    let mut changed = crate::ableton::tests::document(11);
    changed.push_str("<!-- source changed -->");
    std::fs::write(&source, changed).unwrap();
    click(&ctx, &mut fixture, "Publish native project");
    settle(&ctx, &mut fixture);
    assert!(fixture.app.ableton.message.contains("changed after review"));
    assert!(!output.exists());
    click(&ctx, &mut fixture, "Review Live Set");
    settle(&ctx, &mut fixture);
    click(
        &ctx,
        &mut fixture,
        "I reviewed these playback differences and unresolved dependencies",
    );
    click(&ctx, &mut fixture, "Publish native project");
    settle(&ctx, &mut fixture);
    assert_eq!(
        fixture.app.ableton.published.as_deref(),
        Some(output.as_path())
    );
    let saved = std::fs::read(&output).unwrap();
    let bundle = crate::project_file::load::<project::Document>(
        &output,
        &Default::default(),
        &AtomicBool::new(false),
    )
    .unwrap();
    bundle.state.validate().unwrap();
    bundle.state.engine.validate(&bundle.media).unwrap();
    assert!(bundle.state.engine.migration.is_some());
    assert_eq!(fixture.rt.session.namespace, namespace);
    assert_eq!(fixture.rt.master, original_master);
    fixture.app.ableton.published = None;
    fixture.app.ableton.accepted = true;
    fixture.app.ableton.start(&fixture.app.engine, true);
    settle(&ctx, &mut fixture);
    assert_eq!(std::fs::read(&output).unwrap(), saved);
    fixture.app.ableton.start(&fixture.app.engine, false);
    fixture.app.ableton.cancel();
    settle(&ctx, &mut fixture);
    assert!(fixture.app.ableton.draft.is_none());
    assert_eq!(fixture.rt.session.namespace, namespace);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
#[ignore = "requires the independently authored native SDK fixture and a separately qualified isolated worker"]
fn actual_imported_device_relink_buttons_keep_the_running_document_and_publish_original_state() {
    let (instrument, a) = crate::engine::audio::routing::plugin_tests::fixture(true);
    let (effect, _) = crate::engine::audio::routing::plugin_tests::fixture(false);
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!(
        "target/validation/ableton-ui-native-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("User contract.als");
    let text = crate::ableton::plugins::tests::contract_set(&instrument, &effect);
    std::fs::write(&path, &text).unwrap();
    let mut fixture = crate::ui::test_support::Fixture::new(256);
    let namespace = fixture.rt.session.namespace;
    fixture.app.ableton.open = true;
    fixture.app.ableton.path = path.display().to_string();
    fixture.app.ableton.destination = root.join("Restored.omatainer").display().to_string();
    fixture
        .app
        .plugins
        .catalog
        .records
        .push(crate::plugin_host::scanner::Record {
            path: a.binary.bundle.clone(),
            binary: Some(a.binary.clone()),
            classes: vec![instrument.clone(), effect.clone()],
            failure: None,
        });
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    frame(&ctx, &mut fixture, vec![]);
    frame(&ctx, &mut fixture, vec![]);
    click(&ctx, &mut fixture, "Review Live Set");
    settle(&ctx, &mut fixture);
    fixture.app.ableton.device_index = Some(0);
    click(
        &ctx,
        &mut fixture,
        &format!(
            "Restore {} {}",
            instrument.info.name, instrument.info.version
        ),
    );
    settle(&ctx, &mut fixture);
    assert!(fixture
        .app
        .ableton
        .draft
        .as_ref()
        .unwrap()
        .state
        .migration
        .as_ref()
        .unwrap()
        .sources[0]
        .devices[0]
        .resolution
        .is_some());
    assert!(!fixture.app.ableton.accepted);
    fixture.app.ableton.device_index = Some(1);
    click(
        &ctx,
        &mut fixture,
        &format!("Restore {} {}", effect.info.name, effect.info.version),
    );
    settle(&ctx, &mut fixture);
    assert_eq!(
        fixture
            .app
            .ableton
            .draft
            .as_ref()
            .unwrap()
            .state
            .routing
            .as_ref()
            .unwrap()
            .plugins
            .len(),
        2
    );
    assert_eq!(fixture.rt.session.namespace, namespace);
    click(
        &ctx,
        &mut fixture,
        "I reviewed these playback differences and unresolved dependencies",
    );
    click(&ctx, &mut fixture, "Publish native project");
    settle(&ctx, &mut fixture);
    let output = fixture.app.ableton.published.clone().unwrap();
    let reopened = crate::project_file::load::<project::Document>(
        &output,
        &Default::default(),
        &AtomicBool::new(false),
    )
    .unwrap();
    assert!(reopened.state.engine.migration.as_ref().unwrap().sources[0]
        .devices
        .iter()
        .all(|d| d.resolution.is_some()));
    assert_eq!(std::fs::read_to_string(path).unwrap(), text);
    assert_eq!(fixture.rt.session.namespace, namespace);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
#[ignore = "Requires a separately built native app with the isolated migration entry point"]
fn actual_render_attach_restore_and_publication_keep_the_running_set_and_editable_midi() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!(
        "target/validation/ableton-render-ui-{}",
        crate::sampler_bank::BankId::new().unwrap()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let source = root.join("User set.als");
    std::fs::write(&source, crate::ableton::tests::document(11)).unwrap();
    let render = root.join("Original print.wav");
    crate::ableton::renders::tests::wave(&render);
    let mut fixture = crate::ui::test_support::Fixture::new(256);
    let namespace = fixture.rt.session.namespace;
    fixture.app.ableton.open = true;
    fixture.app.ableton.path = source.display().to_string();
    fixture.app.ableton.render_path = render.display().to_string();
    fixture.app.ableton.render_body = 2.;
    fixture.app.ableton.destination = root.join("Printed.omatainer").display().to_string();
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    frame(&ctx, &mut fixture, vec![]);
    frame(&ctx, &mut fixture, vec![]);
    click(&ctx, &mut fixture, "Review Live Set");
    settle(&ctx, &mut fixture);
    click(
        &ctx,
        &mut fixture,
        "The complete mix includes return and master processing/gain",
    );
    click(&ctx, &mut fixture, "Attach aligned render");
    settle(&ctx, &mut fixture);
    assert_eq!(
        fixture
            .app
            .ableton
            .draft
            .as_ref()
            .unwrap()
            .state
            .tracks
            .len(),
        4
    );
    assert!(fixture.app.ableton.render_backup.is_some());
    assert_eq!(fixture.rt.session.namespace, namespace);
    click(&ctx, &mut fixture, "Restore pre-render draft");
    assert_eq!(
        fixture
            .app
            .ableton
            .draft
            .as_ref()
            .unwrap()
            .state
            .tracks
            .len(),
        3
    );
    assert!(fixture
        .app
        .ableton
        .draft
        .as_ref()
        .unwrap()
        .state
        .migration
        .as_ref()
        .unwrap()
        .sources[0]
        .renders
        .is_empty());
    fixture.app.ableton.render_path = root.join("Missing print.wav").display().to_string();
    click(&ctx, &mut fixture, "Attach aligned render");
    settle(&ctx, &mut fixture);
    assert!(fixture.app.ableton.render_backup.is_none());
    assert_eq!(
        fixture
            .app
            .ableton
            .draft
            .as_ref()
            .unwrap()
            .state
            .tracks
            .len(),
        3
    );
    fixture.app.ableton.render_path = render.display().to_string();
    click(&ctx, &mut fixture, "Attach aligned render");
    settle(&ctx, &mut fixture);
    click(
        &ctx,
        &mut fixture,
        "I reviewed these playback differences and unresolved dependencies",
    );
    click(&ctx, &mut fixture, "Publish native project");
    settle(&ctx, &mut fixture);
    let bundle = crate::project_file::load::<project::Document>(
        fixture
            .app
            .ableton
            .published
            .as_ref()
            .expect(&fixture.app.ableton.message),
        &Default::default(),
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(bundle.state.engine.tracks[0].clips[0].notes.len(), 1);
    assert!(bundle.state.engine.tracks[0].mute);
    assert_eq!(
        bundle.state.engine.migration.as_ref().unwrap().sources[0]
            .renders
            .len(),
        1
    );
    assert_eq!(fixture.rt.session.namespace, namespace);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
#[ignore = "Requires a separately built native app with the isolated migration entry point"]
fn actual_saved_archive_review_and_restore_do_not_require_the_original_live_files() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!(
        "target/validation/ableton-archive-ui-{}",
        crate::sampler_bank::BankId::new().unwrap()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let set = root.join("Original.als");
    std::fs::write(&set, crate::ableton::tests::document(11)).unwrap();
    let wave = root.join("Original.wav");
    crate::ableton::renders::tests::wave(&wave);
    let cancel = AtomicBool::new(false);
    let draft = crate::ableton::load(&set, &Default::default(), &cancel).unwrap();
    let printed = crate::ableton::renders::attach(
        draft,
        &wave,
        &crate::ableton::renders::Options {
            target: None,
            start: 0.,
            body_beats: 2.,
            includes_returns_master: true,
        },
        &cancel,
    )
    .unwrap();
    let native = root.join("Saved print.omatainer");
    crate::project_file::save(
        &native,
        &crate::project_file::Bundle {
            state: project::Document {
                engine: printed.state,
                view: Default::default(),
                mapping_schema: project::FACTORY_MAPPING_SCHEMA,
            },
            media: printed.media,
        },
        crate::project_file::Overwrite::Never,
        &Default::default(),
        &cancel,
    )
    .unwrap();
    std::fs::remove_file(set).unwrap();
    std::fs::remove_file(wave).unwrap();
    let mut fixture = crate::ui::test_support::Fixture::new(256);
    let namespace = fixture.rt.session.namespace;
    fixture.app.ableton.open = true;
    fixture.app.ableton.path = native.display().to_string();
    fixture.app.ableton.from = "An irrelevant Live prefix".into();
    fixture.app.ableton.destination = root.join("Restored.omatainer").display().to_string();
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    frame(&ctx, &mut fixture, vec![]);
    frame(&ctx, &mut fixture, vec![]);
    click(
        &ctx,
        &mut fixture,
        "Read a saved Omatainer migration archive",
    );
    click(&ctx, &mut fixture, "Review Live Set");
    settle(&ctx, &mut fixture);
    assert!(fixture
        .app
        .ableton
        .draft
        .as_ref()
        .expect(&fixture.app.ableton.message)
        .reviewed_native
        .is_some());
    click(&ctx, &mut fixture, "Restore saved editable source");
    settle(&ctx, &mut fixture);
    assert_eq!(
        fixture
            .app
            .ableton
            .draft
            .as_ref()
            .unwrap()
            .state
            .tracks
            .len(),
        3,
        "{}",
        fixture.app.ableton.message
    );
    click(
        &ctx,
        &mut fixture,
        "I reviewed these playback differences and unresolved dependencies",
    );
    click(&ctx, &mut fixture, "Publish native project");
    settle(&ctx, &mut fixture);
    assert!(
        fixture.app.ableton.published.is_some(),
        "{}",
        fixture.app.ableton.message
    );
    assert_eq!(fixture.rt.session.namespace, namespace);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
#[ignore = "Requires a separately built native app with the isolated migration entry point"]
fn actual_open_published_migration_preserves_unsaved_cancel_then_installs_stopped_source_state() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!(
        "target/validation/ableton-open-{}",
        crate::sampler_bank::BankId::new().unwrap()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let source = root.join("Source.als");
    std::fs::write(&source, crate::ableton::tests::document(11)).unwrap();
    let output = root.join("Converted.omatainer");
    let mut fixture = crate::ui::test_support::Fixture::new(256);
    let original = fixture.rt.session.namespace;
    fixture.app.ableton.open = true;
    fixture.app.ableton.path = source.display().to_string();
    fixture.app.ableton.destination = output.display().to_string();
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    frame(&ctx, &mut fixture, vec![]);
    frame(&ctx, &mut fixture, vec![]);
    click(&ctx, &mut fixture, "Review Live Set");
    settle(&ctx, &mut fixture);
    click(
        &ctx,
        &mut fixture,
        "I reviewed these playback differences and unresolved dependencies",
    );
    click(&ctx, &mut fixture, "Publish native project");
    settle(&ctx, &mut fixture);
    assert!(
        fixture.app.ableton.published.is_some(),
        "{}",
        fixture.app.ableton.message
    );
    fixture.app.engine.send(Command::Master(0.31)).unwrap();
    fixture.rt.process(&mut [0.; 256]);
    click(&ctx, &mut fixture, "Open imported project");
    let end = Instant::now() + std::time::Duration::from_secs(15);
    while !frame(&ctx, &mut fixture, vec![])
        .iter()
        .any(|(_, node)| node.label() == Some("Discard changes"))
    {
        assert!(Instant::now() < end);
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    click(&ctx, &mut fixture, "Cancel");
    assert_eq!(fixture.rt.session.namespace, original);
    assert_eq!(fixture.rt.master, 0.31);
    click(&ctx, &mut fixture, "Open imported project");
    while !frame(&ctx, &mut fixture, vec![])
        .iter()
        .any(|(_, node)| node.label() == Some("Discard changes"))
    {
        assert!(Instant::now() < end);
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    click(&ctx, &mut fixture, "Discard changes");
    while fixture.app.project_pending_for_test() {
        frame(&ctx, &mut fixture, vec![]);
        assert!(
            Instant::now() < end,
            "{:?}",
            fixture.app.project_result_for_test()
        );
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    assert_ne!(fixture.rt.session.namespace, original);
    let saved = crate::ableton::tests::capture(&fixture.app.engine, &mut fixture.rt);
    assert_eq!(saved.state.tracks.len(), 3);
    assert!(!fixture.app.engine.snapshot().playing);
    assert!(saved.state.migration.is_some());
    assert_eq!(
        fixture.app.recovery_project_path().as_deref(),
        Some(output.as_path())
    );
    std::fs::remove_dir_all(root).unwrap();
}
