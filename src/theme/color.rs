//! Accepted color text: six ASCII hex digits, optionally one leading '#',
//! with surrounding whitespace ignored. Alpha, shorthand and trailing text
//! are deliberately unsupported. Never index a UTF-8 string by byte offset.

pub(super) fn parse_rgb(text: &str) -> Option<[u8; 3]> {
    let text = text.trim();
    let text = text.strip_prefix('#').unwrap_or(text);
    let &[r0, r1, g0, g1, b0, b1] = text.as_bytes() else {
        return None;
    };
    Some([
        nibble(r0)? * 16 + nibble(r1)?,
        nibble(g0)? * 16 + nibble(g1)?,
        nibble(b0)? * 16 + nibble(b1)?,
    ])
}

fn nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}
