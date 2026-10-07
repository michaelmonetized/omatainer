//! USB-MIDI class-compliant I/O, hardware maps,  and clock.

use crate::engine::{Command, DECKS, HOTCUES};
use parking_lot::Mutex;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

mod connections;
mod policy;
mod profile;
mod handoff;
mod framing;
mod relative;
pub(crate) mod controls;
pub use controls::Spec as ControlSpec;
pub use controls::PairOrder;
mod surface;
mod feedback;
pub(crate) mod learn;
pub(crate) mod presets;
pub(crate) mod routing;
pub(crate) mod clock;
pub(crate) mod device_status;
pub use handoff::InputStats;
pub use connections::Retry;
pub use policy::{InputPolicy, PolicyError, PolicyStatus};
#[cfg(test)]
pub(crate) use connections::test_support as connection_test_support;
pub use relative::RelativeSpec;
pub use feedback::Stats as FeedbackStats;
pub(crate) use relative::RelativeEncoding;
#[cfg(test)]
mod profile_tests;
#[cfg(test)]
mod surface_tests;
#[cfg(test)]
mod realtime_tests;
#[cfg(test)]
mod relative_tests;

static NEXT_SOURCE: AtomicU64 = AtomicU64::new(1);

/// Allocate an independent performance input owner.
/// Takes no arguments; returns a unique positive ID, or zero after exhaustion.
pub(crate) fn next_source_id() -> u64 {
    NEXT_SOURCE.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |id|id.checked_add(1)).unwrap_or(0)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum MsgKind {
    Note,
    Cc,
    Cc14,
    CcRel,
    Pitch,
}

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Binding {
    pub kind: MsgKind,
    pub ch: u8, // 0-15, 0xFF = any
    pub data: u8,
    pub action: Action,
    pub deck: u8,
    pub extra: u16,
    pub relative: Option<RelativeSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub controls: Option<ControlSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pair_order: Option<PairOrder>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Action {
    DeckPlay,
    DeckCue,
    DeckCueHold,
    DeckSync,
    DeckJog,
    DeckJogTouch,
    DeckPitch,
    DeckGain,
    DeckEqHi,
    DeckEqMid,
    DeckEqLow,
    DeckFilter,
    DeckPfl,
    DeckHotCue,
    DeckLoop4,
    DeckLoopIn,
    DeckLoopOut,
    DeckBeatJumpBack,
    DeckBeatJumpForward,
    DeckBeatJumpSmaller,
    DeckBeatJumpLarger,
    DeckLoad,
    DeckLoadLock,
    DeckVinyl,
    Xfader,
    XfaderCurve,
    Master,
    CueMix,
    Browse,
    BrowseCrates,
    CrateReturn,
    SamplerSlotStop,
    Prepare,
    PrepareCrate,
    LoadA,
    LoadB,
    Scene,
    Clip,
    TrackFader,
    TrackMute,
    TrackSolo,
    TrackArm,
    TrackPan,
    TrackSendA,
    TrackSendB,
    Play,
    Stop,
    Record,
    Tap,
    Shift,
    FxWet,
    FxSelect,
    SongLocator,
    SongPrevious,
    SongNext,
    SongLoop,
    SongCancel,
}

#[derive(Clone, Debug)]
pub struct MidiMap {
    pub name: String,
    pub matchers: Vec<String>,
    pub bindings: Vec<Binding>,
    pub unmapped_notes: UnmappedNotes,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnmappedNotes {
    Live,
    Ignore,
}

pub struct MidiHub {
    feedback: Option<feedback::Manager>,
    connections: Option<connections::Manager>,
    input_counters: Arc<handoff::InputCounters>,
    routing: Option<routing::Manager>,
    clock: Option<clock::Manager>,
    pub log: Arc<Mutex<Vec<String>>>,
}

#[cfg(test)]
pub(crate) struct TestInput { callback: handoff::InputSink, _worker: handoff::InputGuard, counters: Arc<handoff::InputCounters> }
#[cfg(test)]
impl TestInput {
    pub(crate) fn push(&mut self,message:&[u8]) {
        let before=self.counters.snapshot().dispatched;
        let allocation=super::test_alloc::measure(||self.callback.push(message));
        assert_eq!((allocation.allocations,allocation.frees),(0,0));
        let until=Instant::now()+std::time::Duration::from_secs(2);
        while self.counters.snapshot().dispatched==before {assert!(Instant::now()<until,"synthetic input did not dispatch");std::thread::sleep(std::time::Duration::from_millis(1));}
    }
}
impl MidiHub {
    #[cfg(test)]
    pub(crate) fn open_for_test(&self,cmd:&super::CommandPort,source:u64,map:MidiMap,name:&str,id:&str)->TestInput {
        map.validate().unwrap();let counters=Arc::new(handoff::InputCounters::default());
        let (callback,worker)=handoff::start_on_port(source,map,cmd.clone(),self.log.clone(),name.into(),id.into(),counters.clone(),true,||{}).unwrap();
        TestInput{callback,_worker:worker,counters}
    }
    /// Test the still-connected captured action through normal command admission.
    /// Takes its captured wire message and command port; returns an admission receipt or an explicit refusal.
    pub(crate) fn preview_learn(&self,cmd:&super::CommandPort,capture:&learn::Capture)->Result<String,String> {
        let view=cmd.midi_learn().view();
        if view.capture.as_ref()!=Some(capture) || !view.devices.iter().any(|d|d.source==capture.source && d.endpoint==capture.mapping.endpoint) {return Err("Captured MIDI device or review changed; capture again".into());}
        if view.devices.iter().filter(|device|device.endpoint==capture.mapping.endpoint).count()!=1 {return Err("Captured MIDI port is ambiguous".into());}
        if cmd.performance().protected() {return Err("Performance protection excludes MIDI assignment tests".into());}
        if capture.mapping.binding.action==Action::Shift {return Err("Assign Shift and test its following hardware gesture".into());}
        let shift=Arc::new(Mutex::new([false;4]));let bytes=capture.bytes;
        dispatch_value(&capture.mapping.binding,capture.source,bytes[0]&0xf0,bytes[2],&bytes,cmd,&shift,capture.value).map_err(|e|e.to_string())?;
        if bytes[0]&0xf0==0x90 {let off=[bytes[0]&15|0x80,bytes[1],0];dispatch(&capture.mapping.binding,capture.source,0x80,0,&off,cmd,&shift).map_err(|e|e.to_string())?;}
        Ok("Captured action admitted. Check its normal control or load receipt.".into())
    }

    #[cfg(test)]
    pub(crate) fn receive_for_test(&self, cmd: &super::CommandPort, source: u64, device: &str, message: &[u8]) {
        let map = pick_map(&builtin_maps().unwrap(), device);
        self.receive_map_for_test(cmd, source, map, device, message);
    }

    #[cfg(test)]
    pub(crate) fn receive_map_for_test(&self, cmd: &super::CommandPort, source: u64, map: MidiMap, device: &str, message: &[u8]) {
        map.validate().expect("invalid synthetic controller map");
        let counters = Arc::new(handoff::InputCounters::default());
        let (mut callback, worker) = handoff::start(
            source, map, cmd.clone(), self.log.clone(),  device.into(), counters.clone(),
        ).unwrap();
        let allocation = super::test_alloc::measure(|| callback.push(message));
        assert_eq!(allocation.allocations, 0, "raw MIDI callback allocated");
        assert_eq!(allocation.frees, 0, "raw MIDI callback destroyed source storage");
        let until = Instant::now() + std::time::Duration::from_secs(2);
        while counters.snapshot().dispatched == 0 {
            assert!(Instant::now() < until, "synthetic MIDI worker did not dispatch");
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        drop(callback);
        drop(worker);
    }

    /// Explicit safe startup: no manager, discovery or OS port construction.
    pub(super) fn without_devices() -> Self {
        Self {
            feedback: None,
            connections: None,
            input_counters: Arc::new(handoff::InputCounters::default()),
            routing: None,
            clock: None,
            log: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub fn start(cmd: super::CommandPort, snapshot: Arc<Mutex<super::Snapshot>>) -> anyhow::Result<Self> {
        Self::start_with_policy(cmd, snapshot, InputPolicy::All)
    }

    pub fn start_with_policy(cmd: super::CommandPort, snapshot: Arc<Mutex<super::Snapshot>>, policy: InputPolicy) -> anyhow::Result<Self> {
        Self::start_with_routing(cmd,snapshot,policy,routing::Routing::default())
    }
    pub fn start_with_routing(cmd:super::CommandPort,snapshot:Arc<Mutex<super::Snapshot>>,policy:InputPolicy,routes:routing::Routing)->anyhow::Result<Self>{
        Self::start_with_clock(cmd,snapshot,policy,routes,clock::Config::default())
    }
    pub(crate) fn start_with_clock(cmd:super::CommandPort,snapshot:Arc<Mutex<super::Snapshot>>,policy:InputPolicy,routes:routing::Routing,clocks:clock::Config)->anyhow::Result<Self>{
        clocks.validate().map_err(anyhow::Error::msg)?;
        policy.validate()?;
        // Fail profile validation before a device callback can dispatch it.
        let maps = builtin_maps()?;
        let log = Arc::new(Mutex::new(Vec::new()));
        let input_counters = Arc::new(handoff::InputCounters::default());
        let routing=Some(routing::Manager::start(cmd.clone(),routes).map_err(anyhow::Error::msg)?);
        let connections = connections::Manager::start_with_policy(
            connections::MidirBackend,
            &snapshot, cmd.clone(), maps, log.clone(),  input_counters.clone(), policy,
        )?;
        let clock=Some(clock::Manager::start(cmd.clone(),clocks).map_err(anyhow::Error::msg)?);
        let feedback = Some(feedback::Manager::start(Arc::downgrade(&snapshot), cmd, input_counters.clone(), connections.policy_reader())?);
        Ok(Self { feedback, connections: Some(connections), input_counters, routing, clock, log })
    }

    pub(crate) fn configure_clock(&self,config:clock::Config)->Result<(),String>{self.clock.as_ref().ok_or("MIDI clock output owner unavailable")?.configure(config)}
    pub(crate) fn clock_status(&self)->Option<Arc<clock::Status>>{self.clock.as_ref().map(clock::Manager::status)}
    pub(crate) fn cancel_clock(&self)->bool{self.clock.as_ref().is_some_and(clock::Manager::cancel)}
    pub fn configure_routing(&self,routes:routing::Routing)->Result<u64,String>{
        self.routing.as_ref().ok_or("MIDI output/routing owner is unavailable")?.configure(routes)
    }
    pub fn routing_status(&self)->Option<Arc<routing::Status>>{self.routing.as_ref().map(|r|r.status())}
    pub fn cancel_routing(&self)->bool{self.routing.as_ref().is_some_and(|r|r.cancel())}

    /// Does not wait for discovery, connection teardown, or a queue slot.
    pub fn configure_inputs(&self, policy: InputPolicy) -> Result<u64, PolicyError> {
        self.connections.as_ref().ok_or(PolicyError::Unavailable)?.configure(policy)
    }

    pub fn policy_status(&self) -> Option<Arc<PolicyStatus>> {
        self.connections.as_ref().map(|manager| manager.policy_status())
    }

    /// Retry admission never waits for OS discovery/connect or a queue slot.
    pub fn retry_connections(&self) -> Retry {
        self.connections.as_ref().map_or(Retry::Unavailable, |manager| manager.retry())
    }

    pub fn connections_busy(&self) -> bool {
        self.connections.as_ref().is_some_and(|manager| manager.busy())
    }

    pub fn connections_available(&self) -> bool {
        self.connections.as_ref().is_some_and(|manager| manager.available())
    }

    pub fn input_stats(&self) -> InputStats {
        self.input_counters.snapshot()
    }
    pub fn feedback_stats(&self) -> feedback::Stats { self.feedback.as_ref().map_or_else(Default::default, |feedback| feedback.stats()) }


}

fn handle_msg(
    msg: &[u8],
    source: u64,
    map: &MidiMap,
    cmd: &super::CommandPort,
    log: &Arc<Mutex<Vec<String>>>,
    shift: &Arc<Mutex<[bool; 4]>>,
    dev: &str,
) {
    for message in framing::messages(msg) {
        match message {
            framing::Message::Realtime(status) => {
                // These status-only messages bypass channel data access and
                // learn capture, including when interleaved in a frame.
                let command = match status {
                    0xfa => Some(Command::Play),
                    0xfb => Some(Command::Play),
                    0xfc => Some(Command::Stop),
                    0xf8 => Some(Command::MidiClock { source }),
                    _ => None, // Continue/sensing/reset/reserved: no handler yet.
                };
                if let Some(command) = command {
                    let _ = cmd.send(command);
                }
            }
            framing::Message::Channel(frame) => {
                handle_channel(&frame, source, map, cmd, log,  shift, dev, true);
            }
        }
    }
}

fn handle_channel(
    msg: &[u8; 3],
    source: u64,
    map: &MidiMap,
    cmd: &super::CommandPort,
    log: &Arc<Mutex<Vec<String>>>,
    shift: &Arc<Mutex<[bool; 4]>>,
    dev: &str,
    allow_live: bool,
) {
    handle_channel_value(msg, source, map, cmd, log, shift, dev, allow_live, controls::PairValues::default());
}
fn handle_channel_value(
    msg: &[u8; 3], source: u64, map: &MidiMap, cmd: &super::CommandPort,
    log: &Arc<Mutex<Vec<String>>>, shift: &Arc<Mutex<[bool; 4]>>, dev: &str,
    allow_live: bool, paired: controls::PairValues,
) {
    let st = msg[0];
    let kind_hi = st & 0xF0;
    let ch = st & 0x0F;
    let d1 = msg[1];
    let d2 = msg[2];
    if let Some(release) = super::clip_launch::wire_release(msg, source) { let _ = cmd.send(Command::ClipRelease(release)); }

    {
        let mut l = log.lock();
        l.push(format!(
            "{dev} ch{} {kind_hi:02X} {d1} {d2}",
            ch + 1
        ));
        if l.len() > 64 {
            let n = l.len() - 64;
            l.drain(0..n);
        }
    }


    let mut matched = false;
    for b in &map.bindings {
        if b.ch != 0xFF && b.ch != ch {
            continue;
        }
        let hit = match b.kind {
            MsgKind::Note => kind_hi == 0x90 || kind_hi == 0x80,
            MsgKind::Cc | MsgKind::Cc14 | MsgKind::CcRel => kind_hi == 0xB0,
            MsgKind::Pitch => kind_hi == 0xE0,
        };
        if !hit {
            continue;
        }
        if b.kind != MsgKind::Pitch && b.data != if b.kind == MsgKind::Cc14 { d1 % 32 } else { d1 } {
            continue;
        }
        matched = true;
        let _ = dispatch_value(b, source, kind_hi, d2, msg, cmd, shift, paired.value(b.pair_order));
    }

    // Live MIDI notes onto the selected track when no map consumed a note
    // (generic class-compliant keyboards / Akai MPK keys).
    if allow_live && !matched && map.unmapped_notes == UnmappedNotes::Live {
        if kind_hi == 0x90 && d2 > 0 {
            let _ = cmd.send(Command::LiveNoteOn {
                source,
                ch,
                note: d1,
                vel: d2,
            });
        } else if kind_hi == 0x80 || (kind_hi == 0x90 && d2 == 0) {
            let _ = cmd.send(Command::LiveNoteOff { source, ch, note: d1 });
        }
    }
}

fn dispatch(
    b: &Binding,
    source: u64,
    status: u8,
    d2: u8,
    msg: &[u8; 3],
    cmd: &super::CommandPort,
    shift: &Arc<Mutex<[bool; 4]>>,
) -> Result<(), super::SubmissionError> {
    dispatch_value(b, source, status, d2, msg, cmd, shift, None)
}
fn dispatch_value(
    b: &Binding, source: u64, status: u8, d2: u8, msg: &[u8; 3],
    cmd: &super::CommandPort, shift: &Arc<Mutex<[bool; 4]>>, paired: Option<u16>,
) -> Result<(), super::SubmissionError> {
    let mut failure = None;
    let mut send = |command| { let result = cmd.send(command);if let Err(error) = &result { if failure.is_none() { failure = Some(error.clone()); } } result };
    let pressed = matches!(status, 0x90 | 0xb0) && d2 > 0;
    if let Some(release) = super::clip_launch::wire_release(msg, source) { let _ = send(Command::ClipRelease(release)); }
    if b.kind == MsgKind::Cc14 && (msg[1] >= 64 || paired.is_none()) { return Ok(()); }
    let rel = match b.kind {
        MsgKind::CcRel => {
            let Some(delta) = b.relative.and_then(|spec| spec.decode(d2)) else {
                return Ok(());
            };
            // A stationary report must not reset an active scratch/nudge.
            if delta == 0.0 {
                return Ok(());
            }
            b.controls.unwrap_or_default().direction(delta)
        }
        MsgKind::Cc14 => paired.unwrap() as f32 / 16383.0,
        MsgKind::Pitch => {
            let v = (msg[1] as u16) | ((msg[2] as u16) << 7);
            if v <= 8192 { v as f32 / 16384.0 } else { 0.5 + (v - 8192) as f32 / 16382.0 }
        }
        _ => d2 as f32 / 127.0,
    };
    if b.kind == MsgKind::CcRel && controls::continuous(b.action) {
        return cmd.send(Command::MidiAdjust(controls::Adjust { binding: *b, delta: rel })).map(|_| ());
    }
    let rel = if b.kind != MsgKind::CcRel { b.controls.map_or(rel, |spec| spec.absolute(rel)) } else { rel };
    let deck = b.deck.min((DECKS - 1) as u8);
    match b.action {
        Action::Shift => shift.lock()[deck as usize] = pressed,
        Action::DeckPlay if pressed => {
            let _ = send(Command::DeckPlay { deck });
        }
        Action::DeckCue if pressed => {
            let _ = send(Command::DeckCue { deck });
        }
        Action::DeckCueHold => {
            let _ = send(Command::DeckControl { source, deck, control: super::deck_controls::Control::Hold {
                button: super::deck_controls::Button::Cue, on: pressed,
            } });
        }
        Action::DeckSync if pressed => {
            let _ = send(Command::DeckSync { deck });
        }
        Action::DeckBeatJumpBack | Action::DeckBeatJumpForward if pressed => {
            let _ = send(Command::DeckControl { source, deck, control: super::deck_controls::Control::BeatJump {
                forward: b.action == Action::DeckBeatJumpForward,
            } });
        }
        Action::DeckBeatJumpSmaller | Action::DeckBeatJumpLarger if pressed => {
            let _ = send(Command::DeckControl { source, deck, control: super::deck_controls::Control::BeatJumpScale {
                up: b.action == Action::DeckBeatJumpLarger,
            } });
        }
        Action::DeckJog => {
            let _ = send(Command::DeckJog {
                deck,
                delta: if b.kind == MsgKind::CcRel {
                    rel
                } else {
                    rel * 0.35
                },
            });
        }
        Action::DeckJogTouch => {
            let _ = send(Command::MidiDeckTouch {
                source,
                deck,
                on: pressed,
            });
        }
        Action::DeckPitch => {
            let _ = send(Command::DeckPitch { deck, value: rel });
        }
        Action::DeckGain => {
            let _ = send(Command::DeckGain { deck, value: rel });
        }
        Action::DeckEqHi => {
            let _ = send(Command::DeckEq {
                deck,
                band: 2,
                value: rel,
            });
        }
        Action::DeckEqMid => {
            let _ = send(Command::DeckEq {
                deck,
                band: 1,
                value: rel,
            });
        }
        Action::DeckEqLow => {
            let _ = send(Command::DeckEq {
                deck,
                band: 0,
                value: rel,
            });
        }
        Action::DeckFilter => {
            let _ = send(Command::DeckFilter { deck, value: rel });
        }
        Action::DeckPfl if pressed => {
            let _ = send(Command::DeckPfl { deck });
        }
        Action::DeckHotCue if pressed => {
            let _ = send(Command::DeckHotCue {
                deck,
                pad: b.extra.min((HOTCUES - 1) as u16) as u8,
                del: shift.lock()[deck as usize],
            });
        }
        Action::DeckLoop4 if pressed => {
            let _ = send(Command::DeckLoop { deck, beats: 4.0 });
        }
        Action::DeckLoopIn if pressed => {
            let _ = send(Command::DeckLoopIn { deck });
        }
        Action::DeckLoopOut if pressed => {
            let _ = send(Command::DeckLoopOut { deck });
        }
        Action::Prepare if pressed => { let _ = send(Command::PrepareSelected { all: false }); }
        Action::PrepareCrate if pressed => { let _ = send(Command::PrepareSelected { all: true }); }
        Action::DeckLoad if pressed => {
            let _ = send(if shift.lock()[deck as usize] { Command::PrepareSelected { all: deck == 1 } } else { Command::DeckLoadSelected { deck } });
        }
        Action::DeckLoadLock if pressed => {
            let enabled = !cmd.performance().deck_load_locked(deck as usize);
            let _ = send(Command::DeckLoadLock { deck, enabled });
        }
        Action::DeckVinyl if pressed => {
            let _ = send(Command::DeckVinyl { deck });
        }
        Action::Xfader => {
            let _ = send(Command::Xfader(rel));
        }
        Action::XfaderCurve => {
            let _ = send(Command::XfaderCurve(rel));
        }
        Action::Master => {
            let _ = send(Command::Master(rel));
        }
        Action::CueMix => {
            let _ = send(Command::CueMix(rel));
        }
        Action::BrowseCrates if b.kind == MsgKind::CcRel && b.relative.is_some_and(|spec| spec.scale == 1.0) => { let _ = send(Command::BrowseCrates(rel)); }
        Action::CrateReturn if pressed => { let _ = send(Command::CrateReturn); }
        Action::SamplerSlotStop if pressed && b.extra < 16 => { let _ = send(Command::SamplerSlotStop { pad: b.extra as u8 }); }
        Action::Browse if b.kind == MsgKind::CcRel && b.relative.is_some_and(|spec| spec.scale == 1.0) => {
            let _ = send(Command::Browse(rel));
        }
        Action::LoadA if pressed => {
            let _ = send(if shift.lock()[deck as usize] { Command::PrepareSelected { all: false } } else { Command::DeckLoadSelected { deck: 0 } });
        }
        Action::LoadB if pressed => {
            let _ = send(if shift.lock()[deck as usize] { Command::PrepareSelected { all: true } } else { Command::DeckLoadSelected { deck: 1 } });
        }
        Action::Scene if pressed => {
            let _ = send(Command::LaunchScene {
                scene: b.extra.min((super::session::MAX_SCENES - 1) as u16),
            });
        }
        Action::Clip if pressed => {
            let _ = send(Command::ClipPress(super::clip_launch::Press { source, key: super::clip_launch::wire_key(msg), target: super::clip_launch::Target::Slot {
                track: b.deck.min((super::session::MAX_TRACKS - 1) as u8), scene: b.extra.min((super::session::MAX_SCENES - 1) as u16), looping: true,
            }}));
        }
        Action::TrackFader => {
            let _ = send(Command::TrackGain {
                track: b.extra.min((super::session::MAX_TRACKS - 1) as u16) as u8,
                value: rel,
            });
        }
        Action::TrackMute if pressed => {
            let _ = send(Command::Mute {
                track: b.extra.min((super::session::MAX_TRACKS - 1) as u16) as u8,
            });
        }
        Action::TrackSolo if pressed => { let _ = send(Command::Solo { track: b.extra as u8 }); }
        Action::TrackArm if pressed => { let _ = send(Command::Arm { track: b.extra as u8 }); }
        Action::TrackPan => { let _ = send(Command::TrackPan { track: b.extra as u8, value: rel }); }
        Action::TrackSendA | Action::TrackSendB => { let _ = send(Command::Surface(super::surface_controls::Input::TrackSend { track: b.extra as u8, send: u8::from(b.action == Action::TrackSendB), value: rel })); }
        Action::SongLocator if pressed => { let _ = send(Command::SongNavigation(super::song_navigation::Action::Locator { id:b.extra, grid:super::clip_launch::Grid::Global })); }
        Action::SongPrevious if pressed => { let _ = send(Command::SongNavigation(super::song_navigation::Action::Previous(super::clip_launch::Grid::Global))); }
        Action::SongNext if pressed => { let _ = send(Command::SongNavigation(super::song_navigation::Action::Next(super::clip_launch::Grid::Global))); }
        Action::SongLoop if pressed => { let _ = send(Command::SongNavigation(super::song_navigation::Action::ToggleLoop)); }
        Action::SongCancel if pressed => { let _ = send(Command::SongNavigation(super::song_navigation::Action::Cancel)); }
        Action::Play if pressed => {
            let _ = send(Command::TogglePlay);
        }
        Action::Stop if pressed => {
            let _ = send(Command::Stop);
        }
        Action::Record if pressed => {
            let _ = send(Command::Record);
        }
        Action::Tap if pressed => {
            let _ = send(Command::Tap(Instant::now()));
        }
        Action::FxWet => {
            let _ = send(Command::FxWet {
                slot: b.extra.min(255) as u8,
                value: rel,
            });
        }
        Action::FxSelect if pressed => {
            let _ = send(Command::FxSelect { slot: b.extra.min(255) as u8 });
        }
        _ => {}
    }
    failure.map_or(Ok(()), Err)
}

fn pick_map(maps: &[MidiMap], name: &str) -> MidiMap {
    let n = name.to_lowercase();
    for m in maps {
        for pat in &m.matchers {
            if n.contains(&pat.to_lowercase()) {
                return m.clone();
            }
        }
    }
    maps.iter()
        .find(|m| m.name == "Class-compliant MIDI")
        .cloned()
        .unwrap_or_else(|| maps[0].clone())
}

fn nbind(ch: u8, note: u8, action: Action, deck: u8, extra: u8) -> Binding {
    Binding {
        kind: MsgKind::Note,
        ch,
        data: note,
        action,
        deck,
        extra: u16::from(extra),
        relative: None,
        controls: None,
        pair_order: None,
    }
}
fn cbind(ch: u8, cc: u8, action: Action, deck: u8, extra: u8) -> Binding {
    Binding {
        kind: MsgKind::Cc,
        ch,
        data: cc,
        action,
        deck,
        extra: u16::from(extra),
        relative: None,
        controls: None,
        pair_order: None,
    }
}
fn rbind(ch: u8, cc: u8, action: Action, deck: u8, extra: u8, relative: RelativeSpec) -> Binding {
    Binding {
        kind: MsgKind::CcRel,
        ch,
        data: cc,
        action,
        deck,
        extra: u16::from(extra),
        relative: Some(relative),
        controls: None,
        pair_order: None,
    }
}

pub fn builtin_maps() -> anyhow::Result<Vec<MidiMap>> {
    let mut maps = Vec::new();
    maps.push(surface::pioneer_sp1());
    maps.push(pioneer_ddj_fx());
    maps.push(numark_ns7(true));
    maps.push(numark_ns7(false));
    maps.push(akai_apc_mini());
    // The specific MkII name must precede the original's broader matcher.
    maps.push(akai_apc40_mk2());
    maps.push(akai_apc40());
    maps.push(surface::mpd232::configured()?.unwrap_or_else(akai_mpd232));
    maps.push(akai_mpk());
    maps.push(class_compliant());
    for map in &maps {
        map.validate()?;
    }
    Ok(maps)
}

/// Pioneer DDJ-FLX / DDJ-400 / DDJ-SB3 family ("DDJ-FX").
fn pioneer_ddj_fx() -> MidiMap {
    let mut b = Vec::new();
    for deck in 0..2u8 {
        let ch = deck;
        b.push(nbind(ch, 0x0B, Action::DeckPlay, deck, 0));
        b.push(nbind(ch, 0x0C, Action::DeckCueHold, deck, 0));
        b.push(nbind(ch, 0x58, Action::DeckSync, deck, 0));
        b.push(nbind(ch, 0x3F, Action::Shift, deck, 0));
        b.push(nbind(ch, 0x36, Action::DeckJogTouch, deck, 0));
        b.push(nbind(ch, 0x10, Action::DeckLoopIn, deck, 0));
        b.push(nbind(ch, 0x11, Action::DeckLoopOut, deck, 0));
        b.push(nbind(ch, 0x0D, Action::DeckLoop4, deck, 0));
        b.push(nbind(ch, 0x54, Action::DeckPfl, deck, 0));
        b.push(nbind(ch, 0x02, Action::DeckLoad, deck, 0));
        b.push(nbind(ch, 0x17, Action::DeckVinyl, deck, 0));
        // DDJ-FLX4 / DDJ-400 / DDJ-SB3 MIDI lists: difference counts
        // centered on 0x40 for wheel side and platter (vinyl on/off).
        for cc in [0x21, 0x22, 0x23] {
            b.push(rbind(ch, cc, Action::DeckJog, deck, 0, RelativeSpec::PIONEER_JOG));
        }
        b.push(cbind(ch, 0x00, Action::DeckPitch, deck, 0));
        b.push(cbind(ch, 0x13, Action::DeckGain, deck, 0));
        b.push(cbind(ch, 0x10, Action::DeckEqHi, deck, 0));
        b.push(cbind(ch, 0x0E, Action::DeckEqMid, deck, 0));
        b.push(cbind(ch, 0x0C, Action::DeckEqLow, deck, 0));
        b.push(cbind(ch, 0x17, Action::DeckFilter, deck, 0));
        // performance pads — hot cue mode on ch 7 / 9 (0-index 6 / 8)
        let pad_ch = if deck == 0 { 6 } else { 8 };
        for pad in 0..8u8 {
            b.push(nbind(pad_ch, pad, Action::DeckHotCue, deck, pad));
        }
    }
    b.push(cbind(6, 0x1F, Action::Xfader, 0, 0));
    b.push(cbind(0, 0x1F, Action::Xfader, 0, 0));
    b.push(cbind(5, 0x10, Action::FxWet, 0, 0));
    MidiMap {
        name: "Pioneer DDJ-FX / FLX".into(),
        matchers: vec![
            "ddj".into(),
            "ddj-flx".into(),
            "ddj-sb".into(),
            "ddj-sz".into(),
            "ddj-400".into(),
            "ddj-200".into(),
            "pioneer".into(),
            "alphaTheta".into(),
        ],
        bindings: b,
        unmapped_notes: UnmappedNotes::Live,
    }
}

/// Legacy NS7 / NS7FX controls. Wheel position requires a separate stateful
/// protocol: do not interpret guessed CC21/pitch-bend as relative movement.
/// See docs/validation/issue-42-relative-jog.md for evidence and limitations.
fn numark_ns7(fx: bool) -> MidiMap {
    if !fx { return surface::numark_ns7(); }
    let mut b = Vec::new();
    for deck in 0..2u8 {
        let ch = deck;
        b.push(nbind(ch, 0x0C, Action::DeckPlay, deck, 0));
        b.push(nbind(ch, 0x0D, Action::DeckCueHold, deck, 0));
        b.push(nbind(ch, 0x0E, Action::DeckSync, deck, 0));
        b.push(nbind(ch, 0x1B, Action::DeckPfl, deck, 0));
        b.push(nbind(ch, 0x17, Action::DeckVinyl, deck, 0));
        b.push(cbind(ch, 0x13, Action::DeckGain, deck, 0));
        b.push(cbind(ch, 0x10, Action::DeckEqHi, deck, 0));
        b.push(cbind(ch, 0x0E, Action::DeckEqMid, deck, 0));
        b.push(cbind(ch, 0x0C, Action::DeckEqLow, deck, 0));
        b.push(cbind(ch, 0x15, Action::DeckFilter, deck, 0));
        b.push(cbind(ch, 0x09, Action::DeckPitch, deck, 0));
        for pad in 0..8u8 {
            b.push(nbind(ch, 0x2E + pad, Action::DeckHotCue, deck, pad));
        }
        b.push(nbind(ch, 0x10, Action::DeckLoopIn, deck, 0));
        b.push(nbind(ch, 0x11, Action::DeckLoopOut, deck, 0));
    }
    b.push(cbind(0, 0x1F, Action::Xfader, 0, 0));
    b.push(cbind(0, 0x07, Action::Master, 0, 0));
    if fx {
        b.push(cbind(0, 0x30, Action::FxWet, 0, 0));
        b.push(cbind(0, 0x31, Action::FxWet, 0, 1));
        b.push(cbind(0, 0x32, Action::FxWet, 0, 2));
        b.push(nbind(0, 0x3A, Action::FxSelect, 0, 0));
        b.push(nbind(0, 0x3B, Action::FxSelect, 0, 1));
        b.push(nbind(0, 0x3C, Action::FxSelect, 0, 2));
    }
    MidiMap {
        name: if fx {
            "Numark NS7FX (legacy; wheels unmapped)".into()
        } else {
            "Numark NS7 (legacy; wheels unmapped)".into()
        },
        matchers: if fx {
            vec!["ns7fx".into(), "ns7-fx".into(), "ns7 fx".into()]
        } else {
            vec!["ns7".into(), "numark ns7".into()]
        },
        bindings: b,
        unmapped_notes: UnmappedNotes::Live,
    }
}

fn akai_apc_mini() -> MidiMap {
    let mut b = Vec::new();
    // 8x8 clip grid notes 0-63, row-major from bottom: note = row*8+col
    // We treat rows as scenes, cols as tracks.
    for scene in 0..8u8 {
        for track in 0..8u8 {
            let note = scene * 8 + track;
            b.push(nbind(0, note, Action::Clip, track, scene));
        }
        b.push(nbind(0, 82 + scene, Action::Scene, 0, scene));
    }
    for t in 0..8u8 {
        b.push(cbind(0, 48 + t, Action::TrackFader, 0, t));
        b.push(nbind(0, 64 + t, Action::TrackMute, 0, t));
    }
    b.push(cbind(0, 56, Action::Master, 0, 0));
    b.push(nbind(0, 98, Action::Shift, 0, 0));
    MidiMap {
        name: "Akai APC Mini".into(),
        matchers: vec!["apc mini".into(), "apc-mini".into(), "apcmini".into()],
        bindings: b,
        unmapped_notes: UnmappedNotes::Live,
    }
}

// These addresses are shared by the original and MkII protocols. Clip-grid
// addresses are deliberately constructed separately in the two profiles.
fn apc40_common_bindings() -> Vec<Binding> {
    let mut b = Vec::new();
    for scene in 0..5u8 {
        // Global note controls do not use the track-channel discriminator.
        b.push(nbind(0xff, 0x52 + scene, Action::Scene, 0, scene));
    }
    for track in 0..8u8 {
        b.push(cbind(track, 7, Action::TrackFader, 0, track));
    }
    b.push(cbind(0, 14, Action::Master, 0, 0));
    b
}

fn akai_apc40() -> MidiMap {
    let mut b = apc40_common_bindings();
    // Akai APC40 protocol rev. 1, pp. 16-18: five clip buttons on each
    // track's MIDI channel. The MkII has a different grid address space.
    for scene in 0..5u8 {
        for track in 0..8u8 {
            b.push(nbind(track, 0x35 + scene, Action::Clip, track, scene));
        }
    }
    MidiMap {
        name: "Akai APC40 (original)".into(),
        matchers: vec!["apc40".into(), "apc 40".into(), "apc-40".into()],
        bindings: b,
        // Unmapped surface buttons are not piano keys. In particular record
        // arm and track selection must never mute a track or start a deck.
        unmapped_notes: UnmappedNotes::Ignore,
    }
}

fn akai_apc40_mk2() -> MidiMap {
    let mut b = apc40_common_bindings();
    b.push(nbind(0xff, 0x5c, Action::Stop, 0, 0));
    // Akai APC40 Mk2 protocol v1.2, pp. 30-34: forty distinct clip notes,
    // with the same eight per-channel CC7 track faders as the original.
    for scene in 0..5u8 {
        for track in 0..8u8 {
            b.push(nbind(0xff, (4 - scene) * 8 + track, Action::Clip, track, scene));
        }
    }
    MidiMap {
        name: "Akai APC40 mkII".into(),
        matchers: ["apc40", "apc 40", "apc-40"]
            .into_iter()
            .flat_map(|model| {
                ["mkii", "mk2", "mk ii", "mk 2"]
                    .into_iter()
                    .flat_map(move |variant| {
                        [format!("{model} {variant}"), format!("{model}{variant}")]
                    })
            })
            .collect(),
        bindings: b,
        unmapped_notes: UnmappedNotes::Ignore,
    }
}

fn akai_mpd232() -> MidiMap {
    MidiMap {
        name: "Akai MPD232 (programmable)".into(),
        matchers: vec!["mpd232".into(), "mpd 232".into(), "mpd-232".into()],
        bindings: surface::mpd232::transport_bindings(),
        unmapped_notes: UnmappedNotes::Live,
    }
}

fn akai_mpk() -> MidiMap {
    let mut b = Vec::new();
    // pads typically C1 (36) upward — treat as drum / hotcues
    for i in 0..8u8 {
        b.push(nbind(9, 36 + i, Action::DeckHotCue, 0, i));
        // CC1 already controls the filter below; do not also change FX wet.
        if i > 0 {
            b.push(cbind(0, 1 + i, Action::FxWet, 0, i.min(2)));
        }
    }
    b.push(cbind(0, 1, Action::DeckFilter, 0, 0));
    MidiMap {
        name: "Akai MPK / MPC / LPD".into(),
        matchers: vec![
            "mpk".into(),
            "mpc".into(),
            "lpd".into(),
            "mpd".into(),
            "akai".into(),
        ],
        bindings: b,
        unmapped_notes: UnmappedNotes::Live,
    }
}

fn class_compliant() -> MidiMap {
    MidiMap {
        name: "Class-compliant MIDI".into(),
        matchers: vec!["midi".into(), "usb".into()],
        bindings: vec![
            cbind(0, 7, Action::Master, 0, 0),
            cbind(0, 1, Action::DeckFilter, 0, 0),
            cbind(0, 10, Action::Xfader, 0, 0),
        ],
        unmapped_notes: UnmappedNotes::Live,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map_for(name: &str) -> String {
        let maps = builtin_maps().unwrap();
        pick_map(&maps, name).name
    }

    #[test]
    fn maps_bind_named_hardware() {
        assert!(map_for("Pioneer DDJ-FLX4").contains("DDJ"));
        assert!(map_for("DDJ-400").contains("DDJ"));
        assert_eq!(map_for("Numark NS7FX"), "Numark NS7FX (legacy; wheels unmapped)");
        assert_eq!(map_for("Numark NS7"), "Numark NS7 (original)");
        assert!(map_for("APC Mini mk2").contains("APC Mini"));
        assert!(map_for("Akai APC40 mk2").contains("APC40"));
        assert!(map_for("MPK Mini Plus").contains("Akai"));
    }

    #[test]
    fn identical_named_connections_preserve_source_channel_and_zero_velocity_release() {
        let first = next_source_id();
        let second = next_source_id();
        assert_ne!(first, second);
        let (tx, rx) = crate::engine::CommandPort::channel(32);
        let snapshot = Arc::new(Mutex::new(super::super::Snapshot::default()));
        let mut rt = super::super::RtEngine::new(48_000.0, rx, snapshot);
        let map = class_compliant();
        let log = Arc::new(Mutex::new(Vec::new()));
        let shift = Arc::new(Mutex::new([false; 4]));
        rt.selected_track = 1;
        handle_msg(&[0x93, 60, 100], first, &map, &tx, &log, &shift, "USB MIDI keyboard");
        rt.process(&mut []);
        rt.selected_track = 2;
        handle_msg(&[0x93, 60, 100], second, &map, &tx, &log, &shift, "USB MIDI keyboard");
        rt.process(&mut []);
        let first_key = super::super::dsp::InputKey::Midi { source: first, ch: 3, note: 60 };
        let second_key = super::super::dsp::InputKey::Midi { source: second, ch: 3, note: 60 };
        assert!(rt.tracks[1].poly.voices.iter().any(|voice| voice.input == Some(first_key) && voice.env.stage == 1));
        assert!(rt.tracks[2].poly.voices.iter().any(|voice| voice.input == Some(second_key) && voice.env.stage == 1));
        handle_msg(&[0x83, 60, 0], first, &map, &tx, &log, &shift, "USB MIDI keyboard");
        rt.process(&mut []);
        assert!(rt.tracks[1].poly.voices.iter().filter(|voice| voice.input == Some(first_key)).all(|voice| voice.env.stage == 4));
        assert!(rt.tracks[2].poly.voices.iter().any(|voice| voice.input == Some(second_key) && voice.env.stage == 1));
        rt.selected_track = 3;
        handle_msg(&[0x93, 60, 0], second, &map, &tx, &log, &shift, "USB MIDI keyboard");
        rt.process(&mut []);
        assert!(rt.tracks[2].poly.voices.iter().filter(|voice| voice.input == Some(second_key)).all(|voice| voice.env.stage == 4));
    }
}
