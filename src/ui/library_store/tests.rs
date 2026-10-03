use super::*;
use crate::engine::{decode::decode_audio, media_source::BuiltinStem};
use crate::library::Catalog;
use crate::ui::test_support::{label_center, Fixture};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

struct Files(PathBuf);
impl Files {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "omatainer-dj-gui85-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn store(&self) -> PathBuf {
        self.0.join("saved/library.json")
    }
    fn wave(&self, name: &str, frames: u32, channels: u16) -> PathBuf {
        let path = self.0.join(name);
        let sr = 48000u32;
        let size = frames * u32::from(channels) * 2;
        let mut bytes = b"RIFF".to_vec();
        bytes.extend((36 + size).to_le_bytes());
        bytes.extend(b"WAVEfmt ");
        bytes.extend(16u32.to_le_bytes());
        bytes.extend(1u16.to_le_bytes());
        bytes.extend(channels.to_le_bytes());
        bytes.extend(sr.to_le_bytes());
        bytes.extend((sr * u32::from(channels) * 2).to_le_bytes());
        bytes.extend((channels * 2).to_le_bytes());
        bytes.extend(16u16.to_le_bytes());
        bytes.extend(b"data");
        bytes.extend(size.to_le_bytes());
        bytes.resize(bytes.len() + size as usize, 0);
        std::fs::write(&path, bytes).unwrap();
        path
    }
}
impl Drop for Files {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn wait(mut f: impl FnMut() -> bool) {
    let until = Instant::now() + Duration::from_secs(5);
    while !f() {
        assert!(Instant::now() < until, "library test timeout");
        std::thread::sleep(Duration::from_millis(1));
    }
}
fn settle(f: &mut Fixture) {
    wait(|| {
        f.rt.process(&mut []);
        f.app.poll_load_receipts();
        f.app.poll_library_scan();
        f.rt.process(&mut []);
        f.app.poll_load_receipts();
        !f.app.library_metadata.active() && !f.app.library_scan.active()
    });
}
fn start(f: &mut Fixture, path: PathBuf) {
    f.app.start_library_store(path);
    settle(f);
    assert_eq!(f.app.library_metadata.label(), "DJ library saved");
}
fn select(f: &mut Fixture, source: &LibSource) {
    f.app.refresh_library_view();
    f.app.lib_sel = f
        .app
        .library_view
        .indices
        .iter()
        .position(|&i| &f.app.library[i].source == source)
        .unwrap();
    f.app.refresh_library_view();
}
fn metadata(title: &str) -> Metadata {
    Metadata {
        title: title.into(),
        artist: "fixture artist".into(),
        bpm: Bpm::new(120.0, Origin::User),
        key: "Am".into(),
        duration: Some(2.0),
        last_play: None,
    }
}
fn prepared() -> Preparation {
    Preparation {
        grid: None,        cue: 0.25,
        hotcue_styles: [crate::engine::cue_metadata::Style::default(); 8],
        hotcues: [Some(0.5), None, None, None, None, None, None, Some(1.5)],
        loop_region: Some(crate::engine::preparation::Loop {
            start: 0.5,
            length: 0.5,
            enabled: true,
        }),
    }
}
fn frame(
    ctx: &egui::Context,
    f: &mut Fixture,
    time: f64,
    events: Vec<egui::Event>,
) -> egui::FullOutput {
    ctx.run(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1100.0, 700.0))),
            time: Some(time),
            events,
            ..Default::default()
        },
        |ctx| {
            f.app.library_store_ui(ctx);
            let theme = f.app.theme.clone();
            egui::CentralPanel::default().show(ctx, |ui| f.app.crate_row(ui, &theme));
        },
    )
}
fn import_button(f: &mut Fixture, path: &std::path::Path) {
    f.app.library_import_open = true;
    f.app.library_import_path = path.to_str().unwrap().into();
    let ctx = egui::Context::default();
    frame(&ctx, f, 0.0, vec![]);
    let output = frame(&ctx, f, 0.01, vec![]);
    let pos = label_center(&output, "Import catalog");
    for (pressed, time) in [(true, 0.02), (false, 0.03)] {
        frame(
            &ctx,
            f,
            time,
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                },
            ],
        );
    }
    assert_eq!(f.app.status, "DJ library import queued");
    settle(f);
}
fn load(f: &mut Fixture, path: &PathBuf) {
    select(f, &LibSource::File(path.clone()));
    f.app.load_sel(0);
    let (_, job) = f.decoder_jobs.recv_timeout(Duration::from_secs(2)).unwrap();
    assert_eq!(&job, path);
    f.decoder_results
        .send((0, decode_audio(path).map_err(|e| e)))
        .unwrap();
    f.poll_loads();
    f.rt.process(&mut []);
}
fn await_closed(path: &std::path::Path) {
    wait(|| crate::library::Store::open(path.into()).is_ok());
}

#[test]
fn actual_gui_mixed_import_restart_and_real_load_restore_all_ids_and_preparation() {
    let files = Files::new();
    let a = files.wave("mono.wav", 96000, 1);
    let b = files.wave("stereo.wav", 96000, 2);
    let sources = vec![
        LibSource::File(a.clone()),
        LibSource::File(b.clone()),
        LibSource::Removable {
            volume_id: "usb-uuid".into(),
            relative_path: "mono.wav".into(),
        },
        LibSource::Provider {
            provider: "fixture-provider".into(),
            media_id: "mono.wav".into(),
        },
    ];
    let mut catalog = Catalog::default();
    for (i, source) in sources.iter().enumerate() {
        let fingerprint = match source {
            LibSource::File(path) => FileFingerprint::read(path),
            _ => None,
        };
        catalog
            .upsert(
                source.clone(),
                fingerprint,
                metadata(&format!("Imported {i}")),
            )
            .unwrap()
            .preparation = prepared();
    }
    for item in builtin_crate_items() {
        catalog
            .upsert(item.source.clone(), None, item.stored_metadata())
            .unwrap();
    }
    catalog
        .upsert(
            LibSource::Builtin(BuiltinStem::Drums),
            None,
            builtin_crate_items().remove(0).stored_metadata(),
        )
        .unwrap()
        .preparation = prepared();
    let import = files.0.join("import.json");
    std::fs::write(&import, serde_json::to_vec(&catalog).unwrap()).unwrap();
    let mut f = Fixture::new(64);
    start(&mut f, files.store());
    import_button(&mut f, &import);
    assert_eq!(f.app.library_metadata.label(), "DJ library saved");
    for source in &sources {
        assert_eq!(
            f.app.library_metadata.catalog.track(source),
            catalog.track(source)
        );
        select(&mut f, source);
        if !matches!(source, LibSource::File(_)) {
            f.app.load_sel(0);
            wait(|| {f.app.poll_loads();matches!(&f.app.loads[0].as_ref().unwrap().phase,Phase::Failed(_))});
            assert!(matches!(&f.app.loads[0].as_ref().unwrap().phase,Phase::Failed(message)
                if if matches!(source,LibSource::Removable {..}) {message.contains("offline")} else {message.contains("namespace")}));
            assert!(f.decoder_jobs.try_recv().is_err());
        }
    }
    for path in [&a, &b] {
        load(&mut f, path);
        assert_eq!(f.rt.decks[0].cue_pos, 12000.0);
        assert_eq!(f.rt.decks[0].hotcues[0].pos, 24000.0);
        assert!(f.rt.decks[0].hotcues[7].set);
        assert_eq!(f.rt.decks[0].loop_start, 24000.0);
        assert_eq!(f.rt.decks[0].loop_len, 24000.0);
        assert!(f.rt.decks[0].loop_on);
        assert!(!f.rt.decks[0].playing);
        settle(&mut f);
    }
    let saved = crate::library::read(&files.store()).unwrap();
    drop(f);
    await_closed(&files.store());
    let mut reopened = Fixture::new(64);
    start(&mut reopened, files.store());
    assert_eq!(reopened.app.library_metadata.catalog.tracks, saved.tracks);
    for source in &sources {
        assert_eq!(
            reopened
                .app
                .library_metadata
                .catalog
                .track(source)
                .unwrap()
                .id,
            catalog.track(source).unwrap().id
        );
    }
    load(&mut reopened, &a);
    assert_eq!(reopened.rt.decks[0].cue_pos, 12000.0);
    settle(&mut reopened);
}

#[test]
fn renderer_edits_replacement_before_poll_and_changed_bytes_preserve_exact_version() {
    let files = Files::new();
    let path = files.wave("track.wav", 96000, 1);
    let source = LibSource::File(path.clone());
    let mut f = Fixture::new(64);
    start(&mut f, files.store());
    f.app
        .library_scan
        .start(vec![files.0.clone()], f.app.library.clone());
    settle(&mut f);
    load(&mut f, &path);
    let old = FileFingerprint::read(&path);
    f.rt.apply(Command::DeckSeek {
        deck: 0,
        frac: 0.25,
    });
    f.rt.apply(Command::DeckHotCue {
        deck: 0,
        pad: 3,
        del: false,
    });
    f.rt.apply(Command::DeckLoopIn { deck: 0 });
    f.rt.apply(Command::DeckSeek {
        deck: 0,
        frac: 0.75,
    });
    f.rt.apply(Command::DeckLoopOut { deck: 0 });
    f.rt.apply(Command::DeckPlay { deck: 0 });
    f.rt.process(&mut [0.0; 2]);
    let expected = f.app.loads[0]
        .as_ref()
        .unwrap()
        .receipt
        .as_ref()
        .unwrap()
        .preparation()
        .unwrap()
        .1;
    f.rt.apply(Command::DeckUnload { deck: 0 }); // no intervening GUI observation
    settle(&mut f);
    let record = f.app.library_metadata.catalog.track(&source).unwrap();
    let id = record.id.clone();
    let version = f
        .app
        .library_metadata
        .catalog
        .version(&source, old)
        .unwrap();
    assert_eq!(version.preparation, expected);
    assert!(version.metadata.last_play.is_some());
    assert_eq!(version.metadata.duration, Some(2.0));
    files.wave("track.wav", 48000, 2);
    let new = FileFingerprint::read(&path);
    assert_ne!(old, new);
    f.app
        .library_scan
        .start(vec![files.0.clone()], f.app.library.clone());
    settle(&mut f);
    load(&mut f, &path);
    assert_eq!(f.rt.decks[0].cue_pos, 0.0);
    assert!(f.rt.decks[0].hotcues.iter().all(|cue| !cue.set));
    settle(&mut f);
    let catalog = &f.app.library_metadata.catalog;
    let record = catalog.track(&source).unwrap();
    assert_eq!(record.id, id);
    assert_eq!(record.versions.len(), 2);
    assert_eq!(catalog.version(&source, old).unwrap().preparation, expected);
    assert_eq!(
        catalog.version(&source, new).unwrap().preparation,
        Preparation::default()
    );
    drop(f);
    await_closed(&files.store());
    let saved = crate::library::read(&files.store()).unwrap();
    assert_eq!(saved.track(&source).unwrap().id, id);
    assert_eq!(saved.version(&source, old).unwrap().preparation, expected);
}

#[test]
fn malformed_store_and_import_remain_visible_and_cannot_be_replaced_by_scan_defaults() {
    let files = Files::new();
    let path = files.wave("track.wav", 4800, 1);
    std::fs::create_dir_all(files.store().parent().unwrap()).unwrap();
    let original = b"{\"schema\":999,\"prepared_future_data\":[1,2,3]}";
    std::fs::write(files.store(), original).unwrap();
    let mut f = Fixture::new(64);
    f.app.start_library_store(files.store());
    settle(&mut f);
    assert!(f
        .app
        .library_metadata
        .label()
        .contains("original store preserved"));
    f.app
        .library_scan
        .start(vec![files.0.clone()], f.app.library.clone());
    settle(&mut f);
    f.app.library_metadata.retry_save();
    settle(&mut f);
    assert_eq!(std::fs::read(files.store()).unwrap(), original);
    assert!(f.app.library_metadata.label().contains("schema"));
    let ctx = egui::Context::default();
    let output = frame(&ctx, &mut f, 0.0, vec![]);
    assert!(output.shapes.iter().any(|shape| matches!(&shape.shape, egui::epaint::Shape::Text(text) if text.galley.text().contains("original store preserved"))));
    drop(f);
    await_closed_lock_only(&files.store());
    let other = files.0.join("valid/library.json");
    let mut f = Fixture::new(64);
    start(&mut f, other.clone());
    let before = std::fs::read(&other).unwrap();
    import_button(&mut f, &files.store());
    assert!(f.app.library_metadata.label().contains("import rejected"));
    assert_eq!(std::fs::read(other).unwrap(), before);
    assert!(path.is_file());
}
fn await_closed_lock_only(path: &std::path::Path) {
    wait(|| {
        std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(path.with_extension("lock"))
            .unwrap()
            .try_lock()
            .is_ok()
    });
}

#[test]
fn initial_builtin_preparation_restores_without_overwriting_early_user_edits() {
    let files = Files::new();
    let source = LibSource::Builtin(BuiltinStem::Drums);
    {
        let mut store = crate::library::Store::open(files.store()).unwrap();
        store
            .catalog
            .upsert(source.clone(), None, metadata("Drums (session)"))
            .unwrap()
            .preparation = prepared();
        store.save().unwrap();
    }
    let mut f = Fixture::new(64);
    start(&mut f, files.store());
    assert_eq!(f.rt.decks[0].cue_pos, prepared().cue * 48000.0);
    settle(&mut f);
    assert_eq!(
        f.app
            .library_metadata
            .catalog
            .version(&source, None)
            .unwrap()
            .preparation,
        prepared()
    );
    drop(f);
    await_closed(&files.store());
    let mut f = Fixture::new(64);
    f.app.start_library_store(files.store());
    f.rt.apply(Command::DeckSeek { deck: 0, frac: 0.1 });
    let edited = f.rt.decks[0].cue_pos;
    settle(&mut f);
    assert_eq!(f.rt.decks[0].cue_pos, edited);
    assert_eq!(
        f.app
            .library_metadata
            .catalog
            .version(&source, None)
            .unwrap()
            .preparation
            .cue,
        edited / 48000.0
    );
}

#[test]
fn bounded_background_save_keeps_actual_ui_controls_live_and_close_waits_for_fifo_and_durability() {
    use std::sync::mpsc;
    let files = Files::new();
    let mut f = Fixture::new(64);
    let (entered, entry) = mpsc::sync_channel(1);
    let (release, released) = mpsc::sync_channel(1);
    let mut first = true;
    f.app.library_metadata = library_metadata::Metadata::with_hook(files.store(), move || {
        if first {
            first = false;
            entered.send(()).unwrap();
            released.recv().unwrap();
        }
    });
    f.app.poll_library_metadata();
    entry.recv_timeout(Duration::from_secs(2)).unwrap();
    let ctx = egui::Context::default();
    let now = Instant::now();
    for frame_index in 0..24 {
        let output = frame(&ctx, &mut f, frame_index as f64 / 60.0, vec![]);
        assert!(!output.shapes.is_empty());
        assert!(f
            .app
            .submit(Command::Master(0.25 + frame_index as f32 * 0.01)));
        assert!(f.app.submit(Command::DeckSeek {
            deck: 0,
            frac: frame_index as f32 / 100.0
        }));
        f.rt.process(&mut [0.0; 256]);
    }
    assert!(
        now.elapsed() < Duration::from_secs(2),
        "GUI blocked by held save worker"
    );
    assert!(matches!(f.app.prepare_library_close(), CloseState::Pending));
    f.rt.process(&mut []);
    assert!(matches!(f.app.prepare_library_close(), CloseState::Pending));
    release.send(()).unwrap();
    wait(|| {
        f.rt.process(&mut []);
        matches!(f.app.prepare_library_close(), CloseState::Ready)
    });
    let catalog = crate::library::read(&files.store()).unwrap();
    let saved = catalog
        .version(&LibSource::Builtin(BuiltinStem::Drums), None)
        .unwrap();
    assert!((saved.preparation.cue - f.rt.decks[0].cue_pos / 48000.0).abs() < 1e-12);
    assert_eq!(f.rt.master, 0.48);
    f.app.cancel_library_close();
    // Save failure is persistent until explicit retry, and the close coordinator
    // cannot report Ready merely because the worker queue became empty.
    std::fs::write(files.store(), b"{external replacement}").unwrap();
    f.rt.apply(Command::DeckSeek { deck: 0, frac: 0.4 });
    wait(|| {
        f.rt.process(&mut []);
        matches!(f.app.prepare_library_close(), CloseState::Failed(_))
    });
    assert!(f.app.library_metadata.label().contains("outside"));
    assert_eq!(
        std::fs::read(files.store()).unwrap(),
        b"{external replacement}"
    );
}

#[test]
fn invalid_import_does_not_drop_simultaneous_preparation_and_unapplied_load_never_overwrites_it() {
    let files = Files::new();
    let mut f = Fixture::new(64);
    start(&mut f, files.store());
    let bad = files.0.join("bad.json");
    std::fs::write(&bad, b"{bad import").unwrap();
    f.rt.apply(Command::DeckSeek { deck: 0, frac: 0.2 });
    let cue = f.rt.decks[0].cue_pos;
    assert!(f.app.library_metadata.import(bad));
    settle(&mut f);
    assert!(f.app.library_metadata.label().contains("import rejected"));
    let saved = crate::library::read(&files.store()).unwrap();
    assert_eq!(
        saved
            .version(&LibSource::Builtin(BuiltinStem::Drums), None)
            .unwrap()
            .preparation
            .cue,
        cue / 48000.0
    );
    f.app.load_source(
        0,
        Some(&Selection {
            source: LibSource::Builtin(BuiltinStem::Harmony),
            title: "cancelled".into(),
        }),
    );
    f.app.loads[0]
        .as_ref()
        .unwrap()
        .receipt
        .as_ref()
        .unwrap()
        .cancel_pending();
    f.rt.process(&mut []);
    settle(&mut f);
    let saved = crate::library::read(&files.store()).unwrap();
    assert_eq!(
        saved
            .version(&LibSource::Builtin(BuiltinStem::Drums), None)
            .unwrap()
            .preparation
            .cue,
        cue / 48000.0
    );
    assert_eq!(
        saved
            .version(&LibSource::Builtin(BuiltinStem::Harmony), None)
            .unwrap()
            .preparation,
        Preparation::default()
    );
}

#[test]
fn actual_egui_close_is_cancelled_until_preceding_renderer_edits_are_durable() {
    let files = Files::new();
    let mut f = Fixture::new(64);
    start(&mut f, files.store());
    f.app.submit(Command::DeckSeek { deck: 0, frac: 0.3 });
    let ctx = egui::Context::default();
    // Apply the edit first so the actual project coordinator presents its
    // unsaved-project decision before the independent catalog durability step.
    f.rt.process(&mut []);
    let close_frame =
        |ctx: &egui::Context, f: &mut Fixture, close: bool, time: f64, events: Vec<egui::Event>| {
            let mut input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1000.0, 700.0))),
                time: Some(time),
                events,
                ..Default::default()
            };
            if close {
                input
                    .viewports
                    .get_mut(&egui::ViewportId::ROOT)
                    .unwrap()
                    .events
                    .push(egui::ViewportEvent::Close);
            }
            let output = ctx.run(input, |ctx| f.app.update_frame(ctx));
            f.rt.process(&mut [0.0; 128]);
            output
        };
    let first = close_frame(&ctx, &mut f, true, 0.0, vec![]);
    let commands = &first.viewport_output[&egui::ViewportId::ROOT].commands;
    assert!(commands
        .iter()
        .any(|cmd| matches!(cmd, egui::ViewportCommand::CancelClose)));
    assert!(!commands
        .iter()
        .any(|cmd| matches!(cmd, egui::ViewportCommand::Close)));
    f.rt.process(&mut []);
    let expected = f.rt.decks[0].cue_pos / 48000.0;
    close_frame(&ctx, &mut f, false, 0.01, vec![]);
    let output = close_frame(&ctx, &mut f, false, 0.02, vec![]);
    let discard = label_center(&output, "Discard changes");
    for (time, pressed) in [(0.03, true), (0.04, false)] {
        close_frame(
            &ctx,
            &mut f,
            false,
            time,
            vec![
                egui::Event::PointerMoved(discard),
                egui::Event::PointerButton {
                    pos: discard,
                    button: PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                },
            ],
        );
    }
    let mut time = 0.1;
    wait(|| {
        time += 0.01;
        let output = close_frame(&ctx, &mut f, false, time, vec![]);
        output.viewport_output[&egui::ViewportId::ROOT]
            .commands
            .iter()
            .any(|cmd| matches!(cmd, egui::ViewportCommand::Close))
    });
    let catalog = crate::library::read(&files.store()).unwrap();
    assert_eq!(
        catalog
            .version(&LibSource::Builtin(BuiltinStem::Drums), None)
            .unwrap()
            .preparation
            .cue,
        expected
    );
}

#[test]
fn initial_preparation_retries_full_admission_on_later_actual_ui_frame() {
    let files = Files::new();
    {
        let mut store = crate::library::Store::open(files.store()).unwrap();
        store
            .catalog
            .upsert(
                LibSource::Builtin(BuiltinStem::Drums),
                None,
                metadata("Drums (session)"),
            )
            .unwrap()
            .preparation = prepared();
        store.save().unwrap();
    }
    let mut f = Fixture::new(64);
    f.app.start_library_store(files.store());
    let rejected_import = files.0.join("invalid-import.json");
    std::fs::write(&rejected_import, b"{invalid import").unwrap();
    assert!(f.app.library_metadata.import(rejected_import));
    while f.app.engine.send(Command::Master(0.6)).is_ok() {}
    wait(|| {
        f.app.poll_library_metadata();
        !f.app.library_metadata.active()
    });
    assert!(f.app.library_metadata.label().contains("import rejected"));
    assert!(
        f.app.library_metadata.durable,
        "the valid saved catalog remains usable"
    );
    assert!(
        !f.app.library_initialized,
        "a rejected restore remains pending"
    );
    assert_eq!(f.rt.decks[0].cue_pos, 0.0);
    while !f.rt.cmd_rx.is_empty() {
        f.rt.process(&mut []);
    }
    let ctx = egui::Context::default();
    let _ = ctx.run(Default::default(), |ctx| f.app.update_frame(ctx));
    f.rt.process(&mut []);
    assert!(f.app.library_initialized);
    assert_eq!(f.rt.decks[0].cue_pos, prepared().cue * 48_000.0);
    let preparation = f.app.engine.initial_playback[0].as_ref().unwrap().preparation().unwrap();
    let _ = ctx.run(Default::default(), |ctx| f.app.update_frame(ctx));
    f.rt.process(&mut []);
    assert_eq!(
        f.app.engine.initial_playback[0].as_ref().unwrap().preparation().unwrap(),
        preparation,
        "an admitted restore is not replayed on later frames"
    );
}
