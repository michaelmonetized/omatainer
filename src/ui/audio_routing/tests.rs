use super::*;
use crate::ui::piano_roll::tests::Gui;
use std::time::Duration;

#[test]
fn native_latency_controls_preview_confirm_live_policy_and_undo_without_device_owners() {
    let mut gui = Gui::new();
    gui.click("Audio routing");
    gui.click("Refresh routes");
    settled(&mut gui);
    gui.click("Use explicit routing");
    gui.click("Latency compensation");
    gui.click("Compensate reported latency");
    gui.action("Delay reserve (ms)", egui::accesskit::Action::SetValue, Some(egui::accesskit::ActionData::NumericValue(12.0)));
    assert_eq!(gui.app.audio_routing.draft.as_ref().unwrap().model.latency.as_ref().unwrap().reserve_micros, 12000);
    gui.click("Review routing change…");
    gui.click("Cancel routing change");
    assert!(gui.rt.routing.is_none());
    gui.click("Review routing change…");
    gui.click("Apply routing");
    settled(&mut gui);
    assert!(gui.app.audio_routing.error.is_none(), "{:?}", gui.app.audio_routing.error);
    let saved = gui.rt.routing.as_ref().unwrap().model.clone();
    assert_eq!(saved.latency.as_ref().unwrap().reserve_micros, 12000);
    gui.rt.apply(Command::Play);
    gui.click("Refresh routes");
    settled(&mut gui);
    gui.click("Immediate headphone monitoring");
    gui.click("Review routing change…");
    gui.click("Apply routing");
    settled(&mut gui);
    assert!(gui.rt.playing);
    assert!(gui.app.audio_routing.error.is_none(), "{:?}", gui.app.audio_routing.error);
    assert!(gui.rt.routing.as_ref().unwrap().model.latency.as_ref().unwrap().low_latency_monitor);
    gui.rt.apply(Command::Undo);
    assert_eq!(gui.rt.routing.as_ref().unwrap().model, saved);
    let decoded: Model = serde_json::from_slice(&serde_json::to_vec(&saved).unwrap()).unwrap();
    assert_eq!(decoded, *saved);
    assert!(crate::engine::audio::routing::prepared::Prepared::at_rate(Arc::new(decoded), &gui.rt.session, 48000).is_ok());
}

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
    for _ in 0..8 {
        gui.frame(vec![]);
    }
    gui.click("Save input choice");
    gui.app
        .audio_routing
        .draft
        .as_mut()
        .unwrap()
        .model
        .input
        .as_mut()
        .unwrap()
        .device = "Reviewed fixture device".into();
    gui.click("Request input buffer size");
    for _ in 0..8 {
        gui.frame(vec![]);
    }
    let label = gui
        .nodes
        .iter()
        .filter_map(|(_, node)| node.label())
        .find(|label| label.ends_with("Input buffer frames"))
        .unwrap()
        .to_owned();
    gui.action(
        &label,
        Action::SetValue,
        Some(ActionData::NumericValue(1024.0)),
    );
    assert_eq!(
        gui.app
            .audio_routing
            .draft
            .as_ref()
            .unwrap()
            .model
            .input
            .as_ref()
            .unwrap()
            .buffer_frames,
        Some(1024)
    );
    gui.click("Review routing change…");
    gui.click("Apply routing");
    settled(&mut gui);
    assert!(
        gui.app.audio_routing.error.is_none(),
        "{:?}",
        gui.app.audio_routing.error
    );
    assert_eq!(
        gui.rt
            .routing
            .as_ref()
            .unwrap()
            .model
            .input
            .as_ref()
            .unwrap()
            .buffer_frames,
        Some(1024)
    );
    let handle = gui.app.engine.project.clone();
    let capture = std::thread::spawn(move || handle.capture(&AtomicBool::new(false)));
    let deadline = Instant::now() + Duration::from_secs(15);
    while !capture.is_finished() {
        gui.frame(vec![]);
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(1));
    }
    let state = capture.join().unwrap().unwrap();
    let reopened = crate::engine::project::Prepared::from_state(state.state, state.media, 96000)
        .unwrap()
        .into_offline();
    assert_eq!(
        reopened
            .routing
            .as_ref()
            .unwrap()
            .model
            .input
            .as_ref()
            .unwrap()
            .buffer_frames,
        Some(1024)
    );
    gui.click("Refresh routes");
    settled(&mut gui);
    for _ in 0..8 {
        gui.frame(vec![]);
    }
    gui.click("Request input buffer size");
    assert_eq!(
        gui.app
            .audio_routing
            .draft
            .as_ref()
            .unwrap()
            .model
            .input
            .as_ref()
            .unwrap()
            .buffer_frames,
        None
    );
    gui.click("Review routing change…");
    gui.click("Cancel routing change");
    assert_eq!(
        gui.rt
            .routing
            .as_ref()
            .unwrap()
            .model
            .input
            .as_ref()
            .unwrap()
            .buffer_frames,
        Some(1024)
    );
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
        spectrum: None,
        name: "Print source".into(),
        sr: 48000,
        ch: 2,
        data: vec![0.125; 200000],
        peaks: Vec::new().into(),
        bpm: 120.0,
        path: String::new(),
    }));
    gui.rt.decks[0].playing = true;
    gui.rt.apply(Command::Play);
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
    let metadata = path.with_file_name(format!(
        "{}.omatainer.json",
        path.file_name().unwrap().to_string_lossy()
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
            assert!(!metadata.exists());
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
            let timing: serde_json::Value =
                serde_json::from_slice(&std::fs::read(&metadata).unwrap()).unwrap();
            assert_eq!(timing["placement"]["graph_delay_frames"], 0);
            assert!(timing["audio_sha256"].as_str().is_some_and(|hash| hash.len() == 64));
            gui.rt.apply(Command::Stop);
            gui.rt.decks[0].playing = false;
            gui.app.audio_routing.record_track = gui.rt.session.reference(crate::engine::session::Axis::Track, 2);
            gui.frame(vec![]);
            gui.click("Review recording placement");
            settled(&mut gui);
            assert!(gui.app.audio_routing.error.is_none(), "{:?}", gui.app.audio_routing.error);
            assert!(gui.app.audio_routing.recording_review.is_some());
            gui.click("Discard recording review");
            assert!(gui.rt.arrangement.plan.is_none());
            gui.click("Review recording placement");
            settled(&mut gui);
            gui.click("Place recording on song");
            settled(&mut gui);
            assert!(gui.app.audio_routing.error.is_none(), "{:?}", gui.app.audio_routing.error);
            assert!(gui.rt.arrangement.plan.is_some());
            assert!(!gui.rt.arrangement.enabled());
            gui.rt.apply(Command::Undo);
            assert!(gui.rt.arrangement.plan.is_none());
            gui.click("Refresh routes");
            settled(&mut gui);
            gui.rt.decks[0].playing = true;
            gui.rt.apply(Command::Play);
            std::fs::remove_file(&path).unwrap();
            std::fs::remove_file(&metadata).unwrap();
        }
    }
}

#[test]
fn native_headphone_pair_review_persists_exact_alias_and_rejects_program_overlap() {
    let mut gui = Gui::new();
    gui.click("Audio routing");
    gui.click("Refresh routes");
    settled(&mut gui);
    gui.click("Use explicit routing");
    let model = &mut gui.app.audio_routing.draft.as_mut().unwrap().model;
    model.next_id = 3;
    model.ports.push(Port {
        id: 2,
        alias: "Booth headphones".into(),
        direction: Direction::Output,
        channels: vec![4, 5],
    });
    gui.frame(vec![]);
    gui.click("Headphone output");
    gui.click("Headphones: Booth headphones (outputs 5/6)");
    gui.click("Review routing change…");
    gui.click("Apply routing");
    settled(&mut gui);
    assert!(
        gui.app.audio_routing.error.is_none(),
        "{:?}",
        gui.app.audio_routing.error
    );
    assert_eq!(
        gui.rt.routing.as_ref().unwrap().model.monitor_output,
        Some(2)
    );
    let handle = gui.app.engine.project.clone();
    let capture = std::thread::spawn(move || handle.capture(&AtomicBool::new(false)));
    let deadline = Instant::now() + Duration::from_secs(15);
    while !capture.is_finished() {
        gui.frame(vec![]);
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(1));
    }
    let captured = capture.join().unwrap().unwrap();
    let bytes = serde_json::to_vec(&captured.state).unwrap();
    let state = serde_json::from_slice(&bytes).unwrap();
    let prepared =
        crate::engine::project::Prepared::from_state(state, captured.media, 48000).unwrap();
    let handle = gui.app.engine.project.clone();
    let reopen = std::thread::spawn(move || {
        handle.install(prepared, handle.revision(), &AtomicBool::new(false))
    });
    while !reopen.is_finished() {
        gui.frame(vec![]);
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(1));
    }
    reopen.join().unwrap().unwrap();
    assert_eq!(
        gui.rt.routing.as_ref().unwrap().model.monitor_output,
        Some(2)
    );
    assert_eq!(gui.rt.routing.as_ref().unwrap().model.version, 2);
    gui.click("Refresh routes");
    settled(&mut gui);
    let draft = gui.app.audio_routing.draft.as_mut().unwrap();
    draft.model.ports[0].channels = vec![4, 5];
    gui.click("Review routing change…");
    gui.click("Apply routing");
    settled(&mut gui);
    assert!(gui
        .app
        .audio_routing
        .error
        .as_ref()
        .unwrap()
        .contains("overlap"));
    assert_eq!(
        gui.rt.routing.as_ref().unwrap().model.ports[0].channels,
        [0, 1]
    );
    assert_eq!(
        gui.rt.routing.as_ref().unwrap().model.monitor_output,
        Some(2)
    );
}

#[test]
#[ignore="requires the freshly compiled native worker and original SDK contract fixture"]
fn native_browser_attaches_instrument_and_ordered_effect_through_review_cancel_and_undo() {
    use crate::ui::piano_roll::tests::Gui;
    use crate::plugin_host::scanner::Record;
    let (instrument,saved)=crate::engine::audio::routing::plugin_tests::fixture(true);
    let (effect,_)=crate::engine::audio::routing::plugin_tests::fixture(false);
    let mut gui=Gui::new();gui.app.plugins.catalog.records=vec![Record{path:saved.binary.bundle.clone(),binary:Some(saved.binary),classes:vec![instrument,effect],failure:None}];gui.app.plugins.selected=Some((0,0));gui.app.plugins.open=true;gui.frame(vec![]);
    gui.click("Use as track instrument");settle_graph(&mut gui);gui.app.plugins.open=false;gui.frame(vec![]);
    assert_eq!(gui.app.audio_routing.draft.as_ref().unwrap().model.plugins.len(),1);
    gui.click("Review routing change…");gui.click("Cancel routing change");assert!(gui.rt.routing.is_none());
    gui.click("Review routing change…");gui.click("Apply routing");settle_graph(&mut gui);
    assert!(gui.app.audio_routing.error.is_none(),"{:?}",gui.app.audio_routing.error);
    let original=gui.rt.routing.as_ref().unwrap().model.clone();assert!(original.plugins[0].instrument);
    gui.rt.apply(Command::Undo);assert!(gui.rt.routing.is_none());gui.rt.apply(Command::Redo);assert_eq!(gui.rt.routing.as_ref().unwrap().model,original);
    gui.app.audio_routing.open=false;gui.app.plugins.selected=Some((0,1));gui.app.plugins.open=true;gui.frame(vec![]);gui.click("Append track effect");settle_graph(&mut gui);gui.app.plugins.open=false;gui.frame(vec![]);gui.click("Review routing change…");gui.click("Apply routing");settle_graph(&mut gui);
    assert!(gui.app.audio_routing.error.is_none(),"{:?}",gui.app.audio_routing.error);
    let graph=gui.rt.routing.as_ref().unwrap();assert_eq!(graph.model.plugins.len(),2);assert!(!graph.model.plugins[1].instrument);assert_eq!(graph.model.plugins[1].scene_track,graph.model.plugins[0].midi_track);assert!(graph.plugins.iter().all(|p|p.error.is_none()));
    gui.rt.apply(Command::Undo);assert_eq!(gui.rt.routing.as_ref().unwrap().model,original);
}
fn settle_graph(gui:&mut crate::ui::piano_roll::tests::Gui){
    let deadline=Instant::now()+std::time::Duration::from_secs(20);
    loop{gui.frame(vec![]);if !gui.app.audio_routing.busy(){break;}assert!(Instant::now()<deadline,"{:?}",gui.app.audio_routing.error);std::thread::sleep(std::time::Duration::from_millis(2));}
    for _ in 0..4{gui.frame(vec![]);}
}

#[test]
#[ignore = "requires the native worker and the locally built licensed Nekobi fixture"]
fn native_hidden_cc_parameters_leave_sound_controls_editable_and_persistent() {
    use crate::plugin_host::scanner::Record;
    let files = crate::engine::media_analysis::tests::Files::new();
    let root = PathBuf::from(std::env::var_os("OMATAINER_VST3_FIXTURES").unwrap());
    let (class, saved) = crate::engine::audio::routing::plugin_tests::probe(
        root.join("dpf-plugins/bin/Nekobi.vst3"), true,
    );
    assert!(class.parameters.iter().take(128).any(|p| p.flags & (1 << 4) != 0));
    let cutoff = class.parameters.iter().find(|p| p.name == "Cutoff").unwrap().id;
    let mut gui = Gui::new();
    gui.app.plugins.catalog.records = vec![Record {
        path: saved.binary.bundle.clone(), binary: Some(saved.binary),
        classes: vec![class], failure: None,
    }];
    gui.app.plugins.selected = Some((0, 0));
    gui.app.plugins.open = true;
    gui.frame(vec![]);
    gui.click("Use as track instrument");
    settle_graph(&mut gui);
    gui.app.plugins.open = false;
    gui.frame(vec![]);
    gui.click("Review routing change…");
    gui.click("Apply routing");
    settle_graph(&mut gui);
    assert!(gui.app.audio_routing.error.is_none(), "{:?}", gui.app.audio_routing.error);
    gui.click("Refresh routes");
    settle_graph(&mut gui);
    gui.click("Native plugin processors");
    gui.click("Normalized parameters");
    assert!(!gui.nodes.iter().any(|(_, n)| n.label().is_some_and(|s| s.starts_with("MIDI Ch."))));
    gui.action("Cutoff", egui::accesskit::Action::SetValue,
        Some(egui::accesskit::ActionData::NumericValue(0.625)));
    let control = gui.rt.routing.as_ref().unwrap().plugins[0].endpoint.as_ref().unwrap().control.clone();
    let deadline = Instant::now() + Duration::from_secs(10);
    while control.value(cutoff).is_none_or(|v| (v - 0.625).abs() > 1e-9) {
        gui.frame(vec![]);
        assert!(Instant::now() < deadline, "{:?}", control.error());
        std::thread::sleep(Duration::from_millis(2));
    }
    let handle = gui.app.engine.project.clone();
    let captured = std::thread::spawn(move || handle.capture(&AtomicBool::new(false)));
    while !captured.is_finished() {
        gui.frame(vec![]);
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(2));
    }
    let captured = captured.join().unwrap().unwrap();
    let retained = &captured.state.routing.as_ref().unwrap().plugins[0];
    assert_eq!(retained.parameters.iter().find(|p| p.id == cutoff).unwrap().value, 0.625);
    assert!(!retained.saved.state.is_empty());
    let path = files.0.join("Sound control.omatainer");
    let cancel = AtomicBool::new(false);
    crate::project_file::save(&path, &crate::project_file::Bundle {
        state: captured.state, media: captured.media,
    }, crate::project_file::Overwrite::Never, &Default::default(), &cancel).unwrap();
    let saved: crate::project_file::Bundle<crate::engine::project::State> =
        crate::project_file::load(&path, &Default::default(), &cancel).unwrap();
    let reopened = crate::engine::project::Prepared::from_state(saved.state, saved.media, 48000).unwrap().into_offline();
    let processor = &reopened.routing.as_ref().unwrap().plugins[0];
    assert!(processor.error.is_none(), "{:?}", processor.error);
    assert!((processor.endpoint.as_ref().unwrap().control.value(cutoff).unwrap() - 0.625).abs() < 1e-9);
}

#[test]
#[ignore="opens locally built GPL MVerb editor on the real X11 display; requires OMATAINER_VST3_EDITOR_QUALIFY_DIR"]
fn native_plugin_editor_opens_from_the_routing_widgets_and_closes_without_retiring_audio() {
    use crate::plugin_host::scanner::Record;
    let dir=std::path::PathBuf::from(std::env::var_os("OMATAINER_VST3_EDITOR_QUALIFY_DIR").expect("Private editor qualification directory is required"));std::fs::create_dir_all(&dir).unwrap();
    let root=std::path::PathBuf::from(std::env::var_os("OMATAINER_VST3_FIXTURES").unwrap());let (class,saved)=crate::engine::audio::routing::plugin_tests::probe(root.join("dpf-plugins/bin/MVerb.vst3"),false);assert!(class.info.has_gui);
    let mut gui=Gui::new();gui.app.plugins.catalog.records=vec![Record{path:saved.binary.bundle.clone(),binary:Some(saved.binary),classes:vec![class],failure:None}];gui.app.plugins.selected=Some((0,0));gui.app.plugins.open=true;gui.frame(vec![]);gui.click("Append track effect");settle_graph(&mut gui);gui.app.plugins.open=false;gui.frame(vec![]);gui.click("Review routing change…");gui.click("Apply routing");settle_graph(&mut gui);assert!(gui.app.audio_routing.error.is_none(),"{:?}",gui.app.audio_routing.error);
    gui.click("Refresh routes");settle_graph(&mut gui);gui.click("Native plugin processors");gui.click("Open plugin editor");
    let control=gui.rt.routing.as_ref().unwrap().plugins[0].endpoint.as_ref().unwrap().control.clone();let until=Instant::now()+Duration::from_secs(10);
    while !control.editor_open(){gui.frame(vec![]);assert!(Instant::now()<until,"{:?}",control.editor_error());std::thread::sleep(Duration::from_millis(20));}
    std::fs::write(dir.join("editor-open.txt"),"MVerb native editor opened through routing widgets\n").unwrap();
    let until=Instant::now()+Duration::from_secs(12);while Instant::now()<until{gui.frame(vec![]);std::thread::sleep(Duration::from_millis(20));}
    gui.click("Close plugin editor");let until=Instant::now()+Duration::from_secs(10);while control.editor_open(){gui.frame(vec![]);assert!(Instant::now()<until);std::thread::sleep(Duration::from_millis(20));}
    assert!(control.error().is_none(),"{:?}",control.error());assert!(control.editor_error().is_none());assert!(!gui.rt.routing.as_ref().unwrap().plugins[0].endpoint.as_ref().unwrap().faulted());std::fs::write(dir.join("editor-closed.txt"),"Editor closed; processor and state capture remain available\n").unwrap();
}
