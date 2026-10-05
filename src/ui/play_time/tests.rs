use super::*;
use crate::engine::media_source::FileFingerprint;
use crate::ui::{
    play_history::Identity,
    test_support::{label_center, Fixture},
    *,
};

fn clock() -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(1_704_715_200)
} // 2024-01-08 12:00:00 UTC

#[test]
fn fixed_clock_formats_now_minutes_hours_days_and_unambiguous_older_dates() {
    let now = clock();
    for (age, expected, until_change) in [
        (0, "Just now", Some(60)),
        (59, "Just now", Some(1)),
        (60, "1 min ago", Some(60)),
        (3599, "59 min ago", Some(1)),
        (3600, "1 h ago", Some(3600)),
        (86399, "23 h ago", Some(1)),
        (86400, "1 d ago", Some(86400)),
        (6 * 86400, "6 d ago", Some(86400)),
        (7 * 86400, "2024-01-01 UTC", None),
    ] {
        let result = format(Some(now - Duration::from_secs(age)), now);
        assert_eq!(result.label, expected, "age={age}");
        assert_eq!(
            result.next_change,
            until_change.map(|seconds| now + Duration::from_secs(seconds))
        );
        assert!(result.tooltip.starts_with("Played: 2024-01-"));
        assert!(result.tooltip.ends_with(".000000000 UTC"));
    }
    assert_eq!(
        format(Some(now), now).tooltip,
        "Played: 2024-01-08 12:00:00.000000000 UTC"
    );
    let played = now - Duration::from_millis(59_500);
    let result = format(Some(played), now);
    assert_eq!(result.label, "Just now");
    assert_eq!(result.next_change, Some(now + Duration::from_millis(500)));
    assert_eq!(result.tooltip, "Played: 2024-01-08 11:59:00.500000000 UTC");
}

#[test]
fn unknown_future_and_pre_epoch_inputs_are_safe_and_keep_precise_utc_identity() {
    let now = clock();
    let unknown = format(None, now);
    assert_eq!(unknown.label, "—");
    assert_eq!(unknown.tooltip, "No recorded playback time");
    assert!(unknown.next_change.is_none());
    let future = now + Duration::from_secs(5);
    let result = format(Some(future), now);
    assert_eq!(result.label, "Future time");
    assert_eq!(result.next_change, Some(future));
    assert!(result.tooltip.contains("2024-01-08 12:00:05.000000000 UTC"));
    assert!(result.tooltip.contains("Future timestamp"));
    assert_eq!(format(Some(future), future).label, "Just now");
    let before_epoch = UNIX_EPOCH - Duration::from_nanos(1);
    let result = format(Some(before_epoch), now);
    assert_eq!(result.label, "1969-12-31 UTC");
    assert_eq!(result.tooltip, "Played: 1969-12-31 23:59:59.999999999 UTC");
    let leap_day = UNIX_EPOCH - Duration::from_secs(672 * 86400);
    assert_eq!(
        format(Some(leap_day), now).tooltip,
        "Played: 1968-02-29 00:00:00.000000000 UTC"
    );
    // The reference clock can itself precede the epoch without unsigned wrap.
    assert_eq!(
        format(Some(before_epoch), before_epoch + Duration::from_secs(125)).label,
        "2 min ago"
    );
    for (time, expected) in [
        (
            UNIX_EPOCH.checked_add(Duration::from_secs(i64::MAX as u64)),
            "Future time",
        ),
        (
            UNIX_EPOCH.checked_sub(Duration::from_secs(i64::MAX as u64)),
            "Long ago",
        ),
    ] {
        let time = time.expect("Linux SystemTime must represent the time_t boundary fixture");
        let result = format(Some(time), now);
        assert_eq!(result.label, expected);
        assert!(result.tooltip.contains("Unix epoch offset"));
        assert!(result.tooltip.contains("outside calendar range"));
    }
}

fn items(count: usize, first_play: Option<SystemTime>) -> Vec<LibItem> {
    let fingerprint = FileFingerprint::read(std::path::Path::new("Cargo.toml"));
    (0..count)
        .map(|index| LibItem {
            source: LibSource::File(format!("/synthetic/history/{index}.wav").into()),
            title: format!("History {index:05}"),
            artist: "fixture".into(),
            bpm: Bpm::hint(120.0),
            fingerprint,
            key: "C".into(),
            length: Some(120.0),
            last_play: if index == 0 { first_play } else { None },
        })
        .collect()
}
struct Gui {
    fixture: Fixture,
    ctx: egui::Context,
    frame: u64,
}
impl Gui {
    fn new(count: usize, played: Option<SystemTime>) -> Self {
        let mut fixture = Fixture::new(48);
        fixture.app.library = Arc::new(items(count, played));
        let ctx = egui::Context::default();
        ctx.style_mut(|style| {
            style.interaction.tooltip_delay = 0.0;
            style.interaction.show_tooltips_only_when_still = false;
        });
        Self {
            fixture,
            ctx,
            frame: 0,
        }
    }
    fn draw(&mut self, now: SystemTime, events: Vec<egui::Event>) -> egui::FullOutput {
        self.frame += 1;
        let theme = self.fixture.app.theme.clone();
        self.ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1440.0, 160.0))),
                time: Some(self.frame as f64 / 60.0),
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default()
                    .show(ctx, |ui| self.fixture.app.crate_row_at(ui, &theme, now));
            },
        )
    }
    fn tooltip(&mut self, now: SystemTime, row: usize) -> egui::FullOutput {
        let output = self.draw(now, vec![]);
        let point = label_center(&output, &format!("History {row:05}"));
        self.draw(now, vec![egui::Event::PointerMoved(point)]);
        self.draw(now, vec![]);
        self.draw(now, vec![])
    }
}
fn visible(output: &egui::FullOutput, expected: &str) -> bool {
    output.shapes.iter().any(|shape| {
        matches!(&shape.shape,
        egui::epaint::Shape::Text(text) if text.galley.text().contains(expected))
    })
}

#[test]
fn real_crate_tooltip_exposes_precise_time_future_annotation_and_unknown_state() {
    let now = clock();
    for (played, label, exact) in [
        (
            now - Duration::from_secs(125),
            "2 min ago",
            "2024-01-08 11:57:55.000000000 UTC",
        ),
        (
            now + Duration::from_secs(5),
            "Future time",
            "2024-01-08 12:00:05.000000000 UTC",
        ),
        (
            UNIX_EPOCH - Duration::from_nanos(1),
            "1969-12-31 UTC",
            "1969-12-31 23:59:59.999999999 UTC",
        ),
    ] {
        let mut gui = Gui::new(20, Some(played));
        let output = gui.draw(now, vec![]);
        assert!(visible(&output, label));
        let output = gui.tooltip(now, 0);
        assert!(visible(&output, exact), "precise tooltip missing {exact}");
        assert!(visible(&output, "Crate row / selection"));
        if played > now {
            assert!(visible(&output, "Future timestamp"));
        }
        let output = gui.tooltip(now, 1);
        assert!(visible(&output, "No recorded playback time"));
        assert!(visible(&output, "Crate row / selection"));
    }
}

#[test]
fn visible_cache_ages_at_boundaries_without_reexamining_large_crate_or_reformatting_steady_rows() {
    let now = clock();
    for count in [100, 10_000, 50_000] {
        let mut gui = Gui::new(count, Some(now - Duration::from_secs(59)));
        gui.draw(now, vec![]);
        let baseline = gui.fixture.app.library_view.stats;
        for frame in 0..100 {
            gui.draw(now + Duration::from_millis(frame * 9), vec![]);
            let stats = gui.fixture.app.library_view.stats;
            assert_eq!(stats.rebuilds, baseline.rebuilds);
            assert_eq!(stats.examined, baseline.examined);
            assert_eq!(stats.formatted, 0);
            assert!(stats.rendered > 0 && stats.rendered <= 6);
        }
        let output = gui.draw(now + Duration::from_secs(1), vec![]);
        assert!(visible(&output, "1 min ago"));
        assert_eq!(gui.fixture.app.library_view.stats.formatted, 1);
        assert_eq!(
            gui.fixture.app.library_view.stats.examined,
            baseline.examined
        );
        assert_eq!(
            gui.fixture.app.library_view.stats.rebuilds,
            baseline.rebuilds
        );
        gui.draw(now + Duration::from_secs(1), vec![]);
        assert_eq!(gui.fixture.app.library_view.stats.formatted, 0);
        let output = gui.draw(now + Duration::from_secs(61), vec![]);
        assert!(visible(&output, "2 min ago"));
        assert_eq!(gui.fixture.app.library_view.stats.formatted, 1);
        // Backwards civil-clock movement must invalidate the relative label,
        // while unknown rows need no formatting at all.
        let output = gui.draw(now - Duration::from_secs(1), vec![]);
        assert!(visible(&output, "Just now"));
        assert_eq!(gui.fixture.app.library_view.stats.formatted, 1);
        assert_eq!(
            gui.fixture.app.library_view.stats.examined,
            baseline.examined
        );
        assert_eq!(
            gui.fixture.app.library_view.stats.rebuilds,
            baseline.rebuilds
        );
    }
}

#[test]
fn future_expiry_visible_history_updates_and_hidden_history_preserve_viewport_bound() {
    let now = clock();
    let mut gui = Gui::new(50_000, Some(now + Duration::from_secs(2)));
    gui.draw(now, vec![]);
    let baseline = gui.fixture.app.library_view.stats;
    let output = gui.draw(now + Duration::from_secs(2), vec![]);
    assert!(visible(&output, "Just now"));
    assert_eq!(gui.fixture.app.library_view.stats.formatted, 1);
    assert!(!gui.fixture.app.library_view.cells[&0]
        .played_tooltip
        .contains("Future timestamp"));
    let item = &gui.fixture.app.library[0];
    let identity = Identity::new(item.source.clone(), item.fingerprint).unwrap();
    gui.fixture
        .app
        .last_played
        .record(&identity, now + Duration::from_secs(10));
    let output = gui.draw(now + Duration::from_secs(15), vec![]);
    assert!(visible(&output, "Just now"));
    assert_eq!(gui.fixture.app.library_view.stats.formatted, 1);
    assert!(gui.fixture.app.library_view.cells[&0]
        .played_tooltip
        .contains("12:00:10.000000000 UTC"));
    let hidden = &gui.fixture.app.library[20_000];
    let identity = Identity::new(hidden.source.clone(), hidden.fingerprint).unwrap();
    gui.fixture
        .app
        .last_played
        .record(&identity, now + Duration::from_secs(12));
    gui.draw(now + Duration::from_secs(15), vec![]);
    assert_eq!(gui.fixture.app.library_view.stats.formatted, 0);
    assert_eq!(
        gui.fixture.app.library_view.stats.examined,
        baseline.examined
    );
    assert_eq!(
        gui.fixture.app.library_view.stats.rebuilds,
        baseline.rebuilds
    );
    assert!(gui.fixture.app.library_view.cells.len() <= 6);
    assert!(!gui.fixture.app.library_view.cells.contains_key(&20_000));
}

#[test]
fn extreme_future_ui_deadline_is_bounded_before_native_instant_conversion() {
    let distant = UNIX_EPOCH
        .checked_add(Duration::from_secs(i64::MAX as u64))
        .unwrap();
    let mut gui = Gui::new(20, Some(distant));
    let requests = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let observed = requests.clone();
    gui.ctx.set_request_repaint_callback(move |info| {
        // Match eframe's native deadline addition; this must never receive the
        // unbounded civil-time delta of a malformed/future history timestamp.
        assert!(info.delay <= Duration::from_secs(86400));
        let _ = Instant::now() + info.delay;
        observed.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    });
    for _ in 0..8 {
        gui.draw(clock(), vec![]);
    }
    assert!(requests.load(std::sync::atomic::Ordering::Relaxed) > 0);
    assert_eq!(gui.fixture.app.library_view.cells[&0].played, "Future time");
    assert!(gui.fixture.app.library_view.cells[&0]
        .played_tooltip
        .contains("outside calendar range"));
}

#[test]
fn visible_last_play_cache_reformats_once_when_language_changes_without_changing_time() {
    let fixture=crate::ui::test_support::Fixture::new(144);
    let now=clock();let played=Some(now-Duration::from_secs(7*86400));
    let mut cells=crate::ui::library_view::Cells::new(&fixture.app.library[0],played,now);
    assert_eq!(cells.played,"2024-01-01 UTC");
    let bpm=cells.bpm.clone();
    { let _locale=crate::localization::scope(crate::localization::Locale::Spanish);
      assert!(cells.refresh_play(played,now));assert_eq!(cells.played,"01/01/2024 UTC");
      assert!(cells.played_tooltip.starts_with("Reproducido: "));assert!(!cells.refresh_play(played,now)); }
    assert!(cells.refresh_play(played,now));assert_eq!(cells.played,"2024-01-01 UTC");
    assert_eq!(cells.played_at,played);assert_eq!(cells.bpm,bpm);
}
