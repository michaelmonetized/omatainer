//! Fixed-size, validated hot-cue presentation metadata. No cue edit needs a
//! String allocation or final reference-counted drop on the audio callback.
use serde::{Deserialize, Deserializer, Serialize, Serializer};

pub(crate) const LABEL_BYTES: usize = 64;
pub(crate) const STYLE_WORDS: usize = 1 + LABEL_BYTES / 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Name {
    len: u8,
    bytes: [u8; LABEL_BYTES],
}
impl Default for Name {
    fn default() -> Self {
        Self {
            len: 0,
            bytes: [0; LABEL_BYTES],
        }
    }
}
impl Name {
    pub fn new(value: &str) -> Result<Self, &'static str> {
        if value.len() > LABEL_BYTES {
            return Err("Cue names must fit within 64 UTF-8 bytes");
        }
        if value.chars().any(char::is_control) {
            return Err("Cue names cannot contain control characters");
        }
        let mut name = Self::default();
        name.len = value.len() as u8;
        name.bytes[..value.len()].copy_from_slice(value.as_bytes());
        Ok(name)
    }
    pub fn as_str(&self) -> &str {
        // Only the validated constructors can populate the private fields.
        std::str::from_utf8(&self.bytes[..self.len as usize]).unwrap()
    }
}
impl Serialize for Name {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}
impl<'de> Deserialize<'de> for Name {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        Self::new(&value).map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Style {
    #[serde(default)]
    pub name: Name,
    /// None uses the current theme's slot color; custom colors are opaque RGB.
    #[serde(default)]
    pub color: Option<[u8; 3]>,
}
impl Style {
    pub(super) fn words(self) -> [u64; STYLE_WORDS] {
        let mut words = [0; STYLE_WORDS];
        words[0] = self.name.len as u64;
        if let Some([r, g, b]) = self.color {
            words[0] |= 1 << 8 | (r as u64) << 16 | (g as u64) << 24 | (b as u64) << 32;
        }
        for (word, bytes) in words[1..].iter_mut().zip(self.name.bytes.chunks_exact(8)) {
            *word = u64::from_le_bytes(bytes.try_into().unwrap());
        }
        words
    }
    pub(super) fn from_words(words: [u64; STYLE_WORDS]) -> Option<Self> {
        let header = words[0];
        let known = 0xff | 1 << 8 | 0xffffff << 16;
        if header & !known != 0 {
            return None;
        }
        let len = (header & 0xff) as usize;
        if len > LABEL_BYTES {
            return None;
        }
        let mut bytes = [0; LABEL_BYTES];
        for (dest, word) in bytes.chunks_exact_mut(8).zip(&words[1..]) {
            dest.copy_from_slice(&word.to_le_bytes());
        }
        if bytes[len..].iter().any(|byte| *byte != 0) {
            return None;
        }
        let name = Name::new(std::str::from_utf8(&bytes[..len]).ok()?).ok()?;
        let color = if header & 1 << 8 != 0 {
            Some([
                (header >> 16) as u8,
                (header >> 24) as u8,
                (header >> 32) as u8,
            ])
        } else {
            if header >> 16 != 0 {
                return None;
            }
            None
        };
        Some(Self { name, color })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounded_unicode_labels_and_optional_rgb_roundtrip_without_callback_heap_work() {
        for text in ["", "Drop 1", "café / 東京 🎵", &"🎵".repeat(16)] {
            let style = Style {
                name: Name::new(text).unwrap(),
                color: Some([1, 127, 255]),
            };
            let mut restored = None;
            let counts = crate::engine::test_alloc::measure(|| {
                restored = Style::from_words(style.words());
            });
            assert_eq!(counts, crate::engine::test_alloc::Counts::default());
            assert_eq!(restored, Some(style));
            assert_eq!(
                serde_json::from_str::<Style>(&serde_json::to_string(&style).unwrap()).unwrap(),
                style
            );
        }
        assert_eq!(
            Style::from_words(Style::default().words()),
            Some(Style::default())
        );
        assert!(Name::new(&"🎵".repeat(17)).is_err());
        assert!(Name::new("line\nbreak").is_err());
        assert!(Name::new("nul\0byte").is_err());
        assert!(serde_json::from_str::<Style>(r#"{"name":"x","color":[1,2,256]}"#).is_err());
        assert!(serde_json::from_str::<Style>(r#"{"unexpected":1}"#).is_err());
    }
    #[test]
    fn invalid_word_lengths_utf8_padding_flags_and_colors_fail_closed() {
        for words in [
            {
                let mut w = [0; STYLE_WORDS];
                w[0] = 65;
                w
            },
            {
                let mut w = [0; STYLE_WORDS];
                w[0] = 1;
                w[1] = 255;
                w
            },
            {
                let mut w = [0; STYLE_WORDS];
                w[1] = 65;
                w
            },
            {
                let mut w = [0; STYLE_WORDS];
                w[0] = 1 << 40;
                w
            },
            {
                let mut w = [0; STYLE_WORDS];
                w[0] = 1 << 16;
                w
            },
        ] {
            assert!(Style::from_words(words).is_none());
        }
    }
}
