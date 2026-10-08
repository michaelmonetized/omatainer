use super::*;

#[test]
fn actual_session_frames_use_the_emitted_ramp_clock_and_one_envelope_on_both_channels_without_heap()
{
    use crate::engine::{
        input_monitor::Mode,
        midi_data::{Conductor, Meter, Tempo, TimingSettings},
        test_alloc, Clip, ClipKind, Command, Engine,
    };
    for output_rate in [8000, 44100, 48000] {
        for reverse in [false, true] {
            for curved in [false, true] {
                let (_engine, rt) = Engine::headless_for_test(output_rate, 256);
                let mut rt = Box::new(rt);
                rt.apply(Command::Stop);
                rt.quant = 0.0;
                rt.quantize = false;
                rt.conductor = Some(
                    Conductor::native(
                        960,
                        vec![
                            Tempo::new(0, 120.0, true).unwrap(),
                            Tempo::new(1920, 180.0, false).unwrap(),
                        ],
                        vec![Meter {
                            tick: 0,
                            numerator: 4,
                            denominator_power: 2,
                            clocks: 24,
                            thirty_seconds: 8,
                        }],
                        TimingSettings::default(),
                    )
                    .unwrap(),
                );
                let source = std::sync::Arc::new(signal(44100, 2, 2));
                let mut region = Region::full(&source, 120.0).unwrap();
                region.start = 20007;
                region.end = 70013;
                region.loop_start = region.start;
                region.loop_end = region.end;
                region.reverse = reverse;
                region.fades = if curved {
                    Fades {
                        fade_in: 0.2,
                        fade_out: 0.3,
                        in_curve: -0.5,
                        out_curve: 0.5,
                        automatic: true,
                    }
                } else {
                    Fades {
                        automatic: true,
                        ..Default::default()
                    }
                };
                let plan = region.prepare(&source).unwrap();
                let mut raw_region = region;
                raw_region.fades = Fades::default();
                let raw = raw_region.prepare(&source).unwrap();
                let mut clip = Clip::empty();
                clip.kind = ClipKind::Audio;
                clip.name = source.name.clone();
                clip.audio = Some(source.clone());
                clip.bars = (plan.duration_beats / 4.0) as f32;
                clip.audio_region = Some(plan);
                clip.gain = 1.0;
                rt.tracks[0].clips[7] = clip;
                rt.tracks[0].fx.slots.clear();
                rt.tracks[0].input_monitor = Some(Mode::Off);
                rt.tracks[0].gain = 1.0;
                rt.tracks[0].kind = 4;
                for track in rt.session.tracks.iter_mut().skip(1) {
                    track.active = false;
                }
                rt.apply(Command::FireClip {
                    track: 0,
                    scene: 7,
                    looping: false,
                });
                let mut max_error = 0.0_f32;
                let mut heard = 0;
                let counts = test_alloc::measure(|| {
                    for _frame in 0..output_rate * 2 {
                        let elapsed = rt.precise_midi_beat();
                        rt.process(&mut [0.0; 2]);
                        let beats_per_second = rt.last_midi_step * f64::from(output_rate);
                        let gain = if elapsed >= plan.duration_beats {
                            0.0
                        } else if curved {
                            let incoming = if elapsed < 0.2 {
                                (elapsed / 0.2).powf(2.0_f64.powf(-0.5))
                            } else {
                                1.0
                            };
                            let outgoing = if elapsed > plan.duration_beats - 0.3 {
                                1.0 - ((elapsed - plan.duration_beats + 0.3) / 0.3)
                                    .powf(2.0_f64.powf(0.5))
                            } else {
                                1.0
                            };
                            incoming * outgoing
                        } else {
                            let edge = (beats_per_second * 0.004).min(plan.duration_beats * 0.5);
                            (elapsed / edge).clamp(0.0, 1.0)
                                * ((plan.duration_beats - elapsed) / edge).clamp(0.0, 1.0)
                        } as f32;
                        let expected = raw.sample(&source, elapsed, false).map(|v| v * gain);
                        let actual = rt.routing_track_taps[0];
                        for channel in 0..2 {
                            max_error = max_error.max((actual[channel] - expected[channel]).abs());
                        }
                        if actual[0].abs() > 0.01 {
                            heard += 1;
                        }
                    }
                });
                assert_eq!(counts, test_alloc::Counts::default());
                assert!(
                    heard > output_rate / 4,
                    "{output_rate} {reverse} {curved}: heard {heard} frames, error {max_error}"
                );
                assert!(
                    max_error < 0.000002,
                    "{output_rate} {reverse} {curved}: {max_error}"
                );
            }
        }
    }
}
fn save_pcm(directory: &std::path::Path, name: &str, rate: u32, pcm: &[f32]) {
    use std::io::Write;
    let bytes = (pcm.len() * 4) as u32;
    let mut file = std::fs::File::create(directory.join(name)).unwrap();
    file.write_all(b"RIFF").unwrap();
    file.write_all(&(bytes + 36).to_le_bytes()).unwrap();
    file.write_all(b"WAVEfmt ").unwrap();
    file.write_all(&16u32.to_le_bytes()).unwrap();
    file.write_all(&3u16.to_le_bytes()).unwrap();
    file.write_all(&2u16.to_le_bytes()).unwrap();
    file.write_all(&rate.to_le_bytes()).unwrap();
    file.write_all(&(rate * 8).to_le_bytes()).unwrap();
    file.write_all(&8u16.to_le_bytes()).unwrap();
    file.write_all(&32u16.to_le_bytes()).unwrap();
    file.write_all(b"data").unwrap();
    file.write_all(&bytes.to_le_bytes()).unwrap();
    for value in pcm {
        file.write_all(&value.to_le_bytes()).unwrap();
    }
}

fn signal(rate: u32, channels: u16, kind: usize) -> Sample {
    Sample {
        name: format!("fade-source-{kind}"),
        sr: rate,
        ch: channels,
        data: (0..rate * 2)
            .flat_map(|frame| {
                let t = f64::from(frame) / f64::from(rate);
                let value = match kind {
                    0 => 0.24 + 0.12 * (t * 127.0 * std::f64::consts::TAU + 0.7).sin(),
                    1 => {
                        0.2 + 0.16
                            * (-((t * 7.0) % 1.0) * 18.0).exp()
                            * (t * 83.0 * std::f64::consts::TAU).cos()
                    }
                    _ => {
                        0.18 + (0.08 * (t * 211.0 * std::f64::consts::TAU + 0.3).sin()
                            + 0.04 * (t * 433.0 * std::f64::consts::TAU).sin())
                            * (0.7 + 0.3 * (t * 3.0).cos())
                    }
                } as f32;
                (0..channels).map(move |channel| value * if channel == 0 { 1.0 } else { -0.6 })
            })
            .collect(),
        peaks: Default::default(),
        spectrum: None,
        bpm: 120.0,
        path: String::new(),
    }
}

#[test]
fn fade_geometry_curves_complements_and_nonfinite_positions_are_bounded() {
    for curve in [-1.0, -0.5, 0.0, 0.5, 1.0] {
        let incoming = Fades {
            fade_in: 0.5,
            in_curve: curve,
            automatic: true,
            ..Default::default()
        };
        let outgoing = Fades {
            fade_out: 0.5,
            out_curve: curve,
            automatic: true,
            ..Default::default()
        };
        assert!(incoming.valid(1.0) && outgoing.valid(1.0));
        for n in 0..=1000 {
            let progress = f64::from(n) / 1000.0;
            let a = incoming.gain(progress * 0.5, 1.0, 2.0);
            let b = outgoing.gain(0.5 + progress * 0.5, 1.0, 2.0);
            assert!((a + b - 1.0).abs() < 1e-6, "{curve} {progress}: {a}+{b}");
            let expected = (progress.powf(2.0f64.powf(f64::from(curve)))) as f32;
            assert!((a - expected).abs() < 1e-6);
        }
    }
    for bad in [f64::NAN, f64::INFINITY, -1.0] {
        assert!(!Fades {
            fade_in: bad,
            ..Default::default()
        }
        .valid(1.0));
    }
    assert!(!Fades {
        fade_in: 0.6,
        fade_out: 0.5,
        ..Default::default()
    }
    .valid(1.0));
    assert!(!Fades {
        in_curve: 1.1,
        ..Default::default()
    }
    .valid(1.0));
    for position in [f64::NAN, f64::INFINITY, -0.1, 1.0] {
        assert_eq!(Fades::default().gain(position, 1.0, 2.0), 0.0);
    }
    assert_eq!(Fades::default().gain(0.5, 1.0, 2.0), 1.0);
}

#[test]
fn real_sine_drum_and_vocal_like_pcm_cuts_reduce_nonzero_edge_discontinuities_at_unequal_rates() {
    let mut cases = Vec::new();
    for source_rate in [8000, 44100, 48000] {
        for output_rate in [8000, 44100, 48000, 96000] {
            for channels in [1, 2] {
                for kind in 0..3 {
                    let source = signal(source_rate, channels, kind);
                    let mut region = Region::full(&source, 120.0).unwrap();
                    region.start = u64::from(source_rate) / 5 + 7;
                    region.end = u64::from(source_rate) * 7 / 5 + 13;
                    region.loop_start = region.start;
                    region.loop_end = region.end;
                    let raw = region.prepare(&source).unwrap();
                    region.fades = Fades {
                        automatic: true,
                        ..Default::default()
                    };
                    let faded = region.prepare(&source).unwrap();
                    let step = 2.0 / f64::from(output_rate);
                    let frames = (faded.duration_beats / step).ceil() as usize;
                    let start = raw.sample(&source, 0.0, false);
                    let last = raw.sample(&source, (frames - 1) as f64 * step, false);
                    let faded_start = faded.faded_sample(&source, 0.0, false, 2.0);
                    let faded_last =
                        faded.faded_sample(&source, (frames - 1) as f64 * step, false, 2.0);
                    assert!(start[0].abs() > 0.04 && last[0].abs() > 0.04);
                    assert_eq!(faded_start, [0.0; 2]);
                    assert!(
                        faded_last[0].abs() < last[0].abs() * 0.032,
                        "{source_rate}/{output_rate} {kind}: {faded_last:?} vs {last:?}"
                    );
                    let mut max_gain_error = 0.0f32;
                    let mut emitted = Vec::with_capacity(frames * 2);
                    for frame in 0..frames {
                        let beat = frame as f64 * step;
                        let value = faded.faded_sample(&source, beat, false, 2.0);
                        let original = raw.sample(&source, beat, false);
                        let gain = region.fades.gain(beat, faded.duration_beats, 2.0);
                        for channel in 0..2 {
                            max_gain_error = max_gain_error
                                .max((value[channel] - original[channel] * gain).abs());
                        }
                        if channels == 1 {
                            assert_eq!(value[0], value[1]);
                        } else {
                            assert!((value[1] + value[0] * 0.6).abs() < 1e-7);
                        }
                        emitted.extend(value);
                    }
                    assert!(max_gain_error < 1e-7);
                    cases.push(serde_json::json!({"source_rate":source_rate,"output_rate":output_rate,"channels":channels,"source_kind":kind,"raw_start_jump":start[0].abs(),"raw_end_jump":last[0].abs(),"faded_start_jump":faded_start[0].abs(),"faded_end_jump":faded_last[0].abs(),"max_channel_gain_error":max_gain_error}));
                    if source_rate == 44100 && output_rate == 48000 && channels == 2 {
                        if let Ok(root) = std::env::var("OMATAINER_FADES_PCM_DIR") {
                            std::fs::create_dir_all(&root).unwrap();
                            save_pcm(
                                std::path::Path::new(&root),
                                &format!("source-{kind}-faded.wav"),
                                output_rate,
                                &emitted,
                            );
                        }
                    }
                }
            }
        }
    }
    println!(
        "FADES_PCM_RECEIPT {}",
        serde_json::json!({"physical_devices_opened":false,"cases":cases})
    );
}

#[test]
fn loop_intro_reverse_transpose_and_auto_edges_share_trimmed_coordinates_without_heap_work() {
    let source = signal(48000, 2, 0);
    for reverse in [false, true] {
        for transpose in [-12.0, 0.0, 12.0] {
            let mut region = Region::full(&source, 120.0).unwrap();
            region.start = 12007;
            region.end = 72013;
            region.loop_start = 24003;
            region.loop_end = 48001;
            region.loop_enabled = true;
            region.reverse = reverse;
            region.transpose = transpose;
            region.fades = Fades {
                automatic: true,
                ..Default::default()
            };
            let plan = region.prepare(&source).unwrap();
            let intro = if reverse {
                region.end - region.loop_start
            } else {
                region.loop_end - region.start
            };
            let period = region.loop_end - region.loop_start;
            let intro_beats = intro as f64 / plan.frames_per_beat;
            let period_beats = period as f64 / plan.frames_per_beat;
            for loop_index in 0..4 {
                let boundary = intro_beats + f64::from(loop_index) * period_beats;
                assert!(plan
                    .faded_sample(&source, boundary, true, 2.0)
                    .iter()
                    .all(|v| v.abs() < 1e-7));
                let before = plan.faded_sample(&source, boundary - 2.0 / 48000.0, true, 2.0);
                assert!(
                    before.iter().all(|v| v.abs() < 0.0021),
                    "{reverse} {transpose}: {before:?}"
                );
            }
            assert_eq!(
                crate::engine::test_alloc::measure(|| {
                    for frame in 0..48000 {
                        std::hint::black_box(plan.faded_sample(
                            &source,
                            f64::from(frame) * 2.0 / 48000.0,
                            true,
                            2.0,
                        ));
                    }
                }),
                crate::engine::test_alloc::Counts::default()
            );
        }
    }
}

#[test]
fn reviewed_user_vocal_passage_decodes_and_reduces_actual_pcm_cut_discontinuities_when_qualified() {
    let Ok(path) = std::env::var("OMATAINER_FADE_VOCAL_SOURCE") else {
        return;
    };
    let decoded = crate::engine::decode::decode_audio(std::path::Path::new(&path)).unwrap();
    let source = &decoded.sample;
    assert!(source.frames() > source.sr as usize * 20);
    let start = source.sr as usize * 12;
    let end = source.sr as usize * 18;
    let channels = usize::from(source.ch);
    let peak_frame = |a: usize, b: usize| {
        (a..b)
            .max_by(|&a, &b| {
                source.data[a * channels]
                    .abs()
                    .total_cmp(&source.data[b * channels].abs())
            })
            .unwrap()
    };
    let start = peak_frame(start, start + 2048);
    let end = peak_frame(end, end + 2048) + 1;
    let mut region = Region::full(source, 120.0).unwrap();
    region.start = start as u64;
    region.end = end as u64;
    region.loop_start = region.start;
    region.loop_end = region.end;
    let raw = region.prepare(source).unwrap();
    region.fades = Fades {
        automatic: true,
        ..Default::default()
    };
    let faded = region.prepare(source).unwrap();
    let mut measurements = Vec::new();
    for output_rate in [44100, 48000, 96000] {
        let step = 2.0 / f64::from(output_rate);
        let frames = (faded.duration_beats / step).ceil() as usize;
        let before = raw.sample(source, 0.0, false);
        let after = faded.faded_sample(source, 0.0, false, 2.0);
        let last_raw = raw.sample(source, (frames - 1) as f64 * step, false);
        let last_faded = faded.faded_sample(source, (frames - 1) as f64 * step, false, 2.0);
        assert!(before[0].abs() > 0.01 && last_raw[0].abs() > 0.01);
        assert_eq!(after, [0.0; 2]);
        assert!(last_faded[0].abs() < last_raw[0].abs() * 0.006);
        measurements.push(serde_json::json!({"output_rate":output_rate,"raw_start":before,"faded_start":after,"raw_end":last_raw,"faded_end":last_faded}));
    }
    println!(
        "FADES_VOCAL_RECEIPT {}",
        serde_json::json!({"source_path":path,"source_rate":source.sr,"channels":source.ch,"start_frame":start,"end_frame":end,"measurements":measurements,"physical_devices_opened":false})
    );
}
