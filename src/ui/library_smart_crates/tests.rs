use super::*;
use crate::ui::library_annotations::tests::{Files, Gui};

fn crate_gui(files: &Files) -> Gui {
    let mut gui = Gui::new(files);
    gui.app.library_annotations.open = false;
    gui.app.submit_crate_edit(
        gui.app.library_metadata.catalog.crates.revision(),
        CollectionAction::Create {
            name: "Automatic".into(),
            parent: None,
            before: None,
        },
    );
    gui.finish();
    gui.app.library_crates.open = true;
    gui.frame(vec![]);
    gui.click("Smart crate rules…");
    gui.frame(vec![]);
    gui.app.smart_crates.rule = Rule {
        combine: Combine::All,
        conditions: vec![Condition::Text {
            field: TextField::Title,
            comparison: TextMatch::Contains,
            value: "draft".into(),
        }],
    };
    gui.frame(vec![]);
    gui
}

#[test]
fn actual_native_preview_save_refresh_and_remove_round_trip_the_catalog() {
    let files = Files::new();
    let mut gui = crate_gui(&files);
    let id = gui.app.library_crates.selected.clone().unwrap();
    gui.app
        .engine
        .send(crate::engine::Command::DeckPlay { deck: 0 })
        .unwrap();
    gui.frame(vec![]);
    let media = gui.app.snap.decks[0].media_key;
    gui.text("Smart condition 1 text", "One");
    gui.click("Any condition");
    assert_eq!(gui.app.smart_crates.rule.combine, Combine::Any);
    gui.click("All conditions");
    gui.click("Preview smart crate");
    gui.wait(|gui| gui.app.smart_crates.active.is_none());
    assert_eq!(gui.app.smart_crates.reviewed.as_ref().unwrap().count, 1);
    assert!(gui
        .app
        .library_metadata
        .catalog
        .crates
        .node(&id)
        .unwrap()
        .smart_rule
        .is_none());
    gui.click("Save previewed smart rule");
    gui.finish();
    gui.app.refresh_library_view();
    assert_eq!(gui.app.library_view.indices.len(), 1);
    assert!(gui.app.library[gui.app.library_view.indices[0]]
        .title
        .contains("One"));
    assert!(gui.app.snap.decks[0].playing);
    assert_eq!(gui.app.snap.decks[0].media_key, media);
    let saved: crate::library::Catalog =
        serde_json::from_slice(&std::fs::read(files.0.join("catalog.json")).unwrap()).unwrap();
    assert_eq!(
        saved.crates.node(&id).unwrap().smart_rule,
        Some(gui.app.smart_crates.rule.clone())
    );
    assert_eq!(saved.schema, crate::library::Catalog::default().schema);
    gui.click("Refresh smart membership");
    gui.finish();
    assert_eq!(gui.app.library_view.indices.len(), 1);
    gui.click("Remove smart rule");
    gui.finish();
    gui.app.refresh_library_view();
    assert!(gui
        .app
        .library_metadata
        .catalog
        .crates
        .node(&id)
        .unwrap()
        .smart_rule
        .is_none());
    assert!(gui.app.library_view.indices.is_empty());
    assert!(gui.app.snap.decks[0].playing);
    assert_eq!(gui.app.snap.decks[0].media_key, media);
}

#[test]
fn edited_closed_or_replaced_previews_never_authorize_a_stale_save() {
    let files = Files::new();
    let mut gui = crate_gui(&files);
    gui.text("Smart condition 1 text", "One");
    gui.click("Preview smart crate");
    gui.wait(|gui| gui.app.smart_crates.active.is_none());
    assert!(gui.app.smart_crates.reviewed.is_some());
    let old_save = gui.node("Save previewed smart rule");
    gui.text("Smart condition 1 text", "Two");
    assert!(gui.app.smart_crates.reviewed.is_none());
    gui.click("Preview smart crate");
    gui.app.smart_crates.open = false;
    gui.wait(|gui| gui.app.smart_crates.active.is_none());
    assert!(gui.app.smart_crates.reviewed.is_none());
    gui.app.smart_crates.open = true;
    gui.frame(vec![]);
    gui.click("Preview smart crate");
    gui.wait(|gui| gui.app.smart_crates.active.is_none());
    assert!(gui.app.smart_crates.reviewed.is_some());
    assert_ne!(old_save, gui.node("Save previewed smart rule"));
    gui.frame(vec![egui::Event::AccessKitActionRequest(
        egui::accesskit::ActionRequest {
            target: old_save,
            action: egui::accesskit::Action::Click,
            data: None,
        },
    )]);
    assert!(gui.app.smart_crates.reviewed.is_some());
    assert!(gui
        .app
        .library_metadata
        .catalog
        .crates
        .nodes()
        .iter()
        .all(|node| node.smart_rule.is_none()));
    gui.app.library = Arc::new((*gui.app.library).clone());
    gui.frame(vec![]);
    assert!(gui.app.smart_crates.reviewed.is_none());
    assert!(gui
        .app
        .library_metadata
        .catalog
        .crates
        .nodes()
        .iter()
        .all(|node| node.smart_rule.is_none()));
}

#[test]
fn native_saved_numeric_rule_follows_real_annotation_updates_and_keeps_playback() {
    use crate::library::annotations::Patch;
    use egui::accesskit::{Action, ActionData};
    let files = Files::new();
    let mut gui = crate_gui(&files);
    gui.app.smart_crates.rule = Rule {
        combine: Combine::All,
        conditions: vec![Condition::Number {
            field: NumberField::Rating,
            minimum: 0.0,
            maximum: 5.0,
        }],
    };
    gui.frame(vec![]);
    gui.action(
        "Smart condition 1 minimum",
        Action::SetValue,
        Some(ActionData::NumericValue(4.0)),
    );
    assert_eq!(
        gui.app.smart_crates.rule.conditions,
        vec![Condition::Number {
            field: NumberField::Rating,
            minimum: 4.0,
            maximum: 5.0
        }]
    );
    gui.click("Preview smart crate");
    gui.wait(|gui| gui.app.smart_crates.active.is_none());
    assert_eq!(gui.app.smart_crates.reviewed.as_ref().unwrap().count, 0);
    gui.click("Save previewed smart rule");
    gui.finish();
    let member = gui.app.library_metadata.catalog.tracks[0].id.clone();
    gui.app
        .engine
        .send(crate::engine::Command::DeckPlay { deck: 0 })
        .unwrap();
    gui.frame(vec![]);
    let media = gui.app.snap.decks[0].media_key;
    for (rating, count) in [(5, 1), (1, 0)] {
        gui.app.submit_crate_edit(
            gui.app.library_metadata.catalog.crates.revision(),
            CollectionAction::Annotate {
                ids: vec![member.clone()],
                patch: Patch {
                    rating: Some(rating),
                    ..Default::default()
                },
            },
        );
        gui.finish();
        gui.app.refresh_library_view();
        assert_eq!(gui.app.library_view.indices.len(), count);
        assert!(gui.app.snap.decks[0].playing);
        assert_eq!(gui.app.snap.decks[0].media_key, media);
    }
}
