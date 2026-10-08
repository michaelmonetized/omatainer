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
    frame_with_height(ctx, app, time, events, 108.0)
}

fn wheel_frame(ctx: &egui::Context, app: &mut App, time: f64, events: Vec<egui::Event>) -> egui::FullOutput {
    frame_with_height(ctx, app, time, events, 144.0)
}

fn frame_with_height(ctx: &egui::Context, app: &mut App, time: f64, events: Vec<egui::Event>, height: f32) -> egui::FullOutput {
    let input = egui::RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1440.0, height))),
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
    let mut fixture = Fixture::new(48);
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
    let mut fixture = Fixture::new(48);
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
    let mut fixture = Fixture::new(48);
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
        let mut fixture = Fixture::new(48);
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
    let mut fixture = Fixture::new(48);
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
    let mut fixture = Fixture::new(48);
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
    let mut fixture = Fixture::new(48);
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
    let mut fixture = Fixture::new(48);
    fixture.app.library = Arc::new(items(50_000));
    let ctx = egui::Context::default();
    let output = wheel_frame(&ctx, &mut fixture.app, 0.0, vec![]);
    let pointer = label_center(&output, "Track 00000");
    let source = selected_source(&mut fixture.app);
    wheel_frame(
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
        wheel_frame(&ctx, &mut fixture.app, 0.1 + i as f64 / 60.0, vec![]);
        assert!(fixture.app.library_view.stats.rendered <= 6);
        assert!(fixture.app.library_view.stats.formatted <= 6);
        assert!(fixture.app.library_view.cells.len() <= 6);
    }
    assert!(fixture.app.library_view.offset > 500.0);
    assert_eq!(selected_source(&mut fixture.app), source);
    assert_eq!(fixture.app.lib_sel, 0);
    let offset = fixture.app.library_view.offset;
    let rebuilds = fixture.app.library_view.stats.rebuilds;
    wheel_frame(&ctx, &mut fixture.app, 1.2, vec![]);
    assert!((fixture.app.library_view.offset - offset).abs() < 0.1);
    assert_eq!(fixture.app.library_view.stats.rebuilds, rebuilds);
    assert_eq!(fixture.app.library_view.stats.formatted, 0);
}

#[test]
fn native_indexed_metadata_updates_preserve_selection_and_scrolling_without_full_refilter() {
    let mut f=Fixture::new(48);let mut library=items(1000);
    let catalog=Arc::make_mut(&mut f.app.library_metadata.catalog);
    for (index,item) in library.iter_mut().enumerate() {
        item.source=LibSource::File(format!("/indexed-native/{index}.wav").into());
        catalog.upsert(item.source.clone(),item.fingerprint,item.stored_metadata()).unwrap();
        catalog.tracks[index].annotations.rating=if index%2==0 {5} else {1};
    }
    f.app.library=Arc::new(library);f.app.library_metadata.bind_test_rows(&f.app.library);
    f.app.lib_filter="rating:5".into();let ctx=egui::Context::default();frame(&ctx,&mut f.app,0.0,vec![]);
    f.app.lib_sel=200;f.app.refresh_library_view();let selected=selected_source(&mut f.app);
    f.app.library_view.pending_offset=Some(100.0*f.app.library_view.stride+3.0);frame(&ctx,&mut f.app,0.05,vec![]);let offset=f.app.library_view.offset;
    let stats=f.app.library_view.stats;let old_rows=f.app.library.clone();let old_catalog=f.app.library_metadata.catalog.clone();
    let catalog=Arc::make_mut(&mut f.app.library_metadata.catalog);catalog.tracks[20].annotations.rating=1;catalog.tracks[21].annotations.rating=5;
    f.app.library_metadata.bind_incremental_test_rows(&f.app.library);
    frame(&ctx,&mut f.app,0.1,vec![]);
    assert_eq!(selected_source(&mut f.app),selected);assert!((f.app.library_view.offset-offset).abs()<0.01);
    assert_eq!(f.app.library_view.stats.rebuilds,stats.rebuilds);assert_eq!(f.app.library_view.stats.updates,stats.updates+1);assert_eq!(f.app.library_view.stats.examined,stats.examined+2);
    let expected:Vec<_>=(0..1000).filter(|i|*i!=20&&(*i%2==0||*i==21)).collect();assert_eq!(*f.app.library_view.indices,expected);
    assert!(!f.app.library_metadata.collection_rows().is_for(&old_rows,&old_catalog));
    let mut reversed=(*f.app.library).clone();reversed.reverse();f.app.library=Arc::new(reversed);f.app.library_metadata.bind_incremental_test_rows(&f.app.library);
    frame(&ctx,&mut f.app,0.2,vec![]);assert_eq!(selected_source(&mut f.app),selected);
    assert_eq!(f.app.library_view.stats.rebuilds,stats.rebuilds+1);assert_eq!(f.app.library_view.indices.len(),500);
    f.app.sort_library_column(crate::preferences::library_layout::Column::Title,false);frame(&ctx,&mut f.app,0.3,vec![]);let stats=f.app.library_view.stats;let changed=*f.app.library_view.indices.last().unwrap();let _old=f.app.library.clone();Arc::make_mut(&mut f.app.library)[changed].title="AAA first".into();f.app.library_metadata.bind_incremental_test_rows(&f.app.library);frame(&ctx,&mut f.app,0.4,vec![]);assert_eq!(f.app.library_view.indices[0],changed);assert_eq!(selected_source(&mut f.app),selected);assert_eq!(f.app.library_view.stats.rebuilds,stats.rebuilds);assert_eq!(f.app.library_view.stats.updates,stats.updates+1);assert_eq!(f.app.library_view.stats.examined,stats.examined+1);
    let played_a=f.app.library_view.indices[10];let played_b=f.app.library_view.indices[11];let _old=f.app.library.clone();let rows=Arc::make_mut(&mut f.app.library);
    rows[played_a].last_play=Some(SystemTime::UNIX_EPOCH+std::time::Duration::from_secs(10));rows[played_b].last_play=Some(SystemTime::UNIX_EPOCH+std::time::Duration::from_secs(20));
    f.app.library_metadata.bind_incremental_test_rows(&f.app.library);f.app.sort_library_column(crate::preferences::library_layout::Column::Played,false);frame(&ctx,&mut f.app,0.5,vec![]);assert_eq!(f.app.library_view.indices[0],played_a);let stats=f.app.library_view.stats;
    let _old=f.app.library.clone();Arc::make_mut(&mut f.app.library)[played_b].last_play=Some(SystemTime::UNIX_EPOCH+std::time::Duration::from_secs(5));f.app.library_metadata.bind_incremental_test_rows(&f.app.library);frame(&ctx,&mut f.app,0.6,vec![]);
    assert_eq!(f.app.library_view.indices[0],played_b);assert_eq!(selected_source(&mut f.app),selected);assert_eq!(f.app.library_view.stats.rebuilds,stats.rebuilds);assert_eq!(f.app.library_view.stats.updates,stats.updates+1);assert_eq!(f.app.library_view.stats.examined,stats.examined+1);
}

#[test]
fn native_indexed_10000_and_100000_track_search_scroll_and_updates_meet_budgets_during_a_mix() {
    use std::sync::atomic::{AtomicBool,Ordering};
    let stop=Arc::new(AtomicBool::new(false));let stopping=stop.clone();
    struct Stop(Arc<AtomicBool>);impl Drop for Stop {fn drop(&mut self){self.0.store(true,Ordering::Release);}}
    let _stop=Stop(stop.clone());
    let mix=std::thread::spawn(move||{
        let (engine,mut rt)=crate::engine::Engine::headless_for_test(48000,256);
        for deck in 0..2 {engine.send(Command::DeckPlay {deck}).unwrap();engine.send(Command::DeckLoop {deck,beats:4.0}).unwrap();}
        let mut out=[0.0;512];let mut callbacks=0_u64;let mut energy=0.0_f64;
        while !stopping.load(Ordering::Acquire)||callbacks<56250 {
            assert_eq!(crate::engine::test_alloc::measure(||rt.process(&mut out)),Default::default());
            assert!(out.iter().all(|s|s.is_finite()));assert!(rt.decks.iter().all(|deck|deck.playing));energy+=out.iter().map(|s|f64::from(*s).powi(2)).sum::<f64>();callbacks+=1;
        }
        (callbacks,energy)
    });
    for count in [10000,100000] {
        let mut f=Fixture::new(48);let mut library=items(count);let catalog=Arc::make_mut(&mut f.app.library_metadata.catalog);
        for (index,item) in library.iter_mut().enumerate(){item.source=LibSource::File(format!("/indexed-bench/{index}.wav").into());catalog.upsert(item.source.clone(),item.fingerprint,item.stored_metadata()).unwrap();catalog.tracks[index].annotations.rating=(index%6) as u8;}
        f.app.library=Arc::new(library);let build=Instant::now();f.app.library_metadata.bind_test_rows(&f.app.library);let build_ns=build.elapsed().as_nanos();
        let index_bytes=f.app.library_metadata.collection_rows().search_bytes();assert!(index_bytes<128*1024*1024);
        f.app.sort_library_column(crate::preferences::library_layout::Column::Bpm,false);f.app.sort_library_column(crate::preferences::library_layout::Column::Title,true);let ctx=egui::Context::default();wheel_frame(&ctx,&mut f.app,0.0,vec![]);let mut queries=Vec::new();let mut scroll=Vec::new();let mut updates=Vec::new();
        for number in 1..=120 {
            let query=if number%3==0 {"artist:\"Even Artist\" rating>=4"} else if number%3==1 {"title:Track bpm>=105"} else {"key:C length>=150"};
            f.app.lib_filter=query.into();let started=Instant::now();wheel_frame(&ctx,&mut f.app,number as f64,vec![]);queries.push(started.elapsed().as_nanos());
            let visible=f.app.library_view.indices.len();assert!(visible>0);
            f.app.library_view.pending_offset=Some((visible/4) as f32*f.app.library_view.stride);let output=wheel_frame(&ctx,&mut f.app,number as f64+0.01,vec![]);let first=*f.app.library_view.cells.keys().min().unwrap();let pointer=label_center(&output,&f.app.library[f.app.library_view.indices[first]].title);let offset=f.app.library_view.offset;let selected=selected_source(&mut f.app);
            for step in 0..4 {let started=Instant::now();wheel_frame(&ctx,&mut f.app,number as f64+0.1+step as f64/10.0,vec![egui::Event::PointerMoved(pointer),egui::Event::MouseWheel {unit:egui::MouseWheelUnit::Point,delta:Vec2::new(0.0,-22.0),modifiers:egui::Modifiers::NONE}]);scroll.push(started.elapsed().as_nanos());assert!(f.app.library_view.stats.rendered<=6);}
            assert!(f.app.library_view.offset>offset);assert_eq!(selected_source(&mut f.app),selected);
        }
        f.app.lib_filter="rating:5".into();wheel_frame(&ctx,&mut f.app,200.0,vec![]);f.app.lib_sel=10;f.app.refresh_library_view();let selected=selected_source(&mut f.app);
        for number in 0..32 {let _old=f.app.library_metadata.catalog.clone();Arc::make_mut(&mut f.app.library_metadata.catalog).tracks[number].annotations.rating=5;f.app.library_metadata.bind_incremental_test_rows(&f.app.library);let started=Instant::now();wheel_frame(&ctx,&mut f.app,201.0+number as f64,vec![]);updates.push(started.elapsed().as_nanos());assert_eq!(selected_source(&mut f.app),selected);}
        let distribution=|mut times:Vec<u128>|{times.sort_unstable();serde_json::json!({"samples":times.len(),"median_ns":times[times.len()/2],"p95_ns":times[times.len()*95/100],"max_ns":times[times.len()-1]})};
        let query=distribution(queries);let scroll=distribution(scroll);let update=distribution(updates);
        assert!(query["p95_ns"].as_u64().unwrap()<100_000_000,"query {count}: {query}");assert!(scroll["p95_ns"].as_u64().unwrap()<16_700_000,"scroll {count}: {scroll}");assert!(update["p95_ns"].as_u64().unwrap()<16_700_000,"update {count}: {update}");
        println!("LIBRARY_SCALE_RECEIPT {}",serde_json::json!({"tracks":count,"index_build_ns":build_ns,"retained_index_bytes":index_bytes,"kernel":std::fs::read_to_string("/proc/sys/kernel/osrelease").unwrap().trim(),"architecture":std::env::consts::ARCH,"logical_cpus":std::thread::available_parallelism().unwrap().get(),"process_memory":std::fs::read_to_string("/proc/self/status").unwrap().lines().filter(|line|line.starts_with("VmRSS:")||line.starts_with("VmHWM:")).collect::<Vec<_>>(),"query":query,"scroll":scroll,"incremental_update":update,"physical_devices_opened":false}));
    }
    stop.store(true,Ordering::Release);let (callbacks,energy)=mix.join().unwrap();assert!(energy>1.0);println!("LIBRARY_SCALE_MIX_RECEIPT {}",serde_json::json!({"output_rate":48000,"rendered_frames":callbacks*256,"minimum_seconds":300,"energy":energy,"callback_allocations":0,"physical_devices_opened":false}));
}
