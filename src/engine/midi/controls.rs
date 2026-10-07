use super::{Action, Binding, MsgKind};
use crate::engine::{Command, RtEngine};
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

/// Keep direction and parameter limits with one MIDI assignment.
/// Takes a normalized parameter interval and direction; transforms complete absolute values without changing other assignments.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Spec {
    pub invert: bool,
    pub min: f32,
    pub max: f32,
}
impl Default for Spec {
    fn default() -> Self {
        Self {
            invert: false,
            min: 0.0,
            max: 1.0,
        }
    }
}
impl Spec {
    pub(crate) fn valid(self, action: Action) -> bool {
        let max = match action {
            Action::DeckGain | Action::TrackFader | Action::Master => 1.5,
            _ => 1.0,
        };
        self.min.is_finite()
            && self.max.is_finite()
            && self.min >= 0.0
            && self.min < self.max
            && self.max <= max
            && (continuous(action)
                || matches!(
                    action,
                    Action::DeckJog | Action::Browse | Action::BrowseCrates
                ) && self.min == 0.0
                    && self.max == 1.0)
    }
    pub(super) fn absolute(self, value: f32) -> f32 {
        let value = if self.invert { 1.0 - value } else { value };
        self.min + value.clamp(0.0, 1.0) * (self.max - self.min)
    }
    pub(super) fn direction(self, value: f32) -> f32 {
        if self.invert {
            -value
        } else {
            value
        }
    }
}

/// List parameter actions that can use CC, CC pairs, bend or relative encoders.
/// Takes an action; returns whether it has a finite renderer-owned scalar value.
pub(crate) fn continuous(action: Action) -> bool {
    matches!(
        action,
        Action::DeckPitch
            | Action::DeckGain
            | Action::DeckEqHi
            | Action::DeckEqMid
            | Action::DeckEqLow
            | Action::DeckFilter
            | Action::Xfader
            | Action::XfaderCurve
            | Action::Master
            | Action::CueMix
            | Action::TrackFader
            | Action::TrackPan
            | Action::TrackSendA
            | Action::TrackSendB
            | Action::FxWet
    )
}

/// Identify assignments that require the current encoder format.
/// Takes a decoded binding; returns whether an older preset cannot represent it.
pub(crate) fn modern(binding: Binding) -> bool {
    binding.controls.is_some()
        || binding.pair_order.is_some()
        || binding.kind == MsgKind::Cc14
        || binding
            .relative
            .is_some_and(|spec| spec.encoding == super::RelativeEncoding::SignedBit)
        || binding.kind == MsgKind::CcRel && continuous(binding.action)
        || binding.kind == MsgKind::Pitch && binding.action != Action::DeckPitch
}

/// Reject newer fields before decoding an older saved format.
/// Takes one JSON binding; returns whether it carries new metadata, including an explicit null field.
pub(crate) fn new_fields(value: &serde_json::Value) -> bool {
    value.get("controls").is_some()
        || value.get("pair_order").is_some()
        || serde_json::from_value::<Binding>(value.clone()).is_ok_and(modern)
}

/// Retain an encoder change until the renderer reads the current parameter.
/// Takes one validated binding and signed delta; applies limits after accumulation so GUI edits and other controllers remain authoritative.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Adjust {
    pub binding: Binding,
    pub delta: f32,
}
impl Adjust {
    pub(crate) fn valid(self) -> bool {
        (self.binding.ch < 16 || self.binding.ch == 0xff)
            && self.binding.data < 128
            && self.binding.kind == MsgKind::CcRel
            && self.binding.relative.is_some_and(|spec| spec.is_valid())
            && self.delta.is_finite()
            && continuous(self.binding.action)
            && self
                .binding
                .controls
                .unwrap_or_default()
                .valid(self.binding.action)
            && match self.binding.action {
                Action::TrackFader | Action::TrackPan | Action::TrackSendA | Action::TrackSendB => {
                    usize::from(self.binding.extra) < crate::engine::session::MAX_TRACKS
                }
                Action::FxWet => self.binding.extra < 3,
                _ => self.binding.deck < crate::engine::DECKS as u8,
            }
    }
    pub(crate) fn track(self) -> Option<usize> {
        matches!(
            self.binding.action,
            Action::TrackFader | Action::TrackPan | Action::TrackSendA | Action::TrackSendB
        )
        .then_some(usize::from(self.binding.extra))
    }
    pub(crate) fn command(self, rt: &RtEngine) -> Option<Command> {
        if !self.valid() {
            return None;
        }
        let binding = self.binding;
        let deck = usize::from(binding.deck);
        let track = usize::from(binding.extra);
        let current = match binding.action {
            Action::DeckPitch => rt.decks[deck].pitch,
            Action::DeckGain => rt.decks[deck].eq_store[3],
            Action::DeckEqHi | Action::DeckEqMid | Action::DeckEqLow => {
                let band = match binding.action {
                    Action::DeckEqHi => 2,
                    Action::DeckEqMid => 1,
                    _ => 0,
                };
                let gain = rt.decks[deck].eq_store[band];
                if gain <= 1.0 {
                    gain.max(0.0).powf(1.0 / 1.4) * 0.5
                } else {
                    0.5 + (gain - 1.0) / 2.4
                }
            }
            Action::DeckFilter => rt.decks[deck].filter_amt,
            Action::Xfader => rt.xfader,
            Action::XfaderCurve => rt.xfader_curve,
            Action::Master => rt.master,
            Action::CueMix => rt.cue_mix,
            Action::TrackFader => rt.tracks.get(track)?.gain,
            Action::TrackPan => (rt.tracks.get(track)?.pan + 1.0) * 0.5,
            Action::TrackSendA | Action::TrackSendB => {
                rt.surface.sends.get(track)?[usize::from(binding.action == Action::TrackSendB)]
            }
            Action::FxWet => rt.fx_wet[track],
            _ => return None,
        };
        let spec = binding.controls.unwrap_or_default();
        let value = (current + self.delta).clamp(spec.min, spec.max);
        Some(match binding.action {
            Action::DeckPitch => Command::DeckPitch {
                deck: binding.deck,
                value,
            },
            Action::DeckGain => Command::DeckGain {
                deck: binding.deck,
                value,
            },
            Action::DeckEqHi | Action::DeckEqMid | Action::DeckEqLow => Command::DeckEq {
                deck: binding.deck,
                band: match binding.action {
                    Action::DeckEqHi => 2,
                    Action::DeckEqMid => 1,
                    _ => 0,
                },
                value,
            },
            Action::DeckFilter => Command::DeckFilter {
                deck: binding.deck,
                value,
            },
            Action::Xfader => Command::Xfader(value),
            Action::XfaderCurve => Command::XfaderCurve(value),
            Action::Master => Command::Master(value),
            Action::CueMix => Command::CueMix(value),
            Action::TrackFader => Command::TrackGain {
                track: binding.extra as u8,
                value,
            },
            Action::TrackPan => Command::TrackPan {
                track: binding.extra as u8,
                value,
            },
            Action::TrackSendA | Action::TrackSendB => {
                Command::Surface(crate::engine::surface_controls::Input::TrackSend {
                    track: binding.extra as u8,
                    send: u8::from(binding.action == Action::TrackSendB),
                    value,
                })
            }
            Action::FxWet => Command::FxWet {
                slot: binding.extra as u8,
                value,
            },
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, Default)]
struct Pair {
    bytes: [u8; 2],
    seen: u8,
    at: Option<Instant>,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum PairOrder {
    #[default]
    MsbFirst,
    LsbFirst,
}
#[derive(Clone, Copy, Default)]
struct Reading {
    value: u16,
    complete: bool,
}
#[derive(Clone, Copy, Default)]
pub(super) struct PairValues {
    standard: Option<Reading>,
    reversed: Option<u16>,
}
impl PairValues {
    /// Select the configured byte order.
    /// Takes optional per-binding order; returns its current fourteen-bit value, using MIDI standard order by default.
    pub(super) fn value(self, order: Option<PairOrder>) -> Option<u16> {
        match order.unwrap_or_default() {
            PairOrder::MsbFirst => self.standard.map(|reading| reading.value),
            PairOrder::LsbFirst => self.reversed,
        }
    }
    /// Keep capture waiting for a fresh complete gesture.
    /// Takes optional per-binding order; returns whether both halves arrived for that order.
    pub(super) fn complete(self, order: Option<PairOrder>) -> bool {
        match order.unwrap_or_default() {
            PairOrder::MsbFirst => self.standard.is_some_and(|reading| reading.complete),
            PairOrder::LsbFirst => self.reversed.is_some(),
        }
    }
}
#[derive(Clone, Copy, Default)]
struct Controller {
    standard: Pair,
    reversed: Pair,
}
pub(super) struct Pairs {
    channels: [[Controller; 32]; 16],
}
impl Default for Pairs {
    fn default() -> Self {
        Self {
            channels: [[Controller::default(); 32]; 16],
        }
    }
}
impl Pairs {
    pub(super) fn clear(&mut self) {
        self.channels = [[Controller::default(); 32]; 16];
    }
    /// Interpret standard controls and explicitly reversed paired controls.
    /// Takes ordered bytes and callback time; returns coarse/fine MIDI state and a separately assembled reverse-order pair, with fresh capture flags.
    pub(super) fn input(&mut self, message: &[u8; 3], at: Instant) -> PairValues {
        if message[0] & 0xf0 == 0xb0 && message[1] == 121 {
            self.channels[usize::from(message[0] & 15)] = [Controller::default(); 32];
        }
        if message[0] & 0xf0 != 0xb0 || message[1] >= 64 || message[2] >= 128 {
            return PairValues::default();
        }
        let part = usize::from(message[1] >= 32);
        let controller =
            &mut self.channels[usize::from(message[0] & 15)][usize::from(message[1] % 32)];
        let standard = &mut controller.standard;
        let mut values = PairValues::default();
        if part == 0 {
            standard.bytes = [message[2], 0];
            standard.seen = 1;
            standard.at = Some(at);
            values.standard = Some(Reading {
                value: u16::from(message[2]) << 7,
                complete: false,
            });
        } else if standard.seen != 0 {
            standard.bytes[1] = message[2];
            let complete = standard.seen == 3
                || standard.at.is_some_and(|previous| {
                    at.saturating_duration_since(previous) <= Duration::from_secs(1)
                });
            if complete {
                standard.seen = 3;
            }
            values.standard = Some(Reading {
                value: u16::from(standard.bytes[0]) << 7 | u16::from(message[2]),
                complete,
            });
        }
        let pair = &mut controller.reversed;
        if pair
            .at
            .is_some_and(|previous| at.saturating_duration_since(previous) > Duration::from_secs(1))
        {
            pair.seen = 0;
        }
        if part == 1 {
            pair.bytes[1] = message[2];
            pair.seen = 2;
            pair.at = Some(at);
        } else if pair.seen == 2 {
            values.reversed = Some(u16::from(message[2]) << 7 | u16::from(pair.bytes[1]));
            pair.seen = 0;
            pair.at = None;
        }
        values
    }
}

#[cfg(test)]
mod tests;
