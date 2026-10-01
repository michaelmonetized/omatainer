use super::*;

fn tone(source_sr: u32, frequency: f64) -> Arc<Sample> {
    Arc::new(Sample {
        name: "Original analytical stereo tone".into(),
        sr: source_sr,
        ch: 2,
        data: (0..source_sr as usize * 3)
            .flat_map(|frame| {
                let x = (std::f64::consts::TAU * frequency * frame as f64 / source_sr as f64).sin()
                    as f32
                    * 0.5;
                [x, -x]
            })
            .collect(),
        peaks: Vec::new().into(),
        bpm: 120.0,
        path: String::new(),
    })
}
fn engine_with(source: Arc<Sample>, output_sr: u32, ratio: f32, locked: bool) -> RtEngine {
    let (_, receiver) = crossbeam_channel::bounded(32);
    let mut rt = RtEngine::new(
        output_sr as f32,
        receiver,
        Arc::new(Mutex::new(Snapshot::default())),
    );
    rt.apply(Command::DeckAudio {
        deck: 0,
        audio: source,
    });
    rt.apply(Command::DeckGain {
        deck: 0,
        value: 1.0,
    });
    rt.decks[0].pitch_range = 2;
    rt.decks[0].pitch = ratio - 0.5;
    rt.decks[0].rate = ratio;
    rt.apply(Command::DeckPlay { deck: 0 });
    if locked {
        rt.apply(Command::DeckKeylock { deck: 0 });
    }
    rt
}
fn harmonic_fit(frames: &[f32], sr: u32, hz: f64) -> (f64, f64) {
    let mut sine = 0.0;
    let mut cosine = 0.0;
    let mut energy = 0.0;
    for (index, &sample) in frames.iter().enumerate() {
        let phase = std::f64::consts::TAU * hz * index as f64 / sr as f64;
        sine += sample as f64 * phase.sin();
        cosine += sample as f64 * phase.cos();
        energy += sample as f64 * sample as f64;
    }
    let amplitude = 2.0 * sine.hypot(cosine) / frames.len() as f64;
    let rms = (energy / frames.len() as f64).sqrt();
    (amplitude, rms)
}

#[test]
fn unity_is_bit_exact_with_ordinary_resampling_at_different_source_rates() {
    for (source_sr, output_sr) in [
        (44_100, 48_000),
        (48_000, 44_100),
        (96_000, 48_000),
        (44_100, 192_000),
    ] {
        let source = tone(source_sr, 997.0);
        let mut unlocked = engine_with(source.clone(), output_sr, 1.0, false);
        let mut locked = engine_with(source, output_sr, 1.0, true);
        for _ in 0..output_sr / 4 {
            assert_eq!(locked.render_deck(0), unlocked.render_deck(0));
            assert_eq!(locked.decks[0].pos, unlocked.decks[0].pos);
        }
        assert_eq!(locked.decks[0].keylock_render_mode, keylock::Mode::Unity);
    }
}

#[test]
fn linked_similarity_preserves_bass_amplitude_pitch_and_antiphase_channels() {
    for (source_sr, output_sr) in [
        (44_100, 48_000),
        (48_000, 44_100),
        (48_000, 96_000),
        (192_000, 192_000),
    ] {
        for ratio in [0.5, 0.84, 1.16, 1.5] {
            for hz in [55.0, 93.75, 440.0] {
                let mut rt = engine_with(tone(source_sr, hz), output_sr, ratio, true);
                for _ in 0..output_sr / 5 {
                    rt.render_deck(0);
                }
                let frames: Vec<_> = (0..output_sr)
                    .map(|_| {
                        let (l, r) = rt.render_deck(0);
                        assert_eq!(
                            l, -r,
                            "joint stereo alignment must not use mono cancellation"
                        );
                        l
                    })
                    .collect();
                let (amplitude, rms) = harmonic_fit(&frames, output_sr, hz);
                assert!(amplitude > 0.47 && amplitude < 0.53,
                    "source={source_sr} out={output_sr} ratio={ratio} hz={hz} amplitude={amplitude} rms={rms}");
                assert!(
                    (rms - 0.5 / 2.0_f64.sqrt()).abs() < 0.02,
                    "source={source_sr} out={output_sr} ratio={ratio} hz={hz} RMS={rms}"
                );
            }
        }
    }
}

#[test]
fn renderer_snapshot_reports_the_actual_bypass_predicate_and_mode_changes_allocate_nothing() {
    assert_eq!(keylock::mode(false, true, 2.0), keylock::Mode::Off);
    assert_eq!(
        keylock::mode(true, true, -1.0),
        keylock::Mode::ScratchBypass
    );
    for ratio in [-1.0, 0.0, 0.49, 1.51, f32::INFINITY, f32::NAN] {
        assert_eq!(
            keylock::mode(true, false, ratio),
            keylock::Mode::UnsupportedRate
        );
    }
    let mut rt = engine_with(tone(44_100, 93.75), 48_000, 0.84, true);
    for _ in 0..4096 {
        rt.render_deck(0);
    }
    let counts = test_alloc::measure(|| {
        for ratio in [0.5, 1.0, 1.5, 0.4, 2.0, 0.84] {
            rt.decks[0].sync = true;
            rt.decks[0].sync_bpm = 120.0 * ratio;
            rt.decks[0].rate = ratio;
            for _ in 0..4096 {
                rt.render_deck(0);
            }
            assert_eq!(
                rt.decks[0].keylock_render_mode,
                keylock::mode(true, false, rt.decks[0].rate)
            );
        }
    });
    assert_eq!(counts.allocations, 0);
    assert_eq!(counts.frees, 0);
    rt.publish_for_test();
    // Snapshot plumbing independently reports the current processing predicate.
    assert_eq!(
        rt.snap.lock().decks[0].keylock_mode,
        keylock::mode(true, false, rt.decks[0].rate)
    );
}

#[test]
fn natural_short_loop_keeps_overlap_but_explicit_seek_establishes_a_new_source_fence() {
    let mut rt = engine_with(tone(48_000, 93.75), 48_000, 0.84, true);
    rt.decks[0].loop_on = true;
    rt.decks[0].loop_start = 8192.0;
    rt.decks[0].loop_len = 512.0;
    rt.decks[0].transition_to(8192.0, 48_000.0, DeckTransition::Jump);
    let mut wrapped = 0;
    for frame in 0..8192 {
        let previous_pos = rt.decks[0].pos;
        let previous_phase = rt.decks[0].keylock_dsp.phase;
        let (l, r) = rt.render_deck(0);
        assert_eq!(l, -r);
        if rt.decks[0].pos < previous_pos {
            wrapped += 1;
            assert_eq!(
                rt.decks[0].keylock_dsp.phase,
                previous_phase % rt.decks[0].keylock_dsp.hop + 1
            );
            if frame > 100 {
                assert_eq!(rt.decks[0].transition_remaining, 0);
            }
        }
    }
    assert!(wrapped > 8);
    rt.decks[0].clear_loop();
    rt.decks[0].transition_to(30_000.0, 48_000.0, DeckTransition::Jump);
    for _ in 0..8192 {
        rt.render_deck(0);
        assert!(rt.decks[0].keylock_dsp.origin >= 30_000.0);
    }
}

#[test]
fn stopped_and_empty_decks_report_armed_state_without_a_false_rate_warning() {
    let mut rt = engine_with(tone(44_100, 55.0), 48_000, 0.84, true);
    rt.decks[0].playing = false;
    rt.decks[0].rate = 0.0;
    assert_eq!(rt.decks[0].keylock_mode(), keylock::Mode::Stopped);
    rt.publish_for_test();
    assert_eq!(rt.snap.lock().decks[0].keylock_mode, keylock::Mode::Stopped);
    rt.decks[0].touching = true;
    assert_eq!(rt.decks[0].keylock_mode(), keylock::Mode::ScratchBypass);
    rt.decks[0].audio = None;
    assert_eq!(rt.decks[0].keylock_mode(), keylock::Mode::NoMedia);
    rt.publish_for_test();
    assert_eq!(rt.snap.lock().decks[0].keylock_mode, keylock::Mode::NoMedia);
    rt.decks[0].keylock = false;
    assert_eq!(rt.decks[0].keylock_mode(), keylock::Mode::Off);
}

#[test]
fn independent_stereo_phase_is_preserved_and_silent_channel_cannot_drive_alignment() {
    for output_sr in [44_100, 48_000, 96_000] {
        for ratio in [0.5, 0.84, 1.16, 1.5] {
            for silent_left in [false, true] {
                let mut source = tone(48_000, 93.75);
                let source_mut = Arc::make_mut(&mut source);
                for (index, frame) in source_mut.data.chunks_exact_mut(2).enumerate() {
                    let phase = std::f64::consts::TAU * 93.75 * index as f64 / 48_000.0;
                    frame[0] = if silent_left {
                        0.0
                    } else {
                        phase.sin() as f32 * 0.5
                    };
                    frame[1] = phase.cos() as f32 * 0.25;
                }
                let mut rt = engine_with(source, output_sr, ratio, true);
                for _ in 0..output_sr / 5 {
                    rt.render_deck(0);
                }
                let mut fits = [[0.0_f64; 2]; 2];
                for frame in 0..output_sr {
                    let (l, r) = rt.render_deck(0);
                    if silent_left {
                        assert_eq!(l, 0.0);
                    }
                    let phase = std::f64::consts::TAU * 93.75 * frame as f64 / output_sr as f64;
                    for (channel, sample) in [l, r].into_iter().enumerate() {
                        fits[channel][0] += sample as f64 * phase.sin();
                        fits[channel][1] += sample as f64 * phase.cos();
                    }
                }
                let amplitude = |fit: [f64; 2]| 2.0 * fit[0].hypot(fit[1]) / output_sr as f64;
                assert!((amplitude(fits[1]) - 0.25).abs() < 0.015);
                if !silent_left {
                    assert!((amplitude(fits[0]) - 0.5).abs() < 0.02);
                    let phase = fits[1][1].atan2(fits[1][0]) - fits[0][1].atan2(fits[0][0]);
                    let difference = (phase - std::f64::consts::FRAC_PI_2 + std::f64::consts::PI)
                        .rem_euclid(std::f64::consts::TAU)
                        - std::f64::consts::PI;
                    assert!(
                        difference.abs() < 0.015,
                        "{output_sr} {ratio}: {difference}"
                    );
                }
            }
        }
    }
}

#[test]
fn full_callback_with_two_locked_decks_crosses_analysis_hops_without_heap_work() {
    for sr in [44_100, 48_000, 96_000, 192_000] {
        let source = tone(44_100, 93.75);
        let mut rt = engine_with(source.clone(), sr, 0.84, true);
        rt.apply(Command::DeckAudio {
            deck: 1,
            audio: source,
        });
        rt.decks[1].pitch_range = 2;
        rt.decks[1].pitch = 0.66;
        rt.decks[1].rate = 1.16;
        rt.apply(Command::DeckPlay { deck: 1 });
        rt.apply(Command::DeckKeylock { deck: 1 });
        rt.playing = false;
        let mut callback = audio::OutputCallback::new(rt, 2);
        let mut block = [0.0_f32; 128 * 2];
        for _ in 0..32 {
            callback.render(&mut block);
        }
        let counts = test_alloc::measure(|| {
            for _ in 0..100 {
                callback.render(&mut block);
            }
        });
        assert_eq!(counts.allocations, 0, "rate {sr}");
        assert_eq!(counts.frees, 0, "rate {sr}");
        assert!(block.iter().all(|sample| sample.is_finite()));
    }
}

#[test]
fn unity_and_fallback_retire_overlap_and_restart_from_the_current_transport() {
    let mut rt = engine_with(tone(48_000, 55.0), 48_000, 0.84, true);
    for _ in 0..4000 {
        rt.render_deck(0);
    }
    for ratio in [1.0, 0.5, 1.0, 1.5, 2.0, 0.84] {
        rt.decks[0].sync = true;
        rt.decks[0].sync_bpm = ratio * 120.0;
        rt.decks[0].rate = ratio;
        let previous_position = rt.decks[0].pos;
        rt.render_deck(0);
        assert!((rt.decks[0].pos - previous_position - ratio as f64).abs() < 1e-6);
        assert_eq!(
            rt.decks[0].keylock_render_mode,
            keylock::mode(true, false, ratio)
        );
        if ratio != 1.0 && ratio <= 1.5 {
            assert_eq!(rt.decks[0].keylock_dsp.phase, 1);
            assert_eq!(rt.decks[0].keylock_dsp.origin, rt.decks[0].pos);
        }
        for _ in 0..96 {
            rt.render_deck(0);
        }
        if ratio == 1.0 || ratio > 1.5 {
            // Retain the exact same EQ/filter history in the ordinary renderer;
            // even neutral EQ may round differently from a raw Sample::at.
            rt.decks[1] = rt.decks[0].clone();
            rt.decks[1].keylock = false;
            rt.decks[1].keylock_render_mode = keylock::Mode::Off;
            for _ in 0..200 {
                assert_eq!(rt.render_deck(0), rt.render_deck(1));
                assert_eq!(rt.decks[0].pos, rt.decks[1].pos);
            }
        }
        for _ in 0..2200 {
            rt.render_deck(0);
        }
    }
}

#[test]
fn nonunity_seek_and_hotcue_cannot_search_back_before_the_new_region() {
    for (source_sr, output_sr) in [(44_100, 48_000), (48_000, 96_000)] {
        for ratio in [0.5, 0.84, 1.16, 1.5] {
            for hotcue in [false, true] {
                let mut source = tone(source_sr, 93.75);
                let target = source_sr as usize;
                let source_mut = Arc::make_mut(&mut source);
                for (frame, value) in source_mut.data.chunks_exact_mut(2).enumerate() {
                    let left = if frame < target { 0.75 } else { -0.25 };
                    value.copy_from_slice(&[left, -left * 0.5]);
                }
                let mut rt = engine_with(source, output_sr, ratio, true);
                for _ in 0..output_sr / 8 {
                    rt.render_deck(0);
                }
                assert!((rt.decks[0].last_output[0] - 0.75).abs() < 1e-6);
                if hotcue {
                    rt.decks[0].hotcues[0] = HotCue {
                        set: true,
                        pos: target as f64,
                    };
                    rt.apply(Command::DeckHotCue {
                        deck: 0,
                        pad: 0,
                        del: false,
                    });
                } else {
                    rt.apply(Command::DeckSeek {
                        deck: 0,
                        frac: 1.0 / 3.0,
                    });
                }
                let fence = rt.decks[0].pos;
                let settling = (output_sr as usize).div_ceil(500);
                for frame in 0..output_sr / 4 {
                    let (l, r) = rt.render_deck(0);
                    assert!(rt.decks[0].keylock_dsp.origin >= fence);
                    if frame as usize >= settling {
                        assert!((l + 0.25).abs() < 1e-6,
                            "{source_sr}/{output_sr} ratio={ratio} hotcue={hotcue} frame={frame}: {l}");
                        assert!((r - 0.125).abs() < 1e-6);
                    }
                }
            }
        }
    }
}

#[test]
fn declared_geometry_uses_the_actual_rounded_window_at_each_output_rate() {
    for sr in [44_100, 48_000, 96_000, 192_000] {
        let actual = keylock::Processor::new(sr as f32);
        let declared = keylock::geometry(sr);
        assert_eq!(declared.hop_frames, actual.hop);
        assert_eq!(declared.window_frames, 2 * actual.hop);
        assert_eq!(declared.output_fifo_frames, 0);
        assert_eq!(declared.search_seconds, 0.025);
        assert_eq!(
            declared.max_source_lookahead_seconds,
            declared.search_seconds + 2.0 * actual.hop as f64 / sr as f64
        );
        assert_eq!(
            declared.max_content_displacement_seconds,
            declared.search_seconds + actual.hop as f64 / sr as f64
        );
    }
}

#[test]
fn actual_play_and_fader_centering_converge_to_published_unity_without_setting_rate() {
    for (source_sr, output_sr) in [
        (44_100, 48_000),
        (48_000, 44_100),
        (48_000, 96_000),
        (44_100, 192_000),
    ] {
        let (_, rx) = crossbeam_channel::bounded(16);
        let mut rt = RtEngine::new(
            output_sr as f32,
            rx,
            Arc::new(Mutex::new(Snapshot::default())),
        );
        rt.apply(Command::DeckAudio {
            deck: 0,
            audio: tone(source_sr, 997.0),
        });
        rt.apply(Command::DeckKeylock { deck: 0 });
        // The normal stopped renderer lets the smoothed rate decay. No private
        // rate assignment forces the subsequent Play into the desired branch.
        for _ in 0..4096 {
            rt.render_deck(0);
        }
        assert_eq!(rt.decks[0].keylock_mode(), keylock::Mode::Stopped);
        rt.apply(Command::DeckPlay { deck: 0 });
        for pitch in [0.5, 0.0, 0.5, 1.0, 0.5] {
            rt.apply(Command::DeckPitch {
                deck: 0,
                value: pitch,
            });
            for _ in 0..4096 {
                rt.render_deck(0);
            }
            rt.publish_for_test();
            if pitch == 0.5 {
                assert_eq!(rt.decks[0].rate, 1.0);
                assert_eq!(rt.snap.lock().decks[0].keylock_mode, keylock::Mode::Unity);
                rt.decks[1] = rt.decks[0].clone();
                rt.decks[1].keylock = false;
                rt.decks[1].keylock_render_mode = keylock::Mode::Off;
                for _ in 0..256 {
                    assert_eq!(rt.render_deck(0), rt.render_deck(1));
                }
            } else {
                assert_eq!(rt.snap.lock().decks[0].keylock_mode, keylock::Mode::Locked);
            }
        }
    }
}

#[test]
fn exact_supported_endpoints_converge_from_either_side_but_outside_targets_do_not() {
    for output_sr in [44_100, 48_000, 96_000, 192_000] {
        let mut rt = engine_with(tone(48_000, 93.75), output_sr, 0.84, true);
        rt.apply(Command::DeckSync { deck: 0 });
        for target in [0.4, 0.5, 0.6, 0.5, 1.6, 1.5, 1.4, 1.5, 1.50001, 0.49999] {
            rt.decks[0].sync_bpm = 120.0 * target;
            for _ in 0..4096 {
                rt.render_deck(0);
            }
            rt.publish_for_test();
            if target == 0.5 || target == 1.5 {
                assert_eq!(rt.decks[0].rate, target, "output_sr={output_sr}");
                assert_eq!(rt.snap.lock().decks[0].keylock_mode, keylock::Mode::Locked);
            } else if !(0.5..=1.5).contains(&target) {
                assert_eq!(
                    rt.snap.lock().decks[0].keylock_mode,
                    keylock::Mode::UnsupportedRate,
                    "output_sr={output_sr}, target={target}"
                );
            }
        }
    }
}

#[test]
fn sub_bass_twenty_to_forty_hz_keeps_fundamental_and_energy_at_supported_ratios() {
    let mut failures = Vec::new();
    for (source_sr, output_sr) in [(48_000, 48_000), (44_100, 96_000)] {
        for ratio in [0.5, 0.84, 1.16, 1.5] {
            for hz in [20.0, 27.5, 40.0] {
                let mut rt = engine_with(tone(source_sr, hz), output_sr, ratio, true);
                for _ in 0..output_sr / 5 {
                    rt.render_deck(0);
                }
                let frames: Vec<_> = (0..output_sr).map(|_| rt.render_deck(0).0).collect();
                let (amplitude, rms) = harmonic_fit(&frames, output_sr, hz);
                if !(0.47..0.53).contains(&amplitude) || (rms - 0.5 / 2.0_f64.sqrt()).abs() >= 0.02
                {
                    failures.push(format!("{source_sr}/{output_sr} ratio={ratio} hz={hz}: amplitude={amplitude} RMS={rms}"));
                }
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn upper_band_tones_do_not_alias_the_bounded_correlation_probes() {
    let mut failures = Vec::new();
    for ratio in [0.5, 0.84, 1.16, 1.5] {
        for hz in [1000.0, 6000.0, 8000.0, 12_000.0] {
            let mut rt = engine_with(tone(48_000, hz), 48_000, ratio, true);
            for _ in 0..9600 {
                rt.render_deck(0);
            }
            let frames: Vec<_> = (0..48_000).map(|_| rt.render_deck(0).0).collect();
            let (amplitude, rms) = harmonic_fit(&frames, 48_000, hz);
            // Existing linear source interpolation can attenuate upper-band
            // amplitude. Require preserved pitch/coherence independently of
            // that known resampler response, while rejecting lost energy.
            let coherent_fraction = amplitude / (rms * 2.0_f64.sqrt());
            if coherent_fraction < 0.95 || rms < 0.16 {
                failures.push(format!("ratio={ratio} hz={hz}: amplitude={amplitude} RMS={rms} coherence={coherent_fraction}"));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn search_instrumentation_distinguishes_silent_hops_from_correlation_work() {
    for audible in [false, true] {
        let mut source = tone(48_000, 93.75);
        if !audible {
            Arc::make_mut(&mut source).data.fill(0.0);
        }
        let mut rt = engine_with(source, 48_000, 0.84, true);
        let counts = test_alloc::measure(|| {
            for _ in 0..8192 {
                rt.render_deck(0);
            }
        });
        assert_eq!(counts.allocations, 0);
        assert_eq!(counts.frees, 0);
        let dsp = &rt.decks[0].keylock_dsp;
        let hops = dsp.analysis_count();
        let searches = dsp.full_search_count();
        assert!(hops > 0);
        assert_eq!(searches, if audible { hops } else { 0 });
        let position = rt.decks[0].pos;
        rt.decks[0].transition_to(position, 48_000.0, DeckTransition::Jump);
        assert_eq!(rt.decks[0].keylock_dsp.analysis_count(), hops);
        assert_eq!(rt.decks[0].keylock_dsp.full_search_count(), searches);
        for _ in 0..=rt.decks[0].keylock_dsp.hop {
            rt.render_deck(0);
        }
        assert_eq!(rt.decks[0].keylock_dsp.analysis_count(), hops + 1);
        assert_eq!(
            rt.decks[0].keylock_dsp.full_search_count(),
            searches + u64::from(audible)
        );
    }
}
