use super::*;
use crate::engine::{project, test_alloc, ClipKind};

fn fixture(rate: u32, sends: [f32; 2]) -> (Engine, Box<RtEngine>) {
    let (engine, mut rt) = Engine::headless_for_test(rate, 256);
    rt.apply(Command::Stop);
    for track in &mut rt.tracks {
        track
            .clips
            .iter_mut()
            .for_each(|clip| *clip = crate::engine::Clip::empty());
        track.fx.slots.clear();
    }
    for rack in &mut rt.scene_fx {
        rack.slots.clear();
    }
    rt.quantize = false;
    rt.bpm = 120.;
    rt.master = 0.25;
    rt.xfader = 0.25;
    let mut data = vec![0.; 8192 * 2];
    for (i, left, right) in [(0, 0.11, -0.07), (2048, -0.05, 0.13), (8000, 0.09, 0.03)] {
        data[i * 2] = left;
        data[i * 2 + 1] = right;
    }
    let sample = Arc::new(Sample {
        spectrum: None,
        name: "Unequal impulses".into(),
        sr: rate,
        ch: 2,
        data,
        peaks: Vec::new().into(),
        bpm: 120.,
        path: String::new(),
    });
    let region = crate::engine::audio_clip::Region::full(&sample, 120.)
        .unwrap()
        .prepare(&sample)
        .unwrap();
    let track = &mut rt.tracks[0];
    track.kind = 4;
    track.gain = 1.;
    track.eq.low_g = 1.;
    track.eq.mid_g = 1.;
    track.eq.high_g = 1.;
    track.clips[0].audio_region = Some(region);
    track.clips[0].kind = ClipKind::Audio;
    track.clips[0].audio = Some(sample);
    track.clips[0].bars = region.duration_beats as f32 / 4.;
    rt.apply(Command::TrackSend {
        track: 0,
        send: 0,
        value: sends[0],
    });
    rt.apply(Command::TrackSend {
        track: 0,
        send: 1,
        value: sends[1],
    });
    rt.apply(Command::TrackCrossfader {
        track: 0,
        assignment: 1,
    });
    (engine, Box::new(rt))
}
fn pcm(rt: &mut RtEngine, rate: u32, block: usize) -> Vec<f32> {
    rt.apply(Command::LaunchScene { scene: 0 });
    let mut output = vec![0.; (8192 + rate as usize) * 2];
    let (body, tail) = output.split_at_mut(8192 * 2);
    for chunk in body.chunks_mut(block * 2) {
        rt.process(chunk);
    }
    rt.stop_export_sources();
    for chunk in tail.chunks_mut(block * 2) {
        rt.process(chunk);
    }
    output
}

#[test]
fn fixed_send_fresh_process_renderer() {
    let Some(path) = std::env::var_os("OMATAINER_FIXED_SEND_ARCHIVE") else {
        return;
    };
    let bundle = crate::project_file::load::<project::State>(
        Path::new(&path),
        &Default::default(),
        &AtomicBool::new(false),
    )
    .unwrap();
    let rate = std::env::var("OMATAINER_FIXED_SEND_RATE")
        .unwrap()
        .parse()
        .unwrap();
    let mut prepared = project::Prepared::from_state(bundle.state, bundle.media, rate)
        .unwrap()
        .into_offline();
    let output = pcm(&mut prepared, rate, 127);
    std::fs::write(
        Path::new(&path).with_extension("f32"),
        output
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect::<Vec<_>>(),
    )
    .unwrap();
}

#[test]
fn fixed_send_capture_native_fresh_process_and_encoded_export_null_with_wet_tails() {
    let files = Files::new();
    for rate in [44100, 48000, 96000] {
        let (_, mut dry) = fixture(rate, [0.; 2]);
        let dry = pcm(&mut dry, rate, 64);
        for (case, sends) in [[0.43, 0.], [0., 0.71], [0.29, 0.67]]
            .into_iter()
            .enumerate()
        {
            let (engine, mut live) = fixture(rate, sends);
            let saved = capture(&engine, &mut live);
            let path = files.0.join(format!("sends-{rate}-{case}.omat"));
            crate::project_file::save(
                &path,
                &crate::project_file::Bundle {
                    state: saved.state.clone(),
                    media: saved.media.clone(),
                },
                crate::project_file::Overwrite::Never,
                &Default::default(),
                &AtomicBool::new(false),
            )
            .unwrap();
            let request = Export {
                end: 8192.0 / f64::from(rate),
                tail: 1.,
                options: Options {
                    rate,
                    ..Options::default()
                },
                ..Export::default()
            };
            let output = files.0.join(format!("encoded-{rate}-{case}"));
            run(
                saved,
                &request,
                &output,
                &engine.cmd.performance().optional_work().unwrap(),
                &crate::background::Reporter::default(),
            )
            .unwrap();
            let expected = pcm(&mut live, rate, 256);
            let decoded = crate::engine::decode::decode_audio(&output.join("master.wav")).unwrap();
            assert_eq!(
                decoded.sample.data, expected,
                "export differs at {rate}/{case}"
            );
            let wet: f64 = expected[8192 * 2..]
                .iter()
                .zip(&dry[8192 * 2..])
                .map(|(a, b)| f64::from(a - b).abs())
                .sum();
            assert!(wet > 0.0001, "missing wet tail {rate}/{case}: {wet}");
            let status = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "audio_delivery::tests::fixed_send_tests::fixed_send_fresh_process_renderer",
                    "--test-threads=1",
                ])
                .env("OMATAINER_FIXED_SEND_ARCHIVE", &path)
                .env("OMATAINER_FIXED_SEND_RATE", rate.to_string())
                .status()
                .unwrap();
            assert!(status.success());
            let independent = std::fs::read(path.with_extension("f32")).unwrap();
            assert_eq!(
                independent,
                expected
                    .iter()
                    .flat_map(|v| v.to_le_bytes())
                    .collect::<Vec<_>>(),
                "fresh process differs at {rate}/{case}"
            );
            assert_eq!(
                test_alloc::measure(|| live.process(&mut [0.; 128])),
                test_alloc::Counts::default()
            );
            routing_receipt(
                &format!("fixed-send-{rate}-{case}"),
                serde_json::json!({"schema":1,"kind":"audio_routing_case","status":"pass","proof":"headless_software","rate":rate,"case":case,"sends":sends,"impulse_frames":[0,2048,8000],"fresh_process":true,"native_archive":true,"encoded_wav":true,"live_offline_null":true,"wet_tail_l1":wet,"callback_allocations":0,"callback_frees":0,"physical_proof":false}),
            );
            eprintln!("fixed-send {rate}/{case}: exact fresh-process/live/encoded null; wet tail L1={wet}");
        }
    }
}

fn clone_capture(c: &Captured) -> Captured {
    Captured {
        state: c.state.clone(),
        media: c.media.clone(),
        revision: c.revision,
        checkpoint: c.checkpoint,
        playback_receipts: c.playback_receipts.clone(),
    }
}
fn routing_receipt(name: &str, value: serde_json::Value) {
    if let Some(directory) = std::env::var_os("OMATAINER_ROUTING_RECEIPT_DIR") {
        let directory = PathBuf::from(directory);
        assert!(directory.is_absolute() && directory.starts_with("/home"));
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(
            directory.join(format!("{name}.json")),
            serde_json::to_vec_pretty(&value).unwrap(),
        )
        .unwrap();
    }
}

#[test]
fn named_mixer_contribution_observers_keep_feeders_and_fixed_tails_with_independent_sum() {
    use crate::engine::audio::routing::selection::{Policy, Selection, Target};
    let files = Files::new();
    for rate in [44100, 48000, 96000] {
        let (engine, mut rt) = fixture(rate, [0.31, 0.67]);
        rt.fx_wet.fill(0.);
        rt.tracks[0].surface.crossfader = 0;
        rt.tracks[0].gain = 0.5;
        rt.tracks[0].pan = -1.;
        rt.tracks[1] = rt.tracks[0].clone();
        rt.tracks[1].gain = 0.25;
        rt.tracks[1].pan = 1.;
        let captured = capture(&engine, &mut rt);
        let layout = captured.state.session.as_ref().unwrap();
        let request = Export {
            end: 8192. / f64::from(rate),
            tail: 1.,
            options: Options {
                rate,
                ..Options::default()
            },
            ..Export::default()
        };
        let bounds = request.frames().unwrap();
        let render = |targets: Vec<Target>, policy: Policy, label: &str| {
            let raw = files.0.join(format!("{label}-{rate}.f32"));
            let selection = Selection {
                namespace: layout.namespace,
                targets,
                policy,
            };
            render_selected(
                clone_capture(&captured),
                &request,
                Some(&selection),
                &raw,
                bounds,
                &Arc::new(AtomicBool::new(false)),
                &crate::background::Reporter::default(),
            )
            .unwrap();
            std::fs::read(raw)
                .unwrap()
                .chunks_exact(4)
                .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
                .collect::<Vec<_>>()
        };
        let tracks = render(
            vec![
                Target::Track(layout.tracks[0].id),
                Target::Track(layout.tracks[1].id),
            ],
            Policy::PostDevice,
            "tracks",
        );
        let scene = render(
            vec![Target::Scene(layout.scenes[0].id)],
            Policy::PostDevice,
            "scene",
        );
        assert_eq!(
            tracks, scene,
            "linear post-device track sums must match shared empty scene at {rate}"
        );
        let reverb = render(vec![Target::FixedReturn(0)], Policy::PostDevice, "reverb");
        let echo = render(vec![Target::FixedReturn(1)], Policy::PostDevice, "echo");
        let full_raw = files.0.join(format!("master-{rate}.f32"));
        render_selected(
            clone_capture(&captured),
            &request,
            None,
            &full_raw,
            bounds,
            &Arc::new(AtomicBool::new(false)),
            &crate::background::Reporter::default(),
        )
        .unwrap();
        let full = std::fs::read(full_raw).unwrap();
        let mut residual = 0f32;
        for (index, bytes) in full.chunks_exact(4).enumerate() {
            let expected = ((tracks[index] + reverb[index] + echo[index]) * 0.25).tanh();
            let actual = f32::from_le_bytes(bytes.try_into().unwrap());
            residual = residual.max((actual - expected).abs());
        }
        assert!(
            residual < 2e-7,
            "independent master equation residual {residual} at {rate}"
        );
        assert!(reverb[8192 * 2..].iter().any(|x| x.abs() > 1e-7));
        assert!(echo[8192 * 2..].iter().any(|x| x.abs() > 1e-7));
        let overlap = Selection {
            namespace: layout.namespace,
            targets: vec![
                Target::Track(layout.tracks[0].id),
                Target::Scene(layout.scenes[0].id),
            ],
            policy: Policy::PostDevice,
        };
        assert!(overlap
            .plan(&captured, None)
            .unwrap_err()
            .contains("overlap"));
        let stale = Selection {
            namespace: [99, 100],
            targets: vec![Target::Track(layout.tracks[0].id)],
            policy: Policy::Dry,
        };
        assert!(stale.plan(&captured, None).is_err());
        for block in [1, 64, 127, 256, 512, 1024] {
            let mut prepared =
                project::Prepared::from_state(captured.state.clone(), captured.media.clone(), rate)
                    .unwrap()
                    .into_offline();
            let expected = pcm(&mut prepared, rate, block);
            let actual: Vec<f32> = full
                .chunks_exact(4)
                .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
                .collect();
            assert_eq!(
                actual, expected,
                "normal render block invariance {rate}/{block}"
            );
        }
        routing_receipt(
            &format!("contributions-{rate}"),
            serde_json::json!({"schema":1,"kind":"audio_routing_case","status":"pass","proof":"headless_software","rate":rate,"frames":bounds.3,"blocks":[1,64,127,256,512,1024],"tracks_plus_fixed_returns_master_equation":"tanh(0.25*(tracks+reverb+echo))","maximum_residual":residual,"live_offline_null":true,"fixed_wet_tails":true,"namespace_and_overlap_refusal":true,"physical_proof":false}),
        );
    }
}
