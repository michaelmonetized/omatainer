use super::test_support::Fixture;
use super::*;
use crate::engine::{SubmissionError, SubmissionOutcome};
use std::os::unix::fs::DirBuilderExt;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

fn controller_load(fixture: &Fixture, deck: u8) {
    fixture.app.engine.midi.receive_for_test(
        &fixture.app.engine.cmd,
        41,
        "Pioneer DDJ-FLX4",
        &[0x90 | deck, 0x02, 0x7f],
    );
}

fn frame(app: &mut App, ctx: &egui::Context, time: f64) -> egui::FullOutput {
    ctx.run(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1440.0, 900.0))),
            time: Some(time),
            ..Default::default()
        },
        |ctx| app.update_frame(ctx),
    )
}

fn assert_visible(output: &egui::FullOutput, text: &str) {
    assert!(
        output.shapes.iter().any(|shape| matches!(
            &shape.shape,
            egui::epaint::Shape::Text(label) if label.galley.text().contains(text)
        )),
        "status was not rendered: {text}"
    );
}

#[test]
fn pioneer_loads_both_builtin_sources_on_both_decks_via_actual_app_frames() {
    let mut fixture = Fixture::new(256);
    let ctx = egui::Context::default();
    let mut time = 1.0;
    for (title, stem) in [("Drums (session)", 0), ("Harmony (session)", 1)] {
        fixture.app.lib_filter = title.into();
        fixture.app.lib_sel = 0;
        frame(&mut fixture.app, &ctx, time); // publish the displayed selection
        for deck in 0..2 {
            controller_load(&fixture, deck);
            assert_eq!(fixture.app.engine.cmd.ui_request_stats().pending, 1);
            assert!(fixture.rt.cmd_rx.is_empty(), "load leaked into audio queue");
            assert!(
                fixture.decoder_jobs.try_recv().is_err(),
                "MIDI performed loading"
            );
            time += 0.1;
            let output = frame(&mut fixture.app, &ctx, time);
            assert_visible(
                &output,
                &format!("queued {title} → {}", (b'A' + deck) as char),
            );
            let command = fixture.rt.cmd_rx.try_recv().unwrap();
            assert!(
                matches!(command, Command::LoadBuiltin { deck: actual, stem: selected } if actual == deck && selected == stem)
            );
            fixture.rt.apply(command);
            assert_eq!(fixture.rt.decks[deck as usize].title, title);
            assert!(Arc::ptr_eq(
                fixture.rt.decks[deck as usize].audio.as_ref().unwrap(),
                fixture.rt.builtin[stem as usize].as_ref().unwrap()
            ));
            assert_eq!(
                fixture.app.status,
                format!("queued {title} → {}", (b'A' + deck) as char)
            );
            assert_eq!(fixture.app.engine.cmd.ui_request_stats().pending, 0);
            assert!(fixture.decoder_jobs.try_recv().is_err());
        }
        time += 0.1;
    }
    let stats = fixture.app.engine.cmd.ui_request_stats();
    assert_eq!(stats.accepted, 4);
    assert_eq!(stats.dispatched, 4);
    // Note-off and zero-velocity note-on never retrigger a controller load.
    for message in [[0x80, 0x02, 0x7f], [0x90, 0x02, 0]] {
        fixture.app.engine.midi.receive_for_test(
            &fixture.app.engine.cmd,
            41,
            "Pioneer DDJ-FLX4",
            &message,
        );
    }
    assert_eq!(fixture.app.engine.cmd.ui_request_stats().accepted, 4);
}

struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "omatainer-controller-load-{}-{}-{}",
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

    fn wave(&self, name: &str) -> PathBuf {
        let frames = 480u32;
        let mut data = Vec::new();
        data.extend(b"RIFF");
        data.extend((36 + frames * 2).to_le_bytes());
        data.extend(b"WAVEfmt ");
        data.extend(16u32.to_le_bytes());
        data.extend(1u16.to_le_bytes());
        data.extend(1u16.to_le_bytes());
        data.extend(48000u32.to_le_bytes());
        data.extend(96000u32.to_le_bytes());
        data.extend(2u16.to_le_bytes());
        data.extend(16u16.to_le_bytes());
        data.extend(b"data");
        data.extend((frames * 2).to_le_bytes());
        for _ in 0..frames {
            data.extend(12000i16.to_le_bytes());
        }
        let path = self.0.join(name);
        std::fs::write(&path, data).unwrap();
        path
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn file_item(path: PathBuf, title: &str) -> LibItem {
    LibItem {
        source: LibSource::File(path),
        title: title.into(),
        artist: String::new(),
        bpm: 0.0,
        key: String::new(),
        length: 0.0,
        last_play: None,
    }
}

#[test]
fn controller_file_request_reaches_the_production_decoder_and_renderer() {
    let directory = Directory::new();
    let path = directory.wave("production.wav");
    let mut fixture = Fixture::new(256);
    fixture.app.loader = Some(Loader::start().unwrap());
    fixture.app.library = Arc::new(vec![file_item(path.clone(), "production")]);
    fixture.app.lib_sel = 0;
    fixture.app.publish_library_selection();
    let other = fixture.rt.decks[0].audio.clone().unwrap();
    controller_load(&fixture, 1);
    assert!(fixture.rt.cmd_rx.is_empty());
    fixture.app.poll_ui_requests();
    assert_eq!(fixture.app.status, "loading production → B");
    fixture.poll_loads();
    let command = fixture.rt.cmd_rx.try_recv().unwrap();
    assert!(matches!(&command, Command::DeckDecoded { request, audio }
        if request.deck == 1 && audio.path == path.to_string_lossy()));
    fixture.rt.apply(command);
    let sample = fixture.rt.decks[1].audio.as_ref().unwrap();
    assert_eq!(sample.frames(), 480);
    assert!(sample.data.iter().any(|value| value.abs() > 0.1));
    assert!(Arc::ptr_eq(
        fixture.rt.decks[0].audio.as_ref().unwrap(),
        &other
    ));
    assert!(fixture.app.status.starts_with("queued production → B"));
}

#[test]
fn captured_file_request_survives_browse_and_rescan_and_uses_real_decoder_result() {
    let directory = Directory::new();
    let a = directory.wave("A.wav");
    let b = directory.wave("B.wav");
    let mut fixture = Fixture::new(256);
    fixture.app.library = Arc::new(vec![file_item(a.clone(), "A"), file_item(b.clone(), "B")]);
    fixture.app.lib_sel = 0;
    fixture.app.publish_library_selection();
    controller_load(&fixture, 1);
    fixture.app.lib_sel = 1;
    fixture.app.publish_library_selection();
    assert!(fixture
        .app
        .library_scan
        .start(vec![directory.0.clone()], fixture.app.library.clone()));
    let until = Instant::now() + Duration::from_secs(3);
    while fixture.app.library_scan.active() {
        fixture.app.poll_library_scan();
        assert!(Instant::now() < until);
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(
        fixture.app.filtered()[fixture.app.lib_sel].source,
        LibSource::File(b.clone())
    );
    fixture.app.publish_library_selection();
    assert!(fixture.decoder_jobs.try_recv().is_err());
    fixture.app.poll_ui_requests();
    let (deck, path) = fixture
        .decoder_jobs
        .recv_timeout(Duration::from_secs(3))
        .unwrap();
    assert_eq!((deck, &path), (1, &a));
    assert_eq!(fixture.app.status, "loading A → B");
    assert_visible(
        &frame(&mut fixture.app, &egui::Context::default(), 1.0),
        "loading A → B",
    );
    assert_eq!(
        fixture.app.filtered()[fixture.app.lib_sel].source,
        LibSource::File(b)
    );
    assert!(fixture.rt.cmd_rx.is_empty());
    let decoded = crate::engine::dsp::decode_audio(&path).unwrap();
    fixture.decoder_results.send((deck, Ok(decoded))).unwrap();
    fixture.poll_loads();
    let command = fixture.rt.cmd_rx.try_recv().unwrap();
    fixture.rt.apply(command);
    assert_eq!(
        fixture.rt.decks[1].audio.as_ref().unwrap().path,
        a.to_string_lossy()
    );
    assert_eq!(fixture.rt.decks[1].audio.as_ref().unwrap().frames(), 480);
    assert!(fixture.app.status.starts_with("queued A"));
}

#[test]
fn empty_and_failed_captured_sources_report_failure_without_substituting_later_selection() {
    let mut fixture = Fixture::new(256);
    fixture.app.lib_filter = "no match".into();
    fixture.app.publish_library_selection();
    controller_load(&fixture, 0);
    fixture.app.lib_filter.clear();
    fixture.app.publish_library_selection();
    fixture.app.poll_ui_requests();
    assert_eq!(fixture.app.status, "load failed: no library item selected");
    assert!(fixture.rt.cmd_rx.is_empty());
    let directory = Directory::new();
    let missing = directory.0.join("missing.wav");
    fixture.app.library = Arc::new(vec![file_item(missing.clone(), "missing")]);
    fixture.app.lib_sel = 0;
    fixture.app.publish_library_selection();
    controller_load(&fixture, 1);
    fixture.app.library = Arc::new(builtin_crate_items());
    fixture.app.publish_library_selection();
    fixture.app.poll_ui_requests();
    assert_eq!(
        fixture
            .decoder_jobs
            .recv_timeout(Duration::from_secs(3))
            .unwrap(),
        (1, missing.clone())
    );
    assert_eq!(fixture.app.status, "loading missing → B");
    let failure = crate::engine::dsp::decode_audio(&missing);
    fixture.decoder_results.send((1, failure)).unwrap();
    fixture.poll_loads();
    assert!(fixture.app.status.starts_with("load failed on B:"));
    assert_visible(
        &frame(&mut fixture.app, &egui::Context::default(), 1.0),
        "load failed on B:",
    );
    assert!(fixture.rt.cmd_rx.is_empty());
}

#[test]
fn bounded_handoff_preserves_fifo_sources_and_limits_dispatch_work_per_frame() {
    use crate::engine::ui_requests::{CAPACITY, PER_FRAME};
    let mut fixture = Fixture::new(256);
    for index in 0..CAPACITY {
        fixture.app.lib_sel = index % 2;
        fixture.app.publish_library_selection();
        assert_eq!(
            fixture.app.engine.send(Command::DeckLoadSelected {
                deck: (index % 2) as u8
            }),
            Ok(SubmissionOutcome::Accepted)
        );
    }
    assert_eq!(
        fixture
            .app
            .engine
            .send(Command::DeckLoadSelected { deck: 0 }),
        Err(SubmissionError::UiFull)
    );
    assert_eq!(fixture.app.engine.cmd.ui_request_stats().pending, CAPACITY);
    assert!(fixture.rt.cmd_rx.is_empty());
    fixture.app.poll_ui_requests();
    assert_eq!(
        fixture.app.engine.cmd.ui_request_stats().pending,
        CAPACITY - PER_FRAME
    );
    assert_eq!(fixture.rt.cmd_rx.len(), PER_FRAME);
    for index in 0..PER_FRAME {
        assert!(
            matches!(fixture.rt.cmd_rx.try_recv().unwrap(), Command::LoadBuiltin { deck, stem } if deck == (index % 2) as u8 && stem == deck)
        );
    }
    let ctx = egui::Context::default();
    frame(&mut fixture.app, &ctx, 1.0);
    assert_eq!(
        fixture.app.submission_error.get(),
        Some(SubmissionError::UiFull)
    );
    assert_eq!(fixture.app.engine.cmd.ui_request_stats().pending, 0);
    assert_eq!(
        fixture.app.engine.cmd.ui_request_stats().dispatched,
        CAPACITY as u64
    );
    assert_eq!(fixture.app.engine.cmd.ui_request_stats().rejected, 1);
}

#[test]
fn closed_or_unattached_gui_and_invalid_decks_fail_explicitly() {
    let (commands, audio) = crate::engine::CommandPort::channel(32);
    assert_eq!(
        commands.send(Command::DeckLoadSelected { deck: 0 }),
        Err(SubmissionError::UiUnavailable)
    );
    let gui = commands.take_ui_receiver().unwrap();
    assert!(commands.take_ui_receiver().is_none());
    assert_eq!(
        commands.send(Command::DeckLoadSelected { deck: u8::MAX }),
        Err(SubmissionError::InvalidTarget)
    );
    drop(gui);
    assert_eq!(
        commands.send(Command::DeckLoadSelected { deck: 0 }),
        Err(SubmissionError::UiUnavailable)
    );
    assert_eq!(commands.ui_request_stats().accepted, 0);
    assert_eq!(commands.ui_request_stats().rejected, 3);
    assert!(audio.is_empty());
}

#[test]
fn audio_admission_failure_and_raw_uncaptured_request_are_visible_without_replacement() {
    let mut fixture = Fixture::new(16);
    let original = fixture.rt.decks[0].audio.clone().unwrap();
    while fixture
        .app
        .engine
        .send(Command::Tap(Instant::now()))
        .is_ok()
    {}
    controller_load(&fixture, 0);
    fixture.app.poll_ui_requests();
    assert_eq!(fixture.app.status, "Load was not accepted");
    assert_eq!(
        fixture.app.submission_error.get(),
        Some(SubmissionError::Full)
    );
    assert!(Arc::ptr_eq(
        fixture.rt.decks[0].audio.as_ref().unwrap(),
        &original
    ));
    fixture.rt.apply(Command::DeckLoadSelected { deck: 0 });
    let ctx = egui::Context::default();
    frame(&mut fixture.app, &ctx, 1.0);
    assert_eq!(
        fixture.app.submission_error.get(),
        Some(SubmissionError::UncapturedSelection)
    );
    assert_eq!(fixture.app.engine.cmd.ui_request_stats().pending, 0);
    assert!(Arc::ptr_eq(
        fixture.rt.decks[0].audio.as_ref().unwrap(),
        &original
    ));
}
