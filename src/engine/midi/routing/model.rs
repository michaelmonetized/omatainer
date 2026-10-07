//! Original bounded studio/performance routing preferences. Endpoint names are
//! exact, never substring matches; an optional backend id disambiguates ports.
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Endpoint {
    pub name: String,
    pub id: Option<String>,
}
impl Endpoint {
    pub fn matches(&self, name: &str, id: &str) -> bool {
        self.name == name && self.id.as_ref().is_none_or(|wanted| wanted == id)
    }
    pub(crate) fn validate(&self) -> Result<(), String> {
        fn text(s: &str) -> bool {
            !s.trim().is_empty() && s.len() <= 1024 && !s.contains(['\0', '\r', '\n'])
        }
        if !text(&self.name) || self.id.as_deref().is_some_and(|id| !text(id)) {
            return Err(
                "MIDI endpoint names/ids must be 1–1024 bytes without line breaks or NUL".into(),
            );
        }
        Ok(())
    }
    /// Identify an incoming port on the same backend device.
    /// Takes its borrowed name and backend ID; returns whether it could echo this destination's transport without allocating.
    pub(crate) fn conflicts_port(&self, name: &str, id: &str) -> bool {
        self.name == name || self.id.as_deref().is_some_and(|own| own == id || alsa_client(own).zip(alsa_client(id)).is_some_and(|(a,b)|a==b))
    }
    pub(crate) fn conflicts(&self, other: &Self) -> bool {
        if self.name == other.name {
            return true;
        }
        match (&self.id, &other.id) {
            (Some(a), Some(b)) => {
                a == b
                    || alsa_client(a)
                        .zip(alsa_client(b))
                        .is_some_and(|(a, b)| a == b)
            }
            _ => false,
        }
    }
}
fn alsa_client(id: &str) -> Option<u32> {
    let (client, port) = id.split_once(':')?;
    port.parse::<u32>().ok()?;
    client.parse().ok()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Filter {
    pub notes: bool,
    pub cc: bool,
    pub bank: bool,
    pub program: bool,
    pub pressure: bool,
    pub bend: bool,
    pub sysex: bool,
}
impl Default for Filter {
    fn default() -> Self {
        Self {
            notes: true,
            cc: true,
            bank: true,
            program: true,
            pressure: true,
            bend: true,
            sysex: false,
        }
    }
}
impl Filter {
    pub fn accepts(&self, bytes: &[u8]) -> bool {
        match bytes.first().copied().unwrap_or(0) & 0xf0 {
            0x80 | 0x90 => self.notes,
            0xa0 | 0xd0 => self.pressure,
            0xb0 if matches!(bytes.get(1), Some(0 | 32)) => self.bank,
            0xb0 => self.cc,
            0xc0 => self.program,
            0xe0 => self.bend,
            0xf0 => bytes.first() == Some(&0xf0) && self.sysex,
            _ => false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Input {
    pub port: Endpoint,
    /// One bit per wire channel (bit0 is displayed channel1).
    pub channels: u16,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Route {
    pub track: u8,
    pub inputs: Vec<Input>,
    pub output: Option<Endpoint>,
    /// None preserves each source message's channel; Some uses wire0–15.
    pub output_channel: Option<u8>,
    pub monitor: bool,
    pub thru: bool,
    pub filter: Filter,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Routing {
    /// False keeps the established controller maps and selected-track keyboard.
    /// True with no routes disables generic keyboard destinations explicitly.
    pub enabled: bool,
    pub routes: Vec<Route>,
}
impl Routing {
    pub fn validate(&self) -> Result<(), String> {
        if self.routes.len() > crate::engine::session::MAX_TRACKS {
            return Err("Use at most one MIDI route for each of the 128 tracks".into());
        }
        let mut tracks = 0u128;
        for route in &self.routes {
            if usize::from(route.track) >= crate::engine::session::MAX_TRACKS || tracks & (1 << route.track) != 0
            {
                return Err("MIDI route track must be unique and displayed as1–128".into());
            }
            tracks |= 1 << route.track;
            if route.inputs.len() > 8 || route.output_channel.is_some_and(|ch| ch > 15) {
                return Err(
                    "Use at most eight input ports per track and output channel1–16 or Preserve"
                        .into(),
                );
            }
            if route.thru && route.output.is_none() {
                return Err("Live MIDI thru requires an explicit output port".into());
            }
            for (index, input) in route.inputs.iter().enumerate() {
                input.port.validate()?;
                if input.channels == 0 {
                    return Err("Select at least one MIDI input channel".into());
                }
                if route.inputs[..index].iter().any(|old| {
                    old.port.name == input.port.name
                        && (old.port.id.is_none()
                            || input.port.id.is_none()
                            || old.port.id == input.port.id)
                        && old.channels & input.channels != 0
                }) {
                    return Err("Overlapping input endpoint/channel selections would duplicate one track's events".into());
                }
            }
            if let Some(output) = &route.output {
                output.validate()?;
                if self
                    .routes
                    .iter()
                    .flat_map(|r| &r.inputs)
                    .any(|i| output.conflicts(&i.port))
                {
                    return Err("Feedback guard: a configured input device/port cannot also be an output destination".into());
                }
            }
        }
        Ok(())
    }
    pub fn output_mask(&self) -> u128 {
        if !self.enabled {
            return 0;
        }
        self.routes
            .iter()
            .filter(|r| r.output.is_some())
            .fold(0, |mask, r| mask | (1 << r.track))
    }
}
