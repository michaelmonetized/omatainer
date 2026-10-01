use super::*;
use crate::engine::{
    audio::OutputCallback,
    fx::{FxId, FxSlot},
    test_alloc, Command, Engine,
};
use std::time::Duration;

#[test]
fn sampled_device_fault_matches_independent_wall_timer_and_keeps_cpu_distinct() {
    let (engine, mut rt) = Engine::headless_for_test(48000, 64);
    rt.tracks[2].fx.slots = vec![FxSlot::new(FxId::Dist, rt.sr)];
    rt.load_profile.fault = Some((false, 2, 0, Duration::from_millis(8)));
    engine.cmd.set_profiling(true);
    let mut callback = OutputCallback::new(rt, 2);
    let start = Instant::now();
    callback.render_timed(&mut [0f32; 128], Some(Duration::from_millis(12)));
    let outside = start.elapsed().as_nanos() as u64;
    let profile = engine.cmd.load_profile().unwrap();
    let device = profile
        .costs
        .iter()
        .find(|p| p.point == TRACK_FX + 2 * SLOTS)
        .unwrap();
    let track = profile.costs.iter().find(|p| p.point == 2).unwrap();
    assert!(device.elapsed_ns >= 8_000_000);
    assert!(device.elapsed_ns <= track.elapsed_ns && track.elapsed_ns <= outside);
    let audio = engine.cmd.audio_metrics();
    let measurement = audio.last_callback.unwrap();
    assert!(measurement.elapsed_ns >= track.elapsed_ns && measurement.elapsed_ns <= outside);
    assert!(measurement.render_cpu_ns.unwrap() < measurement.elapsed_ns - 6_000_000);
    assert_eq!(measurement.output_latency_ns, Some(12_000_000));
    assert_eq!(
        (
            measurement.frames,
            measurement.sample_rate,
            measurement.channels
        ),
        (64, 48000, 2)
    );
    assert_eq!(audio.deadline_overruns, 1);
    assert!(audio.dropped_buffers.is_none());
}

#[test]
fn dense_profile_has_fixed_coverage_preserves_audio_and_callback_has_no_heap_traffic() {
    let (engine, mut profiled) = Engine::headless_for_test(48000, 64);
    let (_, mut reference) = Engine::headless_for_test(48000, 64);
    for rt in [&mut profiled, &mut reference] {
        for i in 0..64 {
            rt.tracks[2].poly.note_on(30 + i, 0.3);
        }
        rt.tracks[2].fx.slots = (0..SLOTS + 2)
            .map(|_| FxSlot::new(FxId::Dist, rt.sr))
            .collect();
        rt.apply(Command::LaunchScene { scene: 0 });
    }
    engine.cmd.set_profiling(true);
    let mut callback = OutputCallback::new(profiled, 2);
    let mut control = OutputCallback::new(reference, 2);
    let mut actual = [0f32; 4096];
    let mut expected = [0f32; 4096];
    callback.render(&mut actual);
    control.render(&mut expected);
    assert_eq!(actual, expected);
    let profile = engine.cmd.load_profile().unwrap();
    assert_eq!(profile.frame, 0);
    assert_eq!(profile.omitted_devices, 2);
    assert_eq!(
        profile
            .costs
            .iter()
            .filter(|c| (TRACK_FX + 2 * SLOTS..TRACK_FX + 3 * SLOTS).contains(&c.point))
            .count(),
        SLOTS
    );
    assert!(
        profile
            .costs
            .iter()
            .find(|c| c.point == 2)
            .unwrap()
            .elapsed_ns
            > 0
    );
    let counts = test_alloc::measure(|| {
        for _ in 0..20 {
            callback.render(&mut actual);
        }
    });
    assert_eq!((counts.allocations, counts.frees), (0, 0));
    assert_eq!(engine.cmd.load_profile().unwrap().frame, 20 * 2048);
    engine.cmd.set_profiling(false);
    callback.render(&mut actual);
    assert_eq!(engine.cmd.load_profile().unwrap().frame, 20 * 2048);
}

#[test]
fn queue_pressure_reports_real_capacity_reservations_rejections_and_drain() {
    let (engine, rt) = Engine::headless_for_test(48000, 32);
    engine
        .send(Command::SamplerPad { pad: 0, on: true })
        .unwrap();
    while engine.send(Command::Tap(Instant::now())).is_ok() {}
    let before = engine.cmd.queue_pressure();
    assert_eq!(before.capacity, 32);
    assert_eq!(before.pending, engine.cmd.len());
    assert_eq!(before.observed_high_water, before.pending as u64);
    assert_eq!(before.reserved_releases, 1);
    assert_eq!(before.full_rejections, 1);
    assert!(before.pending < before.capacity);
    engine
        .send(Command::SamplerPad { pad: 0, on: false })
        .unwrap();
    assert_eq!(engine.cmd.queue_pressure().reserved_releases, 0);
    let mut callback = OutputCallback::new(rt, 2);
    callback.render(&mut [0f32; 128]);
    let after = engine.cmd.queue_pressure();
    assert_eq!(after.pending, 0);
    assert!(after.observed_high_water >= before.observed_high_water);
    assert_eq!(after.full_rejections, 1);
}

#[test]
#[ignore = "comparative local profiling-overhead benchmark; no hardware deadline assertion"]
fn compare_optional_sampled_profiling_cpu_cost() {
    fn run(enabled: bool) -> u64 {
        let (engine, mut rt) = Engine::headless_for_test(48000, 64);
        for i in 0..64 {
            rt.tracks[2].poly.note_on(30 + i, 0.3);
        }
        rt.tracks[2].fx.slots = (0..16).map(|_| FxSlot::new(FxId::Dist, rt.sr)).collect();
        engine.cmd.set_profiling(enabled);
        let mut callback = OutputCallback::new(rt, 2);
        let mut output = [0f32; 256];
        callback.render(&mut output);
        let begin = crate::engine::audio_metrics::thread_cpu_ns().unwrap();
        for _ in 0..256 {
            callback.render(&mut output);
        }
        crate::engine::audio_metrics::thread_cpu_ns().unwrap() - begin
    }
    let mut off = Vec::new();
    let mut on = Vec::new();
    for index in 0..7 {
        if index % 2 == 0 {
            off.push(run(false));
            on.push(run(true));
        } else {
            on.push(run(true));
            off.push(run(false));
        }
    }
    off.sort();
    on.sort();
    println!("32768 stereo frames, 64 held voices, 16 drive slots: profiling off CPU median {:.3} ms, on {:.3} ms, change {:.2}%", off[3] as f64/1e6,on[3] as f64/1e6,100.0*(on[3] as f64/off[3] as f64-1.0));
}
