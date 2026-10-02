use super::*;
use crate::engine::{Engine, RtEngine};
use std::{fs, time::{Duration, Instant}};

struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("omatainer-dependencies-{}", crate::sampler_bank::BankId::new().unwrap()));
        fs::create_dir(&path).unwrap(); Self(path)
    }
}
impl Drop for Directory { fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); } }

fn wave(path: &Path, scale: i16) {
    let values: Vec<i16> = (0..2048).map(|i| ((i % 97) as i16 - 48) * scale).collect();
    let size = values.len() as u32 * 2;
    let mut bytes = b"RIFF".to_vec(); bytes.extend_from_slice(&(36 + size).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt "); bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes()); bytes.extend_from_slice(&2u16.to_le_bytes());
    bytes.extend_from_slice(&48000u32.to_le_bytes()); bytes.extend_from_slice(&192000u32.to_le_bytes());
    bytes.extend_from_slice(&4u16.to_le_bytes()); bytes.extend_from_slice(&16u16.to_le_bytes());
    bytes.extend_from_slice(b"data"); bytes.extend_from_slice(&size.to_le_bytes());
    for value in values { bytes.extend_from_slice(&value.to_le_bytes()); }
    fs::write(path, bytes).unwrap();
}
fn capture(engine: &Engine, rt: &mut RtEngine) -> crate::engine::project::Captured {
    let handle = engine.project.clone();
    let job = std::thread::spawn(move || handle.capture(&AtomicBool::new(false)).unwrap());
    let deadline = Instant::now() + Duration::from_secs(5);
    while !job.is_finished() { assert!(Instant::now() < deadline); rt.process(&mut []); std::thread::sleep(Duration::from_millis(1)); }
    job.join().unwrap()
}
fn project(path: &Path) -> crate::engine::project::Captured {
    let (engine, mut rt) = Engine::headless_for_test(48000, 256);
    let sample = decode::decode_sampler_file(path, fs::File::open(path).unwrap(), 1024 * 1024, || false).unwrap().sample;
    rt.decks[0].audio = Some(Arc::new(sample));
    capture(&engine, &mut rt)
}

#[test]
fn moved_audio_duplicates_and_reviewed_swaps_are_content_qualified_without_project_mutation() {
    let directory = Directory::new();
    let original = directory.0.join("original.wav"); wave(&original, 101);
    let saved = project(&original);
    let state = serde_json::to_value(&saved.state).unwrap();
    let audio = saved.media[saved.state.decks[0].audio.unwrap()].clone();
    let checked = inspect(&saved.state, &saved.media, &[], &AtomicBool::new(false)).unwrap();
    assert!(checked.assets.iter().any(|asset| asset.key.original_path == original.to_str().unwrap() && matches!(asset.availability, Availability::Verified)));
    let root = directory.0.join("moved"); fs::create_dir(&root).unwrap();
    let first = root.join("renamed-without-extension"); fs::rename(&original, &first).unwrap();
    let second = root.join("別のコピー.wav"); fs::copy(&first, &second).unwrap();
    wave(&root.join("same-size-wrong-audio.wav"), 103);
    let report = inspect(&saved.state, &saved.media, &[], &AtomicBool::new(false)).unwrap();
    let index = report.assets.iter().position(|asset| asset.key.original_path == original.to_str().unwrap()).unwrap();
    assert!(matches!(report.assets[index].availability, Availability::Unresolved(_)));
    assert_eq!(report.assets[index].frames, audio.frames());
    let found = search(&report.assets, &[root], &AtomicBool::new(false)).unwrap();
    assert!(found.complete, "{:?}", found.warnings);
    assert_eq!(found.matches[index].len(), 2);
    let choice = found.matches[index].iter().find(|candidate| candidate.location.path == second).unwrap().clone();
    let origins = verify_choices(&[(report.assets[index].clone(), choice.clone())], &AtomicBool::new(false)).unwrap();
    let resolved = inspect(&saved.state, &saved.media, &origins, &AtomicBool::new(false)).unwrap();
    assert!(matches!(resolved.assets[index].availability, Availability::Verified));
    assert_eq!(resolved.assets[index].source, Some(LibSource::File(second.clone())));
    assert_eq!(serde_json::to_value(&saved.state).unwrap(), state);
    assert!(Arc::ptr_eq(&audio, &saved.media[saved.state.decks[0].audio.unwrap()]));
    wave(&second, 107);
    assert!(verify_choices(&[(report.assets[index].clone(), choice)], &AtomicBool::new(false)).is_err());
    assert_eq!(audio.path, original.to_str().unwrap());
}

#[test]
fn cancellation_incomplete_roots_alias_bounds_and_bit_identity_are_explicit() {
    let directory = Directory::new(); let original = directory.0.join("original.wav"); wave(&original, 101);
    let saved = project(&original);
    let audio = saved.media[saved.state.decks[0].audio.unwrap()].clone();
    let mut changed = (*audio).clone(); changed.data[0] = f32::from_bits(changed.data[0].to_bits() ^ 1);
    assert_ne!(audio_hash(&audio, &AtomicBool::new(false)).unwrap(), audio_hash(&changed, &AtomicBool::new(false)).unwrap());
    let cancelled = AtomicBool::new(true);
    assert!(inspect(&saved.state, &saved.media, &[], &cancelled).is_err());
    fs::remove_file(&original).unwrap();
    let report = inspect(&saved.state, &saved.media, &[], &AtomicBool::new(false)).unwrap();
    let found = search(&report.assets, &[directory.0.join("offline")], &AtomicBool::new(false)).unwrap();
    assert!(!found.complete && !found.warnings.is_empty());
    assert!(search(&report.assets, &[directory.0.clone()], &cancelled).is_err());
    let asset = report.assets.iter().find(|asset| matches!(asset.availability, Availability::Unresolved(_))).unwrap();
    let origin = Origin { key: asset.key.clone(), source: LibSource::File("relative.wav".into()) };
    assert!(validate_origins(&[origin]).is_err());
    let origin = Origin { key: asset.key.clone(), source: LibSource::File(directory.0.join("new.wav")) };
    assert!(validate_origins(&[origin.clone(), origin]).is_err());
}
