//! One channel identity contract belongs to routing, rendering and delivery.
use serde::{Deserialize, Serialize};
pub(crate) mod monitor;
#[cfg(test)]
pub(crate) mod tests;
pub(crate) const MAX_BINDINGS: usize = 256;

#[derive(Clone, Copy, Debug, Serialize)]
pub(crate) struct Meter {
    pub group: super::model::Group,
    pub layout: Layout,
    pub peaks: [f32; 8],
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Layout {
    Mono,
    #[default]
    Stereo,
    StereoLfe,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Speaker {
    FrontLeft,
    FrontRight,
    FrontCenter,
    LowFrequency,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Binding {
    pub group: super::model::Group,
    pub layout: Layout,
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BassManagement {
    pub output: u64,
    pub cutoff: f32,
    pub level: f32,
}
impl Layout {
    pub const ALL: &'static [Self] = &[Self::Mono, Self::Stereo, Self::StereoLfe];
    /// Name the canonical speaker order for a saved choice.
    /// Takes this layout; returns a human-readable label shared by routing and delivery controls.
    pub fn label(self) -> &'static str {
        match self {
            Self::Mono => "Mono (FC)",
            Self::Stereo => "Stereo (FL FR)",
            Self::StereoLfe => "2.1 (FL FR LFE)",
        }
    }
    /// Read the declared interleaved speaker order.
    /// Takes the saved layout; returns channel identities in ascending WAVE speaker-bit order.
    pub fn speakers(self) -> &'static [Speaker] {
        match self {
            Self::Mono => &[Speaker::FrontCenter],
            Self::Stereo => &[Speaker::FrontLeft, Speaker::FrontRight],
            Self::StereoLfe => &[
                Speaker::FrontLeft,
                Speaker::FrontRight,
                Speaker::LowFrequency,
            ],
        }
    }
    /// Read the exact WAVE speaker mask.
    /// Takes the saved layout; returns its canonical interleaving mask.
    pub fn wave_mask(self) -> u32 {
        match self {
            Self::Mono => 4,
            Self::Stereo => 3,
            Self::StereoLfe => 0xB,
        }
    }
    /// Read the declared bed width.
    /// Takes this layout; returns its exact number of interleaved channels.
    pub fn channels(self) -> usize {
        self.speakers().len()
    }
    /// Fold the program for headphones without bass management.
    /// Takes a canonical logical frame; returns unity stereo or mono duplication, excluding the dedicated LFE channel.
    pub fn downmix(self, frame: &[f32]) -> [f32; 2] {
        match self {
            Self::Mono => [frame[0]; 2],
            Self::Stereo | Self::StereoLfe => [frame[0], frame[1]],
        }
    }
    /// Position a mono source between the two full-range front speakers.
    /// Takes canonical destination channel and normalized pan; returns equal-power gain with endpoints held, never feeding LFE.
    pub fn pan_gain(self, channel: usize, pan: f32) -> f32 {
        self.pan_frame(pan).get(channel).copied().unwrap_or(0.)
    }
    /// Prepare one positional gain vector for reuse by all channel maps.
    /// Takes normalized mono position; returns at most eight bounded gains with no LFE contribution.
    pub fn pan_frame(self, pan: f32) -> [f32; 8] {
        let mut gains = [0.; 8];
        if self != Self::StereoLfe {
            return gains;
        }
        let phase = (pan.clamp(-1., 1.) + 1.) * std::f32::consts::FRAC_PI_4;
        if pan <= -1. {
            gains[0] = 1.;
        } else if pan >= 1. {
            gains[1] = 1.;
        } else {
            let (right, left) = phase.sin_cos();
            gains[0] = left;
            gains[1] = right;
        }
        gains
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
    /// Describe exact encoded speakers from a declared source bed.
    /// Takes the canonical layout, ordered physical source indices and encoded width; returns metadata for the native bed or an explicit mono/stereo fold.
    pub fn declared(
        layout: Layout,
        channels: Vec<u16>,
        channel_count: u16,
    ) -> Result<Self, String> {
        if channels.len() != layout.channels()
            || channels
                .iter()
                .any(|c| usize::from(*c) >= super::model::MAX_PHYSICAL_CHANNELS)
            || channels
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                != channels.len()
        {
            return Err("Declared speaker layout needs exact unique physical channels".into());
        }
        let encoded = match channel_count {
            1 => Layout::Mono,
            2 => Layout::Stereo,
            n if usize::from(n) == layout.channels() => layout,
            _ => {
                return Err(
                    "Native delivery width must match the exact declared speaker layout".into(),
                )
            }
        };
        Ok(Self {
            channel_count,
            channels,
            speakers: encoded.speakers().to_vec(),
            wave_mask: Some(encoded.wave_mask()),
        })
    }
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

impl super::model::Model {
    /// Resolve declared speaker identity without guessing a wide bed from its width.
    /// Takes a stable graph endpoint; returns its saved layout or the legacy mono/stereo default.
    pub fn speaker_layout(&self, group: super::model::Group) -> Option<Layout> {
        use super::model::{Direction, Group};
        self.speaker_layouts
            .iter()
            .find(|b| b.group == group)
            .map(|b| b.layout)
            .or_else(|| match group {
                Group::Track(_)
                | Group::Scene(_)
                | Group::Main
                | Group::Deck(_)
                | Group::FixedReturn(_) => Some(Layout::Stereo),
                Group::Bus(id) => {
                    self.buses
                        .iter()
                        .find(|b| b.id == id)
                        .and_then(|b| match b.channels {
                            1 => Some(Layout::Mono),
                            2 => Some(Layout::Stereo),
                            _ => None,
                        })
                }
                Group::Input(id) | Group::Output(id) | Group::Record(id) => {
                    let direction = match group {
                        Group::Input(_) => Direction::Input,
                        Group::Output(_) => Direction::Output,
                        _ => Direction::Record,
                    };
                    self.port(id, direction)
                        .and_then(|p| match p.channels.len() {
                            1 => Some(Layout::Mono),
                            2 => Some(Layout::Stereo),
                            _ => None,
                        })
                }
                Group::Plugin(id) => self.plugins.iter().find(|p| p.id == id).and_then(|p| {
                    let width = usize::from(p.outputs.first().copied()?);
                    p.return_bus
                        .and_then(|id| self.speaker_layout(Group::Bus(id)))
                        .or_else(|| {
                            p.scene_track
                                .or(p.midi_track)
                                .and_then(|id| self.speaker_layout(Group::Track(id)))
                        })
                        .filter(|l| l.channels() == width)
                        .or_else(|| match width {
                            1 => Some(Layout::Mono),
                            2 => Some(Layout::Stereo),
                            _ => None,
                        })
                }),
            })
    }
    /// Describe an exact saved recording or delivery source.
    /// Takes its stable physical alias and requested encoded width; returns ordered source indices and actual encoded speaker metadata, refusing undeclared wide channels.
    pub fn channel_contract(
        &self,
        group: super::model::Group,
        channel_count: u16,
    ) -> Result<OutputContract, String> {
        use super::model::{Direction, Group};
        let (id, direction) = match group {
            Group::Input(id) => (id, Direction::Input),
            Group::Output(id) => (id, Direction::Output),
            Group::Record(id) => (id, Direction::Record),
            _ => return Err("Speaker output contract requires an exact physical alias".into()),
        };
        let port = self
            .port(id, direction)
            .ok_or("Reviewed channel alias was removed")?;
        if self
            .speaker_layouts
            .iter()
            .filter(|b| b.group == group)
            .count()
            > 1
        {
            return Err("Speaker alias has duplicate layout declarations".into());
        }
        let layout=self.speaker_layout(group).ok_or("Wide audio requires an explicitly declared speaker layout; channel count alone has no speaker identity")?;
        OutputContract::declared(layout, port.channels.clone(), channel_count)
    }
    /// Retain one canonical layout on a producer-side draft.
    /// Takes its stable endpoint and declared layout; replaces that endpoint's binding without changing physical aliases or maps.
    pub fn set_speaker_layout(&mut self, group: super::model::Group, layout: Layout) {
        if let Some(binding) = self.speaker_layouts.iter_mut().find(|b| b.group == group) {
            binding.layout = layout;
        } else {
            self.speaker_layouts.push(Binding { group, layout });
        }
    }
    /// Validate additive saved speaker declarations before graph admission.
    /// Takes retained session identity; rejects duplicates, unsupported owners and mismatched physical widths atomically.
    pub(super) fn validate_speakers(
        &self,
        session: &crate::engine::session::Layout,
    ) -> Result<(), String> {
        use super::model::Group;
        if self.speaker_layouts.len() > MAX_BINDINGS {
            return Err("Speaker declarations exceed 256 retained endpoints".into());
        }
        let mut groups = std::collections::BTreeSet::new();
        for binding in &self.speaker_layouts {
            if binding.layout == Layout::Mono
                && matches!(
                    binding.group,
                    Group::Track(_) | Group::Scene(_) | Group::Main
                )
            {
                return Err("Track, scene and main beds retain stereo or an explicitly declared wide layout; mono sources use a mono alias or return".into());
            }
            let width = if let Group::Plugin(id) = binding.group {
                self.plugins
                    .iter()
                    .find(|p| p.id == id)
                    .and_then(|p| p.outputs.first())
                    .copied()
                    .map(usize::from)
            } else {
                self.width(binding.group, session)
            };
            if !groups.insert(binding.group)
                || matches!(binding.group, Group::Deck(_) | Group::FixedReturn(_))
                || width != Some(binding.layout.channels())
            {
                return Err(
                    "Speaker layout has a missing, duplicate or width-incompatible endpoint".into(),
                );
            }
        }
        for plugin in &self.plugins {
            if let Some(layout) = plugin
                .scene_track
                .or(plugin.midi_track)
                .and_then(|id| self.speaker_layout(Group::Track(id)))
                .filter(|l| l.channels() > 2)
            {
                let width = layout.channels();
                if !plugin.instrument
                    && (plugin.inputs.first().copied().map(usize::from) != Some(width)
                        || plugin.outputs.first().copied().map(usize::from) != Some(width))
                    || plugin.instrument
                        && plugin
                            .outputs
                            .first()
                            .is_some_and(|n| usize::from(*n) > 2 && usize::from(*n) != width)
                {
                    return Err("A declared wide track needs a matching native primary bed or explicit format-conversion routes".into());
                }
            }
            if let Some(layout) = plugin
                .return_bus
                .and_then(|id| self.speaker_layout(Group::Bus(id)))
                .filter(|l| l.channels() > 2)
            {
                if plugin.inputs.first().copied().map(usize::from) != Some(layout.channels())
                    || plugin.outputs.first().copied().map(usize::from) != Some(layout.channels())
                {
                    return Err("A native return must preserve its declared wide primary speaker bed; use explicit reviewed maps for a format conversion".into());
                }
            }
        }
        if let Some(bass) = self.monitor_bass {
            if self.speaker_layout(Group::Output(bass.output)) != Some(Layout::StereoLfe)
                || !bass.cutoff.is_finite()
                || !(20.0..=200.0).contains(&bass.cutoff)
                || !bass.level.is_finite()
                || !(0.0..=1.5).contains(&bass.level)
            {
                return Err("Monitor bass management requires a declared 2.1 output, 20–200 Hz cutoff and level0–1.5".into());
            }
        }
        Ok(())
    }
}
