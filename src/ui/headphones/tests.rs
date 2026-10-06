use super::*;
use crate::ui::piano_roll::tests::Gui;

#[test]
fn native_headphone_controls_change_published_monitor_state_and_leave_master_untouched() {
    let mut gui = Gui::new();
    gui.click("Audio routing");
    gui.click("Headphone controls");
    let master = gui.rt.master;
    for label in [
        "Selected deck cues (PFL)",
        "Headphones: Cue A",
        "Headphones: Cue B",
        "Split cue",
    ] {
        gui.click(label);
    }
    for _ in 0..4 {
        gui.frame(vec![]);
    }
    let snapshot = gui.app.engine.snapshot();
    assert_eq!(snapshot.monitor.source, Source::Pfl);
    assert!(snapshot.monitor.split && snapshot.decks[0].pfl && snapshot.decks[1].pfl);
    assert_eq!(snapshot.master, master);
    let node = gui
        .nodes
        .iter()
        .find(|(_, n)| n.label() == Some("Check left headphone at −40 dBFS"))
        .unwrap();
    assert!(node.1.is_disabled());
    assert!(!snapshot.monitor.available);
    let target = gui
        .nodes
        .iter()
        .find(|(_, node)| node.label() == Some("Cue → master blend"))
        .map(|(id, _)| *id)
        .unwrap();
    let revision = gui.app.engine.project.revision();
    gui.frame(vec![egui::Event::AccessKitActionRequest(
        egui::accesskit::ActionRequest {
            target,
            action: egui::accesskit::Action::SetValue,
            data: Some(egui::accesskit::ActionData::NumericValue(0.625)),
        },
    )]);
    gui.frame(vec![]);
    assert_eq!(gui.app.engine.snapshot().monitor.blend, 0.625);
    assert!(gui.app.engine.project.revision() > revision);
    gui.rt.apply(Command::Undo);
    gui.frame(vec![]);
    gui.frame(vec![]);
    assert_eq!(gui.app.engine.snapshot().monitor.blend, 0.0);
    gui.rt.apply(Command::Redo);
    gui.frame(vec![]);
    gui.frame(vec![]);
    assert_eq!(gui.app.engine.snapshot().monitor.blend, 0.625);
    gui.click("Deck A/B mix");
    for _ in 0..3 {
        gui.frame(vec![]);
    }
    assert_eq!(gui.app.engine.snapshot().monitor.source, Source::DeckMix);
}
