use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

#[test]
fn documented_hex_formats_preserve_every_channel_and_reject_extensions() {
    for value in 0..=255u8 {
        let text = format!("{value:02x}{:02X}{:02x}", 255 - value, value / 2);
        let expected = Some(rgb(value, 255 - value, value / 2));
        for input in [
            text.clone(),
            format!("#{text}"),
            format!(" \t#{text}\n"),
            format!("\u{2003}{text}\u{2003}"),
        ] {
            assert_eq!(parse_hex(&input), expected, "{input:?}");
        }
    }
    for invalid in [
        "",
        "#",
        "abc",
        "#abc",
        "12345",
        "1234567",
        "12345678",
        "#12345678",
        "##abcdef",
        "0x123456",
        "12345g",
        "ab cd ",
        "abc\0ef",
        "１２３",
        "€abc",
        "a€ab",
        "ab€a",
        "abc€",
        "💿ab",
    ] {
        assert_eq!(parse_hex(invalid), None, "{invalid:?}");
    }
}

#[test]
fn unicode_scalar_and_seeded_utf8_fuzz_never_accept_non_ascii_hex() {
    // Every non-ASCII scalar is inserted into a six-byte reproducer when it
    // fits. This exercises each potential UTF-8 boundary, not just one crash.
    for scalar in 128..=0x10ffff {
        let Some(ch) = char::from_u32(scalar) else {
            continue;
        };
        if ch.is_whitespace() {
            continue;
        }
        let padding = 6 - ch.len_utf8();
        for prefix in 0..=padding {
            let input = format!(
                "{}{}{}",
                "a".repeat(prefix),
                ch,
                "f".repeat(padding - prefix)
            );
            assert_eq!(parse_hex(&input), None, "{input:?}");
        }
    }
    let mut seed = 0xd633_e457_f228_710bu64;
    for _ in 0..20_000 {
        let mut input = String::new();
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        for _ in 0..seed % 20 {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            if let Some(ch) = char::from_u32((seed % 0x110000) as u32) {
                input.push(ch);
            }
        }
        let result = parse_hex(&input);
        let trimmed = input.trim();
        let bytes = trimmed.strip_prefix('#').unwrap_or(trimmed).as_bytes();
        assert_eq!(
            result.is_some(),
            bytes.len() == 6 && bytes.iter().all(u8::is_ascii_hexdigit)
        );
    }
}

#[test]
fn actual_color_file_reload_keeps_previous_values_and_reports_invalid_keys_then_recovers() {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let path = std::env::temp_dir().join(format!(
        "omatainer-theme-{}-{}.toml",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    struct Cleanup(PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.0);
        }
    }
    let _cleanup = Cleanup(path.clone());
    let mut theme = Theme::default();
    fs::write(
        &path,
        "background = '#010203'\naccent = 'AbCdEf'\nred = '#456789'\n",
    )
    .unwrap();
    assert!(theme.reload_colors(&path).is_empty());
    let previous = theme.clone();
    fs::write(&path, "background = '#fefdfc'\naccent = '€abc'\nred = 42\n").unwrap();
    let diagnostics = theme.reload_colors(&path);
    assert_eq!(theme.bg, rgb(254, 253, 252));
    assert_eq!(theme.accent, previous.accent);
    assert_eq!(theme.red, previous.red);
    assert_eq!(
        theme.green, previous.green,
        "absent keys preserve the fallback silently"
    );
    assert_eq!(
        diagnostics,
        [
            ColorDiagnostic { key: "accent" },
            ColorDiagnostic { key: "red" }
        ]
    );
    for diagnostic in diagnostics {
        let message = diagnostic.to_string();
        assert!(message.contains(diagnostic.key));
        assert!(message.contains("six ASCII hex digits"));
        assert!(message.contains("keeping the previous color"));
    }
    fs::write(&path, "accent = '#987654'\nred = '000000'\n").unwrap();
    assert!(theme.reload_colors(&path).is_empty());
    assert_eq!(theme.accent, rgb(0x98, 0x76, 0x54));
    assert_eq!(theme.red, Color32::BLACK);
}
