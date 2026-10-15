use super::*;
use crate::engine::audio::routing::speakers::tests::fixture;
use sha2::{Digest, Sha256};
use std::path::Path;

fn independent(path: &Path, tools: &serde_json::Value) -> Vec<f32> {
    let result = std::process::Command::new(tools["ffmpeg"]["path"].as_str().unwrap())
        .args(["-v", "error", "-i"])
        .arg(path)
        .args(["-f", "f32le", "-acodec", "pcm_f32le", "pipe:1"])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    result
        .stdout
        .chunks_exact(4)
        .map(|v| f32::from_le_bytes(v.try_into().unwrap()))
        .collect()
}

#[test]
fn native_declared_speaker_controls_export_and_cli_reopen_share_actual_ordered_graph() {
    let files = Files::new();
    let (engine, rt) = fixture(48000);
    let input = rt.tracks[0].clips[0]
        .audio
        .as_ref()
        .unwrap()
        .data
        .iter()
        .flat_map(|v| v.to_le_bytes())
        .collect::<Vec<_>>();
    let input_sha256 = format!("{:x}", Sha256::digest(&input));
    assert_eq!(
        input_sha256,
        "510c775f101b3b688dc304e676820ec3a5887dc64dbd2992ca517b7bfcfa3da1"
    );
    let mut gui = Gui::with_renderer(&files, 6, engine, rt);
    let revision = gui.app.engine.project.revision();
    gui.click("Review export source");
    gui.wait();
    assert_eq!(gui.app.audio_delivery.request.options.channels, 3);
    assert_eq!(
        gui.app.audio_delivery.plan.as_ref().unwrap().outputs[0]
            .contract
            .wave_mask,
        Some(0xB)
    );
    gui.click("Mono");
    assert_eq!(gui.app.audio_delivery.request.options.channels, 1);
    gui.click("2.1 (FL FR LFE)");
    assert_eq!(gui.app.audio_delivery.request.options.channels, 3);
    gui.app.audio_delivery.request.end = 0.01;
    gui.app.audio_delivery.request.tail = 0.;
    gui.app.audio_delivery.request.options.format = Format::Rf64Float32;
    let folder = files.0.join("wide-master");
    gui.app.audio_delivery.path = folder.to_string_lossy().into_owned();
    gui.click("Render master audio");
    gui.wait();
    assert!(
        gui.app.audio_delivery.message.starts_with("Saved"),
        "{}",
        gui.app.audio_delivery.message
    );
    assert_eq!(gui.app.engine.project.revision(), revision);
    assert!(!gui.callback.renderer_for_test().playing);
    let metadata: serde_json::Value =
        serde_json::from_slice(&std::fs::read(folder.join("export.json")).unwrap()).unwrap();
    assert_eq!(metadata["wave_mask"], 11);
    assert_eq!(
        metadata["speakers"],
        serde_json::json!(["front_left", "front_right", "low_frequency"])
    );
    assert_eq!(
        metadata["physical_source_channels"],
        serde_json::json!([3, 0, 5])
    );
    let tools = data::measurement::preflight(&AtomicBool::new(false)).unwrap();
    let pcm = independent(&folder.join("master.wav"), &tools);
    assert_eq!(pcm.len(), 480 * 3);
    let mut residual = 0_f32;
    for (frame, values) in pcm.chunks_exact(3).enumerate() {
        for (channel, actual) in values.iter().enumerate() {
            let expected = if frame == 8 + 13 * channel {
                (((channel + 1) as f32 / 32.) * 0.25).tanh()
            } else {
                0.
            };
            residual = residual.max((actual - expected).abs());
        }
    }
    assert!(residual < 2e-6, "{residual}");
    let captured = gui.capture();
    let document = project::Document {
        engine: captured.state,
        view: project::UiState::default(),
        mapping_schema: project::FACTORY_MAPPING_SCHEMA,
    };
    document.validate().unwrap();
    let native = files.0.join("speakers.omat");
    crate::project_file::save(
        &native,
        &crate::project_file::Bundle {
            state: document,
            media: captured.media,
        },
        crate::project_file::Overwrite::Never,
        &Default::default(),
        &AtomicBool::new(false),
    )
    .unwrap();
    let path = files.0.join("wide-plan.json");
    assert!(data::cli::run(&[
        "delivery-plan".into(),
        native.to_string_lossy().into_owned(),
        path.to_string_lossy().into_owned()
    ])
    .unwrap());
    let mut recipe = data::batch::Recipe::load(&path, &AtomicBool::new(false)).unwrap();
    assert_eq!(recipe.export.options.channels, 3);
    assert_eq!(recipe.export.output_alias, Some(1));
    recipe.export.end = 0.01;
    recipe.export.tail = 0.;
    recipe.items.truncate(1);
    std::fs::remove_file(&path).unwrap();
    recipe.save(&path).unwrap();
    let batch = files.0.join("wide-cli");
    assert!(data::cli::run(&[
        "delivery-batch".into(),
        native.to_string_lossy().into_owned(),
        path.to_string_lossy().into_owned(),
        batch.to_string_lossy().into_owned()
    ])
    .unwrap());
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(batch.join("batch.json")).unwrap()).unwrap();
    let audio = std::fs::read_dir(&batch)
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| p.extension().is_some_and(|e| e == "wav"))
        .unwrap();
    let raw = independent(&audio, &tools);
    assert_eq!(raw.len(), 480 * 3);
    let mut post_residual = 0_f32;
    for (frame, values) in raw.chunks_exact(3).enumerate() {
        for (channel, actual) in values.iter().enumerate() {
            let expected = if frame == 8 + 13 * channel {
                (channel + 1) as f32 / 32.
            } else {
                0.
            };
            post_residual = post_residual.max((actual - expected).abs());
        }
    }
    assert!(post_residual < 2e-6, "{post_residual}");
    data::fixture::retain(
        "native-declared-speaker-delivery-ui-cli",
        serde_json::json!({"status":"passed","input_sha256":input_sha256,"input_bytes":input.len(),"master_file_sha256":format!("{:x}",Sha256::digest(std::fs::read(folder.join("master.wav")).unwrap())),"selected_file_sha256":format!("{:x}",Sha256::digest(std::fs::read(&audio).unwrap())),"layout":"2.1","native_channels":3,"wave_mask":11,"physical_channels":[3,0,5],"frames":480,"master_analytic_residual":residual,"post_device_analytic_residual":post_residual,"live_state_unchanged":true,"native_project_recipe_reopened":true,"export":metadata,"batch":manifest,"tools":tools}),
    );
}
