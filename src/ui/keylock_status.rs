//! Show the renderer's processing mode, including audible bypass conditions.
use super::*;
use crate::engine::keylock::{Mode, MIN_RATIO, MAX_RATIO};

pub(super) fn presentation(
    snap: &crate::engine::DeckSnap,
    theme: &Theme,
) -> (&'static str, Color32, String) {
    let (mark, accent, state) = match snap.keylock_mode {
        Mode::Off => ("L", theme.cyan, "Off: tempo and pitch change together"),
        Mode::NoMedia => ("L", theme.cyan, "Armed: no media loaded"),
        Mode::Stopped => ("L", theme.cyan, "Armed: deck stopped"),
        Mode::Unity => ("L", theme.cyan, "Original rate: direct playback preserves pitch"),
        Mode::Locked => ("L", theme.cyan, "Pitch preservation active"),
        Mode::ScratchBypass => ("L~", theme.orange, "Scratch bypass: pitch follows platter movement"),
        Mode::UnsupportedRate => (
            "L!", theme.orange,
            "Rate outside pitch-lock range: direct playback changes tempo and pitch together",
        ),
    };
    let rate = if matches!(snap.keylock_mode, Mode::NoMedia | Mode::Stopped) {
        String::new()
    } else {
        format!(" Current playback rate {:.3}×.", snap.playback_rate)
    };
    (
        mark,
        accent,
        format!("{state}.{rate} Pitch-lock range {MIN_RATIO:.2}–{MAX_RATIO:.2}× forward playback."),
    )
}
