use super::test_support::{label_center, Fixture};
use super::*;
use crate::engine::{sampler_identity_tests as sound, SynthInstrument};

struct Menu {
    fixture: Fixture,
    context: egui::Context,
    time: f64,
}

impl Menu {
    fn new() -> Self {
        let mut fixture = Fixture::new(128);
        sound::prepare(&mut fixture.rt);
        Self {
            fixture,
            context: egui::Context::default(),
            time: 0.0,
        }
    }

    fn frame(&mut self, events: Vec<egui::Event>) -> egui::FullOutput {
        self.time += 0.02;
        self.fixture.rt.publish_for_test();
        self.fixture.app.snap = self.fixture.app.engine.snapshot();
        let theme = self.fixture.app.theme.clone();
        let output = self.context.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1000.0, 300.0))),
                time: Some(self.time),
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default()
                    .show(ctx, |ui| self.fixture.app.sampler_row(ui, &theme));
            },
        );
        self.fixture.rt.process(&mut []);
        output
    }

    fn click(&mut self, pos: Pos2) {
        for pressed in [true, false] {
            self.frame(vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                },
            ]);
        }
    }
}

#[test]
fn every_visible_instrument_menu_choice_selects_its_named_identity_envelope_and_sound() {
    let choices = [
        ("samples", SamplerInstrument::Samples),
        ("analog", SamplerInstrument::Synth(SynthInstrument::Analog)),
        ("keys", SamplerInstrument::Synth(SynthInstrument::Keys)),
        ("pad", SamplerInstrument::Synth(SynthInstrument::Pad)),
    ];
    let mut sounds = Vec::new();
    for (label, expected) in choices {
        let mut menu = Menu::new();
        menu.frame(vec![]);
        let output = menu.frame(vec![]);
        menu.click(label_center(&output, "samples"));
        let output = menu.frame(vec![]);
        let labels: Vec<_> = output
            .shapes
            .iter()
            .filter_map(|shape| {
                if let egui::epaint::Shape::Text(text) = &shape.shape {
                    Some(text.galley.text().to_owned())
                } else {
                    None
                }
            })
            .collect();
        for (label, _) in choices {
            assert!(
                labels.iter().any(|text| text == label),
                "missing menu choice {label}"
            );
        }
        assert!(
            !labels.iter().any(|text| text == "drums"),
            "unsupported synth must not appear"
        );
        let before = menu.fixture.rt.command_stats.received;
        menu.click(label_center(&output, label));
        assert_eq!(menu.fixture.rt.command_stats.received, before + 1);
        assert_eq!(menu.fixture.rt.sampler_inst, expected);
        let output = menu.frame(vec![]);
        label_center(&output, label); // actual closed ComboBox now displays selection
        assert_eq!(menu.fixture.app.snap.sampler_inst, expected);
        assert_eq!(menu.fixture.app.snap.sampler_banks, ["Kit", "Perc", "Hits"]);
        sounds.push(sound::assert_selected_sound(&mut menu.fixture.rt, expected));
    }
    for left in 0..sounds.len() {
        for right in left + 1..sounds.len() {
            assert!(
                sounds[left]
                    .iter()
                    .zip(&sounds[right])
                    .map(|(a, b)| (a[0] - b[0]).abs())
                    .sum::<f32>()
                    > 10.0,
                "menu choices {left}/{right} rendered aliased sounds"
            );
        }
    }
}
