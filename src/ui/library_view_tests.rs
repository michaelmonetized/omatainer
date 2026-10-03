use super::test_support::{label_center, Fixture};
use super::*;
use std::time::Duration;

fn items(count: usize) -> Vec<LibItem> {
    let fingerprint = crate::engine::media_source::FileFingerprint::read(std::path::Path::new("Cargo.toml"));
    (0..count)
        .map(|index| LibItem {
            source: LibSource::File(format!("private/{index}.wav").into()),
            title: format!("Track {index:05}"),
            artist: if index % 2 == 0 {
                "Even Artist".into()
            } else {
                "Odd Artist".into()
            },
            bpm: Bpm::hint(100.0 + index as f32 / 1000.0),
            fingerprint,
            key: "C".into(),
            length: Some(123.0 + index as f64),
            last_play: None,
        })
        .collect()
}

fn frame(
    ctx: &egui::Context,
    app: &mut App,
    time: f64,
    events: Vec<egui::Event>,
) -> egui::FullOutput {
    let input = egui::RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1440.0, 108.0))),
        time: Some(time),
        events,
        ..Default::default()
    };
    let theme = app.theme.clone();
    ctx.run(input, |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| app.crate_row(ui, &theme));
        // Exercise the full frame's controller-selection publication too: it
        // previously hid another full filter/Vec allocation behind crate work.
        app.publish_library_selection();
    })
}

fn key(key: Key) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Default::default(),
    }
}

fn click(ctx: &egui::Context, app: &mut App, point: Pos2, time: f64) {
    for (pressed, time) in [(true, time), (false, time + 0.01)] {
        frame(
            ctx,
            app,
            time,
            vec![
                egui::Event::PointerMoved(point),
                egui::Event::PointerButton {
                    pos: point,
                    button: PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                },
            ],
        );
    }
}

fn selected_source(app: &mut App) -> LibSource {
    app.selected_library_item().unwrap().source.clone()
}

#[test]
fn field_search_100000_track_catalog_has_exact_results_and_bounded_steady_frames() {
    let mut fixture = Fixture::new(32);
    let mut library = items(100_000);
    let catalog = Arc::make_mut(&mut fixture.app.library_metadata.catalog);
    for (index, item) in library.iter_mut().enumerate() {
        item.source = LibSource::File(format!("/synthetic/{index}.wav").into());
        item.bpm = Bpm::hint(100.0 + (index % 100) as f32);
        item.length = Some(120.0 + (index % 200) as f64);
        item.last_play = (index % 7 == 0).then_some(SystemTime::UNIX_EPOCH);
        catalog.upsert(item.source.clone(), item.fingerprint, item.stored_metadata()).unwrap();
        let fields = &mut catalog.tracks[index].annotations;
        fields.rating = (index % 6) as u8;
        fields.tags = vec![if index % 3 == 0 { "clean" } else { "explicit" }.into()];
    }
    fixture.app.library = Arc::new(library);
    let ctx = egui::Context::default();
    for (number, query) in [
        "artist:\"Even Artist\" bpm:120..128 rating>=4 tag:explicit played:no length:2:00..4:00 key:C",
        "title:\"Track 99999\"",
        "bpm>=105 length<=130 played:yes",
        "artist:\"Odd Artist\" rating:5",
    ].into_iter().enumerate() {
        fixture.app.lib_filter = query.into();
        let started = Instant::now();
        frame(&ctx, &mut fixture.app, number as f64, vec![]);
        let elapsed = started.elapsed();
        let expected: Vec<_> = (0..100_000).filter(|&i| match number {
            0 => i % 2 == 0 && (20..=28).contains(&(i % 100)) && i % 6 >= 4
                && i % 3 != 0 && i % 7 != 0 && i % 200 <= 120,
            1 => i == 99_999,
            2 => i % 100 >= 5 && i % 200 <= 10 && i % 7 == 0,
            _ => i % 2 == 1 && i % 6 == 5,
        }).collect();
        assert_eq!(*fixture.app.library_view.indices, expected, "{query}");
        eprintln!("100000-track field search {query:?}: {elapsed:?}, {} results", expected.len());
        assert!(elapsed < Duration::from_secs(5), "bounded search exceeded five-second local gate: {elapsed:?}");
        let rebuilds = fixture.app.library_view.stats.rebuilds;
        frame(&ctx, &mut fixture.app, number as f64 + 0.1, vec![]);
        assert_eq!(fixture.app.library_view.stats.rebuilds, rebuilds);
        assert_eq!(fixture.app.library_view.stats.formatted, 0);
        assert!(fixture.app.library_view.cells.len() <= 6);
    }
    fixture.app.lib_filter = "bpm:bad".into();
    let output = frame(&ctx, &mut fixture.app, 5.0, vec![]);
    assert!(fixture.app.library_view.indices.is_empty());
    label_center(&output, "Invalid bpm number");
}

#[test]
fn native_search_scope_restores_named_crate_query_source_and_scroll() {
    use crate::library::crates::{CrateId, Edit};
    let mut fixture = Fixture::new(32);
    let mut library = items(100);
    let catalog = Arc::make_mut(&mut fixture.app.library_metadata.catalog);
    for (index, item) in library.iter_mut().enumerate() {
        item.source = LibSource::File(format!("/synthetic/{index}.wav").into());
        catalog.upsert(item.source.clone(), item.fingerprint, item.stored_metadata()).unwrap();
    }
    let id = CrateId("00000000000000000000000000000139".into());
    catalog.crates.apply(0, &Edit::Create { id: id.clone(), name: "Opening".into(), parent: None, before: None }, |_|true).unwrap();
    let members = catalog.tracks[..80].iter().map(|track|track.id.clone()).collect();
    catalog.crates.apply(1, &Edit::AddMembers { id: id.clone(), members, before: None }, |_|true).unwrap();
    fixture.app.library = Arc::new(library);
    fixture.app.library_metadata.bind_test_rows(&fixture.app.library);
    fixture.app.choose_named_crate(Some(id.clone()));
    fixture.app.lib_filter = "artist:\"Even Artist\"".into();
    let ctx = egui::Context::default();
    frame(&ctx, &mut fixture.app, 0.0, vec![]);
    fixture.app.lib_sel = 25;
    fixture.app.library_view.pending_offset = Some(24.0 * fixture.app.library_view.stride);
    let output = frame(&ctx, &mut fixture.app, 0.1, vec![]);
    let source = selected_source(&mut fixture.app);
    let offset = fixture.app.library_view.offset;
    click(&ctx, &mut fixture.app, label_center(&output, "Search all library"), 0.2);
    assert!(fixture.app.library_view.search_all);
    assert_eq!(fixture.app.library_crates.selected, Some(id.clone()));
    assert_eq!(fixture.app.library_view.indices.len(), 50);
    fixture.app.lib_filter = "title:\"Track 00099\"".into();
    let output = frame(&ctx, &mut fixture.app, 0.4, vec![]);
    assert_eq!(fixture.app.library_view.indices.len(), 1);
    click(&ctx, &mut fixture.app, label_center(&output, "Search all library"), 0.5);
    frame(&ctx, &mut fixture.app, 0.6, vec![]);
    assert!(!fixture.app.library_view.search_all);
    assert_eq!(fixture.app.lib_filter, "artist:\"Even Artist\"");
    assert_eq!(selected_source(&mut fixture.app), source);
    assert!((fixture.app.library_view.offset - offset).abs() < 0.01);
    assert_eq!(fixture.app.library_view.indices.len(), 40);
}

#[test]
fn played_predicates_follow_confirmed_source_versions_without_rebuilding_on_repeated_plays() {
    let mut fixture = Fixture::new(32);
    fixture.app.library = Arc::new(items(100));
    fixture.app.lib_filter = "played:no".into();
    let ctx = egui::Context::default();
    frame(&ctx, &mut fixture.app, 0.0, vec![]);
    let source = &fixture.app.library[42];
    let identity = play_history::Identity::new(source.source.clone(), source.fingerprint).unwrap();
    fixture.app.last_played.record(&identity, SystemTime::UNIX_EPOCH);
    frame(&ctx, &mut fixture.app, 0.1, vec![]);
    assert_eq!(fixture.app.library_view.indices.len(), 99);
    assert!(!fixture.app.library_view.indices.contains(&42));
    let rebuilds = fixture.app.library_view.stats.rebuilds;
    fixture.app.last_played.record(&identity, SystemTime::UNIX_EPOCH + Duration::from_secs(1));
    frame(&ctx, &mut fixture.app, 0.2, vec![]);
    assert_eq!(fixture.app.library_view.stats.rebuilds, rebuilds);
    fixture.app.lib_filter = "played:yes".into();
    frame(&ctx, &mut fixture.app, 0.3, vec![]);
    assert_eq!(*fixture.app.library_view.indices, vec![42]);
    Arc::make_mut(&mut fixture.app.library)[42].fingerprint = None;
    frame(&ctx, &mut fixture.app, 0.4, vec![]);
    assert!(fixture.app.library_view.indices.is_empty());
}

#[test]
fn fixed_viewport_work_is_bounded_at_100_10000_and_50000_tracks() {
    let mut steady_rows = None;
    for count in [100, 10_000, 50_000] {
        let mut fixture = Fixture::new(32);
        fixture.app.library = Arc::new(items(count));
        let ctx = egui::Context::default();
        let began = Instant::now();
        frame(&ctx, &mut fixture.app, 0.0, vec![]);
        let cold = began.elapsed();
        let stats = fixture.app.library_view.stats;
        let mut timings = Vec::new();
        for number in 1..=110 {
            let began = Instant::now();
            frame(&ctx, &mut fixture.app, number as f64 / 60.0, vec![]);
            if number > 10 {
                timings.push(began.elapsed());
            }
            let view = &fixture.app.library_view;
            assert_eq!(view.stats.rebuilds, stats.rebuilds);
            assert_eq!(view.stats.examined, stats.examined);
            assert_eq!(
                view.stats.formatted, 0,
                "steady rows must reuse formatted cells"
            );
            assert!(
                view.stats.rendered > 0 && view.stats.rendered <= 6,
                "{:?}",
                view.stats
            );
            assert!(view.cells.len() <= 6);
            assert_eq!(
                *steady_rows.get_or_insert(view.stats.rendered),
                view.stats.rendered
            );
        }
        timings.sort();
        eprintln!("crate 1440x108 entries={count}: cold={cold:?}, steady median={:?}, p95={:?}, rendered={}, formatted=0, examined=0",
            timings[timings.len()/2], timings[timings.len()*95/100], steady_rows.unwrap());
        // Changing the query performs exactly one new filter pass, even when
        // the controller selection and repeated frames use it afterward.
        fixture.app.lib_filter = "EVEN artist".into();
        frame(&ctx, &mut fixture.app, 2.0, vec![]);
        assert_eq!(fixture.app.library_view.indices.len(), count / 2);
        assert_eq!(fixture.app.library_view.stats.rebuilds, stats.rebuilds + 1);
        assert_eq!(
            fixture.app.library_view.stats.examined,
            stats.examined + count
        );
        frame(&ctx, &mut fixture.app, 2.1, vec![]);
        assert_eq!(fixture.app.library_view.stats.formatted, 0);
        assert_eq!(fixture.app.library_view.stats.rebuilds, stats.rebuilds + 1);
    }
}

#[test]
fn filtering_and_metadata_refresh_reuse_sources_and_refresh_only_visible_cells() {
    let mut fixture = Fixture::new(32);
    fixture.app.library = Arc::new(items(100));
    let ctx = egui::Context::default();
    frame(&ctx, &mut fixture.app, 0.0, vec![]);
    fixture.app.lib_sel = 2;
    let source = selected_source(&mut fixture.app);
    fixture.app.lib_filter = "even".into();
    let output = frame(&ctx, &mut fixture.app, 0.1, vec![]);
    assert_eq!(fixture.app.lib_sel, 1);
    assert_eq!(selected_source(&mut fixture.app), source);
    assert_eq!(fixture.app.library_view.cells[&1].length, "2:05");
    assert!(output.shapes.iter().any(|shape| matches!(&shape.shape, egui::epaint::Shape::Text(text) if text.galley.text() == "Track 00002")));
    // A history edit updates that visible cell without a full filter rebuild.
    let rebuilt = fixture.app.library_view.stats.rebuilds;
    fixture.app.last_played.record(
        &play_history::Identity::new(source.clone(), fixture.app.library[2].fingerprint).unwrap(),
        SystemTime::UNIX_EPOCH + Duration::from_secs(555),
    );
    frame(&ctx, &mut fixture.app, 0.2, vec![]);
    assert_eq!(fixture.app.library_view.stats.formatted, 1);
    assert_eq!(fixture.app.library_view.stats.rebuilds, rebuilt);
    assert_eq!(fixture.app.library_view.cells[&1].played, "1970-01-01 UTC");
    // Weak cache ownership neither pins the large library nor misses mutation.
    assert_eq!(Arc::strong_count(&fixture.app.library), 1);
    Arc::make_mut(&mut fixture.app.library)[2].length = Some(600.0);
    frame(&ctx, &mut fixture.app, 0.3, vec![]);
    assert_eq!(fixture.app.library_view.stats.rebuilds, rebuilt + 1);
    assert_eq!(fixture.app.library_view.cells[&1].length, "10:00");
    assert_eq!(selected_source(&mut fixture.app), source);
    fixture.app.lib_filter.clear();
    frame(&ctx, &mut fixture.app, 0.4, vec![]);
    assert_eq!(fixture.app.lib_sel, 2);
    assert_eq!(selected_source(&mut fixture.app), source);
    fixture.app.lib_filter = "no matches".into();
    frame(&ctx, &mut fixture.app, 0.5, vec![]);
    assert!(fixture.app.library_view.indices.is_empty());
    assert!(fixture.app.library_view.cells.is_empty());
    assert_eq!(fixture.app.lib_sel, 0);
    assert!(fixture.app.published_selection.is_none());
}

#[test]
fn real_row_focus_keyboard_navigation_reveals_offscreen_selection_without_search_interference() {
    let mut fixture = Fixture::new(32);
    fixture.app.library = Arc::new(items(10_000));
    let ctx = egui::Context::default();
    let output = frame(&ctx, &mut fixture.app, 0.0, vec![]);
    click(
        &ctx,
        &mut fixture.app,
        label_center(&output, "Track 00000"),
        0.1,
    );
    frame(&ctx, &mut fixture.app, 0.2, vec![key(Key::End)]);
    assert_eq!(fixture.app.lib_sel, 9999);
    let output = frame(&ctx, &mut fixture.app, 0.3, vec![]);
    label_center(&output, "Track 09999");
    assert!(fixture.app.library_view.offset > 100_000.0);
    assert!(fixture.app.library_view.cells.len() <= 6);
    frame(&ctx, &mut fixture.app, 0.4, vec![key(Key::ArrowUp)]);
    assert_eq!(fixture.app.lib_sel, 9998);
    frame(&ctx, &mut fixture.app, 0.5, vec![key(Key::Home)]);
    assert_eq!(fixture.app.lib_sel, 0);
    frame(&ctx, &mut fixture.app, 0.6, vec![key(Key::PageDown)]);
    let after_page = fixture.app.lib_sel;
    assert!(after_page > 0 && after_page <= 3);
    let output = frame(&ctx, &mut fixture.app, 0.7, vec![]);
    // Focus the actual search TextEdit. Its arrows cannot navigate the crate.
    click(&ctx, &mut fixture.app, label_center(&output, "search"), 0.8);
    frame(&ctx, &mut fixture.app, 0.9, vec![key(Key::ArrowDown)]);
    assert_eq!(fixture.app.lib_sel, after_page);
    frame(
        &ctx,
        &mut fixture.app,
        1.0,
        vec![egui::Event::Text("Track 00002".into())],
    );
    assert_eq!(fixture.app.library_view.indices.len(), 1);
    assert_eq!(
        selected_source(&mut fixture.app),
        LibSource::File("private/2.wav".into())
    );
}

#[test]
fn reordered_publication_keeps_selected_source_and_scroll_anchor() {
    let mut fixture = Fixture::new(32);
    fixture.app.library = Arc::new(items(100));
    let ctx = egui::Context::default();
    frame(&ctx, &mut fixture.app, 0.0, vec![]);
    fixture.app.lib_sel = 42;
    fixture.app.library_view.pending_offset = Some(40.0 * fixture.app.library_view.stride + 3.0);
    frame(&ctx, &mut fixture.app, 0.1, vec![]);
    let source = selected_source(&mut fixture.app);
    let offset = fixture.app.library_view.offset;
    let stride = fixture.app.library_view.stride;
    let mut replacement = items(100);
    replacement.insert(
        0,
        LibItem {
            title: "Inserted first".into(),
            source: LibSource::File("private/new.wav".into()),
            ..replacement[0].clone()
        },
    );
    fixture.app.library = Arc::new(replacement);
    frame(&ctx, &mut fixture.app, 0.2, vec![]);
    assert_eq!(selected_source(&mut fixture.app), source);
    assert_eq!(fixture.app.lib_sel, 43);
    assert!((fixture.app.library_view.offset - offset - stride).abs() < 0.01);
    // Removing the selected source clamps to a valid neighboring row and does
    // not leave an invalid selection or a scroll position beyond the content.
    fixture.app.library = Arc::new(items(3));
    frame(&ctx, &mut fixture.app, 0.3, vec![]);
    frame(&ctx, &mut fixture.app, 0.4, vec![]);
    assert_eq!(fixture.app.lib_sel, 2);
    assert!(fixture.app.library_view.offset <= 3.0 * stride);
    assert!(fixture.app.published_selection.is_some());
}

#[test]
fn real_wheel_scrolling_keeps_work_bounded_and_selection_stable() {
    let mut fixture = Fixture::new(32);
    fixture.app.library = Arc::new(items(50_000));
    let ctx = egui::Context::default();
    let output = frame(&ctx, &mut fixture.app, 0.0, vec![]);
    let pointer = label_center(&output, "Track 00000");
    let source = selected_source(&mut fixture.app);
    frame(
        &ctx,
        &mut fixture.app,
        0.1,
        vec![
            egui::Event::PointerMoved(pointer),
            egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: Vec2::new(0.0, -3000.0),
                modifiers: Default::default(),
            },
        ],
    );
    for i in 1..=60 {
        frame(&ctx, &mut fixture.app, 0.1 + i as f64 / 60.0, vec![]);
        assert!(fixture.app.library_view.stats.rendered <= 6);
        assert!(fixture.app.library_view.stats.formatted <= 6);
        assert!(fixture.app.library_view.cells.len() <= 6);
    }
    assert!(fixture.app.library_view.offset > 500.0);
    assert_eq!(selected_source(&mut fixture.app), source);
    assert_eq!(fixture.app.lib_sel, 0);
    let offset = fixture.app.library_view.offset;
    let rebuilds = fixture.app.library_view.stats.rebuilds;
    frame(&ctx, &mut fixture.app, 1.2, vec![]);
    assert!((fixture.app.library_view.offset - offset).abs() < 0.1);
    assert_eq!(fixture.app.library_view.stats.rebuilds, rebuilds);
    assert_eq!(fixture.app.library_view.stats.formatted, 0);
}
