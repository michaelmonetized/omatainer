//! One channel identity contract belongs to routing, rendering and delivery.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Layout {
    Mono,
    #[default]
    Stereo,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Speaker {
    FrontLeft,
    FrontRight,
    FrontCenter,
}
impl Layout {
    /// Read the declared interleaved speaker order.
    /// Takes the saved layout; returns channel identities in ascending WAVE speaker-bit order.
    pub fn speakers(self) -> &'static [Speaker] {
        match self {
            Self::Mono => &[Speaker::FrontCenter],
            Self::Stereo => &[Speaker::FrontLeft, Speaker::FrontRight],
        }
    }
    /// Read the exact WAVE speaker mask.
    /// Takes the saved layout; returns its canonical interleaving mask.
    pub fn wave_mask(self) -> u32 {
        match self {
            Self::Mono => 4,
            Self::Stereo => 3,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct OutputContract {
    pub channel_count: u16,
    pub channels: Vec<u16>,
    pub speakers: Vec<Speaker>,
    pub wave_mask: Option<u32>,
}
impl OutputContract {
    /// Describe a selected mono/stereo output.
    /// Takes ordered physical source indices and requested delivery width; returns canonical speaker metadata, including the existing mono fold or mono duplication policy.
    pub fn stereo(channels: Vec<u16>, channel_count: u16) -> Result<Self, String> {
        if !(1..=2).contains(&channel_count)
            || !(1..=2).contains(&channels.len())
            || channels
                .iter()
                .any(|c| usize::from(*c) >= super::model::MAX_PHYSICAL_CHANNELS)
        {
            return Err("Choose an exact mono/stereo program output".into());
        }
        let layout = if channel_count == 1 {
            Layout::Mono
        } else {
            Layout::Stereo
        };
        Ok(Self {
            channel_count,
            channels,
            speakers: layout.speakers().to_vec(),
            wave_mask: Some(layout.wave_mask()),
        })
    }
}
