//! Deck waveform views saved in the active preference profile.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Zoom {
    TwoBars,
    #[default]
    FourBars,
    EightBars,
    SixteenBars,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::preferences::{storage, Preferences};
    #[test]
    fn old_headers_migrate_without_waveform_fields_and_new_fields_cannot_be_smuggled() {
        let current = Preferences::defaults(std::path::Path::new("/tmp"));
        let mut old = serde_json::to_value(&current).unwrap();
        old["version"] = 13.into();
        for profile in old["profiles"].as_object_mut().unwrap().values_mut() {
            profile.as_object_mut().unwrap().remove("waveforms");
        }
        let (loaded, migrated) = storage::decode(&serde_json::to_vec(&old).unwrap()).unwrap();
        assert!(migrated);
        assert_eq!(loaded, current);
        for value in [
            serde_json::Value::Null,
            serde_json::json!({}),
            serde_json::to_value(Config::default()).unwrap(),
        ] {
            old["profiles"]["Studio"]["waveforms"] = value;
            assert!(storage::decode(&serde_json::to_vec(&old).unwrap()).is_err());
        }
        let mut invalid = current;
        invalid.profiles.get_mut("Studio").unwrap().waveforms.zoom[1] = Zoom::TwoBars;
        assert!(invalid.validate().is_err());
    }
}
impl Zoom {
    /// Choose the musical span.
    /// Takes this zoom; returns the number of four-beat bars expressed in beats.
    pub fn beats(self) -> f64 {
        match self {
            Self::TwoBars => 8.0,
            Self::FourBars => 16.0,
            Self::EightBars => 32.0,
            Self::SixteenBars => 64.0,
        }
    }
    /// Choose a span without a beatgrid.
    /// Takes this zoom; returns the displayed source seconds.
    pub fn seconds(self) -> f64 {
        7.0 * self.beats() / 16.0
    }
    /// Name the current span.
    /// Takes this zoom and whether a beatgrid exists; returns the visible span with its actual units.
    pub fn label(self, grid: bool) -> &'static str {
        match (self, grid) {
            (Self::TwoBars, true) => "2 bars",
            (Self::FourBars, true) => "4 bars",
            (Self::EightBars, true) => "8 bars",
            (Self::SixteenBars, true) => "16 bars",
            (Self::TwoBars, false) => "3.5 s",
            (Self::FourBars, false) => "7 s",
            (Self::EightBars, false) => "14 s",
            (Self::SixteenBars, false) => "28 s",
        }
    }
    /// Step through supported spans.
    /// Takes this zoom; returns the next span, wrapping after sixteen bars.
    pub fn next(self) -> Self {
        match self {
            Self::TwoBars => Self::FourBars,
            Self::FourBars => Self::EightBars,
            Self::EightBars => Self::SixteenBars,
            Self::SixteenBars => Self::TwoBars,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct Config {
    pub linked: bool,
    pub zoom: [Zoom; crate::engine::DECKS],
}
impl Default for Config {
    fn default() -> Self {
        Self {
            linked: true,
            zoom: [Zoom::default(); crate::engine::DECKS],
        }
    }
}
impl Config {
    /// Check linked view consistency.
    /// Takes this configuration; returns an error when linked decks have different spans.
    pub fn validate(&self) -> Result<(), String> {
        if self.linked && self.zoom.iter().any(|zoom| *zoom != self.zoom[0]) {
            return Err("Linked waveform zoom must use one span".into());
        }
        Ok(())
    }
    /// Change one or both waveform spans.
    /// Takes a deck index and span; updates both when linked and ignores invalid indices.
    pub fn set(&mut self, deck: usize, zoom: Zoom) {
        if deck >= self.zoom.len() {
            return;
        }
        if self.linked {
            self.zoom.fill(zoom);
        } else {
            self.zoom[deck] = zoom;
        }
    }
}
