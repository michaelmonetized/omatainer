use super::*;
use crate::ui::{
    library_annotations::tests::{Files, Gui},
    test_support::Fixture,
};
use std::time::Duration;

fn item(index: usize, title: &str, artist: &str, bpm: Option<f32>) -> LibItem {
    LibItem {
        source: LibSource::File(format!("/synthetic/{index}.flac").into()),
        title: title.into(),
        artist: artist.into(),
        bpm: bpm.map_or(Bpm::UNKNOWN, Bpm::hint),
        key: "C".into(),
        length: Some(123.0),
        last_play: None,
        fingerprint: None,
    }
}
#[test]
fn primary_secondary_missing_values_unicode_and_equal_ties_have_exact_order() {
    let rows = vec![
        item(0, "Straße", "Z", Some(120.0)),
        item(1, "STRASSE", "A", Some(120.0)),
        item(2, "strasse", "A", None),
        item(3, "Álpha", "A", Some(125.0)),
        item(4, "A\u{301}LPHA", "A", Some(125.0)),
    ];
    let catalog = crate::library::Catalog::default();
    let history = play_history::History::default();
    let mut indices = vec![0, 1, 2, 3, 4];
    sort::order(
        &mut indices,
        &rows,
        &catalog,
        &history,
        [
            Some(Sort {
                column: Column::Title,
                descending: false,
            }),
            Some(Sort {
                column: Column::Artist,
                descending: false,
            }),
        ],
    );
    assert_eq!(indices, vec![1, 2, 0, 3, 4]);
    indices = vec![0, 1, 2, 3, 4];
    sort::order(
        &mut indices,
        &rows,
        &catalog,
        &history,
        [
            Some(Sort {
                column: Column::Bpm,
                descending: true,
            }),
            Some(Sort {
                column: Column::Title,
                descending: false,
            }),
        ],
    );
    assert_eq!(indices, vec![3, 4, 0, 1, 2]);
    indices = vec![4, 3, 2, 1, 0];
    sort::order(&mut indices, &rows, &catalog, &history, [None, None]);
    assert_eq!(indices, vec![4, 3, 2, 1, 0]);
}
#[test]
fn long_unicode_prefix_collisions_compare_the_complete_text() {
    let prefix = "é".repeat(200);
    let rows = vec![
        item(0, &format!("{prefix}z"), "", None),
        item(1, &format!("{prefix}a"), "", None),
    ];
    let mut indices = vec![0, 1];
    sort::order(
        &mut indices,
        &rows,
        &crate::library::Catalog::default(),
        &play_history::History::default(),
        [
            Some(Sort {
                column: Column::Title,
                descending: false,
            }),
            None,
        ],
    );
    assert_eq!(indices, vec![1, 0]);
}
#[test]
fn changing_sort_on_100000_rows_retains_source_and_viewport_without_steady_rebuilds() {
    let mut f = Fixture::new(256);
    f.app.library = Arc::new(
        (0..100_000)
            .map(|index| {
                item(
                    index,
                    &format!("曲 {:05}", 99_999 - index),
                    if index % 2 == 0 { "Straße" } else { "STRASSE" },
                    Some(120.0),
                )
            })
            .collect(),
    );
    f.app.refresh_library_view();
    f.app.lib_sel = 321;
    f.app.remember_crate_viewport(500.0 * 22.0, 220.0, 22.0);
    let source = f.app.selected_library_item().unwrap().source.clone();
    let start = Instant::now();
    f.app.sort_library_column(Column::Artist, false);
    f.app.sort_library_column(Column::Title, true);
    f.app.refresh_library_view();
    assert!(start.elapsed() < Duration::from_secs(5));
    assert_eq!(f.app.selected_library_item().unwrap().source, source);
    assert_eq!(f.app.library_view.indices[0], 99_999);
    assert_eq!(f.app.library_view.indices[99_999], 0);
    assert_eq!(f.app.library_view.pending_offset, Some(99_499.0 * 22.0));
    let generation = f.app.library_view.generation;
    let rebuilds = f.app.library_view.stats.rebuilds;
    for _ in 0..100 {
        f.app.refresh_library_view();
    }
    assert_eq!(f.app.library_view.generation, generation);
    assert_eq!(f.app.library_view.stats.rebuilds, rebuilds);
    f.app.library_layout.live.current_mut().primary = None;
    f.app.library_layout.live.current_mut().secondary = None;
    f.app.refresh_library_view();
    assert_eq!(f.app.selected_library_item().unwrap().source, source);
    assert_eq!(f.app.library_view.indices[0], 0);
}
#[test]
fn native_layout_controls_preview_save_and_reopen_the_exact_profile() {
    let files = Files::new();
    let mut gui = Gui::new(&files);
    gui.app.library_annotations.open = false;
    let prefs = files.0.join("preferences.json");
    gui.app.settings = preferences::Settings::with_worker_for_test(prefs.clone());
    gui.click("layout…");
    gui.click("Show BPM column");
    gui.click("Move Title column later");
    gui.action(
        "Title column width",
        egui::accesskit::Action::SetValue,
        Some(egui::accesskit::ActionData::NumericValue(380.0)),
    );
    gui.click("Primary sort");
    gui.click("Artist");
    gui.click("Secondary sort");
    gui.click("Title");
    gui.click("Artwork");
    gui.click("Preview layout");
    let layout = gui.app.library_layout.live.current();
    assert_eq!(layout.density, Density::Artwork);
    assert!(
        !layout
            .columns
            .iter()
            .find(|spec| spec.column == Column::Bpm)
            .unwrap()
            .visible
    );
    assert_eq!(layout.columns[1].column, Column::Title);
    assert_eq!(layout.columns[1].width, 380.0);
    assert_eq!(layout.primary.unwrap().column, Column::Artist);
    assert_eq!(layout.secondary.unwrap().column, Column::Title);
    let source = gui.app.selected_library_item().unwrap().source.clone();
    gui.app.library_layout.open = false;
    gui.frame(vec![]);
    gui.click("Sort library by Title");
    gui.click("Sort library by Title");
    assert!(
        gui.app
            .library_layout
            .live
            .current()
            .primary
            .unwrap()
            .descending
    );
    assert_eq!(gui.app.selected_library_item().unwrap().source, source);
    gui.click("layout…");
    gui.text("New library layout name", "Über Unicode");
    gui.click("Copy layout");
    gui.click("Save layouts");
    gui.wait(|gui| !gui.app.settings.busy());
    let saved =
        crate::preferences::storage::load(&prefs, &std::sync::atomic::AtomicBool::new(false))
            .unwrap();
    assert_eq!(
        saved.preferences.current().unwrap().library_layout,
        gui.app.library_layout.live
    );
    gui.app.library_layout.draft.current_mut().columns[1].width = 777.0;
    gui.click("Preview layout");
    assert_eq!(
        gui.app.library_layout.live.current().columns[1].width,
        777.0
    );
    gui.click("Discard layout preview");
    assert_eq!(
        gui.app.library_layout.live,
        saved.preferences.current().unwrap().library_layout
    );
    gui.app
        .settings
        .draft
        .profiles
        .get_mut("Studio")
        .unwrap()
        .shortcuts_enabled = false;
    gui.app.library_layout.draft.current_mut().columns[1].width = 888.0;
    gui.app.save_library_layout();
    assert!(gui.app.library_layout.message.contains("Preferences draft"));
    assert_eq!(
        crate::preferences::storage::load(&prefs, &std::sync::atomic::AtomicBool::new(false))
            .unwrap()
            .preferences,
        saved.preferences
    );
}

#[test]
fn actual_virtualized_cells_clip_long_text_to_their_resized_columns() {
    let mut f = Fixture::new(256);
    let title = "Long title ".repeat(100);
    let artist = "Long artist ".repeat(100);
    f.app.library = Arc::new(vec![item(0, &title, &artist, Some(120.0))]);
    for spec in &mut f.app.library_layout.live.current_mut().columns {
        spec.visible = matches!(spec.column, Column::Title | Column::Artist);
        spec.width = 64.0;
    }
    let ctx = egui::Context::default();
    let theme = f.app.theme.clone();
    let output = ctx.run(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1600.0, 600.0))),
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| f.app.crate_row(ui, &theme));
        },
    );
    let cells:Vec<_>=output.shapes.iter().filter(|shape|matches!(&shape.shape,egui::epaint::Shape::Text(text) if text.galley.job.text==title || text.galley.job.text==artist)).collect();
    assert_eq!(cells.len(), 2);
    let width = 64.0 * (theme.text_size(11.0) / 11.0).max(1.0);
    assert!(cells
        .iter()
        .all(|shape| shape.clip_rect.width() <= width + 0.01));
    assert!(cells[0].clip_rect.intersect(cells[1].clip_rect).width() <= 0.01);
}
