use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
struct Dir(PathBuf);
impl Dir {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "omat-analysis-cache-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::DirBuilder::new().mode(0o700).create(&path).unwrap();
        Self(path)
    }
}
impl Drop for Dir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn waveform() -> Waveform {
    Waveform {
        sample_rate: 48_000,
        channels: 2,
        frames: 48_000,
        bands: (0..2048).map(|i| [i as f32 / 2048.0, 0.25, 0.75]).collect(),
    }
}

#[test]
fn cache_roundtrip_reuses_exact_blob_and_reopens_without_original_audio() {
    let dir = Dir::new();
    let mut cache = Cache::open(dir.0.clone()).unwrap();
    let wave = waveform();
    let reference = cache.write(wave.clone(), || false).unwrap();
    let path = dir.0.join(basename(&reference.sha256));
    let identity = FileFingerprint::read(&path).unwrap();
    assert_eq!(cache.write(wave.clone(), || false).unwrap(), reference);
    assert_eq!(FileFingerprint::read(&path), Some(identity));
    assert_eq!(cache.read(&reference, || false).unwrap(), wave);
    assert_eq!(fs::read_dir(&dir.0).unwrap().count(), 2);
    drop(cache);
    let reopened = Cache::open(dir.0.clone()).unwrap();
    assert_eq!(reopened.read(&reference, || false).unwrap(), wave);
    assert_eq!(fs::metadata(path).unwrap().mode() & 0o777, 0o600);
}

#[test]
fn cache_reports_corruption_repairs_to_fresh_blob_and_refuses_redirects() {
    let dir = Dir::new();
    let mut cache = Cache::open(dir.0.clone()).unwrap();
    let reference = cache.write(waveform(), || false).unwrap();
    let path = dir.0.join(basename(&reference.sha256));
    let bytes = fs::read(&path).unwrap();
    let mut corrupt = bytes.clone();
    corrupt[0] ^= 1;
    fs::write(&path, &corrupt).unwrap();
    assert!(cache.read(&reference, || false).is_err());
    let repaired = cache.write(waveform(), || false).unwrap();
    assert_ne!(repaired.sha256, reference.sha256);
    assert_eq!(cache.read(&repaired, || false).unwrap(), waveform());
    assert_eq!(fs::read(&path).unwrap(), corrupt);
    fs::remove_file(&path).unwrap();
    assert!(cache.read(&reference, || false).is_err());
    let outside = dir.0.join("unrelated-original");
    fs::write(&outside, &bytes).unwrap();
    std::os::unix::fs::symlink(&outside, &path).unwrap();
    assert!(cache.read(&reference, || false).is_err());
    assert!(cache.write(waveform(), || false).is_err());
    assert!(fs::symlink_metadata(&path)
        .unwrap()
        .file_type()
        .is_symlink());
    assert_eq!(fs::read(&outside).unwrap(), bytes);
}

#[test]
fn cache_validates_algorithm_geometry_finite_bands_and_payload_bounds() {
    let dir = Dir::new();
    let mut cache = Cache::open(dir.0.clone()).unwrap();
    for broken in 0..4 {
        let mut wave = waveform();
        match broken {
            0 => wave.bands[0][0] = f32::NAN,
            1 => wave.channels = 0,
            2 => wave.frames = u64::MAX,
            _ => wave.bands.push([0.0; 3]),
        }
        assert!(cache.write(wave, || false).is_err());
    }
    let reference = cache.write(waveform(), || false).unwrap();
    let mut bad_reference = reference.clone();
    bad_reference.frames += 1;
    assert!(cache.read(&bad_reference, || false).is_err());
    let path = dir.0.join(basename(&reference.sha256));
    let mut value: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    value["algorithm"] = 999.into();
    let bytes = serde_json::to_vec(&value).unwrap();
    let mut future = reference.clone();
    future.sha256 = Sha256::digest(&bytes).into();
    future.bytes = bytes.len() as u32;
    let future_path = dir.0.join(basename(&future.sha256));
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(future_path)
        .unwrap()
        .write_all(&bytes)
        .unwrap();
    assert!(cache.read(&future, || false).is_err());
}

#[test]
fn cache_cancellation_retains_only_complete_immutable_blobs_and_preserves_external_temp() {
    let dir = Dir::new();
    let mut cache = Cache::open(dir.0.clone()).unwrap();
    assert!(cache.write(waveform(), || true).is_err());
    assert_eq!(fs::read_dir(&dir.0).unwrap().count(), 1);
    let stop = std::cell::Cell::new(false);
    assert!(cache
        .write_with(
            waveform(),
            || stop.get(),
            |at, _| if at == 1 {
                stop.set(true);
            }
        )
        .is_err());
    assert_eq!(fs::read_dir(&dir.0).unwrap().count(), 1);
    stop.set(false);
    let complete = cache
        .write_with(
            waveform(),
            || stop.get(),
            |at, _| {
                if at == 2 {
                    stop.set(true);
                }
            },
        )
        .unwrap();
    assert_eq!(cache.read(&complete, || false).unwrap(), waveform());
    let mut changed = waveform();
    changed.bands[0][0] = 0.6;
    let mut replaced_path = None;
    assert!(cache
        .write_with(
            changed,
            || false,
            |at, path| if at == 1 {
                fs::remove_file(path).unwrap();
                fs::write(path, b"external temp replacement").unwrap();
                replaced_path = Some(path.to_path_buf());
            }
        )
        .is_err());
    assert_eq!(
        fs::read(replaced_path.unwrap()).unwrap(),
        b"external temp replacement"
    );
    assert_eq!(cache.read(&complete, || false).unwrap(), waveform());
}

#[test]
fn cache_owner_unlock_is_independent_of_retained_raw_description() {
    let dir = Dir::new();
    let cache = Cache::open(dir.0.clone()).unwrap();
    let inherited = cache.lock.try_clone().unwrap();
    assert!(Cache::open(dir.0.clone()).is_err());
    drop(cache);
    let owner = Cache::open(dir.0.clone()).unwrap();
    drop(inherited);
    assert!(Cache::open(dir.0.clone()).is_err());
    drop(owner);
    assert!(Cache::open(dir.0.clone()).is_ok());
}

#[test]
fn cache_quota_and_directory_replacement_fail_closed_without_evicting_existing_files() {
    let dir = Dir::new();
    let root = dir.0.join("cache");
    let mut cache = Cache::open(root.clone()).unwrap();
    let large = root.join("retained-other-cache-entry");
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&large)
        .unwrap();
    file.set_len(MAX_BYTES + 1).unwrap();
    assert!(cache
        .write(waveform(), || false)
        .unwrap_err()
        .contains("full"));
    assert_eq!(fs::metadata(&large).unwrap().len(), MAX_BYTES + 1);
    fs::rename(&root, dir.0.join("old-cache")).unwrap();
    fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
    assert!(cache
        .write(waveform(), || false)
        .unwrap_err()
        .contains("directory changed"));
    assert_eq!(fs::read_dir(&root).unwrap().count(), 0);
}

#[test]
fn cache_exact_byte_boundary_accepts_once_and_preserves_results_when_full() {
    let dir = Dir::new();
    let mut cache = Cache::open(dir.0.clone()).unwrap();
    let (bytes, _) = encoded(&waveform(), None).unwrap();
    let retained = dir.0.join("retained-sparse-cache-entry");
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&retained)
        .unwrap();
    file.set_len(MAX_BYTES - bytes.len() as u64).unwrap();
    let reference = cache.write(waveform(), || false).unwrap();
    assert_eq!(
        fs::metadata(&retained).unwrap().len() + u64::from(reference.bytes),
        MAX_BYTES
    );
    let mut different = waveform();
    different.bands[0][1] = 0.55;
    assert!(cache
        .write(different, || false)
        .unwrap_err()
        .contains("full"));
    assert_eq!(cache.read(&reference, || false).unwrap(), waveform());
}

#[test]
fn cache_midwrite_lock_and_directory_replacement_cannot_publish_into_external_paths() {
    for replace_directory in [false, true] {
        let dir = Dir::new();
        let root = dir.0.join("cache");
        let mut cache = Cache::open(root.clone()).unwrap();
        let external = b"external path after replacement";
        let mut preserved = None;
        let result = cache.write_with(
            waveform(),
            || false,
            |at, temporary| {
                if at != 1 {
                    return;
                }
                let path = if replace_directory {
                    fs::rename(&root, dir.0.join("displaced-cache")).unwrap();
                    fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
                    temporary.to_path_buf()
                } else {
                    let lock = root.join("writer.lock");
                    fs::remove_file(&lock).unwrap();
                    lock
                };
                OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .mode(0o600)
                    .open(&path)
                    .unwrap()
                    .write_all(external)
                    .unwrap();
                preserved = Some(path);
            },
        );
        assert!(result.is_err());
        assert_eq!(fs::read(preserved.unwrap()).unwrap(), external);
        assert!(!fs::read_dir(&root).unwrap().any(|e| e
            .unwrap()
            .file_name()
            .to_string_lossy()
            .ends_with(".wave.json")));
    }
}
