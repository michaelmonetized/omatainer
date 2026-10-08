use super::*;
use crate::{
    ableton,
    dj_library::{self, Purpose, Request},
    engine::{self, media_analysis::tests::Files},
};
const ORIGINAL_MANIFEST: &str = r#"Ableton#04I
FolderConfigData
{
 String PackUniqueID = "org.omatainer.fixture/東京";
 String PackDisplayName = "Owned sample fixture";
 String PackVendor = "Omatainer test";
 Int PackMinorVersion = 2;
 Int PackMajorVersion = 1;
 Int PackRevision = 7;
 Int MinSoftwareProductId = 4;
}
"#;
fn installation(files: &Files) -> (PathBuf, Candidate, Pack) {
    let root = files.0.join("User 東京 Library");
    let folder = root.join("Ableton Folder Info");
    std::fs::create_dir_all(&folder).unwrap();
    let path = folder.join("Properties.cfg");
    std::fs::write(&path, ORIGINAL_MANIFEST).unwrap();
    let candidate = probe(&path, &Snapshot::discover().unwrap())
        .unwrap()
        .unwrap();
    let pack = review(&candidate, &AtomicBool::new(false)).unwrap();
    (root, candidate, pack)
}
#[test]
fn declared_pack_metadata_unicode_readonly_identity_mutation_and_corruption_are_distinct() {
    let files = Files::new();
    let (root, candidate, _) = installation(&files);
    let before = std::fs::read(&candidate.path).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&candidate.path, std::fs::Permissions::from_mode(0o444)).unwrap();
    let candidate = probe(&candidate.path, &Snapshot::discover().unwrap())
        .unwrap()
        .unwrap();
    let pack = review(&candidate, &AtomicBool::new(false)).unwrap();
    assert_eq!(pack.root(&AtomicBool::new(false)).unwrap(), root);
    assert_eq!(pack.validate().unwrap(), "org.omatainer.fixture/東京");
    assert_eq!(std::fs::read(&candidate.path).unwrap(), before);
    assert!(review(&candidate, &AtomicBool::new(true))
        .unwrap_err()
        .contains("cancelled"));
    let mut corrupt = pack.clone();
    corrupt.fields.insert("PackRevision".into(), "8".into());
    assert!(corrupt.validate().is_err());
    std::fs::set_permissions(&candidate.path, std::fs::Permissions::from_mode(0o600)).unwrap();
    std::fs::write(&candidate.path, ORIGINAL_MANIFEST.replace("= 7;", "= 8;")).unwrap();
    assert!(pack.root(&AtomicBool::new(false)).is_err());
    assert!(review(&candidate, &AtomicBool::new(false)).is_err());
    drop(pack);
}
#[test]
fn manifest_grammar_refuses_external_scripts_duplicate_fields_missing_identity_and_bounds() {
    assert!(manifest::parse(ORIGINAL_MANIFEST).is_ok());
    for invalid in [
        ORIGINAL_MANIFEST.replace("Ableton#04I", "Ableton#99I"),
        ORIGINAL_MANIFEST.replace("String PackVendor", "Exec PackVendor"),
        ORIGINAL_MANIFEST.replace(
            "Int PackRevision = 7;",
            "Int PackRevision = 7; Int PackRevision = 7;",
        ),
        ORIGINAL_MANIFEST.replace("PackUniqueID", "UnknownIdentity"),
        format!("{ORIGINAL_MANIFEST} execute()"),
        ORIGINAL_MANIFEST.replace("= 7;", "= -7;"),
    ] {
        assert!(manifest::parse(&invalid).is_err(), "{invalid}");
    }
    assert!(manifest::parse(&"x".repeat(MAX_MANIFEST + 1)).is_err());
}
#[test]
fn producer_pages_find_user_content_keep_archive_limits_and_reject_cross_purpose_cursors() {
    let files = Files::new();
    let (_, candidate, _) = installation(&files);
    for name in [
        "My preset.adg",
        "Effect.adv",
        "MIDI.alc",
        "License.alp",
        "Sample.wav",
        "Source.als",
        "Original.vstpreset",
    ] {
        std::fs::write(
            files.0.join(name),
            b"source bytes retained only after review",
        )
        .unwrap();
    }
    std::os::unix::fs::symlink(&files.0, files.0.join("loop")).unwrap();
    let request = Request {
        purpose: Purpose::Producer,
        roots: vec![files.0.clone()],
        all_mounts: false,
        cursor: None,
    };
    let page = dj_library::scan(request.clone(), &|| true).unwrap();
    assert!(page.complete);
    assert_eq!(page.candidates.len(), 8);
    assert!(page.candidates.iter().any(|c| c.source == candidate.source));
    assert!(page
        .candidates
        .iter()
        .any(|c| c.format.starts_with("Pack archive") && !c.reviewable));
    assert!(page.notices.iter().any(|n| n.state.contains("Symlink")));
    assert!(dj_library::scan(request, &|| false).is_err());
    for i in 0..10010 {
        std::fs::write(files.0.join(format!("entry-{i}")), b"").unwrap();
    }
    let page = dj_library::scan(
        Request {
            purpose: Purpose::Producer,
            roots: vec![files.0.clone()],
            all_mounts: false,
            cursor: None,
        },
        &|| true,
    )
    .unwrap();
    assert!(!page.complete);
    assert!(dj_library::scan(
        Request {
            purpose: Purpose::Dj,
            roots: vec![],
            all_mounts: false,
            cursor: page.cursor.clone()
        },
        &|| true
    )
    .is_err());
    let next = dj_library::scan(
        Request {
            purpose: Purpose::Producer,
            roots: vec![files.0.clone()],
            all_mounts: false,
            cursor: page.cursor,
        },
        &|| true,
    )
    .unwrap();
    assert!(next.complete);
}
#[test]
fn owned_nested_preset_preserves_original_macros_ranges_order_state_and_native_reopen() {
    let files = Files::new();
    let xml = r#"<Ableton MajorVersion="5" MinorVersion="11.0_433" Creator="Original user fixture"><GroupDevicePreset><Device><InstrumentGroupDevice Id="9"><MacroControls><Macro Id="2"><Manual Value="0.25"/></Macro></MacroControls><Branches><InstrumentBranch Id="4"><KeyRange><Min Value="36"/><Max Value="84"/></KeyRange><VelocityRange><Min Value="12"/><Max Value="110"/></VelocityRange><DeviceChain><Devices><Operator Id="7"><On><Manual Value="false"/></On><State>0102</State></Operator><Eq8 Id="8"><On><Manual Value="true"/></On></Eq8></Devices></DeviceChain></InstrumentBranch></Branches></InstrumentGroupDevice></Device></GroupDevicePreset></Ableton>"#;
    let path = files.0.join("Original.adg");
    std::fs::write(&path, xml).unwrap();
    let draft = ableton::load(&path, &Default::default(), &AtomicBool::new(false)).unwrap();
    let source = &draft.state.migration.as_ref().unwrap().sources[0];
    assert_eq!(source.xml, xml);
    assert_eq!(
        source
            .devices
            .iter()
            .map(|d| d.kind.as_str())
            .collect::<Vec<_>>(),
        vec!["InstrumentGroupDevice", "Operator", "Eq8"]
    );
    assert!(!source.devices[1].enabled);
    assert_eq!(draft.state.tracks.len(), 1);
    assert_eq!(source.tracks[0].role, "MidiTrack");
    draft.state.validate(&draft.media).unwrap();
    let saved = files.0.join("Retained.omatainer");
    crate::project_file::save(
        &saved,
        &crate::project_file::Bundle {
            state: draft.state.clone(),
            media: draft.media.clone(),
        },
        crate::project_file::Overwrite::Never,
        &Default::default(),
        &AtomicBool::new(false),
    )
    .unwrap();
    let reopened = crate::project_file::load::<engine::project::State>(
        &saved,
        &Default::default(),
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(reopened.state.migration.unwrap().sources[0].xml, xml);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), xml);
    let wrong = files.0.join("Disguised.als");
    std::fs::write(&wrong, xml).unwrap();
    assert!(ableton::load(&wrong, &Default::default(), &AtomicBool::new(false)).is_err());
}
#[test]
fn reviewed_pack_reference_embeds_owned_audio_and_retains_device_engine_and_source_proof() {
    let files = Files::new();
    let (root, _, pack) = installation(&files);
    std::fs::create_dir_all(root.join("Samples")).unwrap();
    let sample = root.join("Samples/One.wav");
    std::fs::write(
        &sample,
        engine::media_analysis::tests::wav(4800, 48000, 1, false),
    )
    .unwrap();

    let xml=ableton::tests::document(11).replace("<State><Buffer>","<FileRef><Path Value=\"/old/Pack/Samples/One.wav\"/><RelativePath Value=\"Samples/One.wav\"/><PackId Value=\"org.omatainer.fixture/東京\"/></FileRef><State><Buffer>");
    let path = files.0.join("Pack dependency.als");
    std::fs::write(&path, &xml).unwrap();
    let missing = ableton::load(&path, &Default::default(), &AtomicBool::new(false)).unwrap();
    assert!(matches!(
        missing.state.migration.as_ref().unwrap().sources[0].assets[0].availability,
        ableton::Availability::Missing
    ));
    let draft = ableton::load(
        &path,
        &ableton::Options {
            libraries: vec![pack.clone()],
            ..Default::default()
        },
        &AtomicBool::new(false),
    )
    .unwrap();
    let migration = draft.state.migration.as_ref().unwrap();
    assert_eq!(migration.schema, 2);
    assert_eq!(migration.sources[0].libraries[0].text, ORIGINAL_MANIFEST);
    assert!(matches!(
        migration.sources[0].assets[0].availability,
        ableton::Availability::Embedded
    ));
    assert!(migration.sources[0].assets[0].audio_sha256.is_some());
    assert_eq!(draft.media[1].frames(), 4800);
    assert!(draft.state.tracks[0].synth.offline.is_some());
    ableton::verify_draft(&draft, &AtomicBool::new(false)).unwrap();
    let escaped = xml.replace(
        "Samples/One.wav\"/><PackId",
        "../Samples/One.wav\"/><PackId",
    );
    std::fs::write(&path, escaped).unwrap();
    assert!(ableton::load(
        &path,
        &ableton::Options {
            libraries: vec![pack.clone()],
            ..Default::default()
        },
        &AtomicBool::new(false)
    )
    .is_err());
    let mut old = draft.state.clone();
    old.version = 31;
    assert!(old.validate(&draft.media).is_err());
    std::fs::remove_file(root.join("Ableton Folder Info/Properties.cfg")).unwrap();
    assert!(ableton::verify_draft(&draft, &AtomicBool::new(false)).is_err());
    draft.state.validate(&draft.media).unwrap();
}
#[test]
#[ignore = "requires the actual newly built app executable"]
fn native_pack_worker_checks_real_identity_and_stops_cancelled_or_mutated_reviews() {
    let files = Files::new();
    let (_, candidate, pack) = installation(&files);
    let result = isolated(&candidate, &AtomicBool::new(false)).unwrap();
    assert_eq!(result.sha256, pack.sha256);
    assert!(isolated(&candidate, &AtomicBool::new(true)).is_err());
    std::fs::write(&candidate.path, b"corrupt").unwrap();
    assert!(isolated(&candidate, &AtomicBool::new(false)).is_err());
}

#[test]
#[ignore = "requires the actual app discovery entry point"]
fn native_producer_scan_uses_current_mount_identity_and_keeps_source_bytes_unchanged() {
    let files = Files::new();
    let (_, candidate, _) = installation(&files);
    let path = files.0.join("User.adv");
    let bytes = b"owned preset candidate";
    std::fs::write(&path, bytes).unwrap();
    let request = Request {
        purpose: Purpose::Producer,
        roots: vec![files.0.clone()],
        all_mounts: false,
        cursor: None,
    };
    let executable = std::env::var_os("OMATAINER_TEST_BIN").unwrap();
    let mut command = std::process::Command::new(&executable);
    command.arg("dj-discover-worker");
    let result: Result<dj_library::Page, String> =
        crate::filesystem_worker::invoke(command, &request, &[], &|| true, Duration::from_secs(10))
            .unwrap();
    let page = result.unwrap();
    assert!(page.complete);
    assert_eq!(page.candidates.len(), 2);
    assert!(page.candidates.iter().any(|p| p.source == candidate.source));
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    let mut command = std::process::Command::new(executable);
    command.arg("dj-discover-worker");
    assert!(
        crate::filesystem_worker::invoke::<_, Result<dj_library::Page, String>>(
            command,
            &request,
            &[],
            &|| false,
            Duration::from_secs(10)
        )
        .is_err()
    );
}
#[test]
#[ignore = "requires the original native SDK fixture and actual app worker"]
fn original_vst3_effect_user_preset_restores_exact_class_and_opaque_state() {
    let (instrument, _) = engine::audio::routing::plugin_tests::fixture(true);
    let (effect, saved) = engine::audio::routing::plugin_tests::fixture(false);
    let set = ableton::plugins::tests::contract_set(&instrument, &effect);
    let root = crate::interchange_xml::parse_text(set.as_bytes(), &|| true).unwrap();
    let devices = root.one("LiveSet").unwrap().one("Tracks").unwrap().children[0]
        .one("DeviceChain")
        .unwrap()
        .one("DeviceChain")
        .unwrap()
        .one("Devices")
        .unwrap();
    assert_eq!(devices.children.len(), 2);
    let start = set.find("<PluginDevice Id=\"8\"").unwrap();
    let end = start + set[start..].find("</PluginDevice>").unwrap() + "</PluginDevice>".len();
    let xml = format!(
        "<Ableton MajorVersion=\"5\" MinorVersion=\"11.0_433\">{}</Ableton>",
        &set[start..end]
    );
    let files = Files::new();
    let path = files.0.join("Owned VST3.adv");
    std::fs::write(&path, &xml).unwrap();
    let cancel = AtomicBool::new(false);
    let draft = ableton::process::review(&path, &Default::default(), false, &cancel).unwrap();
    let restored = ableton::process::edit(
        &draft,
        ableton::process::Edit::Relink {
            source: 0,
            device: 0,
            binary: saved.binary,
            class: effect,
            reviewed: false,
        },
        &cancel,
    )
    .unwrap();
    let graph = restored.state.routing.as_ref().unwrap();
    assert_eq!(graph.plugins.len(), 1);
    let source = &restored.state.migration.as_ref().unwrap().sources[0];
    assert_eq!(source.xml, xml);
    assert!(source.devices[0].resolution.is_some());
    assert_eq!(
        &graph.plugins[0].saved.state[28..36],
        0.5f64.to_le_bytes()
    );
    restored.state.validate(&restored.media).unwrap();
}
