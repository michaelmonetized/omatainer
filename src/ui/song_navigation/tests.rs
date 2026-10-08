use super::*;
use crate::ui::library_annotations::tests::{Files, Gui};
use egui::accesskit::{Action as AccessAction, ActionData};
fn set(g: &mut Gui, label: &str, value: f64) {
    g.action(
        label,
        AccessAction::SetValue,
        Some(ActionData::NumericValue(value)),
    );
}
fn wait(g: &mut Gui) {
    g.wait(|g| !g.app.song_navigation.busy());
    g.frame(vec![]);
    g.frame(vec![]);
}
#[test]
fn native_sections_edit_save_trigger_loop_and_keyboard_time_entry_use_real_handlers() {
    let files = Files::new();
    let mut g = Gui::new(&files);
    g.app.library_annotations.open = false;
    g.click("Arrangement timeline");
    g.frame(vec![]);
    g.click("Song sections and loop");
    g.text("Section name", "Intro");
    set(&mut g, "Section beat", 0.0);
    g.click("Add section");
    g.text("Section name", "Verse");
    set(&mut g, "Section beat", 4.0);
    g.click("Add section");
    g.text("Section name", "Chorus");
    set(&mut g, "Section beat", 8.0);
    g.click("Add section");
    assert!(g.app.song_navigation.dirty);
    assert!(g.rt.navigation.saved.is_none());
    g.click("Save song sections");
    wait(&mut g);
    assert_eq!(
        g.rt.navigation.saved.as_ref().unwrap().model.locators.len(),
        3
    );
    g.click("Jump to Verse");
    assert!((g.app.engine.snapshot().beat - 4.0).abs() < 1e-10);
    g.click("Next section");
    assert!((g.app.engine.snapshot().beat - 8.0).abs() < 1e-10);
    g.click("Previous section");
    assert!((g.app.engine.snapshot().beat - 4.0).abs() < 1e-10);
    g.text("Section name", "Refrain");
    set(&mut g, "Section beat", 12.0);
    g.click("Update section");
    g.click("Save song sections");
    wait(&mut g);
    assert_eq!(
        g.rt.navigation.saved.as_ref().unwrap().model.destination(3),
        Some(12.0)
    );
    set(&mut g, "Song loop start", 4.0);
    set(&mut g, "Song loop end", 12.0);
    g.click("Set loop braces");
    g.click("Save song sections");
    wait(&mut g);
    g.click("Toggle song loop");
    assert!(g.rt.navigation.saved.as_ref().unwrap().looping);
    g.click("Toggle song loop");
    assert!(!g.rt.navigation.saved.as_ref().unwrap().looping);
    g.click("Verse · ID 2 · beat 4");
    g.click("Loop section to next");
    wait(&mut g);
    assert_eq!(
        g.rt.navigation.saved.as_ref().unwrap().model.loop_region,
        Some(Loop {
            start: 4.0,
            end: 12.0
        })
    );
    assert!(g.rt.navigation.saved.as_ref().unwrap().looping);
    assert!((g.app.engine.snapshot().beat - 4.0).abs() < 1e-10);
    set(&mut g, "Jump to seconds", 3600.0);
    g.action("Jump to entered time", AccessAction::Focus, None);
    for pressed in [true, false] {
        g.frame(vec![egui::Event::Key {
            key: egui::Key::Enter,
            physical_key: Some(egui::Key::Enter),
            pressed,
            repeat: false,
            modifiers: Default::default(),
        }]);
    }
    g.frame(vec![]);
    assert!((g.rt.timeline_seconds() - 3600.0).abs() < 1e-10);
    assert!(!g.rt.navigation.saved.as_ref().unwrap().looping);
    g.click("Delete section");
    g.click("Save song sections");
    wait(&mut g);
    assert!(g
        .rt
        .navigation
        .saved
        .as_ref()
        .unwrap()
        .model
        .destination(2)
        .is_none());
    g.rt.apply(Command::Undo);
    assert_eq!(
        g.rt.navigation.saved.as_ref().unwrap().model.destination(2),
        Some(4.0)
    );
    g.rt.apply(Command::Redo);
    assert!(g
        .rt
        .navigation
        .saved
        .as_ref()
        .unwrap()
        .model
        .destination(2)
        .is_none());
}
