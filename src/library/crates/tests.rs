use super::*;

type Forest = CrateForest<u32>;
fn id(value: u32) -> CrateId {
    CrateId(format!("{value:032x}"))
}
fn create(forest: &mut Forest, value: u32, parent: Option<u32>) {
    let revision = forest.revision();
    assert!(forest
        .apply(
            revision,
            &Edit::Create {
                id: id(value),
                name: format!("Crate {value}"),
                parent: parent.map(id),
                before: None,
            },
            |_| true
        )
        .unwrap());
}
fn apply(forest: &mut Forest, edit: Edit<u32>) -> bool {
    forest.apply(forest.revision(), &edit, |_| true).unwrap()
}
fn rejected(forest: &mut Forest, edit: Edit<u32>) -> Error {
    let before = serde_json::to_vec(forest).unwrap();
    let error = forest
        .apply(forest.revision(), &edit, |_| true)
        .unwrap_err();
    assert_eq!(
        serde_json::to_vec(forest).unwrap(),
        before,
        "failed edit mutated state"
    );
    error
}
fn members(forest: &Forest, value: u32) -> &[u32] {
    &forest.node(&id(value)).unwrap().members
}
fn add(forest: &mut Forest, value: u32, values: &[u32]) {
    apply(
        forest,
        Edit::AddMembers {
            id: id(value),
            members: values.into(),
            before: None,
        },
    );
}

#[test]
fn nested_overlapping_members_keep_manual_order_through_roundtrip() {
    let mut forest = Forest::default();
    create(&mut forest, 1, None);
    create(&mut forest, 2, Some(1));
    create(&mut forest, 3, Some(1));
    add(&mut forest, 1, &[20, 10, 30]);
    add(&mut forest, 2, &[30, 20]);
    add(&mut forest, 3, &[10]);
    apply(
        &mut forest,
        Edit::MoveMembers {
            source: id(1),
            destination: id(1),
            members: vec![30, 20],
            before: Some(10),
        },
    );
    assert_eq!(members(&forest, 1), [30, 20, 10]);
    assert_eq!(members(&forest, 2), [30, 20]);
    let bytes = serde_json::to_vec(&forest).unwrap();
    let recovered: Forest = serde_json::from_slice(&bytes).unwrap();
    recovered
        .validate(|member| [10, 20, 30].contains(member))
        .unwrap();
    assert_eq!(forest, recovered);
    assert_eq!(recovered.roots(), [id(1)]);
    assert_eq!(recovered.node(&id(1)).unwrap().children, [id(2), id(3)]);
}

#[test]
fn moves_are_atomic_groups_and_copy_is_idempotent_without_reordering() {
    let mut forest = Forest::default();
    create(&mut forest, 1, None);
    create(&mut forest, 2, None);
    add(&mut forest, 1, &[1, 2, 3, 4]);
    add(&mut forest, 2, &[4, 5, 2, 6]);
    apply(
        &mut forest,
        Edit::MoveMembers {
            source: id(1),
            destination: id(2),
            members: vec![2, 4],
            before: Some(5),
        },
    );
    assert_eq!(members(&forest, 1), [1, 3]);
    assert_eq!(members(&forest, 2), [2, 4, 5, 6]);
    let revision = forest.revision();
    assert!(!apply(
        &mut forest,
        Edit::AddMembers {
            id: id(2),
            members: vec![4, 2],
            before: None
        }
    ));
    assert_eq!(forest.revision(), revision);
    assert_eq!(members(&forest, 2), [2, 4, 5, 6]);
    apply(
        &mut forest,
        Edit::RemoveMembers {
            id: id(2),
            members: vec![4, 5],
        },
    );
    assert_eq!(members(&forest, 2), [2, 6]);
}

#[test]
fn tree_moves_order_and_cascade_delete_only_selected_subtree() {
    let mut forest = Forest::default();
    for n in [1, 2, 3] {
        create(&mut forest, n, None);
    }
    create(&mut forest, 4, Some(1));
    create(&mut forest, 5, Some(4));
    add(&mut forest, 5, &[9]);
    add(&mut forest, 3, &[9]);
    apply(
        &mut forest,
        Edit::MoveCrate {
            id: id(3),
            parent: None,
            before: Some(id(1)),
        },
    );
    assert_eq!(forest.roots(), [id(3), id(1), id(2)]);
    apply(
        &mut forest,
        Edit::MoveCrate {
            id: id(4),
            parent: Some(id(2)),
            before: None,
        },
    );
    assert!(forest.node(&id(1)).unwrap().children.is_empty());
    assert_eq!(forest.node(&id(2)).unwrap().children, [id(4)]);
    apply(&mut forest, Edit::DeleteSubtree { id: id(2) });
    assert_eq!(forest.roots(), [id(3), id(1)]);
    assert_eq!(forest.nodes().len(), 2);
    assert_eq!(members(&forest, 3), [9]);
}

#[test]
fn cycles_missing_targets_anchors_and_stale_revisions_never_mutate() {
    let mut forest = Forest::default();
    create(&mut forest, 1, None);
    create(&mut forest, 2, Some(1));
    add(&mut forest, 2, &[7, 8]);
    for edit in [
        Edit::MoveCrate {
            id: id(1),
            parent: Some(id(2)),
            before: None,
        },
        Edit::MoveCrate {
            id: id(1),
            parent: Some(id(1)),
            before: None,
        },
        Edit::MoveCrate {
            id: id(2),
            parent: None,
            before: Some(id(2)),
        },
        Edit::MoveCrate {
            id: id(2),
            parent: None,
            before: Some(id(999)),
        },
        Edit::Create {
            id: id(9),
            name: "New".into(),
            parent: Some(id(999)),
            before: None,
        },
        Edit::MoveMembers {
            source: id(2),
            destination: id(1),
            members: vec![7, 999],
            before: None,
        },
        Edit::MoveMembers {
            source: id(2),
            destination: id(1),
            members: vec![7],
            before: Some(8),
        },
        Edit::MoveMembers {
            source: id(2),
            destination: id(2),
            members: vec![7],
            before: Some(7),
        },
        Edit::RemoveMembers {
            id: id(2),
            members: vec![7, 7],
        },
    ] {
        rejected(&mut forest, edit);
    }
    let before = forest.clone();
    assert_eq!(
        forest.apply(0, &Edit::DeleteSubtree { id: id(1) }, |_| true),
        Err(Error::Conflict)
    );
    assert_eq!(forest, before);
    assert_eq!(
        forest.apply(
            forest.revision(),
            &Edit::AddMembers {
                id: id(1),
                members: vec![99],
                before: None
            },
            |k| *k < 99
        ),
        Err(Error::UnknownTrack)
    );
    assert_eq!(forest, before);
}

#[test]
fn names_are_bounded_and_unique_only_within_their_parent() {
    let mut forest = Forest::default();
    create(&mut forest, 1, None);
    create(&mut forest, 2, None);
    create(&mut forest, 3, Some(1));
    apply(
        &mut forest,
        Edit::Rename {
            id: id(3),
            name: "Crate 2".into(),
        },
    );
    for name in [
        "".into(),
        " space".into(),
        "space ".into(),
        "line\nbreak".into(),
        "x".repeat(MAX_NAME_BYTES + 1),
        "é".repeat(129),
        "Crate 2".into(),
    ] {
        rejected(&mut forest, Edit::Rename { id: id(1), name });
    }
    apply(
        &mut forest,
        Edit::Rename {
            id: id(1),
            name: "é".repeat(128),
        },
    );
    rejected(
        &mut forest,
        Edit::MoveCrate {
            id: id(3),
            parent: None,
            before: None,
        },
    );
    // Names are explicitly case-sensitive rather than guessing locale folding.
    apply(
        &mut forest,
        Edit::Rename {
            id: id(1),
            name: "crate 2".into(),
        },
    );
}

#[test]
fn noops_and_overflow_have_exact_revision_semantics() {
    let mut forest = Forest::default();
    create(&mut forest, 1, None);
    add(&mut forest, 1, &[4]);
    let revision = forest.revision();
    assert!(!apply(
        &mut forest,
        Edit::Rename {
            id: id(1),
            name: "Crate 1".into()
        }
    ));
    assert!(!apply(
        &mut forest,
        Edit::MoveCrate {
            id: id(1),
            parent: None,
            before: None
        }
    ));
    assert!(!apply(
        &mut forest,
        Edit::MoveMembers {
            source: id(1),
            destination: id(1),
            members: vec![4],
            before: None
        }
    ));
    assert!(!apply(
        &mut forest,
        Edit::RemoveMembers {
            id: id(1),
            members: vec![]
        }
    ));
    assert_eq!(forest.revision(), revision);
    forest.revision = u64::MAX;
    assert!(!apply(
        &mut forest,
        Edit::Rename {
            id: id(1),
            name: "Crate 1".into()
        }
    ));
    assert_eq!(
        rejected(
            &mut forest,
            Edit::Rename {
                id: id(1),
                name: "Changed".into()
            }
        ),
        Error::RevisionExhausted
    );
}

#[test]
fn malformed_trees_and_unknown_fields_fail_closed() {
    let mut forest = Forest::default();
    create(&mut forest, 1, None);
    create(&mut forest, 2, Some(1));
    let mut invalid = forest.clone();
    invalid.roots.push(id(2));
    assert!(invalid.validate(|_| true).is_err());
    let mut invalid = forest.clone();
    invalid.nodes[1].children.push(id(1));
    assert!(invalid.validate(|_| true).is_err());
    let mut invalid = forest.clone();
    invalid.roots.clear();
    assert!(invalid.validate(|_| true).is_err());
    let mut invalid = forest.clone();
    invalid.nodes[0].children[0] = id(999);
    assert!(invalid.validate(|_| true).is_err());
    let mut invalid = forest.clone();
    invalid.nodes[1].id = id(1);
    assert!(invalid.validate(|_| true).is_err());
    let mut invalid = forest.clone();
    invalid.nodes[1].members = vec![7, 7];
    assert!(invalid.validate(|_| true).is_err());
    let mut invalid = forest.clone();
    invalid.nodes[1].id = CrateId("X".repeat(32));
    assert!(invalid.validate(|_| true).is_err());
    let mut value = serde_json::to_value(&forest).unwrap();
    value["unexpected"] = true.into();
    assert!(serde_json::from_value::<Forest>(value).is_err());
    let mut value = serde_json::to_value(&forest).unwrap();
    value["nodes"][0]["path"] = "/audio.wav".into();
    assert!(serde_json::from_value::<Forest>(value).is_err());
}

#[test]
fn depth_limit_also_rejects_moving_an_existing_subtree_too_deep() {
    let mut forest = Forest::default();
    for value in 1..=MAX_DEPTH as u32 {
        create(&mut forest, value, (value > 1).then_some(value - 1));
    }
    rejected(
        &mut forest,
        Edit::Create {
            id: id(100),
            name: "Too deep".into(),
            parent: Some(id(MAX_DEPTH as u32)),
            before: None,
        },
    );
    create(&mut forest, 100, None);
    create(&mut forest, 101, Some(100));
    rejected(
        &mut forest,
        Edit::MoveCrate {
            id: id(100),
            parent: Some(id(MAX_DEPTH as u32 - 1)),
            before: None,
        },
    );
}

#[test]
fn node_and_membership_limits_are_validated_before_commit() {
    let mut forest = Forest::default();
    for value in 0..MAX_CRATES as u32 {
        forest.roots.push(id(value));
        forest.nodes.push(Node {
            annotation_rule: None,
            smart_rule: None,
            id: id(value),
            name: value.to_string(),
            children: vec![],
            members: vec![],
        });
    }
    forest.validate(|_| true).unwrap();
    rejected(
        &mut forest,
        Edit::Create {
            id: id(9999),
            name: "Over capacity".into(),
            parent: None,
            before: None,
        },
    );
    forest.nodes[0].members = (0..MAX_MEMBERS as u32).collect();
    forest.nodes[1].members = (0..MAX_MEMBERS as u32).collect();
    forest.nodes[2].members = (0..50_000).collect();
    forest.validate(|_| true).unwrap();
    rejected(
        &mut forest,
        Edit::AddMembers {
            id: id(2),
            members: vec![60_000],
            before: None,
        },
    );
    rejected(
        &mut forest,
        Edit::AddMembers {
            id: id(0),
            members: vec![100_001],
            before: None,
        },
    );
    rejected(
        &mut forest,
        Edit::AddMembers {
            id: id(3),
            members: (0..=MAX_MEMBERS as u32).collect(),
            before: None,
        },
    );
    // A full-capacity move consumes no additional global membership.
    assert!(apply(
        &mut forest,
        Edit::MoveMembers {
            source: id(2),
            destination: id(3),
            members: vec![10],
            before: None
        }
    ));
}

#[test]
fn imports_append_disjoint_roots_and_identical_overlap_is_idempotent() {
    let mut forest = Forest::default();
    create(&mut forest, 1, None);
    create(&mut forest, 2, Some(1));
    add(&mut forest, 2, &[1, 2]);
    let mut imported = forest.clone();
    create(&mut imported, 3, None);
    create(&mut imported, 4, Some(3));
    add(&mut imported, 4, &[2, 1]);
    let before = forest.revision();
    assert!(forest.merge_import(before, &imported, |_| true).unwrap());
    assert_eq!(forest.roots(), [id(1), id(3)]);
    assert_eq!(forest.revision(), before + 1);
    let before = forest.revision();
    imported.revision = u64::MAX;
    assert!(!forest.merge_import(before, &imported, |_| true).unwrap());
    assert_eq!(forest.revision(), before);
}

#[test]
fn conflicting_imports_leave_every_existing_membership_and_order_unchanged() {
    let mut forest = Forest::default();
    create(&mut forest, 1, None);
    create(&mut forest, 2, None);
    create(&mut forest, 3, Some(1));
    add(&mut forest, 1, &[4, 5]);
    let mut variants = vec![];
    let mut other = forest.clone();
    other.nodes[0].name = "Rename".into();
    variants.push(other);
    let mut other = forest.clone();
    other.nodes[0].members.reverse();
    variants.push(other);
    let mut other = forest.clone();
    other.roots.reverse();
    variants.push(other);
    let mut other = forest.clone();
    apply(
        &mut other,
        Edit::MoveCrate {
            id: id(3),
            parent: Some(id(2)),
            before: None,
        },
    );
    variants.push(other);
    let mut other = Forest::default();
    create(&mut other, 9, None);
    apply(
        &mut other,
        Edit::Rename {
            id: id(9),
            name: "Crate 1".into(),
        },
    );
    variants.push(other);
    let mut other = forest.clone();
    create(&mut other, 9, Some(1));
    variants.push(other);
    for other in variants {
        let bytes = serde_json::to_vec(&forest).unwrap();
        assert!(forest
            .merge_import(forest.revision(), &other, |_| true)
            .is_err());
        assert_eq!(serde_json::to_vec(&forest).unwrap(), bytes);
    }
    let mut unknown = Forest::default();
    create(&mut unknown, 9, None);
    add(&mut unknown, 9, &[99]);
    assert_eq!(
        forest.merge_import(forest.revision(), &unknown, |k| *k < 99),
        Err(Error::UnknownTrack)
    );
}

#[test]
fn identity_is_independent_of_paths_versions_and_catalog_iteration_order() {
    #[derive(Clone)]
    struct Track {
        id: u32,
        path: &'static str,
        version: u64,
    }
    let mut catalog = vec![
        Track {
            id: 7,
            path: "/a.wav",
            version: 1,
        },
        Track {
            id: 9,
            path: "/b.wav",
            version: 1,
        },
    ];
    let mut forest = Forest::default();
    create(&mut forest, 1, None);
    add(&mut forest, 1, &[9, 7]);
    let before = serde_json::to_vec(&forest).unwrap();
    catalog[0].path = "/relocated/a.wav";
    catalog[0].version += 1;
    catalog.reverse();
    forest
        .validate(|key| catalog.iter().any(|track| track.id == *key))
        .unwrap();
    assert_eq!(serde_json::to_vec(&forest).unwrap(), before);
    assert_eq!(members(&forest, 1), [9, 7]);
    // This pure-model assertion is not a substitute for the later real
    // Catalog/worker/relocation/source-file-preservation integration fixture.
}
