use super::*;
use crate::ui::piano_roll::tests::Gui;
use egui::accesskit::Action;

struct Files(PathBuf);
impl Files {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "omat-template-{}",
            crate::sampler_bank::BankId::new().unwrap()
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}
impl Drop for Files {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn settle(gui: &mut Gui) {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        gui.frame(vec![]);
        if !gui.app.templates.busy() && !gui.app.project_pending_for_test() {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "template: {:?}; project: {:?}",
            gui.app.templates.error,
            gui.app.project_result_for_test().1
        );
        std::thread::sleep(Duration::from_millis(1));
    }
    gui.frame(vec![]);
}
fn edit(gui: &mut Gui, label: &str, text: &str) {
    gui.action(label, Action::Focus, None);
    gui.key(
        Key::A,
        egui::Modifiers {
            ctrl: true,
            command: true,
            ..Default::default()
        },
    );
    gui.frame(vec![egui::Event::Text(text.into())]);
}
fn open(gui: &mut Gui) {
    gui.click_text("Project");
    gui.click_text("Project and track templates…");
    gui.frame(vec![]);
}
fn save(gui: &mut Gui, path: &Path, track: bool) {
    edit(gui, "Template name", "Reusable session");
    edit(
        gui,
        "New template file (.omtemplate)",
        path.to_str().unwrap(),
    );
    gui.click(if track {
        "Save selected track configuration"
    } else {
        "Save project template"
    });
    settle(gui);
    assert!(
        gui.app.templates.error.is_none(),
        "{:?}",
        gui.app.templates.error
    );
}
fn inspect(gui: &mut Gui, path: &Path) {
    edit(gui, "Template to inspect", path.to_str().unwrap());
    gui.click("Inspect template");
    settle(gui);
    assert!(
        gui.app.templates.error.is_none(),
        "{:?}",
        gui.app.templates.error
    );
}
#[test]
fn native_project_template_copies_start_stopped_and_never_save_over_source() {
    let files = Files::new();
    let original = files.path("source.omtemplate");
    let copy = files.path("copy.omtemplate");
    let mut gui = Gui::new();
    gui.frame(vec![]);
    open(&mut gui);
    save(&mut gui, &original, false);
    let bytes = std::fs::read(&original).unwrap();
    let old_namespace = gui.rt.session.namespace;
    inspect(&mut gui, &original);
    edit(
        &mut gui,
        "New template file (.omtemplate)",
        copy.to_str().unwrap(),
    );
    edit(&mut gui, "Template name", "My copy");
    gui.click("Duplicate template to new file");
    settle(&mut gui);
    assert_eq!(std::fs::read(&original).unwrap(), bytes);
    let (copied, _) = worker::load(&copy, &AtomicBool::new(false)).unwrap();
    assert_eq!(copied.state.metadata.name, "My copy");
    gui.click("Create project from template");
    if gui
        .nodes
        .iter()
        .any(|(_, node)| node.label() == Some("Discard changes"))
    {
        gui.click("Discard changes");
    }
    settle(&mut gui);
    assert_ne!(gui.rt.session.namespace, old_namespace);
    assert!(!gui.rt.playing);
    assert!(gui.app.project_result_for_test().0.is_none());
    assert!(gui.app.project_dirty());
    assert_eq!(std::fs::read(&original).unwrap(), bytes);
    assert!(crate::project_file::load::<project::Document>(
        &original,
        &crate::project_file::Limits::default(),
        &AtomicBool::new(false)
    )
    .is_err());
    let first = startup_session(
        &model::Startup::Template { path: copy.clone() },
        &AtomicBool::new(false),
    )
    .unwrap()
    .unwrap();
    let second = startup_session(
        &model::Startup::Template { path: copy },
        &AtomicBool::new(false),
    )
    .unwrap()
    .unwrap();
    let namespace = |initial| {
        let crate::engine::InitialSession::Project { state, .. } = initial else {
            panic!("project template expected")
        };
        state.session.unwrap().namespace
    };
    assert_ne!(namespace(first.initial), namespace(second.initial));
    let empty = startup_session(&model::Startup::Empty, &AtomicBool::new(false))
        .unwrap()
        .unwrap();
    assert!(matches!(
        empty.initial,
        crate::engine::InitialSession::Empty
    ));
    empty.initial.prepare(48_000).unwrap();
}
#[test]
fn native_track_template_keeps_music_and_requires_explicit_hardware_review() {
    let files = Files::new();
    let template = files.path("track.omtemplate");
    let mut gui = Gui::new();
    gui.app.send(Command::Select { track: 0, scene: 0 });
    gui.frame(vec![]);
    gui.rt.tracks[0].gain = 0.37;
    let namespace = gui.rt.session.namespace;
    open(&mut gui);
    save(&mut gui, &template, true);
    let (saved, _) = worker::load(&template, &AtomicBool::new(false)).unwrap();
    assert_eq!(saved.state.document.engine.tracks[0].gain, 0.37);
    assert!(startup_session(
        &model::Startup::Template {
            path: template.clone()
        },
        &AtomicBool::new(false)
    )
    .is_err());
    inspect(&mut gui, &template);
    gui.rt.tracks[0].gain = 0.82;
    let before = gui.rt.tracks[0].clips[0].notes.clone();
    let active = gui.app.settings.applied.clone();
    gui.click("Apply to selected track");
    settle(&mut gui);
    assert_eq!(gui.rt.session.namespace, namespace);
    assert_eq!(gui.rt.tracks[0].gain, 0.37);
    assert_eq!(gui.rt.tracks[0].clips[0].notes, before);
    assert_eq!(gui.app.settings.applied, active);
    gui.click("Review loaded template hardware");
    gui.frame(vec![]);
    assert!(gui.app.settings.open);
    assert_eq!(gui.app.settings.applied, active);
}

#[test]
fn native_template_archive_migrates_without_sources_and_refuses_existing_destination() {
    let files = Files::new();
    let source = files.path("source.omtemplate");
    let archive = files.path("backup.ompack");
    let destination = files.path("imported");
    let mut gui = Gui::new();
    gui.screen.y = 1900.0;
    gui.frame(vec![]);
    open(&mut gui);
    save(&mut gui, &source, false);
    inspect(&mut gui, &source);
    let original = std::fs::read(&source).unwrap();
    edit(
        &mut gui,
        "New template backup archive (.ompack)",
        archive.to_str().unwrap(),
    );
    gui.click("Back up inspected template");
    settle(&mut gui);
    assert!(
        gui.app.templates.error.is_none(),
        "{:?}",
        gui.app.templates.error
    );
    assert_eq!(std::fs::read(&source).unwrap(), original);
    std::fs::remove_file(&source).unwrap();
    drop(gui);
    let mut fresh = Gui::new();
    fresh.screen.y = 1900.0;
    fresh.frame(vec![]);
    open(&mut fresh);
    edit(
        &mut fresh,
        "Template archive to import",
        archive.to_str().unwrap(),
    );
    fresh.click("Review template archive");
    settle(&mut fresh);
    edit(
        &mut fresh,
        "New imported template folder",
        destination.to_str().unwrap(),
    );
    fresh.click("Import portable template");
    settle(&mut fresh);
    assert!(
        fresh.app.templates.error.is_none(),
        "{:?}",
        fresh.app.templates.error
    );
    let imported = destination.join("template.omtemplate");
    let imported_bytes = std::fs::read(&imported).unwrap();
    inspect(&mut fresh, &imported);
    fresh.click("Create project from template");
    if fresh
        .nodes
        .iter()
        .any(|(_, node)| node.label() == Some("Discard changes"))
    {
        fresh.click("Discard changes");
    }
    settle(&mut fresh);
    fresh.app.send(Command::LaunchScene { scene: 0 });
    fresh.frame(vec![]);
    let mut audio = [0.0; 4096];
    fresh.rt.process(&mut audio);
    assert!(audio.iter().all(|value| value.is_finite()));
    assert!(audio.iter().any(|value| value.abs() > 0.0001));
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "ui::templates::tests::fresh_process_template_import",
            "--exact",
            "--ignored",
        ])
        .env("OMATAINER_TEMPLATE_ARCHIVE", &archive)
        .env(
            "OMATAINER_TEMPLATE_DESTINATION",
            files.path("fresh-process"),
        )
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    fresh.click("Import portable template");
    settle(&mut fresh);
    assert!(fresh.app.templates.error.is_some());
    assert_eq!(std::fs::read(&imported).unwrap(), imported_bytes);
}
#[test]
fn missing_changed_and_conflicting_templates_preserve_session_and_existing_files() {
    let files = Files::new();
    let source = files.path("source.omtemplate");
    let mut gui = Gui::new();
    gui.frame(vec![]);
    open(&mut gui);
    let namespace = gui.rt.session.namespace;
    edit(&mut gui, "Template to inspect", source.to_str().unwrap());
    gui.click("Inspect template");
    settle(&mut gui);
    assert!(gui.app.templates.error.is_some());
    assert_eq!(gui.rt.session.namespace, namespace);
    save(&mut gui, &source, false);
    let bytes = std::fs::read(&source).unwrap();
    inspect(&mut gui, &source);
    edit(
        &mut gui,
        "New template file (.omtemplate)",
        source.to_str().unwrap(),
    );
    gui.click("Duplicate template to new file");
    settle(&mut gui);
    assert!(gui.app.templates.error.is_some());
    assert_eq!(std::fs::read(&source).unwrap(), bytes);
    std::fs::write(&source, b"changed template").unwrap();
    gui.click("Create project from template");
    if gui
        .nodes
        .iter()
        .any(|(_, node)| node.label() == Some("Discard changes"))
    {
        gui.click("Discard changes");
    }
    settle(&mut gui);
    assert_eq!(gui.rt.session.namespace, namespace);
    assert!(
        gui.app
            .project_result_for_test()
            .1
            .unwrap()
            .contains("identity")
            || gui.app.templates.error.is_some()
    );
}

#[test]
fn native_cancel_before_template_publication_keeps_source_and_destination_clean() {
    let files = Files::new();
    let source = files.path("source.omtemplate");
    let cancelled = files.path("cancelled.omtemplate");
    let mut gui = Gui::new();
    gui.frame(vec![]);
    open(&mut gui);
    save(&mut gui, &source, false);
    inspect(&mut gui, &source);
    let bytes = std::fs::read(&source).unwrap();
    let namespace = gui.rt.session.namespace;
    let (entered, resume) = gui.app.templates.worker.as_ref().unwrap().pause_next();
    edit(
        &mut gui,
        "New template file (.omtemplate)",
        cancelled.to_str().unwrap(),
    );
    gui.click("Duplicate template to new file");
    entered.recv_timeout(Duration::from_secs(5)).unwrap();
    gui.click("Cancel template operation");
    resume.send(()).unwrap();
    settle(&mut gui);
    assert!(!cancelled.exists());
    assert_eq!(std::fs::read(&source).unwrap(), bytes);
    assert_eq!(gui.rt.session.namespace, namespace);
    assert!(gui
        .app
        .templates
        .error
        .as_deref()
        .unwrap()
        .contains("cancelled"));
    let leftovers: Vec<_> = std::fs::read_dir(&files.0)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(leftovers, [source.file_name().unwrap()]);
}

#[test]
fn native_hardware_inspection_reports_missing_and_ambiguous_ports_without_rerouting() {
    use crate::engine::midi::routing::{Endpoint, Filter, Route, Routing};
    let files = Files::new();
    let path = files.path("hardware.omtemplate");
    let mut gui = Gui::new();
    gui.screen.y = 1900.0;
    crate::engine::midi::routing::install_ports_for_test(
        &mut gui.app.engine,
        Routing::default(),
        true,
    );
    let route = |track, name: &str| Route {
        track,
        inputs: vec![],
        output: Some(Endpoint {
            name: name.into(),
            id: None,
        }),
        output_channel: None,
        monitor: false,
        thru: false,
        filter: Filter::default(),
    };
    gui.app
        .settings
        .applied
        .profiles
        .get_mut("Studio")
        .unwrap()
        .midi_routing = Routing {
        enabled: true,
        routes: vec![route(0, "Missing synth"), route(1, "Synth")],
    };
    gui.frame(vec![]);
    open(&mut gui);
    save(&mut gui, &path, false);
    inspect(&mut gui, &path);
    let output = gui.frame(vec![]);
    let labels: Vec<_> = output
        .shapes
        .iter()
        .filter_map(|shape| match &shape.shape {
            egui::epaint::Shape::Text(text) => Some(text.galley.text()),
            _ => None,
        })
        .collect();
    assert!(
        labels
            .iter()
            .any(|label| label.contains("Missing synth") && label.contains("unavailable")),
        "{labels:?}"
    );
    assert!(
        labels
            .iter()
            .any(|label| label.contains("Synth") && label.contains("ambiguous")),
        "{labels:?}"
    );
    assert_eq!(
        *gui.app.engine.midi.routing_status().unwrap().applied,
        Routing::default()
    );
    let (saved, _) = worker::load(&path, &AtomicBool::new(false)).unwrap();
    assert_eq!(
        saved.state.metadata.hardware.routing,
        gui.app.settings.profile().midi_routing
    );
}

#[test]
#[ignore = "Private fresh-process entry point exercised by the native archive round trip"]
fn fresh_process_template_import() {
    let archive = PathBuf::from(std::env::var_os("OMATAINER_TEMPLATE_ARCHIVE").unwrap());
    let destination = PathBuf::from(std::env::var_os("OMATAINER_TEMPLATE_DESTINATION").unwrap());
    let mut gui = Gui::new();
    gui.screen.y = 1900.0;
    gui.frame(vec![]);
    open(&mut gui);
    edit(
        &mut gui,
        "Template archive to import",
        archive.to_str().unwrap(),
    );
    gui.click("Review template archive");
    settle(&mut gui);
    edit(
        &mut gui,
        "New imported template folder",
        destination.to_str().unwrap(),
    );
    gui.click("Import portable template");
    settle(&mut gui);
    assert!(
        gui.app.templates.error.is_none(),
        "{:?}",
        gui.app.templates.error
    );
    let path = destination.join("template.omtemplate");
    inspect(&mut gui, &path);
    gui.click("Create project from template");
    if gui
        .nodes
        .iter()
        .any(|(_, node)| node.label() == Some("Discard changes"))
    {
        gui.click("Discard changes");
    }
    settle(&mut gui);
    assert!(!gui.rt.playing);
    assert!(gui.app.project_result_for_test().0.is_none());
    gui.app.send(Command::LaunchScene { scene: 0 });
    gui.frame(vec![]);
    let mut audio = [0.0; 4096];
    gui.rt.process(&mut audio);
    assert!(audio.iter().all(|value| value.is_finite()));
    assert!(audio.iter().any(|value| value.abs() > 0.0001));
}
