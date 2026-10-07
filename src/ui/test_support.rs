//! Real GUI state, command admission, renderer and bounded decoder worker.
//! External audio/MIDI devices stay unopened; tests control decoder replies.
use super::*;
use crate::engine::{
    decode::{DecodeFailure, DecodedAudio},
    RtEngine,
};
use std::sync::mpsc;
use std::time::Duration;

pub(super) struct Fixture {
    pub app: App,
    pub rt: Box<RtEngine>,
    pub decoder_jobs: mpsc::Receiver<(u8, PathBuf)>,
    pub decoder_results: mpsc::Sender<(u8, Result<DecodedAudio, DecodeFailure>)>,
}

impl Fixture {
    pub fn new(capacity: usize) -> Self {
        let (engine, rt) = Engine::headless_for_test(48_000, capacity);
        let (load_tx, decoder_jobs) = mpsc::channel();
        let (decoder_results, load_rx) =
            mpsc::channel::<(u8, Result<DecodedAudio, DecodeFailure>)>();
        let loader = Loader::with_decoder_for_show(move |path, token| {
            load_tx
                .send((token.deck, path.to_path_buf()))
                .map_err(|_| failed_decoder())?;
            let (deck, report) = load_rx.recv().map_err(|_| failed_decoder())?;
            assert_eq!(
                deck, token.deck,
                "fixture decoder result must match its active job"
            );
            report
        },engine.cmd.performance().clone())
        .unwrap();
        let app = App::with_loader(engine, Theme::default(), Some(loader));
        Self {
            app,
            rt: Box::new(rt),
            decoder_jobs,
            decoder_results,
        }
    }

    pub fn poll_loads(&mut self) {
        let loading: [bool; DECKS] = std::array::from_fn(|deck| self.app.loads[deck].as_ref()
            .is_some_and(|load| matches!(load.phase, load_status::Phase::Loading)));
        assert!(loading.iter().any(|loading| *loading), "no pending decoder request");
        let deadline = Instant::now() + Duration::from_secs(3);
        while loading.iter().enumerate().all(|(deck, was_loading)| !was_loading ||
            self.app.loads[deck].as_ref().is_some_and(|load| matches!(load.phase, load_status::Phase::Loading))) {
            self.app.poll_loads();
            assert!(Instant::now() < deadline, "no visible decoder result");
            std::thread::sleep(Duration::from_millis(1));
        }
    }
}

fn failed_decoder() -> DecodeFailure {
    DecodeFailure {
        kind: crate::engine::decode::DecodeFailureKind::Io,
        stage: crate::engine::decode::DecodeStage::Open,
        diagnostics: Default::default(),
        detail: "decoder is unavailable".into(),
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
                let visible=text.visual_bounding_rect().intersect(shape.clip_rect);
                visible.is_positive().then(||visible.center())
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
