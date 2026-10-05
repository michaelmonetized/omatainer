use super::test_support::{label_center, Fixture};
use super::*;
use crate::engine::{sampler_identity_tests as sound, SynthInstrument};

use crate::engine::dsp::InputKey;

type Gates = Vec<(u8, bool)>;

struct Pads {
    f: Fixture,
    ctx: egui::Context,
    time: f64,
    points: [Pos2; 16],
    cover: Option<Pos2>,
    stops: Vec<u8>,
}
impl Pads {
    fn new() -> Self {
        let mut f = Fixture::new(256);
        sound::prepare(&mut f.rt);
        let ctx = egui::Context::default();
        ctx.style_mut(|style| {
            style.interaction.tooltip_delay = 0.0;
            style.interaction.show_tooltips_only_when_still = false;
        });
        let mut pads = Self {
            f,
            ctx,
            time: 0.0,
            points: [Pos2::ZERO; 16],
            cover: None,
            stops: Vec::new(),
        };
        pads.frame(vec![]);
        let (output, gates) = pads.frame(vec![]);
        assert!(gates.is_empty());
        pads.points = std::array::from_fn(|pad| label_center(&output, &(pad + 1).to_string()));
        // Preserve physical piano rows; labels now name these actual slots.
        assert!(pads.points[8].y < pads.points[0].y);
        pads
    }
    fn frame(&mut self, events: Vec<egui::Event>) -> (egui::FullOutput, Gates) {
        self.time += 0.05;
        self.f.rt.publish_for_test();
        self.f.app.snap = self.f.app.engine.snapshot();
        let theme = self.f.app.theme.clone();
        let output = self.ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1200.0, 320.0))),
                time: Some(self.time),
                events,
                ..Default::default()
            },
            |ctx| {
                self.f.app.touch_input.begin(ctx);
                if let Some(center) = self.cover {
                    egui::Area::new(egui::Id::new("pad-test-occlusion"))
                        .order(egui::Order::Foreground)
                        .fixed_pos(center - egui::vec2(25.0, 25.0))
                        .show(ctx, |ui| {
                            ui.allocate_exact_size(egui::vec2(50.0, 50.0), Sense::click());
                        });
                }
                egui::CentralPanel::default().show(ctx, |ui| self.f.app.sampler_row(ui, &theme));
                self.f.app.finish_touch_input(ctx);
            },
        );
        let mut gates = Vec::new();
        while let Ok(command) = self.f.rt.cmd_rx.try_recv() {
            if let Command::SamplerPad { pad, on } = command {
                gates.push((pad, on));
            } else if let Command::ReservedStop { lane, .. } = command {
                assert!(usize::from(lane) >= (crate::engine::session::MAX_TRACKS + 1));
                self.stops.push((usize::from(lane) - (crate::engine::session::MAX_TRACKS + 1)) as u8);
            } else {
                panic!("unexpected command {command:?}");
            }
            self.f.rt.apply(command);
        }
        (output, gates)
    }
    fn pointer(&mut self, point: Pos2, pressed: bool) -> Gates {
        self.frame(vec![
            egui::Event::PointerMoved(point),
            egui::Event::PointerButton {
                pos: point,
                button: PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            },
        ])
        .1
    }
    fn hover(&mut self, pad: usize) -> egui::FullOutput {
        self.frame(vec![egui::Event::PointerMoved(self.points[pad])]);
        self.frame(vec![]);
        self.frame(vec![]).0
    }
    fn instrument(&mut self, instrument: SamplerInstrument) {
        self.f.rt.apply(Command::SamplerInst(instrument));
        assert!(self.frame(vec![]).1.is_empty());
    }
    fn held(&self, pad: u8) -> Vec<(u8, SynthInstrument)> {
        self.f
            .rt
            .sampler_poly
            .voices
            .iter()
            .filter(|voice| {
                voice.input == Some(InputKey::Pad(pad)) && matches!(voice.env.stage, 1..=3)
            })
            .map(|voice| (voice.note(), voice.kind))
            .collect()
    }
}
fn visible(output: &egui::FullOutput, expected: &str) -> bool {
    output.shapes.iter().any(|shape| {
        matches!(&shape.shape,
        egui::epaint::Shape::Text(label) if label.galley.text() == expected)
    })
}

#[test]
fn every_sample_label_and_tooltip_dispatches_exact_bank_slot_and_matching_release() {
    let mut pads = Pads::new();
    for bank in 0..3 {
        pads.f.rt.apply(Command::SamplerBank(bank));
        for pad in 0..16usize {
            let tooltip = pads.hover(pad);
            assert!(visible(
                &tooltip,
                &format!("Sample pad {} · bank slot {pad}", pad + 1)
            ));
            assert_eq!(pads.pointer(pads.points[pad], true), [(pad as u8, true)]);
            assert!(pads.f.app.pad_held[pad]);
            let voice = pads.f.rt.pad_voices[pad].as_ref().unwrap();
            assert!(
                Arc::ptr_eq(&voice.audio, &pads.f.rt.sampler_banks[bank].data.audio[pad].as_ref().unwrap()),
                "bank {bank} label {} misrouted",
                pad + 1
            );
            assert_eq!(voice.track, 4, "original sampler mixer destination");
            assert!(pads.frame(vec![]).1.is_empty(), "held pointer retriggered");
            assert_eq!(pads.pointer(pads.points[pad], false), [(pad as u8, false)]);
            assert!(!pads.f.app.pad_held.iter().any(|held| *held));
        }
    }
}

#[test]
fn every_instrument_cell_preserves_natural_accidental_note_and_release_identity() {
    // Independent expected octave-three piano contract; three top-row gaps
    // have no accidental and must never emit a gate.
    let notes = [
        Some(57),
        Some(59),
        Some(60),
        Some(62),
        Some(64),
        Some(65),
        Some(67),
        Some(69),
        Some(58),
        None,
        Some(61),
        Some(63),
        None,
        Some(66),
        Some(68),
        None,
    ];
    let labels = [
        "A", "B", "C", "D", "E", "F", "G", "A", "A#", "", "C#", "D#", "", "F#", "G#", "",
    ];
    let mut pads = Pads::new();
    for kind in SynthInstrument::ALL {
        pads.instrument(SamplerInstrument::Synth(kind));
        for pad in 0..16usize {
            let tooltip = pads.hover(pad);
            let expected = match notes[pad] {
                Some(note) => format!("{} · pad {} · MIDI note {note}", labels[pad], pad + 1),
                None => format!("Pad {} · no piano accidental", pad + 1),
            };
            assert!(visible(&tooltip, &expected), "missing {expected}");
            let down = pads.pointer(pads.points[pad], true);
            if let Some(note) = notes[pad] {
                assert_eq!(down, [(pad as u8, true)]);
                assert_eq!(pads.held(pad as u8), [(note, kind)]);
            } else {
                assert!(down.is_empty());
                assert!(pads.held(pad as u8).is_empty());
            }
            let release = if pad % 2 == 0 {
                pads.points[pad]
            } else {
                Pos2::new(1.0, 1.0)
            };
            let up = pads.pointer(release, false);
            assert_eq!(
                up,
                notes[pad]
                    .map(|_| vec![(pad as u8, false)])
                    .unwrap_or_default()
            );
            assert!(
                pads.held(pad as u8).is_empty(),
                "pad {pad} remained sustained"
            );
            assert!(!pads.f.app.pad_held.iter().any(|held| *held));
        }
    }
}

#[test]
fn held_identity_survives_mode_changes_gaps_octave_and_selection_without_new_gates() {
    let mut pads = Pads::new();
    // Every sample slot keeps its captured release, even when mode changes
    // make its visual cell an inert piano gap.
    for pad in 0..16usize {
        pads.instrument(SamplerInstrument::Samples);
        assert_eq!(pads.pointer(pads.points[pad], true), [(pad as u8, true)]);
        let sample = pads.f.rt.pad_voices[pad].as_ref().unwrap().audio.clone();
        pads.instrument(SamplerInstrument::Synth(SynthInstrument::Keys));
        assert!(pads.f.app.pad_held[pad]);
        assert!(Arc::ptr_eq(
            &pads.f.rt.pad_voices[pad].as_ref().unwrap().audio,
            &sample
        ));
        assert!(
            pads.held(pad as u8).is_empty(),
            "switching source type retriggered the pointer"
        );
        assert_eq!(
            pads.pointer(Pos2::new(1.0, 1.0), false),
            [(pad as u8, false)]
        );
        assert!(!pads.f.app.pad_held[pad]);
    }
    // A held synth voice retains its original kind until its captured off,
    // even after the UI becomes samples and the selected mixer target changes.
    for pad in [0usize, 8, 10, 11, 13, 14] {
        pads.instrument(SamplerInstrument::Synth(SynthInstrument::Pad));
        pads.f.rt.sampler_oct = 3;
        assert_eq!(pads.pointer(pads.points[pad], true), [(pad as u8, true)]);
        let original = pads.held(pad as u8)[0].0;
        pads.instrument(SamplerInstrument::Samples);
        pads.f.rt.apply(Command::Select { track: 2, scene: 1 });
        pads.f.rt.apply(Command::SamplerOct(1));
        assert!(pads.frame(vec![]).1.is_empty());
        assert_eq!(
            pads.held(pad as u8),
            [(original + 12, SynthInstrument::Pad)]
        );
        assert_eq!(pads.pointer(pads.points[pad], false), [(pad as u8, false)]);
        assert!(pads.held(pad as u8).is_empty());
    }
    // Pressing an inert piano gap cannot turn into a new sample trigger simply
    // because its label becomes enabled while that original pointer is held.
    for pad in [9usize, 12, 15] {
        pads.instrument(SamplerInstrument::Synth(SynthInstrument::Analog));
        assert!(pads.pointer(pads.points[pad], true).is_empty());
        pads.instrument(SamplerInstrument::Samples);
        assert!(pads.frame(vec![]).1.is_empty());
        assert!(pads.pointer(pads.points[pad], false).is_empty());
    }
}

#[test]
fn rejected_pointer_press_has_no_held_identity_or_unmatched_release() {
    let mut pads = Pads::new();
    while pads.f.app.engine.send(Command::Tap(Instant::now())).is_ok() {}
    // Draw directly so the full command queue stays full during admission.
    let point = pads.points[0];
    pads.time += 0.05;
    let theme = pads.f.app.theme.clone();
    let _ = pads.ctx.run(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1200.0, 320.0))),
            time: Some(pads.time),
            events: vec![
                egui::Event::PointerMoved(point),
                egui::Event::PointerButton {
                    pos: point,
                    button: PointerButton::Primary,
                    pressed: true,
                    modifiers: Default::default(),
                },
            ],
            ..Default::default()
        },
        |ctx| {
            pads.f.app.touch_input.begin(ctx);
            egui::CentralPanel::default().show(ctx, |ui| pads.f.app.sampler_row(ui, &theme));
            pads.f.app.finish_touch_input(ctx);
        },
    );
    assert!(pads.f.app.submission_error.get().is_some());
    assert!(!pads.f.app.pad_held.iter().any(|held| *held));
    while let Ok(command) = pads.f.rt.cmd_rx.try_recv() {
        assert!(matches!(command, Command::Tap(_)));
    }
    assert!(pads.pointer(point, false).is_empty());
}

#[test]
fn releasing_then_pressing_another_pad_in_one_frame_releases_the_original_identity() {
    let mut pads = Pads::new();
    pads.instrument(SamplerInstrument::Synth(SynthInstrument::Keys));
    assert_eq!(pads.pointer(pads.points[0], true), [(0, true)]);
    let (_, gates) = pads.frame(vec![
        egui::Event::PointerButton {
            pos: pads.points[0],
            button: PointerButton::Primary,
            pressed: false,
            modifiers: Default::default(),
        },
        egui::Event::PointerMoved(pads.points[1]),
        egui::Event::PointerButton {
            pos: pads.points[1],
            button: PointerButton::Primary,
            pressed: true,
            modifiers: Default::default(),
        },
    ]);
    assert_eq!(gates, [(0, false), (1, true)]);
    assert!(pads.held(0).is_empty());
    assert_eq!(pads.held(1), [(59, SynthInstrument::Keys)]);
    assert_eq!(pads.pointer(pads.points[1], false), [(1, false)]);
    assert!(pads.held(1).is_empty());
}

#[test]
fn quick_tap_retrigger_and_drag_in_use_the_real_primary_press_position() {
    let mut pads = Pads::new();
    let target = pads.points[3];
    let button = |pos, pressed| egui::Event::PointerButton {
        pos,
        button: PointerButton::Primary,
        pressed,
        modifiers: Default::default(),
    };
    let (_, gates) = pads.frame(vec![
        egui::Event::PointerMoved(target),
        button(target, true),
        button(target, false),
    ]);
    assert_eq!(gates, [(3, true), (3, false)]);
    assert!(!pads.f.app.pad_held[3]);
    let outside = Pos2::new(1.0, 1.0);
    let (_, gates) = pads.frame(vec![
        egui::Event::PointerMoved(target),
        button(target, true),
        egui::Event::PointerMoved(outside),
        button(outside, false),
    ]);
    assert_eq!(
        gates,
        [(3, true), (3, false)],
        "same-frame outside release lost its original press"
    );
    assert_eq!(pads.pointer(target, true), [(3, true)]);
    let (_, gates) = pads.frame(vec![button(target, false), button(target, true)]);
    assert_eq!(gates, [(3, false), (3, true)]);
    assert!(pads.f.app.pad_held[3]);
    assert_eq!(pads.pointer(target, false), [(3, false)]);
    let outside = Pos2::new(1.0, 1.0);
    let (_, gates) = pads.frame(vec![
        egui::Event::PointerMoved(outside),
        button(outside, true),
        egui::Event::PointerMoved(target),
    ]);
    assert!(
        gates.is_empty(),
        "dragging into a pad was treated as a fresh press"
    );
    assert!(pads.pointer(target, false).is_empty());
}

#[test]
fn an_overlapping_interactive_layer_blocks_pad_press_without_blocking_existing_release() {
    let mut pads = Pads::new();
    let target = pads.points[0];
    pads.cover = Some(target);
    pads.frame(vec![]);
    pads.frame(vec![]);
    assert!(
        pads.pointer(target, true).is_empty(),
        "press passed through covering layer"
    );
    assert!(pads.pointer(target, false).is_empty());
    assert!(!pads.f.app.pad_held[0]);
    pads.cover = None;
    pads.frame(vec![]);
    pads.frame(vec![]);
    assert_eq!(pads.pointer(target, true), [(0, true)]);
    pads.cover = Some(target);
    assert!(pads.frame(vec![]).1.is_empty());
    assert!(pads.f.app.pad_held[0]);
    assert_eq!(pads.pointer(target, false), [(0, false)]);
    assert!(!pads.f.app.pad_held[0]);
}

#[test]
fn mouse_context_and_focused_shift_escape_stop_exact_slots() {
    let mut pads = Pads::new();
    pads.instrument(SamplerInstrument::Samples);
    let first = pads.points[0]; let second = pads.points[1];
    pads.pointer(first, true); pads.pointer(first, false);
    pads.pointer(second, true); pads.pointer(second, false);
    assert!(pads.f.rt.pad_voices[0].is_some()); assert!(pads.f.rt.pad_voices[1].is_some());
    for pressed in [true, false] { pads.frame(vec![egui::Event::PointerMoved(first), egui::Event::PointerButton {
        pos: first, button: PointerButton::Secondary, pressed, modifiers: Default::default(),
    }]); }
    let menu = pads.frame(vec![]).0;
    let stop = label_center(&menu, "Stop slot");
    pads.pointer(stop, true); pads.pointer(stop, false);
    assert_eq!(pads.stops, [0]);
    assert!(pads.f.rt.pad_voices[0].is_none()); assert!(pads.f.rt.pad_voices[1].is_some());
    pads.frame(vec![]);
    assert_eq!(pads.pointer(second, true), [(1, true)]);
    assert_eq!(pads.pointer(second, false), [(1, false)]);
    assert!(pads.ctx.memory(|memory| memory.focused().is_some()), "pointer press must give its pad focus");
    let focused = pads.ctx.memory(|memory| memory.focused()).unwrap();
    assert!(pads.ctx.read_response(focused).unwrap().rect.contains(second), "focus must belong to the selected pad");
    pads.frame(vec![egui::Event::Key { key: Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::SHIFT }]);
    assert_eq!(pads.stops, [0, 1]);
    assert!(pads.f.rt.pad_voices[1].is_none());
}
