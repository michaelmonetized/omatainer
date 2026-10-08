use super::*;
use crate::engine::RtEngine;
use egui::accesskit::{Action, ActionRequest, Node, NodeId};
use std::sync::atomic::AtomicUsize;
use std::time::Duration;

struct Files(PathBuf);
impl Files {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "omat-analysis-ui-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        use std::os::unix::fs::DirBuilderExt;
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&root)
            .unwrap();
        Self(root)
    }
    fn sources(&self) -> Vec<PathBuf> {
        [
            (
                "Analysis A.flac",
                include_bytes!("../../../tests/fixtures/audio/tone.flac").as_slice(),
            ),
            (
                "Analysis B.ogg",
                include_bytes!("../../../tests/fixtures/audio/tone.ogg").as_slice(),
            ),
            (
                "Analysis C.mp3",
                include_bytes!("../../../tests/fixtures/audio/tone.mp3").as_slice(),
            ),
        ]
        .into_iter()
        .map(|(name, bytes)| {
            let path = self.0.join(name);
            std::fs::write(&path, bytes).unwrap();
            path
        })
        .collect()
    }
}
impl Drop for Files {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
struct Gui {
    app: App,
    rt: Box<RtEngine>,
    ctx: egui::Context,
    nodes: Vec<(NodeId, Node)>,
    time: f64,
    rendered: u64,
}
impl Gui {
    fn new(files: &Files) -> Self {
        Self::with_metadata_hook(files, || {})
    }
    fn with_metadata_hook(files: &Files, hook: impl FnMut() + Send + 'static) -> Self {
        let (engine, mut rt) = Engine::headless_for_test(48_000, 256);
        rt.publish_for_test();
        let loader = Loader::start_with_performance(engine.cmd.performance().clone()).unwrap();
        let mut app = App::with_loader(engine, Theme::default(), Some(loader));
        app.library_metadata =
            library_metadata::Metadata::with_hook(files.0.join("catalog.json"), hook);
        app.library_metadata
            .set_performance(app.engine.cmd.performance().clone());
        app.library_initialized = false;
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut gui = Self {
            app,
            rt: Box::new(rt),
            ctx,
            nodes: Vec::new(),
            time: 0.0,
            rendered: 0,
        };
        gui.wait(|gui| !gui.app.library_metadata.active());
        gui
    }
    fn frame(&mut self, events: Vec<egui::Event>) -> egui::FullOutput {
        self.time += 0.02;
        let out = self.ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1600.0, 1400.0))),
                time: Some(self.time),
                focused: true,
                events,
                ..Default::default()
            },
            |ctx| self.app.update_frame(ctx),
        );
        self.nodes = out
            .platform_output
            .accesskit_update
            .as_ref()
            .unwrap()
            .nodes
            .clone();
        self.rt.process(&mut [0.0; 256]);
        self.rt.publish_for_test();
        self.rendered += 128;
        out
    }
    fn wait(&mut self, mut predicate: impl FnMut(&Self) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            self.frame(vec![]);
            if predicate(self) {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "analysis wait: {}; metadata {:?}",
                self.app.library_analysis.message,
                self.app.library_metadata.storage_error
            );
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    fn click(&mut self, label: &str) {
        let target = self
            .nodes
            .iter()
            .find(|(_, node)| node.label() == Some(label))
            .unwrap_or_else(|| {
                panic!(
                    "missing {label}: {:?}",
                    self.nodes
                        .iter()
                        .filter_map(|(_, n)| n.label())
                        .collect::<Vec<_>>()
                )
            })
            .0;
        self.frame(vec![egui::Event::AccessKitActionRequest(ActionRequest {
            target,
            action: Action::Click,
            data: None,
        })]);
        self.frame(vec![]);
    }
    fn scan(&mut self, files: &Files) {
        assert!(self
            .app
            .library_scan
            .start(vec![files.0.clone()], self.app.library.clone()));
        self.wait(|gui| !gui.app.library_scan.active() && !gui.app.library_metadata.active());
        self.app.lib_filter = "Analysis ".into();
        self.app.refresh_library_view();
        self.app.lib_sel = 0;
        self.app.library_analysis.open = true;
        self.frame(vec![]);
        self.frame(vec![]);
    }
    fn finish(&mut self) {
        self.wait(|gui| !gui.app.library_analysis.busy() && !gui.app.library_metadata.active());
    }
    fn record(&self, path: &PathBuf) -> Option<&crate::track_analysis::Record> {
        self.app
            .library_metadata
            .catalog
            .version(&LibSource::File(path.clone()), FileFingerprint::read(path))
            .and_then(|version| version.analysis.as_ref())
    }
}

#[test]
fn captured_mixed_format_queue_runs_during_rendering_and_reuses_verified_cache_after_restart() {
    let files = Files::new();
    let paths = files.sources();
    let mut gui = Gui::new(&files);
    gui.scan(&files);
    gui.rt.apply(Command::Play);
    let before = gui.rendered;
    let library = gui.app.library.clone();
    let indices = gui.app.library_view.indices.clone();
    gui.click("Analyze filtered crate");
    let queue = gui.app.library_analysis.queue.as_ref().unwrap();
    assert!(Arc::ptr_eq(&queue.rows, &library) && Arc::ptr_eq(&queue.indices, &indices));
    gui.app.lib_filter = "does not match".into();
    gui.app.refresh_library_view();
    gui.finish();
    assert!(gui.rendered > before && gui.rt.playing);
    assert!(
        gui.app.library_analysis.message.contains("3 saved"),
        "{}",
        gui.app.library_analysis.message
    );
    for path in &paths {
        let record = gui.record(path).unwrap();
        assert!(record.contains(Fields::ALL));
    }
    drop(gui);
    wait_store_closed(&files.0.join("catalog.json"));
    let mut reopened = Gui::new(&files);
    reopened.app.lib_filter = "Analysis ".into();
    reopened.app.refresh_library_view();
    reopened.app.library_analysis.open = true;
    reopened.frame(vec![]);
    reopened.frame(vec![]);
    reopened.click("Analyze filtered crate");
    reopened.finish();
    assert!(
        reopened
            .app
            .library_analysis
            .message
            .contains("3 reused from cache"),
        "{}",
        reopened.app.library_analysis.message
    );
    let cached = reopened
        .app
        .library_analysis
        .preview
        .as_ref()
        .unwrap()
        .outcome
        .as_ref()
        .unwrap();
    assert!(cached.waveform.as_ref().unwrap().bands.len() > 0);
    assert!(reopened.app.loads.iter().all(Option::is_none));
}

#[test]
fn inspect_is_read_only_and_selective_force_preserves_other_cached_fields() {
    let files = Files::new();
    let paths = files.sources();
    let mut gui = Gui::new(&files);
    gui.scan(&files);
    gui.click("Inspect selected cache");
    gui.finish();
    assert!(paths.iter().all(|path| gui.record(path).is_none()));
    assert!(gui
        .app
        .library_analysis
        .preview
        .as_ref()
        .unwrap()
        .outcome
        .as_ref()
        .unwrap()
        .needed
        .valid());
    gui.click("Analyze BPM");
    gui.click("Analyze waveform"); gui.click("Analyze musical key"); // duration only
    gui.click("Analyze selected row");
    gui.finish();
    let record = gui.record(&paths[0]).unwrap();
    assert!(record.bpm.is_none() && record.waveform.is_none() && record.duration.is_some());
    let duration = record.duration.clone();
    gui.click("Analyze BPM");
    gui.click("Analyze duration");
    gui.click("Force selected fields");
    gui.click("Analyze selected row");
    gui.finish();
    let record = gui.record(&paths[0]).unwrap();
    assert!(record.bpm.is_some());
    assert_eq!(record.duration, duration);
    assert!(record.waveform.is_none());
}

#[test]
fn over_limit_capture_refuses_every_row_and_empty_fields_cannot_enqueue() {
    let files = Files::new();
    let _paths = files.sources();
    let mut gui = Gui::new(&files);
    gui.scan(&files);
    let one = gui.app.library[gui.app.library_view.indices[0]].clone();
    gui.app.library = Arc::new(vec![one; MAX_TRACKS + 1]);
    gui.app.refresh_library_view();
    gui.frame(vec![]);
    gui.click("Analyze filtered crate");
    assert!(!gui.app.library_analysis.busy());
    assert!(gui.app.library_analysis.message.contains("4096"));
    gui.click("Analyze BPM");
    gui.click("Analyze duration");
    gui.click("Analyze waveform");
    gui.click("Analyze source level"); gui.click("Analyze musical key");
    gui.click("Analyze selected row");
    assert!(!gui.app.library_analysis.busy());
    assert!(gui.app.library_analysis.message.contains("at least one"));
    assert!(gui
        .app
        .loader
        .as_ref()
        .unwrap()
        .take_analysis_ready()
        .is_none());
}

#[test]
fn cancelled_and_foreground_preempted_jobs_have_terminal_status_and_retry_the_captured_source() {
    for foreground in [false, true] {
        let files = Files::new();
        let paths = files.sources();
        let mut gui = Gui::new(&files);
        gui.scan(&files);
        let (entered, seen) = std::sync::mpsc::sync_channel(1);
        let (resume, held) = std::sync::mpsc::sync_channel(1);
        let mut once = true;
        gui.app.loader = Some(
            Loader::with_analysis_hook(gui.app.engine.cmd.performance().clone(), move |_| {
                if once {
                    once = false;
                    entered.send(()).unwrap();
                    held.recv_timeout(Duration::from_secs(5)).unwrap();
                }
            })
            .unwrap(),
        );
        gui.click("Analyze selected row");
        gui.wait(|gui| matches!(gui.app.library_analysis.step, Step::Decoding(_)));
        seen.recv_timeout(Duration::from_secs(2)).unwrap();
        if foreground {
            gui.app
                .load_file(0, paths[1].clone(), "explicit foreground");
        } else {
            gui.click("Cancel analysis queue");
        }
        resume.send(()).unwrap();
        if foreground {
            gui.wait(|gui| matches!(gui.app.library_analysis.step, Step::Paused(_)));
            assert!(gui
                .app
                .library_analysis
                .message
                .contains("explicit media load"));
            assert!(gui.record(&paths[0]).is_none());
            gui.app.lib_filter = "Analysis C".into();
            gui.app.refresh_library_view();
            gui.click("Retry current analysis");
            gui.finish();
            assert!(gui.record(&paths[0]).unwrap().contains(Fields::ALL));
            assert!(
                gui.record(&paths[2]).is_none(),
                "retry retargeted current selection"
            );
        } else {
            gui.finish();
            assert!(gui.app.library_analysis.message.contains("cancelled"));
            assert!(gui.record(&paths[0]).is_none());
        }
    }
}

#[test]
fn replaced_source_pauses_queue_and_skip_does_not_modify_prior_versions() {
    let files = Files::new();
    let paths = files.sources();
    let mut gui = Gui::new(&files);
    gui.scan(&files);
    let old = FileFingerprint::read(&paths[0]).unwrap();
    std::fs::write(&paths[0], b"replacement bytes are not the captured source").unwrap();
    gui.click("Analyze selected row");
    gui.wait(|gui| matches!(gui.app.library_analysis.step, Step::Paused(_)));
    assert!(gui.app.library_analysis.message.contains("changed"));
    gui.click("Skip current analysis");
    gui.finish();
    assert!(gui
        .app
        .library_metadata
        .catalog
        .version(&LibSource::File(paths[0].clone()), Some(old))
        .unwrap()
        .analysis
        .is_none());
    assert_eq!(
        std::fs::read(&paths[0]).unwrap(),
        b"replacement bytes are not the captured source"
    );
}

#[test]
fn retained_cancel_action_cannot_turn_into_retry_when_the_current_job_is_preempted() {
    let files = Files::new();
    let paths = files.sources();
    let mut gui = Gui::new(&files);
    gui.scan(&files);
    let (entered, seen) = std::sync::mpsc::sync_channel(1);
    let (resume, held) = std::sync::mpsc::sync_channel(1);
    let mut first = true;
    gui.app.loader = Some(
        Loader::with_analysis_hook(gui.app.engine.cmd.performance().clone(), move |_| {
            if first {
                first = false;
                entered.send(()).unwrap();
                held.recv_timeout(Duration::from_secs(5)).unwrap();
            }
        })
        .unwrap(),
    );
    gui.click("Analyze selected row");
    gui.wait(|gui| matches!(gui.app.library_analysis.step, Step::Decoding(_)));
    seen.recv_timeout(Duration::from_secs(2)).unwrap();
    let cancel = gui
        .nodes
        .iter()
        .find(|(_, node)| node.label() == Some("Cancel analysis queue"))
        .unwrap()
        .0;
    gui.app.load_file(0, paths[1].clone(), "foreground");
    resume.send(()).unwrap();
    gui.wait(|gui| matches!(gui.app.library_analysis.step, Step::Paused(_)));
    let retry = gui
        .nodes
        .iter()
        .find(|(_, node)| node.label() == Some("Retry current analysis"))
        .unwrap()
        .0;
    assert_ne!(
        cancel, retry,
        "retained Cancel became Retry after a terminal preemption"
    );
    gui.frame(vec![egui::Event::AccessKitActionRequest(ActionRequest {
        target: cancel,
        action: Action::Click,
        data: None,
    })]);
    gui.finish();
    assert!(gui.app.library_analysis.message.contains("cancelled"));
    assert!(
        gui.record(&paths[0]).is_none(),
        "old Cancel restarted and saved analysis"
    );
}

#[test]
fn unavailable_owner_stops_queue_and_retains_large_view_payloads_without_retrying() {
    let files = Files::new();
    let _paths = files.sources();
    let failed = Arc::new(AtomicBool::new(false));
    let flag = failed.clone();
    let mut gui = Gui::with_metadata_hook(&files, move || {
        assert!(
            !flag.load(Ordering::Acquire),
            "controlled analysis owner failure"
        );
    });
    gui.scan(&files);
    let rows = gui.app.library.clone();
    let indices = gui.app.library_view.indices.clone();
    failed.store(true, Ordering::Release);
    gui.click("Analyze filtered crate");
    gui.wait(|gui| {
        !gui.app.library_metadata.analysis_worker_available()
            && gui.app.library_analysis.queue.is_none()
    });
    assert!(gui.app.library_analysis.queue.is_none());
    let retired = gui.app.library_analysis.retiring.as_ref().unwrap();
    assert!(Arc::ptr_eq(&rows, &retired.0) && Arc::ptr_eq(&indices, &retired.1));
    assert!(gui.app.library_analysis.message.contains("unknown outcome"));
    let last = gui.app.library_analysis.next_inspection;
    for _ in 0..32 {
        gui.frame(vec![]);
    }
    assert_eq!(gui.app.library_analysis.next_inspection, last);
    assert!(gui.app.library_analysis.retiring.is_some());
}

#[test]
fn protection_retires_optional_preview_and_close_cancels_unclaimed_queue_without_new_admission() {
    let files = Files::new();
    let paths = files.sources();
    let mut gui = Gui::new(&files);
    gui.scan(&files);
    gui.click("Inspect selected cache");
    gui.finish();
    assert!(gui.app.library_analysis.preview.is_some());
    gui.app.engine.cmd.performance().set_enabled(true).unwrap();
    gui.frame(vec![]);
    assert!(gui.app.library_analysis.preview.is_none());
    gui.click("Analyze selected row");
    assert!(gui.app.library_analysis.queue.is_none());
    gui.app.engine.cmd.performance().set_enabled(false).unwrap();
    let (entered, seen) = std::sync::mpsc::sync_channel(1);
    let (resume, held) = std::sync::mpsc::sync_channel(1);
    gui.app.loader = Some(
        Loader::with_analysis_hook(gui.app.engine.cmd.performance().clone(), move |_| {
            entered.send(()).unwrap();
            held.recv_timeout(Duration::from_secs(5)).unwrap();
        })
        .unwrap(),
    );
    gui.click("Analyze selected row");
    gui.wait(|gui| matches!(gui.app.library_analysis.step, Step::Decoding(_)));
    seen.recv_timeout(Duration::from_secs(2)).unwrap();
    let token = match &gui.app.library_analysis.step {
        Step::Decoding(token) => token.clone(),
        _ => unreachable!(),
    };
    gui.app.request_library_close(&gui.ctx);
    assert!(matches!(
        token.failure(),
        Some(crate::engine::media_load::AnalysisFailure::Cancelled)
    ));
    assert!(gui.app.library_analysis.queue.is_none());
    gui.app.start_library_analysis(false, Purpose::Analyze);
    assert!(gui.app.library_analysis.queue.is_none());
    assert!(gui.app.library_analysis.message.contains("close"));
    resume.send(()).unwrap();
    gui.wait(|gui| {
        !gui.app.library_metadata.active() && gui.app.library_analysis.completion.is_none()
    });
    assert!(gui.record(&paths[0]).is_none());
}

mod long;

fn wait_store_closed(path: &std::path::Path) {
    let until = Instant::now() + Duration::from_secs(8);
    loop {
        match crate::library::Store::open(path.to_path_buf()) {
            Ok(owner) => {
                drop(owner);
                return;
            }
            Err(error) => {
                assert!(
                    Instant::now() < until,
                    "old catalog owner did not retire: {error}"
                );
                std::thread::sleep(Duration::from_millis(2));
            }
        }
    }
}

#[test]
fn native_filtered_replacement_preview_reports_locks_without_decoding_or_saving() {
    let files=Files::new();let paths=files.sources();let mut gui=Gui::new(&files);gui.scan(&files);
    gui.app.library_analysis.open=false;gui.app.library_protection.open=true;gui.frame(vec![]);
    gui.click("Capture filtered preparation");
    for label in ["Change BPM lock","Lock BPM","Change metadata lock","Lock metadata"] {gui.click(label);}
    gui.click("Review preparation locks");gui.click("Save reviewed preparation locks");
    gui.wait(|g|!g.app.library_metadata.active() && g.app.library_crates.pending.is_none());
    gui.app.library_protection.open=false;gui.app.library_analysis.open=true;gui.frame(vec![]);
    let old=std::fs::read(files.0.join("catalog.json")).unwrap();
    gui.click("Force selected fields");gui.click("Preview filtered analysis changes");
    gui.app.lib_filter="no matching track".into();gui.app.refresh_library_view();gui.finish();
    assert_eq!(gui.app.library_analysis.changes.len(),3);
    assert!(gui.app.library_analysis.message.contains("Previewed 3"));
    let labels:Vec<_>=gui.nodes.iter().filter_map(|(_,node)|node.label().or(node.value())).collect();
    assert!(labels.iter().any(|label|label.contains("BPM:") && label.contains("keep: locked")),"{labels:?}");
    assert!(labels.iter().any(|label|label.contains("Duration:") && label.contains("keep: locked")));
    assert!(labels.iter().any(|label|label.contains("Waveform: none → refresh analysis")));
    assert_eq!(std::fs::read(files.0.join("catalog.json")).unwrap(),old);
    for path in &paths {assert!(gui.record(path).is_none());}
    gui.app.lib_filter="Analysis ".into();gui.app.refresh_library_view();gui.frame(vec![]);
    gui.click("Analyze waveform");gui.click("Analyze source level"); gui.click("Analyze musical key");gui.click("Analyze filtered crate");gui.finish();
    assert!(gui.app.library_analysis.message.contains("3 skipped"),"{}",gui.app.library_analysis.message);
    assert_eq!(std::fs::read(files.0.join("catalog.json")).unwrap(),old);
    for path in paths {assert!(gui.record(&path).is_none());}
}

impl Gui {
    fn key_text(&mut self, label: &str, value: &str) {
        let target=self.nodes.iter().find(|(_,node)|node.label()==Some(label)).unwrap().0;
        self.frame(vec![egui::Event::AccessKitActionRequest(ActionRequest {target,action:Action::Focus,data:None})]);
        for pressed in [true,false] {
            self.frame(vec![egui::Event::Key {key:egui::Key::A,physical_key:None,pressed,repeat:false,
                modifiers:egui::Modifiers {ctrl:true,command:true,..Default::default()}}]);
        }
        self.frame(vec![egui::Event::Text(value.into())]);self.frame(vec![]);
    }
}

#[test]
fn musical_key_native_review_compare_correct_force_and_reopen_keep_user_value() {
    let files=Files::new();
    let path=files.0.join("Analysis Key.wav");
    let rate=16000u32;
    let mut wav=crate::engine::media_analysis::tests::wav(rate as usize*8,rate,1,true);
    for frame in 0..rate as usize*8 {
        let sample=[60.0,64.0,67.0].iter().map(|note| {
            let frequency=440.0*2.0f64.powf((note-69.0)/12.0);
            (std::f64::consts::TAU*frequency*frame as f64/f64::from(rate)).sin()*0.08
        }).sum::<f64>();
        wav[44+frame*2..46+frame*2].copy_from_slice(&((sample*i16::MAX as f64) as i16).to_le_bytes());
    }
    std::fs::write(&path,wav).unwrap();
    let mut gui=Gui::new(&files);gui.scan(&files);
    for field in ["Analyze BPM","Analyze duration","Analyze waveform","Analyze source level"] {gui.click(field);}
    gui.click("Analyze selected row");gui.finish();
    let key=gui.record(&path).unwrap().key.as_ref().unwrap().value;
    assert_eq!(key.key,Some(crate::musical_key::Key {tonic:0,minor:false}));
    gui.app.lib_filter="key:C".into();gui.app.refresh_library_view();assert!(gui.app.library_view.indices.iter().any(|index|gui.app.library[*index].source==LibSource::File(path.clone())));
    gui.app.lib_filter="key:G".into();gui.app.refresh_library_view();assert!(!gui.app.library_view.indices.iter().any(|index|gui.app.library[*index].source==LibSource::File(path.clone())));
    gui.app.lib_filter="Analysis".into();gui.app.refresh_library_view();
    gui.click("Inspect selected cache");gui.finish();gui.frame(vec![]);
    assert!(gui.nodes.iter().any(|(_,node)|node.value().or_else(||node.label()).is_some_and(|label|label.contains("Analyzed key: C · 8B"))), "{}", gui.app.library_analysis.evidence());
    gui.key_text("Compare with key","8A");
    assert!(gui.nodes.iter().any(|(_,node)|node.value().or_else(||node.label())==Some("Compatible source keys: same, relative or adjacent on the harmonic wheel")));
    gui.key_text("Compare with key","3A");
    assert!(gui.nodes.iter().any(|(_,node)|node.value().or_else(||node.label())==Some("Source keys are not adjacent on the harmonic wheel")));
    gui.click("Review a key correction for the selected library row");
    gui.wait(|gui|!gui.app.library_tags.busy());
    gui.click("Change Key");gui.key_text("New Key","F#m");
    gui.click("Write supported embedded tags; use a library sidecar when writing is unavailable");
    gui.click("Review these field changes");gui.click("Apply reviewed edits to captured tracks");
    gui.wait(|gui|!gui.app.library_tags.busy()&&!gui.app.library_metadata.active());
    gui.app.library_tags.open=false;gui.frame(vec![]);
    gui.click("Force selected fields");gui.click("Analyze selected row");gui.finish();
    assert!(gui.app.library_analysis.message.contains("1 saved"),"{}",gui.app.library_analysis.message);
    let saved=crate::library::read(&files.0.join("catalog.json")).unwrap();
    let version=saved.version(&LibSource::File(path.clone()),FileFingerprint::read(&path)).unwrap();
    assert_eq!(version.metadata.key,"F#m");assert_eq!(version.analysis.as_ref().unwrap().key.as_ref().unwrap().value,key);
    assert_eq!(crate::musical_key::display(Some(version),"",false).0,"F#m · 11A");
    drop(gui);wait_store_closed(&files.0.join("catalog.json"));
    let mut reopened=Gui::new(&files);reopened.scan(&files);
    assert_eq!(reopened.record(&path).unwrap().key.as_ref().unwrap().value,key);
    assert!(reopened.app.library_view.cells.values().any(|cells|cells.key=="F#m · 11A"));
}

#[test]
fn indexed_large_catalog_analysis_and_real_file_replacement_preserve_selected_row_and_loaded_audio() {
    let files=Files::new();let paths=files.sources();let store_path=files.0.join("catalog.json");
    let mut store=crate::library::Store::open(store_path).unwrap();
    for index in 0..10000 {store.catalog.upsert(LibSource::File(format!("/indexed-owner/{index}.wav").into()),None,crate::library::Metadata {title:format!("Library {index:05}"),artist:"Fixture".into(),bpm:Bpm::hint(120.0),key:"C".into(),duration:Some(180.0),last_play:None}).unwrap();}
    for path in &paths {store.catalog.upsert(LibSource::File(path.clone()),FileFingerprint::read(path),crate::library::Metadata {title:path.file_name().unwrap().to_str().unwrap().into(),artist:"Fixture".into(),bpm:Bpm::UNKNOWN,key:"—".into(),duration:None,last_play:None}).unwrap();}
    store.save().unwrap();drop(store);
    let blocked=Arc::new(AtomicBool::new(false));let entered=Arc::new(AtomicBool::new(false));
    struct Release(Arc<AtomicBool>);impl Drop for Release {fn drop(&mut self){self.0.store(false,Ordering::Release);}}
    let _release=Release(blocked.clone());let block=blocked.clone();let waiting=entered.clone();
    let mut gui=Gui::with_metadata_hook(&files,move||{if block.load(Ordering::Acquire){waiting.store(true,Ordering::Release);while block.load(Ordering::Acquire){std::thread::sleep(Duration::from_millis(1));}}});
    let initial_tracks=gui.app.library.len();assert_eq!(initial_tracks,10005);
    assert_eq!(gui.app.library.iter().filter(|item|matches!(&item.source,LibSource::File(path) if path.starts_with("/indexed-owner"))).count(),10000);
    assert_eq!(gui.app.library.iter().filter(|item|matches!(item.source,LibSource::Builtin(_))).count(),2);
    for path in &paths {assert!(gui.app.library.iter().any(|item|item.source==LibSource::File(path.clone())));}
    assert!(gui.app.library_metadata.collection_rows().is_for(&gui.app.library,&gui.app.library_metadata.catalog));
    gui.app.lib_filter="title:\"Analysis A.flac\"".into();gui.app.refresh_library_view();assert_eq!(gui.app.library_view.indices.len(),1);
    let initial=gui.rt.decks[0].audio.clone().unwrap();gui.app.load_sel(0);
    gui.wait(|gui|gui.app.loads[0].as_ref().is_some_and(|load|matches!(load.phase,crate::ui::load_status::Phase::Loaded))&&!Arc::ptr_eq(&gui.rt.decks[0].audio.as_ref().unwrap(),&initial));
    let loaded=gui.rt.decks[0].audio.clone().unwrap();let key=gui.app.engine.snapshot().decks[0].media_key;let original=FileFingerprint::read(&paths[0]).unwrap();
    gui.rt.apply(Command::DeckPlay {deck:0});gui.rt.apply(Command::DeckLoop {deck:0,beats:1.0});gui.rt.publish_for_test();
    gui.app.lib_filter="title:\"Analysis B.ogg\"".into();gui.app.refresh_library_view();let selected=gui.app.selected_library_item().unwrap().source.clone();
    gui.app.library_analysis.open=true;gui.frame(vec![]);gui.frame(vec![]);
    blocked.store(true,Ordering::Release);let started=Instant::now();gui.click("Analyze selected row");gui.wait(|_|entered.load(Ordering::Acquire));
    let replacement=files.0.join("replacement.flac");std::fs::write(&replacement,include_bytes!("../../../tests/fixtures/audio/tone.flac")).unwrap();std::fs::rename(&replacement,&paths[0]).unwrap();
    let extra=files.0.join("Added.ogg");std::fs::write(&extra,include_bytes!("../../../tests/fixtures/audio/tone.ogg")).unwrap();
    assert!(gui.app.library_scan.import(vec![paths[0].clone(),extra.clone()],gui.app.library.clone()));
    let before=gui.rendered;blocked.store(false,Ordering::Release);
    gui.wait(|gui|!gui.app.library_analysis.busy()&&!gui.app.library_scan.active()&&!gui.app.library_metadata.active());
    let elapsed=started.elapsed();assert!(elapsed<Duration::from_secs(5),"production analysis/import publication {elapsed:?}");
    assert!(gui.record(&paths[1]).unwrap().contains(Fields::ALL));assert_eq!(gui.app.selected_library_item().unwrap().source,selected);
    assert_ne!(FileFingerprint::read(&paths[0]),Some(original));assert!(Arc::ptr_eq(&gui.rt.decks[0].audio.as_ref().unwrap(),&loaded));assert_eq!(gui.app.engine.snapshot().decks[0].media_key,key);assert!(gui.rt.decks[0].playing);assert!(gui.rendered>before);
    assert!(gui.app.library.iter().any(|row|row.source==LibSource::File(extra.clone())));assert!(gui.app.library_metadata.collection_rows().is_for(&gui.app.library,&gui.app.library_metadata.catalog));
    assert!(gui.app.library_metadata.storage_error.is_none(),"{:?}",gui.app.library_metadata.storage_error);
    println!("LIBRARY_SCALE_OWNER_RECEIPT {}",serde_json::json!({"initial_tracks":initial_tracks,"synthetic_sources":10000,"real_audio_sources":paths.len(),"builtin_sources":2,"published_tracks":gui.app.library.len(),"analysis_and_replacement_import_ns":elapsed.as_nanos(),"actual_analysis_fields":"all","loaded_audio_preserved":true,"selection_preserved":true,"physical_devices_opened":false}));
}
