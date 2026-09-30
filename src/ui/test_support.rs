//! Real GUI state, command admission, renderer and load channels, with only the
//! external audio/MIDI device connections and decode thread left unopened.
use super::*;
use crate::engine::{
    decode::{DecodeFailure, DecodedAudio},
    RtEngine,
};

pub(super) struct Fixture {
    pub app: App,
    pub rt: RtEngine,
    pub decoder_jobs: mpsc::Receiver<(u8, PathBuf)>,
    pub decoder_results: mpsc::Sender<(u8, Result<DecodedAudio, DecodeFailure>)>,
}

impl Fixture {
    pub fn new(capacity: usize) -> Self {
        let (engine, rt) = Engine::headless_for_test(48_000, capacity);
        let (load_tx, decoder_jobs) = mpsc::channel();
        let (decoder_results, load_rx) = mpsc::channel();
        let mut app = App::with_loader(engine, Theme::default(), load_tx, load_rx);
        app.library = builtin_crate_items();
        Self {
            app,
            rt,
            decoder_jobs,
            decoder_results,
        }
    }
}

pub(super) fn crate_frame(
    ctx: &egui::Context,
    app: &mut App,
    time: f64,
    events: Vec<egui::Event>,
) -> egui::FullOutput {
    let input = egui::RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1440.0, 240.0))),
        time: Some(time),
        events,
        ..Default::default()
    };
    let theme = app.theme.clone();
    ctx.run(input, |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| app.crate_row(ui, &theme));
    })
}

pub(super) fn label_center(output: &egui::FullOutput, label: &str) -> Pos2 {
    output
        .shapes
        .iter()
        .rev()
        .find_map(|shape| match &shape.shape {
            egui::epaint::Shape::Text(text) if text.galley.text() == label => {
                Some(text.visual_bounding_rect().center())
            }
            _ => None,
        })
        .unwrap_or_else(|| panic!("missing rendered label {label:?}"))
}

pub(super) fn click(ctx: &egui::Context, app: &mut App, pos: Pos2, time: f64) {
    for (pressed, time) in [(true, time), (false, time + 0.01)] {
        crate_frame(
            ctx,
            app,
            time,
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::default(),
                },
            ],
        );
    }
}
