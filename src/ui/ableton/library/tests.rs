use super::*;
use crate::ui::ableton::tests::{click, frame, settle};
#[test]
#[ignore = "requires the actual native migration and producer metadata worker"]
fn actual_library_pack_review_selection_preset_import_and_mutated_publication_preserve_session() {
    let files = crate::engine::media_analysis::tests::Files::new();
    let root = files.0.join("User library");
    let info = root.join("Ableton Folder Info");
    std::fs::create_dir_all(&info).unwrap();
    let metadata = info.join("Properties.cfg");
    let manifest="Ableton#04I\nFolderConfigData\n{\nString PackUniqueID = \"org.omatainer.ui-fixture\";\nString PackDisplayName = \"User library\";\nString PackVendor = \"Owned test\";\nInt PackMajorVersion = 1;\nInt PackMinorVersion = 0;\nInt PackRevision = 9;\n}\n";
    std::fs::write(&metadata, manifest).unwrap();
    let snapshot = crate::media_location::Snapshot::discover().unwrap();
    let candidate = crate::producer_library::probe(&metadata, &snapshot)
        .unwrap()
        .unwrap();
    let mut fixture = crate::ui::test_support::Fixture::new(256);
    let namespace = fixture.rt.session.namespace;
    let master = fixture.rt.master;
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    fixture.app.ableton.open = true;
    frame(&ctx, &mut fixture, vec![]);
    frame(&ctx, &mut fixture, vec![]);
    click(&ctx, &mut fixture, "Discover producer libraries");
    fixture.app.ableton.library.discovery.candidates = vec![candidate];
    frame(&ctx, &mut fixture, vec![]);
    frame(&ctx, &mut fixture, vec![]);
    click(&ctx, &mut fixture, "Review Pack manifest");
    let deadline = Instant::now() + std::time::Duration::from_secs(10);
    while fixture.app.ableton.library.events.is_some() {
        frame(&ctx, &mut fixture, vec![]);
        assert!(
            Instant::now() < deadline,
            "{}",
            fixture.app.ableton.library.message
        );
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    assert!(fixture.app.ableton.library.reviewed.is_some());
    click(
        &ctx,
        &mut fixture,
        "I can use this installed content in the imported project",
    );
    click(&ctx, &mut fixture, "Use reviewed Pack for next import");
    assert_eq!(fixture.app.ableton.library.selected.len(), 1);
    let xml = r#"<Ableton MajorVersion="5" MinorVersion="11.0_433" Creator="Owned UI source"><Eq8 Id="3"><On><Manual Value="true"/></On><State><Original Value="0.73"/></State></Eq8></Ableton>"#;
    let path = root.join("Original.adv");
    std::fs::write(&path, xml).unwrap();
    fixture.app.ableton.library.discovery.candidates =
        vec![crate::producer_library::probe(&path, &snapshot)
            .unwrap()
            .unwrap()];
    click(&ctx, &mut fixture, "Review user source");
    settle(&ctx, &mut fixture);
    let draft = fixture.app.ableton.draft.as_ref().unwrap();
    assert_eq!(draft.state.migration.as_ref().unwrap().sources[0].xml, xml);
    assert_eq!(
        draft.state.migration.as_ref().unwrap().sources[0]
            .libraries
            .len(),
        1
    );
    assert_eq!(
        draft.state.migration.as_ref().unwrap().sources[0].devices[0].kind,
        "Eq8"
    );
    assert_eq!(fixture.rt.session.namespace, namespace);
    assert_eq!(fixture.rt.master, master);
    fixture.app.ableton.library.open = false;
    let output = files.0.join("Reviewed.omatainer");
    fixture.app.ableton.destination = output.display().to_string();
    click(
        &ctx,
        &mut fixture,
        "I reviewed these playback differences and unresolved dependencies",
    );
    click(&ctx, &mut fixture, "Publish native project");
    settle(&ctx, &mut fixture);
    assert!(output.is_file());
    assert_eq!(fixture.rt.session.namespace, namespace);
    let saved = crate::project_file::load::<project::Document>(
        &output,
        &Default::default(),
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(
        saved.state.engine.migration.unwrap().sources[0].libraries[0].text,
        manifest
    );
    std::fs::write(&metadata, manifest.replace("= 9;", "= 10;")).unwrap();
    fixture.app.ableton.published = None;
    fixture.app.ableton.destination = files.0.join("Refused.omatainer").display().to_string();
    click(
        &ctx,
        &mut fixture,
        "I reviewed these playback differences and unresolved dependencies",
    );
    click(&ctx, &mut fixture, "Publish native project");
    settle(&ctx, &mut fixture);
    assert!(!files.0.join("Refused.omatainer").exists());
    assert!(fixture.app.ableton.message.contains("changed"));
    assert_eq!(fixture.rt.session.namespace, namespace);
}
#[test]
fn protected_performance_refuses_library_review_without_replacing_retained_selection() {
    let files = crate::engine::media_analysis::tests::Files::new();
    let path = files.0.join("Properties.cfg");
    std::fs::write(&path, b"metadata").unwrap();
    let candidate = Candidate {
        path: path.clone(),
        source: crate::engine::media_source::LibSource::File(path.clone()),
        fingerprint: crate::engine::media_source::FileFingerprint::read(&path).unwrap(),
        format: "Installed Pack manifest".into(),
        reviewable: true,
    };
    let fixture = crate::ui::test_support::Fixture::new(256);
    fixture
        .app
        .engine
        .cmd
        .performance()
        .set_enabled(true)
        .unwrap();
    let mut panel = Panel::default();
    panel.review(candidate, &fixture.app.engine);
    assert!(panel.events.is_none());
    assert!(panel.message.to_lowercase().contains("protection"));
}
