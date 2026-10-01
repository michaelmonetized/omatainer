//! Instrument identities shared by the menu, commands, snapshots and voices.
//! Sample banks are a different source; they never alias a synth implementation.
use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SynthInstrument {
    Analog,
    Keys,
    Pad,
}

impl SynthInstrument {
    #[cfg(test)]
    pub const ALL: [Self; 3] = [Self::Analog, Self::Keys, Self::Pad];

    pub fn adsr(self) -> [f32; 4] {
        match self {
            Self::Analog => [0.005, 0.18, 0.35, 0.12],
            Self::Keys => [0.008, 0.22, 0.45, 0.28],
            Self::Pad => [0.04, 0.4, 0.7, 0.8],
        }
    }

    pub fn cutoff(self) -> f32 {
        match self {
            Self::Analog => 700.0,
            Self::Keys | Self::Pad => 1800.0,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SamplerInstrument {
    #[default]
    Samples,
    Synth(SynthInstrument),
}

impl SamplerInstrument {
    pub const ALL: [Self; 4] = [
        Self::Samples,
        Self::Synth(SynthInstrument::Analog),
        Self::Synth(SynthInstrument::Keys),
        Self::Synth(SynthInstrument::Pad),
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Samples => "samples",
            Self::Synth(SynthInstrument::Analog) => "analog",
            Self::Synth(SynthInstrument::Keys) => "keys",
            Self::Synth(SynthInstrument::Pad) => "pad",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::Samples => "One-shots from the selected Kit, Perc or Hits bank",
            Self::Synth(SynthInstrument::Analog) => {
                "Saw and square bass with a fast attack and short release"
            }
            Self::Synth(SynthInstrument::Keys) => {
                "Saw and sine keys with a fast attack and medium release"
            }
            Self::Synth(SynthInstrument::Pad) => {
                "Sine and detuned saw pad with a soft attack and long release"
            }
        }
    }

    pub fn synth(self) -> Option<SynthInstrument> {
        match self {
            Self::Samples => None,
            Self::Synth(kind) => Some(kind),
        }
    }
}
