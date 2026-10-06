use super::*;
use crate::engine::*;

fn callback(channels: usize, anti_phase: bool, gain: f32, xfader: f32, pfl: bool, cue_mix: f32)
    -> (audio::OutputCallback, crossbeam_channel::Receiver<Observation>, u64)
{
    let (_, rx) = crossbeam_channel::bounded(64);
    let mut rt = RtEngine::new(48_000.0, rx, Arc::new(Mutex::new(Snapshot::default())));
    let audio = Arc::new(Sample { spectrum: None, name: "capture reference".into(), sr: 48_000, ch: 2,
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
            (false, 1.0, 1.0, true, 1.0, false),
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

#[test]
fn prepare_monitor_confirms_playing_output_without_starting_a_history_session() {
    for (gain, xfader, pfl, cue_mix, active) in [
        (1.0,0.0,false,0.0,true), (0.0,0.0,false,0.0,false),
        (1.0,1.0,true,0.0,false), (1.0,1.0,true,1.0,false),
    ] {
        let (mut callback, receiver, key) = callback(2,false,gain,xfader,pfl,cue_mix);
        let handle = callback.renderer_for_test().history_measurement.as_ref().unwrap().handle();
        handle.set_prepare_monitor(true).unwrap();
        let boundary = handle.clock().unwrap();
        for _ in 0..8 { callback.render(&mut [0.0_f32;256]); }
        let counts = test_alloc::measure(|| { for _ in 0..8 { callback.render(&mut [0_i16;256]); } });
        assert_eq!((counts.allocations,counts.frees),(0,0));
        assert!(receiver.is_empty(),"transient monitoring must not create session-zero history events");
        assert_eq!(handle.status().0,0);
        let event = handle.digital_play(0);
        assert_eq!(event.is_some(),active,"gain{gain} crossfader{xfader} cue{cue_mix}");
        if let Some(event) = event { assert_eq!(event.load,key);assert_eq!(event.frames,480);assert!(event.wall_ns > boundary); }
        let previous = event;
        handle.set_prepare_monitor(false).unwrap();
        for _ in 0..8 { callback.render(&mut [0.0_f32;256]); }
        assert_eq!(handle.digital_play(0),previous);
    }
}

#[test]
fn paused_preview_and_direct_renderer_processing_never_remove_upcoming_tracks() {
    let (mut callback, receiver, key) = callback(2,false,1.0,0.0,false,0.0);
    let handle = callback.renderer_for_test().history_measurement.as_ref().unwrap().handle();
    handle.set_prepare_monitor(true).unwrap();
    {
        let rt = callback.renderer_mut_for_test();
        rt.decks[0].playing = false;
        rt.decks[0].keylock = true;
        rt.decks[0].pitch = 0.6;
        rt.decks[0].pos = 2500.0;
        rt.apply(Command::DeckPreview { deck:0,expected:key,on:true });
    }
    let mut energy = 0.0_f64;
    for _ in 0..12 { let mut out = [0.0_f32;256];callback.render(&mut out);energy += out.iter().map(|v|f64::from(*v).powi(2)).sum::<f64>(); }
    assert!(energy > 0.1);
    assert!(handle.digital_play(0).is_none());
    assert!(receiver.is_empty());
    let rt = callback.renderer_mut_for_test();
    assert!(!rt.decks[0].playing);
    assert!(rt.decks[0].pos > 1000.0,"paused preview must keep advancing after its initial transition");
    assert_eq!(rt.decks[0].keylock_mode(),keylock::Mode::Locked);
    rt.apply(Command::DeckPreview { deck:0,expected:key,on:false });
    assert_eq!(rt.decks[0].pos,2500.0);
    assert!(rt.decks[0].preview_position.is_none());
    rt.decks[0].playing = true;
    for _ in 0..8 { rt.process(&mut [0.0;2048]); }
    assert!(handle.digital_play(0).is_none(),"exports and headless processing are not final converted output");
}

#[test]
fn prepare_monitor_rejects_paused_tails_partial_windows_and_emergency_mute() {
    let (mut callback, receiver, _) = callback(2,false,1.0,0.0,false,0.0);
    let handle = callback.renderer_for_test().history_measurement.as_ref().unwrap().handle();
    handle.set_prepare_monitor(true).unwrap();
    callback.render(&mut [0.0_f32;200]);
    assert!(handle.digital_play(0).is_none());
    handle.set_prepare_monitor(false).unwrap();
    callback.render(&mut [0.0_f32;256]);
    callback.renderer_mut_for_test().decks[0].playing = false;
    handle.set_prepare_monitor(true).unwrap();
    for _ in 0..8 { callback.render(&mut [0.0_f32;256]); }
    assert!(handle.digital_play(0).is_none());
    let rt = callback.renderer_mut_for_test();
    rt.apply(Command::SafetyStop(performance::Safety::Silence));
    rt.decks[0].playing = true;
    for _ in 0..16 { callback.render(&mut [0.0_f32;256]); }
    assert!(handle.digital_play(0).is_none());
    assert!(receiver.is_empty());
}

#[test]
fn explicit_routing_retires_stereo_measurements_and_refuses_reusing_their_evidence() {
    let (mut callback, _receiver, _) = callback(2, false, 1.0, 0.0, false, 0.0);
    let mut block = [0.0_f32; 256];
    for _ in 0..8 { callback.render(&mut block); }
    assert!(callback.renderer_mut_for_test().history_measurement.as_mut().unwrap().start());
    callback.render(&mut block);
    let rt = callback.renderer_mut_for_test();
    rt.routing = Some(Box::new(crate::engine::audio::routing::prepared::Prepared::new(
        std::sync::Arc::new(crate::engine::audio::routing::model::Model::default()), &rt.session).unwrap()));
    callback.render(&mut block);
    let history = callback.renderer_mut_for_test().history_measurement.as_mut().unwrap();
    assert!(history.incomplete);
    assert!(history.end());
    assert!(!history.start());
    callback.renderer_mut_for_test().routing = None;
    callback.render(&mut block);
    assert!(callback.renderer_mut_for_test().history_measurement.as_mut().unwrap().start());
}

#[test]
fn custom_routes_cannot_credit_prepare_removal_and_measurement_recovers_on_default_route() {
    let (mut callback, receiver, _) = callback(2, false, 1.0, 0.0, false, 0.0);
    let handle = callback.renderer_for_test().history_measurement.as_ref().unwrap().handle();
    handle.set_prepare_monitor(true).unwrap();
    let rt = callback.renderer_mut_for_test();
    rt.routing = Some(Box::new(crate::engine::audio::routing::prepared::Prepared::new(
        std::sync::Arc::new(crate::engine::audio::routing::model::Model::default()), &rt.session).unwrap()));
    for _ in 0..8 { callback.render(&mut [0.0_f32; 256]); }
    assert!(!handle.can_measure());
    assert!(handle.digital_play(0).is_none());
    assert!(receiver.is_empty());
    callback.renderer_mut_for_test().routing = None;
    for _ in 0..8 { callback.render(&mut [0.0_f32; 256]); }
    assert!(handle.can_measure());
    assert!(handle.digital_play(0).is_some());
    let rt = callback.renderer_mut_for_test();
    rt.routing = Some(Box::new(crate::engine::audio::routing::prepared::Prepared::new(
        std::sync::Arc::new(crate::engine::audio::routing::model::Model::default()), &rt.session).unwrap()));
    callback.render(&mut [0.0_f32; 256]);
    assert!(!handle.can_measure());
    assert!(handle.digital_play(0).is_none(), "old stereo evidence cannot escape the route guard");
    assert!(receiver.is_empty());
}

#[test]
fn independent_now_playing_monitor_preserves_prepare_intent_and_allocates_nothing() {
    let (mut callback,receiver,key)=callback(2,false,1.0,0.0,false,0.0);
    let handle=callback.renderer_for_test().history_measurement.as_ref().unwrap().handle();
    handle.set_now_playing_monitor(true).unwrap();handle.set_prepare_monitor(false).unwrap();
    for _ in 0..8 {callback.render(&mut [0.0_f32;256]);}
    assert_eq!(handle.digital_play(0).unwrap().load,key);
    let counts=test_alloc::measure(||{for _ in 0..8 {callback.render(&mut [0_i16;256]);}});
    assert_eq!((counts.allocations,counts.frees),(0,0));assert!(receiver.is_empty());
    handle.set_prepare_monitor(true).unwrap();handle.set_now_playing_monitor(false).unwrap();
    let before=handle.digital_play(0).unwrap();for _ in 0..8 {callback.render(&mut [0.0_f32;256]);}
    assert!(handle.digital_play(0).unwrap().first_frame>before.first_frame);
    handle.set_prepare_monitor(false).unwrap();let before=handle.digital_play(0);
    for _ in 0..8 {callback.render(&mut [0.0_f32;256]);}assert_eq!(handle.digital_play(0),before);
}
