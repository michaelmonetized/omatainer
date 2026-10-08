use super::*;
use crate::engine::{audio_clip, ClipKind};
use crate::project_versions::tests::Folder;

pub(crate) fn fixture() -> Bundle<project::Document> {
    let (mut state, _) = crate::engine::project::State::empty().unwrap();
    let make = |name: &str, value: f32| {
        Arc::new(Sample {
            name: name.into(),
            path: "/missing/source.wav".into(),
            sr: 48000,
            ch: 2,
            bpm: 120.0,
            peaks: Arc::new(vec![[-0.0, value, 0.25]]),
            spectrum: None,
            data: (0..8192)
                .map(|i| if i == 0 { -0.0 } else { value })
                .collect(),
        })
    };
    let a = make("Original recording", 0.25);
    let mut alias = (*a).clone();
    alias.name = "Different prepared identity".into();
    alias.path = "/missing/other-name.wav".into();
    alias.bpm = 128.0;
    let media = vec![
        a.clone(),
        Arc::new((*a).clone()),
        Arc::new(alias),
        make("Deck track", 0.5),
        make("Unused recording", 0.75),
    ];
    for (scene, index) in [(0, 1), (1, 2)] {
        let clip = &mut state.tracks[0].clips[scene];
        clip.kind = ClipKind::Audio;
        clip.audio = Some(index);
        let mut region = audio_clip::Region::full(&media[index], 120.0).unwrap();
        region.start = 1024;
        region.end = 2048;
        region.loop_start = 1024;
        region.loop_end = 2048;
        clip.audio_region = Some(region);
        clip.bars = (region.prepare(&media[index]).unwrap().duration_beats / 4.0) as f32;
        clip.name = format!("Slice {scene}");
    }
    state.decks[0].audio = Some(3);
    state.banks[0].media[0] = Some(1);
    state.banks[0].media[1] = Some(2);
    state.builtin[0] = Some(1);
    state.validate(&media).unwrap();
    Bundle {
        state: project::Document {
            engine: state,
            view: project::UiState::default(),
            mapping_schema: project::FACTORY_MAPPING_SCHEMA,
        },
        media,
    }
}
fn write(path: &Path, bundle: &Bundle<project::Document>) {
    assert_eq!(
        project_file::save(
            path,
            bundle,
            Overwrite::Never,
            &Limits::default(),
            &AtomicBool::new(false)
        )
        .unwrap(),
        SaveOutcome::Durable
    );
}
#[test]
fn compacted_copy_reopens_with_exact_metadata_pcm_and_musical_references() {
    let dir = Folder::new();
    let source = dir.0.join("original.omat");
    let destination = dir.0.join("compact.omat");
    let original = fixture();
    write(&source, &original);
    let encoded = fs::read(&source).unwrap();
    let cancel = AtomicBool::new(false);
    let draft = compact(source.clone(), destination.clone(), &cancel).unwrap();
    assert_eq!((draft.before_media, draft.after_media), (5, 3));
    assert_eq!(
        (draft.before_pcm, draft.after_pcm),
        (5 * 8192 * 4, 3 * 8192 * 4)
    );
    assert!(!destination.exists());
    assert_eq!(
        draft.publish(&cancel, || Ok(())).unwrap(),
        SaveOutcome::Durable
    );
    drop(draft);
    let reopened = archive(&destination, &cancel).unwrap();
    let mut expected = project::Document {
        engine: original.state.engine.clone(),
        view: original.state.view.clone(),
        mapping_schema: original.state.mapping_schema,
    };
    expected.engine.tracks[0].clips[0].audio = Some(0);
    expected.engine.tracks[0].clips[1].audio = Some(1);
    expected.engine.decks[0].audio = Some(2);
    expected.engine.banks[0].media[0] = Some(0);
    expected.engine.banks[0].media[1] = Some(1);
    expected.engine.builtin[0] = Some(0);
    assert_eq!(
        serde_json::to_value(&reopened.state).unwrap(),
        serde_json::to_value(&expected).unwrap()
    );
    for (after, before) in reopened.media.iter().zip([0, 2, 3]) {
        assert_eq!(
            serde_json::to_value(project_file::Media::from_sample(after)).unwrap(),
            serde_json::to_value(project_file::Media::from_sample(&original.media[before]))
                .unwrap()
        );
        assert_eq!(
            after.data.iter().map(|x| x.to_bits()).collect::<Vec<_>>(),
            original.media[before]
                .data
                .iter()
                .map(|x| x.to_bits())
                .collect::<Vec<_>>()
        );
    }
    assert_eq!(fs::read(&source).unwrap(), encoded);
    assert!(fs::metadata(&destination).unwrap().len() < encoded.len() as u64);
    println!("PROJECT_STORAGE_COMPACT {{\"sources_before\":5,\"sources_after\":3,\"musical_metadata_preserved\":true,\"original_bytes_unchanged\":true,\"physical_devices_opened\":false}}");
}
#[test]
fn compaction_cancellation_changed_source_failed_save_and_destination_collision_preserve_files() {
    let dir = Folder::new();
    let source = dir.0.join("original.omat");
    let destination = dir.0.join("compact.omat");
    write(&source, &fixture());
    let before = fs::read(&source).unwrap();
    let cancel = AtomicBool::new(false);
    assert!(compact(source.clone(), source.clone(), &cancel).is_err());
    assert!(compact(source.clone(), destination.clone(), &AtomicBool::new(true)).is_err());
    let draft = compact(source.clone(), destination.clone(), &cancel).unwrap();
    assert!(draft.publish(&AtomicBool::new(true), || Ok(())).is_err());
    assert!(draft
        .publish(&cancel, || Err::<(), _>("Protected".into()))
        .is_err());
    assert!(draft
        .publish(&cancel, || {
            cancel.store(true, Ordering::Release);
            Ok(())
        })
        .is_err());
    cancel.store(false, Ordering::Release);
    assert!(!destination.exists());
    assert!(draft
        .publish(&cancel, || {
            fs::write(&destination, b"another owner").unwrap();
            Ok(())
        })
        .is_err());
    assert_eq!(fs::read(&destination).unwrap(), b"another owner");
    assert_eq!(fs::read(&source).unwrap(), before);
    let failed = compact(source.clone(), dir.0.join("missing/copy.omat"), &cancel).unwrap();
    assert!(failed.publish(&cancel, || Ok(())).is_err());
    assert!(!dir.0.join("missing").exists());
    let stale_destination = dir.0.join("stale.omat");
    let stale = compact(source.clone(), stale_destination.clone(), &cancel).unwrap();
    assert!(stale
        .publish(&cancel, || {
            fs::write(&source, b"changed at commit admission").unwrap();
            Ok(())
        })
        .is_err());
    assert!(!stale_destination.exists());
    assert_eq!(fs::read(&source).unwrap(), b"changed at commit admission");
    assert!(stale.publish(&cancel, || Ok(())).is_err());
    assert_eq!(fs::read(&destination).unwrap(), b"another owner");
    assert!(fs::read_dir(&dir.0).unwrap().all(|e| !e
        .unwrap()
        .file_name()
        .to_string_lossy()
        .contains(".tmp")));
    println!("PROJECT_STORAGE_REFUSAL {{\"cancel_before_and_after_admission\":true,\"changed_source_refused\":true,\"occupied_destination_preserved\":true,\"real_failed_save\":true,\"physical_devices_opened\":false}}");
}
#[test]
fn report_distinguishes_shared_slices_saved_versions_chosen_folders_and_recovery_without_mutation()
{
    let dir = Folder::new();
    let cancel = AtomicBool::new(false);
    let source = dir.0.join("original.omat");
    let recording = dir.0.join("recordings");
    let render = dir.0.join("renders");
    let cache = dir.0.join("cache");
    let recovery = dir.0.join("recovery");
    for path in [&recording, &render, &cache, &recovery] {
        fs::create_dir(path).unwrap();
    }
    let paths = [
        recording.join("original.wav"),
        render.join("render.wav"),
        cache.join("waveforms"),
        recovery.join("recover.omat"),
    ];
    for (path, bytes) in paths.iter().zip([37, 53, 71, 97]) {
        fs::write(path, vec![0x42; bytes]).unwrap();
    }
    let mut original = fixture();
    for i in [0, 1] {
        Arc::make_mut(&mut original.media[i]).path = paths[0].display().to_string();
    }
    Arc::make_mut(&mut original.media[3]).path = paths[1].display().to_string();
    write(&source, &original);
    let root = dir.0.join("versions");
    let mut store = Store::open(&root, true, &cancel).unwrap();
    for name in ["First", "Shared second"] {
        store
            .snapshot(&original, name.into(), String::new(), &cancel, || Ok(()))
            .unwrap();
    }
    let mut orphan = fixture();
    Arc::make_mut(&mut orphan.media[4]).data[20] = 0.125;
    assert!(store
        .snapshot(
            &orphan,
            "Unpublished".into(),
            String::new(),
            &cancel,
            || Err::<(), _>("Refused".into())
        )
        .is_err());
    let roots = Roots {
        archive: Some(source.clone()),
        recordings: Some(recording),
        renders: Some(render),
        cache: Some(cache),
        recovery: Some(recovery),
    };
    let before: Vec<_> = [&source, &root.join(INDEX)]
        .into_iter()
        .chain(paths.iter())
        .map(|p| (p.to_owned(), fs::read(p).unwrap()))
        .collect();
    let report = inspect(Some(&store), &roots, &cancel).unwrap();
    for expected in [
        "5 embedded sources",
        "163840 decoded PCM bytes",
        "Original source files: 90 bytes available; 2 source paths unavailable",
        "Referenced slices: 2; 16384 logical PCM bytes",
        "Named versions: 2",
        "1 unused audio and 1 unused revision",
        "Original recording folder: 37 bytes in 1 files",
        "Rendered audio folder: 53 bytes in 1 files",
        "Shared library waveform cache: 71 bytes in 1 files",
        "Project recovery folder: 97 bytes in 1 files",
    ] {
        assert!(report.contains(expected), "Missing {expected}: {report}");
    }
    for (path, bytes) in &before {
        assert_eq!(fs::read(path).unwrap(), *bytes);
    }
    store
        .prune(
            store.review_prune(&[], &cancel).unwrap(),
            &cancel,
            || Ok(()),
        )
        .unwrap();
    let report = inspect(Some(&store), &roots, &cancel).unwrap();
    assert!(report.contains("0 unused audio and 0 unused revision"));
    assert!(!report.contains("Cleanup recovery: 0 retained bytes"));
    for (path, bytes) in before {
        if path != root.join(INDEX) {
            assert_eq!(fs::read(path).unwrap(), bytes);
        }
    }
    assert!(inspect(Some(&store), &roots, &AtomicBool::new(true)).is_err());
    println!("PROJECT_STORAGE_REPORT {{\"all_saved_versions_inspected\":true,\"shared_slice_bytes_separate\":true,\"chosen_recording_render_cache_recovery_roots\":true,\"no_files_deleted\":true,\"physical_devices_opened\":false}}");
}
#[test]
fn folder_inventory_refuses_links_special_files_and_excess_depth_without_following_them() {
    use std::os::unix::fs::symlink;
    let dir = Folder::new();
    let cancel = AtomicBool::new(false);
    let root = dir.0.join("root");
    fs::create_dir(&root).unwrap();
    let outside = dir.0.join("outside");
    fs::write(&outside, b"preserved").unwrap();
    symlink(&outside, root.join("link")).unwrap();
    assert!(folder(&root, &cancel).unwrap_err().contains("symlinks"));
    fs::rename(root.join("link"), dir.0.join("retained-link")).unwrap();
    let fifo = std::ffi::CString::new(root.join("fifo").as_os_str().as_encoded_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
    assert!(folder(&root, &cancel).unwrap_err().contains("special file"));
    fs::rename(root.join("fifo"), dir.0.join("retained-fifo")).unwrap();
    let mut deep = root.clone();
    for _ in 0..17 {
        deep = deep.join("nested");
        fs::create_dir(&deep).unwrap();
    }
    assert!(folder(&root, &cancel)
        .unwrap_err()
        .contains("directory levels"));
    assert_eq!(fs::read(outside).unwrap(), b"preserved");
    assert!(folder(Path::new("relative"), &cancel).is_err());
    assert!(folder(&root, &AtomicBool::new(true)).is_err());
}
