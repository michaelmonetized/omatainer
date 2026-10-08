use super::*;
use crate::engine::{
    midi::{
        routing::{Endpoint, Filter, Input, Route, Routing},
        MidiMap, UnmappedNotes,
    },
    retrospective::Config,
    ClipKind,
};
use crate::ui::piano_roll::tests::Gui;
use std::time::Duration;

fn fixture() -> (Box<Gui>, crate::engine::midi::TestInput) {
    let mut gui = Box::new(Gui::new());
    gui.screen = Vec2::new(1600.0, 4000.0);
    gui.app
        .engine
        .send(Command::Select { track: 2, scene: 7 })
        .unwrap();
    gui.frame(vec![]);
    gui.app
        .engine
        .cmd
        .retrospective()
        .configure(
            Config {
                enabled: true,
                seconds: 30,
                events: 1024,
            },
            Instant::now(),
        )
        .unwrap();
    let routing = Routing {
        enabled: true,
        routes: vec![Route {
            track: 2,
            inputs: vec![Input {
                port: Endpoint {
                    name: "Keyboard A".into(),
                    id: Some("100:0".into()),
                },
                channels: u16::MAX,
            }],
            output: None,
            output_channel: None,
            monitor: true,
            thru: false,
            filter: Filter::default(),
        }],
    };
    crate::engine::midi::routing::install_for_test(&mut gui.app.engine, routing);
    let input = gui.app.engine.midi.open_for_test(
        &gui.app.engine.cmd,
        17,
        MidiMap {
            name: "Recent MIDI fixture".into(),
            matchers: vec!["Keyboard A".into()],
            bindings: vec![],
            unmapped_notes: UnmappedNotes::Ignore,
        },
        "Keyboard A",
        "100:0",
    );
    assert!(!gui.app.snap.recording);
    (gui, input)
}
fn perform(input: &mut crate::engine::midi::TestInput) {
    for bytes in [
        &[0x91, 60, 100][..],
        &[0x92, 60, 88][..],
        &[0xe1, 12, 70][..],
        &[0xd2, 90][..],
        &[0xb2, 74, 83][..],
        &[0xb1, 64, 127][..],
    ] {
        input.push(bytes);
        std::thread::sleep(Duration::from_millis(3));
    }
    input.push(&[0x81, 60, 29]);
    input.push(&[0x82, 60, 19]);
    input.push(&[0xb1, 64, 0]);
}
fn wait(gui: &mut Gui, ready: impl Fn(&Gui) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        gui.frame(vec![]);
        if ready(gui) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "Recent MIDI workflow did not settle: {} / {:?}",
            gui.app.piano_roll.recent.message,
            gui.app.piano_roll.error
        );
        std::thread::sleep(Duration::from_millis(1));
    }
    gui.frame(vec![]);
}
fn preview(gui: &mut Gui) {
    gui.click("Sampler: Capture recent MIDI");
    gui.click("Recent MIDI capture: Preview recent MIDI capture");
    wait(gui, |g| g.app.piano_roll.recent.worker.is_none());
    assert!(
        gui.app.piano_roll.recent.prepared.is_some(),
        "{}",
        gui.app.piano_roll.recent.message
    );
}
#[test]
fn actual_monitored_input_recording_off_capture_range_preview_edit_apply_undo_redo_and_save_reopen()
{
    let (mut gui, mut input) = fixture();
    perform(&mut input);
    gui.frame(vec![]);
    assert!(gui.rt.tracks[2].clips[7].notes.is_empty());
    assert!(!gui.app.snap.recording);
    preview(&mut gui);
    let preview = gui.app.piano_roll.recent.prepared.as_ref().unwrap();
    assert_eq!(preview.content.notes.len(), 2);
    assert_eq!(
        preview
            .content
            .notes
            .iter()
            .map(|n| (n.channel, n.pitch, n.vel, n.release_vel))
            .collect::<Vec<_>>(),
        vec![(1, 60, 100, 29), (2, 60, 88, 19)]
    );
    assert_eq!(
        preview
            .content
            .messages
            .iter()
            .map(|m| (m.bytes, m.length))
            .collect::<Vec<_>>(),
        vec![
            ([0xe1, 12, 70], 3),
            ([0xd2, 90, 0], 2),
            ([0xb2, 74, 83], 3),
            ([0xb1, 64, 127], 3),
            ([0xb1, 64, 0], 3)
        ]
    );
    let source_notes = preview.content.notes.clone();
    let source_messages = preview.content.messages.clone();
    gui.click("Recent MIDI capture: Open captured MIDI draft");
    wait(&mut gui, |g| {
        g.app.piano_roll.recent.transfer.is_none() && !g.app.piano_roll.busy()
    });
    let draft = gui.app.piano_roll.draft.as_ref().unwrap();
    assert_eq!(draft.notes, source_notes);
    assert!(draft.dirty);
    assert!(gui.rt.tracks[2].clips[7].notes.is_empty());
    let note = &draft.notes[0];
    let label = format!(
        "MIDI piano roll: Note 1: {} · {:.6} beats · length {:.6} · velocity {} · {}",
        pitch_name(note.pitch),
        note.start,
        note.len,
        note.vel,
        if note.muted { "muted" } else { "audible" }
    );
    gui.click(&label);
    gui.number("Note velocity", 17.0);
    gui.click("MIDI piano roll: Set selected note values");
    assert_eq!(gui.app.piano_roll.draft.as_ref().unwrap().notes[0].vel, 17);
    assert_eq!(
        gui.app.piano_roll.draft.as_ref().unwrap().notes[1],
        source_notes[1]
    );
    gui.apply();
    let committed = gui.rt.tracks[2].clips[7].notes.clone();
    let lanes = gui.rt.tracks[2].clips[7].lanes.clone().unwrap();
    assert_eq!(committed[0].vel, 17);
    assert_eq!(lanes.messages, source_messages);
    gui.app.engine.send(Command::Undo).unwrap();
    gui.frame(vec![]);
    assert_eq!(gui.rt.tracks[2].clips[7].kind, ClipKind::Empty);
    assert!(gui.rt.tracks[2].clips[7].notes.is_empty());
    gui.app.engine.send(Command::Redo).unwrap();
    gui.frame(vec![]);
    assert_eq!(gui.rt.tracks[2].clips[7].notes, committed);
    assert_eq!(
        gui.rt.tracks[2].clips[7].lanes.as_ref().unwrap().messages,
        source_messages
    );
    let file = std::env::temp_dir().join(format!(
        "omat-recent-midi-{}.omat",
        crate::sampler_bank::BankId::new().unwrap()
    ));
    gui.click("MIDI piano roll: Cancel / close MIDI editor");
    gui.click_text("Project");
    gui.click_text("Save project as…");
    gui.path(&file);
    gui.click_text("Save");
    gui.settle();
    assert_eq!(
        gui.app.project_result_for_test().0.as_deref(),
        Some(file.as_path())
    );
    let mut reopened = Box::new(Gui::new());
    reopened.click_text("Project");
    reopened.click_text("Open project…");
    reopened.path(&file);
    reopened.click_text("Open");
    reopened.settle();
    assert_eq!(reopened.rt.tracks[2].clips[7].notes, committed);
    assert_eq!(
        reopened.rt.tracks[2].clips[7]
            .lanes
            .as_ref()
            .unwrap()
            .messages,
        source_messages
    );
    assert!(reopened
        .app
        .engine
        .cmd
        .retrospective()
        .snapshot(Instant::now())
        .events
        .is_empty());
    std::fs::remove_file(file).unwrap();
    eprintln!("MIDI_RETROSPECTIVE_NATIVE {{\"actual_egui_accesskit\":true,\"normal_monitored_input_handoff\":true,\"recording_off\":true,\"editable_capture\":true,\"release_and_expression_retained\":true,\"one_undo_redo\":true,\"native_save_reopen\":true,\"private_history_not_saved\":true,\"physical_devices_opened\":false}}");
}
#[test]
fn actual_nonempty_target_refusal_clear_and_disconnect_preserve_existing_music_and_discard_private_review(
) {
    let (mut gui, mut input) = fixture();
    let note = crate::engine::MidiNote {
        id: crate::engine::midi_edit::NoteId::new(),
        pitch: 65,
        start: 1.0,
        len: 0.5,
        vel: 91,
        release_vel: 0,
        channel: 0,
        muted: false,
        source_timing: None,
        variation: None,
    };
    gui.rt.tracks[2].clips[7].kind = ClipKind::Midi;
    gui.rt.tracks[2].clips[7].name = "Keep me".into();
    gui.rt.tracks[2].clips[7].bars = 1.0;
    gui.rt.tracks[2].clips[7].notes = vec![note];
    gui.rt.publish_for_test();
    gui.frame(vec![]);
    let original = gui.rt.tracks[2].clips[7].notes.clone();
    perform(&mut input);
    gui.frame(vec![]);
    preview(&mut gui);
    let candidate = gui
        .app
        .piano_roll
        .recent
        .prepared
        .as_ref()
        .unwrap()
        .content
        .notes
        .clone();
    gui.click("Recent MIDI capture: Open captured MIDI draft");
    wait(&mut gui, |g| {
        g.app.piano_roll.recent.transfer.is_none() && !g.app.piano_roll.busy()
    });
    assert!(gui
        .app
        .piano_roll
        .recent
        .message
        .contains("empty Session slot"));
    assert_eq!(gui.rt.tracks[2].clips[7].notes, original);
    assert_eq!(gui.app.piano_roll.draft.as_ref().unwrap().notes, original);
    assert_eq!(
        gui.app
            .piano_roll
            .recent
            .prepared
            .as_ref()
            .unwrap()
            .content
            .notes,
        candidate
    );
    gui.click("Recent MIDI capture: Clear recent MIDI history");
    assert!(gui
        .app
        .engine
        .cmd
        .retrospective()
        .snapshot(Instant::now())
        .events
        .is_empty());
    assert!(gui.app.piano_roll.recent.prepared.is_none());
    assert_eq!(gui.rt.tracks[2].clips[7].notes, original);
    perform(&mut input);
    gui.click("Recent MIDI capture: Refresh recent MIDI");
    gui.click("Recent MIDI capture: Preview recent MIDI capture");
    wait(&mut gui, |g| g.app.piano_roll.recent.worker.is_none());
    assert!(gui.app.piano_roll.recent.prepared.is_some());
    drop(input);
    gui.frame(vec![]);
    assert!(gui.app.piano_roll.recent.prepared.is_none());
    assert!(gui.app.piano_roll.recent.snapshot.is_none());
    assert_eq!(gui.rt.tracks[2].clips[7].notes, original);
    eprintln!("MIDI_RETROSPECTIVE_REFUSAL {{\"actual_egui_accesskit\":true,\"nonempty_target_preserved\":true,\"private_clear\":true,\"disconnect_invalidates_review\":true,\"physical_devices_opened\":false}}");
}
