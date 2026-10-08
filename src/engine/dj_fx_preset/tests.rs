use super::*;
use crate::engine::{dj_fx_recall::{Phase, Request, Transition}, surface_controls::{fx::Placement, Input, State}, test_alloc, Command, Engine};
use std::sync::{atomic::AtomicBool, Arc};

fn fixture() -> Preset {
    let state = State::new(8000.0).unwrap(); let mut units = state.status.fx.map(Settings::from);
    units[0].name = Name::new("Low echo · A").unwrap(); units[1].name = Name::new("Room / B").unwrap();
    units[0].kinds = [FxKind::Reverb, FxKind::Filter, FxKind::Echo]; units[0].wet = [0.11, 0.32, 0.76]; units[0].parameter = [0.84, 0.19, 0.26]; units[0].on = [true, false, true]; units[0].manual_ms = 123.25; units[0].timing = Timing::Manual; units[0].beats = -3; units[0].placement = Placement::PostFader; units[0].master = true;
    units[1].sampler = Some(Reference { namespace: [8, 9], id: crate::engine::session::Id(17) });
    Preset { version: 1, name: Name::new("Night routine ✦").unwrap(), units }
}
fn directory(label: &str) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!("omatainer-fx-{label}-{}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos())); std::fs::create_dir(&path).unwrap(); path
}
#[test]
fn durable_versioned_native_file_keeps_names_order_routes_parameters_and_never_overwrites() {
    let directory = directory("roundtrip"); let path = directory.join("night.omatfx"); let preset = fixture(); let cancel = AtomicBool::new(false);
    assert!(matches!(save(&path, preset, &cancel).unwrap(), crate::project_file::SaveOutcome::Durable));
    let bytes = std::fs::read(&path).unwrap(); assert_eq!(load(&path, &cancel).unwrap(), preset);
    assert!(save(&path, fixture(), &cancel).is_err()); assert_eq!(std::fs::read(&path).unwrap(), bytes);
    let cancelled = directory.join("cancelled.omatfx"); assert!(save(&cancelled, preset, &AtomicBool::new(true)).is_err()); assert!(!cancelled.exists());
    for version in [0, 2, u32::MAX] {
        let mut future = preset; future.version = version; let path = directory.join(format!("v{version}.omatfx"));
        crate::project_file::save(&path, &crate::project_file::Bundle { state: future, media: vec![] }, crate::project_file::Overwrite::Never, &limits(), &cancel).unwrap(); assert!(load(&path, &cancel).is_err());
    }
    let corrupt = directory.join("corrupt.omatfx"); let mut broken = bytes; broken.truncate(broken.len() / 2); std::fs::write(&corrupt, broken).unwrap(); assert!(load(&corrupt, &cancel).is_err());
    println!("DJ_FX_PRESET_FILE {{\"native_version\":1,\"durable_roundtrip\":true,\"no_overwrite\":true,\"physical_devices_opened\":false}}");
}
#[test]
fn malformed_ranges_names_fields_and_stable_reference_shape_are_refused_before_publication() {
    for name in ["", " leading", "trailing ", "line\n", "\0"] { assert!(Name::new(name).is_err()); }
    assert!(Name::new(&"x".repeat(81)).is_err()); assert!(Name::new(&"é".repeat(41)).is_err()); assert_eq!(Name::new(&"é".repeat(40)).unwrap().as_str().len(), 80);
    let preset = fixture(); let mut value = serde_json::to_value(preset).unwrap(); value["unknown"] = serde_json::json!(true); assert!(serde_json::from_value::<Preset>(value).is_err());
    for wet in [f32::NAN, f32::INFINITY, -0.01, 1.01] { let mut invalid = preset; invalid.units[0].wet[0] = wet; assert!(invalid.validate().is_err()); }
    for duration in [0.0, 1998.1, f32::NAN] { let mut invalid = preset; invalid.units[1].manual_ms = duration; assert!(invalid.validate().is_err()); }
    for target in [Reference { namespace: [0, 0], id: crate::engine::session::Id(1) }, Reference { namespace: [1, 2], id: crate::engine::session::Id(0) }] { let mut invalid = preset; invalid.units[0].sampler = Some(target); assert!(invalid.validate().is_err()); }
}
#[test]
fn exact_five_ms_atomic_commit_busy_rejection_and_stale_project_or_control_review_keep_controls() {
    let (_, actual) = Engine::headless_for_test(8000, 64); let current = actual.surface.status.fx.map(Settings::from); let mut desired = current; desired[0].wet = [0.23; 3]; desired[1].name = Name::new("New B").unwrap();
    for rate in [8000.0_f32, 44100.0, 48000.0, 96000.0, 192000.0] {
        let mut transition = Transition::default(); let request = Request { token: 7, namespace: actual.session.namespace, expected: current, desired };
        assert!(transition.begin(request, current, &actual.session, rate)); assert!(!transition.begin(Request { token: 8, ..request }, current, &actual.session, rate)); assert_eq!(transition.receipt.token, 7); assert_eq!(transition.receipt.last_rejected, 8);
        let length = (rate * 0.005).ceil() as usize; let mut applied = current;
        for frame in 1..=length { let next = transition.frame(applied, &actual.session); if frame < length { assert!(next.is_none()); } else { assert_eq!(next, Some(desired)); applied = next.unwrap(); assert_eq!(transition.gain, 0.0); } }
        assert_eq!(transition.receipt.phase, Phase::FadingIn);
        for _ in 0..length { assert!(transition.frame(applied, &actual.session).is_none()); }
        assert_eq!(transition.receipt.phase, Phase::Applied); assert_eq!(transition.gain, 1.0);
        assert!(transition.begin(Request { expected: applied, ..request }, applied, &actual.session, rate)); let mut changed = applied; changed[1].wet[0] = 0.7; assert!(transition.frame(changed, &actual.session).is_none()); assert_eq!(transition.receipt.phase, Phase::Stale); for _ in 0..length { assert!(transition.frame(changed, &actual.session).is_none()); } assert_eq!(transition.gain, 1.0);
        assert!(!transition.begin(Request { namespace: [13, 14], ..request }, current, &actual.session, rate)); assert_eq!(transition.receipt.phase, Phase::Stale);
    }
}
#[test]
fn playing_atomic_recall_preserves_independent_pcm_and_has_no_callback_heap_work() {
    let (engine, mut actual) = Engine::headless_for_test(48000, 128); let (_, mut reference) = Engine::headless_for_test(48000, 128);
    let audio = Arc::new(crate::engine::dsp::Sample { spectrum: None, name: "Independent B".into(), sr: 48000, ch: 2, data: (0..480000).map(|frame| (frame as f32 * if frame % 2 == 0 {0.023} else {0.071}).sin() * 0.1).collect(), peaks: vec![].into(), bpm: 120.0, path: String::new() });
    for rt in [&mut actual, &mut reference] { rt.apply(Command::DeckAudio { deck: 1, audio: audio.clone() }); rt.decks[1].playing = true; rt.decks[1].rate = 1.0; rt.master = 1.0; rt.xfader = 1.0; }
    let current = actual.surface.status.fx.map(Settings::from); let mut desired = current; desired[0].on = [true; 3]; desired[0].kinds = [FxKind::Filter, FxKind::Echo, FxKind::Reverb]; desired[0].placement = Placement::PostFader; desired[0].name = Name::new("Atomic A").unwrap();
    engine.send(Command::Surface(Input::DjFxRecall(Request { token: 9, namespace: actual.session.namespace, expected: current, desired }))).unwrap();
    let mut nonzero = 0;
    for _ in 0..100 { let mut output = [0.0; 512]; let mut dry = [0.0; 512]; assert_eq!(test_alloc::measure(|| actual.process(&mut output)), test_alloc::Counts::default()); reference.process(&mut dry); assert_eq!(output, dry); if output.iter().any(|value| *value != 0.0) { nonzero += 1; } }
    assert_eq!(actual.surface.status.fx.map(Settings::from), desired); assert_eq!(actual.surface.status.fx_recall.phase, Phase::Applied); assert_eq!(nonzero, 100);
    println!("DJ_FX_PRESET_PLAYING {{\"exact_independent_pcm_blocks\":{nonzero},\"callback_heap_work\":false,\"physical_devices_opened\":false}}");
}
#[test]
fn reset_policy_removes_original_stereo_tails_and_rate_or_owner_retirement_cancels_unfinished_recall() {
    let (_, actual) = Engine::headless_for_test(8000, 64); let mut state = State::new(8000.0).unwrap(); state.status.fx[0].on[0] = true; state.status.fx[0].wet[0] = 0.8; state.status.fx[0].parameter[0] = 0.8; state.status.fx[0].timing = Timing::Manual; state.status.fx[0].manual_ms = 40.0;
    for _ in 0..100 { state.deck_fx_at(0, Placement::PreFader, [0.0; 2], 4000.0, 8000.0); }
    state.deck_fx_at(0, Placement::PreFader, [0.2, -0.1], 4000.0, 8000.0);
    let expected = state.status.fx.map(Settings::from); let mut desired = expected; desired[0].assigned = [false, true]; desired[0].name = Name::new("New route B").unwrap();
    let request = Request { token: 17, namespace: actual.session.namespace, expected, desired };
    assert!(state.fx_recall.begin(request, expected, &actual.session, 8000.0));
    for _ in 0..80 { state.fx_recall_frame(&actual.session); state.deck_fx_at(0, Placement::PreFader, [0.0; 2], 4000.0, 8000.0); state.deck_fx_at(1, Placement::PreFader, [0.0; 2], 4000.0, 8000.0); }
    assert_eq!(state.status.fx.map(Settings::from), desired); assert_eq!(state.status.fx_recall.phase, Phase::Applied);
    for _ in 0..2000 { assert_eq!(state.deck_fx_at(0, Placement::PreFader, [0.0; 2], 4000.0, 8000.0), [0.0; 2]); assert_eq!(state.deck_fx_at(1, Placement::PreFader, [0.0; 2], 4000.0, 8000.0), [0.0; 2]); }
    let request = Request { token: 18, expected: desired, desired: expected, ..request }; assert!(state.fx_recall.begin(request, desired, &actual.session, 8000.0)); state.fx_recall_frame(&actual.session);
    let changed_rate = state.at_rate(48000.0).unwrap(); assert_eq!(changed_rate.status.fx.map(Settings::from), desired); assert_eq!(changed_rate.status.fx_recall.phase, Phase::Stale);
    state.reset_fx_histories(); assert_eq!(state.status.fx.map(Settings::from), desired); assert_eq!(state.status.fx_recall.phase, Phase::Stale);
    println!("DJ_FX_PRESET_TAILS {{\"reset_old_routes\":true,\"owner_rate_retirement\":true,\"physical_devices_opened\":false}}");
}
#[test]
fn invalid_callback_requests_are_refused_and_foreign_sampler_routes_never_change_either_unit() {
    let (engine, mut actual) = Engine::headless_for_test(8000, 64); let expected = actual.surface.status.fx.map(Settings::from); let mut desired = expected; desired[0].sampler = Some(Reference { namespace: [42, 43], id: crate::engine::session::Id(1) });
    let request = Request { token: 31, namespace: actual.session.namespace, expected, desired };
    engine.send(Command::Surface(Input::DjFxRecall(request))).unwrap(); actual.process(&mut [0.0; 128]); assert_eq!(actual.surface.status.fx.map(Settings::from), expected); assert_eq!(actual.surface.status.fx_recall.phase, Phase::Stale);
    assert_eq!(engine.send(Command::Surface(Input::DjFxRecall(Request { token: 0, ..request }))), Err(crate::engine::control::SubmissionError::InvalidTarget));
    let mut invalid = request; invalid.desired[1].parameter[2] = f32::NAN; assert_eq!(engine.send(Command::Surface(Input::DjFxRecall(invalid))), Err(crate::engine::control::SubmissionError::InvalidTarget));
}
