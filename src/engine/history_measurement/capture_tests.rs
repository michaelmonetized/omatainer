use super::*;
use crate::engine::*;

fn callback(channels: usize, anti_phase: bool, gain: f32, xfader: f32, pfl: bool, cue_mix: f32)
    -> (audio::OutputCallback, crossbeam_channel::Receiver<Observation>, u64)
{
    let (_, rx) = crossbeam_channel::bounded(64);
    let mut rt = RtEngine::new(48_000.0, rx, Arc::new(Mutex::new(Snapshot::default())));
    let audio = Arc::new(Sample { name: "capture reference".into(), sr: 48_000, ch: 2,
        data: (0..32_768).flat_map(|i| {
            let value = (std::f32::consts::TAU * 375.0 * i as f32 / 48_000.0).sin() * 0.2;
            [value, if anti_phase { -value } else { value * 0.75 }]
        }).collect(), peaks: vec![].into(), bpm: 120.0, path: String::new() });
    rt.apply(Command::DeckAudio { deck: 0, audio });
    rt.decks[0].gain = gain; rt.decks[0].playing = true; rt.decks[0].pfl = pfl;
    rt.decks[1].audio = None; rt.decks[1].playing = false;
    rt.xfader = xfader; rt.master = 1.0; rt.cue_mix = cue_mix;
    let key = rt.decks[0].history_key;
    let receiver = rt.history_measurement.as_mut().unwrap().take_receiver().unwrap();
    (audio::OutputCallback::new(rt, channels), receiver, key)
}

fn format_capture<T>()
where T: cpal::SizedSample + cpal::FromSample<f32>, f64: cpal::FromSample<T>,
{
    for channels in [1, 2, 4, 6, 8] {
        for (anti, gain, xfader, pfl, cue_mix, audible) in [
            (false, 1.0, 0.0, false, 0.0, true),
            (false, 0.0, 0.0, false, 0.0, false),
            (false, 1.0, 1.0, true, 0.0, false),
            (false, 1.0, 1.0, true, 1.0, true),
            (true, 1.0, 0.0, false, 0.0, channels != 1),
        ] {
            let (mut callback, receiver, key) = callback(channels, anti, gain, xfader, pfl, cue_mix);
            let mut block = vec![T::from_sample(0.0); 128 * channels];
            // Start while already playing, after the real output path settled.
            for _ in 0..8 { callback.render(&mut block); }
            assert!(receiver.is_empty(), "no implicit session");
            assert!(callback.renderer_mut_for_test().history_measurement.as_mut().unwrap().start());
            let mut captured = Vec::with_capacity(15 * block.len());
            let mut heap = test_alloc::Counts::default();
            for _ in 0..15 {
                let counts = test_alloc::measure(|| callback.render(&mut block));
                heap.allocations += counts.allocations; heap.frees += counts.frees;
                captured.extend_from_slice(&block);
            }
            assert!(callback.renderer_mut_for_test().history_measurement.as_mut().unwrap().end());
            assert_eq!((heap.allocations, heap.frees), (0, 0));
            let history = callback.renderer_for_test().history_measurement.as_ref().unwrap();
            assert!(!history.incomplete); assert_eq!(history.dropped, 0);
            let observations: Vec<_> = receiver.try_iter().collect();
            let active = observations.iter().filter(|o| o.classification == Classification::Active)
                .map(|o| u64::from(o.frames)).sum::<u64>();
            assert_eq!(active, if audible { 1920 } else { 0 }, "{} channels{channels} gain{gain} xfader{xfader} cue{cue_mix} anti{anti}: {observations:?}", std::any::type_name::<T>());
            assert!(observations.iter().all(|o| o.episode.load == key && o.episode.deck == 0 && o.sample_rate == 48_000));
            // Independent checks consume the actual converted multichannel
            // buffer; neither source flags nor observer sidecars supply it.
            let mut energy = [0.0_f64; 2];
            for frame in captured.chunks_exact(channels) {
                for c in 0..channels.min(2) {
                    let value = cpal::Sample::to_sample::<f64>(frame[c]);
                    assert!(value.is_finite()); energy[c] += value * value;
                }
                for sample in &frame[channels.min(2)..] { assert_eq!(cpal::Sample::to_sample::<f64>(*sample), 0.0); }
            }
            assert_eq!(energy.into_iter().any(|e| e / 1920.0 > ACTIVITY_FLOOR.powi(2)), audible);
            callback.render(&mut block);
            assert!(receiver.is_empty(), "no duration after acknowledged end");
        }
    }
}

#[test]
fn actual_output_capture_matches_history_all_supported_formats_and_main_channel_counts() {
    format_capture::<f32>(); format_capture::<f64>();
    format_capture::<i8>(); format_capture::<i16>(); format_capture::<i32>(); format_capture::<i64>();
    format_capture::<u8>(); format_capture::<u16>(); format_capture::<u32>(); format_capture::<u64>();
}

#[test]
fn direct_renderer_exports_and_zero_frame_service_never_credit_output() {
    let (mut callback, receiver, _) = callback(2, false, 1.0, 0.0, false, 0.0);
    let rt = callback.renderer_mut_for_test();
    rt.history_measurement.as_mut().unwrap().start();
    rt.process(&mut [0.0; 4096]);
    rt.process_interleaved(&mut [0.0; 4096], 2);
    rt.process(&mut []);
    rt.history_measurement.as_mut().unwrap().end();
    assert!(receiver.is_empty());
}

#[test]
fn rate_segments_and_partial_end_preserve_frame_counts_without_padding() {
    let (mut callback, receiver, _) = callback(4, false, 1.0, 0.0, false, 0.0);
    callback.render(&mut [0.0_f32; 4096]);
    callback.renderer_mut_for_test().history_measurement.as_mut().unwrap().start();
    callback.render(&mut [0.0_f32; 4 * 731]);
    callback.renderer_mut_for_test().set_sample_rate(44_100).unwrap();
    callback.render(&mut [0.0_f32; 4 * 619]);
    callback.renderer_mut_for_test().history_measurement.as_mut().unwrap().end();
    let observations: Vec<_> = receiver.try_iter().collect();
    assert!(observations.iter().all(|o| o.classification == Classification::Active));
    for (rate, expected) in [(48_000, 731), (44_100, 619)] {
        assert_eq!(observations.iter().filter(|o| o.sample_rate == rate).map(|o| o.frames).sum::<u32>(), expected);
    }
    assert_eq!(observations.iter().map(|o| o.frames).collect::<Vec<_>>(), [480, 251, 441, 178]);
}

#[test]
fn output_sidecar_and_disconnected_consumer_fail_explicitly_without_breaking_audio() {
    let (mut callback, receiver, _) = callback(2, false, 1.0, 0.0, false, 0.0);
    callback.render(&mut [0.0_f32; 2048]);
    callback.renderer_mut_for_test().history_measurement.as_mut().unwrap().start();
    // The existing renderer accepts a large backend block. The observer has
    // its own explicit ceiling and cannot allocate a larger sidecar on demand.
    let mut oversized = vec![0.0_f32; 2 * 16_385];
    callback.render(&mut oversized);
    assert!(oversized.iter().any(|x| x.abs() > 0.1));
    assert!(receiver.is_empty());
    let history = callback.renderer_for_test().history_measurement.as_ref().unwrap();
    assert!(history.incomplete); assert_eq!(history.dropped, 16_385);
    drop(receiver);
    callback.render(&mut [0.0_f32; 960]);
    let history = callback.renderer_for_test().history_measurement.as_ref().unwrap();
    assert!(history.incomplete); assert!(history.dropped >= 16_865);
}
