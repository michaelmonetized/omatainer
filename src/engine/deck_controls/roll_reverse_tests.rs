use super::*;
use crate::engine::{
    audio::OutputCallback,
    beatgrid::Grid,
    deck_pads::{Mode, Press, Release},
    test_alloc, Engine, Sample,
};
use std::{path::PathBuf, sync::Arc};

fn fixture(source_rate: u32, output_rate: u32) -> (Engine, Box<RtEngine>) {
    let (engine, mut rt) = Engine::headless_for_test(output_rate, 128);
    let data = (0..source_rate * 12)
        .flat_map(|frame| {
            let tone = (std::f64::consts::TAU * 233.0 * f64::from(frame) / f64::from(source_rate))
                .sin() as f32
                * 0.2;
            [tone, -tone]
        })
        .collect();
    let audio = Arc::new(Sample {
        name: "Source-qualified reverse and roll".into(),
        path: String::new(),
        sr: source_rate,
        ch: 2,
        bpm: 120.0,
        data,
        peaks: vec![].into(),
        spectrum: None,
    });
    for deck in 0..2 {
        rt.apply(Command::DeckAudio {
            deck,
            audio: audio.clone(),
        });
        rt.apply(Command::DeckPlay { deck });
        let d = &mut rt.decks[usize::from(deck)];
        d.pos = f64::from(source_rate) * 1.275;
        d.rate = 1.0;
        d.grid = Some(
            Grid::new(0.125, 120.0)
                .unwrap()
                .with_anchor(4.0, 2.125)
                .unwrap()
                .with_anchor(8.0, 5.125)
                .unwrap(),
        );
    }
    (engine, Box::new(rt))
}
fn control(rt: &mut RtEngine, source: u64, control: Control) {
    rt.apply(Command::DeckControl {
        source,
        deck: 0,
        control,
    });
}
fn press(rt: &mut RtEngine, source: u64, key: u32, pad: u8) {
    rt.apply(Command::DeckPadPress(Press {
        source,
        key,
        deck: 0,
        id: pad + 1,
        mode: Some(Mode::Roll),
        pressure: 0.8,
        shifted: false,
    }));
}
fn release(rt: &mut RtEngine, source: u64, key: u32) {
    rt.apply(Command::DeckPadRelease(Release { source, key }));
}
fn render(rt: &mut RtEngine, frames: usize) {
    for _ in 0..frames {
        let (l, r) = rt.render_deck(0);
        assert!(l.is_finite() && r.is_finite());
    }
}

#[test]
fn rolls_use_the_selected_deck_division_and_cancel_original_pending_owners_at_mixed_rates() {
    let mut cases = 0;
    let mut maximum = 0.0_f64;
    for source in [44100, 48000, 96000] {
        for output in [44100, 48000, 96000] {
            let (_engine, mut rt) = fixture(source, output);
            control(
                &mut rt,
                7,
                Control::Quantize {
                    enabled: true,
                    division: 3,
                },
            );
            press(&mut rt, 11, 1, 6);
            let state = rt.decks[0].controls.status();
            assert_eq!(state.roll_pending, Some(6));
            assert_eq!(state.roll_due, Some(3.0));
            assert_eq!(state.roll_active, None);
            release(&mut rt, 11, 1);
            assert_eq!(rt.decks[0].controls.status().roll_pending, None);
            render(&mut rt, 16);
            assert_eq!(rt.decks[0].controls.status().roll_active, None);
            rt.decks[0].pos = f64::from(source) * 1.275;
            press(&mut rt, 12, 2, 6);
            let mut frames = 0;
            while rt.decks[0].controls.status().roll_active.is_none() {
                render(&mut rt, 1);
                frames += 1;
                assert!(frames < output as usize);
            }
            let error = (rt.decks[0].loop_start / f64::from(source) - 1.625).abs();
            maximum = maximum.max(error);
            assert!(error <= 2.0 / f64::from(output));
            assert!((frames as f64 - f64::from(output) * 0.35).abs() <= 2.0);
            assert_eq!(rt.decks[0].controls.status().roll_active, Some(6));
            let length = (2.875 - 1.625) * f64::from(source);
            assert!(
                (rt.decks[0].loop_len - length).abs() < 2.0 * f64::from(source) / f64::from(output)
            );
            release(&mut rt, 12, 2);
            assert_eq!(rt.decks[0].controls.status().roll_active, None);
            cases += 1;
        }
    }
    println!("ROLL_ONSET {{\"cases\":{cases},\"maximum_start_error_seconds\":{maximum},\"physical_devices_opened\":false}}");
}

#[test]
fn immediate_roll_starts_at_the_actual_position_and_latest_owner_survives_slot_reuse() {
    let (_engine, mut rt) = fixture(48000, 48000);
    let (_baseline_engine, mut baseline) = fixture(48000, 48000);
    for target in [&mut rt, &mut baseline] {
        target.decks[0].loop_on = true;
        target.decks[0].loop_start = 6000.0;
        target.decks[0].loop_len = 48000.0;
        target.decks[0].pos = 25000.25;
    }
    let start = rt.decks[0].pos;
    press(&mut rt, 11, 1, 1);
    assert_eq!(rt.decks[0].loop_start, start);
    press(&mut rt, 12, 2, 2);
    press(&mut rt, 13, 3, 3);
    release(&mut rt, 11, 1);
    press(&mut rt, 14, 4, 4);
    assert_eq!(rt.decks[0].controls.status().roll_active, Some(4));
    release(&mut rt, 13, 3);
    assert_eq!(rt.decks[0].controls.status().roll_active, Some(4));
    render(&mut rt, 100000);
    render(&mut baseline, 100000);
    release(&mut rt, 14, 4);
    assert_eq!(rt.decks[0].controls.status().roll_active, Some(2));
    release(&mut rt, 12, 2);
    assert_eq!(rt.decks[0].controls.status().roll_active, None);
    assert!((rt.decks[0].pos - baseline.decks[0].pos).abs() < 1e-6);
    assert_eq!(
        (
            rt.decks[0].loop_on,
            rt.decks[0].loop_start,
            rt.decks[0].loop_len
        ),
        (true, 6000.0, 48000.0)
    );
}

#[test]
fn numbered_impulse_audio_repeats_selected_rolls_retriggers_and_returns_to_the_forward_pcm() {
    let (_engine, mut rt) = fixture(48000, 48000);
    let (_reference_engine, mut reference) = fixture(48000, 48000);
    let mut data = vec![0.0; 48000 * 12 * 2];
    for frame in (6000..48000 * 12 - 24).step_by(6000) {
        let value = (frame / 6000 % 15 + 1) as f32 / 64.0;
        for offset in 0..24 {
            data[(frame + offset) * 2] = value;
            data[(frame + offset) * 2 + 1] = -value;
        }
    }
    let audio = Arc::new(Sample {
        name: "Numbered quarter-beat impulses".into(),
        path: String::new(),
        sr: 48000,
        ch: 2,
        bpm: 120.0,
        data,
        peaks: vec![].into(),
        spectrum: None,
    });
    for target in [&mut rt, &mut reference] {
        target.apply(Command::DeckAudio {
            deck: 0,
            audio: audio.clone(),
        });
        target.apply(Command::DeckPlay { deck: 0 });
        target.apply(Command::DeckGain {
            deck: 0,
            value: 1.0,
        });
        target.decks[0].grid = None;
        target.decks[0].pos = 50000.25;
        target.decks[0].rate = 1.0;
    }
    let collect = |target: &mut RtEngine, frames: usize| {
        let mut words = Vec::new();
        let mut peak = 0.0_f32;
        for _ in 0..frames {
            let (left, right) = target.render_deck(0);
            assert!(left.is_finite() && right.is_finite());
            assert!((left + right).abs() < 1e-6);
            if left.abs() > 0.002 {
                peak = peak.max(left.abs());
            } else if peak > 0.0 {
                words.push(peak);
                peak = 0.0;
            }
        }
        assert_eq!(peak, 0.0);
        words
    };
    let check = |words: &[f32], codes: &[u8]| {
        assert_eq!(
            words.len(),
            codes.len(),
            "Rendered impulse sequence {words:?}"
        );
        let scale = words[0] / f32::from(codes[0]);
        assert!(scale > 0.001);
        for (word, code) in words.iter().zip(codes) {
            assert!(
                (word - f32::from(*code) * scale).abs() < 0.0002,
                "Rendered word {word} does not carry source code {code}"
            );
        }
    };
    press(&mut rt, 51, 1, 5);
    let full = collect(&mut rt, 96000);
    check(&full, &[10, 11, 12, 13].repeat(4));
    press(&mut rt, 52, 2, 4);
    let half = collect(&mut rt, 48000);
    check(&half, &[10, 11].repeat(4));
    release(&mut rt, 52, 2);
    let restored = collect(&mut rt, 24000);
    check(&restored, &[10, 11, 12, 13]);
    release(&mut rt, 51, 1);
    render(&mut reference, 168000);
    assert!((rt.decks[0].pos - reference.decks[0].pos).abs() < 1e-6);
    assert!(!rt.decks[0].loop_on);
    render(&mut rt, 256);
    render(&mut reference, 256);
    let mut maximum = 0.0_f32;
    let mut energy = 0.0_f32;
    for _ in 0..12000 {
        let actual = rt.render_deck(0);
        let expected = reference.render_deck(0);
        maximum = maximum
            .max((actual.0 - expected.0).abs())
            .max((actual.1 - expected.1).abs());
        energy += actual.0.abs() + actual.1.abs();
    }
    assert!(energy > 0.1);
    assert!(maximum < 1e-6, "Forward PCM error {maximum}");
    println!("ROLL_PCM {{\"numbered_words\":{},\"rendered_roll_frames\":168000,\"release_compared_frames\":12000,\"maximum_release_pcm_error\":{maximum},\"physical_devices_opened\":false}}",full.len()+half.len()+restored.len());
}

#[test]
fn reverse_latch_and_independent_holds_define_file_and_backward_loop_boundaries() {
    let (_engine, mut rt) = fixture(48000, 48000);
    control(&mut rt, 21, Control::Reverse { enabled: true });
    assert!(rt.decks[0].controls.status().reverse_latched);
    let start = rt.decks[0].pos;
    render(&mut rt, 24000);
    assert!(rt.decks[0].pos < start - 23000.0);
    control(
        &mut rt,
        22,
        Control::Hold {
            button: Button::Reverse,
            on: true,
        },
    );
    control(&mut rt, 21, Control::Reverse { enabled: false });
    assert!(rt.decks[0].controls.status().reverse);
    control(
        &mut rt,
        22,
        Control::Hold {
            button: Button::Reverse,
            on: false,
        },
    );
    assert!(!rt.decks[0].controls.status().reverse);
    control(&mut rt, 21, Control::Reverse { enabled: true });
    rt.decks[0].loop_on = true;
    rt.decks[0].loop_start = 6000.0;
    rt.decks[0].loop_len = 12000.0;
    rt.decks[0].pos = 6000.25;
    rt.decks[0].rate = -1.0;
    render(&mut rt, 32);
    assert!(rt.decks[0].pos > 17000.0 && rt.decks[0].pos < 18000.0);
    rt.decks[0].loop_on = false;
    rt.decks[0].pos = 0.0;
    render(&mut rt, 64);
    assert_eq!(rt.decks[0].pos, 0.0);
    assert!(rt.decks[0].playing);
    assert!(rt.decks[0].controls.status().reverse_latched);
    rt.apply(Command::SafetyStop(
        crate::engine::performance::Safety::Stop,
    ));
    assert!(!rt.decks[0].controls.status().reverse);
}

#[test]
fn censor_release_resumes_the_advancing_source_and_preserves_other_owners_and_deck_pcm() {
    let (engine, mut rt) = fixture(48000, 48000);
    let (_baseline_engine, mut baseline) = fixture(48000, 48000);
    for runtime in [&mut *rt, &mut *baseline] {
        runtime.apply(Command::Xfader(1.0));
        runtime.process(&mut [0.0; 16384]);
    }
    let source = rt.decks[0].pos;
    let mut callback = Box::new(OutputCallback::new(*rt, 2));
    let mut reference = Box::new(OutputCallback::new(*baseline, 2));
    let mut output = vec![0.0_f32; 32768];
    let mut expected = vec![0.0_f32; 32768];
    engine
        .cmd
        .send(Command::DeckControl {
            source: 31,
            deck: 0,
            control: Control::Hold {
                button: Button::Bleep,
                on: true,
            },
        })
        .unwrap();
    assert_eq!(
        test_alloc::measure(|| callback.render(&mut output)),
        Default::default()
    );
    reference.render(&mut expected);
    assert_eq!(output, expected);
    assert!(output.iter().any(|sample| sample.abs() > 0.001));
    let forward = callback.renderer_for_test().decks[0]
        .controls
        .forward
        .unwrap();
    assert!((forward - (source + 16384.0)).abs() < 1e-5);
    let runtime = callback.renderer_mut_for_test();
    control(
        runtime,
        32,
        Control::Hold {
            button: Button::Bleep,
            on: true,
        },
    );
    control(
        runtime,
        31,
        Control::Hold {
            button: Button::Bleep,
            on: false,
        },
    );
    assert!(runtime.decks[0].controls.status().bleep);
    control(
        runtime,
        32,
        Control::Hold {
            button: Button::Bleep,
            on: false,
        },
    );
    assert!((runtime.decks[0].pos - forward).abs() < 1e-5);
    assert!((reference.renderer_for_test().decks[0].pos - runtime.decks[0].pos).abs() < 1e-5);
    println!(
        "DIRECTION_CALLBACK {}",
        serde_json::json!({"other_deck_compared_frames":16384,"other_deck_max_error":0,"callback_allocations":0,"callback_frees":0,"physical_devices_opened":false})
    );
}

#[test]
fn actual_decoded_formats_keep_direction_switches_and_censor_returns_inside_the_click_envelope() {
    let root = PathBuf::from(std::env::var_os("TMPDIR").expect("Home temporary root required"))
        .join(format!("omatainer-direction-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let mut maximum = 0.0_f32;
    for (extension, codec) in [
        ("wav", "pcm_s16le"),
        ("flac", "flac"),
        ("mp3", "libmp3lame"),
    ] {
        let path = root.join(format!("tonal.{extension}"));
        assert!(std::process::Command::new("ffmpeg")
            .args([
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=233:sample_rate=48000:duration=8",
                "-ac",
                "2",
                "-c:a",
                codec
            ])
            .arg(&path)
            .status()
            .unwrap()
            .success());
        let decoded = crate::engine::decode::decode_audio(&path).unwrap();
        let (_engine, mut rt) = fixture(48000, 48000);
        rt.apply(Command::DeckAudio {
            deck: 0,
            audio: Arc::new(decoded.sample),
        });
        rt.apply(Command::DeckPlay { deck: 0 });
        rt.decks[0].pos = 96000.0;
        let mut previous = 0.0_f32;
        for frame in 0..48000 {
            if frame == 4000 {
                control(&mut rt, 41, Control::Reverse { enabled: true });
            }
            if frame == 12000 {
                control(&mut rt, 41, Control::Reverse { enabled: false });
            }
            if frame == 20000 {
                control(
                    &mut rt,
                    42,
                    Control::Hold {
                        button: Button::Bleep,
                        on: true,
                    },
                );
            }
            if frame == 30000 {
                control(
                    &mut rt,
                    42,
                    Control::Hold {
                        button: Button::Bleep,
                        on: false,
                    },
                );
            }
            let (l, r) = rt.render_deck(0);
            assert!(l.is_finite() && r.is_finite());
            if frame > 1000 {
                maximum = maximum.max((l - previous).abs());
            }
            previous = l;
        }
    }
    assert!(maximum < 0.02, "maximum sample discontinuity {maximum}");
    println!("DIRECTION_FORMATS {{\"formats\":[\"wav\",\"flac\",\"mp3\"],\"maximum_sample_discontinuity\":{maximum},\"physical_devices_opened\":false}}");
}
