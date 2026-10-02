//! Fresh-process UI persistence under the explicit child-only network guard.
use super::*;
use sha2::{Digest, Sha256};
use std::io::Write;

fn hash(sample: &crate::engine::dsp::Sample) -> String {
    let mut digest = Sha256::new();
    digest.update(sample.sr.to_le_bytes());
    digest.update(sample.ch.to_le_bytes());
    for value in &sample.data {
        digest.update(value.to_bits().to_le_bytes());
    }
    format!("{:x}", digest.finalize())
}
fn snapshot(g: &Gui) -> serde_json::Value {
    let bank = &g.rt.sampler_banks[3];
    serde_json::json!({
        "deck_pcm":hash(g.rt.decks[0].audio.as_ref().unwrap()),
        "bank_pcm":hash(bank.data.audio[0].as_ref().unwrap()),
        "bank_id":bank.id,"bank_name":bank.data.settings.name,
        "bank_controls":bank.data.settings.slots[0].controls,
        "range":bank.data.ranges[0],"notes":g.rt.tracks[0].clips[0].notes,
    })
}
fn project(g: &mut Gui, label: &str, button: &str, path: &std::path::Path) {
    g.click("Project");
    g.click(label);
    g.text("Project file path", path.to_str().unwrap());
    g.click(button);
    let end = Instant::now() + Duration::from_secs(10);
    loop {
        g.frame(vec![]);
        let (current, dirty, busy) = g.app.project_help_state();
        if !busy && !dirty && current.as_deref() == Some(path) {
            break;
        }
        assert!(
            Instant::now() < end,
            "project UI did not publish requested clean document"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[test]
#[ignore = "scripts/check-offline.py launches two isolated, network-denied processes"]
fn document_child() {
    assert_eq!(std::env::var("OMATAINER_OFFLINE_CHILD").as_deref(), Ok("1"));
    // Assert denial inside the actual application/test process, after exec.
    let fd = unsafe { libc::socket(libc::AF_INET, libc::SOCK_STREAM, 0) };
    assert_eq!(fd, -1);
    assert_eq!(
        std::io::Error::last_os_error().raw_os_error(),
        Some(libc::EPERM)
    );
    let root = PathBuf::from(std::env::var_os("OMATAINER_OFFLINE_DIR").unwrap());
    let mode = std::env::var("OMATAINER_OFFLINE_STAGE").unwrap();
    let files = std::mem::ManuallyDrop::new(Files(root.clone()));
    let mut g = Gui::new(&files);
    let path = root.join("offline.omat");
    if mode == "prepare" {
        let source = files.wave();
        g.add_source(source.clone());
        g.click("Load selected crate item to deck A");
        g.wait(|g| {
            matches!(
                g.app.loads[0].as_ref().map(|s| &s.phase),
                Some(Phase::Loaded)
            )
        });
        assert_eq!(
            g.rt.decks[0].audio.as_ref().unwrap().path,
            source.to_str().unwrap()
        );
        g.open();
        g.create();
        g.assign();
        g.action(
            "Sampler editor: Slot gain",
            AtAction::SetValue,
            Some(ActionData::NumericValue(0.4)),
        );
        g.text("Sampler editor: Start seconds", "0.05");
        g.text(
            "Sampler editor: End seconds (empty means source end)",
            "0.2",
        );
        g.click("Sampler editor: Prepare slot preview");
        g.ready();
        g.apply();
        g.wait(|g| g.app.sampler_editor.awaiting_snapshot.is_none());
        g.key(Key::Escape, Default::default());
        g.app.send(Command::SetNotes {
            track: 0,
            scene: 0,
            notes: vec![crate::engine::MidiNote {
                id: crate::engine::midi_edit::NoteId::new(), muted: false,
                pitch: 64,
                start: 0.25,
                len: 1.25,
                vel: 99,
            }],
        });
        g.frame(vec![]);
        project(&mut g, "Save project as…", "Save", &path);
        let bundle = crate::project_file::load::<crate::ui::project::Document>(
            &path,
            &crate::project_file::Limits::default(),
            &std::sync::atomic::AtomicBool::new(false),
        )
        .unwrap();
        let original = serde_json::to_value(&bundle.state).unwrap();
        for field in ["credentials", "access_token"] {
            let mut value = original.clone();
            value[field] = "offline-credential-sentinel".into();
            assert!(serde_json::from_value::<crate::ui::project::Document>(value).is_err());
            let mut value = original.clone();
            value["engine"][field] = "offline-credential-sentinel".into();
            assert!(serde_json::from_value::<crate::ui::project::Document>(value).is_err());
        }
    } else {
        assert_eq!(mode, "reopen");
        assert!(
            !root.join("sample.wav").exists(),
            "original media must be absent in fresh process"
        );
        project(&mut g, "Open project…", "Open", &path);
        assert!(!g.rt.playing && g.rt.decks.iter().all(|d| !d.playing));
        assert_eq!(g.rt.sampler_banks.len(), 4);
        let previous: serde_json::Value =
            serde_json::from_slice(&std::fs::read(root.join("prepare.json")).unwrap()).unwrap();
        assert_eq!(
            snapshot(&g),
            previous["state"],
            "embedded PCM, bank controls and notes survive source removal"
        );
        // The reusable definition is a reference; this project is embedded PCM.
        g.click("Deck A: Platter play or pause");
        let mut audio = [0.0f32; 512];
        g.rt.process(&mut audio);
        assert!(audio.iter().all(|v| v.is_finite()) && audio.iter().any(|v| v.abs() > 1e-5));
        g.click("Deck A: Platter play or pause");
        g.open();
        g.click("Sampler editor: Audition selected slot");
        g.rt.process(&mut audio);
        assert!(audio.iter().all(|v| v.is_finite()) && audio.iter().any(|v| v.abs() > 1e-5));
        g.click("Sampler editor: Stop audition");
        g.key(Key::Escape, Default::default());
    }
    let report = serde_json::json!({"schema":1,"stage":mode,"state":snapshot(&g),
        "embedded_manifest":serde_json::from_str::<serde_json::Value>(crate::licenses::MANIFEST).unwrap(),
        "direct_ipv4_socket":"EPERM","physical_devices":false,"debug_assertions":cfg!(debug_assertions)});
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(root.join(format!("{mode}.json")))
        .unwrap();
    file.write_all(&serde_json::to_vec_pretty(&report).unwrap())
        .unwrap();
    file.sync_all().unwrap();
}
