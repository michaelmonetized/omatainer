use super::*;
use crate::ui::piano_roll::tests::Gui;
use egui::accesskit::{Action as Accessible, ActionData};
use std::time::Duration;

fn settled(gui: &mut Gui) {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        gui.frame(vec![]);
        if !gui.app.session_editor.busy() {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "{:?}",
            gui.app.session_editor.error
        );
        std::thread::sleep(Duration::from_millis(1));
    }
    for _ in 0..4 {
        gui.frame(vec![]);
    }
    assert!(
        gui.app.session_editor.error.is_none(),
        "{:?}",
        gui.app.session_editor.error
    );
}
fn number(gui: &mut Gui, label: &str, value: f64) {
    gui.action(
        label,
        Accessible::SetValue,
        Some(ActionData::NumericValue(value)),
    );
}
#[test]
fn controls_from_an_old_painted_snapshot_cannot_mutate_a_replacement_project() {
    let mut gui = Gui::new();
    let old = gui.app.snap.session.as_ref().unwrap().namespace;
    let handle = gui.app.engine.project.clone();
    let revision = handle.revision();
    let worker = std::thread::spawn(move || {
        handle.install(
            crate::engine::project::Prepared::empty(48000).unwrap(),
            revision,
            &AtomicBool::new(false),
        )
    });
    let deadline = Instant::now() + Duration::from_secs(15);
    while !worker.is_finished() {
        gui.rt.process(&mut [0.0; 2]);
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(1));
    }
    worker.join().unwrap().unwrap();
    assert_ne!(gui.rt.session.namespace, old);
    assert_eq!(gui.app.snap.session.as_ref().unwrap().namespace, old);
    let gain = gui.rt.tracks[2].gain;
    let focus = (
        gui.rt.selected_track,
        gui.rt.selected_scene,
        gui.rt.compose_target,
    );
    assert!(gui.app.submit(Command::TrackGain {
        track: 2,
        value: 0.12
    }));
    assert!(gui.app.submit(Command::ComposeArm { track: 2, scene: 7 }));
    assert_eq!(
        crate::engine::test_alloc::measure(|| gui.rt.process(&mut [0.0; 128])),
        crate::engine::test_alloc::Counts::default()
    );
    assert_eq!(gui.rt.tracks[2].gain, gain);
    assert_eq!(
        (
            gui.rt.selected_track,
            gui.rt.selected_scene,
            gui.rt.compose_target
        ),
        focus
    );
    assert!(!gui.rt.recording);
}
#[test]
fn native_editor_creates_renames_reorders_colors_duplicates_deletes_and_undoes_track_and_scene() {
    let mut gui = Gui::new();
    gui.click("+ Audio track");
    settled(&mut gui);
    assert_eq!(gui.rt.tracks[8].kind, 4);
    gui.click("+ MIDI track");
    settled(&mut gui);
    assert_eq!(gui.rt.tracks[9].kind, 2);
    number(&mut gui, "Go to track", 10.0);
    gui.click("Edit session");
    gui.frame(vec![]);
    gui.action("Session track name", Accessible::Focus, None);
    gui.key(
        Key::A,
        egui::Modifiers {
            ctrl: true,
            command: true,
            ..Default::default()
        },
    );
    gui.frame(vec![egui::Event::Text("Keyboard synth".into())]);
    gui.click("Rename track");
    settled(&mut gui);
    assert_eq!(gui.rt.tracks[9].name, "Keyboard synth");
    number(&mut gui, "Session position", 1.0);
    gui.click("Reorder track");
    settled(&mut gui);
    assert_eq!(gui.rt.session.track_order[0], 9);
    assert_eq!(gui.rt.selected_track, 9);
    gui.click("Use custom color");
    gui.frame(vec![]);
    number(&mut gui, "Session color red", 17.0);
    number(&mut gui, "Session color green", 91.0);
    number(&mut gui, "Session color blue", 203.0);
    gui.click("Color track");
    settled(&mut gui);
    assert_eq!(gui.rt.session.tracks[9].color, Some([17, 91, 203]));
    gui.click("Duplicate track");
    settled(&mut gui);
    assert_eq!(gui.rt.tracks.len(), 11);
    let original = gui.rt.session.tracks[9].id;
    let duplicate = gui.rt.session.tracks[10].id;
    assert_ne!(original, duplicate);
    gui.click("Delete track");
    settled(&mut gui);
    assert!(!gui.rt.session.tracks[9].active);
    gui.app.history_action(false);
    settled(&mut gui);
    assert!(gui.rt.session.tracks[9].active);
    gui.app.session_editor.open = false;
    gui.frame(vec![]);
    gui.click("+ Scene");
    settled(&mut gui);
    assert_eq!(gui.rt.session.scene_order.len(), 9);
    number(&mut gui, "Go to scene", 9.0);
    gui.click("Edit session");
    gui.click("Scene");
    gui.action("Session scene name", Accessible::Focus, None);
    gui.key(
        Key::A,
        egui::Modifiers {
            ctrl: true,
            command: true,
            ..Default::default()
        },
    );
    gui.frame(vec![egui::Event::Text("Finale".into())]);
    gui.click("Rename scene");
    settled(&mut gui);
    assert_eq!(gui.rt.session.scenes[8].name, "Finale");
    gui.click("Use custom color");
    number(&mut gui, "Session color red", 41.0);
    number(&mut gui, "Session color green", 177.0);
    number(&mut gui, "Session color blue", 83.0);
    gui.click("Color scene");
    settled(&mut gui);
    assert_eq!(gui.rt.session.scenes[8].color, Some([41, 177, 83]));
    number(&mut gui, "Session position", 1.0);
    gui.click("Reorder scene");
    settled(&mut gui);
    assert_eq!(gui.rt.session.scene_order[0], 8);
    gui.click("Duplicate scene");
    settled(&mut gui);
    assert_eq!(gui.rt.session.scene_order.len(), 10);
    gui.click("Delete scene");
    settled(&mut gui);
    assert!(!gui.rt.session.scenes[8].active);
    gui.app.history_action(false);
    settled(&mut gui);
    assert!(gui.rt.session.scenes[8].active);
    if let Some(root) = std::env::var_os("OMAT_SESSION_EVIDENCE") {
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(std::path::Path::new(&root).join("native-editor.json"), serde_json::to_vec_pretty(&serde_json::json!({
            "kind":"actual App, egui, AccessKit actions and renderer; no desktop Orca or physical hardware",
            "operations":["create audio track","create MIDI track","rename track","reorder track","color track","duplicate track","delete track","undo track deletion","create scene","rename scene","color scene","reorder scene","duplicate scene","delete scene","undo scene deletion"],
            "session":gui.rt.session
        })).unwrap()).unwrap();
    }
}
#[test]
fn maximum_set_reaches_final_cell_and_editor_through_accessible_navigation_with_bounded_nodes() {
    let mut gui = Gui::new();
    let handle = gui.app.engine.project.clone();
    let expected = handle.revision();
    let install = std::thread::spawn(move || {
        handle.install(
            crate::engine::project::maximum_for_test(),
            expected,
            &AtomicBool::new(false),
        )
    });
    let deadline = Instant::now() + Duration::from_secs(15);
    while !install.is_finished() {
        gui.frame(vec![]);
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(1));
    }
    install.join().unwrap().unwrap();
    for _ in 0..12 {
        gui.frame(vec![]);
    }
    assert_eq!(gui.rt.tracks.len(), 128);
    assert_eq!(gui.rt.scene_fx.len(), 512);
    // This fixture initially brings the last backing cell to display position one.
    number(&mut gui, "Go to track", 128.0);
    number(&mut gui, "Go to scene", 512.0);
    gui.ctx.style_mut(|style| style.animation_time = 0.0);
    for _ in 0..8 {
        gui.frame(vec![]);
    }
    let track = usize::from(gui.rt.session.track_order[127]);
    let scene = usize::from(gui.rt.session.scene_order[511]);
    assert_eq!(
        (gui.rt.selected_track, gui.rt.selected_scene),
        (track, scene)
    );
    let (_, cell) = gui.node("Clip track 128 scene 512: Last audio cell");
    let bounds = cell.bounds().unwrap();
    assert!(
        bounds.x0 >= 0.0
            && bounds.x1 <= gui.screen.x as f64
            && bounds.y0 >= 0.0
            && bounds.y1 <= gui.screen.y as f64,
        "{bounds:?}"
    );
    let visible = gui
        .nodes
        .iter()
        .filter(|(_, node)| {
            node.label()
                .is_some_and(|label| label.starts_with("Clip track "))
        })
        .count();
    assert!(visible < 256, "{visible} clip nodes for a 65,536-cell set");
    if let Some(root) = std::env::var_os("OMAT_SESSION_EVIDENCE") {
        let mut timings = Vec::with_capacity(32);
        for _ in 0..32 {
            let start = Instant::now();
            gui.frame(vec![]);
            timings.push(start.elapsed().as_secs_f64() * 1000.0);
        }
        timings.sort_by(f64::total_cmp);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(std::path::Path::new(&root).join("native-maximum.json"), serde_json::to_vec_pretty(&serde_json::json!({
            "kind":"actual App, egui, AccessKit navigation and renderer; no desktop Orca or physical hardware",
            "tracks":128,"scenes":512,"total_cells":65536,"visible_clip_nodes":visible,
            "selected_storage_slot":[track,scene],"last_visible_cell_bounds":[bounds.x0,bounds.y0,bounds.x1,bounds.y1],
            "screen":[gui.screen.x,gui.screen.y],"frame_samples":32,
            "frame_ms":{"minimum":timings[0],"median":timings[16],"p95":timings[30],"maximum":timings[31]},
            "timing_scope":"32 headless App updates including snapshot clone, actual egui paint/accessibility and a 64-frame renderer block; observations, no maximum-set hardware deadline claim"
        })).unwrap()).unwrap();
    }
    gui.click("Edit session");
    gui.frame(vec![]);
    gui.node("Rename track");
    gui.node("Duplicate track");
    gui.node("Delete track");
    gui.click("Scene");
    gui.frame(vec![]);
    gui.node("Rename scene");
    gui.node("Duplicate scene");
    gui.node("Delete scene");
}
#[test]
fn native_scene_properties_save_undo_duplicate_launch_and_cancel_use_real_handlers() {
    use crate::engine::{clip_launch::Grid,scene::{Empty,Signature}};
    let mut gui=Gui::new();
    gui.click("Edit session");gui.click("Scene");gui.click("Scene launch properties");gui.click("Use scene tempo");
    number(&mut gui,"Scene tempo",95.0);gui.click("Use scene time signature");number(&mut gui,"Scene meter numerator",7.0);
    gui.click_text("Scene meter denominator 4");gui.click("Scene denominator 8");
    gui.click_text("Empty slots: stop tracks");gui.click("Keep tracks playing for empty slots");
    gui.click_text("Scene launch timing: Global");gui.click("Scene timing 1 bar");
    gui.click("Save scene launch properties");settled(&mut gui);
    let scene=gui.rt.selected_scene;let value=gui.rt.session.scenes[scene].scene;
    assert!((value.bpm().unwrap()-95.0).abs()<0.001);assert_eq!(value.meter,Some(Signature{numerator:7,denominator_power:3}));assert_eq!(value.empty,Empty::Keep);assert_eq!(value.grid,Grid::Bar);
    gui.app.history_action(false);settled(&mut gui);assert!(gui.rt.session.scenes[scene].scene.is_default());gui.app.history_action(true);settled(&mut gui);assert_eq!(gui.rt.session.scenes[scene].scene,value);
    gui.click("Duplicate scene");settled(&mut gui);let copy=gui.rt.session.scenes.len()-1;assert_eq!(gui.rt.session.scenes[copy].scene,value);assert_ne!(gui.rt.session.scenes[copy].id,gui.rt.session.scenes[scene].id);
    gui.app.session_editor.open=false;gui.frame(vec![]);
    gui.action("Scene 8: Toggle playback",Accessible::CustomAction,Some(ActionData::CustomAction(3)));
    gui.frame(vec![]);assert_eq!(gui.rt.scenes.timing.unwrap().signature.numerator,7);
    gui.rt.apply(Command::Play);gui.frame(vec![]);
    gui.action("Scene 8: Toggle playback",Accessible::CustomAction,Some(ActionData::CustomAction(3)));gui.frame(vec![]);assert!(gui.rt.scenes.pending.is_some());
    gui.click("Edit session");gui.click("Cancel queued scene");gui.frame(vec![]);assert!(gui.rt.scenes.pending.is_none());
}
