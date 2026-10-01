use super::*;
use crate::engine::{audio::OutputCallback, dsp::Sample, preparation::Preparation, test_alloc};
use sha2::{Digest, Sha256};
use std::io::Read;

struct Playback {
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<serde_json::Value>>,
}
impl Drop for Playback {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
    }
}
fn player() -> (Engine, OutputCallback) {
    let (engine, mut rt) = Engine::headless_for_test(48_000, 256);
    let sample = Arc::new(Sample {
        name: "Original 440 Hz callback reference".into(),
        sr: 48_000,
        ch: 2,
        data: (0..48_000)
            .flat_map(|i| {
                let value =
                    (i as f64 * 440.0 * std::f64::consts::TAU / 48_000.0).sin() as f32 * 0.15;
                [value, value * 0.5]
            })
            .collect(),
        peaks: Vec::new().into(),
        bpm: 120.0,
        path: String::new(),
    });
    rt.apply(Command::DeckAudio {
        deck: 0,
        audio: sample,
    });
    rt.apply(Command::DeckPlay { deck: 0 });
    rt.decks[0].loop_on = true;
    rt.decks[0].loop_start = 0.0;
    rt.decks[0].loop_len = 48_000.0;
    (engine, OutputCallback::new(rt, 4))
}
fn playback(mut output: OutputCallback) -> Playback {
    let stop = Arc::new(AtomicBool::new(false));
    let done = stop.clone();
    let thread = std::thread::spawn(move || {
        let (_reference_owner, mut reference) = player();
        let mut actual = [0.0f32; 512];
        let mut expected = [0.0f32; 512];
        for _ in 0..256 {
            output.render(&mut actual);
            reference.render(&mut expected);
        }
        let (mut blocks, mut allocations, mut frees) = (0u64, 0usize, 0usize);
        let (mut maximum_error, mut energy) = (0.0f32, 0.0f64);
        let started = Instant::now();
        while !done.load(Ordering::Acquire) {
            let counts = test_alloc::measure(|| output.render(&mut actual));
            reference.render(&mut expected);
            allocations += counts.allocations;
            frees += counts.frees;
            for (value, wanted) in actual.iter().zip(&expected) {
                assert!(value.is_finite());
                maximum_error = maximum_error.max((value - wanted).abs());
                energy += f64::from(*value).powi(2);
            }
            blocks += 1;
            std::thread::sleep(Duration::from_micros(500));
        }
        assert!(
            blocks > 16,
            "analysis ended without a meaningful callback overlap"
        );
        assert_eq!((allocations, frees), (0, 0));
        assert_eq!(maximum_error, 0.0);
        assert!(energy > 0.01);
        let renderer = output.renderer_for_test();
        assert!(renderer.decks[0].playing);
        serde_json::json!({"callback":"production OutputCallback<f32>","channels":4,"frames_per_block":128,
            "blocks":blocks,"rendered_frames":blocks*128,"warmup_blocks_excluded":256,
            "rust_allocations":allocations,"rust_frees":frees,"maximum_reference_error":maximum_error,
            "finite":true,"energy":energy,"elapsed_host_seconds":started.elapsed().as_secs_f64(),
            "deck_still_playing":true,"deadline_policy":null,"hardware_xruns":null,
            "pacing":"500 microsecond sleep outside each measured callback; functional continuity only"})
    });
    Playback {
        stop,
        thread: Some(thread),
    }
}
struct LongGui {
    app: App,
    ctx: egui::Context,
    nodes: Vec<(NodeId, Node)>,
    time: f64,
}
impl LongGui {
    fn frame(&mut self, events: Vec<egui::Event>) {
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
        self.nodes = out.platform_output.accesskit_update.unwrap().nodes;
    }
    fn wait(&mut self, mut ready: impl FnMut(&Self) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(120);
        loop {
            self.frame(vec![]);
            if ready(self) {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "long analysis: {} / {:?}",
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
            .unwrap_or_else(|| panic!("missing {label}"))
            .0;
        self.frame(vec![egui::Event::AccessKitActionRequest(ActionRequest {
            target,
            action: Action::Click,
            data: None,
        })]);
        self.frame(vec![]);
    }
}
fn digest(path: &std::path::Path) -> String {
    let mut file = std::fs::File::open(path).unwrap();
    let mut hash = Sha256::new();
    let mut block = [0u8; 65536];
    loop {
        let read = file.read(&mut block).unwrap();
        if read == 0 {
            break;
        }
        hash.update(&block[..read]);
    }
    format!("{:x}", hash.finalize())
}

#[test]
#[ignore = "explicit long mixed-format qualification; requires OMATAINER_ANALYSIS_LONG_DIR"]
fn long_files_under_callback_analysis() {
    let root = PathBuf::from(
        std::env::var_os("OMATAINER_ANALYSIS_LONG_DIR")
            .expect("explicit fixture directory required"),
    );
    let sources = root.join("sources");
    assert!(sources.is_dir());
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("source-manifest.json")).unwrap()).unwrap();
    assert_eq!(manifest["schema"], 1);
    let entries = manifest["sources"].as_array().unwrap();
    assert_eq!(entries.len(), 3);
    let mut paths = Vec::new();
    let mut original = Vec::new();
    let mut names = std::collections::HashSet::new();
    for entry in entries {
        let name = entry["name"].as_str().unwrap();
        assert!(["long.wav", "long.flac", "long.ogg"].contains(&name));
        assert!(names.insert(name));
        assert_eq!(entry["sample_rate"], 48_000);
        assert_eq!(entry["channels"], 2);
        assert_eq!(entry["requested_seconds"].as_f64(), Some(180.0));
        let path = sources.join(name);
        let hash = digest(&path);
        assert_eq!(hash, entry["file_sha256"].as_str().unwrap());
        assert_eq!(
            std::fs::metadata(&path).unwrap().len(),
            entry["bytes"].as_u64().unwrap()
        );
        original.push((FileFingerprint::read(&path).unwrap(), hash));
        paths.push(path);
    }
    let run = root.join("run");
    std::fs::create_dir(&run).expect("fresh run directory only");
    let (engine, output) = player();
    let mut running = playback(output);
    let (entered, seen) = std::sync::mpsc::sync_channel(1);
    let (resume, held) = std::sync::mpsc::sync_channel(1);
    let mut first = true;
    let loader = Loader::with_analysis_hook(engine.cmd.performance().clone(), move |_| {
        if first {
            first = false;
            entered.send(()).unwrap();
            held.recv_timeout(Duration::from_secs(10)).unwrap();
        }
    })
    .unwrap();
    let metadata_hold = Arc::new(AtomicBool::new(false));
    let hold = metadata_hold.clone();
    let (release_metadata, blocked_metadata) = std::sync::mpsc::sync_channel(1);
    let mut app = App::with_loader(engine, Theme::default(), Some(loader));
    app.library_metadata =
        library_metadata::Metadata::with_hook(run.join("catalog.json"), move || {
            if hold.swap(false, Ordering::AcqRel) {
                blocked_metadata
                    .recv_timeout(Duration::from_secs(20))
                    .unwrap();
            }
        });
    app.library_metadata
        .set_performance(app.engine.cmd.performance().clone());
    app.library_initialized = false;
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let mut gui = LongGui {
        app,
        ctx,
        nodes: vec![],
        time: 0.0,
    };
    gui.wait(|gui| !gui.app.library_metadata.active());
    assert!(gui
        .app
        .library_scan
        .start(vec![sources.clone()], gui.app.library.clone()));
    gui.wait(|gui| !gui.app.library_scan.active() && !gui.app.library_metadata.active());
    // Restrict the captured crate to these exact sources without copying rows.
    // This fixture-only filter is the common filename prefix supplied by launcher.
    gui.app.lib_filter = "long".into();
    gui.app.refresh_library_view();
    assert_eq!(gui.app.library_view.indices.len(), 3);
    let mut preparation = Preparation::default();
    preparation.cue = 1.25;
    preparation.hotcues[0] = Some(2.5);
    preparation.grid = Some(crate::engine::beatgrid::Grid::new(0.125, 137.0).unwrap());
    let item = gui
        .app
        .library
        .iter()
        .find(|item| item.source == LibSource::File(paths[0].clone()))
        .unwrap()
        .clone();
    let mut manual = item.stored_metadata();
    manual.bpm = Bpm::new(137.0, Origin::User);
    gui.app
        .library_metadata
        .capture(super::super::super::library_store::Capture {
            source: item.source,
            fingerprint: item.fingerprint,
            metadata: manual,
            preparation: Some(preparation),
            played: None,
        });
    gui.wait(|gui| !gui.app.library_metadata.active());
    let ids: Vec<_> = paths
        .iter()
        .map(|path| {
            gui.app
                .library_metadata
                .catalog
                .track(&LibSource::File(path.clone()))
                .unwrap()
                .id
                .clone()
        })
        .collect();
    let creative_revision = gui.app.engine.project.revision();
    gui.app.library_analysis.open = true;
    gui.frame(vec![]);
    gui.frame(vec![]);
    gui.click("Analyze filtered crate");
    gui.wait(|gui| matches!(gui.app.library_analysis.step, Step::Decoding(_)));
    seen.recv_timeout(Duration::from_secs(5)).unwrap();
    // Let the real decoder advance into the long source. Delay only the next
    // metadata job so a heavily descheduled GUI can still cancel before claim.
    let first_token = match &gui.app.library_analysis.step {
        Step::Decoding(token) => token.clone(),
        _ => unreachable!(),
    };
    metadata_hold.store(true, Ordering::Release);
    resume.send(()).unwrap();
    gui.wait(|_| {
        matches!(
            first_token.progress().stage,
            Stage::Decoding | Stage::Tempo | Stage::Waveform | Stage::Ready
        )
    });
    // A descheduled GUI may observe Ready rather than the intermediate stage;
    // the metadata hook still makes the pre-publication cancellation precise.
    let cancelled_stage = format!("{:?}", first_token.progress().stage);
    gui.click("Cancel analysis queue");
    release_metadata.send(()).unwrap();
    gui.wait(|gui| !gui.app.library_analysis.busy() && !gui.app.library_metadata.active());
    assert!(gui.app.library_analysis.message.contains("cancelled"));
    assert!(paths.iter().all(|path| gui
        .app
        .library_metadata
        .catalog
        .version(&LibSource::File(path.clone()), FileFingerprint::read(path))
        .unwrap()
        .analysis
        .is_none()));
    gui.click("Analyze filtered crate");
    gui.wait(|gui| !gui.app.library_analysis.busy() && !gui.app.library_metadata.active());
    assert!(
        gui.app.library_analysis.message.contains("3 saved"),
        "{}",
        gui.app.library_analysis.message
    );
    let records:Vec<_>=paths.iter().enumerate().map(|(index,path)|{
        let source=LibSource::File(path.clone());let track=gui.app.library_metadata.catalog.track(&source).unwrap();assert_eq!(track.id,ids[index]);
        let version=gui.app.library_metadata.catalog.version(&source,Some(original[index].0)).unwrap();let record=version.analysis.as_ref().unwrap();assert!(record.contains(Fields::ALL));
        assert!((record.duration.as_ref().unwrap().value-180.0).abs()<=0.05,"unexpected long source duration");
        if index==0 {assert_eq!(version.preparation,preparation);assert_eq!(version.metadata.bpm,Bpm::new(137.0,Origin::User));}
        serde_json::json!({"file":path.file_name().unwrap().to_string_lossy(),"sha256":original[index].1,"track":track.id,"analysis":record,"decoded_frames":record.waveform.as_ref().unwrap().value.frames,"sample_rate":record.waveform.as_ref().unwrap().value.sample_rate,"channels":record.waveform.as_ref().unwrap().value.channels})
    }).collect();
    assert_eq!(
        gui.app.engine.project.revision(),
        creative_revision,
        "analysis changed creative engine controls"
    );
    running.stop.store(true, Ordering::Release);
    let callback = running.thread.take().unwrap().join().unwrap();
    drop(gui);
    wait_store_closed(&run.join("catalog.json"));
    // A new owner reopens the real saved catalog and cache. Explicit decode is
    // forbidden by the hook, proving cache reuse instead of silent recompute.
    let (engine, _rt) = Engine::headless_for_test(48_000, 256);
    let loader = Loader::with_analysis_hook(engine.cmd.performance().clone(), |_| {
        panic!("cached reopen attempted source decode")
    })
    .unwrap();
    let mut app = App::with_loader(engine, Theme::default(), Some(loader));
    app.start_library_store(run.join("catalog.json"));
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let mut reopened = LongGui {
        app,
        ctx,
        nodes: vec![],
        time: 0.0,
    };
    reopened.wait(|gui| !gui.app.library_metadata.active());
    reopened.app.lib_filter = "long".into();
    reopened.app.refresh_library_view();
    reopened.app.library_analysis.open = true;
    reopened.frame(vec![]);
    reopened.frame(vec![]);
    reopened.click("Analyze filtered crate");
    reopened.wait(|gui| !gui.app.library_analysis.busy() && !gui.app.library_metadata.active());
    assert!(reopened
        .app
        .library_analysis
        .message
        .contains("3 reused from cache"));
    reopened.click("Inspect selected cache");
    reopened.wait(|gui| !gui.app.library_analysis.busy() && !gui.app.library_metadata.active());
    assert!(reopened
        .app
        .library_analysis
        .preview
        .as_ref()
        .unwrap()
        .outcome
        .as_ref()
        .unwrap()
        .waveform
        .is_some());
    for (path, (fingerprint, hash)) in paths.iter().zip(&original) {
        assert_eq!(FileFingerprint::read(path), Some(*fingerprint));
        assert_eq!(&digest(path), hash);
    }
    let report = serde_json::json!({"schema":1,"status":"pass","embedded_manifest":serde_json::from_str::<serde_json::Value>(crate::licenses::MANIFEST).unwrap(),"fixture":"actual App captured queue under production four-channel callback", "cancelled_before_first_save":true,"cancelled_stage":cancelled_stage,
        "restarted_saved":3,"reopened_cache_hits":3,"manual_preparation_preserved":true,"creative_revision_unchanged":true,"creative_revision":creative_revision,"source_hashes_unchanged":true,"sources":records,"callback":callback,
        "physical_hardware_test":false,"deadline_claim":false});
    use std::io::Write;
    let mut output = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(run.join("report.json"))
        .unwrap();
    output
        .write_all(&serde_json::to_vec_pretty(&report).unwrap())
        .unwrap();
    output.sync_all().unwrap();
}
