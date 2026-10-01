use super::{
    test_support::Fixture,
    theme_reload_tests::{frame, installed},
    *,
};
use crate::theme::reload::test_support::{self as sources, Fixture as Sources};
use ab_glyph::{Font, FontRef};
use egui::{FontDefinitions, FontFamily};

fn assert_fallbacks(ctx: &egui::Context) {
    let chosen = installed(ctx);
    let face = FontRef::try_from_slice_and_index(&chosen.font, chosen.index).unwrap();
    let defaults = FontDefinitions::default();
    let mut actual_fallbacks = 0;
    for family in [FontFamily::Proportional, FontFamily::Monospace] {
        let font = FontId::new(16.0, family.clone());
        ctx.fonts(|fonts| {
            let held = fonts.lock();
            let definitions = held.fonts.definitions();
            // Every bundled fallback remains present after the selected font.
            assert_eq!(
                &definitions.families[&family][1..=defaults.families[&family].len()],
                defaults.families[&family]
            );
            for fallback in defaults.families.values().flatten() {
                assert!(definitions.families[&family][1..].contains(fallback));
            }
        });
        for symbol in ['↻', '⇄', '→', '↓', '↑', '½', '×', '…', '—', '🎵'] {
            assert!(
                ctx.fonts(|fonts| fonts.has_glyph(&font, symbol)),
                "{family:?} missing {symbol}"
            );
            if face.glyph_id(symbol).0 == 0 {
                actual_fallbacks += 1;
                let mut selected_only = defaults.clone();
                selected_only
                    .font_data
                    .insert("chosen".into(), chosen.clone());
                selected_only
                    .families
                    .insert(family.clone(), vec!["chosen".into()]);
                let isolated = egui::Context::default();
                isolated.set_fonts(selected_only);
                let _ = isolated.run(Default::default(), |_| {});
                assert!(
                    !isolated.fonts(|fonts| fonts.has_glyph(&font, symbol)),
                    "fixture symbol must require a fallback font"
                );
            }
        }
    }
    assert!(
        actual_fallbacks > 0,
        "fixture must exercise real glyph fallback"
    );
}

#[test]
fn two_private_installed_fontconfig_fonts_reach_both_actual_gui_text_families() {
    let source = Sources::new();
    let mut fixture = Fixture::new(64);
    fixture.app.theme_reload = Some(source.installed_loader("Hack"));
    let ctx = egui::Context::default();
    let mut time = 0.0;
    let mut widths = Vec::new();
    for (family, file) in [("Hack", "hack.ttf"), ("Ubuntu", "ubuntu.ttf")] {
        source.write_fontconfig(family);
        sources::until(|| {
            frame(&ctx, &mut fixture, &mut time);
            fixture.app.theme.font == family
        });
        frame(&ctx, &mut fixture, &mut time);
        assert_eq!(
            installed(&ctx).font.as_ref(),
            std::fs::read(source.root.join("fonts").join(file)).unwrap()
        );
        let pair: Vec<_> = [FontFamily::Proportional, FontFamily::Monospace]
            .into_iter()
            .map(|family| {
                ctx.fonts(|fonts| {
                    [
                        fonts.glyph_width(&FontId::new(16.0, family.clone()), 'i'),
                        fonts.glyph_width(&FontId::new(16.0, family), 'W'),
                    ]
                })
            })
            .collect();
        assert_eq!(
            pair[0], pair[1],
            "both text families must prioritize the chosen face"
        );
        widths.push(pair[0]);
        assert_fallbacks(&ctx);
    }
    assert_eq!(
        widths[0][0], widths[0][1],
        "Hack is the controlled monospace fixture"
    );
    assert_ne!(
        widths[1][0], widths[1][1],
        "Ubuntu is the controlled proportional fixture"
    );
    assert_ne!(widths[0], widths[1]);
}

#[test]
fn unavailable_font_starts_with_bundled_defaults_then_keeps_last_loaded_font_and_recovers() {
    let source = Sources::new();
    source.select("unavailable fixture", "missing.ttf");
    let mut fixture = Fixture::new(64);
    fixture.app.theme_reload = Some(source.loader());
    let ctx = egui::Context::default();
    let mut time = 0.0;
    sources::until(|| {
        frame(&ctx, &mut fixture, &mut time);
        fixture.app.theme_fonts.is_some()
    });
    frame(&ctx, &mut fixture, &mut time);
    assert_eq!(fixture.app.theme.font, "bundled default");
    let defaults = FontDefinitions::default();
    ctx.fonts(|fonts| {
        let held = fonts.lock();
        assert!(!held
            .fonts
            .definitions()
            .font_data
            .contains_key("omatainer-selected"));
        for family in [FontFamily::Proportional, FontFamily::Monospace] {
            assert_eq!(
                &held.fonts.definitions().families[&family][..defaults.families[&family].len()],
                defaults.families[&family]
            );
        }
        for family in [FontFamily::Proportional, FontFamily::Monospace] {
            assert!(!held.fonts.definitions().families[&family].is_empty());
        }
    });
    for family in [FontFamily::Proportional, FontFamily::Monospace] {
        assert!(ctx.fonts(|fonts| fonts.has_glyphs(&FontId::new(16.0, family), "↻⇄→↓↑½×…—")));
    }
    source.select("Ubuntu", "ubuntu.ttf");
    sources::until(|| {
        frame(&ctx, &mut fixture, &mut time);
        fixture.app.theme.font == "Ubuntu"
    });
    frame(&ctx, &mut fixture, &mut time);
    let loaded = installed(&ctx);
    source.select("unavailable fixture", "missing.ttf");
    source.shell("[font]\nbase-size = 17\n");
    sources::until(|| {
        frame(&ctx, &mut fixture, &mut time);
        fixture.app.theme.font_size == 17.0
    });
    frame(&ctx, &mut fixture, &mut time);
    assert_eq!(fixture.app.theme.font, "Ubuntu");
    assert!(Arc::ptr_eq(&loaded, &installed(&ctx)));
    assert_fallbacks(&ctx);
    source.select("Hack", "hack.ttf");
    sources::until(|| {
        frame(&ctx, &mut fixture, &mut time);
        fixture.app.theme.font == "Hack"
    });
    frame(&ctx, &mut fixture, &mut time);
    assert_ne!(loaded.font, installed(&ctx).font);
}

#[test]
fn unavailable_fontconfig_family_uses_the_real_resolved_installed_substitute() {
    let source = Sources::new();
    let mut fixture = Fixture::new(64);
    fixture.app.theme_reload =
        Some(source.installed_loader("NotInstalled-Omatainer-Private-Fixture"));
    let ctx = egui::Context::default();
    let mut time = 0.0;
    sources::until(|| {
        frame(&ctx, &mut fixture, &mut time);
        fixture.app.theme.font != "bundled default"
    });
    frame(&ctx, &mut fixture, &mut time);
    let file = match fixture.app.theme.font.as_str() {
        "Hack" => "hack.ttf",
        "Ubuntu" => "ubuntu.ttf",
        other => panic!("font outside private configuration: {other}"),
    };
    assert_eq!(
        installed(&ctx).font.as_ref(),
        std::fs::read(source.root.join("fonts").join(file)).unwrap()
    );
    assert_fallbacks(&ctx);
}
