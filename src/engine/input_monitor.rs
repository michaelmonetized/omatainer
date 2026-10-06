//! Explicit track input and audio-clip precedence.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    In,
    #[default]
    Auto,
    Off,
}

impl Mode {
    /// Choose the audible source.
    /// Takes arm, sounding-clip and recording states; returns live-input and audio-clip gains.
    pub fn gains(self, armed: bool, clip: bool, recording: bool) -> [f32; 2] {
        let live = match self {
            Self::In => true,
            Self::Auto => armed && (!clip || recording),
            Self::Off => false,
        };
        if live {
            [1.0, 0.0]
        } else {
            [0.0, 1.0]
        }
    }

    /// Name a monitoring choice.
    /// Takes a mode; returns its native label.
    pub fn label(self) -> &'static str {
        match self {
            Self::In => "In",
            Self::Auto => "Auto",
            Self::Off => "Off",
        }
    }
}

impl super::TrackRt {
    /// Resolve retained input precedence.
    /// Takes sounding-clip and recording states; returns the chosen gains, retaining additive playback for migrated tracks.
    pub(super) fn input_gains(&self, clip: bool, recording: bool) -> [f32; 2] {
        self.input_monitor
            .map_or([1.0; 2], |mode| mode.gains(self.armed, clip, recording))
    }

    /// Observe the current monitor request.
    /// Takes recording state; returns whether the selected policy requests routed input, independently of device availability.
    pub(super) fn input_enabled(&self, recording: bool) -> bool {
        self.input_gains(
            self.playing.is_some_and(|clip| clip.last_beat >= 0.0),
            recording,
        )[0] > 0.0
    }
}

pub(super) fn identity(input: f32, clip: f32) -> [f32; 2] {
    [input, clip]
}

#[cfg(test)]
mod tests;
