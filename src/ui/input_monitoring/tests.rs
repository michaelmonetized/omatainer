use super::*;
use crate::ui::piano_roll::tests::Gui;

#[test]
fn native_input_modes_arm_and_cue_publish_and_mode_changes_support_undo() {
    let mut gui = Gui::new();
    gui.click("Edit session");
    let selected = gui.rt.selected_track;
    let master = gui.rt.master;
    let revision = gui.app.engine.project.revision();
    gui.click("Input monitoring: In");
    assert_eq!(gui.rt.tracks[selected].input_monitor, Some(Mode::In));
    assert!(gui.app.engine.project.revision() > revision);
    gui.app.send(Command::Undo);
    gui.frame(vec![]);
    assert_eq!(gui.rt.tracks[selected].input_monitor, None);
    gui.app.send(Command::Redo);
    gui.frame(vec![]);
    assert_eq!(gui.rt.tracks[selected].input_monitor, Some(Mode::In));
    gui.click("Input monitoring: Auto");
    gui.click("Arm input track");
    gui.click("Cue input track");
    for _ in 0..4 {
        gui.frame(vec![]);
    }
    let s = gui.app.engine.snapshot();
    assert_eq!(s.tracks[selected].input_monitor, Some(Mode::Auto));
    assert!(s.tracks[selected].armed && s.tracks[selected].pfl && s.tracks[selected].input_enabled);
    gui.click("Input monitoring: Off");
    for _ in 0..4 {
        gui.frame(vec![]);
    }
    let s = gui.app.engine.snapshot();
    assert_eq!(s.tracks[selected].input_monitor, Some(Mode::Off));
    assert!(!s.tracks[selected].input_enabled);
    assert_eq!(s.master, master);
    assert!(gui
        .rt
        .tracks
        .iter()
        .enumerate()
        .filter(|(slot, _)| *slot != selected)
        .all(|(_, track)| track.input_monitor.is_none() && !track.armed && !track.pfl));
}
