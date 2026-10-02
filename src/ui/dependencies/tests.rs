use super::*;
use crate::ui::piano_roll::tests::Gui;
use egui::accesskit::{Action, ActionData};

struct Files(PathBuf);
impl Files {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("omat-dependencies-ui-{}", crate::sampler_bank::BankId::new().unwrap()));
        std::fs::create_dir(&path).unwrap(); Self(path)
    }
    fn audio(&self, name: &str) -> PathBuf {
        let path = self.0.join(name);
        std::fs::write(&path, include_bytes!("../../../tests/fixtures/audio/tone-tags.wav")).unwrap(); path
    }
}
impl Drop for Files { fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); } }
fn settle(gui: &mut Gui) {
    let deadline = Instant::now() + std::time::Duration::from_secs(20);
    loop {
        gui.frame(vec![]);
        if !gui.app.dependencies.busy() && !gui.app.project_pending_for_test() { break; }
        assert!(Instant::now() < deadline, "{:?}", gui.app.dependencies.error);
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    gui.frame(vec![]);
}
fn edit(gui: &mut Gui, label: &str, text: &str) {
    gui.action(label, Action::Focus, None);
    gui.key(Key::A, egui::Modifiers { ctrl: true, command: true, ..Default::default() });
    gui.frame(vec![egui::Event::Text(text.into())]);
}
fn with_missing_audio(files: &Files) -> (Gui, PathBuf) {
    let mut gui = Gui::new();
    let original = files.audio("original.wav");
    let sample = crate::engine::decode::decode_sampler_file(&original, std::fs::File::open(&original).unwrap(), 1024 * 1024, || false).unwrap().sample;
    gui.rt.decks[0].audio = Some(Arc::new(sample));
    gui.rt.tracks[2].poly.offline = Some(Arc::new(crate::engine::fx::OfflineDevice::new("org.example.missing-synth".into(), Some(crate::engine::fx::DeviceState { schema: 7, data: vec![0, 255, 42] })).unwrap()));
    gui.rt.publish_for_test(); gui.frame(vec![]);
    let moved = files.0.join("移動-renamed"); std::fs::rename(&original, &moved).unwrap();
    (gui, moved)
}
fn inspect(gui: &mut Gui) {
    gui.click_text("Project"); gui.click_text("Project dependencies…"); gui.frame(vec![]);
    gui.click("Check dependencies"); settle(gui);
    assert!(gui.app.dependencies.error.is_none(), "{:?}", gui.app.dependencies.error);
}
fn choose(gui: &mut Gui, moved: &PathBuf) {
    edit(gui, "Project dependency search folders", moved.parent().unwrap().to_str().unwrap());
    gui.click("Search moved sources"); settle(gui);
    let report = gui.app.dependencies.review.as_ref().unwrap();
    let index = report.inventory.assets.iter().position(|asset| matches!(asset.availability, data::Availability::Unresolved(_))).unwrap();
    gui.action("Project asset", Action::SetValue, Some(ActionData::NumericValue((index + 1) as f64)));
    let search = gui.app.dependencies.search.as_ref().unwrap(); assert!(search.complete, "{:?}", search.warnings);
    assert_eq!(search.matches[index].len(), 1);
    gui.click(&format!("Use replacement 1: {}", moved.display()));
    assert_eq!(gui.app.dependencies.choices[index], Some(0));
}

#[test]
fn actual_menu_review_apply_and_save_reopen_preserve_pcm_notes_and_offline_instrument() {
    let files = Files::new(); let (mut gui, moved) = with_missing_audio(&files);
    let sample = gui.rt.decks[0].audio.clone().unwrap(); let notes = gui.rt.tracks[2].clips[7].notes.clone();
    inspect(&mut gui);
    let devices = &gui.app.dependencies.review.as_ref().unwrap().inventory.devices;
    assert!(devices.iter().any(|device| device.identifier == "org.example.missing-synth" && device.state_schema == Some(7) && device.state_bytes == 3));
    choose(&mut gui, &moved); gui.click("Apply reviewed relinks"); settle(&mut gui);
    assert!(gui.app.dependencies.error.is_none(), "{:?}", gui.app.dependencies.error);
    assert_eq!(gui.app.dependencies.origins.len(), 1);
    assert_eq!(gui.app.dependencies.origins[0].source, LibSource::File(moved.clone()));
    assert!(Arc::ptr_eq(&sample, gui.rt.decks[0].audio.as_ref().unwrap()));
    assert_eq!(gui.rt.tracks[2].clips[7].notes, notes);
    gui.click("Check dependencies"); settle(&mut gui);
    assert!(gui.app.dependencies.review.as_ref().unwrap().inventory.assets.iter().any(|asset| asset.source == Some(LibSource::File(moved.clone())) && matches!(asset.availability, data::Availability::Verified)));
    gui.click("Close dependency report");
    let path = files.0.join("relinked.omat");
    gui.click_text("Project"); gui.click_text("Save project as…"); gui.path(&path); gui.click_text("Save"); settle(&mut gui);
    assert_eq!(gui.app.project_result_for_test().0, Some(path.clone()));
    let mut reopened = Gui::new(); reopened.click_text("Project"); reopened.click_text("Open project…"); reopened.path(&path); reopened.click_text("Open"); settle(&mut reopened);
    assert_eq!(reopened.app.dependencies.origins, gui.app.dependencies.origins);
    assert_eq!(reopened.rt.decks[0].audio.as_ref().unwrap().data, sample.data);
    assert_eq!(reopened.rt.tracks[2].clips[7].notes, notes);
    assert_eq!(reopened.rt.tracks[2].poly.offline, gui.rt.tracks[2].poly.offline);
    inspect(&mut reopened);
    assert!(reopened.app.dependencies.review.as_ref().unwrap().inventory.assets.iter().any(|asset| asset.source == Some(LibSource::File(moved.clone())) && matches!(asset.availability, data::Availability::Verified)));
    let mut view = reopened.app.project_view(); view.media_origins[0].source = LibSource::File("relative.wav".into());
    assert!(view.validate().is_err());
}

#[test]
fn changed_files_stale_projects_and_explicit_discard_preserve_entire_source_book() {
    let files = Files::new(); let (mut gui, moved) = with_missing_audio(&files);
    inspect(&mut gui); choose(&mut gui, &moved);
    let notes = gui.rt.tracks[2].clips[7].notes.clone();
    std::fs::write(&moved, b"not the reviewed file").unwrap();
    gui.click("Apply reviewed relinks"); settle(&mut gui);
    assert!(gui.app.dependencies.error.as_ref().unwrap().contains("changed"));
    assert!(gui.app.dependencies.origins.is_empty()); assert_eq!(gui.rt.tracks[2].clips[7].notes, notes);
    assert!(gui.app.dependencies.choices.iter().any(Option::is_some));
    gui.click("Close dependency report"); assert!(gui.app.dependencies.confirm_discard);
    gui.click("Keep source review"); assert!(gui.app.dependencies.open);
    gui.native_close = true; let output = gui.frame(vec![]);
    assert!(output.viewport_output[&egui::ViewportId::ROOT].commands.iter().any(|command| matches!(command, egui::ViewportCommand::CancelClose)));
    gui.click("Close dependency report"); gui.click("Discard source review"); settle(&mut gui);
    assert!(!gui.app.dependencies.open && gui.app.dependencies.origins.is_empty());
    inspect(&mut gui);
    gui.app.engine.send(Command::SetBpm(130.0)).unwrap(); gui.frame(vec![]);
    let review = gui.app.dependencies.review.clone().unwrap();
    gui.app.dependencies.start(&gui.app.engine, Kind::Search { review, roots: vec![files.0.clone()] }); settle(&mut gui);
    assert!(gui.app.dependencies.error.as_ref().unwrap().contains("changed"));
    gui.render = false; gui.click("Check dependencies"); assert!(gui.app.dependencies.busy());
    gui.click("Cancel dependency operation"); gui.render = true; settle(&mut gui);
    assert!(gui.app.dependencies.error.as_ref().unwrap().contains("cancelled"));
    assert!(gui.app.dependencies.origins.is_empty());
}
