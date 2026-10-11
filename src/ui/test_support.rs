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

    #[track_caller]
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

/// Find the menu that contains a moved workspace control.
/// Takes its visible label; returns the menu to open before exercising the real widget.
pub(super) fn workspace_menu(label: &str) -> Option<&'static str> {
    match label {
        "Help" | "MIDI" | "Background jobs" | "Diagnostics" | "Content & licenses" | "Music providers" | "Video" | "Audio export & recording" | "Session clips" | "Arrangement timeline" | "Audio clip" | "Automation" | "Preferences" | "Workspaces" | "Touch & pen" | "Mic & aux" | "Audio routing" | "Audio devices and latency" | "Saved audio pending" | "Setup notice" | "Audio operation pending" | "Audio operation failed" => Some("Tools"),
        "Turn on performance mode" | "Turn off performance mode…" | "Stop everything…" | "Mute output…" | "Resume controls…" | "Reset audio and unmute…" => Some("Performance"),
        "Cancel project operation" | "History" | "History · recording" => Some("Project"),
        "Next live set…" | "Support and crash reports…" | "Autosave and recovery…" | "Import MIDI file…" | "Export MIDI file…" | "Project dependencies…" | "Portable project…" | "Project and track templates…" | "Named versions…" | "Import from another project…" | "Import Ableton Live Set…" | "DJ FX…" | "Tempo and meter…" => Some("Project tools"),
        label if label.starts_with("Commands") => Some("Tools"),
        label if label.starts_with("Master effect ") || label.starts_with("1: ") || label.starts_with("2: ") || label.starts_with("3: ") => Some("FX"),
        _ => None,
    }
}

/// Open a workspace menu through its actual accessibility action.
/// Takes current nodes and the desired control label; returns input events only when that control is hidden.
pub(super) fn workspace_menu_events(nodes: &[(egui::accesskit::NodeId, egui::accesskit::Node)], label: &str) -> Option<Vec<egui::Event>> {
    if nodes.iter().any(|(_, node)| node.label() == Some(label)) { return None; }
    let menu = workspace_menu(label)?;
    let menu = if menu == "Project tools" && !nodes.iter().any(|(_, node)| node.label().is_some_and(|label| label.starts_with(menu))) { "Project" } else { menu };
    let target = nodes.iter().find_map(|(id, node)| {
        let matches = (node.label() == Some(menu) || (menu == "Project tools" && node.label().is_some_and(|label| label.starts_with(menu)))) || (menu == "Performance" && matches!(node.label(), Some("Output muted" | "Controls paused" | "Live" | "Muted" | "Paused"))) || (menu == "Project" && node.label() == Some("File"));
        if matches && menu == "Project tools" { return node.bounds().map(|bounds| (
            *id, Some(egui::pos2(((bounds.x0 + bounds.x1) / 2.0) as f32, ((bounds.y0 + bounds.y1) / 2.0) as f32))
        )); }
        (matches && node.supports_action(egui::accesskit::Action::Click)).then_some((*id, None))
    })?;
    if let Some(pos) = target.1 { return Some(vec![egui::Event::PointerMoved(pos)]); }
    Some(vec![egui::Event::AccessKitActionRequest(egui::accesskit::ActionRequest {
        target: target.0, action: egui::accesskit::Action::Click, data: None,
    })])
}
