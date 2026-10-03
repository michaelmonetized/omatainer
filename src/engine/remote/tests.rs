use super::*;
use crate::engine::{test_alloc, Engine};

fn request(rt: &RtEngine, action: Action, at: Option<f64>) -> Request {
    Request {
        namespace: rt.session.namespace,
        transport_epoch: rt.transport_epoch,
        safety_epoch: rt.performance.safety_epoch(),
        at,
        action,
        ack: Ack::new(),
    }
}
fn gain(rt: &RtEngine, value: f32) -> Action {
    Action::Gain {
        slot: 2,
        target: rt.session.reference(Axis::Track, 2).unwrap(),
        value,
    }
}

#[test]
fn action_applies_on_the_exact_musical_sample_without_callback_heap_work() {
    let (engine, mut rt) = Engine::headless_for_test(48000, 256);
    rt.process(&mut []);
    rt.playing = true;
    let original = rt.tracks[2].gain;
    let beat = 17.0 / (f64::from(rt.sr) * 60.0 / f64::from(rt.bpm));
    let job = request(&rt, gain(&rt, 0.17), Some(beat));
    let ack = job.ack.clone();
    engine.send(Command::Remote(job)).unwrap();
    for frame in 0..32 {
        assert_eq!(
            test_alloc::measure(|| rt.process(&mut [0.0; 2])),
            test_alloc::Counts::default()
        );
        assert_eq!(rt.tracks[2].gain, if frame < 17 { original } else { 0.17 });
    }
    assert_eq!(ack.state(), Outcome::Applied);
}

#[test]
fn scheduled_gain_updates_audio_inside_a_large_block_at_the_reference_sample() {
    let (engine, mut scheduled) = Engine::headless_for_test(48000, 256);
    let (reference_engine, mut reference) = Engine::headless_for_test(48000, 256);
    scheduled.apply(Command::LaunchScene { scene: 0 });
    reference.apply(Command::LaunchScene { scene: 0 });
    scheduled.process(&mut [0.0; 128]);
    reference.process(&mut [0.0; 128]);
    let beat = scheduled.beat + 17.0 / (f64::from(scheduled.sr) * 60.0 / f64::from(scheduled.bpm));
    let job = request(&scheduled, gain(&scheduled, 0.17), Some(beat));
    let ack = job.ack.clone();
    engine.send(Command::Remote(job)).unwrap();
    let mut actual = [0.0f32; 128];
    let mut expected = [0.0f32; 128];
    assert_eq!(
        test_alloc::measure(|| scheduled.process(&mut actual)),
        test_alloc::Counts::default()
    );
    for frame in 0..64 {
        if frame == 17 {
            reference_engine
                .send(Command::TrackGain {
                    track: 2,
                    value: 0.17,
                })
                .unwrap();
        }
        reference.process(&mut expected[frame * 2..frame * 2 + 2]);
    }
    assert!(actual.iter().any(|value| *value != 0.0));
    assert_eq!(actual.map(f32::to_bits), expected.map(f32::to_bits));
    assert_eq!(ack.state(), Outcome::Applied);
}

#[test]
fn equal_beat_actions_stay_in_order_and_cancelled_actions_do_not_apply() {
    let (engine, mut rt) = Engine::headless_for_test(48000, 256);
    rt.process(&mut []);
    rt.playing = true;
    let mut receipts = Vec::new();
    for value in [0.2, 0.4, 0.8] {
        let job = request(&rt, gain(&rt, value), Some(1.0));
        receipts.push(job.ack.clone());
        engine.send(Command::Remote(job)).unwrap();
    }
    assert!(receipts[2].cancel());
    rt.process(&mut []);
    rt.beat = 1.0;
    assert_eq!(
        test_alloc::measure(|| rt.process(&mut [0.0; 2])),
        test_alloc::Counts::default()
    );
    assert_eq!(rt.tracks[2].gain, 0.4);
    assert_eq!(receipts[0].state(), Outcome::Applied);
    assert_eq!(receipts[1].state(), Outcome::Applied);
    assert_eq!(receipts[2].state(), Outcome::Cancelled);
    assert_eq!(rt.remote_schedule.len, 0);
}

#[test]
fn stops_replacement_and_deleted_targets_reject_pending_actions() {
    for change in 0..3 {
        let (engine, mut rt) = Engine::headless_for_test(48000, 256);
        rt.process(&mut []);
        rt.playing = true;
        let job = request(&rt, gain(&rt, 0.17), Some(2.0));
        let ack = job.ack.clone();
        engine.send(Command::Remote(job)).unwrap();
        rt.process(&mut []);
        let before = rt.tracks[2].gain;
        match change {
            0 => rt.apply(Command::Stop),
            1 => rt.session.namespace = [7, 8],
            _ => {
                let id = rt.session.tracks[2].id;
                rt.session.delete(Axis::Track, id).unwrap();
            }
        }
        assert_eq!(
            test_alloc::measure(|| rt.process(&mut [0.0; 2])),
            test_alloc::Counts::default()
        );
        assert_eq!(ack.state(), Outcome::Rejected);
        assert_eq!(rt.tracks[2].gain, before);
        assert_eq!(rt.remote_schedule.len, 0);
    }
}

#[test]
fn schedule_is_bounded_and_rejects_stopped_past_nonfinite_and_invalid_values() {
    let (engine, mut rt) = Engine::headless_for_test(48000, 256);
    rt.process(&mut []);
    for (at, action) in [(Some(2.0), gain(&rt, 0.2)), (None, gain(&rt, f32::NAN))] {
        let job = request(&rt, action, at);
        let ack = job.ack.clone();
        engine.send(Command::Remote(job)).unwrap();
        rt.process(&mut []);
        assert_eq!(ack.state(), Outcome::Rejected);
    }
    rt.playing = true;
    for at in [0.0, f64::NAN, f64::INFINITY, 20000.0] {
        let job = request(&rt, gain(&rt, 0.2), Some(at));
        let ack = job.ack.clone();
        engine.send(Command::Remote(job)).unwrap();
        rt.process(&mut []);
        assert_eq!(ack.state(), Outcome::Rejected);
    }
    let mut receipts = Vec::new();
    for index in 0..65 {
        let job = request(&rt, gain(&rt, 0.2), Some(2.0 + index as f64));
        receipts.push(job.ack.clone());
        engine.send(Command::Remote(job)).unwrap();
        rt.process(&mut []);
    }
    assert_eq!(rt.remote_schedule.len, 64);
    assert_eq!(receipts[64].state(), Outcome::Rejected);
    for ack in receipts {
        ack.cancel();
    }
    assert_eq!(
        test_alloc::measure(|| rt.process(&mut [])),
        test_alloc::Counts::default()
    );
    assert_eq!(rt.remote_schedule.len, 0);
}

#[test]
fn job_results_survive_reconnect_and_pending_jobs_are_never_evicted() {
    let jobs = Jobs::default();
    let mut ids = Vec::new();
    for _ in 0..128 {
        ids.push(jobs.insert(Ack::new()).unwrap());
    }
    assert!(jobs.insert(Ack::new()).is_err());
    assert!(jobs.get(&ids[0]).unwrap().cancel());
    assert_eq!(jobs.get(&ids[0]).unwrap().state(), Outcome::Cancelled);
    let next = jobs.insert(Ack::new()).unwrap();
    assert!(jobs.get(&ids[0]).is_none());
    assert!(jobs.get(&ids[1]).is_some());
    assert_eq!(jobs.get(&next).unwrap().state(), Outcome::Pending);
}
