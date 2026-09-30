use super::test_support::Fixture;
use super::*;
use std::os::unix::fs::DirBuilderExt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

#[test]
fn load_sel_routes_both_builtins_to_both_decks_and_reload_resets_playhead() {
    let mut fixture = Fixture::new(16);
    for (title, stem) in [("Drums (session)", 0), ("Harmony (session)", 1)] {
        // Match the real filtered selection, not the unfiltered library index.
        fixture.app.lib_filter = title.into();
        fixture.app.lib_sel = 0;
        for deck in 0..2 {
            for reload in [false, true] {
                fixture.rt.apply(Command::DeckSeek { deck, frac: 0.5 });
                fixture.rt.apply(Command::DeckPlay { deck });
                fixture.rt.process(&mut [0.0; 512]);
                assert!(fixture.rt.decks[deck as usize].pos > 0.0);
                assert!(fixture.rt.decks[deck as usize].playing);
                let other = &fixture.rt.decks[1 - deck as usize];
                let other_position = other.pos;
                let other_audio = other.audio.clone().unwrap();

                // This is the same entry point used by both arrow
                // buttons and a crate row's double-click handler.
                fixture.app.load_sel(deck);
                let command = fixture.rt.cmd_rx.try_recv().unwrap();
                assert!(
                    matches!(command, Command::LoadBuiltin { deck: d, stem: s } if d == deck && s == stem)
                );
                assert!(fixture.rt.cmd_rx.is_empty());
                assert!(matches!(
                    fixture.decoder_jobs.try_recv(),
                    Err(mpsc::TryRecvError::Empty)
                ));
                assert_eq!(
                    fixture.app.status,
                    format!("queued {title} → {}", (b'A' + deck) as char)
                );
                assert_eq!(fixture.app.submission_error.get(), None);

                // Queue admission is not application. Inspect the actual
                // renderer after applying the exact emitted command too.
                fixture.rt.apply(command);
                let loaded = &fixture.rt.decks[deck as usize];
                assert!(Arc::ptr_eq(
                    loaded.audio.as_ref().unwrap(),
                    fixture.rt.builtin[stem as usize].as_ref().unwrap()
                ));
                assert_eq!(loaded.title, title);
                assert_eq!(loaded.pos, 0.0, "deck {deck}, stem {stem}, reload={reload}");
                assert_eq!(loaded.cue_pos, 0.0);
                assert!(!loaded.playing);
                assert!(!loaded.loop_on);
                assert_eq!(fixture.rt.decks[1 - deck as usize].pos, other_position);
                assert!(Arc::ptr_eq(
                    fixture.rt.decks[1 - deck as usize].audio.as_ref().unwrap(),
                    &other_audio
                ));
            }
        }
    }
}

struct WaveFile(PathBuf);

impl WaveFile {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let directory = std::env::temp_dir().join(format!(
            "omatainer-ui-load-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&directory)
            .unwrap();
        // A real file may have a builtin-looking basename. Its typed source
        // still selects the decoder, not a heuristic string match.
        let path = directory.join("builtin:harmony.wav");
        let frames = 4800u32;
        let mut bytes = Vec::new();
        bytes.extend(b"RIFF");
        bytes.extend((36 + frames * 2).to_le_bytes());
        bytes.extend(b"WAVEfmt ");
        bytes.extend(16u32.to_le_bytes());
        bytes.extend(1u16.to_le_bytes()); // PCM
        bytes.extend(1u16.to_le_bytes()); // mono
        bytes.extend(48_000u32.to_le_bytes());
        bytes.extend(96_000u32.to_le_bytes());
        bytes.extend(2u16.to_le_bytes());
        bytes.extend(16u16.to_le_bytes());
        bytes.extend(b"data");
        bytes.extend((frames * 2).to_le_bytes());
        for frame in 0..frames {
            let sample =
                ((frame as f32 * std::f32::consts::TAU * 440.0 / 48_000.0).sin() * 16_000.0) as i16;
            bytes.extend(sample.to_le_bytes());
        }
        std::fs::write(&path, bytes).unwrap();
        Self(path)
    }

    fn item(&self) -> LibItem {
        LibItem {
            title: "real WAV".into(),
            artist: String::new(),
            bpm: 0.0,
            key: String::new(),
            length: 0.0,
            last_play: None,
            source: LibSource::File(self.0.clone()),
        }
    }
}

impl Drop for WaveFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(self.0.parent().unwrap());
    }
}

#[test]
fn load_sel_sends_real_files_only_to_decoder_then_applies_decoded_audio() {
    let wave = WaveFile::new();
    let mut fixture = Fixture::new(16);
    fixture.app.library.push(wave.item());
    fixture.app.lib_filter = "real WAV".into();
    for deck in 0..2 {
        fixture.app.load_sel(deck);
        assert!(fixture.rt.cmd_rx.is_empty());
        let (job_deck, path) = fixture.decoder_jobs.try_recv().unwrap();
        assert_eq!(job_deck, deck);
        assert_eq!(path, wave.0);
        assert!(matches!(
            fixture.decoder_jobs.try_recv(),
            Err(mpsc::TryRecvError::Empty)
        ));
        assert_eq!(
            fixture.app.status,
            format!("loading real WAV → {}", (b'A' + deck) as char)
        );
        let sample = crate::engine::dsp::decode_audio(&path).unwrap();
        assert_eq!(sample.sample.frames(), 4800);
        assert!(sample.sample.data.iter().any(|x| x.abs() > 0.1));
        fixture.decoder_results.send((deck, Ok(sample))).unwrap();
        fixture.app.poll_loads();
        let command = fixture.rt.cmd_rx.try_recv().unwrap();
        assert!(
            matches!(&command, Command::DeckAudio { deck: d, audio } if *d == deck && audio.path == wave.0.to_string_lossy())
        );
        fixture.rt.apply(command);
        assert_eq!(fixture.rt.decks[deck as usize].pos, 0.0);
        assert_eq!(
            fixture.rt.decks[deck as usize]
                .audio
                .as_ref()
                .unwrap()
                .frames(),
            4800
        );
        assert!(fixture.app.status.starts_with("queued builtin:harmony"));
    }
}

#[test]
fn builtin_load_rejection_is_reported_without_decoder_fallback() {
    let mut fixture = Fixture::new(16);
    // Occupy ordinary queue capacity without coalescing adjacent assignments.
    while fixture
        .app
        .engine
        .send(Command::Tap(Instant::now()))
        .is_ok()
    {}
    let queued = fixture.rt.cmd_rx.len();
    fixture.app.load_sel(1);
    assert_eq!(fixture.rt.cmd_rx.len(), queued);
    assert_eq!(fixture.app.status, "Load was not accepted");
    assert_eq!(
        fixture.app.submission_error.get(),
        Some(crate::engine::SubmissionError::Full)
    );
    assert!(matches!(
        fixture.decoder_jobs.try_recv(),
        Err(mpsc::TryRecvError::Empty)
    ));

    drop(fixture.rt);
    fixture.app.load_sel(0);
    assert_eq!(fixture.app.status, "Load was not accepted");
    assert_eq!(
        fixture.app.submission_error.get(),
        Some(crate::engine::SubmissionError::Disconnected)
    );
    assert!(matches!(
        fixture.decoder_jobs.try_recv(),
        Err(mpsc::TryRecvError::Empty)
    ));
}

#[test]
fn file_load_reports_unavailable_decoder_and_empty_selection_is_inert() {
    let wave = WaveFile::new();
    let mut fixture = Fixture::new(16);
    fixture.app.library = vec![wave.item()];
    drop(fixture.decoder_jobs);
    fixture.app.load_sel(0);
    assert_eq!(fixture.app.status, "load failed: decoder is unavailable");
    assert!(fixture.rt.cmd_rx.is_empty());
    fixture.app.lib_filter = "no matching media".into();
    fixture.app.status = "unchanged".into();
    fixture.app.load_sel(1);
    assert_eq!(fixture.app.status, "unchanged");
    assert!(fixture.rt.cmd_rx.is_empty());
}

#[test]
fn incomplete_decode_leaves_loaded_decks_untouched_and_surfaces_the_reason() {
    let wave = WaveFile::new();
    let bytes = std::fs::read(&wave.0).unwrap();
    std::fs::write(&wave.0, &bytes[..bytes.len() - 1000]).unwrap();
    let mut fixture = Fixture::new(16);
    let before: Vec<_> = fixture
        .rt
        .decks
        .iter()
        .map(|deck| deck.audio.clone().unwrap())
        .collect();
    for deck in 0..2 {
        let result = crate::engine::dsp::decode_audio(&wave.0);
        assert_eq!(
            result.as_ref().unwrap_err().kind,
            crate::engine::decode::DecodeFailureKind::Incomplete
        );
        fixture.decoder_results.send((deck, result)).unwrap();
        fixture.app.poll_loads();
        assert!(fixture.rt.cmd_rx.is_empty());
        assert!(fixture.app.status.contains("Incomplete audio"));
        assert!(fixture.app.status.contains("media was not loaded"));
        assert_load_status_is_painted(&fixture.app, "Incomplete audio");
        assert!(Arc::ptr_eq(
            fixture.rt.decks[deck as usize].audio.as_ref().unwrap(),
            &before[deck as usize]
        ));
    }
}

#[test]
fn uncertain_length_warning_survives_worker_result_and_deck_admission() {
    let directory = WaveFile::new();
    let path = directory.0.with_extension("mp3");
    std::fs::write(
        &path,
        include_bytes!("../../tests/fixtures/audio/tone-estimated.mp3"),
    )
    .unwrap();
    let mut fixture = Fixture::new(16);
    let report = crate::engine::dsp::decode_audio(&path).unwrap();
    assert!(report.diagnostics.warning().is_some());
    fixture.decoder_results.send((0, Ok(report))).unwrap();
    fixture.app.poll_loads();
    assert!(fixture.app.status.contains("length unverified"));
    assert!(fixture
        .app
        .status
        .contains("incomplete media cannot be ruled out"));
    assert_load_status_is_painted(&fixture.app, "length unverified");
    let command = fixture.rt.cmd_rx.try_recv().unwrap();
    assert!(matches!(&command, Command::DeckAudio { deck: 0, .. }));
    fixture.rt.apply(command);
    assert_eq!(
        fixture.rt.decks[0].audio.as_ref().unwrap().path,
        path.to_string_lossy()
    );
}

fn assert_load_status_is_painted(app: &App, expected: &str) {
    let ctx = egui::Context::default();
    let output = ctx.run(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(860.0, 640.0))),
            ..Default::default()
        },
        |ctx| app.load_status(ctx),
    );
    assert!(
        output.shapes.iter().any(|shape| match &shape.shape {
            egui::epaint::Shape::Text(text) =>
                text.galley.text().contains(expected)
                    && text.visual_bounding_rect().intersects(ctx.screen_rect()),
            _ => false,
        }),
        "load status was not painted: {expected}"
    );
}

#[test]
fn crate_buttons_and_double_click_emit_builtin_commands_on_the_selected_deck() {
    use super::test_support::{click, crate_frame, label_center};
    let mut fixture = Fixture::new(16);
    let ctx = egui::Context::default();
    let mut time = 1.0;
    for (title, stem) in [("Drums (session)", 0), ("Harmony (session)", 1)] {
        fixture.app.lib_filter = title.into();
        fixture.app.lib_sel = 0;
        for (deck, label) in [(0, "→ A"), (1, "→ B")] {
            let output = crate_frame(&ctx, &mut fixture.app, time, Vec::new());
            let button = label_center(&output, label);
            click(&ctx, &mut fixture.app, button, time + 0.1);
            assert!(
                matches!(fixture.rt.cmd_rx.try_recv().unwrap(), Command::LoadBuiltin { deck: d, stem: s } if d == deck && s == stem)
            );
            assert!(fixture.rt.cmd_rx.is_empty());

            fixture.app.snap.selected_deck = deck as usize;
            time += 1.0;
            let output = crate_frame(&ctx, &mut fixture.app, time, Vec::new());
            let row = label_center(&output, title);
            click(&ctx, &mut fixture.app, row, time + 0.1);
            assert!(
                fixture.rt.cmd_rx.is_empty(),
                "single row click must only select"
            );
            click(&ctx, &mut fixture.app, row, time + 0.2);
            assert!(
                matches!(fixture.rt.cmd_rx.try_recv().unwrap(), Command::LoadBuiltin { deck: d, stem: s } if d == deck && s == stem)
            );
            assert!(fixture.rt.cmd_rx.is_empty());
            assert!(matches!(
                fixture.decoder_jobs.try_recv(),
                Err(mpsc::TryRecvError::Empty)
            ));
            time += 1.0;
        }
    }
}
