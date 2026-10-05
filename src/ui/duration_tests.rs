use super::*;
use crate::engine::{decode::decode_audio, load_receipt::Media};
use crate::ui::test_support::{crate_frame, Fixture};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{mpsc, Mutex};
use std::time::Duration;

struct Files(PathBuf);
impl Files {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "omatainer-duration-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn wave(&self, name: &str, sr: u32, channels: u16, frames: u32) -> PathBuf {
        let path = self.0.join(name);
        let bytes_per_frame = u32::from(channels) * 2;
        let size = frames * bytes_per_frame;
        let mut bytes = b"RIFF".to_vec();
        bytes.extend((36 + size).to_le_bytes());
        bytes.extend(b"WAVEfmt ");
        bytes.extend(16u32.to_le_bytes());
        bytes.extend(1u16.to_le_bytes());
        bytes.extend(channels.to_le_bytes());
        bytes.extend(sr.to_le_bytes());
        bytes.extend((sr * bytes_per_frame).to_le_bytes());
        bytes.extend((bytes_per_frame as u16).to_le_bytes());
        bytes.extend(16u16.to_le_bytes());
        bytes.extend(b"data");
        bytes.extend(size.to_le_bytes());
        for index in 0..frames * u32::from(channels) {
            bytes.extend(((index % 1024) as i16 - 512).to_le_bytes());
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
    let deadline = Instant::now() + Duration::from_secs(5);
    while !check() {
        assert!(Instant::now() < deadline, "duration fixture timed out");
        std::thread::sleep(Duration::from_millis(1));
    }
}
fn finish(f: &mut Fixture) {
    wait(|| {
        f.app.poll_load_receipts();
        !f.app.library_metadata.active()
    });
}
fn scan(f: &mut Fixture, files: &Files) {
    assert!(f
        .app
        .library_scan
        .start(vec![files.0.clone()], f.app.library.clone()));
    wait(|| {
        f.app.poll_library_scan();
        !f.app.library_scan.active() && !f.app.library_metadata.active()
    });
}
fn item<'a>(f: &'a Fixture, path: &PathBuf) -> &'a LibItem {
    f.app
        .library
        .iter()
        .find(|item| item.source == LibSource::File(path.clone()))
        .unwrap()
}
fn select(f: &mut Fixture, path: &PathBuf) {
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
fn apply(f: &mut Fixture) {
    let command = f.rt.cmd_rx.try_recv().unwrap();
    assert!(matches!(command, Command::DeckLoadRequested { .. }));
    f.rt.apply(command);
    finish(f);
}
fn visible(output: &egui::FullOutput, text: &str) -> bool {
    output.shapes.iter().any(|shape| matches!(&shape.shape, egui::epaint::Shape::Text(label) if label.galley.text() == text))
}

#[test]
fn real_mono_and_stereo_decodes_refresh_visible_duration_for_the_original_row() {
    let files = Files::new();
    let mut f = Fixture::new(48);
    f.app.loader = Some(Loader::start().unwrap());
    let fixtures = [
        (files.wave("Mono 155.wav", 44_100, 1, 143_325), 3.25, "0:03"),
        (
            files.wave("Stereo 140.wav", 48_000, 2, 216_000),
            4.5,
            "0:04",
        ),
        (
            files.wave("Stereo 160.wav", 96_000, 2, 108_000),
            1.125,
            "0:01",
        ),
    ];
    scan(&mut f, &files);
    let ctx = egui::Context::default();
    let output = crate_frame(&ctx, &mut f.app, 0.0, vec![]);
    assert!(visible(&output, "unknown"));
    for (index, (path, expected, label)) in fixtures.iter().enumerate() {
        assert_eq!(item(&f, path).length, None);
        select(&mut f, path);
        let fingerprint = item(&f, path).fingerprint;
        f.app.load_sel((index % 2) as u8);
        f.poll_loads();
        assert_eq!(
            item(&f, path).length,
            None,
            "decode alone did not become current"
        );
        // Browsing elsewhere while decoding/publishing never retargets the row.
        let selection = &fixtures[(index + 1) % fixtures.len()].0;
        select(&mut f, selection);
        let command = f.rt.cmd_rx.try_recv().unwrap();
        if let Command::DeckLoadRequested {
            media: Media::Decoded { audio, .. },
            ..
        } = &command
        {
            assert_eq!(decoded_duration(audio), Some(*expected));
        } else {
            panic!("expected decoded load");
        }
        f.rt.apply(command);
        finish(&mut f);
        assert_eq!(item(&f, path).fingerprint, fingerprint);
        assert_eq!(item(&f, path).length, Some(*expected));
        assert_eq!(
            f.app.selected_library_item().unwrap().source,
            LibSource::File(selection.clone())
        );
        let output = crate_frame(&ctx, &mut f.app, index as f64 + 1.0, vec![]);
        assert!(visible(&output, label), "missing visible duration {label}");
    }
    scan(&mut f, &files);
    for (path, expected, _) in fixtures {
        assert_eq!(item(&f, &path).length, Some(expected));
    }
}

#[test]
fn duration_is_cached_by_bytes_and_cannot_be_rolled_back_by_an_older_scan() {
    let files = Files::new();
    let path = files.wave("Track 155.wav", 48_000, 1, 168_000);
    let mut f = Fixture::new(48);
    f.app.loader = Some(Loader::start().unwrap());
    scan(&mut f, &files);
    let (entered, ready) = mpsc::sync_channel(1);
    let (release, held) = mpsc::sync_channel(1);
    let held = Mutex::new(held);
    let once = AtomicBool::new(false);
    assert!(f.app.library_scan.start_with(
        vec![files.0.clone()],
        f.app.library.clone(),
        library_scan::Options {
            before_entry: Some(Arc::new(move |_| {
                if !once.swap(true, Ordering::AcqRel) {
                    entered.send(()).unwrap();
                    held.lock()
                        .unwrap()
                        .recv_timeout(Duration::from_secs(5))
                        .unwrap();
                }
            })),
        }
    ));
    ready.recv_timeout(Duration::from_secs(3)).unwrap();
    select(&mut f, &path);
    f.app.load_sel(0);
    f.poll_loads();
    apply(&mut f);
    assert_eq!(item(&f, &path).length, Some(3.5));
    // A later BPM-only patch retains this duration in the shared worker cache.
    f.app.library_metadata.update(library_metadata::Patch {
        tags: None,
        source: LibSource::File(path.clone()),
        fingerprint: item(&f, &path).fingerprint.unwrap(),
        bpm: Bpm::new(121.0, Origin::Heuristic),
        duration: None,
    });
    finish(&mut f);
    release.send(()).unwrap();
    wait(|| {
        f.app.poll_library_scan();
        assert_eq!(
            item(&f, &path).length,
            Some(3.5),
            "old scan exposed unknown duration"
        );
        !f.app.library_scan.active() && !f.app.library_metadata.active()
    });
    assert_eq!(
        f.app.selected_library_item().unwrap().source,
        LibSource::File(path.clone())
    );
    let previous = item(&f, &path).fingerprint;
    let replacement = files.wave("replacement.wav", 44_100, 2, 55_125);
    std::fs::rename(replacement, &path).unwrap();
    scan(&mut f, &files);
    assert_ne!(item(&f, &path).fingerprint, previous);
    assert_eq!(
        item(&f, &path).length,
        None,
        "replacement inherited old duration"
    );
    f.app.load_sel(1);
    f.poll_loads();
    apply(&mut f);
    assert_eq!(item(&f, &path).length, Some(1.25));
}

#[test]
fn held_decode_keeps_real_crate_frames_and_controls_live_and_stale_file_duration_unknown() {
    let files = Files::new();
    let path = files.wave("Slow 155.wav", 48_000, 2, 72_000);
    let mut f = Fixture::new(48);
    scan(&mut f, &files);
    select(&mut f, &path);
    let outgoing = f.rt.decks[0].audio.as_ref().unwrap().clone();
    f.app.load_sel(0);
    assert_eq!(
        f.decoder_jobs
            .recv_timeout(Duration::from_secs(3))
            .unwrap()
            .1,
        path
    );
    let ctx = egui::Context::default();
    let start = Instant::now();
    for index in 0..24 {
        let output = crate_frame(&ctx, &mut f.app, index as f64 / 60.0, vec![]);
        assert!(visible(&output, "unknown"));
        let gain = 0.2 + index as f32 * 0.01;
        assert!(f.app.submit(Command::Master(gain)));
        f.rt.process(&mut [0.0; 128]);
        assert_eq!(f.rt.master, gain);
    }
    assert!(start.elapsed() < Duration::from_secs(2));
    let report = decode_audio(&path).unwrap();
    let replacement = files.wave("replacement.wav", 48_000, 1, 24_000);
    std::fs::rename(replacement, &path).unwrap();
    f.decoder_results.send((0, Ok(report))).unwrap();
    f.poll_loads();
    finish(&mut f);
    assert!(matches!(f.app.loads[0].as_ref().unwrap().phase, Phase::Failed(_)));
    assert!(f.rt.cmd_rx.try_recv().is_err(), "changed decode must never request deck replacement");
    assert!(Arc::ptr_eq(f.rt.decks[0].audio.as_ref().unwrap(), &outgoing));
    assert_eq!(
        item(&f, &path).length,
        None,
        "changed-during-decode file acquired stale duration"
    );
    scan(&mut f, &files);
    assert_eq!(item(&f, &path).length, None);
}

#[test]
fn unknown_and_failed_duration_are_distinct_from_known_zero_and_fractional_lengths() {
    assert_eq!(fmt_len(None), "unknown");
    assert_eq!(fmt_len(Some(0.0)), "0:00");
    assert_eq!(fmt_len(Some(59.999)), "0:59");
    assert_eq!(fmt_len(Some(3601.5)), "60:01");
    for invalid in [f64::NAN, f64::INFINITY, -1.0] {
        assert_eq!(fmt_len(Some(invalid)), "unknown");
    }
    let files = Files::new();
    let path = files.0.join("Broken 155.wav");
    std::fs::write(&path, b"not audio").unwrap();
    let mut f = Fixture::new(48);
    f.app.loader = Some(Loader::start().unwrap());
    scan(&mut f, &files);
    select(&mut f, &path);
    f.app.load_sel(0);
    f.poll_loads();
    finish(&mut f);
    assert!(matches!(
        f.app.loads[0].as_ref().unwrap().phase,
        Phase::Failed(_)
    ));
    assert_eq!(item(&f, &path).length, None);
}

#[test]
fn pending_bpm_only_patch_keeps_duration_but_never_crosses_file_identity() {
    let files = Files::new();
    let path = files.wave("Pending 155.wav", 48_000, 1, 72_000);
    let mut f = Fixture::new(48);
    scan(&mut f, &files);
    let mut patch = library_metadata::Patch {
        tags: None,
        source: LibSource::File(path.clone()),
        fingerprint: item(&f, &path).fingerprint.unwrap(),
        bpm: Bpm::new(120.0, Origin::Heuristic),
        duration: Some(1.5),
    };
    f.app.library_metadata.update(patch.clone());
    patch.duration = None;
    f.app.library_metadata.update(patch.clone());
    finish(&mut f);
    assert_eq!(item(&f, &path).length, Some(1.5));
    let replacement = files.wave("replacement.wav", 48_000, 1, 96_000);
    std::fs::rename(replacement, &path).unwrap();
    scan(&mut f, &files);
    assert_eq!(item(&f, &path).length, None);
    let mut old_pending = patch.clone();
    old_pending.duration = Some(1.5);
    f.app.library_metadata.update(old_pending);
    patch.fingerprint = item(&f, &path).fingerprint.unwrap();
    f.app.library_metadata.update(patch);
    finish(&mut f);
    assert_eq!(item(&f, &path).length, None);
}
