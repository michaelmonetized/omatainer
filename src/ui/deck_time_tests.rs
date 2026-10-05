use super::*;
use crate::engine::{dsp::Sample, DeckSnap, RtEngine};
use test_support::label_center;

fn snap(seconds: f64, position: f64, rate: f32) -> DeckSnap {
    DeckSnap {
        frames: seconds * 48_000.0,
        pos: position * 48_000.0,
        source_sample_rate: 48_000,
        playback_rate: rate,
        playing: true,
        ..Default::default()
    }
}
fn settings(mode: TimeMode, lead: u16) -> DeckTimeSettings {
    DeckTimeSettings {
        mode,
        warning_lead_seconds: lead,
    }
}
fn read(snap: &DeckSnap, mode: TimeMode, lead: u16) -> Readout {
    Readout::from_snapshot(snap, settings(mode, lead))
}

#[test]
fn fixed_playhead_times_and_warning_boundaries_are_rate_aware_and_unambiguous() {
    for (duration, pos, rate, elapsed, remaining, warn) in [
        (3.0, 0.0, 1.0, "0:00.0", "−0:03.0", true),
        (600.0, 569.0, 1.0, "9:29.0", "−0:31.0", false),
        (600.0, 570.0, 1.0, "9:30.0", "−0:30.0", true),
        (600.0, 540.0, 2.0, "9:00.0", "−0:30.0", true),
        (600.0, 584.5, 0.5, "9:44.5", "−0:31.0", false),
        (7261.25, 3661.25, 1.0, "1:01:01.2", "−1:00:00.0", false),
        (60.0, 0.01, 1.0, "0:00.0", "−0:59.9", false),
    ] {
        let s = snap(duration, pos, rate);
        let e = read(&s, TimeMode::Elapsed, 30);
        let r = read(&s, TimeMode::Remaining, 30);
        assert_eq!(e.text, elapsed);
        assert_eq!(r.text, remaining);
        assert_eq!(e.warning, warn, "warning is independent of display mode");
        assert_eq!(r.warning, warn);
        assert!(e.tooltip.contains("source playhead time"));
        assert!(r.tooltip.contains("estimated wall time"));
        assert!(!read(&s, TimeMode::Remaining, 0).warning);
    }
    let mut s = snap(3.0, 2.0, 1.0);
    s.playing = false;
    let r = read(&s, TimeMode::Remaining, 30);
    assert_eq!(r.text, "−0:01.0");
    assert_eq!(r.status, "PAUSED");
    assert!(!r.warning);
    s.playing = true;
    s.touching = true;
    assert_eq!(read(&s, TimeMode::Remaining, 30).status, "SCRATCH");
    assert!(!read(&s, TimeMode::Remaining, 30).warning);
    for rate in [0.0, -1.0, f32::NAN, f32::INFINITY] {
        s.playback_rate = rate;
        let r = read(&s, TimeMode::Remaining, 30);
        assert_eq!(r.text, "—:——");
        assert!(!r.warning);
        assert_eq!(read(&s, TimeMode::Elapsed, 30).text, "0:02.0");
    }
}

#[test]
fn only_a_valid_containing_repeat_loop_suppresses_runout() {
    let mut s = snap(60.0, 59.0, 1.0);
    s.loop_on = true;
    s.loop_start = 55.0 * 48_000.0;
    s.loop_len = 5.0 * 48_000.0;
    let r = read(&s, TimeMode::Remaining, 30);
    assert_eq!(r.status, "LOOP");
    assert!(!r.warning);
    assert!(r.tooltip.contains("ignores future loop repeats"));
    for (start, len) in [
        (s.loop_start, 0.0),
        (s.loop_start, 1.0),
        (s.loop_start, -1.0),
        (-1.0, s.loop_len),
        (f64::NAN, s.loop_len),
        (s.loop_start, f64::INFINITY),
        (s.loop_start, s.loop_len + 1.0),
        (0.0, 10.0),
        (s.pos + 1.0, 10.0),
    ] {
        let mut invalid = s.clone();
        invalid.loop_start = start;
        invalid.loop_len = len;
        assert!(
            read(&invalid, TimeMode::Remaining, 30).warning,
            "invalid/outside loop {start}, {len}"
        );
    }
    s.loop_on = false;
    assert!(read(&s, TimeMode::Elapsed, 30).warning);
    for mut invalid in [
        DeckSnap::default(),
        snap(0.0, 0.0, 1.0),
        snap(f64::NAN, 0.0, 1.0),
        snap(60.0, -1.0, 1.0),
        snap(60.0, 61.0, 1.0),
    ] {
        invalid.playing = true;
        assert!(!read(&invalid, TimeMode::Remaining, 30).warning);
        assert_eq!(read(&invalid, TimeMode::Elapsed, 30).text, "—:——");
    }
}

fn source(sr: u32, seconds: usize) -> Arc<Sample> {
    Arc::new(Sample {
        name: "Runout reference".into(),
        sr,
        ch: 1,
        data: vec![0.1; sr as usize * seconds],
        peaks: Arc::new(vec![]),
        bpm: 120.0,
        path: String::new(),
    })
}
fn capture(app: &mut App, rt: &mut RtEngine) {
    rt.publish_for_test();
    app.snap = app.engine.snapshot();
}

#[test]
fn real_renderer_snapshot_tracks_media_output_rates_sync_smoothing_pause_and_loop_wrap() {
    for (media_sr, output_sr) in [(44_100, 48_000), (48_000, 96_000), (96_000, 44_100)] {
        let (engine, mut rt) = Engine::headless_for_test(output_sr, 80);
        let mut app = App::with_loader(engine, Theme::default(), None);
        for deck in 0..DECKS {
            rt.apply(Command::DeckAudio {
                deck: deck as u8,
                audio: source(media_sr, 12),
            });
            rt.decks[deck].pos = media_sr as f64 * 8.0;
            rt.decks[deck].playing = true;
            rt.decks[deck].sync = true;
            rt.decks[deck].sync_bpm = if deck == 0 { 240.0 } else { 60.0 };
            rt.decks[deck].pitch = if deck == 0 { 0.0 } else { 1.0 }; // intentionally disagrees with sync
        }
        let mut elapsed = [8.0; DECKS];
        for block in [1, 17, 127, 512] {
            let mut out = vec![0.0; block * 2];
            let before: [f64; DECKS] = std::array::from_fn(|i| rt.decks[i].pos);
            rt.process(&mut out);
            capture(&mut app, &mut rt);
            for deck in 0..DECKS {
                let d = &app.snap.decks[deck];
                let r = read(d, TimeMode::Remaining, 3);
                assert_eq!(d.source_sample_rate, media_sr);
                assert_eq!(d.playback_rate, rt.decks[deck].rate);
                assert_eq!(d.pos, rt.decks[deck].pos);
                assert!((r.elapsed.unwrap() - d.pos / media_sr as f64).abs() < 1e-10);
                assert!(
                    (r.remaining.unwrap()
                        - (12.0 - d.pos / media_sr as f64) / d.playback_rate as f64)
                        .abs()
                        < 1e-10
                );
                assert!(r.elapsed.unwrap() > elapsed[deck]);
                elapsed[deck] = r.elapsed.unwrap();
                let min_rate = if deck == 0 { 1.0 } else { 0.5 };
                let max_rate = if deck == 0 { 2.0 } else { 1.0 };
                let advance = (d.pos - before[deck]) / media_sr as f64;
                assert!(advance >= block as f64 / output_sr as f64 * min_rate - 1e-9);
                assert!(advance <= block as f64 / output_sr as f64 * max_rate + 1e-9);
                assert_eq!(r.warning, r.remaining.unwrap() <= 3.0);
            }
        }
        // Stop and render, rather than inventing a UI-only paused flag.
        rt.apply(Command::DeckPlay { deck: 0 });
        let before = rt.decks[0].pos;
        rt.process(&mut [0.0; 256]);
        capture(&mut app, &mut rt);
        assert_eq!(app.snap.decks[0].pos, before);
        assert_eq!(
            read(&app.snap.decks[0], TimeMode::Remaining, 30).status,
            "PAUSED"
        );
        assert!(!read(&app.snap.decks[0], TimeMode::Remaining, 30).warning);

        let d = &mut rt.decks[1];
        d.sync_bpm = 120.0;
        d.rate = 1.0;
        d.pos = media_sr as f64 * 12.0 - 1.0;
        d.loop_on = true;
        d.loop_start = media_sr as f64 * 11.0;
        d.loop_len = media_sr as f64;
        rt.process(&mut [0.0; 256]);
        capture(&mut app, &mut rt);
        let d = &app.snap.decks[1];
        assert!(d.pos < media_sr as f64 * 11.1, "actual renderer wrapped");
        assert_eq!(d.loop_start, rt.decks[1].loop_start);
        assert_eq!(d.loop_len, rt.decks[1].loop_len);
        assert_eq!(read(d, TimeMode::Remaining, 30).status, "LOOP");
        assert!(!read(d, TimeMode::Remaining, 30).warning);
        rt.apply(Command::DeckTouch { deck: 1, on: true });
        rt.apply(Command::DeckJog {
            deck: 1,
            delta: -0.1,
        });
        rt.process(&mut [0.0; 2]);
        capture(&mut app, &mut rt);
        assert!(app.snap.decks[1].touching);
        assert!(app.snap.decks[1].playback_rate < 0.0);
        assert!(!read(&app.snap.decks[1], TimeMode::Remaining, 30).warning);
        // Media replacement resets the old loop, and unload removes time data.
        rt.apply(Command::DeckAudio {
            deck: 1,
            audio: source(media_sr, 1),
        });
        capture(&mut app, &mut rt);
        assert!(!app.snap.decks[1].loop_on);
        rt.apply(Command::DeckUnload { deck: 1 });
        capture(&mut app, &mut rt);
        assert_eq!(app.snap.decks[1].source_sample_rate, 0);
        assert_eq!(
            read(&app.snap.decks[1], TimeMode::Remaining, 30).status,
            "NO AUDIO"
        );
    }
}

struct Gui {
    app: App,
    rt: RtEngine,
    ctx: egui::Context,
    time: f64,
}
impl Gui {
    fn new() -> Self {
        let (engine, rt) = Engine::headless_for_test(48_000, 80);
        Self {
            app: App::with_loader(engine, Theme::default(), None),
            rt,
            ctx: egui::Context::default(),
            time: 0.0,
        }
    }
    fn frame(&mut self, events: Vec<egui::Event>) -> egui::FullOutput {
        self.time += 0.05;
        let theme = self.app.theme.clone();
        self.ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1440.0, 1000.0))),
                time: Some(self.time),
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.allocate_ui(Vec2::new(1424.0, 240.0), |ui| {
                        self.app.scratch_row(ui, &theme)
                    });
                });
            },
        )
    }
    fn click(&mut self, pos: Pos2) -> egui::FullOutput {
        self.frame(vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed: true,
                modifiers: Default::default(),
            },
        ]);
        self.frame(vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed: false,
                modifiers: Default::default(),
            },
        ])
    }
}
fn labels<'a>(output: &'a egui::FullOutput, text: &str) -> Vec<&'a egui::epaint::TextShape> {
    output
        .shapes
        .iter()
        .filter_map(|shape| match &shape.shape {
            egui::epaint::Shape::Text(t) if t.galley.text() == text => Some(t),
            _ => None,
        })
        .collect()
}

#[test]
fn actual_egui_per_deck_menus_change_modes_and_lead_without_transport_commands() {
    let mut gui = Gui::new();
    gui.app.snap.decks[0] = snap(120.0, 50.0, 1.0);
    gui.app.snap.decks[1] = snap(120.0, 119.0, 1.0);
    gui.frame(vec![]);
    let first = gui.frame(vec![]);
    let buttons = labels(&first, "Remain est. ▾");
    assert_eq!(buttons.len(), 2);
    let a = buttons[0].visual_bounding_rect().center();
    let b = buttons[1].visual_bounding_rect().center();
    for button in buttons {
        let rect = button.visual_bounding_rect();
        assert!(
            rect.left() >= 0.0 && rect.right() <= 1440.0 && rect.bottom() < 252.0,
            "time control fits existing footer: {rect:?}"
        );
    }
    gui.click(a);
    let popup = gui.frame(vec![]);
    let elapsed = label_center(&popup, "Elapsed · source time");
    gui.click(elapsed);
    assert_eq!(gui.app.deck_time[0].mode, TimeMode::Elapsed);
    assert_eq!(gui.app.deck_time[1].mode, TimeMode::Remaining);
    let popup = gui.frame(vec![]);
    let lead = label_center(&popup, "30 s");
    // Double-click the real DragValue to enter keyboard editing.
    gui.click(lead);
    gui.click(lead);
    gui.frame(vec![
        egui::Event::Key {
            key: Key::A,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::CTRL,
        },
        egui::Event::Text("5".into()),
        egui::Event::Key {
            key: Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Default::default(),
        },
    ]);
    assert_eq!(gui.app.deck_time[0].warning_lead_seconds, 5);
    assert_eq!(gui.app.deck_time[1].warning_lead_seconds, 30);
    gui.click(Pos2::new(700.0, 500.0));
    let shown = gui.frame(vec![]);
    assert_eq!(labels(&shown, "Elapsed ▾").len(), 1);
    assert_eq!(labels(&shown, "0:50.0").len(), 1);
    assert_eq!(labels(&shown, "−0:01.0").len(), 1);
    gui.click(b);
    let popup = gui.frame(vec![]);
    gui.click(label_center(&popup, "Elapsed · source time"));
    assert_eq!(gui.app.deck_time[1].mode, TimeMode::Elapsed);
    while let Ok(command) = gui.rt.cmd_rx.try_recv() {
        assert!(
            matches!(command, Command::SelectDeckRequested { .. }),
            "time preferences may select their deck but never play/cue/seek: {command:?}"
        );
    }
}

#[test]
fn actual_egui_runout_text_and_red_ring_follow_captured_state_and_configured_lead() {
    let mut gui = Gui::new();
    for deck in 0..DECKS {
        gui.rt.apply(Command::DeckAudio {
            deck: deck as u8,
            audio: source(44_100, 3),
        });
        gui.rt.decks[deck].playing = true;
    }
    gui.rt.process(&mut [0.0; 2]);
    capture(&mut gui.app, &mut gui.rt);
    gui.app.deck_time[0] = settings(TimeMode::Elapsed, 1);
    gui.app.deck_time[1] = settings(TimeMode::Remaining, 3);
    let output = gui.frame(vec![]);
    let warnings = labels(&output, "RUNOUT");
    assert_eq!(warnings.len(), 1);
    assert_eq!(
        warnings[0].galley.rows[0].visuals.mesh.vertices[0].color,
        gui.app.theme.red
    );
    assert_eq!(output.shapes.iter().filter(|shape| matches!(&shape.shape,
        egui::epaint::Shape::Circle(circle) if circle.stroke.color == gui.app.theme.red && circle.stroke.width == 3.0)).count(), 1);
    gui.app.deck_time[1].warning_lead_seconds = 0;
    assert!(labels(&gui.frame(vec![]), "RUNOUT").is_empty());
    gui.app.deck_time[1].warning_lead_seconds = 30;
    gui.rt.decks[1].loop_on = true;
    gui.rt.decks[1].loop_start = 0.0;
    gui.rt.decks[1].loop_len = 44_100.0 * 3.0;
    capture(&mut gui.app, &mut gui.rt);
    let output = gui.frame(vec![]);
    assert!(labels(&output, "RUNOUT").is_empty());
    assert_eq!(labels(&output, "LOOP").len(), 1);
    gui.rt.apply(Command::DeckPlay { deck: 1 });
    capture(&mut gui.app, &mut gui.rt);
    assert_eq!(labels(&gui.frame(vec![]), "PAUSED").len(), 1);
}

#[test]
fn deck_time_settings_serialize_independently_with_bounded_lead_and_defaults() {
    let settings = [
        settings(TimeMode::Elapsed, 0),
        settings(TimeMode::Remaining, 120),
    ];
    let encoded = serde_json::to_string(&settings).unwrap();
    assert_eq!(
        serde_json::from_str::<[DeckTimeSettings; DECKS]>(&encoded).unwrap(),
        settings
    );
    assert_eq!(
        serde_json::from_str::<DeckTimeSettings>("{}").unwrap(),
        DeckTimeSettings::default()
    );
    let excessive =
        serde_json::from_str::<DeckTimeSettings>(r#"{"warning_lead_seconds":65535}"#).unwrap();
    assert_eq!(excessive.warning_lead(), 300);
    assert!(serde_json::from_str::<DeckTimeSettings>(r#"{"warning_lead_seconds":-1}"#).is_err());
}

#[test]
fn long_media_pitch_ranges_and_end_of_file_use_the_real_captured_playhead() {
    let (engine, mut rt) = Engine::headless_for_test(96_000, 80);
    let mut app = App::with_loader(engine, Theme::default(), None);
    rt.apply(Command::DeckAudio {
        deck: 0,
        audio: source(44_100, 125),
    });
    for (range, pitch, target) in [(0, 1.0, 1.08), (1, 0.0, 0.84), (2, 0.0, 0.5)] {
        let d = &mut rt.decks[0];
        d.pos = 122.0 * 44_100.0;
        d.rate = target;
        d.pitch_range = range;
        d.playing = true;
        rt.apply(Command::DeckPitch {
            deck: 0,
            value: pitch,
        });
        rt.process(&mut [0.0; 1024]);
        capture(&mut app, &mut rt);
        let s = &app.snap.decks[0];
        let actual = read(s, TimeMode::Remaining, 3);
        let elapsed = 122.0 + 512.0 / 96_000.0 * f64::from(s.playback_rate);
        assert!((actual.elapsed.unwrap() - elapsed).abs() < 1e-7);
        assert!(
            (actual.remaining.unwrap() - (125.0 - elapsed) / f64::from(s.playback_rate)).abs()
                < 1e-7
        );
        assert_eq!(actual.warning, range == 0);
        assert_eq!(read(s, TimeMode::Elapsed, 3).text, "2:02.0");
    }
    rt.decks[0].pos = 125.0 * 44_100.0 - 0.01;
    rt.process(&mut [0.0; 2]);
    capture(&mut app, &mut rt);
    let s = &app.snap.decks[0];
    assert!(!s.playing);
    assert_eq!(s.pos, 0.0);
    assert!(!read(s, TimeMode::Remaining, 30).warning);
}

#[test]
fn actual_full_app_time_controls_and_settings_popup_fit_at_1440_by_900() {
    let mut gui = Gui::new();
    let frame = |gui: &mut Gui, events: Vec<egui::Event>| {
        gui.time += 0.05;
        gui.ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1440.0, 900.0))),
                time: Some(gui.time),
                events,
                ..Default::default()
            },
            |ctx| gui.app.update_frame(ctx),
        )
    };
    frame(&mut gui, vec![]);
    let output = frame(&mut gui, vec![]);
    let buttons = labels(&output, "Remain est. ▾");
    assert_eq!(buttons.len(), 2);
    let screen = Rect::from_min_size(Pos2::ZERO, Vec2::new(1440.0, 900.0));
    for button in buttons {
        assert!(screen.contains_rect(button.visual_bounding_rect()));
        let pos = button.visual_bounding_rect().center();
        for pressed in [true, false] {
            frame(
                &mut gui,
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
        let popup = frame(&mut gui, vec![]);
        for label in [
            "Elapsed · source time",
            "Remaining · wall estimate",
            "Warn before file end (seconds)",
            "30 s",
        ] {
            let label = labels(&popup, label);
            assert_eq!(label.len(), 1);
            assert!(screen.contains_rect(label[0].visual_bounding_rect()));
        }
        frame(
            &mut gui,
            vec![egui::Event::Key {
                key: Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Default::default(),
            }],
        );
        frame(&mut gui, vec![]);
    }
    while let Ok(command) = gui.rt.cmd_rx.try_recv() {
        assert!(
            matches!(command, Command::SelectDeckRequested { .. }),
            "deck pointer selection is allowed; popup Escape must not leak: {command:?}"
        );
    }
}
