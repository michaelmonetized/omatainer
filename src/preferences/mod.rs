//! Versioned user preferences. Profiles contain supported app behavior only;
//! audio selection is consumed at startup or by the confirmed audio-owner workflow.
pub mod recovery;
pub mod storage;
pub mod worker;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

pub const VERSION: u32 = 6;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AudioFormat {
    F32,
    F64,
    I8,
    I16,
    I32,
    I64,
    U8,
    U16,
    U32,
    U64,
}
impl AudioFormat {
    pub const ALL: [Self; 10] = [
        Self::F32,
        Self::F64,
        Self::I8,
        Self::I16,
        Self::I32,
        Self::I64,
        Self::U8,
        Self::U16,
        Self::U32,
        Self::U64,
    ];
    pub fn cpal(self) -> cpal::SampleFormat {
        match self {
            Self::F32 => cpal::SampleFormat::F32,
            Self::F64 => cpal::SampleFormat::F64,
            Self::I8 => cpal::SampleFormat::I8,
            Self::I16 => cpal::SampleFormat::I16,
            Self::I32 => cpal::SampleFormat::I32,
            Self::I64 => cpal::SampleFormat::I64,
            Self::U8 => cpal::SampleFormat::U8,
            Self::U16 => cpal::SampleFormat::U16,
            Self::U32 => cpal::SampleFormat::U32,
            Self::U64 => cpal::SampleFormat::U64,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CalibrationInput {
    pub device: Option<String>,
    pub channels: Option<u16>,
    pub format: Option<AudioFormat>,
    pub buffer_frames: Option<u32>,
    pub channel: u16,
    pub output_channel: u16,
    pub level_db: f32,
}
impl Default for CalibrationInput {
    fn default() -> Self {
        Self {
            device: None,
            channels: None,
            format: None,
            buffer_frames: None,
            channel: 0,
            output_channel: 0,
            level_db: -40.0,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Audio {
    #[serde(default)]
    pub backend: Option<String>,
    #[serde(default)]
    pub format: Option<AudioFormat>,
    #[serde(default)]
    pub calibration: CalibrationInput,
    pub device: Option<String>,
    pub sample_rate: Option<u32>,
    pub channels: Option<u16>,
    pub buffer_frames: Option<u32>,
}
impl Audio {
    pub fn output_eq(&self, other: &Self) -> bool {
        self.backend == other.backend
            && self.device == other.device
            && self.sample_rate == other.sample_rate
            && self.channels == other.channels
            && self.buffer_frames == other.buffer_frames
            && self.format == other.format
    }
}
impl Default for Audio {
    fn default() -> Self {
        Self {
            backend: None,
            format: None,
            calibration: CalibrationInput::default(),
            device: None,
            sample_rate: None,
            channels: None,
            buffer_frames: None,
        }
    }
}

pub use crate::engine::midi::InputPolicy as MidiInputs;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Appearance {
    pub follow_theme: bool,
    pub font_size: Option<f32>,
    pub scale: f32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Shortcut {
    pub key: String,
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Startup {
    #[serde(default)]
    pub performance_mode: bool,
    pub scan_library: bool,
    pub show_help: bool,
    pub show_midi: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    pub audio: Audio,
    pub midi_inputs: MidiInputs,
    #[serde(default)]
    pub midi_routing: crate::engine::midi::routing::Routing,
    pub library_roots: Vec<PathBuf>,
    pub appearance: Appearance,
    pub shortcuts_enabled: bool,
    /// Stable action id to override; null explicitly disables that action.
    #[serde(deserialize_with = "unique_map")]
    pub shortcuts: BTreeMap<String, Option<Shortcut>>,
    pub startup: Startup,
    #[serde(default)]
    pub recovery: crate::recovery::Config,
}
impl Profile {
    pub fn defaults(home: &std::path::Path) -> Self {
        Self {
            audio: Audio::default(),
            midi_inputs: MidiInputs::All,
            midi_routing: crate::engine::midi::routing::Routing::default(),
            library_roots: vec![home.join("Music"), home.join("music")],
            appearance: Appearance {
                follow_theme: true,
                font_size: None,
                scale: 1.0,
            },
            shortcuts_enabled: true,
            shortcuts: BTreeMap::new(),
            recovery: crate::recovery::Config::default(),
            startup: Startup {
                performance_mode: false,
                scan_library: true,
                show_help: false,
                show_midi: false,
            },
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Preferences {
    pub version: u32,
    pub active: String,
    #[serde(deserialize_with = "unique_map")]
    pub profiles: BTreeMap<String, Profile>,
}
impl Preferences {
    pub fn defaults(home: &std::path::Path) -> Self {
        let studio = Profile::defaults(home);
        let mut performance = studio.clone();
        performance.startup.scan_library = false;
        performance.startup.performance_mode = true;
        Self {
            version: VERSION,
            active: "Studio".into(),
            profiles: BTreeMap::from([
                ("Studio".into(), studio),
                ("Performance".into(), performance),
            ]),
        }
    }
}

impl Preferences {
    pub fn current(&self) -> Option<&Profile> {
        self.profiles.get(&self.active)
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.version != VERSION {
            return Err(format!(
                "Unsupported preferences version {}; this release reads version {VERSION}",
                self.version
            ));
        }
        if self.profiles.is_empty() || self.profiles.len() > 32 {
            return Err("Keep between 1 and 32 profiles".into());
        }
        if !self.profiles.contains_key(&self.active) {
            return Err("Active profile does not exist".into());
        }
        for (name, profile) in &self.profiles {
            if name.trim().is_empty()
                || name.trim() != name
                || name.chars().count() > 80
                || name.chars().any(char::is_control)
            {
                return Err(
                    "Profile names must contain 1–80 visible characters without surrounding spaces"
                        .into(),
                );
            }
            profile
                .validate()
                .map_err(|error| format!("{name}: {error}"))?;
        }
        Ok(())
    }
}

impl Profile {
    pub fn validate(&self) -> Result<(), String> {
        self.recovery.validate()?;
        let calibration = &self.audio.calibration;
        for name in [&self.audio.backend, &calibration.device]
            .into_iter()
            .flatten()
        {
            if name.trim().is_empty() || name.len() > 1024 || name.chars().any(char::is_control) {
                return Err("Choose a valid exact audio backend/device name".into());
            }
        }
        if calibration.channels.is_some_and(|v| !(1..=64).contains(&v))
            || calibration
                .buffer_frames
                .is_some_and(|v| !(16..=32768).contains(&v))
            || calibration.channel >= 64
            || calibration.output_channel >= 64
            || !calibration.level_db.is_finite()
            || !(-60.0..=-24.0).contains(&calibration.level_db)
        {
            return Err("Calibration channel, buffer or safe probe level is invalid".into());
        }
        if self
            .audio
            .device
            .as_ref()
            .is_some_and(|name| name.trim().is_empty() || name.len() > 1024 || name.contains('\0'))
        {
            return Err("Choose a device name or System default".into());
        }
        if self
            .audio
            .sample_rate
            .is_some_and(|rate| !(8_000..=384_000).contains(&rate))
        {
            return Err("Audio sample rate must be 8000–384000 Hz or Device default".into());
        }
        if self
            .audio
            .channels
            .is_some_and(|channels| !(1..=64).contains(&channels))
        {
            return Err("Audio channel count must be 1–64 or Device default".into());
        }
        if self
            .audio
            .buffer_frames
            .is_some_and(|frames| !(16..=32768).contains(&frames))
        {
            return Err("Audio buffer must be 16–32768 frames or Device default".into());
        }
        self.midi_inputs
            .validate()
            .map_err(|error| error.to_string())?;
        self.midi_routing.validate()?;
        if let MidiInputs::Selected(names) = &self.midi_inputs {
            if names.is_empty() || names.len() > 64 {
                return Err("Select 1–64 MIDI inputs, or choose Disabled".into());
            }
            let mut unique = std::collections::BTreeSet::new();
            for name in names {
                if name.trim().is_empty()
                    || name.len() > 1024
                    || name.contains(['\0', '\n', '\r'])
                    || !unique.insert(name)
                {
                    return Err("MIDI input names must be nonempty and unique".into());
                }
            }
        }
        if self.library_roots.len() > 64 {
            return Err("Use at most 64 library folders".into());
        }
        let mut unique = std::collections::BTreeSet::new();
        for root in &self.library_roots {
            if !root.is_absolute()
                || root.as_os_str().len() > 4096
                || root
                    .to_str()
                    .is_none_or(|text| text.contains(['\0', '\n', '\r']))
                || !unique.insert(root)
            {
                return Err("Library folders must be unique absolute paths".into());
            }
        }
        if !self.appearance.scale.is_finite() || !(0.5..=3.0).contains(&self.appearance.scale) {
            return Err("UI scale must be 50–300%".into());
        }
        if self
            .appearance
            .font_size
            .is_some_and(|size| !size.is_finite() || !(8.0..=48.0).contains(&size))
        {
            return Err("Font size must be 8–48 points or Theme default".into());
        }
        if self.shortcuts.len() > 64 {
            return Err("Too many shortcut overrides".into());
        }
        for shortcut in self.shortcuts.values().flatten() {
            shortcut.validate()?;
        }
        crate::ui::validate_shortcuts(self)?;
        Ok(())
    }
}

impl Shortcut {
    pub fn validate(&self) -> Result<(), String> {
        let key = egui::Key::from_name(&self.key)
            .ok_or_else(|| format!("Unknown shortcut key {}", self.key))?;
        if matches!(
            key,
            egui::Key::Tab | egui::Key::Enter | egui::Key::F2 | egui::Key::F10
        ) || (self.ctrl
            && matches!(
                key,
                egui::Key::N
                    | egui::Key::O
                    | egui::Key::S
                    | egui::Key::Z
                    | egui::Key::Y
                    | egui::Key::Comma
            ))
        {
            return Err("That key is reserved for navigation, editing or project commands".into());
        }
        Ok(())
    }
    pub fn modifiers(&self) -> egui::Modifiers {
        egui::Modifiers {
            ctrl: self.ctrl,
            command: self.ctrl,
            shift: self.shift,
            alt: self.alt,
            mac_cmd: false,
        }
    }
    pub fn label(&self) -> String {
        format!(
            "{}{}{}{}",
            if self.ctrl { "Ctrl+" } else { "" },
            if self.alt { "Alt+" } else { "" },
            if self.shift { "Shift+" } else { "" },
            self.key
        )
    }
}

fn unique_map<'de, D, T>(deserializer: D) -> Result<BTreeMap<String, T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    struct Unique<T>(std::marker::PhantomData<T>);
    impl<'de, T: Deserialize<'de>> serde::de::Visitor<'de> for Unique<T> {
        type Value = BTreeMap<String, T>;
        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("a map with unique keys")
        }
        fn visit_map<M: serde::de::MapAccess<'de>>(
            self,
            mut map: M,
        ) -> Result<Self::Value, M::Error> {
            let mut values = BTreeMap::new();
            while let Some((key, value)) = map.next_entry::<String, T>()? {
                if values.insert(key.clone(), value).is_some() {
                    return Err(serde::de::Error::custom(format!(
                        "duplicate preference key {key}"
                    )));
                }
            }
            Ok(values)
        }
    }
    deserializer.deserialize_map(Unique(std::marker::PhantomData))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shortcut_catalog_rejects_unknown_reserved_and_effective_conflicts_without_loss() {
        let mut profile = Profile::defaults(std::path::Path::new("/private"));
        profile.validate().unwrap();
        let shortcut = Shortcut {
            key: "Q".into(),
            ctrl: false,
            shift: false,
            alt: false,
        };
        profile
            .shortcuts
            .insert("transport".into(), Some(shortcut.clone()));
        assert!(profile.validate().unwrap_err().contains("both use"));
        profile.shortcuts.insert("play_a".into(), None);
        profile.validate().unwrap();
        profile.shortcuts.insert("nonexistent".into(), None);
        assert!(profile.validate().unwrap_err().contains("Unknown shortcut"));
        profile.shortcuts.remove("nonexistent");
        for key in ["F2", "Tab", "Enter"] {
            let mut s = shortcut.clone();
            s.key = key.into();
            assert!(s.validate().is_err());
        }
        profile.shortcuts.insert(
            "transport".into(),
            Some(Shortcut {
                key: "S".into(),
                ctrl: true,
                shift: false,
                alt: false,
            }),
        );
        assert!(profile.validate().is_err());
        profile.shortcuts.clear();
        profile.appearance.scale = f32::NAN;
        assert!(profile.validate().is_err());
    }
}
