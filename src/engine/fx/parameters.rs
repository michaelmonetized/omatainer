//! UI ranges describe the DSP's actual mapping from stored normalized values.
use super::FxId;

#[derive(Clone, Copy, Debug)]
pub struct Control {
    /// None is the common wet/dry interpolation, Some is a consumed p[] index.
    pub parameter: Option<u8>,
    pub name: &'static str,
    pub unit: &'static str,
    pub min: f32,
    pub max: f32,
    pub consumer: &'static str,
    pub help: &'static str,
}
impl Control {
    pub fn display(self, normalized: f32) -> f32 {
        self.min + normalized.clamp(0.0, 1.0) * (self.max - self.min)
    }
    pub fn normalized(self, display: f32) -> f32 {
        ((display - self.min) / (self.max - self.min)).clamp(0.0, 1.0)
    }
    pub fn label(self) -> String {
        format!("{} ({})", self.name, self.unit)
    }
}
const WET: Control = Control {
    parameter: None,
    name: "Wet",
    unit: "%",
    min: 0.0,
    max: 100.0,
    consumer: "FxSlot::process_enabled final linear dry/wet interpolation",
    help: "0% original signal; 100% processed signal.",
};
const fn p(
    index: u8,
    name: &'static str,
    unit: &'static str,
    min: f32,
    max: f32,
    consumer: &'static str,
    help: &'static str,
) -> Control {
    Control {
        parameter: Some(index),
        name,
        unit,
        min,
        max,
        consumer,
        help,
    }
}
const LOW: Control = p(
    0,
    "Low gain",
    "×",
    0.25,
    1.75,
    "eq_n lower-band gain interpolation",
    "Linear gain: 1× is neutral. Shapes the lower part of this EQ's bands.",
);
const MID: Control = p(
    1,
    "Mid gain",
    "×",
    0.25,
    1.75,
    "eq_n middle-band gain interpolation",
    "Linear gain: 1× is neutral. Shapes the middle of this EQ's bands.",
);
const HIGH: Control = p(
    2,
    "High gain",
    "×",
    0.25,
    1.75,
    "eq_n upper-band gain interpolation",
    "Linear gain: 1× is neutral. Shapes the upper part of this EQ's bands.",
);
const COMP: &[Control] = &[
    WET,
    p(
        0,
        "Threshold",
        "% peak",
        5.0,
        45.0,
        "Envelope compressor threshold = 0.05 + p0*0.4",
        "Linked stereo peak-envelope threshold.",
    ),
    p(
        1,
        "Compression",
        "%",
        0.0,
        100.0,
        "Envelope compressor gain blends unity and threshold reduction by p1",
        "0% unity gain; 100% full threshold reduction. This is not a ratio control.",
    ),
];
const SPREAD: &[Control] = &[
    WET,
    p(
        0,
        "Width",
        "%",
        0.0,
        200.0,
        "Spread delay time and mono/stereo interpolation from p0",
        "0% mono; 100% original stereo; 200% right-channel Haas spread up to 6 ms.",
    ),
];
const BALANCE: &[Control] = &[
    WET,
    p(
        0,
        "Balance",
        "%",
        -100.0,
        100.0,
        "Balance square-root channel attenuation from p0",
        "-100% left only; 0% centered; +100% right only.",
    ),
];
const DELAY: &[Control] = &[
    WET,
    p(
        1,
        "Feedback",
        "%",
        0.0,
        100.0,
        "Delay::fb = p1",
        "Amount returned to each delay line. 100% repeats without decay; time is fixed at 250 ms.",
    ),
];
const GATE: &[Control] = &[
    WET,
    p(
        0,
        "Threshold",
        "% peak",
        0.0,
        100.0,
        "Envelope gate compares linked peak envelope against p0",
        "Below the threshold the signal is reduced to 5%; detector timing is fixed.",
    ),
];
const DIST: &[Control] = &[
    WET,
    p(
        0,
        "Drive",
        "×",
        1.0,
        9.0,
        "tanh(input * (1 + p0*8))",
        "Input gain before the saturator.",
    ),
];
const FILTER: &[Control] = &[
    WET,
    p(
        0,
        "Mode",
        "LP/BP/HP",
        0.0,
        2.0,
        "Svf::process morph = p0",
        "0 = low-pass; 1 = band-pass; 2 = high-pass. Intermediate values blend adjacent modes.",
    ),
    p(
        1,
        "Cutoff",
        "Hz",
        120.0,
        8120.0,
        "Svf::process cutoff = 120 + p1*8000",
        "Filter cutoff frequency; resonance is fixed.",
    ),
];

impl FxId {
    pub fn from_name(name: &str) -> Option<Self> {
        Self::all().iter().copied().find(|id| id.name() == name)
    }
    pub fn supports_scene(self) -> bool {
        self != Self::Arp
    }
    pub fn controls(self) -> &'static [Control] {
        match self {
            Self::Comp => COMP,
            Self::Spread => SPREAD,
            Self::Balance => BALANCE,
            Self::Reverb | Self::Chorus => &[WET],
            Self::Delay => DELAY,
            Self::Gate => GATE,
            Self::Arp | Self::Unavailable => &[],
            Self::Dist => DIST,
            Self::Filter => FILTER,
            Self::Eq3 | Self::Eq5 | Self::Eq8 => &[WET, LOW, MID, HIGH],
        }
    }
    pub fn fixed_settings(self) -> Option<&'static str> {
        match self {
            Self::Delay => Some("Time fixed at 250 ms"),
            Self::Reverb => Some("Fixed room network and decay"),
            Self::Chorus => Some("Fixed rate 0.7 Hz · delay 2–14 ms"),
            Self::Arp => Some("MIDI tracks only · ascending 1/16 steps · on/off; no wet mix"),
            _ => None,
        }
    }
}
