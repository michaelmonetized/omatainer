use super::*;
use crate::ui::piano_roll::tests::Gui;
use egui::accesskit::{Action, ActionData};

struct Files(PathBuf);
impl Files {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "omat-portable-ui-{}",
            crate::sampler_bank::BankId::new().unwrap()
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Files {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn settle(gui: &mut Gui) {
    let deadline = Instant::now() + std::time::Duration::from_secs(30);
    loop {
        gui.frame(vec![]);
        if !gui.app.portability.busy() && !gui.app.project_pending_for_test() {
            break;
        }
        assert!(Instant::now() < deadline, "{:?}", gui.app.portability.error);
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    gui.frame(vec![]);
}
fn edit(gui: &mut Gui, label: &str, text: &Path) {
    gui.action(label, Action::Focus, None);
    gui.key(
        Key::A,
        egui::Modifiers {
            ctrl: true,
            command: true,
            ..Default::default()
        },
    );
    gui.frame(vec![egui::Event::Text(text.to_str().unwrap().into())]);
}
fn open(gui: &mut Gui) {
    gui.click_text("Project");
    gui.click_text("Portable project…");
    gui.frame(vec![]);
}
fn inspect(gui: &mut Gui) {
    gui.click("Inspect portable dependencies");
    settle(gui);
    assert!(
        gui.app.portability.error.is_none(),
        "{:?}",
        gui.app.portability.error
    );
}
fn fixture(gui: &mut Gui, files: &Files) -> Arc<crate::engine::dsp::Sample> {
    let originals = files.0.join("originals");
    std::fs::create_dir(&originals).unwrap();
    let original = originals.join("original.wav");
    std::fs::write(
        &original,
        include_bytes!("../../../tests/fixtures/audio/tone-tags.wav"),
    )
    .unwrap();
    let sample = Arc::new(
        crate::engine::decode::decode_sampler_file(
            &original,
            std::fs::File::open(&original).unwrap(),
            1024 * 1024,
            || false,
        )
        .unwrap()
        .sample,
    );
    gui.rt.decks[0].audio = Some(sample.clone());
    let second_path = originals.join("duplicate.wav");
    std::fs::copy(&original, &second_path).unwrap();
    let mut second = (*sample).clone();
    second.path = second_path.to_str().unwrap().into();
    second.name = "Duplicate original".into();
    gui.rt.decks[1].audio = Some(Arc::new(second));
    gui.rt.tracks[2].poly.offline = Some(Arc::new(
        crate::engine::fx::OfflineDevice::new(
            "org.example.missing".into(),
            Some(crate::engine::fx::DeviceState {
                schema: 2,
                data: vec![42, 0, 255],
            }),
        )
        .unwrap(),
    ));
    let clip = &mut gui.rt.tracks[2].clips[7];
    clip.kind = crate::engine::ClipKind::Midi;
    clip.audio = Some(sample.clone());
    clip.notes = vec![crate::engine::MidiNote {
        channel: 0,
        release_vel: 64,
        source_timing: None,
        id: crate::engine::midi_edit::NoteId::new(),
        muted: false,
        pitch: 64,
        start: 0.25,
        len: 1.25,
        vel: 99,
    }];
    clip.lanes = Some(
        crate::engine::midi_data::Lanes::new(
            960,
            3840,
            vec![crate::midi_file::Message {
                tick: 960,
                order: 0,
                bytes: [0xb0, 74, 91],
                length: 3,
            }],
            vec![],
        )
        .unwrap(),
    );
    let mut settings =
        crate::sampler_bank::resident::Settings::empty("Portable user bank".into()).unwrap();
    let hash = dependencies::audio_hash(&sample, &AtomicBool::new(false)).unwrap();
    settings.slots[0].source = Some(crate::sampler_bank::Source::Project {
        source: LibSource::File(original),
        audio_hash: hash,
    });
    settings.slots[0].controls.gain = 0.37;
    settings.slots[1].source = Some(crate::sampler_bank::Source::Project {
        source: LibSource::File(originals.join("missing.wav")),
        audio_hash: [1; 32],
    });
    let mut audio = std::array::from_fn(|_| None);
    audio[0] = Some(sample.clone());
    let mut issues = std::array::from_fn(|_| None);
    issues[1] = Some("Missing original source".into());
    let data =
        crate::sampler_bank::resident::Data::prepare(Arc::new(settings), audio, issues).unwrap();
    let bank =
        crate::engine::sampler::Bank::imported(gui.app.engine.sampler_assets.pin(data).unwrap())
            .unwrap();
    gui.rt.sampler_banks.push(bank);
    gui.rt.publish_for_test();
    gui.frame(vec![]);
    sample
}

#[test]
fn shipped_menu_collect_export_import_and_open_play_without_original_directory() {
    let files = Files::new();
    let mut gui = Gui::new();
    let sample = fixture(&mut gui, &files);
    let notes = gui.rt.tracks[2].clips[7].notes.clone();
    open(&mut gui);
    inspect(&mut gui);
    let review = gui.app.portability.review.as_ref().unwrap();
    assert!(review
        .manifest
        .devices
        .iter()
        .any(|device| device.identifier == "org.example.missing"
            && device.unavailable
            && device.state_schema == Some(2)));
    assert!(review
        .manifest
        .devices
        .iter()
        .any(|device| !device.unavailable && device.placement == "Master effect 1"));
    assert_eq!(review.manifest.unresolved.len(), 1);
    let notices = review.catalog.manifest.entries.len();
    let assets: Vec<_> = review
        .inventory
        .assets
        .iter()
        .enumerate()
        .filter_map(|(index, asset)| asset.source.is_some().then_some(index))
        .collect();
    assert_eq!(assets.len(), 2);
    gui.action(
        "Portable export license-sensitive dependency",
        Action::SetValue,
        Some(ActionData::NumericValue(notices as f64)),
    );
    assert_eq!(gui.app.portability.notice, notices - 1);
    for asset in assets {
        gui.action(
            "Portable export sample",
            Action::SetValue,
            Some(ActionData::NumericValue((asset + 1) as f64)),
        );
        gui.click("Collect this original source");
        assert!(gui.app.portability.selected[asset]);
    }
    let archive = files.0.join("session.ompack");
    edit(&mut gui, "New archive path (.ompack)", &archive);
    gui.click("Export portable project");
    settle(&mut gui);
    assert!(
        gui.app.portability.error.is_none(),
        "{:?}",
        gui.app.portability.error
    );
    assert!(archive.is_file());
    assert_eq!(
        data::preview(&archive, &AtomicBool::new(false))
            .unwrap()
            .entries
            .len(),
        2,
        "two original paths share one encoded source entry"
    );
    assert!(Arc::ptr_eq(
        &sample,
        gui.rt.decks[0].audio.as_ref().unwrap()
    ));
    let isolated = files.0.join("clean-profile");
    std::fs::create_dir(&isolated).unwrap();
    let relocated_archive = isolated.join("session.ompack");
    std::fs::rename(&archive, &relocated_archive).unwrap();
    std::fs::remove_dir_all(files.0.join("originals")).unwrap();
    let mut clean = Gui::new();
    open(&mut clean);
    edit(&mut clean, "Archive to import", &relocated_archive);
    clean.click("Review portable archive");
    settle(&mut clean);
    assert!(
        clean.app.portability.error.is_none(),
        "{:?}",
        clean.app.portability.error
    );
    let destination = isolated.join("restored");
    edit(&mut clean, "New imported project folder", &destination);
    clean.click("Import into new folder");
    settle(&mut clean);
    assert!(
        clean.app.portability.error.is_none(),
        "{:?}",
        clean.app.portability.error
    );
    assert!(destination.join("session.omat").is_file());
    clean.click("Open imported project");
    settle(&mut clean);
    assert_eq!(
        clean.app.project_result_for_test().0,
        Some(destination.join("session.omat"))
    );
    assert_eq!(clean.rt.decks[0].audio.as_ref().unwrap().data, sample.data);
    assert_eq!(clean.rt.tracks[2].clips[7].notes, notes);
    let bank = clean.rt.sampler_banks.last().unwrap().clone();
    assert_eq!(bank.data.settings.slots[0].controls.gain, 0.37);
    assert!(bank.data.audio[1].is_none() && bank.data.issues[1].is_some());
    assert!(
        matches!(&bank.data.settings.slots[0].source, Some(crate::sampler_bank::Source::Project { source: LibSource::File(path), .. }) if path.starts_with(&destination) && path.is_file())
    );
    let retry = crate::sampler_bank::prepare::run(
        crate::sampler_bank::prepare::Request {
            epoch: 1,
            revision: clean.rt.sampler_revision,
            sample_rate: 48000,
            origins: Vec::new(),
            catalog: Arc::new(crate::library::Catalog::default()),
            operation: crate::sampler_bank::prepare::Operation::Change {
                settings: (*bank.data.settings).clone(),
                bank,
                assignment: None,
                retry: Some(0),
                clear: None,
            },
        },
        &clean.app.engine.sampler_assets,
        || false,
    )
    .unwrap();
    assert!(retry.bank.data.issues[0].is_none());
    assert_eq!(retry.bank.data.audio[0].as_ref().unwrap().data, sample.data);
    assert_eq!(
        clean.rt.tracks[2].poly.offline,
        gui.rt.tracks[2].poly.offline
    );
    assert_eq!(
        serde_json::to_value(&clean.rt.tracks[2].clips[7].lanes).unwrap(),
        serde_json::to_value(&gui.rt.tracks[2].clips[7].lanes).unwrap()
    );
    assert!(clean.app.dependencies.origins.iter().any(|origin| matches!(&origin.source, LibSource::File(path) if path.starts_with(&destination) && path.is_file())));
    clean
        .app
        .engine
        .send(Command::DeckPlay { deck: 0 })
        .unwrap();
    let mut output = [0.0; 4096];
    clean.rt.process(&mut output);
    assert!(output.iter().any(|sample| sample.abs() > 0.0001));
    clean
        .app
        .engine
        .send(Command::DeckPlay { deck: 0 })
        .unwrap();
    clean
        .app
        .engine
        .send(Command::LaunchClip { track: 2, scene: 7 })
        .unwrap();
    clean.rt.process(&mut output);
    assert!(output.iter().all(|sample| sample.is_finite()));
    assert!(
        output.iter().any(|sample| sample.abs() > 0.0001),
        "unavailable instrument renders its retained proxy"
    );
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "ui::portability::tests::fresh_process_import_probe",
            "--ignored",
            "--nocapture",
        ])
        .env("OMATAINER_PORTABLE_ARCHIVE", &relocated_archive)
        .env(
            "OMATAINER_PORTABLE_DESTINATION",
            isolated.join("fresh-process"),
        )
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
#[ignore = "private fresh-process entry point exercised by the portable UI round trip"]
fn fresh_process_import_probe() {
    let archive = PathBuf::from(std::env::var_os("OMATAINER_PORTABLE_ARCHIVE").unwrap());
    let destination = PathBuf::from(std::env::var_os("OMATAINER_PORTABLE_DESTINATION").unwrap());
    let mut gui = Gui::new();
    assert!(gui.app.dependencies.origins.is_empty());
    open(&mut gui);
    edit(&mut gui, "Archive to import", &archive);
    gui.click("Review portable archive");
    settle(&mut gui);
    edit(&mut gui, "New imported project folder", &destination);
    gui.click("Import into new folder");
    settle(&mut gui);
    assert!(
        gui.app.portability.error.is_none(),
        "{:?}",
        gui.app.portability.error
    );
    gui.click("Open imported project");
    settle(&mut gui);
    let sample = gui.rt.decks[0].audio.as_ref().unwrap();
    assert!(!Path::new(&sample.path).exists());
    assert!(gui.rt.tracks[2].poly.offline.is_some());
    gui.app.engine.send(Command::DeckPlay { deck: 0 }).unwrap();
    let mut output = [0.0; 4096];
    gui.rt.process(&mut output);
    assert!(output.iter().all(|sample| sample.is_finite()));
    assert!(output.iter().any(|sample| sample.abs() > 0.0001));
}

#[test]
fn stale_review_cancel_and_destination_conflicts_preserve_current_session() {
    let files = Files::new();
    let mut gui = Gui::new();
    let sample = fixture(&mut gui, &files);
    open(&mut gui);
    inspect(&mut gui);
    let archive = files.0.join("new.ompack");
    edit(&mut gui, "New archive path (.ompack)", &archive);
    gui.app.engine.send(Command::SetBpm(130.0)).unwrap();
    gui.frame(vec![]);
    gui.click("Export portable project");
    settle(&mut gui);
    assert!(gui
        .app
        .portability
        .error
        .as_ref()
        .unwrap()
        .contains("changed"));
    assert!(!archive.exists());
    inspect(&mut gui);
    std::fs::write(&archive, b"keep").unwrap();
    gui.click("Export portable project");
    settle(&mut gui);
    assert!(gui
        .app
        .portability
        .error
        .as_ref()
        .unwrap()
        .contains("exists"));
    assert_eq!(std::fs::read(&archive).unwrap(), b"keep");
    gui.render = false;
    gui.click("Inspect portable dependencies");
    assert!(gui.app.portability.busy());
    gui.click("Cancel portable operation");
    gui.render = true;
    settle(&mut gui);
    assert!(gui
        .app
        .portability
        .error
        .as_ref()
        .unwrap()
        .contains("cancelled"));
    assert!(Arc::ptr_eq(
        &sample,
        gui.rt.decks[0].audio.as_ref().unwrap()
    ));
    gui.render = false;
    gui.click("Inspect portable dependencies");
    gui.native_close = true;
    let frame = gui.frame(vec![]);
    assert!(frame.viewport_output[&egui::ViewportId::ROOT]
        .commands
        .iter()
        .any(|command| matches!(command, egui::ViewportCommand::CancelClose)));
    gui.render = true;
    settle(&mut gui);
    assert!(gui
        .app
        .portability
        .error
        .as_ref()
        .unwrap()
        .contains("cancelled"));
}

fn rewrite_manifest(path: &Path, change: impl FnOnce(&mut data::Manifest)) {
    use sha2::{Digest, Sha256};
    let old = std::fs::read(path).unwrap();
    let length = u64::from_le_bytes(old[12..20].try_into().unwrap()) as usize;
    let mut manifest: data::Manifest = serde_json::from_slice(&old[20..20 + length]).unwrap();
    change(&mut manifest);
    let metadata = serde_json::to_vec(&manifest).unwrap();
    let mut bytes = old[..12].to_vec();
    bytes.extend_from_slice(&(metadata.len() as u64).to_le_bytes());
    bytes.extend_from_slice(&metadata);
    bytes.extend_from_slice(&old[20 + length..old.len() - 32]);
    let hash = Sha256::digest(&bytes);
    bytes.extend_from_slice(&hash);
    std::fs::write(path, bytes).unwrap();
}

#[test]
fn reviewed_archive_changes_forged_device_lists_and_import_conflicts_never_install() {
    let files = Files::new();
    let mut gui = Gui::new();
    let sample = fixture(&mut gui, &files);
    open(&mut gui);
    inspect(&mut gui);
    let archive = files.0.join("good.ompack");
    edit(&mut gui, "New archive path (.ompack)", &archive);
    gui.click("Export portable project");
    settle(&mut gui);
    assert!(
        gui.app.portability.error.is_none(),
        "{:?}",
        gui.app.portability.error
    );
    edit(&mut gui, "Archive to import", &archive);
    gui.click("Review portable archive");
    settle(&mut gui);
    let destination = files.0.join("restored");
    edit(&mut gui, "New imported project folder", &destination);
    rewrite_manifest(&archive, |manifest| {
        manifest.application.push_str(" changed")
    });
    gui.click("Import into new folder");
    settle(&mut gui);
    assert!(gui
        .app
        .portability
        .error
        .as_ref()
        .unwrap()
        .contains("changed since review"));
    assert!(!destination.exists());
    rewrite_manifest(&archive, |manifest| {
        manifest.devices[0].identifier = "forged.identifier".into()
    });
    gui.click("Review portable archive");
    settle(&mut gui);
    gui.click("Import into new folder");
    settle(&mut gui);
    assert!(gui
        .app
        .portability
        .error
        .as_ref()
        .unwrap()
        .contains("Device/preset manifest differs"));
    assert!(!destination.exists());
    std::fs::create_dir(&destination).unwrap();
    std::fs::write(destination.join("keep"), b"existing project").unwrap();
    gui.click("Import into new folder");
    settle(&mut gui);
    assert!(gui
        .app
        .portability
        .error
        .as_ref()
        .unwrap()
        .contains("exists"));
    assert_eq!(
        std::fs::read(destination.join("keep")).unwrap(),
        b"existing project"
    );
    assert!(Arc::ptr_eq(
        &sample,
        gui.rt.decks[0].audio.as_ref().unwrap()
    ));
    assert!(gui.app.dependencies.origins.is_empty());
    assert_eq!(
        std::fs::read_dir(&files.0)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry
                .file_name()
                .to_string_lossy()
                .starts_with(".omatainer-portable-"))
            .count(),
        0
    );
}
