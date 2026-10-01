use super::*;
use std::os::unix::fs::{symlink, PermissionsExt};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct State {
    revision: u64,
    title: String,
    media_ids: Vec<usize>,
    notes: Vec<(u8, f32, f32)>,
}
struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "omatainer-codec-test-{}-{}",
            std::process::id(),
            NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn file(&self) -> PathBuf {
        self.0.join("session.omat")
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn fixture(revision: u64) -> Bundle<State> {
    let special = [
        0.0,
        -0.0,
        f32::from_bits(1),
        -f32::from_bits(1),
        f32::MIN,
        f32::MAX,
        1.25,
        -2.0,
        0.25,
        -0.25,
    ];
    let data = (0..10_000).map(|i| special[i % special.len()]).collect();
    Bundle {
        state: State {
            revision,
            title: "Native α / drums + harmony".into(),
            media_ids: vec![0, 1, 0],
            notes: vec![(60, 0.0, 1.25), (67, 3.75, 9.0)],
        },
        media: vec![
            Arc::new(Sample {
                name: "α sample\nexact".into(),
                path: "/nonexistent/original/संगीत.wav".into(),
                sr: 48000,
                ch: 2,
                data,
                peaks: Arc::new(vec![[-0.0, 0.0, f32::from_bits(1)], [-1.0, 1.0, 0.25]]),
                bpm: -0.0,
            }),
            Arc::new(Sample {
                name: "empty but valid mono".into(),
                path: String::new(),
                sr: 8000,
                ch: 1,
                data: vec![],
                peaks: Arc::new(vec![]),
                bpm: 124.125,
            }),
        ],
    }
}
fn compare(actual: &Bundle<State>, expected: &Bundle<State>) {
    assert_eq!(actual.state, expected.state);
    assert_eq!(actual.media.len(), expected.media.len());
    for (a, e) in actual.media.iter().zip(&expected.media) {
        assert_eq!(
            (&a.name, &a.path, a.sr, a.ch, a.bpm.to_bits()),
            (&e.name, &e.path, e.sr, e.ch, e.bpm.to_bits())
        );
        assert_eq!(
            a.data.iter().map(|f| f.to_bits()).collect::<Vec<_>>(),
            e.data.iter().map(|f| f.to_bits()).collect::<Vec<_>>()
        );
        assert_eq!(
            a.peaks
                .iter()
                .map(|p| p.map(f32::to_bits))
                .collect::<Vec<_>>(),
            e.peaks
                .iter()
                .map(|p| p.map(f32::to_bits))
                .collect::<Vec<_>>()
        );
    }
}
fn write(path: &Path, revision: u64) {
    assert_eq!(
        save(
            path,
            &fixture(revision),
            Overwrite::Replace,
            &Limits::default(),
            &AtomicBool::new(false)
        )
        .unwrap(),
        SaveOutcome::Durable
    );
}

#[test]
fn native_file_roundtrip_preserves_state_complete_media_float_bits_and_crc_reference() {
    let dir = Directory::new();
    let path = dir.file();
    let expected = fixture(1);
    assert_eq!(
        save(
            &path,
            &expected,
            Overwrite::Never,
            &Limits::default(),
            &AtomicBool::new(false)
        )
        .unwrap(),
        SaveOutcome::Durable
    );
    let actual = load::<State>(&path, &Limits::default(), &AtomicBool::new(false)).unwrap();
    compare(&actual, &expected);
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let bytes = fs::read(&path).unwrap();
    assert_eq!(&bytes[..8], &MAGIC);
    assert_eq!(
        u32::from_le_bytes(bytes[8..12].try_into().unwrap()),
        FORMAT_VERSION
    );
    let begin = HEADER_LEN + u64::from_le_bytes(bytes[12..20].try_into().unwrap()) as usize;
    assert_eq!(&bytes[begin..begin + 8], &[0, 0, 0, 0, 0, 0, 0, 128]); // LE +0 and -0
    let mut crc = Crc::new();
    crc.update(b"123456789");
    assert_eq!(crc.finish(), 0xcbf43926);
}

#[test]
fn invalid_media_and_limits_fail_before_publication_and_preserve_existing_project() {
    let dir = Directory::new();
    let path = dir.file();
    write(&path, 1);
    let original = fs::read(&path).unwrap();
    let changes: [fn(&mut Sample); 8] = [
        |s| s.sr = 0,
        |s| s.ch = 0,
        |s| s.ch = MAX_CHANNELS + 1,
        |s| {
            s.data.pop();
        },
        |s| s.data[0] = f32::NAN,
        |s| s.data[0] = f32::INFINITY,
        |s| s.bpm = f32::NEG_INFINITY,
        |s| s.peaks = Arc::new(vec![[f32::NAN, 0.0, 0.0]]),
    ];
    for change in changes {
        let mut bundle = fixture(2);
        change(Arc::make_mut(&mut bundle.media[0]));
        assert!(save(
            &path,
            &bundle,
            Overwrite::Replace,
            &Limits::default(),
            &AtomicBool::new(false)
        )
        .is_err());
        assert_eq!(fs::read(&path).unwrap(), original);
    }
    for limits in [
        Limits {
            max_metadata_bytes: 32,
            ..Limits::default()
        },
        Limits {
            max_media: 1,
            ..Limits::default()
        },
        Limits {
            max_pcm_bytes: 8,
            ..Limits::default()
        },
    ] {
        assert!(save(
            &path,
            &fixture(2),
            Overwrite::Replace,
            &limits,
            &AtomicBool::new(false)
        )
        .is_err());
        assert_eq!(fs::read(&path).unwrap(), original);
        assert!(load::<State>(&path, &limits, &AtomicBool::new(false)).is_err());
    }
    let mut peaks = fixture(2);
    Arc::make_mut(&mut peaks.media[0]).peaks = Arc::new(vec![[0.0; 3]; 100]);
    assert!(save(
        &path,
        &peaks,
        Overwrite::Replace,
        &Limits {
            max_metadata_bytes: 1000,
            ..Limits::default()
        },
        &AtomicBool::new(false)
    )
    .unwrap_err()
    .to_string()
    .contains("peaks"));
    assert_eq!(fs::read_dir(&dir.0).unwrap().count(), 1);
}

fn rewrite_metadata(bytes: &[u8], change: impl FnOnce(&mut serde_json::Value)) -> Vec<u8> {
    let old_len = u64::from_le_bytes(bytes[12..20].try_into().unwrap()) as usize;
    let mut value: serde_json::Value =
        serde_json::from_slice(&bytes[HEADER_LEN..HEADER_LEN + old_len]).unwrap();
    change(&mut value);
    let json = serde_json::to_vec(&value).unwrap();
    let mut out = bytes[..HEADER_LEN].to_vec();
    out[12..20].copy_from_slice(&(json.len() as u64).to_le_bytes());
    out.extend_from_slice(&json);
    out.extend_from_slice(&bytes[HEADER_LEN + old_len..bytes.len() - 4]);
    let mut crc = Crc::new();
    crc.update(&out);
    out.extend_from_slice(&crc.finish().to_le_bytes());
    out
}
#[test]
fn load_rejects_version_length_corruption_nonfinite_data_unknown_fields_and_trailing_bytes_read_only(
) {
    let dir = Directory::new();
    let path = dir.file();
    write(&path, 1);
    let valid = fs::read(&path).unwrap();
    let mut cases = Vec::new();
    let mut bytes = valid.clone();
    bytes[0] ^= 1;
    cases.push(bytes);
    let mut bytes = valid.clone();
    bytes[8..12].copy_from_slice(&2u32.to_le_bytes());
    cases.push(bytes);
    let mut bytes = valid.clone();
    bytes[12..20].copy_from_slice(&u64::MAX.to_le_bytes());
    cases.push(bytes);
    let mut bytes = valid.clone();
    bytes[20..28].copy_from_slice(&u64::MAX.to_le_bytes());
    cases.push(bytes);
    cases.push(valid[..valid.len() - 1].to_vec());
    let mut bytes = valid.clone();
    bytes.push(0);
    cases.push(bytes);
    let mut bytes = valid.clone();
    let n = bytes.len();
    bytes[n - 8] ^= 1;
    cases.push(bytes);
    let begin = HEADER_LEN + u64::from_le_bytes(valid[12..20].try_into().unwrap()) as usize;
    let mut bytes = valid.clone();
    bytes[begin..begin + 4].copy_from_slice(&f32::NAN.to_le_bytes());
    let n = bytes.len();
    let mut crc = Crc::new();
    crc.update(&bytes[..n - 4]);
    bytes[n - 4..].copy_from_slice(&crc.finish().to_le_bytes());
    cases.push(bytes);
    for mutate in [
        (|v: &mut serde_json::Value| v["unexpected"] = true.into()) as fn(&mut serde_json::Value),
        |v| v["state"]["unexpected"] = true.into(),
        |v| v["media"][0]["unexpected"] = true.into(),
        |v| v["media"][0]["channels"] = 0.into(),
        |v| v["media"][0]["sample_rate"] = 0.into(),
        |v| {
            v["media"][0]["channels"] = 1.into();
            v["media"][0]["values"] = u64::MAX.into();
        },
        |v| v["media"][0]["peaks_bits"][0][0] = f32::NAN.to_bits().into(),
        |v| v["media"][0]["bpm_bits"] = f32::INFINITY.to_bits().into(),
        |v| v["media"][0]["values"] = 2.into(),
        |v| v["format_version"] = 2.into(),
    ] {
        cases.push(rewrite_metadata(&valid, mutate));
    }
    for (index, bytes) in cases.into_iter().enumerate() {
        fs::write(&path, &bytes).unwrap();
        assert!(
            load::<State>(&path, &Limits::default(), &AtomicBool::new(false)).is_err(),
            "case {index} accepted"
        );
        assert_eq!(fs::read(&path).unwrap(), bytes, "load mutated case {index}");
    }
}

#[test]
fn precommit_failures_cancellation_and_no_overwrite_preserve_old_file_and_clean_temporary() {
    let dir = Directory::new();
    let path = dir.file();
    write(&path, 1);
    let old = fs::read(&path).unwrap();
    for phase in [
        Phase::MetadataReady,
        Phase::TempCreated,
        Phase::PayloadChunk,
        Phase::BeforeCommit,
    ] {
        let error = save_with_hook(
            &path,
            &fixture(2),
            Overwrite::Replace,
            &Limits::default(),
            &AtomicBool::new(false),
            &mut |current| {
                if current == phase {
                    Err(io::Error::other("injected write failure"))
                } else {
                    Ok(())
                }
            },
        )
        .unwrap_err();
        assert!(matches!(error, Error::Io { .. }));
        assert_eq!(fs::read(&path).unwrap(), old);
        assert_eq!(fs::read_dir(&dir.0).unwrap().count(), 1);
    }
    for phase in [
        Phase::MetadataReady,
        Phase::TempCreated,
        Phase::PayloadChunk,
        Phase::BeforeCommit,
    ] {
        let cancel = AtomicBool::new(false);
        let error = save_with_hook(
            &path,
            &fixture(2),
            Overwrite::Replace,
            &Limits::default(),
            &cancel,
            &mut |current| {
                if current == phase {
                    cancel.store(true, Ordering::Release);
                }
                Ok(())
            },
        )
        .unwrap_err();
        assert!(matches!(error, Error::Cancelled));
        assert_eq!(fs::read(&path).unwrap(), old);
        assert_eq!(fs::read_dir(&dir.0).unwrap().count(), 1);
    }
    assert!(save(
        &path,
        &fixture(2),
        Overwrite::Never,
        &Limits::default(),
        &AtomicBool::new(false)
    )
    .is_err());
    assert_eq!(fs::read(&path).unwrap(), old);
    assert!(matches!(
        load::<State>(&path, &Limits::default(), &AtomicBool::new(true)),
        Err(Error::Cancelled)
    ));
    let absent = dir.0.join("missing").join("file.omat");
    assert!(save(
        &absent,
        &fixture(1),
        Overwrite::Never,
        &Limits::default(),
        &AtomicBool::new(false)
    )
    .is_err());
    assert!(!absent.exists());
}

#[test]
fn publication_wins_late_cancel_and_directory_sync_failure_reports_committed_file() {
    let dir = Directory::new();
    let path = dir.file();
    write(&path, 1);
    let outcome = save_with_hook(
        &path,
        &fixture(2),
        Overwrite::Replace,
        &Limits::default(),
        &AtomicBool::new(false),
        &mut |phase| {
            if phase == Phase::DirectorySync {
                Err(io::Error::other("injected directory sync failure"))
            } else {
                Ok(())
            }
        },
    )
    .unwrap();
    assert!(matches!(
        outcome,
        SaveOutcome::CommittedButDirectorySyncFailed(_)
    ));
    compare(
        &load::<State>(&path, &Limits::default(), &AtomicBool::new(false)).unwrap(),
        &fixture(2),
    );
    let cancel = AtomicBool::new(false);
    let outcome = save_with_hook(
        &path,
        &fixture(3),
        Overwrite::Replace,
        &Limits::default(),
        &cancel,
        &mut |phase| {
            if phase == Phase::AfterCommit {
                cancel.store(true, Ordering::Release);
            }
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(outcome, SaveOutcome::Durable);
    compare(
        &load::<State>(&path, &Limits::default(), &AtomicBool::new(false)).unwrap(),
        &fixture(3),
    );
}

#[test]
fn symlinks_directories_and_fifos_are_rejected_without_mutating_referents_or_waiting() {
    let dir = Directory::new();
    let path = dir.file();
    write(&path, 1);
    let old = fs::read(&path).unwrap();
    let link = dir.0.join("link.omat");
    symlink(&path, &link).unwrap();
    assert!(load::<State>(&link, &Limits::default(), &AtomicBool::new(false)).is_err());
    assert!(save(
        &link,
        &fixture(2),
        Overwrite::Replace,
        &Limits::default(),
        &AtomicBool::new(false)
    )
    .is_err());
    assert_eq!(fs::read(&path).unwrap(), old);
    assert!(load::<State>(&dir.0, &Limits::default(), &AtomicBool::new(false)).is_err());
    let fifo = dir.0.join("pipe.omat");
    let name = std::ffi::CString::new(fifo.as_os_str().as_encoded_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
    assert!(load::<State>(&fifo, &Limits::default(), &AtomicBool::new(false)).is_err());
}

struct ChildGuard(Child);
impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn child(path: &Path, mode: &str, revision: u64, marker: &Path) -> ChildGuard {
    ChildGuard(
        Command::new(std::env::current_exe().unwrap())
            .args([
                "--ignored",
                "--exact",
                "project_file::tests::child_probe",
                "--nocapture",
            ])
            .env("OMATAINER_CODEC_PATH", path)
            .env("OMATAINER_CODEC_MODE", mode)
            .env("OMATAINER_CODEC_REVISION", revision.to_string())
            .env("OMATAINER_CODEC_MARKER", marker)
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap(),
    )
}
#[test]
#[ignore = "private subprocess entry point, exercised by fresh_process_and_interrupted_save"]
fn child_probe() {
    let path = PathBuf::from(std::env::var_os("OMATAINER_CODEC_PATH").expect("private codec path"));
    let revision = std::env::var("OMATAINER_CODEC_REVISION")
        .unwrap()
        .parse()
        .unwrap();
    if std::env::var("OMATAINER_CODEC_MODE").unwrap() == "load" {
        compare(
            &load::<State>(&path, &Limits::default(), &AtomicBool::new(false)).unwrap(),
            &fixture(revision),
        );
    } else {
        let marker = PathBuf::from(std::env::var_os("OMATAINER_CODEC_MARKER").unwrap());
        let _ = save_with_hook(
            &path,
            &fixture(revision),
            Overwrite::Replace,
            &Limits::default(),
            &AtomicBool::new(false),
            &mut |phase| {
                if phase == Phase::PayloadChunk {
                    fs::write(&marker, b"temporary PCM written")?;
                    let deadline = Instant::now() + Duration::from_secs(10);
                    while Instant::now() < deadline {
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    return Err(io::Error::other("parent did not interrupt fixture"));
                }
                Ok(())
            },
        );
        panic!("interruption fixture should have been killed before commit");
    }
}
#[test]
fn fresh_process_and_interrupted_save_preserve_previous_good_native_file() {
    let dir = Directory::new();
    let path = dir.file();
    let marker = dir.0.join("written");
    write(&path, 1);
    let old = fs::read(&path).unwrap();
    assert!(child(&path, "load", 1, &marker).0.wait().unwrap().success());
    let mut interrupted = child(&path, "interrupt", 2, &marker);
    let deadline = Instant::now() + Duration::from_secs(3);
    while !marker.exists() {
        assert!(
            interrupted.0.try_wait().unwrap().is_none(),
            "writer ended early"
        );
        assert!(
            Instant::now() < deadline,
            "writer did not reach actual PCM write"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    interrupted.0.kill().unwrap();
    assert!(!interrupted.0.wait().unwrap().success());
    assert_eq!(fs::read(&path).unwrap(), old);
    assert!(
        fs::read_dir(&dir.0).unwrap().any(|p| p
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".omatainer-project-")),
        "no interrupted temporary file"
    );
    assert!(child(&path, "load", 1, &marker).0.wait().unwrap().success());
    write(&path, 2);
    assert!(child(&path, "load", 2, &marker).0.wait().unwrap().success());
}

#[test]
fn metadata_writer_and_racing_create_new_publication_stay_within_their_bounds() {
    let mut writer = BoundedJson {
        bytes: Vec::new(),
        limit: 1000,
    };
    for _ in 0..100 {
        writer.write_all(&[b'x'; 10]).unwrap();
        assert!(writer.bytes.capacity() <= 1000);
    }
    assert!(writer.write_all(b"x").is_err());
    assert_eq!(writer.bytes.len(), 1000);
    let dir = Directory::new();
    let path = dir.file();
    let outcome = save_with_hook(
        &path,
        &fixture(1),
        Overwrite::Never,
        &Limits::default(),
        &AtomicBool::new(false),
        &mut |phase| {
            if phase == Phase::BeforeCommit {
                fs::write(&path, b"newer file created by another writer")?;
            }
            Ok(())
        },
    );
    assert!(outcome.is_err());
    assert_eq!(
        fs::read(&path).unwrap(),
        b"newer file created by another writer"
    );
    assert_eq!(fs::read_dir(&dir.0).unwrap().count(), 1);
    assert!(load::<State>(&path, &Limits::default(), &AtomicBool::new(false)).is_err());
    assert_eq!(
        fs::read(&path).unwrap(),
        b"newer file created by another writer"
    );
}

#[test]
fn generic_state_preserves_finite_float64_positions_exactly() {
    let dir = Directory::new();
    let mut values = vec![
        0.0f64,
        -0.0,
        f64::MIN,
        f64::MAX,
        f64::from_bits(1),
        -f64::from_bits(1),
    ];
    let mut bits = 0x510e527fade682d1u64;
    for _ in 0..20_000 {
        bits = bits
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let value = f64::from_bits(bits);
        if value.is_finite() {
            values.push(value);
        }
    }
    let expected = Bundle {
        state: values,
        media: vec![],
    };
    save(
        &dir.file(),
        &expected,
        Overwrite::Never,
        &Limits::default(),
        &AtomicBool::new(false),
    )
    .unwrap();
    let actual =
        load::<Vec<f64>>(&dir.file(), &Limits::default(), &AtomicBool::new(false)).unwrap();
    assert_eq!(actual.state.len(), expected.state.len());
    for (index, (actual, expected)) in actual.state.iter().zip(&expected.state).enumerate() {
        assert_eq!(
            actual.to_bits(),
            expected.to_bits(),
            "state f64 at index {index}"
        );
    }
}
