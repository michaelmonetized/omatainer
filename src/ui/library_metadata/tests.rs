use super::*;
use crate::engine::{decode::decode_audio, load_receipt::Media};
use crate::ui::test_support::{crate_frame, Fixture};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

struct Files(PathBuf);
impl Files {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "omatainer-bpm-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn wave(&self, name: &str) -> PathBuf {
        let path = self.0.join(name);
        // Short constant signal deliberately returns the existing heuristic's
        // fallback 120. This fixture tests provenance, not detection accuracy.
        let frames = 1024u32;
        let mut bytes = b"RIFF".to_vec();
        bytes.extend((36 + frames * 2).to_le_bytes());
        bytes.extend(b"WAVEfmt ");
        bytes.extend(16u32.to_le_bytes());
        bytes.extend(1u16.to_le_bytes());
        bytes.extend(1u16.to_le_bytes());
        bytes.extend(48000u32.to_le_bytes());
        bytes.extend(96000u32.to_le_bytes());
        bytes.extend(2u16.to_le_bytes());
        bytes.extend(16u16.to_le_bytes());
        bytes.extend(b"data");
        bytes.extend((frames * 2).to_le_bytes());
        for _ in 0..frames {
            bytes.extend(1000i16.to_le_bytes());
        }
        std::fs::write(&path, bytes).unwrap();
        path
    }
}
impl Drop for Files {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn wait(mut check: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(4);
    while !check() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(1));
    }
}
fn scan(f: &mut Fixture, directory: &Files) {
    assert!(f
        .app
        .library_scan
        .start(vec![directory.0.clone()], f.app.library.clone()));
    wait(|| {
        f.app.poll_library_scan();
        !f.app.library_scan.active()
    });
    finish(f);
}
fn finish(f: &mut Fixture) {
    wait(|| {
        f.app.poll_load_receipts();
        !f.app.library_metadata.active()
    });
}
fn find<'a>(f: &'a Fixture, path: &PathBuf) -> &'a LibItem {
    f.app
        .library
        .iter()
        .find(|item| item.source == LibSource::File(path.clone()))
        .unwrap()
}
fn select(f: &mut Fixture, path: &PathBuf) {
    f.app.lib_filter.clear();
    f.app.refresh_library_view();
    f.app.lib_sel = f
        .app
        .library_view
        .indices
        .iter()
        .position(|&i| f.app.library[i].source == LibSource::File(path.clone()))
        .unwrap();
    f.app.refresh_library_view();
}
fn apply_load(f: &mut Fixture) {
    let command = f.rt.cmd_rx.try_recv().unwrap();
    assert!(matches!(command, Command::DeckLoadRequested { .. }));
    f.rt.apply(command);
    finish(f);
}
fn visible(output: &egui::FullOutput, text: &str) -> bool {
    output.shapes.iter().any(|shape| matches!(&shape.shape, egui::epaint::Shape::Text(label) if label.galley.text().contains(text)))
}

#[test]
fn real_decode_reconciles_hint_to_labeled_current_deck_and_sorted_crate_without_retargeting_selection(
) {
    let files = Files::new();
    let path = files.wave("Artist - Track 155.wav");
    let other = files.wave("Artist - Track 140.wav");
    let mut f = Fixture::new(32);
    f.app.loader = Some(Loader::start().unwrap());
    scan(&mut f, &files);
    assert_eq!(find(&f, &path).bpm, Bpm::hint(155.0));
    select(&mut f, &path);
    f.app.load_sel(0);
    f.poll_loads();
    // Decoder completion alone never commits crate analysis.
    assert_eq!(find(&f, &path).bpm, Bpm::hint(155.0));
    select(&mut f, &other);
    let before = f.app.library.clone();
    apply_load(&mut f);
    assert!(!Arc::ptr_eq(&before, &f.app.library));
    assert_eq!(f.rt.decks[0].bpm, 120.0);
    assert_eq!(find(&f, &path).bpm, Bpm::new(120.0, Origin::Heuristic));
    assert_eq!(find(&f, &other).bpm, Bpm::hint(140.0));
    assert_eq!(
        f.app.selected_library_item().unwrap().source,
        LibSource::File(other)
    );
    assert!(f
        .app
        .library
        .windows(2)
        .all(|pair| pair[0].bpm.value().unwrap_or(f32::INFINITY)
            <= pair[1].bpm.value().unwrap_or(f32::INFINITY)));
    let ctx = egui::Context::default();
    let output = crate_frame(&ctx, &mut f.app, 0.0, vec![]);
    assert!(visible(&output, "120.0 est"));
    assert!(visible(&output, "140.0 hint"));
    let output = ctx.run(Default::default(), |ctx| f.app.load_status(ctx));
    assert!(visible(
        &output,
        "BPM 120.0 (heuristic estimate · unverified)"
    ));
    scan(&mut f, &files);
    assert_eq!(
        find(&f, &path).bpm.origin,
        Origin::Heuristic,
        "rescan lost cached provenance"
    );
}

#[test]
fn explicit_user_value_wins_real_decode_submission_and_same_identity_rescans() {
    let files = Files::new();
    let path = files.wave("Correction 155.wav");
    let mut f = Fixture::new(32);
    f.app.loader = Some(Loader::start().unwrap());
    scan(&mut f, &files);
    let items = Arc::make_mut(&mut f.app.library);
    items
        .iter_mut()
        .find(|item| item.source == LibSource::File(path.clone()))
        .unwrap()
        .bpm = Bpm::new(133.25, Origin::User);
    select(&mut f, &path);
    f.app.load_sel(1);
    f.poll_loads();
    let command = f.rt.cmd_rx.try_recv().unwrap();
    assert!(
        matches!(&command, Command::DeckLoadRequested { media: Media::Decoded { audio, .. }, .. } if audio.bpm == 133.25)
    );
    f.rt.apply(command);
    finish(&mut f);
    assert_eq!(f.rt.decks[1].bpm, 133.25);
    assert_eq!(find(&f, &path).bpm, Bpm::new(133.25, Origin::User));
    scan(&mut f, &files);
    assert_eq!(find(&f, &path).bpm.origin, Origin::User);
    // Replacing the pathname retires corrections attached to the old bytes.
    let replacement = files.wave("replacement.wav");
    std::fs::rename(replacement, &path).unwrap();
    scan(&mut f, &files);
    assert_eq!(find(&f, &path).bpm, Bpm::hint(155.0));
}

#[test]
fn rejected_cancelled_and_superseded_loads_do_not_publish_analysis() {
    let files = Files::new();
    let path = files.wave("Track 155.wav");
    for mode in ["rejected", "cancelled", "superseded"] {
        let mut f = Fixture::new(16);
        scan(&mut f, &files);
        select(&mut f, &path);
        f.app.load_sel(0);
        f.decoder_jobs.recv_timeout(Duration::from_secs(3)).unwrap();
        if mode == "rejected" {
            while f.app.engine.send(Command::Tap(Instant::now())).is_ok() {}
        }
        f.decoder_results
            .send((0, Ok(decode_audio(&path).unwrap())))
            .unwrap();
        f.poll_loads();
        if mode == "cancelled" {
            f.app.load_sel(0);
        }
        while let Ok(command) = f.rt.cmd_rx.try_recv() {
            f.rt.apply(command);
        }
        if mode == "superseded" {
            f.rt.apply(Command::DeckUnload { deck: 0 });
        }
        finish(&mut f);
        assert_eq!(find(&f, &path).bpm, Bpm::hint(155.0), "{mode}");
        assert_eq!(find(&f, &path).length, None, "{mode} acquired a duration");
    }
}

#[test]
fn changed_bytes_during_decode_or_after_scan_cannot_acquire_old_analysis() {
    let files = Files::new();
    let path = files.wave("Track 155.wav");
    let mut f = Fixture::new(16);
    scan(&mut f, &files);
    select(&mut f, &path);
    f.app.load_sel(0);
    f.decoder_jobs.recv_timeout(Duration::from_secs(3)).unwrap();
    let old_report = decode_audio(&path).unwrap();
    let replacement = files.wave("replacement.wav");
    std::fs::rename(replacement, &path).unwrap();
    f.decoder_results.send((0, Ok(old_report))).unwrap();
    f.poll_loads();
    apply_load(&mut f);
    assert_eq!(find(&f, &path).bpm, Bpm::hint(155.0));
    scan(&mut f, &files);
    assert_eq!(find(&f, &path).bpm, Bpm::hint(155.0));
    // A valid decode with a new fingerprint also cannot alter an old scan row.
    let other = files.wave("replacement2.wav");
    std::fs::rename(other, &path).unwrap();
    f.app.loader = Some(Loader::start().unwrap());
    f.app.load_sel(0);
    f.poll_loads();
    apply_load(&mut f);
    assert_eq!(find(&f, &path).bpm, Bpm::hint(155.0));
    scan(&mut f, &files);
    assert_eq!(find(&f, &path).bpm.origin, Origin::Heuristic);
}

#[test]
fn pending_metadata_rebases_on_new_arc_and_preserves_intervening_user_correction() {
    let files = Files::new();
    let path = files.wave("Track 155.wav");
    let mut f = Fixture::new(16);
    scan(&mut f, &files);
    let patch = Patch {
        source: LibSource::File(path.clone()),
        fingerprint: find(&f, &path).fingerprint.unwrap(),
        bpm: Bpm::new(120.0, Origin::Heuristic),
        duration: None,
    };
    f.app.library_metadata.update(patch);
    f.app.poll_library_metadata();
    assert!(f.app.library_metadata.in_flight);
    // The job has captured its old base. Replace it before any result can publish.
    let mut next = f.app.library.as_ref().clone();
    next.iter_mut()
        .find(|item| item.source == LibSource::File(path.clone()))
        .unwrap()
        .bpm = Bpm::new(131.0, Origin::User);
    f.app.library = Arc::new(next);
    f.app.library_metadata.rebase();
    finish(&mut f);
    assert_eq!(find(&f, &path).bpm, Bpm::new(131.0, Origin::User));
}

#[test]
fn unknown_and_invalid_bpm_have_explicit_state_and_cannot_sort_as_zero() {
    for value in [0.0, -1.0, f32::NAN, f32::INFINITY] {
        assert_eq!(Bpm::hint(value), Bpm::UNKNOWN);
        assert_eq!(Bpm::new(value, Origin::Heuristic).cell(), "— unknown");
    }
    let mut items = builtin_crate_items();
    items[0].bpm = Bpm::UNKNOWN;
    sort_crate(&mut items);
    assert_eq!(items.last().unwrap().bpm, Bpm::UNKNOWN);
    assert_eq!(
        Bpm::new(125.0, Origin::User).reconcile(Bpm::UNKNOWN).origin,
        Origin::User
    );
}

#[test]
fn scan_started_before_analysis_never_republishes_its_stale_filename_hint() {
    use std::sync::{atomic::AtomicBool, Mutex};
    let files = Files::new();
    let path = files.wave("Track 155.wav");
    let mut f = Fixture::new(32);
    f.app.loader = Some(Loader::start().unwrap());
    scan(&mut f, &files);
    let (entered, ready) = mpsc::sync_channel(1);
    let (release, held) = mpsc::sync_channel(1);
    let once = AtomicBool::new(false);
    let held = Mutex::new(held);
    let options = library_scan::Options {
        before_entry: Some(Arc::new(move |_| {
            if !once.swap(true, Ordering::AcqRel) {
                entered.send(()).unwrap();
                held.lock()
                    .unwrap()
                    .recv_timeout(Duration::from_secs(4))
                    .unwrap();
            }
        })),
    };
    assert!(f
        .app
        .library_scan
        .start_with(vec![files.0.clone()], f.app.library.clone(), options));
    ready.recv_timeout(Duration::from_secs(3)).unwrap();
    select(&mut f, &path);
    f.app.load_sel(0);
    f.poll_loads();
    apply_load(&mut f);
    assert_eq!(find(&f, &path).bpm.origin, Origin::Heuristic);
    release.send(()).unwrap();
    wait(|| {
        f.app.poll_library_scan();
        assert_eq!(
            find(&f, &path).bpm.origin,
            Origin::Heuristic,
            "stale scan metadata became visible"
        );
        !f.app.library_scan.active() && !f.app.library_metadata.active()
    });
    assert_eq!(
        f.app.selected_library_item().unwrap().source,
        LibSource::File(path)
    );
}

#[test]
fn newer_patch_revision_wins_over_an_already_inflight_candidate() {
    let files = Files::new();
    let path = files.wave("Track 155.wav");
    let mut f = Fixture::new(16);
    scan(&mut f, &files);
    let mut patch = Patch {
        source: LibSource::File(path.clone()),
        fingerprint: find(&f, &path).fingerprint.unwrap(),
        bpm: Bpm::new(120.0, Origin::Heuristic),
        duration: None,
    };
    f.app.library_metadata.update(patch.clone());
    f.app.poll_library_metadata();
    assert!(f.app.library_metadata.in_flight);
    patch.bpm = Bpm::new(118.0, Origin::Heuristic);
    f.app.library_metadata.update(patch);
    finish(&mut f);
    assert_eq!(find(&f, &path).bpm, Bpm::new(118.0, Origin::Heuristic));
}
