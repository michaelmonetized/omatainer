use super::*;
use crate::track_gain::{Policy, Resolved};

#[test]
fn captured_gain_reopens_from_embedded_pcm_without_changing_fader_or_source() {
    let mut original = rt();
    let level = original.decks[0]
        .load_receipt
        .as_ref()
        .unwrap()
        .source_level();
    original.decks[0].source_gain = Resolved::prepare(
        Policy::Auto {
            target_dbfs: -20.0,
            peak_dbfs: -4.0,
        },
        level,
    )
    .unwrap();
    let saved = captured(&original);
    let policy = saved.state.decks[0].source_gain;
    let expected = original.decks[0].source_gain;
    let prepared = Prepared::from_state(saved.state.clone(), saved.media.clone(), 96_000).unwrap();
    assert_eq!(prepared.rt.decks[0].source_gain, expected);
    assert_eq!(prepared.rt.decks[0].gain, original.decks[0].gain);
    assert!(Arc::ptr_eq(
        prepared.rt.decks[0].audio.as_ref().unwrap(),
        original.decks[0].audio.as_ref().unwrap()
    ));
    assert_eq!(
        prepared.rt.decks[0]
            .load_receipt
            .as_ref()
            .unwrap()
            .preparation()
            .unwrap()
            .1
            .source_gain,
        policy
    );
    let mut legacy = serde_json::to_value(&saved.state).unwrap();
    legacy["version"] = 13.into();
    assert!(serde_json::from_value::<State>(legacy.clone()).is_err());
    legacy["decks"][0]
        .as_object_mut()
        .unwrap()
        .remove("source_gain");
    let migrated: State = serde_json::from_value(legacy.clone()).unwrap();
    assert_eq!(migrated.decks[0].source_gain, Policy::Off);
    for value in [
        serde_json::Value::Null,
        serde_json::json!({"mode":"off"}),
        serde_json::json!({}),
    ] {
        legacy["decks"][0]["source_gain"] = value;
        assert!(serde_json::from_value::<State>(legacy.clone()).is_err());
    }
}
