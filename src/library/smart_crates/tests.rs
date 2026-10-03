use super::*;
use crate::library::annotations::Annotations;

#[test]
fn typed_all_any_rules_cover_every_supported_field_and_unknown_values() {
    let fields = Annotations {
        rating: 4,
        color: None,
        group: "Peak Time".into(),
        tags: vec!["CLEAN".into()],
        notes: "Café request".into(),
    };
    let conditions = vec![
        Condition::Text {
            field: TextField::Title,
            comparison: TextMatch::Contains,
            value: "SUN".into(),
        },
        Condition::Text {
            field: TextField::Artist,
            comparison: TextMatch::Equals,
            value: "STRAẞE".into(),
        },
        Condition::Text {
            field: TextField::Key,
            comparison: TextMatch::Equals,
            value: "8a".into(),
        },
        Condition::Text {
            field: TextField::Tag,
            comparison: TextMatch::Equals,
            value: "clean".into(),
        },
        Condition::Text {
            field: TextField::Group,
            comparison: TextMatch::Contains,
            value: "peak".into(),
        },
        Condition::Text {
            field: TextField::Note,
            comparison: TextMatch::Contains,
            value: "Cafe\u{301}".into(),
        },
        Condition::Number {
            field: NumberField::Bpm,
            minimum: 120.0,
            maximum: 128.0,
        },
        Condition::Number {
            field: NumberField::Length,
            minimum: 180.0,
            maximum: 240.0,
        },
        Condition::Number {
            field: NumberField::Rating,
            minimum: 4.0,
            maximum: 5.0,
        },
        Condition::Played { value: false },
    ];
    let row = || Row {
        title: "Sunrise",
        artist: "strasse",
        key: "8A",
        bpm: Some(128.0),
        seconds: Some(180.0),
        played: false,
        annotations: &fields,
    };
    for condition in &conditions {
        assert!(
            Rule {
                combine: Combine::All,
                conditions: vec![condition.clone()]
            }
            .compile()
            .unwrap()
            .matches(row()),
            "{condition:?}"
        );
    }
    let mut rule = Rule {
        combine: Combine::All,
        conditions,
    };
    assert!(rule.compile().unwrap().matches(row()));
    rule.conditions.push(Condition::Played { value: true });
    assert!(!rule.compile().unwrap().matches(row()));
    rule.combine = Combine::Any;
    assert!(rule.compile().unwrap().matches(row()));
    for field in [NumberField::Bpm, NumberField::Length] {
        let compiled = Rule {
            combine: Combine::All,
            conditions: vec![Condition::Number {
                field,
                minimum: 0.0,
                maximum: 500.0,
            }],
        }
        .compile()
        .unwrap();
        let mut unknown = row();
        unknown.bpm = None;
        unknown.seconds = None;
        assert!(!compiled.matches(unknown));
    }
}

#[test]
fn malformed_imports_and_bounds_fail_before_compilation() {
    for conditions in [
        vec![],
        vec![Condition::Played { value: true }; 17],
        vec![Condition::Text {
            field: TextField::Key,
            comparison: TextMatch::Equals,
            value: "\n".into(),
        }],
        vec![Condition::Number {
            field: NumberField::Bpm,
            minimum: f64::NAN,
            maximum: 120.0,
        }],
        vec![Condition::Number {
            field: NumberField::Bpm,
            minimum: 130.0,
            maximum: 120.0,
        }],
        vec![Condition::Number {
            field: NumberField::Rating,
            minimum: 0.5,
            maximum: 4.0,
        }],
        vec![Condition::Number {
            field: NumberField::Length,
            minimum: 0.0,
            maximum: 604801.0,
        }],
    ] {
        assert!(Rule {
            combine: Combine::All,
            conditions
        }
        .compile()
        .is_err());
    }
    for value in [
        r#"{"combine":"xor","conditions":[]}"#,
        r#"{"combine":"all","conditions":[{"kind":"played","value":true,"extra":0}]}"#,
        r#"{"combine":"all","conditions":[{"kind":"number","field":"playcount","minimum":0,"maximum":1}]}"#,
    ] {
        assert!(serde_json::from_str::<Rule>(value).is_err());
    }
    let rule = Rule::default();
    assert_eq!(
        serde_json::from_str::<Rule>(&serde_json::to_string(&rule).unwrap()).unwrap(),
        rule
    );
}

#[test]
fn smart_rules_preserve_manual_members_and_fail_closed_in_legacy_or_invalid_catalogs() {
    use crate::engine::{media_analysis::tests::Files, media_source::LibSource};
    use crate::library::{
        crates::{CrateId, Edit},
        Catalog, Metadata, Store,
    };
    use crate::ui::bpm::Bpm;
    let files = Files::new();
    let path = files.0.join("smart.json");
    let mut catalog = Catalog::default();
    catalog
        .upsert(
            LibSource::File(files.0.join("absent.wav")),
            None,
            Metadata {
                title: "Saved".into(),
                artist: "Artist".into(),
                bpm: Bpm::UNKNOWN,
                key: "C".into(),
                duration: None,
                last_play: None,
            },
        )
        .unwrap();
    let id = CrateId("00000000000000000000000000000205".into());
    catalog
        .edit_crates(
            0,
            &Edit::Create {
                id: id.clone(),
                name: "Saved".into(),
                parent: None,
                before: None,
            },
        )
        .unwrap();
    let member = catalog.tracks[0].id.clone();
    catalog
        .edit_crates(
            1,
            &Edit::AddMembers {
                id: id.clone(),
                members: vec![member.clone()],
                before: None,
            },
        )
        .unwrap();
    assert!(catalog
        .edit_crates(
            2,
            &Edit::SetSmartRule {
                id: id.clone(),
                rule: Some(Rule::default())
            }
        )
        .is_err());
    assert_eq!(
        catalog.crates.node(&id).unwrap().members,
        vec![member.clone()]
    );
    catalog
        .edit_crates(
            2,
            &Edit::RemoveMembers {
                id: id.clone(),
                members: vec![member.clone()],
            },
        )
        .unwrap();
    catalog
        .edit_crates(
            3,
            &Edit::SetSmartRule {
                id: id.clone(),
                rule: Some(Rule::default()),
            },
        )
        .unwrap();
    assert!(catalog
        .edit_crates(
            4,
            &Edit::AddMembers {
                id: id.clone(),
                members: vec![member],
                before: None
            }
        )
        .is_err());
    assert!(catalog
        .edit_crates(
            4,
            &Edit::SetAnnotationRule {
                id: id.clone(),
                rule: Some(Default::default())
            }
        )
        .is_err());
    let mut store = Store::open(path.clone()).unwrap();
    store.catalog = catalog;
    store.save().unwrap();
    drop(store);
    let saved = Store::open(path.clone()).unwrap();
    assert_eq!(
        saved.catalog.crates.node(&id).unwrap().smart_rule,
        Some(Rule::default())
    );
    let value = serde_json::to_value(&saved.catalog).unwrap();
    drop(saved);
    for schema in [8, 9] {
        let mut legacy = value.clone();
        legacy["schema"] = schema.into();
        let bytes = serde_json::to_vec(&legacy).unwrap();
        std::fs::write(&path, &bytes).unwrap();
        assert!(Store::open(path.clone())
            .err()
            .unwrap()
            .contains("require library schema 10"));
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
    }
    let mut old = value.clone();
    old["schema"] = 9.into();
    old["crates"]["nodes"][0]
        .as_object_mut()
        .unwrap()
        .remove("smart_rule");
    std::fs::write(&path, serde_json::to_vec(&old).unwrap()).unwrap();
    let migrated = Store::open(path.clone()).unwrap();
    assert_eq!(migrated.catalog.schema, 10);
    assert!(migrated
        .catalog
        .crates
        .node(&id)
        .unwrap()
        .smart_rule
        .is_none());
    drop(migrated);
    for conditions in [
        serde_json::json!([]),
        serde_json::json!([{"kind":"number","field":"bpm","minimum":130,"maximum":120}]),
    ] {
        let mut invalid = value.clone();
        invalid["crates"]["nodes"][0]["smart_rule"]["conditions"] = conditions;
        let bytes = serde_json::to_vec(&invalid).unwrap();
        std::fs::write(&path, &bytes).unwrap();
        assert!(Store::open(path.clone()).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
    }
}
