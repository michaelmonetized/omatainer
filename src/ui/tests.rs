use super::test_support::Fixture;
use super::*;
use std::os::unix::fs::DirBuilderExt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::Duration;

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
                    matches!(command, Command::DeckLoadRequested { deck: d, media: Media::Builtin(s), .. } if d == deck && s == stem)
                );
                assert!(fixture.rt.cmd_rx.is_empty());
                assert!(matches!(
                    fixture.decoder_jobs.try_recv(),
                    Err(mpsc::TryRecvError::Empty)
                ));
                assert_eq!(
                    fixture.app.status,
                    format!("queued {title} → {} · waiting for audio engine", (b'A' + deck) as char)
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
            bpm: Bpm::UNKNOWN,
            fingerprint: None,
            key: String::new(),
            length: None,
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
    Arc::make_mut(&mut fixture.app.library).push(wave.item());
    fixture.app.lib_filter = "real WAV".into();
    for deck in 0..2 {
        fixture.app.load_sel(deck);
        assert!(fixture.rt.cmd_rx.is_empty());
        let (job_deck, path) = fixture
            .decoder_jobs
            .recv_timeout(Duration::from_secs(3))
            .unwrap();
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
        fixture.poll_loads();
        let command = fixture.rt.cmd_rx.try_recv().unwrap();
        assert!(
            matches!(&command, Command::DeckLoadRequested { media: Media::Decoded { token: request, audio }, .. } if request.deck == deck && audio.path == wave.0.to_string_lossy())
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
        assert!(fixture.app.status.starts_with("queued real WAV"));
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
    assert!(fixture.app.status.contains("Load was not accepted"));
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
    assert!(fixture.app.status.contains("Load was not accepted"));
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
    fixture.app.library = Arc::new(vec![wave.item()]);
    fixture.app.loader.take();
    fixture.app.load_sel(0);
    assert!(fixture.app.status.contains("decoder is unavailable"));
    assert!(fixture.rt.cmd_rx.is_empty());
    fixture.app.lib_filter = "no matching media".into();
    fixture.app.status = "unchanged".into();
    fixture.app.load_sel(1);
    assert!(fixture.app.status.contains("no library item selected"));
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
        fixture.app.load_file(deck, wave.0.clone(), "corrupt WAV");
        fixture
            .decoder_jobs
            .recv_timeout(Duration::from_secs(3))
            .unwrap();
        fixture.decoder_results.send((deck, result)).unwrap();
        fixture.poll_loads();
        assert!(fixture.rt.cmd_rx.is_empty());
        assert!(fixture.app.status.contains("Incomplete audio"));
        assert!(fixture.app.status.contains("media was not loaded"));
        assert_load_status_is_painted(&mut fixture.app, "Incomplete audio");
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
    fixture.app.load_file(0, path.clone(), "uncertain MP3");
    fixture
        .decoder_jobs
        .recv_timeout(Duration::from_secs(3))
        .unwrap();
    fixture.decoder_results.send((0, Ok(report))).unwrap();
    fixture.poll_loads();
    assert!(fixture.app.status.contains("length unverified"));
    assert!(fixture
        .app
        .status
        .contains("incomplete media cannot be ruled out"));
    assert_load_status_is_painted(&mut fixture.app, "length unverified");
    let command = fixture.rt.cmd_rx.try_recv().unwrap();
    assert!(matches!(&command, Command::DeckLoadRequested { media: Media::Decoded { token: request, .. }, .. } if request.deck == 0));
    fixture.rt.apply(command);
    assert_eq!(
        fixture.rt.decks[0].audio.as_ref().unwrap().path,
        path.to_string_lossy()
    );
}

fn assert_load_status_is_painted(app: &mut App, expected: &str) {
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
                matches!(fixture.rt.cmd_rx.try_recv().unwrap(), Command::DeckLoadRequested { deck: d, media: Media::Builtin(s), .. } if d == deck && s == stem)
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
                matches!(fixture.rt.cmd_rx.try_recv().unwrap(), Command::DeckLoadRequested { deck: d, media: Media::Builtin(s), .. } if d == deck && s == stem)
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

#[test]
fn newer_file_selection_discards_delayed_old_success_and_error_before_ui_publication() {
    for fail_old in [false, true] {
        let a = WaveFile::new();
        let b = WaveFile::new();
        let mut fixture = Fixture::new(32);
        let mut a_item = a.item();
        a_item.title = "choice A".into();
        let mut b_item = b.item();
        b_item.title = "choice B".into();
        fixture.app.library = Arc::new(vec![a_item, b_item]);
        fixture.app.lib_sel = 0;
        fixture.app.load_sel(0);
        let (_, path_a) = fixture
            .decoder_jobs
            .recv_timeout(Duration::from_secs(3))
            .unwrap();
        fixture.app.lib_sel = 1;
        fixture.app.load_sel(0);
        let old_result = if fail_old {
            crate::engine::dsp::decode_audio(&path_a.with_file_name("missing.wav"))
        } else {
            crate::engine::dsp::decode_audio(&path_a)
        };
        fixture.decoder_results.send((0, old_result)).unwrap();
        // Starting B proves the worker has already retired A, including its
        // result. It must not appear as either an error or an engine command.
        let (_, path_b) = fixture
            .decoder_jobs
            .recv_timeout(Duration::from_secs(3))
            .unwrap();
        assert_eq!(path_b, b.0);
        fixture.app.poll_loads();
        assert!(fixture.rt.cmd_rx.is_empty());
        assert_eq!(fixture.app.status, "loading choice B → A");
        fixture
            .decoder_results
            .send((0, crate::engine::dsp::decode_audio(&path_b)))
            .unwrap();
        fixture.poll_loads();
        let command = fixture.rt.cmd_rx.try_recv().unwrap();
        assert!(
            matches!(&command, Command::DeckLoadRequested { media: Media::Decoded { token: request, audio }, .. } if request.deck == 0 && audio.path == b.0.to_string_lossy())
        );
        fixture.rt.apply(command);
        assert_eq!(
            fixture.rt.decks[0].audio.as_ref().unwrap().path,
            b.0.to_string_lossy()
        );
    }
}

#[test]
fn builtin_selection_and_real_platter_unload_invalidate_active_file_decodes() {
    for unload in [false, true] {
        let wave = WaveFile::new();
        let mut fixture = Fixture::new(32);
        fixture.app.load_file(0, wave.0.clone(), "obsolete A");
        fixture
            .decoder_jobs
            .recv_timeout(Duration::from_secs(3))
            .unwrap();
        if unload {
            shift_click_platter(&mut fixture.app);
            assert_eq!(fixture.app.status, "queued unload → A");
        } else {
            fixture.app.lib_filter = "Harmony".into();
            fixture.app.load_sel(0);
        }
        let replacement = fixture.rt.cmd_rx.try_recv().unwrap();
        if unload {
            assert!(matches!(replacement, Command::DeckUnload { deck: 0 }));
        } else {
            assert!(matches!(
                replacement,
                Command::DeckLoadRequested { deck: 0, media: Media::Builtin(1), .. }
            ));
        }
        fixture.rt.apply(replacement);
        let expected = fixture.rt.decks[0].audio.clone();
        fixture.app.load_file(1, wave.0.clone(), "barrier B");
        fixture
            .decoder_results
            .send((0, crate::engine::dsp::decode_audio(&wave.0)))
            .unwrap();
        let (deck, _) = fixture
            .decoder_jobs
            .recv_timeout(Duration::from_secs(3))
            .unwrap();
        assert_eq!(
            deck, 1,
            "B starts only after the old A result was discarded"
        );
        fixture.app.poll_loads();
        assert!(fixture.rt.cmd_rx.is_empty());
        fixture
            .decoder_results
            .send((1, crate::engine::dsp::decode_audio(&wave.0)))
            .unwrap();
        fixture.poll_loads();
        let completion = fixture.rt.cmd_rx.try_recv().unwrap();
        assert!(matches!(&completion, Command::DeckLoadRequested { media: Media::Decoded { token: request, .. }, .. } if request.deck == 1));
        fixture.rt.apply(completion);
        match expected {
            None => assert!(fixture.rt.decks[0].audio.is_none()),
            Some(audio) => assert!(Arc::ptr_eq(
                &audio,
                fixture.rt.decks[0].audio.as_ref().unwrap()
            )),
        }
    }
}

fn shift_click_platter(app: &mut App) {
    let ctx = egui::Context::default();
    let theme = app.theme.clone();
    let snap = app.snap.decks[0].clone();
    let mut center = Pos2::ZERO;
    for frame in 0..3 {
        let modifiers = egui::Modifiers {
            shift: true,
            ..Default::default()
        };
        let events = if frame == 0 {
            Vec::new()
        } else {
            vec![
                egui::Event::PointerMoved(center),
                egui::Event::PointerButton {
                    pos: center,
                    button: PointerButton::Primary,
                    pressed: frame == 1,
                    modifiers,
                },
            ]
        };
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(300.0, 300.0))),
                time: Some(frame as f64 * 0.1),
                events,
                modifiers,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    app.platter_col(ui, &theme, 0, &snap, theme.accent, 140.0)
                });
            },
        );
        if frame == 0 {
            center = output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::epaint::Shape::Circle(circle) if circle.radius > 50.0 => {
                        Some(circle.center)
                    }
                    _ => None,
                })
                .expect("real platter circle must be painted");
        }
    }
}

#[test]
fn actual_dropped_file_supersedes_completion_already_queued_for_audio() {
    let a = WaveFile::new();
    let b = WaveFile::new();
    let mut fixture = Fixture::new(32);
    let before = fixture.rt.decks[0].audio.clone().unwrap();
    fixture.app.load_file(0, a.0.clone(), "A");
    fixture
        .decoder_jobs
        .recv_timeout(Duration::from_secs(3))
        .unwrap();
    fixture
        .decoder_results
        .send((0, crate::engine::dsp::decode_audio(&a.0)))
        .unwrap();
    fixture.poll_loads();
    assert_eq!(fixture.rt.cmd_rx.len(), 1);
    let ctx = egui::Context::default();
    let output = ctx.run(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1440.0, 900.0))),
            time: Some(0.0),
            events: vec![egui::Event::PointerMoved(Pos2::new(100.0, 100.0))],
            dropped_files: vec![egui::DroppedFile {
                path: Some(b.0.clone()),
                ..Default::default()
            }],
            ..Default::default()
        },
        |ctx| fixture.app.update_frame(ctx),
    );
    assert!(!output.shapes.is_empty());
    fixture.rt.process(&mut [0.0; 2]);
    assert!(
        Arc::ptr_eq(&before, fixture.rt.decks[0].audio.as_ref().unwrap()),
        "queued obsolete A must never become current"
    );
    let (deck, path) = fixture
        .decoder_jobs
        .recv_timeout(Duration::from_secs(3))
        .unwrap();
    assert_eq!(deck, 0);
    assert_eq!(path, b.0);
    fixture
        .decoder_results
        .send((0, crate::engine::dsp::decode_audio(&path)))
        .unwrap();
    fixture.poll_loads();
    let latest = fixture.rt.cmd_rx.try_recv().unwrap();
    fixture.rt.apply(latest);
    assert_eq!(
        fixture.rt.decks[0].audio.as_ref().unwrap().path,
        b.0.to_string_lossy()
    );
}
