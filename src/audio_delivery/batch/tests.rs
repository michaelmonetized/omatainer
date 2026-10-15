use super::*;
use crate::engine::{dsp::Sample, ClipKind, Command, Engine, RtEngine};
use std::sync::atomic::{AtomicBool, Ordering};
struct Files(PathBuf);
impl Files {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!(
            "delivery-batch-{}",
            crate::sampler_bank::BankId::new().unwrap()
        ));
        std::fs::create_dir(&p).unwrap();
        Self(p)
    }
}
impl Drop for Files {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn fixture() -> (Engine, RtEngine) {
    let (engine, mut rt) = Engine::headless_for_test(48000, 256);
    rt.apply(Command::Stop);
    rt.quantize = false;
    rt.master = 0.25;
    for track in &mut rt.tracks {
        track
            .clips
            .iter_mut()
            .for_each(|c| *c = crate::engine::Clip::empty());
        track.fx.slots.clear();
        track.surface.sends = [0.; 2];
        track.surface.crossfader = 0;
    }
    for fx in &mut rt.scene_fx {
        fx.slots.clear();
    }
    for slot in 0..2 {
        let sample = Arc::new(Sample {
            spectrum: None,
            name: format!("Independent source{slot}"),
            sr: 48000,
            ch: 2,
            data: (0..48000)
                .flat_map(|i| {
                    let value =
                        (std::f32::consts::TAU * (if slot == 0 { 440. } else { 731. }) * i as f32
                            / 48000.)
                            .sin()
                            * 0.05;
                    [value, value * (if slot == 0 { 0.7 } else { -0.4 })]
                })
                .collect(),
            peaks: Vec::new().into(),
            bpm: 120.,
            path: String::new(),
        });
        let region = crate::engine::audio_clip::Region::full(&sample, 120.)
            .unwrap()
            .prepare(&sample)
            .unwrap();
        let track = &mut rt.tracks[slot];
        track.kind = 4;
        track.gain = 0.5;
        track.scene_bus = 0;
        track.eq.low_g = 1.;
        track.eq.mid_g = 1.;
        track.eq.high_g = 1.;
        track.clips[0].kind = ClipKind::Audio;
        track.clips[0].audio = Some(sample);
        track.clips[0].bars = (region.duration_beats / 4.) as f32;
        track.clips[0].audio_region = Some(region);
    }
    (engine, rt)
}
fn capture(engine: &Engine, rt: &mut RtEngine) -> Captured {
    let h = engine.project.clone();
    let task = std::thread::spawn(move || h.capture(&AtomicBool::new(false)).unwrap());
    let end = std::time::Instant::now() + std::time::Duration::from_secs(15);
    while !task.is_finished() {
        rt.process(&mut []);
        assert!(std::time::Instant::now() < end);
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    task.join().unwrap()
}
fn recipe(c: &Captured) -> Recipe {
    let request = Export {
        end: 0.5,
        tail: 0.1,
        ..Default::default()
    };
    let mut recipe = Recipe::aligned(c, request, Config::default()).unwrap();
    let layout = c.state.session.as_ref().unwrap();
    recipe.items = (0..2)
        .map(|i| Item {
            id: i as u64 + 1,
            name: format!("track-{}", i + 1),
            kind: Kind::Stem,
            selection: Selection {
                namespace: layout.namespace,
                targets: vec![Target::Track(layout.tracks[i].id)],
                policy: Policy::PostDevice,
            },
        })
        .collect();
    recipe.next_id = 4;
    recipe.items.push(Item {
        id: 3,
        name: "explicit-combined-version".into(),
        kind: Kind::Version,
        selection: Selection {
            namespace: layout.namespace,
            targets: layout.tracks[..2]
                .iter()
                .map(|t| Target::Track(t.id))
                .collect(),
            policy: Policy::PostDevice,
        },
    });
    recipe
}
fn decode(path: &Path) -> Vec<f32> {
    crate::engine::decode::decode_audio(path)
        .unwrap()
        .sample
        .data
        .clone()
}
fn retain(case: &str, value: &serde_json::Value) {
    if let Some(root) = std::env::var_os("OMATAINER_DELIVERY_RECEIPTS") {
        let root = PathBuf::from(root);
        assert!(root.starts_with("/home"));
        std::fs::create_dir_all(&root).unwrap();
        let mut v = value.clone();
        v["schema"] = serde_json::json!(1);
        v["case"] = serde_json::json!(case);
        v["source_revision"] = serde_json::json!(std::env::var("OMATAINER_DELIVERY_REVISION").ok());
        v["boundaries"] = serde_json::json!([
            "headless native graph/encoded files only",
            "no device/listening/desktop/physical timing"
        ]);
        std::fs::write(
            root.join(format!("{case}.json")),
            serde_json::to_vec_pretty(&v).unwrap(),
        )
        .unwrap();
    }
}

#[test]
fn aligned_encoded_stems_null_sum_explicit_version_and_normal_graph_oracle_after_reopen() {
    let files = Files::new();
    let (engine, mut rt) = fixture();
    let captured = capture(&engine, &mut rt);
    let before = serde_json::to_value(&captured.state).unwrap();
    let recipe = recipe(&captured);
    let path = files.0.join("recipe.json");
    recipe.save(&path).unwrap();
    assert!(recipe.save(&path).is_err());
    let loaded = Recipe::load(&path, &AtomicBool::new(false)).unwrap();
    assert_eq!(
        serde_json::to_value(&recipe).unwrap(),
        serde_json::to_value(&loaded).unwrap()
    );
    let review = loaded.review(&captured).unwrap();
    let permit = engine.cmd.performance().optional_work().unwrap();
    let folder = files.0.join("aligned");
    let progress = Progress::default();
    let outcome = run(
        copy(&captured),
        &review,
        &folder,
        &permit,
        &Default::default(),
        &progress,
    )
    .unwrap();
    assert_eq!(outcome.files, 3);
    assert_eq!(outcome.frames, 28800);
    assert_eq!(progress.lock().unwrap().phase, "Saved");
    let a = decode(&folder.join("track-1.wav"));
    let b = decode(&folder.join("track-2.wav"));
    let version = decode(&folder.join("explicit-combined-version.wav"));
    assert_eq!(a.len(), 57600);
    let residual = a
        .iter()
        .zip(&b)
        .zip(&version)
        .map(|((&a, &b), &v)| (a + b - v).abs())
        .fold(0., f32::max);
    assert!(residual < 2e-7, "sum residual{residual}");
    let raw = files.0.join("normal.f32");
    super::super::render_selected(
        copy(&captured),
        &loaded.export,
        None,
        &raw,
        loaded.export.frames().unwrap(),
        &permit.cancel(),
        &Default::default(),
    )
    .unwrap();
    let normal = std::fs::read(&raw).unwrap();
    let normal_residual = normal
        .chunks_exact(4)
        .zip(&version)
        .map(|(b, &v)| (f32::from_le_bytes(b.try_into().unwrap()) - (v * 0.25).tanh()).abs())
        .fold(0., f32::max);
    assert!(
        normal_residual < 2e-7,
        "actual normal graph oracle residual{normal_residual}"
    );
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(folder.join("batch.json")).unwrap()).unwrap();
    assert_eq!(manifest["status"], "passed");
    for item in &loaded.items {
        let metadata: serde_json::Value = serde_json::from_slice(
            &std::fs::read(folder.join(format!("{}.json", item.name))).unwrap(),
        )
        .unwrap();
        assert_eq!(metadata["frames"], 28800);
        assert_eq!(metadata["qc"]["frames"], 28800);
        assert_eq!(metadata["metadata"]["time_reference"], 0);
        assert_eq!(
            metadata["selection"],
            serde_json::to_value(&item.selection).unwrap()
        );
    }
    let after = capture(&engine, &mut rt);
    assert_eq!(before, serde_json::to_value(&after.state).unwrap());
    assert!(!rt.playing);
    retain(
        "aligned-native-encoded-null-and-master-oracle",
        &serde_json::json!({"status":"passed","passes":3,"frames":28800,"linear_residual":residual,"normal_graph_residual":normal_residual,"recipe_reopen":true,"live_state_unchanged":true,"manifest":manifest}),
    );
}

#[test]
fn batch_stale_missing_ids_collision_cancel_and_failed_qc_never_publish_partial_folder() {
    let files = Files::new();
    let (engine, mut rt) = fixture();
    let c = capture(&engine, &mut rt);
    let recipe = recipe(&c);
    let review = recipe.review(&c).unwrap();
    let permit = engine.cmd.performance().optional_work().unwrap();
    let occupied = files.0.join("occupied");
    std::fs::create_dir(&occupied).unwrap();
    std::fs::write(occupied.join("keep"), b"preserve").unwrap();
    assert!(run(
        copy(&c),
        &review,
        &occupied,
        &permit,
        &Default::default(),
        &Progress::default()
    )
    .is_err());
    assert_eq!(std::fs::read(occupied.join("keep")).unwrap(), b"preserve");
    let mut stale = copy(&c);
    stale.revision += 1;
    assert!(run(
        stale,
        &review,
        &files.0.join("stale"),
        &permit,
        &Default::default(),
        &Progress::default()
    )
    .unwrap_err()
    .contains("changed since"));
    let mut duplicate = recipe.clone();
    duplicate.items[1].name = duplicate.items[0].name.to_uppercase();
    assert!(duplicate.review(&c).is_err());
    let mut missing = recipe.clone();
    missing.items[0].selection.targets = vec![Target::Track(crate::engine::session::Id(u64::MAX))];
    assert!(missing.review(&c).is_err());
    let mut blocked = recipe.clone();
    blocked.config.targets.policy = super::super::measurement::Policy::Block;
    blocked.config.targets.integrated_lufs = Some(-10.);
    let blocked_review = blocked.review(&c).unwrap();
    let blocked_folder = files.0.join("blocked");
    let e = run(
        copy(&c),
        &blocked_review,
        &blocked_folder,
        &permit,
        &Default::default(),
        &Progress::default(),
    )
    .unwrap_err();
    assert!(e.contains("QC blocked"), "{e}");
    assert!(!blocked_folder.exists());
    let receipt = std::fs::read_dir(&files.0)
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| {
            p.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with(".delivery-failure-")
        })
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&std::fs::read(receipt).unwrap()).unwrap();
    assert_eq!(json["cases"][0]["status"], "failed");
    assert_eq!(json["cases"][1]["status"], "skipped");
    drop(permit);
    let permit = engine.cmd.performance().optional_work().unwrap();
    permit.cancel().store(true, Ordering::Release);
    assert!(run(
        c,
        &review,
        &files.0.join("cancelled"),
        &permit,
        &Default::default(),
        &Progress::default()
    )
    .unwrap_err()
    .contains("cancelled"));
    assert!(!files.0.join("cancelled").exists());
    assert!(!files.0.join("stale").exists());
    assert!(std::fs::read_dir(&files.0).unwrap().all(|p| !p
        .unwrap()
        .file_name()
        .to_string_lossy()
        .contains("stage")));
    retain(
        "batch-atomic-refusals",
        &serde_json::json!({"status":"passed","collision_preserved":true,"stale_rejected":true,"deleted_identity_rejected":true,"case_collision_rejected":true,"cancel_no_final":true,"qc_failure_receipt":json}),
    );
}

#[test]
fn batch_cancel_during_second_pass_keeps_only_failure_receipt_and_no_final_audio() {
    let files = Files::new();
    let (engine, mut rt) = fixture();
    let c = capture(&engine, &mut rt);
    let recipe = recipe(&c);
    let review = recipe.review(&c).unwrap();
    let permit = engine.cmd.performance().optional_work().unwrap();
    let cancel = permit.cancel();
    let progress = Progress::default();
    let observed = progress.clone();
    let destination = files.0.join("cancel-mid-batch");
    let path = destination.clone();
    let worker =
        std::thread::spawn(move || run(c, &review, &path, &permit, &Default::default(), &progress));
    let end = std::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        if observed.lock().unwrap().item >= 2 {
            cancel.store(true, Ordering::Release);
            break;
        }
        assert!(
            !worker.is_finished(),
            "batch ended before second-pass cancellation"
        );
        assert!(std::time::Instant::now() < end);
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    let error = worker.join().unwrap().unwrap_err();
    assert!(error.contains("cancelled"), "{error}");
    assert!(!destination.exists());
    let receipt = std::fs::read_dir(&files.0)
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| {
            p.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with(".delivery-failure-")
        })
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&std::fs::read(receipt).unwrap()).unwrap();
    assert_eq!(json["cases"][0]["status"], "passed");
    assert_eq!(json["cases"][1]["status"], "failed");
    assert_eq!(json["cases"][2]["status"], "skipped");
    assert_eq!(std::fs::read_dir(&files.0).unwrap().count(), 1);
    retain(
        "batch-second-pass-cancel",
        &serde_json::json!({"status":"passed","no_final_folder":true,"receipt":json}),
    );
}

#[test]
fn recipe_fifo_refuses_without_writer_and_regular_recipe_preserves_bytes_with_cancel() {
    use std::os::unix::{ffi::OsStrExt, fs::FileTypeExt};
    let files = Files::new();
    let fifo = files.0.join("recipe.fifo");
    let name = std::ffi::CString::new(fifo.as_os_str().as_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
    assert!(Recipe::load(&fifo, &AtomicBool::new(false))
        .unwrap_err()
        .contains("regular file"));
    assert!(std::fs::symlink_metadata(&fifo)
        .unwrap()
        .file_type()
        .is_fifo());
    let (engine, mut rt) = fixture();
    let captured = capture(&engine, &mut rt);
    let recipe = recipe(&captured);
    let path = files.0.join("regular.json");
    recipe.save(&path).unwrap();
    let before = std::fs::read(&path).unwrap();
    let loaded = Recipe::load(&path, &AtomicBool::new(false)).unwrap();
    assert_eq!(
        serde_json::to_value(&loaded).unwrap(),
        serde_json::to_value(&recipe).unwrap()
    );
    assert!(Recipe::load(&path, &AtomicBool::new(true))
        .unwrap_err()
        .contains("cancelled"));
    assert_eq!(before, std::fs::read(&path).unwrap());
}
