use super::*;
use crate::ui::piano_roll::tests::Gui;
use egui::accesskit::{Action, ActionData};

struct Files(PathBuf);
impl Files {
    fn new() -> Self {
        let id = crate::engine::midi_edit::NoteId::new().words();
        let directory = std::env::temp_dir().join(format!("next-set-{}-{}", std::process::id(), id[1]));
        std::fs::create_dir(&directory).unwrap();
        Self(directory)
    }
    fn path(&self, name: &str) -> PathBuf { self.0.join(name) }
}
impl Drop for Files { fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); } }

fn frame(gui: &mut Gui) {
    gui.rt.process_interleaved(&mut [0.0; 512], 4);
    gui.rt.publish_for_test();
    gui.frame(vec![]);
    std::thread::sleep(Duration::from_millis(1));
}
fn wait(gui: &mut Gui, condition: impl Fn(&Gui) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(15);
    while !condition(gui) {
        frame(gui);
        assert!(Instant::now() < deadline, "next-set native workflow timed out: {:?}", gui.app.project.live.message);
    }
    frame(gui);
}
fn choose(gui: &mut Gui, path: &PathBuf) {
    gui.action("Next live set path", Action::Focus, None);
    gui.key(egui::Key::A, egui::Modifiers::CTRL | egui::Modifiers::COMMAND);
    gui.frame(vec![egui::Event::Text(path.display().to_string())]);
    gui.click("Preload next set");
}

#[test]
fn native_preload_separate_cue_cancel_review_transition_and_saved_reopen() {
    let files = Files::new();
    let path = files.path("incoming.omat");
    let mut gui = Gui::new();
    gui.render = false;
    let saved_master = gui.rt.master;
    gui.app.begin_project_save(SaveKind::As, None, path.clone(), false);
    wait(&mut gui, |gui| !gui.app.project.busy() && gui.app.project.current_path.as_ref() == Some(&path));
    let original_file = std::fs::read(&path).unwrap();
    gui.app.send(crate::engine::Command::Master(0.31));
    gui.app.send(crate::engine::Command::DeckPlay { deck: 0 });
    frame(&mut gui);
    gui.click("Project");
    gui.click("Next live set…");
    choose(&mut gui, &path);
    wait(&mut gui, |gui| gui.app.project.live.control.as_ref().is_some_and(|control| control.ready()));
    assert!(gui.node("Transition to next set").1.is_disabled());
    assert!(gui.rt.decks[0].playing);
    let position = gui.rt.decks[0].pos;
    gui.click("Cue next set on outputs 3/4");
    for _ in 0..5 { frame(&mut gui); }
    assert!(gui.rt.decks[0].pos > position);
    assert!(gui.app.project.live.control.as_ref().unwrap().preview.load(Ordering::Acquire));
    assert_eq!(gui.rt.master, 0.31);
    gui.click("Cancel next set");
    wait(&mut gui, |gui| !gui.app.project.live.busy);
    assert_eq!(gui.rt.master, 0.31);
    assert!(gui.rt.decks[0].playing);
    choose(&mut gui, &path);
    wait(&mut gui, |gui| gui.app.project.live.control.as_ref().is_some_and(|control| control.ready()));
    gui.action("Live-set fade seconds", Action::SetValue, Some(ActionData::NumericValue(0.01)));
    assert_eq!(gui.app.project.live.fade, 0.01);
    gui.click("Discard current unsaved edits at transition");
    assert!(!gui.node("Transition to next set").1.is_disabled());
    gui.click("Transition to next set");
    wait(&mut gui, |gui| !gui.app.project.live.busy && gui.app.project.awaiting_snapshot.is_none());
    assert_eq!(gui.rt.master, saved_master);
    assert!(gui.rt.decks[0].playing);
    assert_eq!(gui.app.project.current_path.as_ref(), Some(&path));
    assert_eq!(std::fs::read(&path).unwrap(), original_file);
    let recalled = files.path("recalled.omat");
    gui.app.begin_project_save(SaveKind::As, None, recalled.clone(), false);
    wait(&mut gui, |gui| !gui.app.project.busy() && gui.app.project.current_path.as_ref() == Some(&recalled));
    gui.app.request_project_action(super::super::Action::Open(recalled.clone()));
    wait(&mut gui, |gui| !gui.app.project.busy() && gui.app.project.awaiting_snapshot.is_none());
    assert_eq!(gui.rt.master, saved_master);
    assert!(!gui.rt.playing);
    assert!(!gui.rt.decks[0].playing);
    assert!(gui.rt.decks[0].audio.is_some());
}

#[test]
fn malformed_and_oversized_preloads_preserve_the_playing_show() {
    let files = Files::new();
    let malformed = files.path("broken.omat");
    std::fs::write(&malformed, b"not a project").unwrap();
    let oversized = files.path("oversized.omat");
    let mut header = Vec::from(crate::project_file::MAGIC);
    header.extend_from_slice(&crate::project_file::FORMAT_VERSION.to_le_bytes());
    header.extend_from_slice(&0u64.to_le_bytes());
    header.extend_from_slice(&(257 * crate::background::MIB).to_le_bytes());
    std::fs::write(&oversized, header).unwrap();
    let mut gui = Gui::new();
    gui.render = false;
    gui.app.send(crate::engine::Command::DeckPlay { deck: 0 });
    frame(&mut gui);
    let namespace = gui.rt.session.namespace;
    gui.click("Project");
    gui.click("Next live set…");
    for path in [malformed, oversized] {
        choose(&mut gui, &path);
        wait(&mut gui, |gui| !gui.app.project.live.busy);
        assert!(gui.rt.decks[0].playing);
        assert_eq!(gui.rt.session.namespace, namespace);
        assert!(gui.app.project.live.message.as_deref().unwrap().contains("not applied"));
        assert!(!gui.app.engine.project.live_sets().busy());
    }
}

#[test]
fn unavailable_processors_and_missing_embedded_sampler_audio_refuse_preflight() {
    let files = Files::new();
    let source = files.path("complete.omat");
    let mut gui = Gui::new();
    gui.render = false;
    gui.app.begin_project_save(SaveKind::As, None, source.clone(), false);
    wait(&mut gui, |gui| !gui.app.project.busy() && gui.app.project.current_path.as_ref() == Some(&source));
    let mut unavailable: crate::project_file::Bundle<Document> = crate::project_file::load(&source, &Default::default(), &AtomicBool::new(false)).unwrap();
    unavailable.state.engine.tracks[0].synth.offline = Some(Arc::new(crate::engine::fx::OfflineDevice::new("test.unavailable.instrument".into(), None).unwrap()));
    let offline = files.path("unavailable.omat");
    crate::project_file::save(&offline, &unavailable, Overwrite::Never, &Default::default(), &AtomicBool::new(false)).unwrap();
    let mut missing: crate::project_file::Bundle<Document> = crate::project_file::load(&source, &Default::default(), &AtomicBool::new(false)).unwrap();
    let bank = missing.state.engine.banks.iter_mut().find(|bank| bank.settings.as_ref().is_some_and(|settings|
        settings.slots.iter().zip(bank.media).any(|(slot, media)| slot.source.is_some() && media.is_some()))).unwrap();
    let slot = bank.settings.as_ref().unwrap().slots.iter().zip(bank.media).position(|(slot, media)| slot.source.is_some() && media.is_some()).unwrap();
    bank.media[slot] = None;
    let absent = files.path("missing-sampler.omat");
    crate::project_file::save(&absent, &missing, Overwrite::Never, &Default::default(), &AtomicBool::new(false)).unwrap();
    gui.app.send(crate::engine::Command::DeckPlay { deck: 0 });
    frame(&mut gui);
    let namespace = gui.rt.session.namespace;
    gui.click("Project");
    gui.click("Next live set…");
    for (path, reason) in [(offline, "unavailable instrument"), (absent, "missing sampler audio")] {
        choose(&mut gui, &path);
        wait(&mut gui, |gui| !gui.app.project.live.busy);
        assert_eq!(gui.rt.session.namespace, namespace);
        assert!(gui.rt.decks[0].playing);
        assert!(gui.app.project.live.message.as_deref().unwrap().contains(reason));
        assert!(!gui.app.engine.project.live_sets().busy());
    }
}

#[test]
fn routed_prints_preload_with_muted_source_devices_but_pre_mute_sends_still_refuse() {
    use crate::engine::audio::routing::model::*;
    let files = Files::new();
    let source = files.path("source.omat");
    let mut gui = Gui::new();
    gui.render = false;
    gui.app.begin_project_save(SaveKind::As, None, source.clone(), false);
    wait(&mut gui, |gui| !gui.app.project.busy() && gui.app.project.current_path.as_ref() == Some(&source));
    let mut bundle: crate::project_file::Bundle<Document> = crate::project_file::load(&source, &Default::default(), &AtomicBool::new(false)).unwrap();
    let track = &mut bundle.state.engine.tracks[0];
    track.mute = true;
    track.synth.offline = Some(Arc::new(crate::engine::fx::OfflineDevice::new("retained.unavailable.source".into(), None).unwrap()));
    bundle.state.engine.routing = Some(Arc::new(Model::default()));
    let printed = files.path("printed.omat");
    crate::project_file::save(&printed, &bundle, Overwrite::Never, &Default::default(), &AtomicBool::new(false)).unwrap();
    let track_id = bundle.state.engine.session.as_ref().unwrap().tracks[0].id;
    Arc::make_mut(bundle.state.engine.routing.as_mut().unwrap()).connections.push(Connection {
        source: Source { group: Group::Track(track_id), tap: Tap::PreFx }, destination: Group::Main,
        map: vec![ChannelMap { source: 0, destination: 0, gain: 1.0 }],
    });
    let sent = files.path("sent-before-mute.omat");
    crate::project_file::save(&sent, &bundle, Overwrite::Never, &Default::default(), &AtomicBool::new(false)).unwrap();
    gui.app.send(crate::engine::Command::DeckPlay { deck: 0 });
    frame(&mut gui);
    let namespace = gui.rt.session.namespace;
    gui.click("Project");
    gui.click("Next live set…");
    choose(&mut gui, &printed);
    wait(&mut gui, |gui| gui.app.project.live.control.as_ref().is_some_and(|control| control.ready()));
    assert!(gui.app.project.live.control.as_ref().unwrap().cue_available.load(Ordering::Acquire));
    assert!(gui.rt.decks[0].playing);
    gui.click("Cancel next set");
    wait(&mut gui, |gui| !gui.app.project.live.busy);
    choose(&mut gui, &sent);
    wait(&mut gui, |gui| !gui.app.project.live.busy);
    assert!(gui.app.project.live.message.as_deref().unwrap().contains("unavailable instrument"));
    assert_eq!(gui.rt.session.namespace, namespace);
    assert!(gui.rt.decks[0].playing);
}
