//! Controller assignments and one-shot capture belong to MIDI dispatch workers.
use super::{Action, Binding, MidiMap, MsgKind, RelativeSpec};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

pub(crate) const MAX_MAPPINGS: usize = 256;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Endpoint {
    pub name: String,
    pub id: String,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Mapping {
    pub endpoint: Endpoint,
    pub binding: Binding,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Config {
    pub mappings: Vec<Mapping>,
}

/// List the actions with a usable wire encoding in the learn editor.
/// Takes no arguments; returns the implemented performance actions.
pub(crate) fn actions() -> &'static [Action] {
    &[
        Action::DeckPlay,
        Action::DeckCue,
        Action::DeckSync,
        Action::DeckJog,
        Action::DeckJogTouch,
        Action::DeckPitch,
        Action::DeckGain,
        Action::DeckEqHi,
        Action::DeckEqMid,
        Action::DeckEqLow,
        Action::DeckFilter,
        Action::DeckPfl,
        Action::DeckHotCue,
        Action::DeckLoop4,
        Action::DeckLoopIn,
        Action::DeckLoopOut,
        Action::DeckLoad,
        Action::DeckLoadLock,
        Action::DeckVinyl,
        Action::Xfader,
        Action::XfaderCurve,
        Action::Master,
        Action::CueMix,
        Action::Browse,
        Action::BrowseCrates,
        Action::CrateReturn,
        Action::SamplerSlotStop,
        Action::Prepare,
        Action::PrepareCrate,
        Action::LoadA,
        Action::LoadB,
        Action::Scene,
        Action::Clip,
        Action::TrackFader,
        Action::TrackMute,
        Action::Play,
        Action::Stop,
        Action::Record,
        Action::Tap,
        Action::Shift,
        Action::FxWet,
        Action::FxSelect,
    ]
}
/// Choose the required MIDI message class for a performance action.
/// Takes an action; returns Note, absolute CC, or explicitly decoded relative CC.
pub(crate) fn kind(action: Action) -> MsgKind {
    match action {
        Action::DeckJog | Action::Browse | Action::BrowseCrates => MsgKind::CcRel,
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
        | Action::FxWet => MsgKind::Cc,
        _ => MsgKind::Note,
    }
}
/// Compare message addresses independently of absolute or relative decoding.
/// Takes a message kind; returns its MIDI status family.
fn class(kind: MsgKind) -> u8 {
    match kind {
        MsgKind::Note => 0x90,
        MsgKind::Cc | MsgKind::CcRel => 0xb0,
        MsgKind::Pitch => 0xe0,
    }
}
/// Detect overlapping channel and controller addresses.
/// Takes two bindings; returns true when the same wire message can reach both.
fn address(first: &Binding, second: &Binding) -> bool {
    class(first.kind) == class(second.kind)
        && (first.ch == second.ch || first.ch == 0xff || second.ch == 0xff)
        && (first.kind == MsgKind::Pitch || first.data == second.data)
}
/// Match a complete message against a reviewed address.
/// Takes a binding and three wire bytes; returns whether it owns that message.
fn hit(binding: &Binding, msg: &[u8; 3]) -> bool {
    let message = if msg[0] & 0xf0 == 0x80 {
        0x90
    } else {
        msg[0] & 0xf0
    };
    class(binding.kind) == message
        && (binding.ch == msg[0] & 15 || binding.ch == 0xff)
        && (binding.kind == MsgKind::Pitch || binding.data == msg[1])
}
/// Validate a learned action before it can replace any input behavior.
/// Takes a binding; refuses invalid target slots or incompatible wire encoding.
pub(crate) fn validate_binding(binding: &Binding) -> Result<(), String> {
    if binding.ch >= 16 || !actions().contains(&binding.action) {
        return Err("Choose an exact channel and implemented action".into());
    }
    let expected = kind(binding.action);
    if binding.kind != expected
        && !(binding.action == Action::DeckPitch && binding.kind == MsgKind::Pitch)
    {
        return Err("Message type cannot operate this action".into());
    }
    if binding.action != Action::Clip && binding.deck >= 2 {
        return Err("Deck must be A or B".into());
    }
    let map = MidiMap {
        name: "Learned assignment".into(),
        matchers: vec![],
        bindings: vec![*binding],
        unmapped_notes: super::UnmappedNotes::Ignore,
    };
    map.validate().map_err(|error| error.to_string())
}
impl Config {
    /// Validate bounded exact-port mappings without applying them.
    /// Takes the draft configuration; returns an error for conflicts, unsafe targets or exceeded storage.
    pub(crate) fn validate(&self) -> Result<(), String> {
        if self.mappings.len() > MAX_MAPPINGS {
            return Err("Keep at most 256 learned assignments".into());
        }
        for (index, mapping) in self.mappings.iter().enumerate() {
            if [&mapping.endpoint.name, &mapping.endpoint.id]
                .iter()
                .any(|value| {
                    value.is_empty() || value.len() > 256 || value.chars().any(char::is_control)
                })
            {
                return Err("Device and exact backend port need 1–256 visible bytes".into());
            }
            validate_binding(&mapping.binding)?;
            if self.mappings[..index].iter().any(|other| {
                other.endpoint == mapping.endpoint && address(&other.binding, &mapping.binding)
            }) {
                return Err("Two learned assignments use the same device/channel/message".into());
            }
        }
        if serde_json::to_vec(self).map_err(|e| e.to_string())?.len() > 65_536 {
            return Err("Learned assignments exceed 64 KiB".into());
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Capture {
    pub mapping: Mapping,
    pub source: u64,
    pub bytes: [u8; 3],
    pub conflicts: Vec<Binding>,
}
#[derive(Clone, Debug)]
pub(crate) struct Device {
    pub source: u64,
    pub endpoint: Endpoint,
}
#[derive(Clone, Debug)]
pub(crate) struct View {
    pub revision: u64,
    pub config: Config,
    pub devices: Vec<Device>,
    pub armed: bool,
    pub capture: Option<Capture>,
    pub message: String,
}
struct Armed {
    binding: Binding,
    endpoint: Option<Endpoint>,
    deadline: Instant,
}
#[derive(Default)]
struct State {
    config: Config,
    devices: Vec<Device>,
    armed: Option<Armed>,
    capture: Option<Capture>,
    suppressed: Option<(u64, u8, u8)>,
    message: String,
}
pub(crate) struct Shared {
    pub revision: AtomicU64,
    pub ordered: AtomicBool,
    state: Mutex<State>,
}
impl Default for Shared {
    fn default() -> Self {
        Self {
            revision: AtomicU64::new(0),
            ordered: AtomicBool::new(false),
            state: Mutex::new(State::default()),
        }
    }
}
pub(super) enum Dispatch {
    Normal,
    Consume,
    Binding(Binding),
}
impl Shared {
    #[cfg(test)]
    pub(super) fn with_editor_lock_for_test(&self, work: impl FnOnce()) {
        let _held = self.state.lock();
        work();
    }

    fn advance(&self) -> Result<u64, String> {
        self.revision
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |r| r.checked_add(1))
            .map(|r| r + 1)
            .map_err(|_| "MIDI assignment revision exhausted".into())
    }
    /// Publish validated saved mappings outside the native callback.
    /// Takes the complete configuration; returns the new input revision or leaves the current one intact.
    pub(crate) fn configure(&self, config: Config) -> Result<u64, String> {
        config.validate()?;
        let mut state = self.state.lock();
        if state.config == config && state.armed.is_none() && state.capture.is_none() {
            return Ok(self.revision.load(Ordering::Acquire));
        }
        self.ordered.store(true, Ordering::Release);
        let revision = self.advance()?;
        state.suppressed = None;
        state.config = config;
        state.armed = None;
        state.capture = None;
        self.refresh_ordered(&state);
        state.message =
            "Learned assignments applied; built-in mappings handle other messages.".into();
        Ok(revision)
    }
    /// Arm one physical message for review, releasing this source's older gates at the worker boundary.
    /// Takes the desired binding and optional exact input; returns an error without changing input on invalid requests.
    pub(crate) fn begin(&self, binding: Binding, endpoint: Option<Endpoint>) -> Result<(), String> {
        validate_binding(&binding)?;
        let mut state = self.state.lock();
        if endpoint
            .as_ref()
            .is_some_and(|port| !state.devices.iter().any(|d| &d.endpoint == port))
        {
            return Err("Chosen MIDI device is disconnected".into());
        }
        self.ordered.store(true, Ordering::Release);
        self.advance()?;
        state.capture = None;
        state.suppressed = None;
        state.armed = Some(Armed {
            binding,
            endpoint,
            deadline: Instant::now() + Duration::from_secs(30),
        });
        state.message =
            "Move one compatible control. Capture ends after one message or 30 seconds.".into();
        Ok(())
    }
    /// Cancel capture without changing installed assignments.
    /// Takes no arguments; subsequent performance input returns to its existing mapping.
    pub(crate) fn cancel(&self) {
        let mut state = self.state.lock();
        if state.armed.is_some() || state.capture.is_some() {
            let _ = self.advance();
        }
        state.armed = None;
        state.capture = None;
        state.suppressed = None;
        state.message = "Learning cancelled; current assignments retained.".into();
        self.refresh_ordered(&state);
    }
    /// End capture when the editor closes without cloning its mapping storage.
    /// Takes no arguments; current assignments remain active and later performance gestures are admitted.
    pub(crate) fn close(&self) {
        let mut state = self.state.lock();
        if state.armed.is_some() || state.capture.is_some() {
            let _ = self.advance();
            state.suppressed = None;
            state.armed = None;
            state.capture = None;
            state.message = "Learning closed; current assignments retained.".into();
            self.refresh_ordered(&state);
        }
    }
    /// Read the bounded editor state and expire an idle capture.
    /// Takes no arguments; returns current connected ports, assignments and review state.
    pub(crate) fn view(&self) -> View {
        let mut state = self.state.lock();
        Self::expire(&mut state);
        self.refresh_ordered(&state);
        View {
            revision: self.revision.load(Ordering::Acquire),
            config: state.config.clone(),
            devices: state.devices.clone(),
            armed: state.armed.is_some(),
            capture: state.capture.clone(),
            message: state.message.clone(),
        }
    }
    fn refresh_ordered(&self, state: &State) {
        self.ordered.store(
            !state.config.mappings.is_empty()
                || state.armed.is_some()
                || state.capture.is_some()
                || state.suppressed.is_some(),
            Ordering::Release,
        );
    }
    fn expire(state: &mut State) {
        if state
            .armed
            .as_ref()
            .is_some_and(|armed| Instant::now() >= armed.deadline)
        {
            state.armed = None;
            state.message = "Capture timed out; performance input remains available.".into();
        }
    }
    /// Register an opened input with the editor.
    /// Takes its source, device name and port id; retains at most 256 bounded endpoint identities.
    pub(super) fn connected(&self, source: u64, name: &str, id: &str) {
        let mut state = self.state.lock();
        if state.devices.len() < 256 && name.len() <= 256 && id.len() <= 256 {
            state.devices.push(Device {
                source,
                endpoint: Endpoint {
                    name: name.into(),
                    id: id.into(),
                },
            });
        }
    }
    /// Retire an input and its uncommitted capture.
    /// Takes the source id; retains installed mappings for an explicit exact-port reconnect.
    pub(super) fn disconnected(&self, source: u64) {
        let mut state = self.state.lock();
        let endpoint = state
            .devices
            .iter()
            .find(|d| d.source == source)
            .map(|d| d.endpoint.clone());
        state.devices.retain(|d| d.source != source);
        if state
            .suppressed
            .is_some_and(|(owner, _, _)| owner == source)
        {
            state.suppressed = None;
        }
        if state
            .capture
            .as_ref()
            .is_some_and(|capture| capture.source == source)
            || state
                .armed
                .as_ref()
                .is_some_and(|armed| armed.endpoint == endpoint && endpoint.is_some())
        {
            state.armed = None;
            state.capture = None;
            state.message =
                "MIDI device disconnected; capture cancelled and assignments retained.".into();
        }
        self.refresh_ordered(&state);
    }
    /// Resolve one ordered message on its dispatch worker.
    /// Takes its exact device, wire bytes and built-in map; returns normal dispatch, reviewed override or capture consumption.
    pub(super) fn input(
        &self,
        source: u64,
        name: &str,
        id: &str,
        msg: &[u8; 3],
        map: &MidiMap,
    ) -> Dispatch {
        self.input_at(
            source,
            name,
            id,
            msg,
            map,
            self.revision.load(Ordering::Acquire),
        )
    }
    pub(super) fn input_at(
        &self,
        source: u64,
        name: &str,
        id: &str,
        msg: &[u8; 3],
        map: &MidiMap,
        revision: u64,
    ) -> Dispatch {
        let mut state = self.state.lock();
        if self.revision.load(Ordering::Acquire) != revision {
            return Dispatch::Consume;
        }
        Self::expire(&mut state);
        self.refresh_ordered(&state);
        let channel = msg[0] & 15;
        let status = msg[0] & 0xf0;
        if state.suppressed == Some((source, channel, msg[1])) {
            if status == 0x80 || (status == 0x90 && msg[2] == 0) {
                state.suppressed = None;
                self.refresh_ordered(&state);
                return Dispatch::Consume;
            }
            if status == 0x90 {
                return Dispatch::Consume;
            }
        }
        if state.armed.is_none() && state.suppressed.is_none() && state.config.mappings.is_empty() {
            return Dispatch::Normal;
        }
        let endpoint = Endpoint {
            name: name.into(),
            id: id.into(),
        };
        let capture = state.armed.as_ref().is_some_and(|armed| {
            armed.endpoint.as_ref().is_none_or(|port| port == &endpoint)
                && class(armed.binding.kind) == status
                && (status != 0x90 || msg[2] != 0)
                && state.devices.iter().any(|device| device.source == source)
        });
        if capture {
            let mut binding = state.armed.take().unwrap().binding;
            binding.ch = channel;
            binding.data = if status == 0xe0 { 0 } else { msg[1] };
            if state
                .devices
                .iter()
                .filter(|device| device.endpoint == endpoint)
                .count()
                != 1
            {
                state.message =
                    "Ambiguous backend port; capture from a uniquely identified device.".into();
                return Dispatch::Normal;
            }
            let conflicts = map
                .bindings
                .iter()
                .copied()
                .filter(|other| address(other, &binding))
                .chain(
                    state
                        .config
                        .mappings
                        .iter()
                        .filter(|m| m.endpoint == endpoint && address(&m.binding, &binding))
                        .map(|m| m.binding),
                )
                .collect();
            if status == 0x90 {
                state.suppressed = Some((source, channel, msg[1]));
            }
            state.capture = Some(Capture {
                mapping: Mapping { endpoint, binding },
                source,
                bytes: *msg,
                conflicts,
            });
            state.message="Message captured. Review its exact port, channel, address and action before assignment.".into();
            return Dispatch::Consume;
        }
        if let Some(mapping) = state
            .config
            .mappings
            .iter()
            .find(|m| m.endpoint == endpoint && hit(&m.binding, msg))
        {
            if state
                .devices
                .iter()
                .filter(|d| d.endpoint == endpoint)
                .count()
                != 1
            {
                state.message =
                    "Ambiguous MIDI input; learned address held until the exact port is unique."
                        .into();
                return Dispatch::Consume;
            }
            Dispatch::Binding(mapping.binding)
        } else {
            Dispatch::Normal
        }
    }
    /// Assign only the still-connected reviewed capture to a bounded configuration.
    /// Takes the capture revision and explicit replacement choice; returns the new complete draft or refuses stale/conflicting work.
    pub(crate) fn assign(&self, revision: u64, replace: bool) -> Result<Config, String> {
        let state = self.state.lock();
        if self.revision.load(Ordering::Acquire) != revision {
            return Err("Assignments changed; capture again".into());
        }
        let captured = state.capture.as_ref().ok_or("Capture one message first")?;
        if !state
            .devices
            .iter()
            .any(|d| d.source == captured.source && d.endpoint == captured.mapping.endpoint)
        {
            return Err("Captured device disconnected".into());
        }
        if state
            .devices
            .iter()
            .filter(|d| d.endpoint == captured.mapping.endpoint)
            .count()
            != 1
        {
            return Err("Captured MIDI port is ambiguous".into());
        }
        if !replace && !captured.conflicts.is_empty() {
            return Err("Address already assigned; review Replace assignment".into());
        }
        let mut config = state.config.clone();
        if replace {
            config.mappings.retain(|m| {
                m.endpoint != captured.mapping.endpoint
                    || !address(&m.binding, &captured.mapping.binding)
            });
        }
        config.mappings.push(captured.mapping.clone());
        config.validate()?;
        Ok(config)
    }
}

#[cfg(test)]
mod tests;
