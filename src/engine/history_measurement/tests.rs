use super::*;
use crate::engine::{dsp::OnePole, master_fx::MasterSlot, FxKind};

fn episode() -> Episode { Episode { load: 42, generation: 1, deck: 0 } }
fn frame(actual: [f64; 2], contribution: [f64; 2], error: f64) -> Frame {
    let without = std::array::from_fn(|c| actual[c] - contribution[c]);
    Frame { analog: actual, digital: actual,
        without_analog: [without; LANES], without_digital: [without; LANES],
        error: [[error; 2]; LANES], digital_error: [[error; 2]; LANES] }
}
fn classify(value: Frame) -> Classification {
    let mut windows = Windows::new(48_000, 0, [Some(episode()), None, None, None]).unwrap();
    assert!(windows.push(&value).is_none());
    windows.finish().unwrap()[0].unwrap().classification
}

#[test]
fn activity_requires_actual_and_source_dependent_main_signal_on_the_same_channel() {
    assert_eq!(classify(frame([0.2, -0.1], [0.2, -0.1], 0.0)), Classification::Active);
    // Audible other sources cannot make a cued/zero-gain deck qualify.
    assert_eq!(classify(frame([0.2, -0.1], [0.0; 2], 0.0)), Classification::BelowFloor);
    // Two opposing decks, or a mono anti-phase sum, produce no actual signal.
    assert_eq!(classify(frame([0.0; 2], [0.2, -0.2], 0.0)), Classification::BelowFloor);
    assert_eq!(classify(frame([0.2, 0.0], [0.0, 0.2], 0.0)), Classification::BelowFloor);
    let mut quantized = frame([0.1; 2], [0.001; 2], 0.0);
    quantized.without_digital = [[0.1; 2]; LANES];
    assert_eq!(classify(quantized), Classification::BelowFloor);
    quantized.digital = [0.0; 2];
    assert_eq!(classify(quantized), Classification::BelowFloor);
    // Both sides of a defensible numerical interval must not become certainty.
    assert_eq!(classify(frame([0.1; 2], [ACTIVITY_FLOOR * 1.1; 2], ACTIVITY_FLOOR * 0.2)), Classification::Ambiguous);
    assert_eq!(classify(frame([0.1; 2], [ACTIVITY_FLOOR * 2.0; 2], ACTIVITY_FLOOR * 0.2)), Classification::Active);
    assert_eq!(classify(frame([0.1; 2], [ACTIVITY_FLOOR * 0.5; 2], 0.0)), Classification::BelowFloor);
    for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert_eq!(classify(frame([bad, 0.1], [0.1; 2], 0.0)), Classification::Nonfinite);
    }
    assert_eq!(classify(frame([0.1; 2], [0.1; 2], -1.0)), Classification::Nonfinite);
}

#[test]
fn sample_clock_windows_keep_zero_crossings_partial_intervals_and_noninteger_rates() {
    for rate in [8_000_u32, 44_100, 44_101, 48_000, 96_000, 192_000, 384_000] {
        let width = rate.div_ceil(100);
        let mut windows = Windows::new(rate, 712, [Some(episode()), None, None, None]).unwrap();
        let mut observations = Vec::new();
        for i in 0..(2 * width + 17) {
            let value = (std::f64::consts::TAU * i as f64 / width as f64).sin() * 0.1;
            if let Some(result) = windows.push(&frame([value; 2], [value; 2], 0.0)) {
                observations.push(result[0].unwrap());
                assert!(result[1..].iter().all(Option::is_none));
            }
        }
        observations.push(windows.finish().unwrap()[0].unwrap());
        assert!(windows.finish().is_none());
        assert_eq!(observations.iter().map(|o| o.frames).collect::<Vec<_>>(), [width, width, 17]);
        assert_eq!(observations.iter().map(|o| o.first_frame).collect::<Vec<_>>(), [712, 712 + width as u64, 712 + 2 * width as u64]);
        assert!(observations.iter().all(|o| o.classification == Classification::Active && o.sample_rate == rate && o.episode == episode()));
    }
    for rate in [0, 7_999, 384_001, u32::MAX] { assert!(Windows::new(rate, 0, [None; LANES]).is_none()); }
    let mut overflow = Windows::new(48_000, u64::MAX - 1, [Some(episode()); LANES]).unwrap();
    for _ in 0..3 { overflow.push(&frame([0.2; 2], [0.2; 2], 0.0)); }
    assert!(overflow.finish().unwrap().iter().flatten().all(|o| o.classification == Classification::ClockOverflow));
    overflow.push(&frame([0.2; 2], [0.2; 2], 0.0));
    assert_eq!(overflow.finish().unwrap()[0].unwrap().classification, Classification::ClockOverflow);
}

#[test]
fn future_bound_has_margin_for_supported_recursive_f32_roundoff() {
    // For one actual OnePole tick (three rounded operations), gamma3
    // bounds the forward local error. Its absolute coefficients sum to at
    // most (1 + 2a); geometric accumulation divides by 1-a. Two poles and
    // the three wet/dry operations still fit well inside 0.1%.
    let u = f64::from(f32::EPSILON) / 2.0;
    let gamma3 = 3.0 * u / (1.0 - 3.0 * u);
    for rate in MIN_RATE..=MAX_RATE {
        let a = f64::from(OnePole::lpf(rate as f32, 1000.0).a);
        let relative = gamma3 * (1.0 + 2.0 * a) / (1.0 - a);
        assert!(2.0 * relative + gamma3 * 4.0 < 0.001);
        // Worst comb feedback .72 plus linear interpolation, summation,
        // wet/dry mixing: even a deliberately inflated 64-operation term
        // leaves a much smaller gain than the implemented bound of four.
        let gamma64 = 64.0 * u / (1.0 - 64.0 * u);
        assert!((1.0 + gamma64) / (1.0 - 0.72 * (1.0 + gamma64)) < 4.0);
    }
}

#[test]
fn observed_processor_is_bit_exact_and_bound_covers_later_controls_and_decay() {
    for rate in [8_000, 44_100, 48_000, 96_000, 384_000] {
        for kind in [FxKind::Echo, FxKind::Reverb, FxKind::Filter] {
            let mut plain = MasterSlot::new(rate as f32);
            let mut observed = MasterSlot::new(rate as f32);
            let mut bounds = SlotBounds::default();
            for i in 0..rate / 4 {
                let input = [(i as f32 * 0.013).sin() * 0.5, (i as f32 * 0.019).cos() * 0.3];
                let wet = (i % 100) as f32 / 99.0;
                let spb = rate as f64 * 60.0 / 180.0;
                plain.configure(wet, spb); observed.configure(wet, spb);
                assert_eq!(plain.process(input, kind, wet), observed.process_observed(input, kind, wet, &mut bounds));
            }
            let future = bounds.future_bound(kind, [0.0; 2]);
            for i in 0..rate * 5 {
                // Future tempo/wet changes may reveal samples which are
                // currently inaudible; the certificate must cover them too.
                let wet = (i % 71) as f32 / 70.0;
                observed.configure(wet, rate as f64 * 60.0 / if i % 113 < 50 { 400.0 } else { 20.0 });
                let out = observed.process_observed([0.0; 2], kind, wet, &mut bounds);
                for c in 0..2 { assert!(f64::from(out[c]).abs() <= future[c], "{rate} {kind:?} {i}"); }
            }
            observed.reset(kind); bounds.reset(kind);
            assert_eq!(bounds.future_bound(kind, [0.0; 2]), [0.0; 2]);
            assert_eq!(observed.process_observed([0.0; 2], kind, 1.0, &mut bounds), [0.0; 2]);
        }
    }
}

#[test]
fn serial_retirement_certificate_is_conservative_and_eventually_reusable() {
    let rate = 8_000;
    let mut slots: [MasterSlot; 3] = std::array::from_fn(|_| MasterSlot::new(rate as f32));
    let mut bounds = [SlotBounds::default(); 3];
    let kinds = [FxKind::Echo, FxKind::Reverb, FxKind::Filter];
    for slot in &mut slots { slot.configure(0.7, rate as f64 * 60.0 / 200.0); }
    assert!(tail::certified_negligible(&bounds, kinds));
    let mut certified_at = None;
    for i in 0..rate * 60 {
        let mut out = if i < 100 { [0.5, -0.3] } else { [0.0; 2] };
        for slot in 0..3 { out = slots[slot].process_observed(out, kinds[slot], 0.7, &mut bounds[slot]); }
        if i == 99 { assert!(!tail::certified_negligible(&bounds, kinds)); }
        if i > 100 && certified_at.is_none() && tail::certified_negligible(&bounds, kinds) { certified_at = Some(i); }
        if certified_at.is_some() { assert!(out.into_iter().all(|x| f64::from(x).abs() * 1.5 < RETIRE_FLOOR)); }
    }
    assert!(certified_at.is_some_and(|frame| frame < rate * 40), "tail must release its lane during an ordinary track");
}

#[test]
fn replacement_fade_keeps_old_identity_and_never_changes_rendered_samples() {
    use crate::engine::*;
    let (_, rx) = crossbeam_channel::bounded(32);
    let mut rt = RtEngine::new(48_000.0, rx, Arc::new(Mutex::new(Snapshot::default())));
    let sample = |value| Arc::new(Sample { name: "attribution".into(), sr: 48_000,
        ch: 2, data: vec![value; 8_192], peaks: vec![].into(), bpm: 120.0, path: String::new() });
    rt.apply(Command::DeckAudio { deck: 0, audio: sample(0.25) });
    rt.decks[0].gain = 1.0;
    rt.decks[0].playing = true;
    for _ in 0..1024 { rt.render_deck(0); }
    let original_key = rt.decks[0].history_key;
    let original = rt.decks[0].last_output;
    rt.apply(Command::DeckAudio { deck: 0, audio: sample(-0.4) });
    rt.decks[0].playing = true;
    let new_key = rt.decks[0].history_key;
    assert_ne!(original_key, new_key);
    for i in 0..96 {
        let actual = rt.render_deck(0);
        let parts = rt.decks[0].history_last;
        let old = parts.values.iter().find(|p| p.key == original_key).map_or([0.0; 2], |p| p.value);
        let mix = i as f32 / 95.0;
        for c in 0..2 { assert!((old[c] - f64::from(original[c]) * f64::from(1.0 - mix)).abs() < 1e-12); }
        assert!(parts.residual([actual.0, actual.1]).into_iter().all(|r| r < 1e-7));
        assert!(!parts.incomplete);
    }
    rt.render_deck(0);
    assert!(rt.decks[0].history_last.values.iter().all(|p| p.key == 0 || p.key == new_key));
    // Seeking keeps the same source; the short envelope is not a new track.
    rt.apply(Command::DeckSeek { deck: 0, frac: 0.5 });
    rt.render_deck(0);
    assert!(rt.decks[0].history_last.values.iter().all(|p| p.key == 0 || p.key == new_key));
}

#[test]
fn attribution_overflow_is_explicit_and_a_completed_fade_recovers() {
    let mut parts = parts::Parts::default();
    for key in 1..=LANES as u64 + 1 { parts = parts::Parts::transition(parts, key, [0.1; 2], 0.5); }
    assert!(parts.incomplete);
    assert_eq!(parts.values.iter().filter(|p| p.key != 0).count(), LANES);
    parts = parts::Parts::transition(parts, 99, [0.2; 2], 1.0);
    assert!(!parts.incomplete);
    assert_eq!(parts.values[0].key, 99);
    assert!(parts.residual([0.2; 2]).into_iter().all(|x| x == 0.0));
    parts = parts::Parts::transition(parts, 0, [0.2; 2], 1.0);
    assert!(parts.incomplete, "missing identity cannot be reassigned to a current deck label");
}
