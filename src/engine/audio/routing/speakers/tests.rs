use super::*;
use crate::engine::{
    audio::routing::{model::*, prepared::Prepared},
    test_alloc, ClipKind, Command, Engine, Sample,
};
use std::sync::Arc;

#[test]
fn stereo_lfe_declared_contract_preserves_order_and_rejects_ambiguous_widths() {
    let legacy: super::super::model::Model = serde_json::from_str(
        &serde_json::to_string(&super::super::model::Model::default()).unwrap(),
    )
    .unwrap();
    assert!(legacy.speaker_layouts.is_empty());
    assert!(!serde_json::to_string(&legacy)
        .unwrap()
        .contains("speaker_layouts"));
    let mut model = legacy;
    model.ports[0].channels = vec![3, 0, 5];
    assert!(model.channel_contract(Group::Output(1), 3).is_err());
    model.set_speaker_layout(Group::Output(1), Layout::StereoLfe);
    let contract = model.channel_contract(Group::Output(1), 3).unwrap();
    assert_eq!(contract.channels, [3, 0, 5]);
    assert_eq!(contract.channel_count, 3);
    assert_eq!(contract.wave_mask, Some(0xB));
    assert_eq!(
        serde_json::to_value(&contract.speakers).unwrap(),
        serde_json::json!(["front_left", "front_right", "low_frequency"])
    );
    assert_eq!(
        model
            .channel_contract(Group::Output(1), 2)
            .unwrap()
            .speakers,
        [Speaker::FrontLeft, Speaker::FrontRight]
    );
    assert!(model.channel_contract(Group::Output(1), 6).is_err());
    model.speaker_layouts.push(model.speaker_layouts[0]);
    assert!(model.channel_contract(Group::Output(1), 3).is_err());
    assert_eq!(Layout::StereoLfe.downmix(&[0.2, -0.3, 0.9]), [0.2, -0.3]);
}

#[test]
fn stereo_lfe_clip_track_scene_main_physical_channels_follow_independent_pcm_without_heap() {
    for rate in [44100, 48000, 96000] {
        for block in [1, 64, 127, 512] {
            let (_engine, mut rt) = Engine::headless_for_test(rate, 256);
            rt.apply(Command::Stop);
            rt.quantize = false;
            rt.bpm = 120.;
            rt.master = 0.25;
            for track in &mut rt.tracks {
                for clip in &mut track.clips {
                    *clip = crate::engine::Clip::empty();
                }
                track.fx.slots.clear();
            }
            for rack in &mut rt.scene_fx {
                rack.slots.clear();
            }
            let mut data = vec![0.; 4096 * 3];
            for channel in 0..3 {
                data[(8 + 13 * channel) * 3 + channel] = (channel + 1) as f32 / 32.;
            }
            let sample = Arc::new(Sample {
                name: "Original ordered speaker impulses".into(),
                sr: rate,
                ch: 3,
                data,
                peaks: Default::default(),
                spectrum: None,
                bpm: 120.,
                path: String::new(),
            });
            let region = crate::engine::audio_clip::Region::full(&sample, 120.)
                .unwrap()
                .prepare(&sample)
                .unwrap();
            let track = &mut rt.tracks[0];
            track.gain = 1.;
            track.pan = 0.;
            track.kind = 4;
            track.eq.low_g = 1.;
            track.eq.mid_g = 1.;
            track.eq.high_g = 1.;
            track.clips[0].kind = ClipKind::Audio;
            track.clips[0].audio = Some(sample);
            track.clips[0].audio_region = Some(region);
            track.clips[0].bars = region.duration_beats as f32 / 4.;
            let mut model = super::super::model::Model::default();
            model.ports[0].channels = vec![3, 0, 5];
            model.connections[0].map = (0..3)
                .map(|i| ChannelMap {
                    source: i,
                    destination: i,
                    gain: 1.,
                })
                .collect();
            model.set_speaker_layout(Group::Track(rt.session.tracks[0].id), Layout::StereoLfe);
            for scene in &rt.session.scenes {
                model.set_speaker_layout(Group::Scene(scene.id), Layout::StereoLfe);
            }
            model.set_speaker_layout(Group::Main, Layout::StereoLfe);
            model.set_speaker_layout(Group::Output(1), Layout::StereoLfe);
            let saved = serde_json::from_slice::<super::super::model::Model>(
                &serde_json::to_vec(&model).unwrap(),
            )
            .unwrap();
            rt.routing = Some(Box::new(
                Prepared::at_rate(Arc::new(saved), &rt.session, rate).unwrap(),
            ));
            rt.apply(Command::LaunchScene { scene: 0 });
            let mut output = vec![0.; 4096 * 6];
            assert_eq!(
                test_alloc::measure(|| {
                    for chunk in output.chunks_mut(block * 6) {
                        rt.process_interleaved(chunk, 6);
                    }
                }),
                Default::default(),
                "{rate}/{block}"
            );
            for (frame, values) in output.chunks_exact(6).enumerate() {
                for physical in 0..6 {
                    let expected =
                        [3, 0, 5]
                            .iter()
                            .position(|p| *p == physical)
                            .map_or(0., |channel| {
                                if frame == 8 + 13 * channel {
                                    (((channel + 1) as f32 / 32.) * 0.25).tanh()
                                } else {
                                    0.
                                }
                            });
                    assert!(
                        (values[physical] - expected).abs() < 2e-6,
                        "{rate}/{block} frame{frame} physical{physical}: {} != {expected}",
                        values[physical]
                    );
                }
            }
        }
    }
}
