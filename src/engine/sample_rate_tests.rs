use super::*;
use dsp::{Env, OnePole};
use fx::{FxChain, FxId, FxSlot};

fn engine() -> RtEngine {
    let (_tx, rx) = CommandPort::channel(256);
    RtEngine::new(48_000.0, rx, Arc::new(Mutex::new(Snapshot::default())))
}

fn envelope_durations(mut env: Env, sr: f32) -> [f64; 3] {
    env.on();
    let mut frames = [0usize; 3];
    while env.stage == 1 {
        env.tick();
        frames[0] += 1;
        assert!(frames[0] < sr as usize);
    }
    while env.stage == 2 {
        env.tick();
        frames[1] += 1;
        assert!(frames[1] < sr as usize);
    }
    // The existing linear release coefficient specifies full-scale duration.
    env.level = 1.0;
    env.off();
    while env.active() {
        env.tick();
        frames[2] += 1;
        assert!(frames[2] < sr as usize);
    }
    frames.map(|frames| frames as f64 / sr as f64)
}

#[test]
fn sample_rate_updates_all_instrument_and_sampler_adsrs_and_oscillator_hz() {
    for sr in [44_100, 96_000] {
        let mut rt = engine();
        rt.apply(Command::SamplerInst(2));
        rt.sampler_poly.cutoff = 2700.0;
        rt.set_sample_rate(sr);
        for poly in rt
            .tracks
            .iter()
            .map(|track| &track.poly)
            .chain([&rt.sampler_poly])
        {
            let expected = match poly.kind {
                0 => [0.005, 0.18, 0.12],
                1 => [0.008, 0.22, 0.28],
                _ => [0.04, 0.4, 0.8],
            };
            for voice in &poly.voices {
                let measured = envelope_durations(voice.env, rt.sr);
                for (actual, expected) in measured.into_iter().zip(expected) {
                    assert!(
                        (actual - expected).abs() < 0.002,
                        "kind {}, {sr} Hz: {actual} vs {expected} seconds",
                        poly.kind
                    );
                }
            }
        }
        assert_eq!(rt.sampler_poly.kind, 2);
        assert_eq!(rt.sampler_poly.cutoff, 2700.0);
        let voice = &mut rt.sampler_poly.voices[0];
        voice.trig(69, 1.0);
        let mut filter = Svf::default();
        let mut periods = 0;
        for _ in 0..sr {
            let before = voice.phase;
            voice.tick(rt.sr, 2700.0, &mut filter);
            periods += usize::from(voice.phase < before);
        }
        assert!(
            (periods as i32 - 440).abs() <= 1,
            "{sr} Hz: {periods} oscillator periods"
        );
        for sample in rt.tracks[0]
            .drum_samples
            .iter()
            .chain(rt.pad_banks.iter().flatten())
        {
            assert_eq!(sample.sr, sr);
        }
    }
}

fn rack(sr: f32) -> FxChain {
    let mut rack = FxChain::new(sr);
    for (index, id) in FxId::all().iter().copied().enumerate() {
        let mut slot = FxSlot::new(id, sr);
        slot.mix = 0.21 + index as f32 * 0.035;
        slot.p = [0.7, 0.17, 0.62, 0.3];
        slot.on = index % 3 != 0;
        rack.slots.push(slot);
    }
    rack
}

#[test]
fn sample_rate_rebuilds_every_track_and_scene_slot_preserving_controls() {
    for sr in [44_100, 96_000] {
        let mut rt = engine();
        for chain in rt
            .tracks
            .iter_mut()
            .map(|track| &mut track.fx)
            .chain([&mut rt.scene_fx])
        {
            *chain = rack(48_000.0);
            for slot in &mut chain.slots {
                slot.on = true;
            }
            for frame in 0..1000 {
                chain.process_stereo([if frame == 0 { 0.8 } else { 0.0 }, -0.1], 48_000.0);
            }
            for (index, slot) in chain.slots.iter_mut().enumerate() {
                slot.on = index % 3 != 0;
            }
        }
        rt.set_sample_rate(sr);
        for chain in rt
            .tracks
            .iter_mut()
            .map(|track| &mut track.fx)
            .chain([&mut rt.scene_fx])
        {
            let mut reference = rack(sr as f32);
            for (actual, expected) in chain.slots.iter_mut().zip(&mut reference.slots) {
                assert_eq!(
                    (actual.id(), actual.on, actual.mix, actual.p),
                    (expected.id(), expected.on, expected.mix, expected.p)
                );
                // Bypassed slots must have reset too, not retain an old-rate tail.
                actual.on = true;
                expected.on = true;
            }
            for frame in 0..2048 {
                let t = frame as f32 / sr as f32;
                let input = if frame == 0 {
                    [0.9, -0.2]
                } else {
                    [
                        (t * 311.0 * std::f32::consts::TAU).sin() * 0.1,
                        (t * 977.0 * std::f32::consts::TAU).sin() * 0.2,
                    ]
                };
                assert_eq!(
                    chain.process_stereo(input, sr as f32),
                    reference.process_stereo(input, sr as f32)
                );
            }
        }
    }
}

fn effect(id: FxId, sr: u32) -> FxChain {
    let mut chain = FxChain::new(48_000.0);
    let mut slot = FxSlot::new(id, 48_000.0);
    slot.mix = 1.0;
    slot.p[1] = 0.0;
    if id == FxId::Spread {
        slot.p[0] = 1.0;
    }
    chain.slots.push(slot);
    chain.process_stereo([1.0, -1.0], 48_000.0);
    chain.set_sample_rate(sr as f32);
    chain
}

#[test]
fn sample_rate_delay_reverb_and_haas_arrivals_keep_their_durations() {
    for sr in [44_100, 96_000] {
        for (id, seconds, channel) in [
            (FxId::Delay, 0.25, 0),
            (FxId::Reverb, 887.0 / 48_000.0, 0),
            (FxId::Spread, 0.006, 1),
        ] {
            let mut chain = effect(id, sr);
            let mut arrival = None;
            for frame in 0..(sr as f64 * seconds).ceil() as usize + 8 {
                let y =
                    chain.process_stereo(if frame == 0 { [1.0, 1.0] } else { [0.0; 2] }, sr as f32);
                if y[channel].abs() > 1e-5 && arrival.is_none() {
                    arrival = Some(frame);
                }
            }
            let arrival = arrival.expect("effect emitted no impulse");
            let expected = sr as f64 * seconds;
            assert!(
                (arrival as f64 - expected).abs() <= 1.01,
                "{id:?}, {sr}: {arrival} vs {expected}"
            );
        }

        // The master uses tempo duration rather than the slot's fixed 250 ms.
        let mut rt = engine();
        rt.set_sample_rate(sr);
        rt.bpm = 120.0;
        rt.fx_wet = [1.0, 1.0, 0.0];
        rt.process(&mut []);
        for channel in 0..2 {
            for (processor, expected) in [
                (0, (sr as f64 * 0.375).floor() as usize),
                (1, (sr as f64 * 887.0 / 48_000.0).round() as usize),
            ] {
                let mut arrival = None;
                for frame in 0..=expected + 1 {
                    let input = if frame == 0 { 1.0 } else { 0.0 };
                    let value = if processor == 0 {
                        rt.delay[channel].tick(input)
                    } else {
                        rt.reverb[channel].tick(input)
                    };
                    if value.abs() > 1e-5 && arrival.is_none() {
                        arrival = Some(frame);
                    }
                }
                assert_eq!(
                    arrival,
                    Some(expected),
                    "master {processor}, channel {channel}, {sr}"
                );
            }
        }
    }
}

#[test]
fn sample_rate_comp_and_gate_detector_durations_are_preserved() {
    for sr in [44_100, 96_000] {
        for (id, blend, threshold) in [(FxId::Comp, 0.005f64, 0.21f64), (FxId::Gate, 0.02, 0.5)] {
            let mut chain = effect(id, sr);
            chain.slots[0].p = if id == FxId::Comp {
                [0.4, 1.0, 0.0, 0.0]
            } else {
                [0.5, 0.0, 0.0, 0.0]
            };
            let expected_seconds = (1.0 - threshold).ln() / (1.0 - blend).ln() / 48_000.0;
            let crossing = (0..sr / 100)
                .find(|_| {
                    let value = chain.process_stereo([1.0, 1.0], sr as f32)[0];
                    if id == FxId::Comp {
                        value < 0.9999999
                    } else {
                        value > 0.5
                    }
                })
                .unwrap();
            assert!(
                (crossing as f64 / sr as f64 - expected_seconds).abs() <= 1.01 / sr as f64,
                "{id:?}, {sr}: crossed at frame {crossing}, expected {expected_seconds} seconds"
            );
        }
    }
}

#[test]
fn sample_rate_chorus_matches_independent_variable_delay_in_seconds() {
    for sr in [44_100, 96_000] {
        let mut chain = effect(FxId::Chorus, sr);
        let mut delays = std::array::from_fn::<_, 2, _>(|_| {
            let mut delay = Delay::new((sr as f32 * 0.05) as usize);
            delay.mix = 1.0;
            delay.fb = 0.0;
            delay
        });
        let mut phase = 0.0f32;
        let mut first_echo = None;
        for frame in 0..sr * 2 {
            let input = if frame == 0 {
                [1.0, -0.5]
            } else if frame < sr / 2 {
                [0.0; 2]
            } else {
                let t = frame as f32 / sr as f32;
                [
                    (t * 220.0 * std::f32::consts::TAU).sin() * 0.1,
                    (t * 731.0 * std::f32::consts::TAU).sin() * 0.2,
                ]
            };
            phase = (phase + 0.7 / sr as f32) % 1.0;
            let expected = std::array::from_fn(|channel| {
                delays[channel].time_samples =
                    sr as f32 * (0.008 + 0.006 * (phase * std::f32::consts::TAU).sin());
                delays[channel].tick(input[channel])
            });
            let actual = chain.process_stereo(input, sr as f32);
            assert_eq!(actual, expected);
            if first_echo.is_none() && actual[0].abs() > 1e-5 {
                first_echo = Some(frame);
            }
        }
        // First modulated 8 ms tap is near 8.22 ms at 0.7 Hz, within two
        // output samples of its continuous-time delay equation.
        let t = first_echo.unwrap() as f64 / sr as f64;
        let delay = 0.008 + 0.006 * (std::f64::consts::TAU * 0.7 * t).sin();
        assert!((t - delay).abs() < 2.0 / sr as f64);
    }
}

fn rms(chain: &mut FxChain, sr: u32, hz: f32) -> f64 {
    let mut energy = 0.0;
    for frame in 0..sr / 5 {
        let x = (frame as f32 / sr as f32 * hz * std::f32::consts::TAU).sin();
        let y = chain.process_stereo([x, x], sr as f32)[0];
        if frame >= sr / 10 {
            energy += y as f64 * y as f64;
        }
    }
    (energy / (sr / 10) as f64).sqrt()
}

#[test]
fn sample_rate_filters_keep_cutoff_hz_and_controls() {
    for sr in [44_100, 96_000] {
        let mut rt = engine();
        rt.tracks[2].eq.low_g = 0.3;
        rt.tracks[2].eq.mid_g = 1.7;
        rt.tracks[2].eq.high_g = 0.6;
        rt.decks[0].eq[0].low_g = 0.4;
        rt.set_sample_rate(sr);
        assert_eq!(
            [
                rt.tracks[2].eq.low_g,
                rt.tracks[2].eq.mid_g,
                rt.tracks[2].eq.high_g
            ],
            [0.3, 1.7, 0.6]
        );
        assert_eq!(rt.decks[0].eq[0].low_g, 0.4);
        for eq in rt
            .tracks
            .iter()
            .map(|t| &t.eq)
            .chain(rt.decks.iter().flat_map(|d| &d.eq))
        {
            for (mut filter, hz) in [(eq.low, 250.0), (eq.high, 3200.0)] {
                let initial = filter.tick(1.0);
                let frames = (sr as f64 / (std::f64::consts::TAU * hz)).round() as usize;
                let mut last = initial;
                for _ in 0..frames {
                    last = filter.tick(0.0);
                }
                let expected = (-std::f64::consts::TAU * hz * frames as f64 / sr as f64).exp();
                assert!((last as f64 / initial as f64 - expected).abs() < 1e-5);
                assert_eq!(filter.a, OnePole::lpf(sr as f32, hz as f32).a);
            }
        }
        for id in [FxId::Filter, FxId::Eq3, FxId::Eq5, FxId::Eq8] {
            for hz in [80.0, 250.0, 1000.0] {
                let configure = |chain: &mut FxChain| {
                    chain.slots[0].p = [0.1, 0.11, 0.9, 0.0];
                };
                let mut reference = effect(id, 48_000);
                configure(&mut reference);
                let mut changed = effect(id, sr);
                configure(&mut changed);
                let expected = rms(&mut reference, 48_000, hz);
                let measured = rms(&mut changed, sr, hz);
                assert!(
                    (measured / expected - 1.0).abs() < 0.04,
                    "{id:?}, {sr}, {hz} Hz: {measured} vs {expected}"
                );
            }
        }
    }
}

#[test]
fn sample_rate_svf_upper_cutoff_stays_at_the_original_frequency() {
    let cutoff_hz = 48_000.0 * 0.45 / std::f64::consts::PI;
    for sr in [44_100, 48_000, 96_000] {
        let mut filter = Svf::default();
        let mut input_energy = 0.0;
        let mut output_energy = 0.0;
        for frame in 0..sr {
            let x = (frame as f64 / sr as f64 * cutoff_hz * std::f64::consts::TAU).sin() as f32;
            let y = filter.process(x, 20_000.0, 0.0, sr as f32, 0.0);
            if frame >= sr / 2 {
                input_energy += x as f64 * x as f64;
                output_energy += y as f64 * y as f64;
            }
        }
        // At its cutoff a critically damped SVF has gain 1/2. This independent
        // frequency probe catches a stale normalized clamp at either rate.
        let gain = (output_energy / input_energy).sqrt();
        assert!(
            (gain - 0.5).abs() < 0.002,
            "{sr} Hz: {cutoff_hz} Hz gain {gain}"
        );
    }
}

#[test]
fn sample_rate_reset_policy_and_equal_rate_noop_are_explicit() {
    let mut rt = engine();
    rt.sampler_poly.note_on(72, 1.0);
    rt.sampler_poly.tick(rt.sr);
    rt.scene_fx = rack(rt.sr);
    rt.scene_fx.process_stereo([0.5, -0.2], rt.sr);
    let mut reference = rt.scene_fx.clone();
    let phase = rt.sampler_poly.voices[0].phase;
    let level = rt.sampler_poly.voices[0].env.level;
    let counts = test_alloc::measure(|| {
        rt.set_sample_rate(48_000);
        rt.set_sample_rate(0);
    });
    assert_eq!(counts, test_alloc::Counts::default());
    assert_eq!(rt.sampler_poly.voices[0].phase, phase);
    assert_eq!(rt.sampler_poly.voices[0].env.level, level);
    assert_eq!(
        rt.scene_fx.process_stereo([0.0; 2], rt.sr),
        reference.process_stereo([0.0; 2], rt.sr)
    );

    rt.quant = 0.0;
    rt.apply(Command::SetNotes {
        track: 2,
        scene: 0,
        notes: vec![MidiNote {
            pitch: 60,
            start: 0.0,
            len: 4.0,
            vel: 100,
        }],
    });
    rt.apply(Command::LaunchClip { track: 2, scene: 0 });
    rt.process(&mut [0.0; 2048]);
    rt.trig_drum(0, 36, 1.0);
    rt.pad_voices[0] = Some((rt.pad_banks[0][0].clone(), 12.0, 1.0));
    let beat = rt.beat;
    let start = rt.tracks[2].playing.unwrap().start_beat;
    let sample = rt.decks[0].audio.clone().unwrap();
    let position = rt.decks[0].pos;
    rt.set_sample_rate(96_000);
    assert_eq!(rt.beat, beat);
    assert_eq!(rt.tracks[2].playing.unwrap().start_beat, start);
    assert_eq!(rt.decks[0].pos, position);
    assert!(Arc::ptr_eq(rt.decks[0].audio.as_ref().unwrap(), &sample));
    assert!(rt.tracks.iter().all(|track| track
        .poly
        .voices
        .iter()
        .all(|voice| !voice.env.active())));
    assert!(rt
        .sampler_poly
        .voices
        .iter()
        .all(|voice| !voice.env.active()));
    assert!(rt.pad_voices.iter().all(Option::is_none));
    assert!(rt
        .tracks
        .iter()
        .all(|track| track.drum_pos.iter().all(Option::is_none)));
    assert!(rt.decks.iter().all(|deck| deck.last_output == [0.0; 2]));
    rt.process(&mut [0.0; 2]);
    assert!(rt.tracks[2]
        .poly
        .voices
        .iter()
        .any(|voice| voice.note == 60 && voice.env.active()));
}

#[test]
fn sample_rate_preparation_allocates_before_callback_and_keeps_deck_time_constants() {
    for sr in [44_100, 96_000] {
        let mut rt = engine();
        rt.tracks[2].fx = rack(rt.sr);
        rt.scene_fx = rack(rt.sr);
        rt.apply(Command::LaunchScene { scene: 0 });
        let counts = test_alloc::measure(|| rt.set_sample_rate(sr));
        assert!(
            counts.allocations > 0,
            "preparation should allocate effect/sample buffers"
        );
        for deck in &rt.decks {
            assert!(
                (deck.grain_frames as f64 / sr as f64 - 1024.0 / 48_000.0).abs() <= 1.0 / sr as f64
            );
            let frames = (sr as f64 * 0.001).round();
            let decay = (1.0 - deck.rate_smoothing as f64).powf(frames);
            assert!((decay - 0.92f64.powf(frames * 48_000.0 / sr as f64)).abs() < 1e-6);
        }
        // Production callback ownership begins only after preparation. Warm its
        // channel-conversion buffer; these blocks do not publish a snapshot.
        let mut callback = audio::OutputCallback::new(rt, 2);
        let mut output = [0.0f32; 2048];
        callback.render(&mut output);
        let counts = test_alloc::measure(|| callback.render(&mut output));
        assert_eq!(counts, test_alloc::Counts::default());
        assert!(output.iter().any(|sample| sample.abs() > 1e-5));
    }
}
