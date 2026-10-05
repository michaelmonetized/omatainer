use super::*;
use crate::ui::piano_roll::tests::Gui;
use std::time::Duration;

fn settled(gui: &mut Gui) {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        gui.frame(vec![]);
        if !gui.app.audio_routing.busy() {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "{:?}",
            gui.app.audio_routing.error
        );
        std::thread::sleep(Duration::from_millis(1));
    }
    for _ in 0..4 {
        gui.frame(vec![]);
    }
}

#[test]
fn native_routing_editor_confirms_cancels_rejects_cycles_and_undoes_saved_routes() {
    let mut gui = Gui::new();
    gui.click("Audio routing");
    gui.click("Refresh routes");
    settled(&mut gui);
    assert!(gui.app.audio_routing.error.is_none());
    gui.click("Use explicit routing");
    gui.frame(vec![]);
    gui.click("Review routing change…");
    gui.click("Cancel routing change");
    assert!(gui.rt.routing.is_none());
    gui.click("Review routing change…");
    gui.click("Apply routing");
    settled(&mut gui);
    assert!(
        gui.app.audio_routing.error.is_none(),
        "{:?}",
        gui.app.audio_routing.error
    );
    assert!(gui.rt.routing.is_some());
    gui.rt.apply(Command::Undo);
    assert!(gui.rt.routing.is_none());
    gui.rt.apply(Command::Redo);
    assert!(gui.rt.routing.is_some());
    let handle = gui.app.engine.project.clone();
    let capture = std::thread::spawn(move || handle.capture(&AtomicBool::new(false)));
    let deadline = Instant::now() + Duration::from_secs(15);
    while !capture.is_finished() {
        gui.frame(vec![]);
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(1));
    }
    let state = capture.join().unwrap().unwrap();
    assert!(state.state.routing.is_some());
    let model = state.state.routing.clone();
    let opened =
        crate::engine::project::Prepared::from_state(state.state, state.media, 48000).unwrap();
    let handle = gui.app.engine.project.clone();
    let install = std::thread::spawn(move || {
        handle.install(opened, handle.revision(), &AtomicBool::new(false))
    });
    while !install.is_finished() {
        gui.frame(vec![]);
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(1));
    }
    install.join().unwrap().unwrap();
    assert_eq!(gui.rt.routing.as_ref().unwrap().model, model.unwrap());
    gui.click("Refresh routes");
    settled(&mut gui);
    let draft = gui.app.audio_routing.draft.as_mut().unwrap();
    let id = draft.layout.tracks[0].id;
    draft.model.connections.push(Connection {
        source: Source {
            group: Group::Main,
            tap: Tap::PostFx,
        },
        destination: Group::Track(id),
        map: vec![ChannelMap {
            source: 0,
            destination: 0,
            gain: 1.0,
        }],
    });
    let before = gui.rt.routing.as_ref().unwrap().model.clone();
    gui.frame(vec![]);
    gui.click("Review routing change…");
    gui.click("Apply routing");
    settled(&mut gui);
    assert!(gui
        .app
        .audio_routing
        .error
        .as_ref()
        .unwrap()
        .contains("feedback"));
    assert_eq!(gui.rt.routing.as_ref().unwrap().model, before);
}

#[test]
fn routing_worker_cancellation_and_stale_confirmation_preserve_audio() {
    let mut gui = Gui::new();
    gui.click("Audio routing");
    gui.click("Refresh routes");
    settled(&mut gui);
    gui.click("Use explicit routing");
    gui.frame(vec![]);
    gui.click("Review routing change…");
    gui.rt.apply(Command::Master(0.31));
    gui.click("Apply routing");
    settled(&mut gui);
    assert!(gui.rt.routing.is_none());
    assert!(gui.app.audio_routing.error.is_some());
    gui.rt.apply(Command::Stop);
    gui.click("Refresh routes");
    gui.app.audio_routing.cancel();
    settled(&mut gui);
    assert!(gui.rt.routing.is_none());
    gui.click("Refresh routes");
    settled(&mut gui);
    assert!(
        gui.app.audio_routing.error.is_none(),
        "{:?}",
        gui.app.audio_routing.error
    );
}

#[test]
fn native_alias_channel_values_apply_and_cancel_without_retargeting_saved_routes() {
    use egui::accesskit::{Action, ActionData};
    let mut gui = Gui::new();
    gui.click("Audio routing");
    gui.click("Refresh routes");
    settled(&mut gui);
    gui.click("Use explicit routing");
    gui.click("Channel aliases and buses");
    for _ in 0..8 {
        gui.frame(vec![]);
    }
    let label = gui
        .nodes
        .iter()
        .filter_map(|(_, node)| node.label())
        .find(|label| label.ends_with("Alias 1 physical channel 1"))
        .unwrap()
        .to_owned();
    gui.action(
        &label,
        Action::SetValue,
        Some(ActionData::NumericValue(32.0)),
    );
    assert_eq!(
        gui.app.audio_routing.draft.as_ref().unwrap().model.ports[0].channels[0],
        31
    );
    gui.click("Review routing change…");
    gui.click("Apply routing");
    settled(&mut gui);
    assert_eq!(
        gui.rt.routing.as_ref().unwrap().model.ports[0].channels[0],
        31
    );
    gui.click("Refresh routes");
    gui.app.audio_routing.cancel();
    settled(&mut gui);
    assert_eq!(
        gui.rt.routing.as_ref().unwrap().model.ports[0].channels[0],
        31
    );
}

#[test]
fn native_input_buffer_control_is_reviewed_saved_reopened_and_cancelled() {
    use egui::accesskit::{Action, ActionData};
    let mut gui = Gui::new();
    gui.click("Audio routing");
    gui.click("Refresh routes");
    settled(&mut gui);
    gui.click("Use explicit routing");
    gui.click("Live input choices");
    for _ in 0..8 { gui.frame(vec![]); }
    gui.click("Save input choice");
    gui.app.audio_routing.draft.as_mut().unwrap().model.input.as_mut().unwrap().device = "Reviewed fixture device".into();
    gui.click("Request input buffer size");
    for _ in 0..8 { gui.frame(vec![]); }
    let label = gui.nodes.iter().filter_map(|(_, node)| node.label()).find(|label| label.ends_with("Input buffer frames")).unwrap().to_owned();
    gui.action(&label, Action::SetValue, Some(ActionData::NumericValue(1024.0)));
    assert_eq!(gui.app.audio_routing.draft.as_ref().unwrap().model.input.as_ref().unwrap().buffer_frames, Some(1024));
    gui.click("Review routing change…");
    gui.click("Apply routing");
    settled(&mut gui);
    assert!(gui.app.audio_routing.error.is_none(), "{:?}", gui.app.audio_routing.error);
    assert_eq!(gui.rt.routing.as_ref().unwrap().model.input.as_ref().unwrap().buffer_frames, Some(1024));
    let handle = gui.app.engine.project.clone();
    let capture = std::thread::spawn(move || handle.capture(&AtomicBool::new(false)));
    let deadline = Instant::now() + Duration::from_secs(15);
    while !capture.is_finished() { gui.frame(vec![]); assert!(Instant::now() < deadline); std::thread::sleep(Duration::from_millis(1)); }
    let state = capture.join().unwrap().unwrap();
    let reopened = crate::engine::project::Prepared::from_state(state.state, state.media, 96000).unwrap().into_offline();
    assert_eq!(reopened.routing.as_ref().unwrap().model.input.as_ref().unwrap().buffer_frames, Some(1024));
    gui.click("Refresh routes");
    settled(&mut gui);
    for _ in 0..8 { gui.frame(vec![]); }
    gui.click("Request input buffer size");
    assert_eq!(gui.app.audio_routing.draft.as_ref().unwrap().model.input.as_ref().unwrap().buffer_frames, None);
    gui.click("Review routing change…");
    gui.click("Cancel routing change");
    assert_eq!(gui.rt.routing.as_ref().unwrap().model.input.as_ref().unwrap().buffer_frames, Some(1024));
}

#[test]
fn native_record_source_controls_publish_complete_audio_and_cancel_partial_files() {
    let mut gui = Gui::new();
    gui.click("Audio routing");
    gui.click("Refresh routes");
    settled(&mut gui);
    gui.click("Use explicit routing");
    let draft = gui.app.audio_routing.draft.as_mut().unwrap();
    draft.model.next_id = 3;
    draft.model.ports.push(Port {
        id: 2,
        alias: "Deck print".into(),
        direction: Direction::Record,
        channels: vec![0, 1],
    });
    draft.model.connections.push(Connection {
        source: Source {
            group: Group::Deck(0),
            tap: Tap::PreFx,
        },
        destination: Group::Record(2),
        map: (0..2)
            .map(|channel| ChannelMap {
                source: channel,
                destination: channel,
                gain: 1.0,
            })
            .collect(),
    });
    gui.frame(vec![]);
    gui.click("Review routing change…");
    gui.click("Apply routing");
    settled(&mut gui);
    gui.click("Refresh routes");
    settled(&mut gui);
    gui.rt.decks[0].audio = Some(Arc::new(crate::engine::dsp::Sample {
        name: "Print source".into(),
        sr: 48000,
        ch: 2,
        data: vec![0.125; 200000],
        peaks: Vec::new().into(),
        bpm: 120.0,
        path: String::new(),
    }));
    gui.rt.decks[0].playing = true;
    gui.rt.decks[0].sync = false;
    gui.rt.decks[0].keylock = false;
    gui.app.audio_routing.record_alias = 2;
    gui.click("Capture a record source");
    for _ in 0..8 {
        gui.frame(vec![]);
    }
    let path = std::env::temp_dir().join(format!(
        "omatainer-ui-routing-{}.wav",
        crate::sampler_bank::BankId::new().unwrap()
    ));
    for cancel in [false, true] {
        gui.app.audio_routing.record_path = path.to_string_lossy().into_owned();
        gui.frame(vec![]);
        gui.click("Record source to WAV");
        let deadline = Instant::now() + Duration::from_secs(15);
        while gui.app.engine.routing.recorder.alias() == 0 {
            gui.frame(vec![]);
            assert!(
                Instant::now() < deadline,
                "{:?}",
                gui.app.audio_routing.error
            );
            std::thread::sleep(Duration::from_millis(1));
        }
        for _ in 0..8 {
            gui.frame(vec![]);
        }
        gui.click(if cancel {
            "Cancel routing operation"
        } else {
            "Stop record-source capture"
        });
        settled(&mut gui);
        if cancel {
            assert!(!path.exists());
        } else {
            assert!(
                gui.app.audio_routing.error.is_none(),
                "{:?}",
                gui.app.audio_routing.error
            );
            let decoded = crate::engine::decode::decode_audio(&path).unwrap();
            assert_eq!(decoded.sample.ch, 2);
            assert!(decoded.sample.frames() >= 512);
            assert!(decoded.sample.data.iter().all(|value| *value == 0.125));
            std::fs::remove_file(&path).unwrap();
        }
    }
}
