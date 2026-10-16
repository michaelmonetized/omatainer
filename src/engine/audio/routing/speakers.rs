//! One channel identity contract belongs to routing, rendering and delivery.
use serde::{Deserialize, Serialize};
pub(crate) mod monitor;
#[cfg(test)]
mod native_tests;
#[cfg(test)]
mod surround71_tests;
#[cfg(test)]
mod surround_tests;
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
    Surround51Side,
    Surround71,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Speaker {
    FrontLeft,
    FrontRight,
    FrontCenter,
    LowFrequency,
    SideLeft,
    SideRight,
    BackLeft,
    BackRight,
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
    pub const ALL: &'static [Self] = &[
        Self::Mono,
        Self::Stereo,
        Self::StereoLfe,
        Self::Surround51Side,
        Self::Surround71,
    ];
    /// Name the canonical speaker order for a saved choice.
    /// Takes this layout; returns a human-readable label shared by routing and delivery controls.
    pub fn label(self) -> &'static str {
        match self {
            Self::Mono => "Mono (FC)",
            Self::Stereo => "Stereo (FL FR)",
            Self::StereoLfe => "2.1 (FL FR LFE)",
            Self::Surround51Side => "5.1 side (FL FR FC LFE SL SR)",
            Self::Surround71 => "7.1 (FL FR FC LFE BL BR SL SR)",
        }
    }
    /// Name a canonical bed without repeating its channel list.
    /// Takes this layout; returns a stable short label for named routing controls.
    pub fn short_label(self) -> &'static str {
        match self {
            Self::Mono => "Mono",
            Self::Stereo => "Stereo",
            Self::StereoLfe => "2.1",
            Self::Surround51Side => "5.1 side",
            Self::Surround71 => "7.1",
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
            Self::Surround51Side => &[
                Speaker::FrontLeft,
                Speaker::FrontRight,
                Speaker::FrontCenter,
                Speaker::LowFrequency,
                Speaker::SideLeft,
                Speaker::SideRight,
            ],
            Self::Surround71 => &[
                Speaker::FrontLeft,
                Speaker::FrontRight,
                Speaker::FrontCenter,
                Speaker::LowFrequency,
                Speaker::BackLeft,
                Speaker::BackRight,
                Speaker::SideLeft,
                Speaker::SideRight,
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
            Self::Surround51Side => 0x60F,
            Self::Surround71 => 0x63F,
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
            Self::Surround51Side => {
                let q = std::f32::consts::FRAC_1_SQRT_2;
                [
                    frame[0] + q * (frame[2] + frame[4]),
                    frame[1] + q * (frame[2] + frame[5]),
                ]
            }
            Self::Surround71 => {
                let q = std::f32::consts::FRAC_1_SQRT_2;
                [
                    frame[0] + q * (frame[2] + frame[4] + frame[6]),
                    frame[1] + q * (frame[2] + frame[5] + frame[7]),
                ]
            }
        }
    }
    /// Read the explicit mono panner angle span.
    /// Takes this layout; returns degrees represented by normalized Pan1, keeping the front-pair policy for2.1.
    pub fn pan_extent(self) -> f32 {
        if matches!(self, Self::Surround51Side | Self::Surround71) {
            180.
        } else {
            30.
        }
    }
    /// Position a mono source between adjacent full-range speakers.
    /// Takes canonical destination channel and normalized pan; returns equal-power gain with endpoints held, never feeding LFE.
    pub fn pan_gain(self, channel: usize, pan: f32) -> f32 {
        self.pan_frame(pan).get(channel).copied().unwrap_or(0.)
    }
    /// Prepare one positional gain vector for reuse by all channel maps.
    /// Takes normalized mono position; returns at most eight bounded gains with no LFE contribution.
    pub fn pan_frame(self, pan: f32) -> [f32; 8] {
        let mut gains = [0.; 8];
        if matches!(self, Self::Surround51Side | Self::Surround71) {
            let anchors: &[(f32, usize)] = if self == Self::Surround71 {
                &[
                    (-150., 4),
                    (-90., 6),
                    (-30., 0),
                    (0., 2),
                    (30., 1),
                    (90., 7),
                    (150., 5),
                ]
            } else {
                &[(-110., 4), (-30., 0), (0., 2), (30., 1), (110., 5)]
            };
            let mut angle = pan.clamp(-1., 1.) * 180.;
            if angle < anchors[0].0 {
                angle += 360.;
            }
            let (left, right) = anchors
                .windows(2)
                .find(|pair| angle >= pair[0].0 && angle <= pair[1].0)
                .map_or(
                    (
                        anchors[anchors.len() - 1],
                        (anchors[0].0 + 360., anchors[0].1),
                    ),
                    |pair| (pair[0], pair[1]),
                );
            let position = ((angle - left.0) / (right.0 - left.0)).clamp(0., 1.);
            if position <= 1e-6 {
                gains[left.1] = 1.;
            } else if position >= 1. - 1e-6 {
                gains[right.1] = 1.;
            } else {
                let (r, l) = (position * std::f32::consts::FRAC_PI_2).sin_cos();
                gains[left.1] = l;
                gains[right.1] = r;
            }
            return gains;
        }
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
    /// Sum one canonical bed into another declared bed.
    /// Takes source and destination speaker identities and fixed frames; applies the explicit mono/stereo fold or speaker-preserving upmix without feeding LFE from full-range audio.
    pub(crate) fn mix_into(self, destination: Self, frame: &[f32], output: &mut [f32]) {
        if destination.channels() <= 2 {
            let pair = self.downmix(frame);
            output[0] += if destination == Self::Mono {
                0.5 * (pair[0] + pair[1])
            } else {
                pair[0]
            };
            if destination == Self::Stereo {
                output[1] += pair[1];
            }
        } else if self == Self::Mono {
            if let Some(center) = destination
                .speakers()
                .iter()
                .position(|s| *s == Speaker::FrontCenter)
            {
                output[center] += frame[0];
            } else {
                output[0] += frame[0] * std::f32::consts::FRAC_1_SQRT_2;
                output[1] += frame[0] * std::f32::consts::FRAC_1_SQRT_2;
            }
        } else if self == Self::Surround71 && destination == Self::Surround51Side {
            for channel in 0..4 {
                output[channel] += frame[channel];
            }
            let q = std::f32::consts::FRAC_1_SQRT_2;
            output[4] += q * (frame[4] + frame[6]);
            output[5] += q * (frame[5] + frame[7]);
        } else if destination == Self::StereoLfe && self.channels() > 3 {
            let pair = self.downmix(frame);
            output[0] += pair[0];
            output[1] += pair[1];
            if let Some(lfe) = self
                .speakers()
                .iter()
                .position(|s| *s == Speaker::LowFrequency)
            {
                output[2] += frame[lfe];
            }
        } else {
            for (channel, speaker) in self.speakers().iter().enumerate() {
                if let Some(target) = destination.speakers().iter().position(|s| s == speaker) {
                    output[target] += frame[channel];
                }
            }
        }
    }
    /// Prepare explicit maps from the same canonical bed conversion used by live sends.
    /// Takes source and destination layouts; returns bounded producer-owned channel maps for contribution rendering.
    pub(crate) fn maps_to(self, destination: Self) -> Vec<super::model::ChannelMap> {
        let mut maps = Vec::new();
        for source in 0..self.channels() {
            let mut frame = [0.; 8];
            frame[source] = 1.;
            let mut output = [0.; 8];
            self.mix_into(destination, &frame, &mut output);
            for (target, &gain) in output[..destination.channels()].iter().enumerate() {
                if gain != 0. {
                    maps.push(super::model::ChannelMap {
                        source: source as u8,
                        destination: target as u8,
                        gain,
                    });
                }
            }
        }
        maps
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
            if self
                .speaker_layout(Group::Output(bass.output))
                .is_none_or(|layout| !layout.speakers().contains(&Speaker::LowFrequency))
                || !bass.cutoff.is_finite()
                || !(20.0..=200.0).contains(&bass.cutoff)
                || !bass.level.is_finite()
                || !(0.0..=1.5).contains(&bass.level)
            {
                return Err("Monitor bass management requires a declared LFE output, 20–200 Hz cutoff and level0–1.5".into());
            }
        }
        Ok(())
    }
}
