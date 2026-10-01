use super::{test_support::*, *};
use std::time::Instant;

#[test]
fn font_and_shell_only_edits_reload_with_unchanged_colors_and_retain_invalid_interim_sources() {
    let fixture = Fixture::new();
    let colors = fs::read(&fixture.theme.path).unwrap();
    let mut reader = Reader::new(fixture.theme.clone(), fixture.resolver());
    let first = reader.read().unwrap();
    assert_eq!(first.theme.font, "Hack");
    assert_eq!(first.theme.path, fixture.theme.path);
    assert!(reader.read().is_none());
    fixture.shell("[font]\nbase-size = 18\n");
    let sized = reader.read().unwrap();
    assert_eq!(sized.theme.font_size, 18.0);
    assert!(Arc::ptr_eq(&first.fonts, &sized.fonts));
    fixture.select("Ubuntu", "ubuntu.ttf");
    let selected = reader.read().unwrap();
    assert_eq!(selected.theme.font, "Ubuntu");
    assert!(!Arc::ptr_eq(&sized.fonts, &selected.fonts));
    assert_eq!(fs::read(&fixture.theme.path).unwrap(), colors);
    for family in [FontFamily::Proportional, FontFamily::Monospace] {
        assert_eq!(selected.fonts.families[&family][0], SELECTED);
    }
    fixture.colors("background = '");
    fixture.shell("[font\nbase-size = ");
    fs::write(fixture.root.join("fonts/ubuntu.ttf"), b"partial-font").unwrap();
    assert!(reader.read().is_none());
    assert_eq!(reader.current.theme, selected.theme);
    assert!(Arc::ptr_eq(&reader.current.fonts, &selected.fonts));
    assert_eq!(reader.diagnostics.len(), 3);
    fixture.colors("background = '#abcdef'\n");
    fixture.shell("[font]\nbase-size = 16.5\n");
    fs::write(
        fixture.root.join("fonts/ubuntu.ttf"),
        &first.fonts.font_data[SELECTED].font,
    )
    .unwrap();
    let restored = reader.read().unwrap();
    assert_eq!(restored.theme.bg, egui::Color32::from_rgb(0xab, 0xcd, 0xef));
    assert_eq!(restored.theme.font_size, 16.5);
    assert!(
        !Arc::ptr_eq(&restored.fonts, &selected.fonts),
        "same selected pathname must reload changed font bytes"
    );
    assert!(reader.diagnostics.is_empty());
    for value in ["nan", "inf", "-1", "0", "97", "'large'"] {
        fixture.shell(&format!("[font]\nbase-size = {value}\n"));
        assert!(reader.read().is_none());
        assert_eq!(reader.current.theme.font_size, 16.5);
    }
}

#[test]
fn real_fontconfig_private_selection_changes_and_invalid_config_preserve_last_good_font() {
    let fixture = Fixture::new();
    let resolver = fixture.fontconfig("Hack");
    let mut reader = Reader::new(fixture.theme.clone(), resolver);
    let first = reader.read().unwrap();
    assert_eq!(first.theme.font, "Hack", "private real fc-match selection");
    fixture.write_fontconfig("Ubuntu");
    let selected = reader.read().unwrap();
    assert_eq!(selected.theme.font, "Ubuntu");
    assert!(!Arc::ptr_eq(&first.fonts, &selected.fonts));
    fs::write(fixture.root.join("fonts.conf"), "<fontconfig><match").unwrap();
    assert!(reader.read().is_none());
    assert_eq!(reader.current.theme.font, "Ubuntu");
    fixture.write_fontconfig("Hack");
    assert_eq!(reader.read().unwrap().theme.font, "Hack");
    assert_eq!(reader.current.theme.path, fixture.theme.path);
}

#[test]
fn bounded_request_and_latest_result_slots_coalesce_without_losing_font_state() {
    let fixture = Fixture::new();
    let (loader, control) = held(&fixture);
    control
        .started
        .recv_timeout(Duration::from_secs(2))
        .unwrap();
    for _ in 0..1000 {
        loader.request();
    }
    assert_eq!(loader.requests.as_ref().unwrap().len(), 1);
    control.release.send(()).unwrap();
    control
        .started
        .recv_timeout(Duration::from_secs(2))
        .unwrap();
    // Worker sampled colors/size before resolving the held font. Queue one
    // final pass, while the first complete result is deliberately unconsumed.
    fixture.select("Ubuntu", "ubuntu.ttf");
    fixture.shell("[font]\nbase-size = 20\n");
    fixture.colors("background = '#090807'\n");
    loader.request();
    control.release.send(()).unwrap();
    control
        .started
        .recv_timeout(Duration::from_secs(2))
        .unwrap();
    control.release.send(()).unwrap();
    let mut latest = None;
    until(|| {
        if let Some(update) = loader.poll() {
            latest = Some(update);
        }
        latest
            .as_ref()
            .is_some_and(|update| update.theme.font_size == 20.0)
    });
    let latest = latest.unwrap();
    assert_eq!(latest.theme.font, "Ubuntu");
    assert_eq!(latest.fonts.families[&FontFamily::Monospace][0], SELECTED);
    assert_eq!(latest.theme.bg, egui::Color32::from_rgb(9, 8, 7));
    assert!(loader.poll().is_none());
    assert!(loader.requests.as_ref().unwrap().is_empty());
}

#[test]
fn invalid_missing_and_oversized_files_never_replace_a_valid_font_or_size() {
    let fixture = Fixture::new();
    let mut reader = Reader::new(fixture.theme.clone(), fixture.resolver());
    let first = reader.read().unwrap();
    fixture.select("missing", "absent.ttf");
    assert!(reader.read().is_none());
    let large = File::create(fixture.root.join("fonts/large.ttf")).unwrap();
    large.set_len(FONT_LIMIT as u64 + 1).unwrap();
    fixture.select("large", "large.ttf");
    assert!(reader.read().is_none());
    fs::write(fixture.root.join("shell.toml"), vec![b' '; TEXT_LIMIT + 1]).unwrap();
    assert!(reader.read().is_none());
    assert!(Arc::ptr_eq(&first.fonts, &reader.current.fonts));
    fixture.select("Hack", "hack.ttf");
    fixture.shell("[font]\nbase-size = 14\n");
    assert_eq!(reader.read().unwrap().theme.font_size, 14.0);
    let (loader, control) = held(&fixture);
    control
        .started
        .recv_timeout(Duration::from_secs(2))
        .unwrap();
    let start = Instant::now();
    drop(loader);
    assert!(start.elapsed() < Duration::from_millis(100));
    control.release.send(()).unwrap();
}

#[test]
fn forced_reload_is_atomic_revalidates_unchanged_font_and_recovers_only_complete_bundles() {
    let fixture = Fixture::new();
    let mut reader = Reader::new(fixture.theme.clone(), fixture.resolver());
    let first = reader.read().unwrap();
    let forced = reader.force().unwrap();
    assert_eq!(first.theme, forced.theme);
    assert!(
        !Arc::ptr_eq(&first.fonts, &forced.fonts),
        "force must re-read cached bytes"
    );
    fixture.colors("background = '#aabbcc'\n");
    fixture.shell("[font]\nbase-size = 'broken'\n");
    fixture.select("Ubuntu", "ubuntu.ttf");
    assert!(reader.force().err().unwrap().contains("font.base-size"));
    assert_eq!(reader.current.theme, forced.theme);
    assert!(Arc::ptr_eq(&reader.current.fonts, &forced.fonts));
    assert!(
        reader.read().is_none(),
        "watcher cannot leak the valid subset after failure"
    );
    fixture.shell("[font]\nbase-size = 18\n");
    let recovered = reader.read().unwrap();
    assert_eq!(recovered.theme.font_size, 18.0);
    assert_eq!(recovered.theme.font, "Ubuntu");
    assert_eq!(
        recovered.theme.bg,
        egui::Color32::from_rgb(0xaa, 0xbb, 0xcc)
    );
    fixture.colors("background = 'broken'\n");
    assert!(reader.force().is_err());
    fixture.colors("background = '#aabbcc'\n");
    fs::write(fixture.root.join("fonts/ubuntu.ttf"), "broken").unwrap();
    assert!(reader.force().is_err());
    assert_eq!(reader.current.theme, recovered.theme);
    assert!(Arc::ptr_eq(&reader.current.fonts, &recovered.fonts));
}

#[test]
fn automatic_watch_can_publish_valid_resources_after_the_forced_ticket_is_abandoned() {
    let fixture = Fixture::new();
    let mut reader = Reader::new(fixture.theme.clone(), fixture.resolver());
    reader.read().unwrap();
    fixture.select("Ubuntu", "ubuntu.ttf");
    let ignored_ticket = reader.force().unwrap();
    let automatic = reader
        .read()
        .expect("an expired GUI ticket cannot suppress the watcher forever");
    assert_eq!(automatic.theme.font, "Ubuntu");
    assert!(Arc::ptr_eq(&automatic.fonts, &ignored_ticket.fonts));
    assert!(
        reader.read().is_none(),
        "one republication, not an ongoing font/style loop"
    );
}

#[test]
fn performance_protection_defers_first_font_read_and_cancels_held_results() {
    let fixture = Fixture::new();
    let performance = Handle::default();
    performance.set_enabled(true).unwrap();
    let (loader, control) = held_with_performance(&fixture, performance.clone());
    assert!(control
        .started
        .recv_timeout(Duration::from_millis(60))
        .is_err());
    assert!(loader.poll_candidate().is_none());
    assert_eq!(loader.force(1).unwrap_err().code, "performance_protected");
    performance.set_enabled(false).unwrap();
    loader.request();
    control
        .started
        .recv_timeout(Duration::from_secs(2))
        .unwrap();
    performance.set_enabled(true).unwrap();
    control.release.send(()).unwrap();
    until(|| performance.status().optional_active == 0);
    assert!(loader.poll_candidate().is_none());
    performance.set_enabled(false).unwrap();
    loader.request();
    control
        .started
        .recv_timeout(Duration::from_secs(2))
        .unwrap();
    control.release.send(()).unwrap();
    let mut installed = None;
    until(|| {
        installed = loader.poll_candidate();
        installed.is_some()
    });
    let (update, guard) = installed.unwrap().claim().unwrap();
    assert_eq!(update.theme.font, "Hack");
    assert_eq!(
        performance.set_enabled(true),
        Err(crate::engine::performance::Error::Changing)
    );
    drop(guard);
    performance.set_enabled(true).unwrap();
}

#[test]
fn performance_rejected_queued_theme_reappears_after_protection_leaves_between_watcher_ticks() {
    let fixture = Fixture::new();
    let performance = Handle::default();
    let loader = fixture.quiet_loader_with_performance(performance.clone());
    until(|| !loader.results.is_empty());
    performance.set_enabled(true).unwrap();
    performance.set_enabled(false).unwrap();
    let candidate = loader.poll_candidate().unwrap();
    assert!(candidate.claim().is_err());
    loader.request();
    let mut candidate = None;
    until(|| {
        candidate = loader.poll_candidate();
        candidate.is_some()
    });
    assert_eq!(candidate.unwrap().claim().unwrap().0.theme.font, "Hack");
}
