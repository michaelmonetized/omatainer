//! Opt-in native Linux accessibility bridge. The Python harness creates private
//! D-Bus buses and consumes the actual AT-SPI interfaces; no desktop is opened.
use super::*;
use egui::accesskit::{
    ActionHandler, ActionRequest, ActivationHandler, DeactivationHandler, TreeUpdate,
};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{mpsc, Mutex as StdMutex};
use std::time::Duration;

struct Initial(Arc<StdMutex<TreeUpdate>>);
impl ActivationHandler for Initial {
    fn request_initial_tree(&mut self) -> Option<TreeUpdate> {
        Some(self.0.lock().unwrap().clone())
    }
}
struct Actions {
    send: mpsc::SyncSender<ActionRequest>,
    dropped: Arc<AtomicUsize>,
}
impl ActionHandler for Actions {
    fn do_action(&mut self, request: ActionRequest) {
        if self.send.try_send(request).is_err() {
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }
}
struct Deactivate;
impl DeactivationHandler for Deactivate {
    fn deactivate_accessibility(&mut self) {}
}

#[test]
#[ignore = "run scripts/check-accessibility.py on private D-Bus buses"]
fn private_atspi_bridge_child() {
    assert_eq!(
        std::env::var("OMATAINER_ATSPI_PRIVATE").as_deref(),
        Ok("1"),
        "use the isolated Python harness"
    );
    let directory = PathBuf::from(
        std::env::var_os("OMATAINER_ATSPI_TEST_DIR").expect("private evidence directory"),
    );
    assert!(directory.is_dir() && directory.join("private-harness").is_file());
    std::env::set_current_dir(&directory).unwrap(); // This isolated child owns its process cwd.
    assert!(std::env::var("DBUS_SESSION_BUS_ADDRESS").is_ok());
    assert!(std::env::var("AT_SPI_BUS_ADDRESS").is_ok());
    let (engine, mut rt) = Engine::headless_for_test(48_000, 256);
    rt.publish_for_test();
    let loader = Loader::start_with_performance(engine.cmd.performance().clone()).unwrap();
    let mut app = App::with_loader(engine, Theme::default(), Some(loader));
    app.sampler_editor.set_store_path(directory.join("sampler-banks.json"));
    app.start_library_store(directory.join("library.json"));
    app.start_session_history(directory.join("performance-history"));
    let sampler_path = directory.join("sampler.flac");
    std::fs::write(&sampler_path, include_bytes!("../../tests/fixtures/audio/tone.flac")).unwrap();
    let sampler_source = LibSource::File(sampler_path.clone());
    let sampler_fingerprint = FileFingerprint::read(&sampler_path).unwrap();
    Arc::make_mut(&mut app.library).push(LibItem { title: "ZZ sampler native fixture".into(), artist: "fixture".into(),
        bpm: Bpm::UNKNOWN, fingerprint: Some(sampler_fingerprint), key: String::new(), length: None,
        last_play: None, source: sampler_source.clone() });
    app.library_metadata.rebase();
    app.settings.path = Some(directory.join("preferences.json"));
    app.settings.worker = Some(crate::preferences::worker::Worker::with_discovery(
        directory.join("preferences.json"), || Ok(crate::engine::audio::config::tests::inventory())
    ).unwrap());
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let mut time = 0.0;
    let mut frame = |app: &mut App, events: Vec<egui::Event>| {
        time += 0.02;
        ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1600.0, 1200.0))),
                time: Some(time),
                events,
                focused: true,
                ..Default::default()
            },
            |ctx| app.update_frame(ctx),
        )
        .platform_output
        .accesskit_update
        .expect("real egui tree")
    };
    frame(&mut app, vec![]);
    let mut tree = frame(&mut app, vec![]);
    let initial = Arc::new(StdMutex::new(tree.clone()));
    let (send, receive) = mpsc::sync_channel(64);
    let dropped = Arc::new(AtomicUsize::new(0));
    let mut adapter = accesskit_unix::Adapter::new(
        Initial(initial.clone()),
        Actions {
            send,
            dropped: dropped.clone(),
        },
        Deactivate,
    );
    adapter.update_window_focus_state(true);
    let mut callback = crate::engine::audio::OutputCallback::new(rt, 2);
    let deadline = Instant::now() + Duration::from_secs(70);
    let mut actions = Vec::new();
    let mut frames = 0u64;
    while !directory.join("done").exists() {
        assert!(
            Instant::now() < deadline,
            "private AT-SPI consumer timed out"
        );
        assert_eq!(
            dropped.load(Ordering::Relaxed),
            0,
            "fixture action queue overflow"
        );
        let mut events = Vec::new();
        for request in receive.try_iter().take(64) {
            let label = tree
                .nodes
                .iter()
                .find(|(id, _)| *id == request.target)
                .and_then(|(_, node)| node.label())
                .unwrap_or("unlabelled")
                .to_owned();
            actions.push(serde_json::json!({"action":format!("{:?}",request.action),"label":label,"data":format!("{:?}",request.data)}));
            events.push(egui::Event::AccessKitActionRequest(request));
        }
        assert!(actions.len() <= 184, "unbounded fixture action stream");
        tree = frame(&mut app, events);
        *initial.lock().unwrap() = tree.clone();
        adapter.update_if_active(|| tree.clone());
        callback.render(&mut [0.0_f32; 128]);
        let rt = callback.renderer_mut_for_test();
        rt.publish_for_test();
        frames += 1;
        let focus = tree
            .nodes
            .iter()
            .find(|(id, _)| *id == tree.focus)
            .and_then(|(_, node)| node.label());
        let held_pad_voices = rt
            .sampler_poly
            .voices
            .iter()
            .filter(|voice| {
                voice.input == Some(crate::engine::dsp::InputKey::Pad(0))
                    && matches!(voice.env.stage, 1..=3)
            })
            .count();
        let saved_project = directory.join("Untitled.omat");
        let saved_notes = if saved_project.exists() {
            let bundle = crate::project_file::load::<serde_json::Value>(
                &saved_project,
                &crate::project_file::Limits::default(),
                &std::sync::atomic::AtomicBool::new(false),
            )
            .expect("native Save produced a valid project");
            Some(
                bundle.state["engine"]["tracks"][0]["clips"][0]["notes"]
                    .as_array()
                    .unwrap()
                    .len(),
            )
        } else {
            None
        };
        let undo = app.engine.undo.view();
        let evidence = serde_json::json!({"pid":std::process::id(),"frames":frames,"actions":actions,
            "pitch":rt.decks[0].pitch,"playing":rt.decks[0].playing,"loaded":rt.decks[0].audio.is_some(),
            "grid":rt.decks[0].grid,"grid_editor":app.grid_editor.as_ref().map(grid_editor::Editor::evidence),
            "hotcue_1":rt.decks[0].hotcues[0].set,"cue_slots":rt.decks[0].hotcues.iter().map(|cue|cue.set).collect::<Vec<_>>(),"cue_editor_open":app.cue_editor.editor.is_some(),"pad_held":app.pad_held[0],"focus":focus,
            "sampler_editor":app.sampler_editor.evidence(),
            "analysis":app.library_analysis.evidence(),
            "named_crates":app.crate_evidence(),
            "session_history":app.session_history_evidence(),
            "sampler_banks":rt.sampler_banks.iter().map(|b|serde_json::json!({"id":b.id,"name":b.name(),"gain":b.data.settings.slots[0].controls.gain,"frames":b.data.audio[0].as_ref().map(|s|s.frames())})).collect::<Vec<_>>(),
            "sampler_fixture_row":app.library_view.indices.iter().position(|&i|app.library[i].source==sampler_source).map(|i|i+1),
            "sampler_instrument":rt.sampler_inst.label(),"held_pad_voices":held_pad_voices,
            "pad_sample_active":rt.pad_voices[0].is_some(),
            "notes":rt.tracks[0].clips[0].notes.len(),"undo_cursor":undo.cursor,"undo_epoch":undo.epoch,
            "compose_armed":rt.compose_target.is_some(),"saved_notes":saved_notes,
            "all_pad_inputs_clear":app.pad_inputs.iter().all(|v| *v==0),
            "project_file":saved_project.display().to_string(),
            "help_open":app.keys_open,"help_lesson":app.help.evidence(),
            "performance":app.engine.cmd.performance().status(),
            "lesson_notes":rt.tracks[0].clips[2].notes.len(),
            "preferences_open":app.settings.open,"preferences_busy":app.settings.busy(),
            "preferences_scale":app.settings.profile().appearance.scale,
            "preferences_draft_scale":app.settings.draft.current().unwrap().appearance.scale,
            "preferences_saved":app.settings.revision.is_some(),"preferences_message":app.settings.message,
            "preferences_follow_theme":app.settings.profile().appearance.follow_theme});
        std::fs::write(
            directory.join("state.tmp"),
            serde_json::to_vec(&evidence).unwrap(),
        )
        .unwrap();
        std::fs::rename(directory.join("state.tmp"), directory.join("state.json")).unwrap();
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        actions.len() >= 6,
        "native consumer did not exercise the bridge"
    );
}

#[test]
#[ignore = "run scripts/check-accessibility.py --support on private D-Bus buses"]
fn private_support_bridge_child() {
    assert_eq!(std::env::var("OMATAINER_ATSPI_PRIVATE").as_deref(),Ok("1"));
    let directory=PathBuf::from(std::env::var_os("OMATAINER_ATSPI_TEST_DIR").unwrap());
    assert!(directory.join("private-harness").is_file());
    std::env::set_current_dir(&directory).unwrap();
    let mut engine=Engine::start_safe().unwrap();
    let session=crate::support::worker::Session::start(&directory.join("support"),true).unwrap();
    engine.cmd.attach_support(session.port.clone());
    let mut app=App::with_loader(engine,Theme::default(),None);
    let restarting=Arc::new(std::sync::atomic::AtomicBool::new(false));
    app.initialize_support(Some(session.client()),directory.join("support"),restarting.clone());
    let ctx=egui::Context::default();ctx.enable_accesskit();let mut time=0.0;
    let mut frame=|app:&mut App,events| {
        time+=0.02;
        ctx.run(egui::RawInput{screen_rect:Some(Rect::from_min_size(Pos2::ZERO,Vec2::new(1900.0,1800.0))),time:Some(time),events,focused:true,..Default::default()},|ctx|app.update_frame(ctx))
    };
    frame(&mut app,vec![]);
    let mut tree=frame(&mut app,vec![]).platform_output.accesskit_update.unwrap();
    let initial=Arc::new(StdMutex::new(tree.clone()));let (send,receive)=mpsc::sync_channel(64);let dropped=Arc::new(AtomicUsize::new(0));
    let mut adapter=accesskit_unix::Adapter::new(Initial(initial.clone()),Actions{send,dropped:dropped.clone()},Deactivate);
    adapter.update_window_focus_state(true);
    let deadline=Instant::now()+Duration::from_secs(65);let mut frames=0;let mut actions=0;let mut closes=0;
    while !directory.join("done").exists() {
        assert!(Instant::now()<deadline,"private support consumer timed out");
        let events=receive.try_iter().take(64).map(|action|{actions+=1;egui::Event::AccessKitActionRequest(action)}).collect();
        let output=frame(&mut app,events);tree=output.platform_output.accesskit_update.unwrap();
        closes+=output.viewport_output[&egui::ViewportId::ROOT].commands.iter().filter(|cmd|matches!(cmd,egui::ViewportCommand::Close)).count();
        *initial.lock().unwrap()=tree.clone();adapter.update_if_active(||tree.clone());frames+=1;
        let destination=directory.join("support-report.omasupport.json");
        let exported=destination.exists().then(||crate::support::storage::reopen(&destination,&std::sync::atomic::AtomicBool::new(false)).unwrap().safe_mode);
        let engine_controls_disabled=["Deck A: Pitch","Deck A: Platter play or pause","Sampler: Sample pad 1"].iter().all(|name|tree.nodes.iter().any(|(_,node)|node.label()==Some(*name)&&node.is_disabled()));
        let value=serde_json::json!({"frames":frames,"actions":actions,"safe_mode":app.engine.safe_mode(),"callbacks":app.engine.cmd.audio_metrics().callbacks,"support_open":app.support.open,"command_accepted":app.engine.cmd.stats().accepted,"command_rejected":app.engine.cmd.stats().rejected,"engine_controls_disabled":engine_controls_disabled,"transport_stopped":!app.snap.playing&&app.snap.decks.iter().all(|deck|!deck.playing),"message":app.support.message_for_native_test(),"exported_safe_report":exported,"restarting":restarting.load(Ordering::Acquire),"closes":closes});
        std::fs::write(directory.join("state.tmp"),serde_json::to_vec(&value).unwrap()).unwrap();std::fs::rename(directory.join("state.tmp"),directory.join("state.json")).unwrap();
        std::thread::sleep(Duration::from_millis(12));
    }
    assert_eq!(dropped.load(Ordering::Acquire),0);assert_eq!(app.engine.cmd.audio_metrics().callbacks,0);
    assert!(app.engine.output_info().is_none());assert!(!app.engine.midi.connections_available());
    assert!(actions>=8);assert!(closes>0);assert!(restarting.load(Ordering::Acquire));
    assert!(session.finish(crate::support::Exit::Clean,Duration::from_secs(2)));
}
