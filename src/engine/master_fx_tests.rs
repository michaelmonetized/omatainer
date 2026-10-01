use super::master_fx::MasterSlot;
use super::*;

fn engine(sr: u32) -> (RtEngine, CommandPort, midi::MidiHub) {
    let (commands, rx) = CommandPort::channel(256);
    let rt = RtEngine::new(sr as f32, rx, Arc::new(Mutex::new(Snapshot::default())));
    (rt, commands, midi::MidiHub::without_devices())
}
fn message(hub: &midi::MidiHub, commands: &CommandPort, bytes: &[u8]) {
    hub.receive_for_test(commands, 551, "Numark NS7FX", bytes);
}

fn signal(rt: &mut RtEngine, frames: usize) {
    // Independent channels and frequencies; quiet preroll settles both deck
    // transitions and the sample-rate-scaled 5 ms mixer control ramp.
    let settle = 256.max((rt.sr * 0.005).ceil() as usize);
    let mut data = vec![0.0; (settle + 1) * 2];
    for frame in 0..frames {
        let phase = std::f32::consts::TAU * frame as f32 / rt.sr;
        data.push((phase * 311.0).sin() * 0.2 + if frame == 0 { 0.15 } else { 0.0 });
        data.push((phase * 5011.0).sin() * 0.13);
    }
    data.extend([0.0; 2]);
    rt.apply(Command::DeckAudio {
        deck: 0,
        audio: Arc::new(Sample {
            name: "legacy FX test".into(),
            sr: rt.sr as u32,
            ch: 2,
            data,
            peaks: vec![].into(),
            bpm: 120.0,
            path: String::new(),
        }),
    });
    rt.decks[0].playing = true;
    rt.decks[0].rate = 1.0; // fixture starts at settled neutral speed
    rt.decks[0].gain = 1.0;
    rt.decks[1].audio = None;
    rt.master = 1.0;
    rt.xfader = 0.0;
    rt.process(&mut vec![0.0; settle * 2]);
}

#[test]
fn mapped_selectors_cycle_real_processors_and_publish_reviewable_types() {
    for sr in [44100, 48000, 96000] {
        for slot in 0..3 {
            let (mut rt, commands, hub) = engine(sr);
            let mut outputs = Vec::new();
            for _ in 0..3 {
                // Retire the previous source's normal 2 ms deck fade before
                // selecting/resetting the next processor under measurement.
                rt.apply(Command::DeckUnload { deck: 0 });
                rt.process(&mut [0.0; 512]);
                let previous = rt.fx_kind;
                message(&hub, &commands, &[0x90, 0x3a + slot, 0x7f]);
                message(&hub, &commands, &[0xb0, 0x30 + slot, 127]);
                rt.process(&mut []);
                assert_eq!(rt.fx_kind[slot as usize], previous[slot as usize].next());
                for other in 0..3 {
                    if other != slot as usize {
                        assert_eq!(rt.fx_kind[other], previous[other]);
                    }
                }
                rt.publish_for_test();
                assert_eq!(rt.snap.lock().fx_kind, rt.fx_kind);
                assert_eq!(rt.snap.lock().fx_wet[slot as usize], 1.0);
                signal(&mut rt, sr as usize / 2);
                let mut output = vec![0.0; sr as usize];
                rt.process(&mut output);
                assert!(output.iter().all(|value| value.is_finite()));
                assert!(output.iter().any(|value| value.abs() > 1e-4));
                // Independent raw mono processors verify actual engine routing,
                // not just changed sound or a second copy of the slot wrapper.
                let mut echo: [Delay; 2] = std::array::from_fn(|_| {
                    let mut delay = Delay::new(sr as usize * 2);
                    delay.time_samples = (sr as f64 * 60.0 / rt.bpm as f64 * 0.75) as f32;
                    delay.mix = 1.0;
                    delay
                });
                let mut reverb: [Reverb; 2] = std::array::from_fn(|_| {
                    let mut reverb = Reverb::at_sample_rate(sr as f32);
                    reverb.mix = 1.0;
                    reverb
                });
                let mut filter = [[dsp::OnePole::lpf(sr as f32, 1000.0); 2]; 2];
                let mut eq = [ThreeBand::new(sr as f32); 2];
                for (frame, actual) in output.chunks_exact(2).enumerate() {
                    let phase = std::f32::consts::TAU * frame as f32 / sr as f32;
                    let input = [
                        (phase * 311.0).sin() * 0.2 + if frame == 0 { 0.15 } else { 0.0 },
                        (phase * 5011.0).sin() * 0.13,
                    ];
                    for channel in 0..2 {
                        let input = eq[channel].tick(input[channel]);
                        let expected = match rt.fx_kind[slot as usize] {
                            FxKind::Echo => echo[channel].tick(input),
                            FxKind::Reverb => reverb[channel].tick(input),
                            FxKind::Filter => {
                                let [a, b] = &mut filter[channel];
                                b.tick(a.tick(input))
                            }
                        }
                        .tanh();
                        assert!((actual[channel] - expected).abs() < 2e-6,
                            "wrong processor: slot{slot} {:?} {sr} frame{frame} ch{channel}: {} vs {expected}", rt.fx_kind[slot as usize], actual[channel]);
                    }
                }
                for previous in &outputs {
                    let difference: f32 = output
                        .iter()
                        .zip(previous)
                        .map(|(a, b): (&f32, &f32)| (a - b).abs())
                        .sum();
                    assert!(
                        difference > 1.0,
                        "selector did not change output for slot {slot}"
                    );
                }
                outputs.push(output);
            }
            let kinds = rt.fx_kind;
            for bytes in [[0x80, 0x3a + slot, 127], [0x90, 0x3a + slot, 0]] {
                message(&hub, &commands, &bytes);
            }
            rt.process(&mut []);
            assert_eq!(rt.fx_kind, kinds, "release retriggered selector");
        }
    }
}

#[test]
fn each_slot_matches_independent_stereo_processors_and_has_no_shared_history() {
    for sr in [44100.0, 48000.0, 96000.0] {
        let mut slots: [MasterSlot; 3] = std::array::from_fn(|_| MasterSlot::new(sr));
        for kind in [FxKind::Echo, FxKind::Reverb, FxKind::Filter] {
            let mut echo: [[Delay; 2]; 3] = std::array::from_fn(|_| {
                std::array::from_fn(|_| {
                    let mut delay = Delay::new((sr * 2.0) as usize);
                    delay.time_samples = 6.0;
                    delay.mix = 1.0;
                    delay
                })
            });
            let mut reverb: [[Reverb; 2]; 3] = std::array::from_fn(|_| {
                std::array::from_fn(|_| {
                    let mut reverb = Reverb::at_sample_rate(sr);
                    reverb.mix = 1.0;
                    reverb
                })
            });
            let mut filter = [[[dsp::OnePole::lpf(sr, 1000.0); 2]; 2]; 3];
            for slot in &mut slots {
                slot.reset(kind);
                slot.configure(1.0, 8.0);
            }
            for frame in 0..12000 {
                for slot in 0..3 {
                    let input = if frame == slot {
                        [0.2 + slot as f32 * 0.1, 0.0]
                    } else {
                        [0.0; 2]
                    };
                    let actual = slots[slot].process(input, kind, 1.0);
                    let expected = std::array::from_fn(|channel| match kind {
                        FxKind::Echo => echo[slot][channel].tick(input[channel]),
                        FxKind::Reverb => reverb[slot][channel].tick(input[channel]),
                        FxKind::Filter => {
                            let [a, b] = &mut filter[slot][channel];
                            b.tick(a.tick(input[channel]))
                        }
                    });
                    assert_eq!(actual, expected);
                    assert_eq!(actual[1], 0.0, "cross-channel feedback");
                }
            }
        }
    }
}

#[test]
fn fractional_delay_resets_match_fresh_zeroed_storage_across_wraps_and_clones() {
    for capacity in [64, 511, 4800] {
        for time in [1.25, 1.5, 7.75, capacity as f32 - 2.5] {
            let mut used = Delay::new(capacity);
            used.time_samples = time;
            used.fb = 0.93;
            used.mix = 0.73;
            for i in 0..capacity * 3 {
                used.tick((i as f32 * 0.31).sin());
            }
            let allocations = test_alloc::measure(|| used.reset_history());
            assert_eq!((allocations.allocations, allocations.frees), (0, 0));
            let mut fresh = Delay::new(capacity);
            fresh.time_samples = time;
            fresh.fb = used.fb;
            fresh.mix = used.mix;
            for i in 0..capacity * 4 {
                let x = if i < 9 { (i as f32 * 0.2).cos() } else { 0.0 };
                assert_eq!(
                    used.tick(x),
                    fresh.tick(x),
                    "capacity={capacity} time={time} frame={i}"
                );
                if i == capacity / 2 || i == capacity * 2 + 3 {
                    let mut clone = used.clone();
                    let mut reference = fresh.clone();
                    for _ in 0..capacity * 2 {
                        assert_eq!(clone.tick(0.0), reference.tick(0.0));
                    }
                }
            }
        }
    }
}

#[test]
fn selection_resets_tails_without_allocation_and_rate_changes_preserve_controls() {
    let (mut rt, commands, _) = engine(48000);
    for slot in 0..3 {
        rt.fx_wet[slot] = 0.7;
    }
    rt.process(&mut [0.0; 128]);
    for kind in [FxKind::Echo, FxKind::Reverb, FxKind::Filter] {
        let mut processor = MasterSlot::new(48000.0);
        processor.configure(1.0, 64.0);
        for i in 0..20000 {
            processor.process(if i < 1024 { [0.4, -0.2] } else { [0.0; 2] }, kind, 1.0);
        }
        let counts = test_alloc::measure(|| processor.reset(kind));
        assert_eq!((counts.allocations, counts.frees), (0, 0));
        for _ in 0..20000 {
            assert_eq!(processor.process([0.0; 2], kind, 1.0), [0.0; 2]);
        }
    }
    let counts = test_alloc::measure(|| {
        for _ in 0..30 {
            for slot in 0..3 {
                rt.apply(Command::FxSelect { slot });
            }
        }
    });
    assert_eq!((counts.allocations, counts.frees), (0, 0));
    let kinds = rt.fx_kind;
    let wet = rt.fx_wet;
    rt.set_sample_rate(96000);
    assert_eq!(rt.fx_kind, kinds);
    assert_eq!(rt.fx_wet, wet);
    rt.process(&mut [0.0; 128]);
    for slot in 0..3 {
        assert_eq!(
            rt.master_fx[slot].echo[0].time_samples,
            96000.0 * 60.0 / rt.bpm * 0.75
        );
    }
    for slot in [3, 255] {
        assert_eq!(
            commands.send(Command::FxSelect { slot }),
            Err(SubmissionError::InvalidTarget)
        );
        rt.apply(Command::FxSelect { slot });
        assert_eq!(rt.fx_kind, kinds);
    }
}
