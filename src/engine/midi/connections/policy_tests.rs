use super::{test_support::*, *};
use crate::engine::{test_alloc, Command, Engine, RtEngine};
use std::time::{Duration, Instant};

fn applied(engine: &Engine, generation: u64) -> Arc<PolicyStatus> {
    until(|| {
        engine.midi.policy_status().unwrap().applied == Some(generation)
            && !engine.midi.connections_busy()
    });
    engine.midi.policy_status().unwrap()
}
fn held(rt: &RtEngine) -> usize {
    rt.tracks[1]
        .poly
        .voices
        .iter()
        .filter(|voice| voice.input.is_some() && matches!(voice.env.stage, 1..=3))
        .count()
}

#[test]
fn startup_disabled_and_exact_selected_names_never_expand_to_all() {
    let ports = [
        ("1", "Keyboard A"),
        ("2", "keyboard a"),
        ("3", "Keyboard B"),
    ];
    let (mut engine, _rt) = Engine::headless_for_test(48_000, 64);
    let control = install_with_policy(&mut engine, InputPolicy::Disabled);
    control.discover(&ports);
    let status = applied(&engine, 1);
    assert_eq!(
        *status.applied_policy.as_ref().unwrap().as_ref(),
        InputPolicy::Disabled
    );
    assert!(control.attempts.try_recv().is_err());
    assert!(engine.snapshot().midi[0].contains("[connected] keyboard + mouse"));
    assert!(engine.snapshot().midi[1..]
        .iter()
        .all(|label| label.starts_with("[disabled]")));
    assert_eq!(
        status.available_inputs.as_ref(),
        ["Keyboard A", "Keyboard B", "keyboard a"]
    );
    let selected = InputPolicy::Selected(vec!["Keyboard A".into(), "missing keyboard".into()]);
    let generation = engine.midi.configure_inputs(selected.clone()).unwrap();
    control.discover(&ports);
    control.connect("1", Ok(()));
    let status = applied(&engine, generation);
    assert_eq!(status.applied_policy.as_deref(), Some(&selected));
    assert_eq!(status.missing_names.as_ref(), ["missing keyboard"]);
    assert!(status.error.is_none());
    assert!(
        control.attempts.try_recv().is_err(),
        "case-insensitive matching admitted another input"
    );
    assert!(engine.snapshot().midi[2].starts_with("[disabled]"));
}

#[test]
fn policy_change_releases_only_removed_sources_and_preserves_allowed_connections() {
    let ports = [("1", "Keyboard A"), ("2", "Keyboard B")];
    let (mut engine, mut rt) = Engine::headless_for_test(48_000, 64);
    rt.selected_track = 1;
    let control = install(&mut engine);
    control.discover(&ports);
    let a = control.connect("1", Ok(()));
    let b = control.connect("2", Ok(()));
    applied(&engine, 1);
    a.push(&[0x90, 60, 100]);
    b.push(&[0x90, 60, 100]);
    until(|| engine.midi.input_stats().dispatched == 2);
    rt.process(&mut [0.0; 128]);
    assert_eq!(held(&rt), 2);
    let generation = engine
        .midi
        .configure_inputs(InputPolicy::Selected(vec!["Keyboard A".into()]))
        .unwrap();
    control.discover(&ports);
    applied(&engine, generation);
    assert!(!a.is_closed());
    assert!(b.is_closed());
    assert!(
        control.attempts.try_recv().is_err(),
        "allowed input was reconnected"
    );
    rt.process(&mut [0.0; 128]);
    assert_eq!(
        held(&rt),
        1,
        "same-pitch voice from allowed source remains held"
    );
    a.push(&[0x80, 60, 0]);
    until(|| engine.midi.input_stats().dispatched == 3);
    rt.process(&mut [0.0; 128]);
    assert_eq!(held(&rt), 0);
    a.push(&[0x90, 62, 100]);
    until(|| engine.midi.input_stats().dispatched == 4);
    rt.process(&mut [0.0; 128]);
    assert_eq!(held(&rt), 1);
    let generation = engine.midi.configure_inputs(InputPolicy::Disabled).unwrap();
    control.discover(&ports);
    applied(&engine, generation);
    rt.process(&mut [0.0; 128]);
    assert!(a.is_closed());
    assert_eq!(held(&rt), 0);
    assert!(
        engine.send(Command::Master(0.4)).is_ok(),
        "keyboard/mouse command route remains usable"
    );
}

#[test]
fn blocked_connect_is_inert_and_superseded_policy_never_publishes_or_dispatches_it() {
    let (mut engine, mut rt) = Engine::headless_for_test(48_000, 64);
    rt.selected_track = 1;
    let control = install(&mut engine);
    control.discover(&[("1", "Keyboard A")]);
    let Attempt::Connect { input, .. } = control.next() else {
        panic!()
    };
    assert_eq!(
        test_alloc::measure(|| input.push(&[0x90, 60, 100])),
        test_alloc::Counts::default()
    );
    let began = Instant::now();
    for _ in 0..128 {
        engine
            .midi
            .configure_inputs(InputPolicy::Selected(vec!["Keyboard A".into()]))
            .unwrap();
    }
    let generation = engine.midi.configure_inputs(InputPolicy::Disabled).unwrap();
    assert!(
        began.elapsed() < Duration::from_secs(1),
        "configuration waited for the blocked OS call"
    );
    let pending = engine.midi.policy_status().unwrap();
    assert_eq!(pending.requested, generation);
    assert!(pending.pending());
    assert_eq!(pending.applied, None);
    assert_eq!(
        test_alloc::measure(|| {
            for _ in 0..32 {
                input.push(&[0x90, 61, 100]);
            }
        }),
        test_alloc::Counts::default()
    );
    engine.send(Command::Master(0.27)).unwrap();
    rt.process(&mut [0.0; 128]);
    assert_eq!(rt.master, 0.27);
    assert_eq!(held(&rt), 0);
    assert_eq!(engine.midi.input_stats().dispatched, 0);
    control.replies.send(Reply::Connected(Ok(()))).unwrap();
    control.discover(&[("1", "Keyboard A")]);
    applied(&engine, generation);
    assert!(input.is_closed());
    assert!(
        control.attempts.try_recv().is_err(),
        "coalesced policies formed a job backlog"
    );
    rt.process(&mut [0.0; 128]);
    assert_eq!(held(&rt), 0);
    assert!(engine.snapshot().midi[1].starts_with("[disabled]"));
}

#[test]
fn validation_failure_and_unavailable_manager_preserve_current_policy() {
    let (mut engine, _rt) = Engine::headless_for_test(48_000, 32);
    assert_eq!(
        engine.midi.configure_inputs(InputPolicy::All),
        Err(PolicyError::Unavailable)
    );
    let control = install(&mut engine);
    control.discover(&[]);
    let original = applied(&engine, 1);
    for names in [
        vec![],
        vec!["a".into(), "a".into()],
        vec!["a\0b".into()],
        vec!["x".repeat(1025)],
        (0..65).map(|i| i.to_string()).collect(),
    ] {
        assert!(matches!(
            engine.midi.configure_inputs(InputPolicy::Selected(names)),
            Err(PolicyError::Invalid(_))
        ));
        assert!(Arc::ptr_eq(
            &original,
            &engine.midi.policy_status().unwrap()
        ));
    }
    assert!(control.attempts.try_recv().is_err());
    assert_eq!(
        test_alloc::measure(|| {
            for _ in 0..1000 {
                let _ = engine.midi.policy_status();
            }
        }),
        test_alloc::Counts::default()
    );
    let value = serde_json::json!({"mode":"selected","names":["Exact A"]});
    let parsed: InputPolicy = serde_json::from_value(value.clone()).unwrap();
    assert_eq!(serde_json::to_value(parsed).unwrap(), value);
    for value in [
        serde_json::json!({"mode":"all","unexpected":true}),
        serde_json::json!({"mode":"future"}),
    ] {
        assert!(serde_json::from_value::<InputPolicy>(value).is_err());
    }
}

#[test]
fn selected_connection_failure_has_generation_receipt_and_retry_keeps_filter() {
    let (mut engine, _rt) = Engine::headless_for_test(48_000, 32);
    let control = install_with_policy(&mut engine, InputPolicy::Selected(vec!["Wanted".into()]));
    let ports = [("1", "Wanted"), ("2", "Other")];
    control.discover(&ports);
    control.connect("1", Err("port busy"));
    let status = applied(&engine, 1);
    assert_eq!(status.error.as_deref(), Some("port busy"));
    assert!(status.missing_names.is_empty());
    assert_eq!(engine.midi.retry_connections(), Retry::Queued);
    control.discover(&ports);
    control.connect("1", Ok(()));
    until(|| !engine.midi.connections_busy());
    assert!(engine.midi.policy_status().unwrap().error.is_none());
    assert!(control.attempts.try_recv().is_err());
    assert!(engine.snapshot().midi[2].starts_with("[disabled]"));
}

#[test]
fn concurrent_policy_requests_have_unique_ordered_generations_and_one_latest_job() {
    let (mut engine, _rt) = Engine::headless_for_test(48_000, 32);
    let control = install(&mut engine);
    assert!(matches!(control.next(), Attempt::Discover)); // OS discovery held.
    let manager = engine.midi.connections.as_ref().unwrap();
    let barrier = std::sync::Barrier::new(9);
    let mut receipts = std::thread::scope(|scope| {
        let callers: Vec<_> = (0..8)
            .map(|i| {
                let barrier = &barrier;
                scope.spawn(move || {
                    barrier.wait();
                    (0..32)
                        .map(|j| {
                            let name = format!("input-{i}-{j}");
                            let generation = manager
                                .configure(InputPolicy::Selected(vec![name.clone()]))
                                .unwrap();
                            (generation, name)
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        barrier.wait();
        callers
            .into_iter()
            .flat_map(|caller| caller.join().unwrap())
            .collect::<Vec<_>>()
    });
    receipts.sort();
    assert_eq!(
        receipts
            .iter()
            .map(|(generation, _)| *generation)
            .collect::<Vec<_>>(),
        (2..=257).collect::<Vec<_>>()
    );
    let (generation, name) = receipts.last().unwrap();
    let pending = manager.policy_status();
    assert_eq!(pending.requested, *generation);
    assert_eq!(pending.applied, None);
    assert_eq!(
        pending.requested_policy.as_ref(),
        &InputPolicy::Selected(vec![name.clone()])
    );
    control.replies.send(Reply::Ports(Ok(vec![]))).unwrap();
    // Superseded discovery cannot acknowledge the new policy. It starts one
    // fresh pass with the last generation rather than 256 queued passes.
    control.discover(&[("last", name), ("excluded", "other")]);
    control.connect("last", Ok(()));
    let status = applied(&engine, *generation);
    assert!(status.missing_names.is_empty());
    assert!(control.attempts.try_recv().is_err());
}

#[test]
fn bounded_discovery_preview_does_not_limit_exact_policy_matching() {
    let (mut engine, _rt) = Engine::headless_for_test(48_000, 32);
    let control = install_with_policy(&mut engine, InputPolicy::Disabled);
    let mut ports: Vec<_> = (0..260)
        .map(|i| (i.to_string(), format!("Input {i:03}")))
        .collect();
    ports.push(("oversized".into(), "n".repeat(1025)));
    ports.push(("empty".into(), String::new()));
    ports.push(("nul".into(), "a\0b".into()));
    let borrowed: Vec<_> = ports
        .iter()
        .map(|(id, name)| (id.as_str(), name.as_str()))
        .collect();
    control.discover(&borrowed);
    let status = applied(&engine, 1);
    assert_eq!(status.available_inputs.len(), policy::MAX_AVAILABLE_INPUTS);
    assert!(status.available_truncated);
    assert_eq!(
        status.available_inputs.last().map(String::as_str),
        Some("Input 255")
    );
    assert!(control.attempts.try_recv().is_err());
    let generation = engine
        .midi
        .configure_inputs(InputPolicy::Selected(vec!["Input 259".into()]))
        .unwrap();
    control.discover(&borrowed);
    control.connect("259", Ok(()));
    let status = applied(&engine, generation);
    assert!(
        status.missing_names.is_empty(),
        "preview truncation must not claim a discovered selected input is missing"
    );
    assert!(control.attempts.try_recv().is_err());
}

#[test]
fn disabled_policy_applies_and_releases_owned_gates_even_when_discovery_fails() {
    let (mut engine, mut rt) = Engine::headless_for_test(48_000, 32);
    rt.selected_track = 1;
    let control = install(&mut engine);
    control.discover(&[("1", "Keyboard")]);
    let input = control.connect("1", Ok(()));
    applied(&engine, 1);
    input.push(&[0x90, 60, 100]);
    until(|| engine.midi.input_stats().dispatched == 1);
    rt.process(&mut [0.0; 128]);
    assert_eq!(held(&rt), 1);
    let generation = engine.midi.configure_inputs(InputPolicy::Disabled).unwrap();
    assert!(matches!(control.next(), Attempt::Discover));
    // Teardown precedes discovery, so a blocked/failing OS probe cannot keep
    // an excluded controller's held voice alive after the manager processes it.
    assert!(input.is_closed());
    rt.process(&mut [0.0; 128]);
    assert_eq!(held(&rt), 0);
    control
        .replies
        .send(Reply::Ports(Err("probe unavailable".repeat(200))))
        .unwrap();
    let status = applied(&engine, generation);
    assert_eq!(
        status.applied_policy.as_deref(),
        Some(&InputPolicy::Disabled)
    );
    assert_eq!(status.error.as_ref().unwrap().chars().count(), 1024);
    assert_eq!(status.available_inputs.as_ref(), ["Keyboard"]);
    assert!(engine.snapshot().midi[0].starts_with("[connected]"));
}
