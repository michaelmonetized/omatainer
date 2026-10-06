use super::*;
use crate::engine::input_monitor::Mode;

#[test]
fn input_mode_persists_with_arm_undo_and_legacy_files_keep_their_original_mix() {
    let (_engine, mut live) = crate::engine::Engine::headless_for_test(48000, 256);
    live.process(&mut []);
    let original = captured(&live);
    let mut old = serde_json::to_value(&original.state).unwrap();
    old["version"] = 15.into();
    let legacy: State = serde_json::from_value(old.clone()).unwrap();
    let reopened = Prepared::from_state(legacy, original.media.clone(), 48000).unwrap();
    assert_eq!(reopened.rt.tracks[2].input_monitor, None);
    let revision = live.project.revision();
    live.apply(Command::TrackMonitor {
        track: 2,
        mode: Mode::Off,
    });
    assert!(live.project.revision() > revision);
    live.apply(Command::Undo);
    assert_eq!(live.tracks[2].input_monitor, None);
    live.apply(Command::Redo);
    assert_eq!(live.tracks[2].input_monitor, Some(Mode::Off));
    live.apply(Command::TrackArm {
        track: 2,
        value: true,
    });
    live.apply(Command::TrackPfl {
        track: 2,
        value: true,
    });
    let saved = captured(&live);
    let json = serde_json::to_value(&saved.state).unwrap();
    assert_eq!(json["version"], 16);
    assert_eq!(json["tracks"][2]["input_monitor"], "off");
    let state: State = serde_json::from_value(json.clone()).unwrap();
    let reopened = Prepared::from_state(state, saved.media.clone(), 44100).unwrap();
    assert_eq!(reopened.rt.tracks[2].input_monitor, Some(Mode::Off));
    assert!(reopened.rt.tracks[2].armed);
    assert!(!reopened.rt.tracks[2].pfl);
    let mut false_version = json.clone();
    false_version["version"] = 15.into();
    assert!(serde_json::from_value::<State>(false_version).is_err());
    old["tracks"][2]["input_monitor"] = serde_json::Value::Null;
    assert!(serde_json::from_value::<State>(old).is_err());
    let mut invalid = json;
    invalid["tracks"][2]["input_monitor"] = "auto-ish".into();
    assert!(serde_json::from_value::<State>(invalid).is_err());
}
