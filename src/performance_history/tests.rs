use super::*;
use crate::engine::history_measurement::Episode;

fn session() -> Session { Session::new("a".repeat(32), 1, 1000, 0).unwrap() }
fn source() -> Source { Source::Catalog { track_id: "b".repeat(32), version: 2,
    title: "Late set".into(), artist: "Composer".into() } }
fn observed(first: u64, frames: u32, rate: u32, class: Classification, wall_ns: u64) -> Observation {
    Observation { session: 1, wall_ns, episode: Episode { load: 11, generation: 1, deck: 0 },
        first_frame: first, frames, sample_rate: rate, classification: class }
}

#[test]
fn measured_manual_and_unplayed_entries_remain_distinct_through_roundtrip_and_export() {
    let mut set = session();
    let first = set.loaded(11, 0, source()).unwrap();
    let silent = set.loaded(12, 1, Source::Unresolved).unwrap();
    assert!(set.entries.iter().all(|entry| !entry.played()));
    set.observe(observed(0, 480, 48_000, Classification::BelowFloor, 1100), source()).unwrap();
    set.observe(observed(480, 441, 44_100, Classification::Active, 1200), source()).unwrap();
    assert!((set.entries[0].measured_seconds() - 0.01).abs() < 1e-12);
    set.mark(0, first, Some(false)).unwrap();
    assert!(!set.entries[0].played());
    assert_eq!(set.entries[0].rates[1].active, 441);
    assert!(set.mark(0, silent, Some(true)).is_err(), "stale revision must not change another entry");
    let external = set.external(1, "External turntable".into(), "Guest".into()).unwrap();
    assert_eq!(set.entries[2].id, external);
    assert!(set.entries[2].played()); assert_eq!(set.entries[2].measured_seconds(), 0.0);
    set.end(1500, 921, false, 0).unwrap();
    set.validate().unwrap();
    let restored: Session = serde_json::from_slice(&serde_json::to_vec(&set).unwrap()).unwrap();
    assert_eq!(restored, set); restored.validate().unwrap();
    let export: serde_json::Value = serde_json::from_slice(&restored.export().unwrap()).unwrap();
    assert_eq!(export["entries"][0]["source"]["track_id"], "b".repeat(32));
    assert_eq!(export["entries"][0]["played"], false);
    assert_eq!(export["entries"][0]["digital_main_frames_by_rate"][1]["active"], 441);
    assert_eq!(export["entries"][2]["source"]["kind"], "external");
    fn inspect(value: &serde_json::Value) {
        match value {
            serde_json::Value::Object(map) => for (key, value) in map {
                assert!(!["path", "source_path", "fingerprint", "load_key", "environment", "credentials"].contains(&key.as_str()));
                inspect(value);
            },
            serde_json::Value::Array(array) => for value in array { inspect(value); },
            _ => {},
        }
    }
    inspect(&export);
}

#[test]
fn unclean_recovery_uses_last_durable_observation_without_inventing_crash_time() {
    let mut set = session();
    set.observe(observed(0, 96, 48_000, Classification::Active, 1250), source()).unwrap();
    set.observe(observed(96, 44, 44_100, Classification::Ambiguous, 1400), source()).unwrap();
    let bytes = serde_json::to_vec(&set).unwrap();
    let mut restored: Session = serde_json::from_slice(&bytes).unwrap();
    restored.recover_unclean(); restored.validate().unwrap();
    assert_eq!(restored.state, State::Unclean);
    assert_eq!(restored.ended_ns, Some(1400));
    assert_eq!(restored.end_frame, Some(140));
    assert!(restored.incomplete);
    assert_eq!(restored.entries[0].rates[1].ambiguous, 44);
    let before = restored.clone(); restored.recover_unclean(); assert_eq!(restored, before);
}

#[test]
fn invalid_duplicate_foreign_and_overflowing_observations_cannot_inflate_duration() {
    let mut set = session();
    let value = observed(0, 480, 48_000, Classification::Active, 1100);
    set.observe(value, source()).unwrap();
    let before = set.clone();
    let mut foreign = observed(480, 48, 48_000, Classification::Active, 1200); foreign.session = 2;
    assert!(set.observe(foreign, source()).is_err()); assert_eq!(set, before);
    assert!(set.observe(value, source()).is_err());
    assert_eq!(set, before);
    for value in [observed(480, 0, 48_000, Classification::Active, 1200),
        observed(480, 2, 7_999, Classification::Active, 1200),
        observed(480, 2, 48_000, Classification::Active, 1),
        observed(u64::MAX, 2, 48_000, Classification::ClockOverflow, 1200)] {
        assert!(set.observe(value, source()).is_err()); assert_eq!(set, before);
    }
    let mut other = source(); if let Source::Catalog { track_id, .. } = &mut other { *track_id = "c".repeat(32); }
    assert!(set.loaded(11, 0, other).is_err()); assert_eq!(set, before);
    set.end(1300, 480, false, 0).unwrap();
    assert!(set.observe(observed(480, 1, 48_000, Classification::Active, 1250), source()).is_err());
}

#[test]
fn strict_schema_and_entry_limits_preserve_reviewable_history() {
    let mut set = session();
    for key in 1..=MAX_ENTRIES as u64 { set.loaded(key, 0, Source::Unresolved).unwrap(); }
    assert!(set.loaded(MAX_ENTRIES as u64 + 1, 0, Source::Unresolved).is_err());
    assert!(set.external(0, "Extra".into(), String::new()).is_err());
    set.validate().unwrap();
    let mut json = serde_json::to_value(session()).unwrap();
    json["source_path"] = serde_json::json!("/private/never-accepted.wav");
    assert!(serde_json::from_value::<Session>(json).is_err());
    assert!(Session::new("../escape".into(), 1, 1000, 0).is_err());
    let mut invalid = session(); invalid.schema = 99; assert!(invalid.validate().is_err());
    let mut invalid = session(); invalid.entries.push(Entry { id: 1, source: Source::External {
        title: "Manual".into(), artist: String::new() }, deck: Some(0), load_key: Some(11),
        rates: Vec::new(), played_override: None, first_active_ns: None, last_active_ns: None, last_frame: None });
    assert!(invalid.validate().is_err());
}
