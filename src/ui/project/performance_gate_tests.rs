//! Fixed local release workloads through the real App, renderer, raw MIDI
//! handoff and Unix IPC handler. No physical device or display server opens.
use super::*;
use crate::engine::{midi::connection_test_support as midi, test_alloc, ClipKind, RtEngine};
use egui::accesskit::{Action, ActionData, ActionRequest, Node, NodeId};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::{fs::OpenOptionsExt, net::UnixStream};
use std::time::Duration;

const UI_FRAMES: usize = 512;
const WARMUP: usize = 16;

struct Gui {
    app: App,
    rt: Box<RtEngine>,
    ctx: egui::Context,
    nodes: Vec<(NodeId, Node)>,
    time: f64,
    frame_ns: Vec<u64>,
}
impl Gui {
    fn new() -> Self {
        let (engine, mut rt) = Engine::headless_for_test(48_000, 256);
        rt.publish_for_test();
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut gui = Self {
            app: App::with_loader(engine, Theme::default(), None),
            rt: Box::new(rt),
            ctx,
            nodes: Vec::new(),
            time: 0.0,
            frame_ns: Vec::new(),
        };
        for _ in 0..WARMUP {
            gui.frame(vec![]);
        }
        gui.frame_ns.clear();
        gui
    }
    fn frame(&mut self, events: Vec<egui::Event>) {
        self.rt.publish_for_test(); // worker synchronization is outside UI timing
        self.time += 1.0 / 60.0;
        let modifiers = events
            .iter()
            .rev()
            .find_map(|event| match event {
                egui::Event::Key { modifiers, .. } => Some(*modifiers),
                _ => None,
            })
            .unwrap_or_default();
        let began = Instant::now();
        let mut out = self.ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1440.0, 1000.0))),
                time: Some(self.time),
                events,
                modifiers,
                ..Default::default()
            },
            |ctx| self.app.update_frame(ctx),
        );
        let primitives = self
            .ctx
            .tessellate(std::mem::take(&mut out.shapes), out.pixels_per_point);
        self.frame_ns.push(began.elapsed().as_nanos() as u64);
        assert!(
            !primitives.is_empty(),
            "complete App must produce render geometry"
        );
        self.nodes = out.platform_output.accesskit_update.unwrap().nodes;
        self.rt.process(&mut [0.0; 128]);
    }
    fn event(&self, label: &str, action: Action, data: Option<ActionData>) -> egui::Event {
        let (target, node) = self
            .nodes
            .iter()
            .find(|(_, node)| node.label() == Some(label))
            .unwrap_or_else(|| {
                panic!(
                    "missing App control {label}; {:?}",
                    self.nodes
                        .iter()
                        .filter_map(|(_, n)| n.label())
                        .collect::<Vec<_>>()
                )
            });
        assert!(
            node.supports_action(action),
            "{label}: unsupported {action:?}"
        );
        egui::Event::AccessKitActionRequest(ActionRequest {
            action,
            target: *target,
            data,
        })
    }
    fn action(&mut self, label: &str, action: Action, data: Option<ActionData>) {
        let event = self.event(label, action, data);
        self.frame(vec![event]);
        self.frame(vec![]);
    }
    fn key(&mut self, key: Key, mut modifiers: egui::Modifiers) {
        modifiers.command |= modifiers.ctrl;
        for pressed in [true, false] {
            self.frame(vec![egui::Event::Key {
                key,
                physical_key: Some(key),
                pressed,
                repeat: false,
                modifiers,
            }]);
        }
    }
    fn menu(&mut self, label: &str) {
        self.action("Project", Action::Click, None);
        self.action(label, Action::Click, None);
    }
    fn settle(&mut self) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while self.app.project.busy() || self.app.project.awaiting_snapshot.is_some() {
            self.frame(vec![]);
            assert!(
                Instant::now() < deadline,
                "project did not settle: {:?}",
                self.app.project.message
            );
            std::thread::sleep(Duration::from_millis(1));
        }
        self.frame(vec![]);
    }
    fn path(&mut self, path: &std::path::Path) {
        self.action("Project file path", Action::Focus, None);
        self.key(Key::A, egui::Modifiers::CTRL);
        self.frame(vec![egui::Event::Text(path.display().to_string())]);
    }
    fn save(&mut self, path: &std::path::Path) {
        self.menu("Save project as…");
        self.path(path);
        self.action("Save", Action::Click, None);
        self.settle();
        assert_eq!(
            self.app.project.current_path.as_deref(),
            Some(path),
            "{:?}",
            self.app.project.message
        );
        assert!(!self.app.project_dirty());
    }
    fn reopen(&mut self, path: &std::path::Path) {
        self.menu("New project");
        self.settle();
        assert!(self
            .rt
            .tracks
            .iter()
            .all(|t| t.clips.iter().all(|c| c.notes.is_empty())));
        self.menu("Open project…");
        self.path(path);
        self.action("Open", Action::Click, None);
        self.settle();
        assert_eq!(self.app.project.current_path.as_deref(), Some(path));
        assert!(!self.app.project_dirty());
    }
}

fn private_directory(label: &str) -> PathBuf {
    use std::os::unix::fs::DirBuilderExt;
    let path = std::env::temp_dir().join(format!(
        "omatainer-performance-{}-{label}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&path)
        .unwrap();
    path
}

fn request(gui: &mut Gui, op: &str) {
    let (mut client, server) = UnixStream::pair().unwrap();
    client
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    let commands = gui.app.engine.cmd.clone();
    let snapshot = gui.app.engine.snap.clone();
    let handler = std::thread::spawn(move || {
        crate::handle_client_with_limits(
            server,
            commands,
            snapshot,
            crate::ipc_transport::Limits {
                requests: 1,
                ..Default::default()
            },
        )
    });
    writeln!(client, "{{\"id\":\"recording-recipe\",\"op\":\"{op}\"}}").unwrap();
    let mut line = String::new();
    BufReader::new(client).read_line(&mut line).unwrap();
    let reply: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(reply["ok"], true, "{reply}");
    assert_eq!(reply["id"], "recording-recipe");
    handler.join().unwrap().unwrap();
    gui.rt.process(&mut []);
}

fn large_crate() -> Value {
    let mut gui = Gui::new();
    gui.app.library = Arc::new(
        (0..50_000)
            .map(|n| LibItem {
                source: LibSource::File(format!("private-fixture/{n}.wav").into()),
                title: format!("Track {n:05}"),
                artist: if n % 2 == 0 {
                    "Even Artist"
                } else {
                    "Odd Artist"
                }
                .into(),
                bpm: Bpm::hint(120.0),
                fingerprint: None,
                key: "C".into(),
                length: Some(180.0),
                last_play: None,
            })
            .collect(),
    );
    gui.frame(vec![]);
    let rebuilds = gui.app.library_view.stats.rebuilds;
    let examined = gui.app.library_view.stats.examined;
    gui.frame_ns.clear();
    let mut max_rows = 0;
    let mut exact_selection = true;
    let mut expected_row = 49_999usize;
    for n in 0..UI_FRAMES {
        let events = if n % 16 == 0 {
            vec![gui.event(
                "Crate selection",
                Action::SetValue,
                Some(ActionData::NumericValue(if n % 32 == 0 {
                    50_000.0
                } else {
                    1.0
                })),
            )]
        } else {
            vec![]
        };
        gui.frame(events);
        if n % 16 == 0 {
            expected_row = if n % 32 == 0 { 49_999 } else { 0 };
        }
        let expected = LibSource::File(format!("private-fixture/{expected_row}.wav").into());
        exact_selection &= gui
            .app
            .selected_library_item()
            .is_some_and(|item| item.source == expected);
        // Publication occurs at the next App frame after the native value action.
        if n % 16 != 0 {
            exact_selection &= gui
                .app
                .published_selection
                .as_ref()
                .is_some_and(|item| item.source == expected);
        }
        max_rows = max_rows.max(gui.app.library_view.stats.rendered);
    }
    let unchanged = gui.app.library_view.stats.rebuilds == rebuilds
        && gui.app.library_view.stats.examined == examined;
    let samples = std::mem::take(&mut gui.frame_ns);
    gui.action("Search crate", Action::Focus, None);
    gui.frame(vec![egui::Event::Text("Even Artist".into())]);
    gui.frame(vec![]);
    let filtered = gui.app.library_view.indices.len();
    json!({"metrics":{"max_visible_rows":max_rows,"filtered_tracks":filtered,
        "rejected_commands":gui.app.engine.cmd.stats().rejected},
        "samples":{"frame_wall_ns":samples},"observations":{},
        "checks":{"steady_filter_cached":unchanged,"bounded_visible_rows":max_rows>0 && max_rows<=64,
            "real_search_applied":filtered==25_000,"selection_matches_published":exact_selection}})
}

fn multi_controller() -> Value {
    let mut gui = Gui::new();
    gui.rt.quant = 0.0;
    gui.rt.selected_track = 1; // first live chord targets a synth, not the drum track
    let control = midi::install(&mut gui.app.engine);
    control.discover(&[
        ("a", "Pioneer DDJ-FLX4"),
        ("b", "Numark NS7"),
        ("c", "Akai APC40 mkII"),
        ("d", "MPD232"),
        ("e", "Generic MIDI Keyboard"),
    ]);
    let inputs: [midi::Input; 5] = ["a", "b", "c", "d", "e"].map(|id| control.connect(id, Ok(())));
    midi::until(|| gui.app.engine.midi.policy_status().unwrap().applied == Some(1));
    let directory = private_directory("ipc");
    let socket = directory.join("control.sock");
    let server = crate::ipc_server::start_at(
        &socket,
        gui.app.engine.cmd.clone(),
        gui.app.engine.snap.clone(),
    )
    .unwrap();
    let mut reader: Option<BufReader<UnixStream>> = None;
    let mut line = String::new();
    let mut ipc_ns = Vec::with_capacity(UI_FRAMES);
    let mut ingress_ns = Vec::with_capacity(UI_FRAMES);
    let before = gui.app.engine.midi.input_stats().dispatched;
    let mut sent = 0;
    let mut held_ownership = true;
    let mut held_sources: Option<(u64, u64)> = None;
    gui.frame_ns.clear();
    for n in 0..UI_FRAMES {
        let note = 60 + ((n / 2) % 12) as u8;
        // The MPD profile reserves channel-10 notes 36..43 for hot cues.
        // This fixed preset uses its unmapped upper pads for live note ownership.
        let pad = 60 + ((n / 2) % 12) as u8;
        let on = n % 2 == 0;
        let keyboard_status = if on { 0x90 } else { 0x80 };
        let pad_status = if on { 0x99 } else { 0x89 };
        let began = Instant::now();
        for (input, message) in [
            (0, [0xB1, 0, 70]),
            (0, [0xB0, 0x21, 65]),
            (1, [0xB1, 0x13, 88]),
            (2, [0xB0, 7, 64]),
            (3, [pad_status, pad, if on { 100 } else { 0 }]),
            (3, [pad_status, pad + 12, if on { 95 } else { 0 }]),
            (4, [keyboard_status, note, if on { 90 } else { 0 }]),
            (4, [keyboard_status, note + 12, if on { 85 } else { 0 }]),
        ] {
            inputs[input].push(&message);
            sent += 1;
        }
        midi::until(|| gui.app.engine.midi.input_stats().dispatched == before + sent);
        ingress_ns.push(began.elapsed().as_nanos() as u64);
        let began = Instant::now();
        if n % 32 == 0 {
            reader.take();
            let client = UnixStream::connect(&socket).unwrap();
            client
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            client
                .set_write_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            reader = Some(BufReader::new(client));
        }
        let connection = reader.as_mut().unwrap();
        writeln!(
            connection.get_mut(),
            "{{\"id\":\"perf-{n}\",\"op\":\"scene\",\"n\":0}}"
        )
        .unwrap();
        line.clear();
        connection.read_line(&mut line).unwrap();
        ipc_ns.push(began.elapsed().as_nanos() as u64);
        let response: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(response["id"], format!("perf-{n}"));
        assert_eq!(response["ok"], true, "{response}");
        let event = gui.event(
            "Crossfader",
            Action::SetValue,
            Some(ActionData::NumericValue(25.0)),
        );
        gui.frame(vec![event]);
        let held: Vec<_> = gui
            .rt
            .tracks
            .iter()
            .flat_map(|t| t.poly.voices.iter())
            .filter(|v| v.input.is_some() && matches!(v.env.stage, 1..=3))
            .collect();
        if on {
            use crate::engine::dsp::{InputKey, VoiceOwner};
            let source = |channel| {
                held.iter().find_map(|v| match v.input {
                    Some(InputKey::Midi { source, ch, .. }) if ch == channel => Some(source),
                    _ => None,
                })
            };
            let sources = source(9).zip(source(0));
            if held_sources.is_none() {
                held_sources = sources;
            }
            held_ownership &= held.len() == 4 && sources == held_sources;
            if let Some((pad_source, keyboard_source)) = held_sources {
                held_ownership &= pad_source != keyboard_source;
                for (source, ch, pitch) in [
                    (pad_source, 9, pad),
                    (pad_source, 9, pad + 12),
                    (keyboard_source, 0, note),
                    (keyboard_source, 0, note + 12),
                ] {
                    held_ownership &= held
                        .iter()
                        .filter(|voice| {
                            voice.owner == VoiceOwner::Live
                                && voice.input
                                    == Some(InputKey::Midi {
                                        source,
                                        ch,
                                        note: pitch,
                                    })
                                && voice.note() == pitch
                        })
                        .count()
                        == 1;
                }
            } else {
                held_ownership = false;
            }
        } else {
            held_ownership &= held.is_empty();
        }
    }
    drop(reader);
    drop(server);
    std::fs::remove_dir_all(directory).unwrap();
    let stats = gui.app.engine.midi.input_stats();
    let held = gui
        .rt
        .tracks
        .iter()
        .flat_map(|t| t.poly.voices.iter())
        .filter(|v| v.input.is_some() && matches!(v.env.stage, 1..=3))
        .count();
    let checks = json!({"all_input_events_dispatched":stats.dispatched-before==4096,
        "independent_held_chords":held_ownership,
        "no_stuck_notes":held==0,"ui_gain_applied":(gui.rt.xfader-0.25).abs()<1e-6,
        "midi_values_applied":(gui.rt.decks[1].pitch-70.0/127.0).abs()<1e-6
            &&(gui.rt.decks[1].gain-88.0/127.0).abs()<1e-6 &&(gui.rt.tracks[0].gain-64.0/127.0).abs()<1e-6,
        "ipc_scene_applied":gui.rt.tracks[0].playing.is_some_and(|p|p.scene==0),
        "no_history_failure":gui.app.engine.undo.view().failures==0});
    let value = json!({"metrics":{"midi_received":stats.received,"midi_dispatched":stats.dispatched-before,
        "midi_dropped":stats.dropped,"midi_resets":stats.resets,"ipc_requests":UI_FRAMES,
        "rejected_commands":gui.app.engine.cmd.stats().rejected},
        "samples":{"frame_wall_ns":gui.frame_ns,"ipc_roundtrip_ns":ipc_ns,"midi_dispatch_ns":ingress_ns},
        "checks":checks,"observations":{}});
    // Unblock the controllable manager's next discovery before its owner drops.
    drop(control);
    value
}

fn project_roundtrip() -> Value {
    let directory = private_directory("project");
    let path = directory.join("native project.omat");
    let mut gui = Gui::new();
    for track in 0..TRACKS {
        gui.app.send(Command::SetNotes {
            track: track as u8,
            scene: 0,
            notes: (0..1024)
                .map(|n| crate::engine::MidiNote {
                    channel:0,release_vel:64,source_timing:None, id: crate::engine::midi_edit::NoteId::new(), muted: false,
                    pitch: 36 + ((n + track) % 48) as u8,
                    start: n as f32 * 0.125,
                    len: 0.1,
                    vel: 90,
                })
                .collect(),
        });
        gui.rt.process(&mut []);
    }
    gui.action(
        "Track 1: Gain",
        Action::SetValue,
        Some(ActionData::NumericValue(31.0)),
    );
    gui.key(Key::Z, egui::Modifiers::CTRL);
    let undo = gui.rt.tracks[0].gain != 0.31;
    gui.key(Key::Z, egui::Modifiers::CTRL | egui::Modifiers::SHIFT);
    let redo = (gui.rt.tracks[0].gain - 0.31).abs() < 1e-6;
    let began = Instant::now();
    gui.save(&path);
    let save_ns = began.elapsed().as_nanos() as u64;
    let saved = crate::project_file::load::<Document>(
        &path,
        &crate::project_file::Limits::default(),
        &AtomicBool::new(false),
    )
    .unwrap();
    let began = Instant::now();
    gui.reopen(&path);
    let reopen_ns = began.elapsed().as_nanos() as u64;
    let notes = gui
        .rt
        .tracks
        .iter()
        .map(|t| t.clips[0].notes.len())
        .sum::<usize>();
    let exact = gui.rt.tracks.iter().enumerate().all(|(track, t)| {
        t.clips[0].notes.iter().enumerate().all(|(n, note)| {
            note.pitch == 36 + ((n + track) % 48) as u8
                && note.start == n as f32 * 0.125
                && note.len == 0.1
                && note.vel == 90
        })
    });
    let value = json!({"metrics":{"saved_notes":saved.state.engine.tracks.iter().map(|t|t.clips[0].notes.len()).sum::<usize>(),
        "reopened_notes":notes,"save_ns":save_ns,"reopen_ns":reopen_ns,"rejected_commands":gui.app.engine.cmd.stats().rejected},
        "samples":{"frame_wall_ns":gui.frame_ns},"observations":{},
        "checks":{"undo_applied":undo,"redo_applied":redo,"all_notes_exact":notes==8192&&exact,
            "gain_persisted":(gui.rt.tracks[0].gain-0.31).abs()<1e-6,"opened_stopped":!gui.rt.playing&&gui.rt.decks.iter().all(|d|!d.playing),
            "opened_clean":!gui.app.project_dirty()}});
    drop(gui);
    std::fs::remove_dir_all(directory).unwrap();
    value
}

fn long_recording() -> Value {
    let directory = private_directory("recording");
    let path = directory.join("ten minute notes.omat");
    let mut gui = Gui::new();
    gui.rt.bpm = 120.0;
    gui.rt.quant = 0.0;
    gui.rt.selected_track = 1;
    gui.rt.selected_scene = 0;
    gui.rt.tracks[1].clips[0].notes.clear();
    gui.rt.tracks[1].clips[0].kind = ClipKind::Midi;
    gui.rt.tracks[1].clips[0].bars = 512.0;
    gui.frame(vec![]);
    gui.action("Arm selected cell", Action::Click, None);
    assert_eq!(
        gui.rt.compose_target,
        Some(crate::engine::ComposeTarget { track: 1, scene: 0 })
    );
    request(&mut gui, "record");
    let clip = gui
        .nodes
        .iter()
        .filter_map(|(_, n)| n.label())
        .find(|label| label.starts_with("Clip track 2 scene 1:"))
        .unwrap()
        .to_owned();
    gui.action(&clip, Action::Click, None);
    assert!(gui.rt.recording && gui.rt.playing);
    let origin = (gui.rt.beat - gui.rt.tracks[1].playing.unwrap().start_beat) as f32;
    let control = midi::install(&mut gui.app.engine);
    control.discover(&[("a", "Generic MIDI Keyboard")]);
    let input = control.connect("a", Ok(()));
    midi::until(|| gui.app.engine.midi.policy_status().unwrap().applied == Some(1));
    let mut output = vec![0.0; 9600];
    let mut render_ns = Vec::with_capacity(6000);
    let mut allocations = 0;
    let mut frees = 0;
    let mut sent = 0;
    let mut finite = true;
    for block in 0..6000 {
        let pitch = 48 + ((block / 10) % 24) as u8;
        if block % 10 == 0 || block % 10 == 5 {
            input.push(&[
                if block % 10 == 0 { 0x90 } else { 0x80 },
                pitch,
                if block % 10 == 0 { 95 } else { 0 },
            ]);
            sent += 1;
            midi::until(|| gui.app.engine.midi.input_stats().dispatched == sent);
        }
        let began = Instant::now();
        let measure = test_alloc::measure(|| gui.rt.process(&mut output));
        render_ns.push(began.elapsed().as_nanos() as u64);
        allocations += measure.allocations;
        frees += measure.frees;
        finite &= output.iter().all(|v| v.is_finite());
        std::thread::yield_now();
    }
    request(&mut gui, "stop");
    gui.frame(vec![]);
    let correct = |notes: &[crate::engine::MidiNote]| {
        notes.len() == 600
            && notes.iter().enumerate().all(|(i, n)| {
                n.pitch == 48 + (i % 24) as u8
                    && n.vel == 95
                    && (n.len - 1.0).abs() < 1e-4
                    && (n.start - (origin + i as f32 * 2.0)).abs() < 0.0002
            })
    };
    let recorded = correct(&gui.rt.tracks[1].clips[0].notes);
    gui.save(&path);
    gui.reopen(&path);
    let stats = gui.app.engine.midi.input_stats();
    let value = json!({"metrics":{"allocations":allocations,"frees":frees,"recorded_notes":gui.rt.tracks[1].clips[0].notes.len(),
        "midi_dispatched":stats.dispatched,"midi_dropped":stats.dropped,"rejected_commands":gui.app.engine.cmd.stats().rejected},
        "samples":{"renderer_wall_ns":render_ns,"frame_wall_ns":gui.frame_ns},"observations":{},
        "checks":{"finite_output":finite,"recorded_notes_exact":recorded,"reopened_notes_exact":correct(&gui.rt.tracks[1].clips[0].notes),
            "no_history_failure":gui.app.engine.undo.view().failures==0,"compose_disarmed":gui.rt.compose_target.is_none()}});
    drop(control);
    drop(gui);
    std::fs::remove_dir_all(directory).unwrap();
    value
}

fn group(id: &str, conditions: Value, run: fn() -> Value) -> Value {
    eprintln!("fixed workload {id}: three fresh sessions");
    json!({"id":id,"conditions":conditions,"measurements":(0..3).map(|_|run()).collect::<Vec<_>>()})
}

#[test]
#[ignore = "fixed local release gate; requires an explicit private raw report destination"]
fn fixed_native_workloads() {
    assert!(!cfg!(debug_assertions), "release build required");
    let destination =
        std::env::var_os("OMATAINER_BENCHMARK_RAW_REPORT").expect("private raw report destination");
    let mut workloads = crate::engine::performance_workload_tests::callbacks();
    workloads.push(group("large_crate_ui",json!({"width":1440,"height":1000,"frames":UI_FRAMES,"warmup_frames":WARMUP,"entries":50_000}),large_crate));
    workloads.push(group("multi_controller_ipc",json!({"width":1440,"height":1000,"frames":UI_FRAMES,"warmup_frames":WARMUP,"inputs":5,"events_per_frame":8,"ipc_requests":UI_FRAMES,"ipc_connections":16,"requests_per_connection":32}),multi_controller));
    workloads.push(group("project_roundtrip",json!({"width":1440,"height":1000,"warmup_frames":WARMUP,"tracks":8,"notes_per_track":1024}),project_roundtrip));
    workloads.push(group("long_note_recording",json!({"width":1440,"height":1000,"warmup_frames":WARMUP,"sample_rate":48_000,"frames":4800,"blocks":6000,"virtual_seconds":600,"notes":600}),long_recording));
    let report = json!({"schema":1,"suite":"supported-workloads-v1","workloads":workloads,
        "embedded_manifest":serde_json::from_str::<Value>(crate::licenses::MANIFEST).unwrap()});
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(destination)
        .unwrap();
    file.write_all(&serde_json::to_vec_pretty(&report).unwrap())
        .unwrap();
    file.sync_all().unwrap();
    for workload in report["workloads"].as_array().unwrap() {
        for measurement in workload["measurements"].as_array().unwrap() {
            assert!(
                measurement["checks"]
                    .as_object()
                    .unwrap()
                    .values()
                    .all(|check| check == true),
                "{}: {:?}",
                workload["id"],
                measurement["checks"]
            );
        }
    }
}

#[test]
#[ignore = "release component investigation; complete gate still requires every workload"]
fn controller_component() {
    assert!(!cfg!(debug_assertions));
    let result = multi_controller();
    assert!(
        result["checks"]
            .as_object()
            .unwrap()
            .values()
            .all(|v| v == true),
        "{:?}",
        result["checks"]
    );
}
