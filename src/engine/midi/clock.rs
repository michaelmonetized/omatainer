use super::routing::Endpoint;
use serde::{Deserialize, Serialize};
use std::{
    sync::{
        atomic::{AtomicBool, AtomicI32, AtomicU64, AtomicU8, Ordering::*},
        Arc,
    },
    time::{Duration, Instant},
};

mod output;
mod runtime;
pub(crate) use output::{Manager, Status};
pub(crate) use runtime::Runtime;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Config {
    pub enabled: bool,
    pub ports: Vec<Endpoint>,
    pub compensation_ms: i32,
}
impl Config {
    /// Validate explicitly selected clock destinations and timing compensation.
    /// Takes the saved or edited clock policy; returns a refusal for empty enabled output, duplicate ports or excessive compensation.
    pub(crate) fn validate(&self) -> Result<(), String> {
        if self.ports.len() > 8
            || self.enabled && self.ports.is_empty()
            || !(-500..=500).contains(&self.compensation_ms)
        {
            return Err("MIDI clock needs 1–8 selected outputs when enabled and compensation from −500 to +500 ms".into());
        }
        for (index, port) in self.ports.iter().enumerate() {
            port.validate()?;
            if self.ports[..index].iter().any(|other| {
                other == port
                    || other
                        .id
                        .as_ref()
                        .zip(port.id.as_ref())
                        .is_some_and(|(a, b)| a == b)
                    || other.name == port.name && (other.id.is_none() || port.id.is_none())
            }) {
                return Err("Each clock output must identify a distinct exact port".into());
            }
        }
        Ok(())
    }
    /// Identify an omitted legacy clock policy.
    /// Takes this configuration; returns true only when output is disabled, no ports are selected and compensation is zero.
    pub(crate) fn is_default(&self) -> bool {
        self == &Self::default()
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Message {
    Clock,
    Start,
    Continue,
    Stop,
    Position(u16),
}
impl Message {
    fn bytes(self) -> ([u8; 3], usize) {
        match self {
            Self::Clock => ([0xf8, 0, 0], 1),
            Self::Start => ([0xfa, 0, 0], 1),
            Self::Continue => ([0xfb, 0, 0], 1),
            Self::Stop => ([0xfc, 0, 0], 1),
            Self::Position(position) => ([0xf2, (position & 127) as u8, (position >> 7) as u8], 3),
        }
    }
}
#[derive(Clone, Copy, Debug)]
struct Event {
    message: Message,
    deadline_ns: u64,
    generation: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum Fault {
    QueueFull = 1,
    Position = 2,
    Timing = 3,
    Send = 4,
    Callback = 5,
    Safety = 6,
}
impl Fault {
    fn text(self) -> &'static str {
        match self {
            Self::QueueFull => "MIDI clock queue overflowed; clock output stopped. Retry the saved clock policy.",
            Self::Position => "Song position exceeds MIDI's 14-bit position range; clock output stopped. Move before beat 4095.75 and retry.",
            Self::Timing => "Audio transport timing cannot produce ordered MIDI clocks; clock output stopped. Retry after transport recovery.",
            Self::Send => "A selected MIDI clock output failed; all clock outputs stopped. Reconnect the exact output and retry.",
            Self::Callback => "Audio callbacks stopped; external transport was stopped. Retry after audio recovery.",
            Self::Safety => "Emergency recovery stopped external MIDI clock. Retry the saved clock policy after recovery.",
        }
    }
    fn from(value: u8) -> Option<Self> {
        match value {
            1 => Some(Self::QueueFull),
            2 => Some(Self::Position),
            3 => Some(Self::Timing),
            4 => Some(Self::Send),
            5 => Some(Self::Callback),
            6 => Some(Self::Safety),
            _ => None,
        }
    }
}
pub(crate) struct Shared {
    origin: Instant,
    events: crossbeam_channel::Sender<Event>,
    receiver: parking_lot::Mutex<Option<crossbeam_channel::Receiver<Event>>>,
    generation: AtomicU64,
    enabled: AtomicBool,
    compensation_ms: AtomicI32,
    running: AtomicBool,
    fault: AtomicU8,
    callback_until: AtomicU64,
    backend_timing: AtomicBool,
    position_sixteenths: AtomicU64,
    scheduled: AtomicU64,
    sent: AtomicU64,
    late: AtomicU64,
    max_late_ns: AtomicU64,
    overflow: AtomicU64,
    actual_outputs: arc_swap::ArcSwap<Vec<Endpoint>>,
    retired_outputs: arc_swap::ArcSwap<Vec<Endpoint>>,
    retired_until: AtomicU64,
}
impl Default for Shared {
    fn default() -> Self {
        let (events, receiver) = crossbeam_channel::bounded(8192);
        Self {
            origin: Instant::now(),
            events,
            receiver: parking_lot::Mutex::new(Some(receiver)),
            generation: AtomicU64::new(0),
            enabled: AtomicBool::new(false),
            compensation_ms: AtomicI32::new(0),
            running: AtomicBool::new(false),
            fault: AtomicU8::new(0),
            callback_until: AtomicU64::new(0),
            backend_timing: AtomicBool::new(false),
            position_sixteenths: AtomicU64::new(0),
            scheduled: AtomicU64::new(0),
            sent: AtomicU64::new(0),
            late: AtomicU64::new(0),
            max_late_ns: AtomicU64::new(0),
            overflow: AtomicU64::new(0),
            actual_outputs: arc_swap::ArcSwap::from_pointee(Vec::new()),
            retired_outputs: arc_swap::ArcSwap::from_pointee(Vec::new()),
            retired_until: AtomicU64::new(0),
        }
    }
}
fn ns(duration: Duration) -> u64 {
    duration.as_nanos().min(u128::from(u64::MAX)) as u64
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub(crate) struct Counters {
    pub enabled: bool,
    pub running: bool,
    pub backend_timing: bool,
    pub scheduled: u64,
    pub sent: u64,
    pub late: u64,
    pub max_late_ns: u64,
    pub overflow: u64,
    pub position_sixteenths: u64,
    pub error: Option<&'static str>,
}
impl Shared {
    #[cfg(test)]
    pub(crate) fn maximum_activity_for_test(&self) {
        self.enabled.store(true, Release);
        self.running.store(true, Release);
        self.backend_timing.store(true, Release);
        for counter in [
            &self.scheduled,
            &self.sent,
            &self.late,
            &self.max_late_ns,
            &self.overflow,
            &self.position_sixteenths,
        ] {
            counter.store(u64::MAX, Relaxed);
        }
        let longest = [
            Fault::QueueFull,
            Fault::Position,
            Fault::Timing,
            Fault::Send,
            Fault::Callback,
            Fault::Safety,
        ]
        .into_iter()
        .max_by_key(|fault| fault.text().len())
        .unwrap();
        self.fault.store(longest as u8, Release);
    }
    fn now_ns(&self) -> u64 {
        ns(self.origin.elapsed())
    }
    fn fail(&self, fault: Fault) {
        self.enabled.store(false, Release);
        self.running.store(false, Release);
        let _ = self.fault.compare_exchange(0, fault as u8, AcqRel, Acquire);
    }
    /// Read the renderer and output owner's bounded clock status.
    /// Takes this shared clock state; returns counters and the actual refusal without opening a MIDI device.
    pub(crate) fn counters(&self) -> Counters {
        Counters {
            enabled: self.enabled.load(Acquire),
            running: self.running.load(Acquire),
            backend_timing: self.backend_timing.load(Acquire),
            scheduled: self.scheduled.load(Relaxed),
            sent: self.sent.load(Relaxed),
            late: self.late.load(Relaxed),
            max_late_ns: self.max_late_ns.load(Relaxed),
            overflow: self.overflow.load(Relaxed),
            position_sixteenths: self.position_sixteenths.load(Relaxed),
            error: Fault::from(self.fault.load(Acquire)).map(Fault::text),
        }
    }
    /// Refuse echoed realtime transport from a clock destination.
    /// Takes the incoming endpoint; returns whether its name or backend device/client matches an active clock output.
    pub(crate) fn guards_transport(&self, input: &Endpoint) -> bool {
        self.guards_transport_port(&input.name, input.id.as_deref().unwrap_or(""))
    }
    /// Refuse echoed transport from current and recently stopped clock ports.
    /// Takes a borrowed input name and ID; returns its bounded echo guard without constructing endpoint strings.
    pub(crate) fn guards_transport_port(&self, name: &str, id: &str) -> bool {
        self.actual_outputs
            .load()
            .iter()
            .any(|output| output.conflicts_port(name, id))
            || self.now_ns() < self.retired_until.load(Acquire)
                && self
                    .retired_outputs
                    .load()
                    .iter()
                    .any(|output| output.conflicts_port(name, id))
    }
}

#[cfg(test)]
mod tests;
