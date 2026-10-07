use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Mode {
    HotCue,
    Roll,
    Slice,
    Sampler,
    SavedLoop,
    AutoLoop,
    ManualLoop,
    VelocitySampler,
}
impl Mode {
    pub(crate) const ALL: [Self; 8] = [
        Self::HotCue,
        Self::Roll,
        Self::Slice,
        Self::Sampler,
        Self::SavedLoop,
        Self::AutoLoop,
        Self::ManualLoop,
        Self::VelocitySampler,
    ];
    /// Resolve a stable mode number.
    /// Takes a zero-based mode index; returns the existing supported function or refuses an unknown mode.
    pub(crate) fn from_index(index: u8) -> Option<Self> {
        Self::ALL.get(usize::from(index)).copied()
    }
    /// Keep manufacturer and native mode identities consistent.
    /// Takes this mode; returns its existing zero-based controller index.
    pub(crate) fn index(self) -> u8 {
        match self {
            Self::HotCue => 0,
            Self::Roll => 1,
            Self::Slice => 2,
            Self::Sampler => 3,
            Self::SavedLoop => 4,
            Self::AutoLoop => 5,
            Self::ManualLoop => 6,
            Self::VelocitySampler => 7,
        }
    }
    /// Describe the actual supported pad function.
    /// Takes this mode; returns its native chooser label.
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::HotCue => "Hot Cue",
            Self::Roll => "Roll",
            Self::Slice => "Slice",
            Self::Sampler => "Sampler",
            Self::SavedLoop => "Saved Loop",
            Self::AutoLoop => "Auto Loop",
            Self::ManualLoop => "Manual Loop",
            Self::VelocitySampler => "Velocity Sampler",
        }
    }
    /// Show independent mode colors before a named slot supplies its own color.
    /// Takes this mode; returns one stable RGB color used by native and controller profile descriptions.
    pub(crate) fn color(self) -> [u8; 3] {
        match self {
            Self::HotCue => [255, 90, 80],
            Self::Roll => [255, 160, 40],
            Self::Slice => [230, 210, 40],
            Self::Sampler => [70, 210, 100],
            Self::SavedLoop => [40, 205, 205],
            Self::AutoLoop => [60, 145, 255],
            Self::ManualLoop => [165, 95, 255],
            Self::VelocitySampler => [235, 95, 200],
        }
    }
}
#[derive(Clone, Copy, Debug)]
pub(crate) struct Press {
    pub source: u64,
    pub key: u32,
    pub deck: u8,
    pub id: u8,
    pub mode: Option<Mode>,
    pub pressure: f32,
    pub shifted: bool,
}
#[derive(Clone, Copy, Debug)]
pub(crate) struct Release {
    pub source: u64,
    pub key: u32,
}
impl Press {
    /// Admit only one supported physical or native pad.
    /// Takes this onset; returns whether deck, fixed ID and normalized velocity fit the implemented surface.
    pub(crate) fn valid(self) -> bool {
        self.deck < 2
            && (1..=8).contains(&self.id)
            && self.pressure.is_finite()
            && (0.0..=1.0).contains(&self.pressure)
    }
}
/// Retire the original learned pad without consulting a changed profile.
/// Takes complete wire bytes and source; returns a stable release key only for note-off or zero-velocity Note On.
pub(crate) fn wire_release(message: &[u8], source: u64) -> Option<Release> {
    (message.len() == 3
        && message[1] < 128
        && message[2] < 128
        && (message[0] & 0xf0 == 0x80 || message[0] & 0xf0 == 0x90 && message[2] == 0))
        .then(|| Release {
            source,
            key: wire_key(message[0] & 15, message[1]),
        })
}
/// Qualify one physical address independently of deck and mode assignments.
/// Takes exact MIDI channel and note; returns the stable ordinary pad key.
pub(crate) fn wire_key(channel: u8, note: u8) -> u32 {
    u32::from(channel) * 128 + u32::from(note)
}
/// Normalize the SP1's mode and layer addresses to the original physical pad.
/// Takes its manufacturer pad channel and address; returns a disjoint stable left/right pad key.
pub(crate) fn sp1_key(channel: u8, address: u8) -> u32 {
    0x8000_0000 | u32::from((channel - 7) % 2) * 8 | u32::from(address % 8)
}

mod runtime;
pub(in crate::engine) use runtime::State;

#[cfg(test)]
mod tests;
