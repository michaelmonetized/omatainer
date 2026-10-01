use super::*;
use crate::ui::test_support::Fixture;
use crate::ui::{egui, App, BuiltinStem};
use std::os::unix::fs::DirBuilderExt;
use std::time::{Duration, Instant, SystemTime};

struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "omatainer-scan-{}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&path)
            .unwrap();
        Self(path)
    }

    fn file(&self, name: &str) -> PathBuf {
        let path = self.0.join(name);
        std::fs::write(&path, b"synthetic scan fixture, never decoded").unwrap();
        path
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn wait_for(mut check: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while !check() {
        assert!(
            Instant::now() < deadline,
            "scan worker did not make progress"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn finish(app: &mut App) {
    wait_for(|| {
        app.poll_library_scan();
        !app.library_scan.active() && !app.library_metadata.active()
    });
}

fn start(app: &mut App, roots: Vec<PathBuf>) {
    assert!(app.library_scan.start(roots, app.library.clone()));
}

fn item(path: PathBuf, title: &str, bpm: f32) -> LibItem {
    LibItem {
        fingerprint: FileFingerprint::read(&path),
        source: LibSource::File(path),
        title: title.into(),
        artist: "cached artist".into(),
        bpm: Bpm::new(bpm, super::super::Origin::User),
        key: "DM".into(),
        length: Some(183.25),
        last_play: Some(SystemTime::UNIX_EPOCH + Duration::from_secs(123)),
    }
}

#[test]
fn real_ui_frames_and_deck_controls_continue_while_filesystem_worker_is_held() {
    let directory = Directory::new();
    for index in 0..48 {
        directory.file(&format!("Artist - Track {index} 130 C.wav"));
    }
    let mut fixture = Fixture::new(256);
    let before = fixture.app.library.clone();
    let (entered, ready) = mpsc::sync_channel(1);
    let (release, held) = mpsc::sync_channel(1);
    let held = std::sync::Mutex::new(held);
    let once = AtomicBool::new(false);
    let options = Options {
        before_entry: Some(Arc::new(move |_| {
            if !once.swap(true, Ordering::AcqRel) {
                entered.send(()).unwrap();
                let _ = held.lock().unwrap().recv_timeout(Duration::from_secs(5));
            }
        })),
    };
    let began = Instant::now();
    assert!(fixture.app.library_scan.start_with(
        vec![directory.0.clone()],
        fixture.app.library.clone(),
        options,
    ));
    assert!(began.elapsed() < Duration::from_secs(1));
    ready.recv_timeout(Duration::from_secs(2)).unwrap();
    let ctx = egui::Context::default();
    let began = Instant::now();
    let mut rendered = 0;
    for frame in 0..32 {
        let events = if frame <= 1 {
            vec![egui::Event::Key {
                key: egui::Key::Q,
                physical_key: Some(egui::Key::Q),
                pressed: frame == 0,
                repeat: false,
                modifiers: egui::Modifiers::default(),
            }]
        } else {
            Vec::new()
        };
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1440.0, 900.0),
            )),
            time: Some(frame as f64 / 60.0),
            events,
            ..Default::default()
        };
        let output = ctx.run(input, |ctx| fixture.app.update_frame(ctx));
        assert!(!output.shapes.is_empty());
        rendered += 1;
        fixture.rt.process(&mut [0.0; 512]);
        assert!(fixture.rt.decks[0].playing, "Q was not applied by renderer");
        assert!(Arc::ptr_eq(&fixture.app.library, &before));
        assert_eq!(fixture.app.library_scan.state, ScanState::Scanning);
    }
    let elapsed = began.elapsed();
    release.send(()).unwrap();
    assert!(
        elapsed < Duration::from_secs(2),
        "UI waited on held scan: {elapsed:?}"
    );
    assert_eq!(rendered, 32);
    assert!(fixture.rt.decks[0].pos >= 32.0 * 256.0);
    assert_eq!(fixture.rt.command_stats.received, 1);
    finish(&mut fixture.app);
    assert_eq!(fixture.app.library_scan.state, ScanState::Complete(50));
    eprintln!("32 real headless UI frames and deck renders while scan held: {elapsed:?}");
}

#[test]
fn atomic_merge_preserves_filtered_selection_cached_metadata_and_inflight_history() {
    let directory = Directory::new();
    let selected = directory.file("Track B 120 C.wav");
    let other = directory.file("Track A 140 C.wav");
    let mut fixture = Fixture::new(32);
    let mut items = builtin_crate_items();
    items.push(item(selected.clone(), "Track B custom title", 135.5));
    fixture.app.library = Arc::new(items);
    fixture.app.lib_filter = "Track".into();
    fixture.app.lib_sel = 0;
    let before = fixture.app.library.clone();
    start(
        &mut fixture.app,
        vec![directory.0.clone(), directory.0.clone()],
    );
    // Actual playback during traversal changes history without cloning the
    // shared baseline or allowing publication to overwrite the timestamp.
    fixture.app.load_sel(0);
    assert_eq!(
        fixture
            .decoder_jobs
            .recv_timeout(Duration::from_secs(3))
            .unwrap()
            .1,
        selected
    );
    fixture
        .decoder_results
        .send((
            0,
            Ok(crate::engine::decode::DecodedAudio {
                sample: crate::engine::dsp::Sample {
                    name: "history fixture".into(),
                    sr: 48_000,
                    ch: 1,
                    data: vec![0.25; 1024],
                    peaks: vec![[0.25; 3]; 8].into(),
                    bpm: 120.0,
                    path: selected.to_string_lossy().into(),
                },
                diagnostics: Default::default(),
            }),
        ))
        .unwrap();
    fixture.poll_loads();
    fixture.rt.process(&mut []);
    fixture
        .rt
        .apply(crate::engine::Command::DeckPlay { deck: 0 });
    fixture.rt.process(&mut [0.0; 128]);
    fixture.app.poll_play_history();
    let latest = fixture.app.item_last_play(&fixture.app.library[2]).unwrap();
    assert!(Arc::ptr_eq(&before, &fixture.app.library));
    finish(&mut fixture.app);
    assert_eq!(
        fixture.app.library.len(),
        4,
        "duplicate roots duplicated tracks"
    );
    let selected_item = fixture.app.filtered()[fixture.app.lib_sel];
    assert_eq!(selected_item.source, LibSource::File(selected.clone()));
    assert_eq!(selected_item.title, "Track B custom title");
    assert_eq!(selected_item.artist, "cached artist");
    assert_eq!(selected_item.bpm.value(), Some(135.5));
    assert_eq!(selected_item.key, "DM");
    assert_eq!(selected_item.length, Some(183.25));
    assert_eq!(fixture.app.item_last_play(selected_item), Some(latest));
    assert!(fixture
        .app
        .library
        .iter()
        .any(|item| item.source == LibSource::File(other.clone())));
    assert!(fixture
        .app
        .library
        .windows(2)
        .all(|items| items[0].bpm.value() <= items[1].bpm.value()));
}

#[test]
fn cancelled_scan_keeps_crate_and_rejects_overlap_until_worker_exits() {
    let directory = Directory::new();
    for index in 0..50 {
        directory.file(&format!("track{index}.wav"));
    }
    let mut fixture = Fixture::new(32);
    let before = fixture.app.library.clone();
    assert!(fixture.app.library_scan.start_with(
        vec![directory.0.clone()],
        before.clone(),
        Options {
            before_entry: Some(Arc::new(|_| std::thread::sleep(Duration::from_millis(8))))
        },
    ));
    wait_for(|| {
        fixture
            .app
            .library_scan
            .progress
            .visited
            .load(Ordering::Relaxed)
            >= 2
    });
    assert!(fixture.app.library_scan.label().contains("audio files"));
    let start_time = Instant::now();
    let ctx = egui::Context::default();
    let output = crate::ui::test_support::crate_frame(&ctx, &mut fixture.app, 1.0, Vec::new());
    let cancel = crate::ui::test_support::label_center(&output, "cancel scan");
    crate::ui::test_support::click(&ctx, &mut fixture.app, cancel, 1.1);
    assert_eq!(fixture.app.library_scan.state, ScanState::Cancelling);
    for _ in 0..100 {
        assert!(!fixture.app.library_scan.start(vec![], before.clone()));
    }
    finish(&mut fixture.app);
    assert!(start_time.elapsed() < Duration::from_secs(1));
    assert_eq!(fixture.app.library_scan.state, ScanState::Cancelled);
    assert!(Arc::ptr_eq(&fixture.app.library, &before));
    start(&mut fixture.app, vec![directory.0.clone()]);
    finish(&mut fixture.app);
    assert_eq!(fixture.app.library.len(), 52);
}

#[test]
fn cancelling_an_already_ready_result_prevents_stale_publication() {
    let directory = Directory::new();
    directory.file("candidate.wav");
    let mut fixture = Fixture::new(32);
    let before = fixture.app.library.clone();
    start(&mut fixture.app, vec![directory.0.clone()]);
    wait_for(|| {
        fixture
            .app
            .library_scan
            .progress
            .phase
            .load(Ordering::Acquire)
            == 2
    });
    fixture.app.library_scan.cancel();
    finish(&mut fixture.app);
    assert_eq!(fixture.app.library_scan.state, ScanState::Cancelled);
    assert!(Arc::ptr_eq(&fixture.app.library, &before));
    start(&mut fixture.app, vec![directory.0.clone()]);
    finish(&mut fixture.app);
    assert_eq!(fixture.app.library.len(), 3);
}

#[test]
fn errors_preserve_prior_crate_and_selection_and_a_later_scan_recovers() {
    let directory = Directory::new();
    let invalid_root = directory.file("not-a-directory");
    directory.file("discovered.wav");
    let mut fixture = Fixture::new(32);
    fixture.app.lib_sel = 1;
    let before = fixture.app.library.clone();
    start(&mut fixture.app, vec![directory.0.clone(), invalid_root]);
    finish(&mut fixture.app);
    assert!(
        matches!(&fixture.app.library_scan.state, ScanState::Failed(message) if message.contains("not a directory"))
    );
    assert!(Arc::ptr_eq(&fixture.app.library, &before));
    assert_eq!(fixture.app.lib_sel, 1);
    start(&mut fixture.app, vec![directory.0.clone()]);
    finish(&mut fixture.app);
    assert_eq!(fixture.app.library_scan.state, ScanState::Complete(3));
    assert_eq!(
        fixture.app.filtered()[fixture.app.lib_sel].source,
        LibSource::Builtin(BuiltinStem::Harmony)
    );
}

#[test]
fn stable_file_metadata_is_retained_but_changed_content_invalidates_cached_analysis() {
    let directory = Directory::new();
    let path = directory.file("Artist - Tune 120 C.wav");
    let mut fixture = Fixture::new(32);
    start(&mut fixture.app, vec![directory.0.clone()]);
    finish(&mut fixture.app);
    let position = fixture
        .app
        .library
        .iter()
        .position(|item| item.source == LibSource::File(path.clone()))
        .unwrap();
    let entries = Arc::make_mut(&mut fixture.app.library);
    entries[position].bpm = Bpm::new(132.5, super::super::Origin::User);
    entries[position].length = Some(45.0);
    start(&mut fixture.app, vec![directory.0.clone()]);
    finish(&mut fixture.app);
    let item = fixture
        .app
        .library
        .iter()
        .find(|item| item.source == LibSource::File(path.clone()))
        .unwrap();
    assert_eq!(item.bpm.value(), Some(132.5));
    assert_eq!(item.length, Some(45.0));
    std::fs::write(&path, b"changed content with a different size").unwrap();
    start(&mut fixture.app, vec![directory.0.clone()]);
    finish(&mut fixture.app);
    let item = fixture
        .app
        .library
        .iter()
        .find(|item| item.source == LibSource::File(path.clone()))
        .unwrap();
    assert_eq!(item.bpm.value(), Some(120.0));
    assert_eq!(item.length, None);
}

#[test]
fn scan_discovers_supported_regular_files_and_missing_roots_are_an_empty_success() {
    let directory = Directory::new();
    directory.file("upper.WAV");
    directory.file("lower.flac");
    directory.file("ignored.txt");
    std::fs::create_dir(directory.0.join("directory.mp3")).unwrap();
    let mut fixture = Fixture::new(32);
    start(
        &mut fixture.app,
        vec![directory.0.clone(), directory.0.join("absent")],
    );
    finish(&mut fixture.app);
    assert_eq!(fixture.app.library.len(), 4);
    std::fs::remove_file(directory.0.join("upper.WAV")).unwrap();
    fixture.app.lib_filter = "upper".into();
    fixture.app.lib_sel = 0;
    start(&mut fixture.app, vec![directory.0.clone()]);
    finish(&mut fixture.app);
    assert!(fixture.app.filtered().is_empty());
    fixture.app.load_sel(0);
    assert!(fixture.rt.cmd_rx.is_empty());
    assert_eq!(fixture.app.lib_sel, 0);
}

#[test]
fn dropping_scanner_never_joins_a_blocked_filesystem_hook_on_the_ui_thread() {
    let directory = Directory::new();
    directory.file("held.wav");
    let (entered, ready) = mpsc::sync_channel(1);
    let (release, wait) = mpsc::sync_channel(1);
    let wait = std::sync::Mutex::new(wait);
    let mut scanner = LibraryScan::default();
    scanner.start_with(
        vec![directory.0.clone()],
        Arc::new(builtin_crate_items()),
        Options {
            before_entry: Some(Arc::new(move |_| {
                entered.send(()).unwrap();
                let _ = wait.lock().unwrap().recv_timeout(Duration::from_secs(3));
            })),
        },
    );
    ready.recv_timeout(Duration::from_secs(2)).unwrap();
    let began = Instant::now();
    drop(scanner);
    assert!(began.elapsed() < Duration::from_millis(100));
    release.send(()).unwrap();
}

#[test]
fn history_merge_requires_verified_identity_even_before_first_worker_scan() {
    let directory = Directory::new();
    let unchanged = directory.file("Unchanged 120.wav");
    let replaced = directory.file("Replaced 120.wav");
    let unknown = directory.file("Unknown 120.wav");
    let mut unchanged_item = item(unchanged.clone(), "Unchanged", 130.0);
    let mut unknown_item = item(unknown.clone(), "Unknown", 130.0);
    unknown_item.fingerprint = None;
    let replaced_item = item(replaced.clone(), "Replaced", 130.0);
    std::fs::write(
        &replaced,
        b"different content, longer than the original synthetic scan fixture",
    )
    .unwrap();
    assert_ne!(replaced_item.fingerprint, FileFingerprint::read(&replaced));
    let known = Some(SystemTime::UNIX_EPOCH + Duration::from_secs(456));
    unchanged_item.last_play = known;
    let mut baseline = builtin_crate_items();
    baseline[0].last_play = known;
    baseline.extend([unchanged_item, replaced_item, unknown_item]);
    let mut fixture = Fixture::new(32);
    fixture.app.library = Arc::new(baseline);
    fixture.app.lib_filter = "Unchanged".into();
    fixture.app.refresh_library_view();
    start(&mut fixture.app, vec![directory.0.clone()]);
    finish(&mut fixture.app);
    for (source, expected) in [
        (LibSource::Builtin(BuiltinStem::Drums), known),
        (LibSource::File(unchanged.clone()), known),
        (LibSource::File(replaced), None),
        (LibSource::File(unknown), None),
    ] {
        let row = fixture
            .app
            .library
            .iter()
            .find(|row| row.source == source)
            .unwrap();
        assert_eq!(fixture.app.item_last_play(row), expected, "{source:?}");
    }
    assert_eq!(
        fixture.app.selected_library_item().unwrap().source,
        LibSource::File(unchanged)
    );
}

#[test]
fn repeated_rescans_preserve_history_and_selection_without_moving_history_to_other_files() {
    use crate::ui::play_history::Identity;
    let directory = Directory::new();
    let stable = directory.file("Stable 120.wav");
    let moved = directory.file("Moved 120.wav");
    let removed = directory.file("Removed 120.wav");
    let replaced = directory.file("Replaced 120.wav");
    let mut baseline = builtin_crate_items();
    for (path, title) in [
        (&stable, "Stable"),
        (&moved, "Moved"),
        (&removed, "Removed"),
        (&replaced, "Replaced"),
    ] {
        baseline.push(item(path.clone(), title, 130.0));
    }
    let mut fixture = Fixture::new(32);
    fixture.app.library = Arc::new(baseline);
    fixture.app.lib_filter = "".into();
    fixture.app.refresh_library_view();
    fixture.app.lib_sel = fixture
        .app
        .library_view
        .indices
        .iter()
        .position(|&index| fixture.app.library[index].source == LibSource::File(stable.clone()))
        .unwrap();
    fixture.app.refresh_library_view();
    start(&mut fixture.app, vec![directory.0.clone()]);
    finish(&mut fixture.app);
    let latest = SystemTime::UNIX_EPOCH + Duration::from_secs(789);
    let stable_identity = Identity::new(
        LibSource::File(stable.clone()),
        FileFingerprint::read(&stable),
    )
    .unwrap();
    fixture.app.last_played.record(&stable_identity, latest);
    let moved_source = LibSource::File(moved.clone());
    let moved_identity =
        Identity::new(moved_source.clone(), FileFingerprint::read(&moved)).unwrap();
    fixture
        .app
        .last_played
        .record(&moved_identity, latest - Duration::from_secs(1));
    let renamed = directory.0.join("Renamed 120.wav");
    std::fs::rename(&moved, &renamed).unwrap();
    std::fs::remove_file(&removed).unwrap();
    std::fs::write(&replaced, b"replacement with different file metadata").unwrap();
    directory.file("New 100.wav");
    start(&mut fixture.app, vec![directory.0.clone()]);
    finish(&mut fixture.app);
    assert_eq!(
        fixture.app.selected_library_item().unwrap().source,
        LibSource::File(stable.clone())
    );
    for row in fixture
        .app
        .library
        .iter()
        .filter(|row| matches!(row.source, LibSource::File(_)))
    {
        let expected = (row.source == LibSource::File(stable.clone())).then_some(latest);
        assert_eq!(
            fixture.app.item_last_play(row),
            expected,
            "{:?}",
            row.source
        );
        assert_ne!(row.source, moved_source);
        assert_ne!(row.source, LibSource::File(removed.clone()));
    }
    // A source outside the active roots is absent from the crate, but its
    // confirmed in-session history remains keyed by its unchanged identity.
    start(&mut fixture.app, vec![]);
    finish(&mut fixture.app);
    assert_eq!(fixture.app.library.len(), 2);
    assert!(fixture.app.selected_library_item().is_some());
    start(&mut fixture.app, vec![directory.0.clone()]);
    finish(&mut fixture.app);
    let stable_source = LibSource::File(stable);
    let row = fixture
        .app
        .library
        .iter()
        .find(|row| row.source == stable_source)
        .unwrap();
    assert_eq!(fixture.app.item_last_play(row), Some(latest));
}

#[test]
fn performance_protection_refuses_scans_and_retires_held_and_completed_candidates() {
    let directory = Directory::new();
    directory.file("Never visible.wav");
    let performance = Handle::default();
    let mut scanner = LibraryScan::default();
    scanner.set_performance(performance.clone());
    let mut library = Arc::new(builtin_crate_items());
    let original = library.clone();
    performance.set_enabled(true).unwrap();
    assert!(!scanner.start(vec![directory.0.clone()], library.clone()));
    assert!(matches!(scanner.state, ScanState::Failed(_)));
    performance.set_enabled(false).unwrap();
    let (entered, ready) = mpsc::sync_channel(1);
    let (release, held) = mpsc::sync_channel(1);
    let held = std::sync::Mutex::new(held);
    let once = AtomicBool::new(false);
    assert!(scanner.start_with(
        vec![directory.0.clone()],
        library.clone(),
        Options {
            before_entry: Some(Arc::new(move |_| {
                if !once.swap(true, Ordering::AcqRel) {
                    entered.send(()).unwrap();
                    held.lock()
                        .unwrap()
                        .recv_timeout(Duration::from_secs(2))
                        .unwrap();
                }
            })),
        }
    ));
    ready.recv_timeout(Duration::from_secs(2)).unwrap();
    performance.set_enabled(true).unwrap();
    release.send(()).unwrap();
    wait_for(|| {
        assert!(scanner.poll().is_none());
        !scanner.active()
    });
    assert!(Arc::ptr_eq(&library, &original));
    performance.set_enabled(false).unwrap();
    assert!(scanner.start(vec![directory.0.clone()], library.clone()));
    let mut publication = None;
    wait_for(|| {
        publication = scanner.poll();
        publication.is_some()
    });
    performance.set_enabled(true).unwrap();
    publication.unwrap().publish(&mut library);
    assert!(Arc::ptr_eq(&library, &original));
    wait_for(|| performance.status().optional_active == 0);
}
