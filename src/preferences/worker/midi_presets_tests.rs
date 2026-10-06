use super::*;
use crate::engine::midi::{
    presets::{Layer, Preset},
    Action, Binding, MsgKind,
};
use std::time::{Duration, Instant};

struct Files(std::path::PathBuf);
impl Files {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "omatainer-midi-presets-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
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

fn preset() -> Preset {
    Preset {
        version: 1,
        name: "Portable".into(),
        device_hint: "Old controller".into(),
        revision_hint: String::new(),
        layer: Layer::FactoryOverlay,
        bindings: vec![Binding {
            kind: MsgKind::Note,
            ch: 0,
            data: 60,
            action: Action::DeckCueHold,
            deck: 0,
            extra: 0,
            relative: None,
        }],
    }
}
fn wait(worker: &mut Worker) -> Event {
    let until = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(event) = worker.poll() {
            return event;
        }
        assert!(Instant::now() < until);
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn midi_preset_worker_exports_private_new_files_and_refuses_overwrites_symlinks_and_invalid_imports(
) {
    let files = Files::new();
    let path = files.0.join("portable.json");
    let mut worker = Worker::with_discovery(files.0.join("preferences.json"), || {
        Err("No audio discovery needed".into())
    })
    .unwrap();
    worker
        .request(Job::ExportMidiPreset {
            path: path.clone(),
            preset: preset(),
        })
        .unwrap();
    assert!(matches!(wait(&mut worker), Event::MidiPresetExported(_)));
    let original = std::fs::read(&path).unwrap();
    assert_eq!(Preset::decode(&original).unwrap(), preset());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    worker
        .request(Job::ExportMidiPreset {
            path: path.clone(),
            preset: preset(),
        })
        .unwrap();
    assert!(matches!(wait(&mut worker), Event::Failed(_)));
    assert_eq!(std::fs::read(&path).unwrap(), original);
    worker
        .request(Job::ImportMidiPreset {
            path: path.clone(),
            token: 42,
        })
        .unwrap();
    assert!(
        matches!(wait(&mut worker),Event::MidiPresetImported {token:42,preset:p} if p==preset())
    );
    let bad = files.0.join("bad.json");
    std::fs::write(&bad, b"{\"version\":1,\"bindings\":[\"invented\"]}").unwrap();
    worker
        .request(Job::ImportMidiPreset {
            path: bad,
            token: 43,
        })
        .unwrap();
    assert!(matches!(wait(&mut worker), Event::Failed(_)));
    #[cfg(unix)]
    {
        let link = files.0.join("link.json");
        std::os::unix::fs::symlink(&path, &link).unwrap();
        worker
            .request(Job::ImportMidiPreset {
                path: link,
                token: 44,
            })
            .unwrap();
        assert!(matches!(wait(&mut worker), Event::Failed(_)));
        assert_eq!(std::fs::read(&path).unwrap(), original);
    }
}

#[test]
fn midi_preset_worker_cancellation_fences_before_read_and_after_reply_delivery() {
    let files = Files::new();
    let path = files.0.join("import.json");
    std::fs::write(&path, serde_json::to_vec(&preset()).unwrap()).unwrap();
    let mut worker =
        Worker::with_discovery(files.0.join("preferences.json"), || Err("unused".into())).unwrap();
    worker.delay.store(50, Ordering::Release);
    worker
        .request(Job::ImportMidiPreset {
            path: path.clone(),
            token: 1,
        })
        .unwrap();
    worker.cancel();
    assert!(matches!(wait(&mut worker), Event::Cancelled));
    worker.delay.store(0, Ordering::Release);
    worker
        .request(Job::ImportMidiPreset { path, token: 2 })
        .unwrap();
    let until = Instant::now() + Duration::from_secs(5);
    while worker.results.is_empty() {
        assert!(Instant::now() < until);
        std::thread::sleep(Duration::from_millis(2));
    }
    worker.cancel();
    assert!(matches!(wait(&mut worker), Event::Cancelled));
    assert!(!worker.busy());
}
