use super::*;
use crate::engine::{
    decode::{decode_audio, DecodedAudio},
    dsp::Sample,
    test_alloc,
};
use crate::ui::test_support::{crate_frame, Fixture};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

struct Files(PathBuf);
impl Files {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "omatainer-play-history-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn wave(&self, name: &str, frames: u32) -> PathBuf {
        let path = self.0.join(name);
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
            bytes.extend(10000i16.to_le_bytes());
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

fn item(path: &PathBuf, title: &str) -> LibItem {
    LibItem {
        source: LibSource::File(path.clone()),
        title: title.into(),
        artist: "fixture".into(),
        bpm: Bpm::hint(120.0),
        fingerprint: FileFingerprint::read(path),
        key: "C".into(),
        length: None,
        last_play: None,
    }
}
fn render(f: &mut Fixture, frames: usize) -> Vec<f32> {
    let mut out = vec![0.0; frames * 2];
    f.rt.process(&mut out);
    f.app.poll_play_history();
    out
}
fn history(f: &Fixture, source: &LibSource) -> Option<SystemTime> {
    let item = f
        .app
        .library
        .iter()
        .find(|item| &item.source == source)
        .unwrap();
    f.app.item_last_play(item)
}
fn load_receipt(f: &Fixture, deck: usize) -> Receipt {
    f.app.loads[deck]
        .as_ref()
        .unwrap()
        .receipt
        .as_ref()
        .unwrap()
        .clone()
}
fn file_load(f: &mut Fixture, path: &PathBuf, deck: u8) {
    f.app.load_file(deck, path.clone(), "captured file");
    f.poll_loads();
    assert!(matches!(
        f.app.loads[deck as usize].as_ref().unwrap().phase,
        Phase::Queued
    ));
    f.rt.process(&mut []);
    assert_eq!(load_receipt(f, deck as usize).state(), State::Current);
}
fn builtin_load(f: &mut Fixture, stem: BuiltinStem, deck: u8) {
    f.app.load_source(
        deck,
        Some(&Selection {
            source: LibSource::Builtin(stem),
            title: "captured builtin".into(), fingerprint: None, }),
    );
    f.rt.process(&mut []);
    assert_eq!(load_receipt(f, deck as usize).state(), State::Current);
}
fn fake_report(path: &PathBuf) -> DecodedAudio {
    DecodedAudio {
        sample: Sample {
            name: "controlled decoder".into(),
            sr: 48_000,
            ch: 1,
            data: vec![0.25; 4800],
            peaks: vec![[0.25; 3]; 8].into(),
            bpm: 120.0,
            path: path.to_string_lossy().into(),
        },
        diagnostics: Default::default(),
    }
}
fn visible(output: &egui::FullOutput, expected: &str) -> bool {
    output.shapes.iter().any(|shape| {
        matches!(&shape.shape,
        egui::epaint::Shape::Text(label) if label.galley.text() == expected)
    })
}

#[test]
fn failed_and_cancelled_decoder_jobs_never_credit_history() {
    let files = Files::new();
    let missing = files.0.join("missing.wav");
    let corrupt = files.0.join("corrupt.wav");
    std::fs::write(&corrupt, b"not audio").unwrap();
    let valid = files.wave("valid.wav", 4800);
    let mut f = Fixture::new(32);
    f.app.library = Arc::new(vec![
        item(&missing, "missing"),
        item(&corrupt, "corrupt"),
        item(&valid, "valid"),
    ]);
    f.app.loader = Some(Loader::start().unwrap());
    for path in [&missing, &corrupt] {
        f.app.load_file(0, path.clone(), "failure");
        f.poll_loads();
        assert!(matches!(
            f.app.loads[0].as_ref().unwrap().phase,
            Phase::Failed(_)
        ));
        assert!(history(&f, &LibSource::File(path.clone())).is_none());
        assert!(f.rt.cmd_rx.is_empty());
    }
    // The controlled decoder completes the real valid WAV only after this
    // load has been cancelled and replaced by an independently loaded stem.
    let mut f = Fixture::new(32);
    f.app.library = Arc::new(vec![item(&valid, "valid")]);
    f.app.load_file(0, valid.clone(), "cancelled file");
    assert_eq!(
        f.decoder_jobs
            .recv_timeout(Duration::from_secs(3))
            .unwrap()
            .1,
        valid
    );
    let token = f.app.loads[0].as_ref().unwrap().token.clone().unwrap();
    builtin_load(&mut f, BuiltinStem::Harmony, 0);
    f.decoder_results
        .send((0, Ok(decode_audio(&valid).unwrap())))
        .unwrap();
    assert!(!token.is_current());
    // Drive the worker onward to prove it observed the cancelled completion.
    f.app.load_file(1, valid.clone(), "next paused load");
    assert_eq!(
        f.decoder_jobs
            .recv_timeout(Duration::from_secs(3))
            .unwrap()
            .0,
        1
    );
    f.decoder_results
        .send((1, Ok(decode_audio(&valid).unwrap())))
        .unwrap();
    f.poll_loads();
    render(&mut f, 64);
    assert!(history(&f, &LibSource::File(valid)).is_none());
    assert!(f.app.last_played.by_source.is_empty());
}

#[test]
fn builtin_rejection_unavailable_and_pending_cancellation_never_mark_played() {
    let mut f = Fixture::new(32);
    while f.app.engine.send(Command::Tap(Instant::now())).is_ok() {}
    f.app.load_source(
        0,
        Some(&Selection {
            source: LibSource::Builtin(BuiltinStem::Drums),
            title: "rejected".into(), fingerprint: None, }),
    );
    assert!(matches!(
        f.app.loads[0].as_ref().unwrap().phase,
        Phase::Failed(_)
    ));
    render(&mut f, 64);
    assert!(f.app.last_played.by_source.is_empty());
    f.rt.builtin[0] = None;
    f.app.load_source(
        0,
        Some(&Selection {
            source: LibSource::Builtin(BuiltinStem::Drums),
            title: "unavailable".into(), fingerprint: None, }),
    );
    let unavailable = load_receipt(&f, 0);
    render(&mut f, 64);
    assert_eq!(unavailable.state(), State::Unavailable);
    assert!(unavailable.last_play().is_none());
    f.app.load_source(
        0,
        Some(&Selection {
            source: LibSource::Builtin(BuiltinStem::Harmony),
            title: "cancelled".into(), fingerprint: None, }),
    );
    let cancelled = load_receipt(&f, 0);
    cancelled.cancel_pending();
    render(&mut f, 64);
    assert_eq!(cancelled.state(), State::Superseded);
    assert!(f.app.last_played.by_source.is_empty());
}

#[test]
fn real_file_load_stays_unplayed_until_render_and_records_captured_identity_when_filtered_out() {
    let files = Files::new();
    let first = files.wave("first.wav", 4800);
    let other = files.wave("other.wav", 4800);
    let mut f = Fixture::new(32);
    f.app.library = Arc::new(vec![item(&other, "Other"), item(&first, "Original")]);
    f.app.loader = Some(Loader::start().unwrap());
    file_load(&mut f, &first, 0);
    let receipt = load_receipt(&f, 0);
    let selected = LibSource::File(other);
    f.app.lib_filter = "Other".into();
    f.app.lib_sel = 0;
    f.app.refresh_library_view();
    render(&mut f, 32);
    assert!(
        receipt.last_play().is_none(),
        "loaded paused media must remain unplayed"
    );
    f.rt.apply(Command::DeckPlay { deck: 0 });
    render(&mut f, 0);
    assert!(
        receipt.last_play().is_none(),
        "command application is not rendering"
    );
    let earliest = SystemTime::now();
    let audio = render(&mut f, 256);
    let played = receipt.last_play().unwrap();
    assert!(played >= earliest - Duration::from_millis(10) && played <= SystemTime::now());
    assert!(audio.iter().any(|x| x.abs() > 0.001));
    assert_eq!(history(&f, &LibSource::File(first)), Some(played));
    assert_eq!(history(&f, &selected), None);
    assert_eq!(f.app.selected_library_item().unwrap().source, selected);
    f.app.lib_filter.clear();
    f.app.refresh_library_view();
    assert_eq!(
        f.app.last_play_idx, 1,
        "restore last played source after it was hidden"
    );
    assert_eq!(f.app.lib_sel, 0, "history must not browse for the user");
    let ctx = egui::Context::default();
    let output = crate_frame(&ctx, &mut f.app, 0.0, vec![]);
    assert!(visible(&output, &play_time::format(Some(played), SystemTime::now()).label));
    assert!(visible(&output, "—"));
}

#[test]
fn initial_builtins_and_new_play_episodes_record_once_without_callback_heap_work() {
    let mut f = Fixture::new(32);
    let receipt = f.app.engine.initial_playback[0].clone().unwrap();
    let mut output = [0.0; 512];
    f.rt.process(&mut output); // warm render/mixer state
    assert!(receipt.last_play().is_none());
    f.rt.apply(Command::DeckPlay { deck: 0 });
    let counts = test_alloc::measure(|| f.rt.process(&mut output));
    assert_eq!(counts, test_alloc::Counts::default());
    let first = receipt.last_play().unwrap();
    f.app.poll_play_history();
    assert_eq!(
        history(&f, &LibSource::Builtin(BuiltinStem::Drums)),
        Some(first)
    );
    let counts = test_alloc::measure(|| {
        for _ in 0..100 {
            f.rt.process(&mut output);
        }
    });
    assert_eq!(counts, test_alloc::Counts::default());
    assert_eq!(
        receipt.last_play(),
        Some(first),
        "continuous playback is one episode"
    );
    f.rt.apply(Command::DeckCue { deck: 0 });
    f.rt.process(&mut output); // paused tail must not credit a new event
    assert_eq!(receipt.last_play(), Some(first));
    f.rt.apply(Command::DeckPlay { deck: 0 });
    let counts = test_alloc::measure(|| f.rt.process(&mut output));
    assert_eq!(counts, test_alloc::Counts::default());
    let resumed = receipt.last_play().unwrap();
    assert!(resumed > first);
    f.app.poll_play_history();
    assert_eq!(
        history(&f, &LibSource::Builtin(BuiltinStem::Drums)),
        Some(resumed)
    );
}

#[test]
fn loops_muted_mix_and_silent_content_follow_source_play_contract_but_paused_scratch_does_not() {
    let files = Files::new();
    let path = files.wave("silent.wav", 4800);
    let mut f = Fixture::new(32);
    f.app.library = Arc::new(vec![item(&path, "silent")]);
    f.app.load_file(0, path.clone(), "silent");
    f.decoder_jobs.recv_timeout(Duration::from_secs(3)).unwrap();
    let mut report = fake_report(&path);
    report.sample.data.fill(0.0);
    f.decoder_results.send((0, Ok(report))).unwrap();
    f.poll_loads();
    render(&mut f, 64);
    let receipt = load_receipt(&f, 0);
    f.rt.apply(Command::DeckTouch { deck: 0, on: true });
    f.rt.apply(Command::DeckJog {
        deck: 0,
        delta: 0.1,
    });
    render(&mut f, 64);
    assert!(
        receipt.last_play().is_none(),
        "paused hand audition is not a play episode"
    );
    f.rt.apply(Command::DeckTouch { deck: 0, on: false });
    f.rt.apply(Command::Xfader(1.0));
    f.rt.apply(Command::DeckGain {
        deck: 0,
        value: 0.0,
    });
    f.rt.decks[0].loop_on = true;
    f.rt.decks[0].loop_start = 10.0;
    f.rt.decks[0].loop_len = 64.0;
    f.rt.apply(Command::DeckPlay { deck: 0 });
    let audio = render(&mut f, 256);
    assert!(audio.iter().all(|x| *x == 0.0));
    let first = receipt.last_play().unwrap();
    for _ in 0..20 {
        render(&mut f, 256);
    }
    assert_eq!(
        receipt.last_play(),
        Some(first),
        "loop wraps do not restart history"
    );
    assert_eq!(history(&f, &LibSource::File(path)), Some(first));
}

#[test]
fn dismissal_replacement_and_unload_do_not_lose_already_rendered_old_media() {
    let files = Files::new();
    let path = files.wave("old.wav", 4800);
    let missing = files.0.join("missing.wav");
    let mut f = Fixture::new(32);
    f.app.library = Arc::new(vec![item(&path, "old")]);
    f.app.loader = Some(Loader::start().unwrap());
    file_load(&mut f, &path, 0);
    let old = load_receipt(&f, 0);
    // Dismiss the real egui label; history observation outlives presentation.
    let ctx = egui::Context::default();
    let output = status_frame(&ctx, &mut f.app, vec![]);
    let point = crate::ui::test_support::label_center(&output, "Dismiss");
    for pressed in [true, false] {
        status_frame(
            &ctx,
            &mut f.app,
            vec![
                egui::Event::PointerMoved(point),
                egui::Event::PointerButton {
                    pos: point,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                },
            ],
        );
    }
    assert!(f.app.loads[0].is_none());
    f.rt.apply(Command::DeckPlay { deck: 0 });
    f.rt.process(&mut [0.0; 256]);
    assert!(old.last_play().is_some());
    f.app.load_file(0, missing, "later failure");
    f.poll_loads();
    assert_eq!(
        old.state(),
        State::Current,
        "failed replacement keeps original media"
    );
    // Resume again and retire before any UI poll. The last write must still
    // be observed even when the receipt's first observed state is terminal.
    f.rt.apply(Command::DeckPlay { deck: 0 });
    f.rt.process(&mut [0.0; 2]);
    f.rt.apply(Command::DeckPlay { deck: 0 });
    f.rt.process(&mut [0.0; 2]);
    let final_play = old.last_play().unwrap();
    f.rt.apply(Command::DeckUnload { deck: 0 });
    assert_eq!(old.state(), State::Superseded);
    f.app.poll_play_history();
    assert_eq!(history(&f, &LibSource::File(path)), Some(final_play));
    assert!(old.retained_by_history());
    assert!(f.app.playback_watches.iter().any(|watch|watch.identity.source==LibSource::File(files.0.join("old.wav"))));
    f.rt.clear_undo_for_test();
    let until=Instant::now()+Duration::from_secs(2);
    while old.retained_by_history() {assert!(Instant::now()<until);std::thread::sleep(Duration::from_millis(1));}
    f.app.poll_play_history();
    assert!(!f.app.playback_watches.iter().any(|watch|watch.identity.source==LibSource::File(files.0.join("old.wav"))));
    assert_eq!(history(&f,&LibSource::File(files.0.join("old.wav"))),Some(final_play));
    // Loading a second source after an unpolled play does not credit the second.
    builtin_load(&mut f, BuiltinStem::Harmony, 0);
    assert!(load_receipt(&f, 0).last_play().is_none());
}

#[test]
fn file_replacement_same_path_and_moved_content_do_not_inherit_earlier_version_history() {
    let files = Files::new();
    let path = files.wave("replace.wav", 4800);
    let mut f = Fixture::new(32);
    let old_item = item(&path, "original version");
    f.app.library = Arc::new(vec![old_item.clone()]);
    f.app.loader = Some(Loader::start().unwrap());
    file_load(&mut f, &path, 0);
    f.rt.apply(Command::DeckPlay { deck: 0 });
    render(&mut f, 64);
    let old_play = history(&f, &LibSource::File(path.clone())).unwrap();
    files.wave("replace.wav", 9600); // guarantee different length/fingerprint
    let replacement = item(&path, "replacement version");
    assert_ne!(old_item.fingerprint, replacement.fingerprint);
    f.app.library = Arc::new(vec![replacement]);
    assert!(history(&f, &LibSource::File(path.clone())).is_none());
    file_load(&mut f, &path, 1);
    assert!(history(&f, &LibSource::File(path.clone())).is_none());
    f.rt.apply(Command::DeckPlay { deck: 1 });
    render(&mut f, 64);
    let replacement_play = history(&f, &LibSource::File(path.clone())).unwrap();
    assert!(replacement_play > old_play);
    assert_eq!(f.app.item_last_play(&old_item), Some(old_play));
    assert_eq!(
        f.app.last_played.by_source[&LibSource::File(path.clone())].len(),
        2
    );
    let moved = files.0.join("moved.wav");
    std::fs::rename(path, &moved).unwrap();
    assert!(f.app.item_last_play(&item(&moved, "moved")).is_none());
    // A missing fingerprint cannot attach a known old file history by path.
    let mut unknown = old_item;
    unknown.fingerprint = None;
    assert!(f.app.item_last_play(&unknown).is_none());
}

#[test]
fn chronological_timestamp_wins_over_watch_order_and_deck_unload_order() {
    let mut f = Fixture::new(32);
    let drums = LibSource::Builtin(BuiltinStem::Drums);
    let harmony = LibSource::Builtin(BuiltinStem::Harmony);
    // Harmony watch is second in the vector but plays earlier than Drums.
    f.rt.apply(Command::DeckPlay { deck: 1 });
    f.rt.process(&mut [0.0; 128]);
    let first = f.app.engine.initial_playback[1].as_ref().unwrap().last_play().unwrap();
    f.rt.apply(Command::DeckPlay { deck: 0 });
    f.rt.process(&mut [0.0; 128]);
    let last = f.app.engine.initial_playback[0].as_ref().unwrap().last_play().unwrap();
    assert!(last > first);
    f.rt.apply(Command::DeckUnload { deck: 0 });
    f.rt.apply(Command::DeckUnload { deck: 1 });
    f.app.poll_play_history();
    assert_eq!(history(&f, &drums), Some(last));
    assert_eq!(history(&f, &harmony), Some(first));
    assert_eq!(f.app.last_played.latest_identity().unwrap().source, drums);
    assert!(f.app.playback_watches.iter().all(|watch|watch.receipt.retained_by_history()));
    f.rt.clear_undo_for_test();
    let until=Instant::now()+Duration::from_secs(2);
    while f.app.playback_watches.iter().any(|watch|watch.receipt.retained_by_history()) {assert!(Instant::now()<until);std::thread::sleep(Duration::from_millis(1));}
    f.app.poll_play_history();assert!(f.app.playback_watches.is_empty());
    assert_eq!(history(&f,&drums),Some(last));assert_eq!(history(&f,&harmony),Some(first));
}

#[test]
fn end_of_file_missing_empty_invalid_sources_and_transition_tails_never_create_a_false_play() {
    let mut f = Fixture::new(32);
    let receipt = f.app.engine.initial_playback[0].clone().unwrap();
    f.rt.apply(Command::DeckSeek { deck: 0, frac: 1.0 });
    f.rt.apply(Command::DeckPlay { deck: 0 });
    render(&mut f, 64);
    assert!(
        receipt.last_play().is_none(),
        "EOF stopped before a source frame rendered"
    );
    for (frames, sr) in [(0, 48000), (1, 48000), (100, 0)] {
        let invalid = Sample {
            name: "invalid".into(),
            sr,
            ch: 1,
            data: vec![0.2; frames],
            peaks: vec![].into(),
            bpm: 120.0,
            path: String::new(),
        };
        f.rt.builtin[1] = Some(Arc::new(invalid));
        builtin_load(&mut f, BuiltinStem::Harmony, 0);
        let receipt = load_receipt(&f, 0);
        f.rt.apply(Command::DeckPlay { deck: 0 });
        render(&mut f, 128);
        assert!(
            receipt.last_play().is_none(),
            "invalid sr={sr} frames={frames}"
        );
    }
    f.rt.apply(Command::DeckUnload { deck: 0 });
    f.rt.apply(Command::DeckPlay { deck: 0 });
    render(&mut f, 128);
    assert!(f.app.last_played.by_source.is_empty());
}

fn status_frame(ctx: &egui::Context, app: &mut App, events: Vec<egui::Event>) -> egui::FullOutput {
    ctx.run(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1440.0, 900.0))),
            events,
            ..Default::default()
        },
        |ctx| app.load_status(ctx),
    )
}

#[test]
fn queued_decoded_load_cancelled_before_application_cannot_create_history() {
    let files = Files::new();
    let path = files.wave("queued.wav", 4800);
    let mut f = Fixture::new(32);
    f.app.library = Arc::new(vec![item(&path, "queued")]);
    f.app.loader = Some(Loader::start().unwrap());
    f.app.load_file(0, path.clone(), "queued");
    f.poll_loads();
    let pending = load_receipt(&f, 0);
    assert_eq!(pending.state(), State::Pending);
    assert!(f.app.submit(Command::DeckUnload { deck: 0 }));
    f.app.send(Command::DeckPlay { deck: 0 });
    render(&mut f, 128);
    assert_eq!(pending.state(), State::Superseded);
    assert!(pending.last_play().is_none());
    assert!(f.rt.decks[0].audio.is_none());
    assert!(history(&f, &LibSource::File(path)).is_none());
}

#[test]
fn renderer_disconnect_preserves_final_play_and_retires_watches() {
    let Fixture {
        mut app, mut rt, ..
    } = Fixture::new(32);
    rt.apply(Command::DeckPlay { deck: 1 });
    rt.process(&mut [0.0; 128]);
    let played = app.engine.initial_playback[1].as_ref().unwrap().last_play().unwrap();
    drop(rt);
    assert!(!app.engine.cmd.is_connected());
    app.poll_play_history();
    let harmony = app
        .library
        .iter()
        .find(|item| item.source == LibSource::Builtin(BuiltinStem::Harmony))
        .unwrap();
    assert_eq!(app.item_last_play(harmony), Some(played));
    assert!(app.playback_watches.is_empty());
    let counts = test_alloc::measure(|| {
        for _ in 0..100 {
            app.poll_play_history();
        }
    });
    assert_eq!(counts, test_alloc::Counts::default());
}
