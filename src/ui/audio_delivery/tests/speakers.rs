use super::*;
use crate::engine::{
    audio::routing::speakers::{
        tests::{fixture, fixture_layout},
        Layout,
    },
    Engine, RtEngine,
};
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

struct Expected {
    case: &'static str,
    test: &'static str,
    mask: u32,
    physical: &'static [u16],
    speakers: &'static [&'static str],
    input_sha256: &'static str,
    independent_layout: &'static str,
}

#[test]
fn native_declared_speaker_controls_export_and_cli_reopen_share_actual_ordered_graph() {
    let (engine, rt) = fixture(48000);
    verify(Expected {
        case:"native-declared-speaker-delivery-ui-cli",
        test:"ui::audio_delivery::tests::speakers::native_declared_speaker_controls_export_and_cli_reopen_share_actual_ordered_graph",
        mask:0xB,physical:&[3,0,5],speakers:&["front_left","front_right","low_frequency"],
        input_sha256:"510c775f101b3b688dc304e676820ec3a5887dc64dbd2992ca517b7bfcfa3da1",
        independent_layout:"2.1",
    },Layout::StereoLfe,engine,Box::new(rt));
}

#[test]
fn surround_51_native_encoded_graph_ui_cli_reopen_retains_actual_speaker_files() {
    let (engine, rt) = fixture_layout(48000, Layout::Surround51Side);
    verify(Expected {
        case:"native-declared-surround-51-delivery-ui-cli",
        test:"ui::audio_delivery::tests::speakers::surround_51_native_encoded_graph_ui_cli_reopen_retains_actual_speaker_files",
        mask:0x60F,physical:&[3,0,5,1,8,7],speakers:&["front_left","front_right","front_center","low_frequency","side_left","side_right"],
        input_sha256:"6a913031fad2d92cd16f45427932415437ee8724dbe2e54034042030d4d024ed",
        independent_layout:"5.1(side)",
    },Layout::Surround51Side,engine,rt);
}

fn verify(expected: Expected, layout: Layout, engine: Engine, rt: Box<RtEngine>) {
    let files = Files::new();
    let retained = data::fixture::producer(expected.case);
    let root = retained.as_ref().unwrap_or(&files.0);
    let input = rt.tracks[0].clips[0]
        .audio
        .as_ref()
        .unwrap()
        .data
        .iter()
        .flat_map(|v| v.to_le_bytes())
        .collect::<Vec<_>>();
    let input_sha256 = format!("{:x}", Sha256::digest(&input));
    assert_eq!(input_sha256, expected.input_sha256);
    let input_path = root.join("input.f32");
    std::fs::write(&input_path, &input).unwrap();
    let channels = expected.speakers.len();
    let physical_width = usize::from(*expected.physical.iter().max().unwrap()) + 1;
    let mut gui = Gui::with_renderer(&files, physical_width, engine, *rt);
    let revision = gui.app.engine.project.revision();
    gui.click("Review export source");
    gui.wait();
    assert_eq!(
        gui.app.audio_delivery.request.options.channels,
        channels as u16
    );
    assert_eq!(
        gui.app.audio_delivery.plan.as_ref().unwrap().outputs[0]
            .contract
            .wave_mask,
        Some(expected.mask)
    );
    gui.click("Mono");
    assert_eq!(gui.app.audio_delivery.request.options.channels, 1);
    gui.click(layout.label());
    assert_eq!(
        gui.app.audio_delivery.request.options.channels,
        channels as u16
    );
    gui.app.audio_delivery.request.end = 0.01;
    gui.app.audio_delivery.request.tail = 0.;
    gui.app.audio_delivery.request.options.format = Format::Rf64Float32;
    let folder = root.join("wide-master");
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
    assert_eq!(metadata["wave_mask"], expected.mask);
    assert_eq!(metadata["speakers"], serde_json::json!(expected.speakers));
    assert_eq!(
        metadata["physical_source_channels"],
        serde_json::json!(expected.physical)
    );
    let tools = data::measurement::preflight(&AtomicBool::new(false)).unwrap();
    let encoded = std::fs::read(folder.join("master.wav")).unwrap();
    assert_eq!(&encoded[..4], b"RF64");
    let mut offset = 12;
    while &encoded[offset..offset + 4] != b"fmt " {
        let count =
            u32::from_le_bytes(encoded[offset + 4..offset + 8].try_into().unwrap()) as usize;
        offset += 8 + count + (count & 1);
    }
    let fmt = &encoded[offset + 8..offset + 48];
    assert_eq!(&encoded[offset + 4..offset + 8], &40_u32.to_le_bytes());
    assert_eq!(&fmt[..2], &0xfffe_u16.to_le_bytes());
    assert_eq!(&fmt[2..4], &(channels as u16).to_le_bytes());
    assert_eq!(&fmt[16..18], &22_u16.to_le_bytes());
    assert_eq!(&fmt[18..20], &32_u16.to_le_bytes());
    assert_eq!(&fmt[20..24], &expected.mask.to_le_bytes());
    assert_eq!(&fmt[24..28], &3_u32.to_le_bytes());
    assert_eq!(
        &fmt[28..],
        &[0, 0, 0x10, 0, 0x80, 0, 0, 0xaa, 0, 0x38, 0x9b, 0x71]
    );
    let probe = std::process::Command::new(tools["ffprobe"]["path"].as_str().unwrap())
        .args([
            "-v",
            "error",
            "-show_entries",
            "stream=channels,channel_layout",
            "-of",
            "json",
        ])
        .arg(folder.join("master.wav"))
        .output()
        .unwrap();
    assert!(
        probe.status.success(),
        "{}",
        String::from_utf8_lossy(&probe.stderr)
    );
    let independent_probe: serde_json::Value = serde_json::from_slice(&probe.stdout).unwrap();
    assert_eq!(independent_probe["streams"][0]["channels"], channels);
    assert_eq!(
        independent_probe["streams"][0]["channel_layout"],
        expected.independent_layout
    );
    let pcm = independent(&folder.join("master.wav"), &tools);
    assert_eq!(pcm.len(), 480 * channels);
    let mut residual = 0_f32;
    for (frame, values) in pcm.chunks_exact(channels).enumerate() {
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
    let native = root.join("speakers.omat");
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
    let path = root.join("wide-plan.json");
    assert!(data::cli::run(&[
        "delivery-plan".into(),
        native.to_string_lossy().into_owned(),
        path.to_string_lossy().into_owned()
    ])
    .unwrap());
    let mut recipe = data::batch::Recipe::load(&path, &AtomicBool::new(false)).unwrap();
    assert_eq!(recipe.export.options.channels, channels as u16);
    assert_eq!(recipe.export.output_alias, Some(1));
    recipe.export.end = 0.01;
    recipe.export.tail = 0.;
    recipe.items.truncate(1);
    std::fs::remove_file(&path).unwrap();
    recipe.save(&path).unwrap();
    let batch = root.join("wide-cli");
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
    assert_eq!(raw.len(), 480 * channels);
    let mut post_residual = 0_f32;
    for (frame, values) in raw.chunks_exact(channels).enumerate() {
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
    let producer_files = if retained.is_some() {
        [
            &input_path,
            &native,
            &path,
            &folder.join("master.wav"),
            &folder.join("export.json"),
            &audio,
            &batch.join("batch.json"),
        ]
        .into_iter()
        .map(|path| data::fixture::file(path))
        .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    data::fixture::retain(
        expected.case,
        serde_json::json!({"status":"passed","test_identity":expected.test,"current_bytes_retained":retained.is_some(),"producer_files":producer_files,"input_sha256":input_sha256,"input_bytes":input.len(),"master_file_sha256":format!("{:x}",Sha256::digest(&encoded)),"selected_file_sha256":format!("{:x}",Sha256::digest(std::fs::read(&audio).unwrap())),"layout":layout.label(),"native_channels":channels,"wave_mask":expected.mask,"physical_channels":expected.physical,"frames":480,"master_analytic_residual":residual,"post_device_analytic_residual":post_residual,"live_state_unchanged":true,"native_project_recipe_reopened":true,"export":metadata,"batch":manifest,"independent_probe":independent_probe,"tools":tools}),
    );
}
