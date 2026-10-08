use super::*;
use crate::engine::{
    audible::Writer, audio::OutputCallback, dsp::Sample, keylock::Mode, test_alloc, Command,
    Engine, RtEngine,
};
use std::sync::Arc;

fn tone(sr: u32, hz: f64) -> Arc<Sample> {
    Arc::new(Sample {
        name: "Original key-shift analytical stereo tone".into(),
        path: String::new(),
        sr,
        ch: 2,
        bpm: 120.0,
        peaks: vec![].into(),
        spectrum: None,
        data: (0..sr as usize * 4)
            .flat_map(|i| {
                let x = (std::f64::consts::TAU * hz * i as f64 / f64::from(sr)).sin() as f32 * 0.5;
                [x, -x]
            })
            .collect(),
    })
}
fn renderer(audio: Arc<Sample>, sr: u32, rate: f32, lock: bool) -> (Engine, Box<RtEngine>) {
    let (engine, mut rt) = Engine::headless_for_test(sr, 256);
    rt.apply(Command::DeckAudio { deck: 0, audio });
    let deck = &mut rt.decks[0];
    deck.pitch_range = 2;
    deck.pitch = rate - 0.5;
    deck.rate = rate;
    deck.target_rate = rate;
    deck.keylock = lock;
    deck.gain = 1.0;
    rt.apply(Command::DeckPlay { deck: 0 });
    rt.publish_for_test();
    (engine, Box::new(rt))
}
fn request(engine: &Engine, deck: u8, value: i8, lock: bool) -> Request {
    Request::new(
        deck,
        &engine.snapshot().decks[usize::from(deck)],
        value,
        lock,
    )
    .unwrap()
}
fn projection(samples: &[f32], sr: u32, hz: f64) -> (f64, f64) {
    let (mut sin, mut cos, mut energy) = (0.0, 0.0, 0.0);
    for (i, &x) in samples.iter().enumerate() {
        let phase = std::f64::consts::TAU * hz * i as f64 / f64::from(sr);
        sin += f64::from(x) * phase.sin();
        cos += f64::from(x) * phase.cos();
        energy += f64::from(x).powi(2);
    }
    (
        2.0 * sin.hypot(cos) / samples.len() as f64,
        (energy / samples.len() as f64).sqrt(),
    )
}
#[test]
fn actual_tuned_stereo_pcm_shifts_key_at_fixed_source_time_and_reports_combined_tempo_bypass() {
    let mut groups = 0;
    let mut active = 0;
    let mut bypass = 0;
    let mut max_timing_error = 0.0_f64;
    let mut minimum_amplitude = f64::INFINITY;
    for (source_sr, output_sr) in [(44100, 48000), (48000, 44100), (96000, 96000)] {
        for offset in [-6, -1, 1, 6] {
            for rate in [0.84, 1.0, 1.16] {
                for locked in [false, true] {
                    for hz in [20.0, 55.0, 997.0] {
                        let (engine, mut rt) =
                            renderer(tone(source_sr, hz), output_sr, rate, locked);
                        rt.apply(Command::DeckKeyShift(request(&engine, 0, offset, false)));
                        let ratio = if locked {
                            f64::from(rate) / 2.0_f64.powf(f64::from(offset) / 12.0)
                        } else {
                            2.0_f64.powf(-f64::from(offset) / 12.0)
                        };
                        let supported = (0.5..=1.5).contains(&ratio);
                        let expected = if supported {
                            hz * 2.0_f64.powf(f64::from(offset) / 12.0)
                                * if locked { 1.0 } else { f64::from(rate) }
                        } else {
                            hz * f64::from(rate)
                        };
                        for _ in 0..output_sr / 10 {
                            rt.render_deck(0);
                        }
                        let samples: Vec<_> = (0..output_sr / 2)
                            .map(|_| {
                                let (l, r) = rt.render_deck(0);
                                assert_eq!(l, -r);
                                assert!(l.is_finite());
                                l
                            })
                            .collect();
                        let (amplitude, rms) = projection(&samples, output_sr, expected);
                        assert!((0.43..0.57).contains(&amplitude),"src={source_sr} output={output_sr} shift={offset} rate={rate} lock={locked} hz={hz} expected={expected} amplitude={amplitude} rms={rms}");
                        assert!(
                            (rms - 0.5 / 2.0_f64.sqrt()).abs() < 0.035,
                            "PCM energy changed: {rms}"
                        );
                        let frames = output_sr / 10 + output_sr / 2;
                        let wanted = f64::from(frames) * f64::from(source_sr)
                            / f64::from(output_sr)
                            * f64::from(rate);
                        let error = (rt.decks[0].pos - wanted).abs();
                        assert!(error < 1e-5, "Transport drift {error}");
                        assert_eq!(
                            rt.decks[0].key_shift_mode(),
                            if supported {
                                Mode::Locked
                            } else {
                                Mode::UnsupportedRate
                            }
                        );
                        assert_eq!(rt.decks[0].keylock, locked);
                        max_timing_error = max_timing_error.max(error);
                        minimum_amplitude = minimum_amplitude.min(amplitude);
                        groups += 1;
                        if supported {
                            active += 1;
                        } else {
                            bypass += 1;
                        }
                    }
                }
            }
        }
    }
    println!(
        "KEY_SHIFT_TUNED_RECEIPT {}",
        serde_json::json!({"groups":groups,"active":active,"bypass":bypass,"max_source_frame_error":max_timing_error,"minimum_fundamental_amplitude":minimum_amplitude,"source_rates":[44100,48000,96000],"output_rates":[44100,48000,96000],"frequencies_hz":[20,55,997],"physical_devices_opened":false})
    );
}
#[test]
fn zero_offset_preserves_existing_pcm_exactly_and_reset_restores_original_pitch_without_moving_transport(
) {
    for (source_sr, output_sr) in [(44100, 48000), (48000, 44100), (96000, 96000)] {
        for lock in [false, true] {
            let source = tone(source_sr, 440.0);
            let (engine, mut candidate) = renderer(source.clone(), output_sr, 1.0, lock);
            let (_reference, mut baseline) = renderer(source, output_sr, 1.0, lock);
            candidate.apply(Command::DeckKeyShift(request(&engine, 0, 0, false)));
            for _ in 0..4096 {
                assert_eq!(candidate.render_deck(0), baseline.render_deck(0));
                assert_eq!(candidate.decks[0].pos, baseline.decks[0].pos);
            }
            let pos = candidate.decks[0].pos;
            candidate.apply(Command::DeckKeyShift(request(&engine, 0, 3, false)));
            assert_eq!(candidate.decks[0].pos, pos);
            for _ in 0..4096 {
                candidate.render_deck(0);
            }
            let pos = candidate.decks[0].pos;
            candidate.apply(Command::DeckKeyShift(request(&engine, 0, 0, false)));
            assert_eq!(candidate.decks[0].pos, pos);
            assert_eq!(candidate.decks[0].keylock, lock);
            for _ in 0..output_sr / 10 {
                candidate.render_deck(0);
            }
            let data: Vec<_> = (0..output_sr / 2)
                .map(|_| candidate.render_deck(0).0)
                .collect();
            assert!((projection(&data, output_sr, 440.0).0 - 0.5).abs() < 0.02);
            assert_eq!(candidate.decks[0].key_shift_mode(), Mode::Off);
        }
    }
}
#[test]
fn source_qualified_shift_refuses_replacement_invalid_offsets_and_wrong_decks_without_editing() {
    let (engine, mut rt) = renderer(tone(48000, 440.0), 48000, 1.0, false);
    rt.apply(Command::DeckPlay { deck: 0 });
    rt.publish_for_test();
    let captured = request(&engine, 0, 2, true);
    rt.apply(Command::DeckAudio {
        deck: 0,
        audio: tone(48000, 330.0),
    });
    rt.process(&mut []);
    let revision = rt.project.revision();
    let before = engine.undo.view().cursor;
    assert_eq!(
        test_alloc::measure(|| rt.apply(Command::DeckKeyShift(captured))),
        Default::default()
    );
    rt.process(&mut []);
    assert_eq!(rt.decks[0].key_shift, 0);
    assert!(!rt.decks[0].keylock);
    assert_eq!(rt.project.revision(), revision);
    assert_eq!(engine.undo.view().cursor, before);
    for (deck, semitones, media) in [(2, 0, 1), (0, 7, 1), (0, -7, 1), (0, 0, 0)] {
        let invalid = Request {
            deck,
            semitones,
            media,
            enable_lock: false,
        };
        assert!(engine.send(Command::DeckKeyShift(invalid)).is_err());
    }
}
#[test]
fn real_callback_retunes_and_resets_without_heap_work_or_changing_the_other_decks_pcm() {
    let (engine, mut rt) = renderer(tone(48000, 440.0), 48000, 1.0, false);
    rt.decks[0].playing = false;
    let (_reference, mut baseline) = renderer(tone(48000, 440.0), 48000, 1.0, false);
    baseline.decks[0].playing = false;
    for renderer in [&mut *rt, &mut *baseline] {
        renderer.apply(Command::Xfader(1.0));
        for _ in 0..64 {
            renderer.process(&mut [0.0; 256]);
        }
        renderer.apply(Command::DeckPlay { deck: 1 });
        renderer.apply(Command::DeckLoop {
            deck: 1,
            beats: 4.0,
        });
        renderer.publish_for_test();
    }
    let original = rt.decks[1].audio.clone().unwrap();
    let other = (rt.decks[1].gain, rt.decks[1].pitch);
    rt.apply(Command::DeckPlay { deck: 0 });
    rt.publish_for_test();
    let mut callback = Box::new(OutputCallback::new(*rt, 2));
    let mut reference = Box::new(OutputCallback::new(*baseline, 2));
    let mut frames = 0;
    let mut energy = 0.0;
    let mut peak_cpu_ns = 0;
    for offset in (-6..=6).chain([0]) {
        engine
            .send(Command::DeckKeyShift(request(&engine, 0, offset, false)))
            .unwrap();
        for _ in 0..32 {
            let mut actual = [0.0; 256];
            let mut expected = [0.0; 256];
            assert_eq!(
                test_alloc::measure(|| callback.render(&mut actual)),
                Default::default()
            );
            reference.render(&mut expected);
            assert_eq!(actual, expected);
            energy += actual
                .iter()
                .map(|value| f64::from(*value).powi(2))
                .sum::<f64>();
            frames += 128;
            peak_cpu_ns = peak_cpu_ns.max(
                engine
                    .cmd
                    .audio_metrics()
                    .last_callback
                    .unwrap()
                    .render_cpu_ns
                    .unwrap(),
            );
        }
        let d = &callback.renderer_for_test().decks[1];
        assert!(Arc::ptr_eq(d.audio.as_ref().unwrap(), &original));
        assert_eq!((d.gain, d.pitch), other);
        assert!(d.playing);
        assert_eq!(callback.renderer_for_test().decks[0].key_shift, offset);
    }
    assert!(energy > 0.001);
    println!(
        "KEY_SHIFT_CALLBACK_RECEIPT {}",
        serde_json::json!({"compared_other_deck_frames":frames,"other_deck_max_error":0,"callback_allocations":0,"callback_frees":0,"peak_render_cpu_ns":peak_cpu_ns,"energy":energy,"offsets":14,"physical_devices_opened":false})
    );
}
#[test]
fn shifting_without_key_lock_marks_overlap_positions_and_scratch_reverse_and_stopped_modes_truthfully(
) {
    let (engine, mut rt) = renderer(tone(48000, 440.0), 48000, 1.0, false);
    rt.apply(Command::DeckKeyShift(request(&engine, 0, 2, false)));
    for _ in 0..256 {
        rt.render_deck(0);
    }
    assert_eq!(rt.decks[0].keylock_mode(), Mode::Off);
    assert_eq!(rt.decks[0].key_shift_mode(), Mode::Locked);
    let mut writer = Writer::new();
    let handle = writer.handle();
    writer.begin(48000, Some(1_000_000_000));
    writer.push(&rt.decks);
    writer.finish();
    assert_eq!(handle.positions_at(1_000_000_000).unwrap()[0].media_key, 0);
    rt.apply(Command::DeckTouch { deck: 0, on: true });
    assert_eq!(rt.decks[0].key_shift_mode(), Mode::ScratchBypass);
    rt.apply(Command::DeckTouch { deck: 0, on: false });
    rt.decks[0].rate = -1.0;
    assert_eq!(rt.decks[0].key_shift_mode(), Mode::UnsupportedRate);
    rt.decks[0].playing = false;
    assert_eq!(rt.decks[0].key_shift_mode(), Mode::Stopped);
    rt.decks[0].audio = None;
    assert_eq!(rt.decks[0].key_shift_mode(), Mode::NoMedia);
    rt.publish_for_test();
    assert_eq!(engine.snapshot().decks[0].key_shift_mode, Mode::NoMedia);
    assert_eq!(engine.snapshot().decks[0].key_shift, 2);
}
#[test]
fn chosen_harmonic_targets_choose_the_closest_offset_preserving_major_and_minor() {
    for (source, target, expected) in [
        ("C", "D", 2),
        ("B", "C", 1),
        ("C", "B", -1),
        ("A minor", "G minor", -2),
        ("8B", "2B", 6),
    ] {
        let source = Key::parse(source).unwrap();
        let target = Key::parse(target).unwrap();
        let offset = matching(source, target).unwrap();
        assert_eq!(offset, expected);
        assert_eq!(shifted(source, offset), Some(target));
    }
    assert!(matching(Key::parse("C").unwrap(), Key::parse("Cm").unwrap()).is_err());
    assert!(matching(
        Key {
            tonic: 12,
            minor: false
        },
        Key::parse("C").unwrap()
    )
    .is_err());
}
fn wav(path: &std::path::Path, sr: u32, data: &[f32]) {
    use std::io::Write;
    let bytes = u32::try_from(data.len() * 4).unwrap();
    let mut f = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)
        .unwrap();
    for part in [
        b"RIFF".as_slice(),
        &(36 + bytes).to_le_bytes(),
        b"WAVEfmt ",
        &16_u32.to_le_bytes(),
        &3_u16.to_le_bytes(),
        &2_u16.to_le_bytes(),
        &sr.to_le_bytes(),
        &(sr * 8).to_le_bytes(),
        &8_u16.to_le_bytes(),
        &32_u16.to_le_bytes(),
        b"data",
        &bytes.to_le_bytes(),
    ] {
        f.write_all(part).unwrap();
    }
    for sample in data {
        f.write_all(&sample.to_le_bytes()).unwrap();
    }
    f.sync_all().unwrap();
}
#[test]
fn complete_original_full_mix_renders_shifted_samples_and_bounded_transitions_at_fixed_tempo() {
    let evidence =
        std::env::var_os("OMATAINER_KEY_SHIFT_EVIDENCE_ROOT").map(std::path::PathBuf::from);
    if let Some(root) = &evidence {
        assert!(root.starts_with("/home/"));
        std::fs::create_dir(root).unwrap();
    }
    let mut reports = Vec::new();
    for sr in [44100, 48000, 96000] {
        let (drums, harmony) = crate::engine::demo_stems(sr, 124.0);
        let audio = Arc::new(Sample {
            name: "Original complete drums and harmony mix".into(),
            path: String::new(),
            sr,
            ch: 2,
            bpm: 124.0,
            peaks: vec![].into(),
            spectrum: None,
            data: drums
                .data
                .iter()
                .zip(&harmony.data)
                .map(|(a, b)| 0.35 * (a + b))
                .collect(),
        });
        for offset in [-6, 0, 6] {
            let (engine, mut rt) = renderer(audio.clone(), sr, 1.0, true);
            rt.apply(Command::DeckKeyShift(request(&engine, 0, offset, false)));
            let frames = audio.frames();
            let mut output = Vec::with_capacity(frames * 2);
            let mut max_step = 0.0_f32;
            let mut previous = [0.0_f32; 2];
            for _ in 0..frames {
                let (l, r) = rt.render_deck(0);
                for (channel, x) in [l, r].into_iter().enumerate() {
                    assert!(x.is_finite());
                    max_step = max_step.max((x - previous[channel]).abs());
                    previous[channel] = x;
                    output.push(x);
                }
            }
            assert!(!rt.decks[0].playing);
            assert_eq!(rt.decks[0].natural_end, 1);
            let energy = output.iter().map(|v| f64::from(*v).powi(2)).sum::<f64>();
            assert!(energy > 1.0);
            assert!(max_step < 0.75);
            if let Some(root) = &evidence {
                wav(
                    &root.join(format!("complete-mix-{sr}-{offset:+}.wav")),
                    sr,
                    &output,
                );
            }
            reports.push(serde_json::json!({"sample_rate":sr,"offset":offset,"frames":frames,"natural_ends":1,"energy":energy,"max_adjacent_step":max_step}));
        }
        let (engine, mut rt) = renderer(audio.clone(), sr, 1.0, true);
        let mut output = Vec::new();
        let mut max_step = 0.0_f32;
        let mut previous = [0.0_f32; 2];
        for i in 0..audio.frames() {
            if i == sr as usize * 2 || i == sr as usize * 4 || i == sr as usize * 6 {
                let before = rt.decks[0].last_output;
                let position = rt.decks[0].pos;
                let value = if i == sr as usize * 2 {
                    3
                } else if i == sr as usize * 4 {
                    -3
                } else {
                    0
                };
                rt.apply(Command::DeckKeyShift(request(&engine, 0, value, false)));
                assert_eq!(rt.decks[0].pos, position);
                let _ = rt.render_deck(0);
                assert_eq!(rt.decks[0].last_output, before);
                output.extend_from_slice(&before);
                continue;
            }
            let (l, r) = rt.render_deck(0);
            for (channel, x) in [l, r].into_iter().enumerate() {
                assert!(x.is_finite());
                max_step = max_step.max((x - previous[channel]).abs());
                previous[channel] = x;
                output.push(x);
            }
        }
        assert_eq!(output.len(), audio.frames() * 2);
        assert!(max_step < 0.75);
        assert_eq!(rt.decks[0].key_shift, 0);
        assert_eq!(rt.decks[0].natural_end, 1);
        if let Some(root) = &evidence {
            wav(
                &root.join(format!("complete-mix-transitions-{sr}.wav")),
                sr,
                &output,
            );
        }
    }
    println!(
        "KEY_SHIFT_MIX_RECEIPT {}",
        serde_json::json!({"complete_mix_groups":reports,"transition_groups":3,"transition_offsets":[3,-3,0],"first_transition_sample_preserved":true,"exported_wavs":if evidence.is_some(){12}else{0},"physical_devices_opened":false,"human_listening_claimed":false})
    );
}

#[test]
fn actual_tempo_fader_converges_to_shifted_range_endpoints_and_unity_from_both_directions() {
    let mut checks = 0;
    for output_sr in [44100, 48000, 192000] {
        for shift in [-6, 6] {
            for lock in [false, true] {
                let (low, high) = tempo_range(lock, shift);
                let mut targets = vec![low, high];
                let unity = factor(shift) as f32;
                if lock && (low..=high).contains(&unity) {
                    targets.push(unity);
                }
                for target in targets {
                    for from in [0.0, 1.5] {
                        let (engine, mut rt) = renderer(tone(48000, 997.0), output_sr, 1.0, lock);
                        rt.apply(Command::DeckKeyShift(request(&engine, 0, shift, false)));
                        if from == 0.0 {
                            rt.apply(Command::DeckPlay { deck: 0 });
                        } else {
                            rt.apply(Command::DeckPitch {
                                deck: 0,
                                value: 1.0,
                            });
                        }
                        for _ in 0..4096 {
                            rt.render_deck(0);
                        }
                        rt.apply(Command::DeckPitch {
                            deck: 0,
                            value: target - 0.5,
                        });
                        if from == 0.0 {
                            rt.apply(Command::DeckPlay { deck: 0 });
                        }
                        assert_eq!(
                            rt.decks[0].pitch_rate(),
                            target,
                            "Fader must request the exact target"
                        );
                        for _ in 0..4096 {
                            rt.render_deck(0);
                        }
                        assert_eq!(
                            rt.decks[0].rate, target,
                            "sr={output_sr} shift={shift} lock={lock} from={from}"
                        );
                        let expected = if lock && target == unity {
                            Mode::Unity
                        } else {
                            Mode::Locked
                        };
                        assert_eq!(
                            rt.decks[0].key_shift_mode(),
                            expected,
                            "sr={output_sr} shift={shift} lock={lock} target={target}"
                        );
                        let before = rt.decks[0].pos;
                        rt.render_deck(0);
                        assert!(
                            (rt.decks[0].pos
                                - before
                                - f64::from(target) * 48000.0 / f64::from(output_sr))
                            .abs()
                                < 1e-8
                        );
                        checks += 1;
                    }
                }
            }
        }
    }
    println!(
        "KEY_SHIFT_FADER_RECEIPT {}",
        serde_json::json!({"checks":checks,"output_rates":[44100,48000,192000],"source_rate":48000,"requested_shifts":[-6,6],"actual_pitch_commands":true,"physical_devices_opened":false})
    );
}
