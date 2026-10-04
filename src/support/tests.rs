use super::*;
use std::fs;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

static NEXT: AtomicU64 = AtomicU64::new(0);
struct Temp(std::path::PathBuf);
impl Temp {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "omatainer-support-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::DirBuilder::new().mode(0o700).create(&path).unwrap();
        Self(path)
    }
    fn root(&self) -> std::path::PathBuf {
        self.0.join("support")
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn uncancelled() -> AtomicBool {
    AtomicBool::new(false)
}
fn sample(elapsed_ms: u64) -> Sample {
    Sample {
        elapsed_ms,
        audio: Audio::default(),
        commands: Commands::default(),
        midi: [0; 8],
        ui_update_ns: None,
    }
}

#[test]
fn report_is_bounded_strict_and_allowlisted_without_any_user_payload() {
    let mut report = Report::new(Id::digest(b"owned run"), false, 1);
    assert!(report
        .build
        .as_ref()
        .unwrap()
        .dependencies
        .iter()
        .any(|d| d.name == "cpal"));
    for i in 0..700 {
        report.record(Event {
            elapsed_ms: i,
            code: if i % 2 == 0 {
                Code::ParserRejected
            } else {
                Code::PluginHostingUnavailable
            },
            failure: Some(FailureClass::Invalid),
            repeats: 0,
        });
        report.sample(sample(i));
    }
    assert_eq!(
        (report.events.len(), report.samples.len()),
        (MAX_EVENTS, MAX_SAMPLES)
    );
    assert_eq!((report.dropped_events, report.dropped_samples), (188, 580));
    let bytes = storage::encode(&report).unwrap();
    assert!(bytes.len() < MAX_BYTES);
    let mut value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    value["samples"][0]["audio"]["device_name"] = "private device".into();
    assert!(serde_json::from_value::<Report>(value).is_err());
    report.audio_plugin_host_available = true;
    assert!(storage::encode(&report).is_err());
    report.audio_plugin_host_available = false;
    report.backend_runtime_version = Some("private driver error and path".into());
    assert!(storage::encode(&report).is_err());
    report.backend_runtime_version = None;
    report
        .build
        .as_mut()
        .unwrap()
        .dependencies
        .push(Dependency {
            name: "/private/media".into(),
            version: "secret".into(),
        });
    assert!(storage::encode(&report).is_err());
    // Synthetic plugin-domain evidence is serialization coverage only: this
    // application has no audio plugin host and cannot catch such plugin crashes.
}

#[test]
fn inspect_export_reopen_cancel_and_create_new_preserve_existing_evidence() {
    let temp = Temp::new();
    let cancel = uncancelled();
    let run = storage::Run::begin(&temp.root(), false).unwrap();
    let mut report = run.report();
    report.record(Event {
        elapsed_ms: 2,
        code: Code::ProjectWriteFailed,
        failure: Some(FailureClass::NoSpace),
        repeats: 0,
    });
    assert_eq!(
        run.persist(&report, &cancel).unwrap(),
        storage::Published::Durable
    );
    let path = temp.0.join("report.omasupport.json");
    assert_eq!(
        storage::export(&path, &report, &cancel).unwrap(),
        storage::Published::Durable
    );
    let original = fs::read(&path).unwrap();
    assert_eq!(storage::reopen(&path, &cancel).unwrap().run, run.id);
    assert!(storage::export(&path, &report, &cancel).is_err());
    assert_eq!(fs::read(&path).unwrap(), original);
    let cancelled = temp.0.join("cancelled.json");
    assert!(matches!(
        storage::export(&cancelled, &report, &AtomicBool::new(true)),
        Err(Error::Cancelled)
    ));
    assert!(!cancelled.exists());
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(
        storage::discover(&temp.root(), &cancel)
            .unwrap()
            .skipped_active,
        1
    );
    run.mark_clean().unwrap();
    drop(run);
    let discovered = storage::discover(&temp.root(), &cancel).unwrap();
    assert_eq!(discovered.previous[0].report.exit, Exit::Clean);
    assert_eq!(
        discovered.previous[0].report.events[0].failure,
        Some(FailureClass::NoSpace)
    );
}

#[test]
fn private_paths_symlinks_fifo_corruption_and_storage_cap_fail_without_clobbering() {
    let temp = Temp::new();
    let cancel = uncancelled();
    let public = temp.0.join("public");
    fs::create_dir(&public).unwrap();
    fs::set_permissions(&public, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(storage::Run::begin(&public, false).is_err());
    let link = temp.0.join("alias");
    std::os::unix::fs::symlink(&public, &link).unwrap();
    assert!(storage::Run::begin(&link, false).is_err());
    let run = storage::Run::begin(&temp.root(), false).unwrap();
    let report = run.report();
    run.persist(&report, &cancel).unwrap();
    let dir = temp.root().join(format!("run-{}", run.id.hex()));
    let original = fs::read(dir.join("report.json")).unwrap();
    let spare = dir.join(format!(
        ".pending-{}",
        Id::digest(b"interrupted staging").hex()
    ));
    let file = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(&spare)
        .unwrap();
    file.set_len(MAX_STORAGE_BYTES).unwrap();
    assert!(run.persist(&report, &cancel).is_err());
    assert_eq!(fs::read(dir.join("report.json")).unwrap(), original);
    fs::remove_file(spare).unwrap();
    let corrupt = temp.0.join("bad.json");
    fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(&corrupt)
        .unwrap();
    fs::write(&corrupt, b"{untrusted partial file").unwrap();
    assert!(storage::reopen(&corrupt, &cancel).is_err());
    let pipe = temp.0.join("pipe");
    let name = std::ffi::CString::new(pipe.as_os_str().as_encoded_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
    let before = Instant::now();
    assert!(storage::reopen(&pipe, &cancel).is_err());
    assert!(before.elapsed() < Duration::from_secs(1));
    drop(run);
    assert_eq!(
        storage::discover(&temp.root(), &cancel).unwrap().previous[0]
            .report
            .exit,
        Exit::Unclean
    );
}

#[test]
fn interrupted_retention_resumes_but_never_deletes_unknown_files() {
    let temp = Temp::new();
    for _ in 0..MAX_RUNS + 3 {
        let run = storage::Run::begin(&temp.root(), false).unwrap();
        run.mark_clean().unwrap();
    }
    let cancel = uncancelled();
    assert_eq!(
        storage::discover(&temp.root(), &cancel)
            .unwrap()
            .previous
            .len(),
        MAX_RUNS
    );
    let old = fs::read_dir(temp.root())
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| p.file_name().unwrap().to_str().unwrap().starts_with("run-"))
        .unwrap();
    let id = old
        .file_name()
        .unwrap()
        .to_str()
        .unwrap()
        .trim_start_matches("run-");
    let retired = temp.root().join(format!(".retired-{id}"));
    fs::rename(&old, &retired).unwrap();
    fs::remove_file(retired.join("marker")).unwrap();
    let run = storage::Run::begin(&temp.root(), true).unwrap();
    assert!(!retired.exists());
    drop(run);
    let unknown = temp
        .root()
        .join(format!(".retired-{}", Id::digest(b"unknown").hex()));
    fs::DirBuilder::new().mode(0o700).create(&unknown).unwrap();
    fs::write(unknown.join("user-owned"), b"preserve").unwrap();
    assert!(storage::Run::begin(&temp.root(), false).is_err());
    assert_eq!(fs::read(unknown.join("user-owned")).unwrap(), b"preserve");
}

#[test]
fn missing_report_preserves_marker_truth_without_guessing_the_crashed_build() {
    let temp = Temp::new();
    let run = storage::Run::begin(&temp.root(), true).unwrap();
    let id = run.id;
    drop(run);
    let found = storage::discover(&temp.root(), &uncancelled()).unwrap();
    assert_eq!(found.incomplete, 1);
    assert_eq!(found.previous[0].report.run, id);
    assert!(found.previous[0].report.safe_mode);
    assert!(found.previous[0].report.build.is_none());
    assert_eq!(found.previous[0].report.exit, Exit::Unclean);
}

#[test]
fn native_panic_and_abrupt_termination_leave_exact_marker_classifications() {
    for (mode, expected) in [
        ("clean", Exit::Clean),
        ("startup_failed", Exit::StartupFailed),
        ("panic", Exit::ObservedRustPanic),
        ("kill", Exit::Unclean),
    ] {
        let temp = Temp::new();
        let ready = temp.0.join("ready");
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "support::tests::crash_child",
                "--ignored",
                "--nocapture",
            ])
            .env("OMATAINER_SUPPORT_CHILD_ROOT", temp.root())
            .env("OMATAINER_SUPPORT_CHILD_MODE", mode)
            .env("OMATAINER_SUPPORT_CHILD_READY", &ready)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(8);
        while !ready.exists() && Instant::now() < deadline {
            if let Some(status) = child.try_wait().unwrap() {
                assert!(
                    ready.exists(),
                    "child exited before marker was durable: {status}"
                );
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(
            ready.exists(),
            "child did not finish private marker startup"
        );
        if mode == "kill" {
            child.kill().unwrap();
        }
        let status = child.wait().unwrap();
        assert_eq!(
            status.success(),
            mode == "clean" || mode == "startup_failed"
        );
        let found = storage::discover(&temp.root(), &uncancelled()).unwrap();
        assert_eq!(found.previous.len(), 1);
        let report = &found.previous[0].report;
        assert_eq!(report.exit, expected);
        assert_eq!(report.recovery[0].sequence, 1);
        let reference = &report.recovery[0];
        let candidate = crate::recovery::lookup_exact(
            &temp.0.join("recovery"),
            reference.session,
            reference.epoch,
            reference.sequence,
            &uncancelled(),
        )
        .unwrap()
        .unwrap();
        assert_eq!(candidate.metadata.revision, reference.revision);
        let recovered =
            crate::recovery::recover::<serde_json::Value>(&candidate, &uncancelled()).unwrap();
        assert_eq!(recovered.bundle.state["tempo"], 127);
        let json = String::from_utf8(storage::encode(report).unwrap()).unwrap();
        assert!(!json.contains("panic payload secret"));
        assert!(!json.contains("private project title"));
        assert!(!json.contains("/private/project.omat"));
    }
}

#[test]
#[ignore = "guarded child entry point; parent test launches private process with core dumps disabled"]
fn crash_child() {
    let Ok(root) = std::env::var("OMATAINER_SUPPORT_CHILD_ROOT") else {
        return;
    };
    let limit = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    assert_eq!(unsafe { libc::setrlimit(libc::RLIMIT_CORE, &limit) }, 0);
    let run = storage::Run::begin(Path::new(&root), true).unwrap();
    run.install_panic_hook();
    let mut recovery =
        crate::recovery::Store::open(&Path::new(&root).parent().unwrap().join("recovery")).unwrap();
    let metadata = crate::recovery::RecordMeta {
        epoch: 12,
        revision: 9,
        view_revision: 3,
        saved_path: Some("/private/project.omat".into()),
        captured_unix_ms: 1,
    };
    let commit = recovery
        .append(
            &crate::project_file::Bundle {
                state: serde_json::json!({"title":"private project title","tempo":127}),
                media: Vec::new(),
            },
            metadata,
            &crate::recovery::Config::default(),
            &uncancelled(),
        )
        .unwrap();
    let mut report = run.report();
    report.recovery.push(RecoveryRef {
        session: crate::recovery::session_digest(recovery.session_id()),
        epoch: 12,
        sequence: commit.sequence,
        revision: 9,
        view_revision: 3,
        captured_unix_ms: 1,
        committed_unix_ms: commit.committed_unix_ms,
    });
    run.persist(&report, &uncancelled()).unwrap();
    fs::write(
        std::env::var_os("OMATAINER_SUPPORT_CHILD_READY").unwrap(),
        b"ready",
    )
    .unwrap();
    match std::env::var("OMATAINER_SUPPORT_CHILD_MODE")
        .unwrap()
        .as_str()
    {
        "clean" => run.mark_clean().unwrap(),
        "startup_failed" => run.mark_startup_failed().unwrap(),
        "panic" => {
            extern "C" fn panic_abort() {
                panic!("panic payload secret /private/media");
            }
            panic_abort();
        }
        "kill" => loop {
            std::thread::sleep(Duration::from_secs(1));
        },
        _ => panic!("unknown fixture mode"),
    }
}

#[test]
fn exact_recovery_reference_never_substitutes_newer_or_different_epoch_records() {
    let temp = Temp::new();
    let root = temp.0.join("recovery");
    let mut store = crate::recovery::Store::open(&root).unwrap();
    let digest = crate::recovery::session_digest(store.session_id());
    for revision in [1, 2] {
        store
            .append(
                &crate::project_file::Bundle {
                    state: revision,
                    media: Vec::new(),
                },
                crate::recovery::RecordMeta {
                    epoch: 11,
                    revision,
                    view_revision: 0,
                    saved_path: None,
                    captured_unix_ms: revision,
                },
                &crate::recovery::Config::default(),
                &uncancelled(),
            )
            .unwrap();
    }
    assert!(crate::recovery::lookup_exact(&root, digest, 11, 1, &uncancelled()).is_err());
    drop(store);
    assert_eq!(
        crate::recovery::discover(&root, &uncancelled())
            .unwrap()
            .candidates[0]
            .sequence,
        2
    );
    let exact = crate::recovery::lookup_exact(&root, digest, 11, 1, &uncancelled())
        .unwrap()
        .unwrap();
    assert_eq!(
        crate::recovery::recover::<u64>(&exact, &uncancelled())
            .unwrap()
            .bundle
            .state,
        1
    );
    assert!(
        crate::recovery::lookup_exact(&root, digest, 11, 3, &uncancelled())
            .unwrap()
            .is_none()
    );
    assert!(
        crate::recovery::lookup_exact(&root, digest, 12, 1, &uncancelled())
            .unwrap()
            .is_none()
    );
    assert!(
        crate::recovery::lookup_exact(&root, [0; 32], 11, 1, &uncancelled())
            .unwrap()
            .is_none()
    );
    assert!(matches!(
        crate::recovery::lookup_exact(&root, digest, 11, 1, &AtomicBool::new(true)),
        Err(crate::recovery::Error::Cancelled)
    ));
}

#[test]
fn actual_safe_owner_keeps_save_open_real_and_rejects_audio_midi_and_playback() {
    use crate::engine::{Command, Engine};
    let temp = Temp::new();
    let engine = Engine::start_safe().unwrap();
    assert!(engine.safe_mode());
    assert!(!engine.midi.connections_available());
    assert!(engine.midi.policy_status().is_none());
    assert!(engine.output_info().is_none());
    assert!(engine.cmd.send(Command::Play).is_err());
    assert!(engine.cmd.send(Command::DeckPlay { deck: 0 }).is_err());
    assert!(engine
        .midi
        .configure_inputs(crate::engine::midi::InputPolicy::All)
        .is_err());
    let audio = engine.audio_handle().unwrap();
    assert!(audio
        .switch(
            crate::preferences::Audio::default(),
            std::sync::Arc::new(uncancelled())
        )
        .is_err());
    assert_eq!(
        audio.status().phase,
        crate::engine::audio::owner::Phase::Offline
    );
    let captured = engine.project.capture(&uncancelled()).unwrap();
    assert!(captured.state.decks.iter().all(|deck| deck.audio.is_none()));
    assert!(captured
        .state
        .tracks
        .iter()
        .all(|track| track.clips.iter().all(|clip| clip.notes.is_empty())));
    assert!(!engine.snapshot().playing);
    let original_revision = captured.revision;
    let file = temp.0.join("safe-save.omat");
    crate::project_file::save(
        &file,
        &crate::project_file::Bundle {
            state: captured.state,
            media: captured.media,
        },
        crate::project_file::Overwrite::Never,
        &crate::project_file::Limits::default(),
        &uncancelled(),
    )
    .unwrap();
    let bundle: crate::project_file::Bundle<crate::engine::project::State> =
        crate::project_file::load(
            &file,
            &crate::project_file::Limits::default(),
            &uncancelled(),
        )
        .unwrap();
    let prepared =
        crate::engine::project::Prepared::from_state(bundle.state, bundle.media, engine.sr())
            .unwrap();
    let applied = engine
        .project
        .install(prepared, original_revision, &uncancelled())
        .unwrap();
    assert!(applied.revision > original_revision);
    let _captured = engine.project.capture(&uncancelled()).unwrap();
    assert!(!engine.snapshot().playing);
    assert_eq!(engine.cmd.audio_metrics().callbacks, 0);
}

#[test]
fn finished_run_releases_raw_descriptors_but_waits_for_a_live_panic_hook_owner() {
    let files = Temp::new();
    let run = storage::Run::begin(&files.root(), true).unwrap();
    let retained = run.retained_marker_for_test();
    let hook_owner = run.retained_hook_owner_for_test();
    run.persist(&run.report(), &uncancelled()).unwrap();
    run.mark_clean().unwrap();
    assert_eq!(
        storage::discover(&files.root(), &uncancelled())
            .unwrap()
            .skipped_active,
        1
    );
    drop(run);
    assert_eq!(
        storage::discover(&files.root(), &uncancelled())
            .unwrap()
            .skipped_active,
        1
    );
    drop(hook_owner);
    let discovered = storage::discover(&files.root(), &uncancelled()).unwrap();
    assert_eq!(discovered.skipped_active, 0);
    assert_eq!(discovered.previous.len(), 1);
    assert_eq!(discovered.previous[0].report.exit, Exit::Clean);
    drop(retained);
}

#[test]
fn exact_recovery_lookup_and_later_restore_reject_a_replaced_assets_directory() {
    let temp = Temp::new();
    let root = temp.0.join("recovery");
    let mut store = crate::recovery::Store::open(&root).unwrap();
    let session = store.session_id().to_owned();
    let digest = crate::recovery::session_digest(&session);
    let media = std::sync::Arc::new(crate::engine::dsp::Sample {
        name: "fixture".into(),
        path: String::new(),
        sr: 48000,
        ch: 1,
        bpm: 120.0,
        data: vec![0.25; 128],
        peaks: std::sync::Arc::new(vec![[0.25; 3]]),
    });
    store
        .append(
            &crate::project_file::Bundle {
                state: 1u64,
                media: vec![media],
            },
            crate::recovery::RecordMeta {
                epoch: 7,
                revision: 1,
                view_revision: 0,
                saved_path: None,
                captured_unix_ms: 1,
            },
            &crate::recovery::Config::default(),
            &uncancelled(),
        )
        .unwrap();
    drop(store);
    let candidate = crate::recovery::lookup_exact(&root, digest, 7, 1, &uncancelled())
        .unwrap()
        .unwrap();
    let assets = root.join(session).join("assets");
    let external = temp.0.join("external-assets");
    fs::rename(&assets, &external).unwrap();
    std::os::unix::fs::symlink(&external, &assets).unwrap();
    assert!(crate::recovery::lookup_exact(&root, digest, 7, 1, &uncancelled()).is_err());
    assert!(crate::recovery::recover::<u64>(&candidate, &uncancelled()).is_err());
    assert_eq!(fs::read_dir(&external).unwrap().count(), 1);
}

#[test]
fn active_collection_and_saturated_observations_add_no_callback_heap_work() {
    let files = Temp::new();
    let session = worker::Session::start(&files.root(), false).unwrap();
    let (mut engine, rt) = crate::engine::Engine::headless_for_test(48000, 256);
    engine.cmd.attach_support(session.port.clone());
    let mut callback = crate::engine::audio::OutputCallback::new(rt, 2);
    let mut block = [0.0f32; 256];
    for _ in 0..128 {
        callback.render(&mut block);
    }
    for _ in 0..10000 {
        session
            .port
            .event(Code::ParserRejected, Some(FailureClass::Invalid));
    }
    let counts = crate::engine::test_alloc::measure(|| {
        for _ in 0..1024 {
            callback.render(&mut block);
        }
    });
    assert_eq!((counts.allocations, counts.frees), (0, 0));
    assert!(block.iter().all(|v| v.is_finite()));
    assert!(session.finish(Exit::Clean, Duration::from_secs(2)));
    assert!(session.view().report.events.len() <= MAX_EVENTS);
}

#[test]
fn routing_distinguishes_default_intent_resolved_backend_and_actual_fixed_channel_mapping() {
    let requested = Route::requested(&crate::preferences::Audio::default());
    assert_eq!(requested.backend, Backend::SystemDefault);
    assert_eq!(requested.device, DeviceChoice::SystemDefault);
    assert_eq!(requested.output_route, OutputRoute::Unresolved);
    for (channels, expected) in [
        (1, OutputRoute::MonoSumToOne),
        (2, OutputRoute::MainLeftRightToOneTwo),
        (4, OutputRoute::MainLeftRightToOneTwoOthersSilent),
    ] {
        let plan = crate::engine::audio::config::Plan {
            graph: Default::default(),
            backend: "ALSA".into(),
            device: "never export this device".into(),
            channels,
            rate: 48000,
            format: cpal::SampleFormat::F32,
            buffer: Some(128),
            warning: None,
        };
        let active = Route::active(&plan);
        assert_eq!(active.device, DeviceChoice::ResolvedRedacted);
        assert_eq!(active.backend, Backend::Alsa);
        assert_eq!(active.output_route, expected);
        assert!(!serde_json::to_string(&active)
            .unwrap()
            .contains("never export"));
    }
}

#[test]
fn offline_contract_rejects_credential_fields_in_preferences_and_support() {
    let preferences =
        crate::preferences::Preferences::defaults(std::path::Path::new("/unused-private-root"));
    let mut value = serde_json::to_value(&preferences).unwrap();
    value["access_token"] = "OMATAINER_OFFLINE_CREDENTIAL_SENTINEL".into();
    assert!(crate::preferences::storage::decode(&serde_json::to_vec(&value).unwrap()).is_err());
    let report = Report::new(Id::digest(b"offline-contract"), true, 1);
    let mut value = serde_json::to_value(&report).unwrap();
    value["credentials"] = serde_json::json!({"token":"OMATAINER_OFFLINE_CREDENTIAL_SENTINEL"});
    assert!(serde_json::from_value::<Report>(value).is_err());
    let encoded = storage::encode(&report).unwrap();
    assert!(!String::from_utf8(encoded)
        .unwrap()
        .contains("OMATAINER_OFFLINE_CREDENTIAL_SENTINEL"));
}
