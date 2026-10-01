//! USB-MIDI class-compliant I/O, hardware maps, learn, and clock.

use crate::engine::{Command, DECKS, HOTCUES, SCENES, TRACKS};
use midir::{Ignore, MidiInput, MidiInputConnection, MidiOutput, MidiOutputConnection};
use parking_lot::Mutex;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

mod profile;
mod handoff;
mod framing;
mod relative;
pub use handoff::InputStats;
pub use relative::RelativeSpec;
#[cfg(test)]
use relative::RelativeEncoding;
#[cfg(test)]
mod profile_tests;
#[cfg(test)]
mod realtime_tests;
#[cfg(test)]
mod relative_tests;

static NEXT_SOURCE: AtomicU64 = AtomicU64::new(1);

fn next_source_id() -> u64 {
    NEXT_SOURCE.fetch_add(1, Ordering::Relaxed)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MsgKind {
    Note,
    Cc,
    CcRel,
    Pitch,
}

#[derive(Clone, Copy, Debug)]
pub struct Binding {
    pub kind: MsgKind,
    pub ch: u8, // 0-15, 0xFF = any
    pub data: u8,
    pub action: Action,
    pub deck: u8,
    pub extra: u8,
    pub relative: Option<RelativeSpec>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    DeckPlay,
    DeckCue,
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
    DeckLoad,
    DeckVinyl,
    Xfader,
    Master,
    CueMix,
    Browse,
    LoadA,
    LoadB,
    Scene,
    Clip,
    TrackFader,
    TrackMute,
    Play,
    Stop,
    Record,
    Tap,
    Shift,
    FxWet,
    FxSelect,
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

#[derive(Clone, Debug)]
pub struct MidiDevice {
    pub name: String,
    pub map: String,
}

pub struct MidiHub {
    _ins: Vec<MidiInputConnection<()>>,
    // Connections close before worker guards join, ending callback ownership.
    _workers: Vec<handoff::InputGuard>,
    input_counters: Arc<handoff::InputCounters>,
    outs: Arc<Mutex<Vec<MidiOutputConnection>>>,
    pub devices: Arc<Mutex<Vec<MidiDevice>>>,
    pub log: Arc<Mutex<Vec<String>>>,
    pub learn: Arc<Mutex<Option<String>>>,
}

impl MidiHub {
    #[cfg(test)]
    pub(crate) fn receive_for_test(&self, cmd: &super::CommandPort, source: u64, device: &str, message: &[u8]) {
        let map = pick_map(&builtin_maps().unwrap(), device);
        let counters = Arc::new(handoff::InputCounters::default());
        let (mut callback, worker) = handoff::start(
            source, map, cmd.clone(), self.log.clone(), self.learn.clone(), device.into(), counters.clone(),
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

    #[cfg(test)]
    pub(super) fn without_devices() -> Self {
        Self {
            _ins: Vec::new(),
            _workers: Vec::new(),
            input_counters: Arc::new(handoff::InputCounters::default()),
            outs: Arc::new(Mutex::new(Vec::new())),
            devices: Arc::new(Mutex::new(Vec::new())),
            log: Arc::new(Mutex::new(Vec::new())),
            learn: Arc::new(Mutex::new(None)),
        }
    }

    pub fn start(cmd: super::CommandPort) -> anyhow::Result<Self> {
        // Fail profile validation before a device callback can dispatch it.
        let maps = builtin_maps()?;
        let devices = Arc::new(Mutex::new(Vec::new()));
        let log = Arc::new(Mutex::new(Vec::new()));
        let learn = Arc::new(Mutex::new(None));
        let input_counters = Arc::new(handoff::InputCounters::default());
        let outs = Arc::new(Mutex::new(Vec::new()));
        let mut ins = Vec::new();
        let mut workers = Vec::new();

        let in_ports: Vec<(String, midir::MidiInputPort)> = match MidiInput::new("omatainer") {
            Ok(probe) => probe
                .ports()
                .into_iter()
                .filter_map(|p| probe.port_name(&p).ok().map(|n| (n, p)))
                .collect(),
            Err(_) => Vec::new(),
        };
        for (idx, (name, port)) in in_ports.into_iter().enumerate() {
            if name.to_lowercase().contains("through") {
                continue;
            }
            let Ok(mut midi_in) = MidiInput::new(&format!("omatainer-in-{idx}")) else {
                break;
            };
            midi_in.ignore(Ignore::None);
            let map = pick_map(&maps, &name);
            devices.lock().push(MidiDevice {
                name: name.clone(),
                map: map.name.clone(),
            });
            // Names and channels are not identities: two identical keyboards
            // can use the same channel and pitch simultaneously.
            let source = next_source_id();
            let (mut input, worker) = handoff::start(source, map, cmd.clone(),
                log.clone(), learn.clone(), name.clone(), input_counters.clone())?;
            match midi_in.connect(
                &port,
                &format!("omatainer-in-{name}"),
                move |_t, msg, _| {
                    input.push(msg);
                },
                (),
            ) {
                Ok(conn) => {
                    ins.push(conn);
                    workers.push(worker);
                }
                Err(e) => {
                    log.lock().push(format!("in fail {name}: {e}"));
                }
            }
        }

        if let Ok(probe) = MidiOutput::new("omatainer") {
            let out_ports: Vec<(String, midir::MidiOutputPort)> = probe
                .ports()
                .into_iter()
                .filter_map(|p| probe.port_name(&p).ok().map(|n| (n, p)))
                .collect();
            drop(probe);
            for (name, port) in out_ports {
                if name.to_lowercase().contains("through") {
                    continue;
                }
                if let Ok(midi_out) = MidiOutput::new("omatainer") {
                    if let Ok(c) = midi_out.connect(&port, &format!("omatainer-out-{name}")) {
                        outs.lock().push(c);
                        break;
                    }
                }
            }
        }

        if devices.lock().is_empty() {
            devices.lock().push(MidiDevice {
                name: "keyboard + mouse".into(),
                map: "built-in".into(),
            });
        }

        Ok(Self {
            _ins: ins,
            _workers: workers,
            input_counters,
            outs,
            devices,
            log,
            learn,
        })
    }

    pub fn input_stats(&self) -> InputStats {
        self.input_counters.snapshot()
    }

    pub fn send_clock_tick(&self) {
        if let Some(out) = self.outs.lock().first_mut() {
            let _ = out.send(&[0xF8]);
        }
    }

    pub fn send_clock_start(&self, start: bool) {
        if let Some(out) = self.outs.lock().first_mut() {
            let _ = out.send(&[if start { 0xFA } else { 0xFC }]);
        }
    }

    pub fn note_led(&self, ch: u8, note: u8, vel: u8) {
        if let Some(out) = self.outs.lock().first_mut() {
            let _ = out.send(&[0x90 | (ch & 0x0F), note, vel]);
        }
    }
}

fn handle_msg(
    msg: &[u8],
    source: u64,
    map: &MidiMap,
    cmd: &super::CommandPort,
    log: &Arc<Mutex<Vec<String>>>,
    learn: &Arc<Mutex<Option<String>>>,
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
                    0xfc => Some(Command::Stop),
                    0xf8 => Some(Command::MidiClock { source }),
                    _ => None, // Continue/sensing/reset/reserved: no handler yet.
                };
                if let Some(command) = command {
                    let _ = cmd.send(command);
                }
            }
            framing::Message::Channel(frame) => {
                handle_channel(&frame, source, map, cmd, log, learn, shift, dev);
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
    learn: &Arc<Mutex<Option<String>>>,
    shift: &Arc<Mutex<[bool; 4]>>,
    dev: &str,
) {
    let st = msg[0];
    let kind_hi = st & 0xF0;
    let ch = st & 0x0F;
    let d1 = msg[1];
    let d2 = msg[2];

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

    if let Some(param) = learn.lock().as_ref() {
        let _ = cmd.send(Command::LearnCapture {
            param: param.clone(),
            ch,
            d1,
            d2,
            status: kind_hi,
        });
        return;
    }

    let mut matched = false;
    for b in &map.bindings {
        if b.ch != 0xFF && b.ch != ch {
            continue;
        }
        let hit = match b.kind {
            MsgKind::Note => kind_hi == 0x90 || kind_hi == 0x80,
            MsgKind::Cc | MsgKind::CcRel => kind_hi == 0xB0,
            MsgKind::Pitch => kind_hi == 0xE0,
        };
        if !hit {
            continue;
        }
        if b.kind != MsgKind::Pitch && b.data != d1 {
            continue;
        }
        matched = true;
        dispatch(b, source, kind_hi, d2, msg, cmd, shift);
    }

    // Live MIDI notes onto the selected track when no map consumed a note
    // (generic class-compliant keyboards / Akai MPK keys).
    if !matched && map.unmapped_notes == UnmappedNotes::Live {
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
) {
    let pressed = status == 0x90 && d2 > 0;
    let rel = match b.kind {
        MsgKind::CcRel => {
            let Some(delta) = b.relative.and_then(|spec| spec.decode(d2)) else {
                return;
            };
            // A stationary report must not reset an active scratch/nudge.
            if delta == 0.0 {
                return;
            }
            delta
        }
        MsgKind::Pitch => {
            let v = (msg[1] as u16) | ((msg[2] as u16) << 7);
            (v as f32 - 8192.0) / 8192.0
        }
        _ => d2 as f32 / 127.0,
    };
    let deck = b.deck.min((DECKS - 1) as u8);
    match b.action {
        Action::Shift => shift.lock()[deck as usize] = pressed,
        Action::DeckPlay if pressed => {
            let _ = cmd.send(Command::DeckPlay { deck });
        }
        Action::DeckCue if pressed => {
            let _ = cmd.send(Command::DeckCue { deck });
        }
        Action::DeckSync if pressed => {
            let _ = cmd.send(Command::DeckSync { deck });
        }
        Action::DeckJog => {
            let _ = cmd.send(Command::DeckJog {
                deck,
                delta: if b.kind == MsgKind::CcRel {
                    rel
                } else {
                    rel * 0.35
                },
            });
        }
        Action::DeckJogTouch => {
            let _ = cmd.send(Command::MidiDeckTouch {
                source,
                deck,
                on: pressed,
            });
        }
        Action::DeckPitch => {
            let _ = cmd.send(Command::DeckPitch { deck, value: rel });
        }
        Action::DeckGain => {
            let _ = cmd.send(Command::DeckGain { deck, value: rel });
        }
        Action::DeckEqHi => {
            let _ = cmd.send(Command::DeckEq {
                deck,
                band: 2,
                value: rel,
            });
        }
        Action::DeckEqMid => {
            let _ = cmd.send(Command::DeckEq {
                deck,
                band: 1,
                value: rel,
            });
        }
        Action::DeckEqLow => {
            let _ = cmd.send(Command::DeckEq {
                deck,
                band: 0,
                value: rel,
            });
        }
        Action::DeckFilter => {
            let _ = cmd.send(Command::DeckFilter { deck, value: rel });
        }
        Action::DeckPfl if pressed => {
            let _ = cmd.send(Command::DeckPfl { deck });
        }
        Action::DeckHotCue if pressed => {
            let _ = cmd.send(Command::DeckHotCue {
                deck,
                pad: b.extra.min((HOTCUES - 1) as u8),
                del: shift.lock()[deck as usize],
            });
        }
        Action::DeckLoop4 if pressed => {
            let _ = cmd.send(Command::DeckLoop { deck, beats: 4.0 });
        }
        Action::DeckLoopIn if pressed => {
            let _ = cmd.send(Command::DeckLoopIn { deck });
        }
        Action::DeckLoopOut if pressed => {
            let _ = cmd.send(Command::DeckLoopOut { deck });
        }
        Action::DeckLoad if pressed => {
            let _ = cmd.send(Command::DeckLoadSelected { deck });
        }
        Action::DeckVinyl if pressed => {
            let _ = cmd.send(Command::DeckVinyl { deck });
        }
        Action::Xfader => {
            let _ = cmd.send(Command::Xfader(rel));
        }
        Action::Master => {
            let _ = cmd.send(Command::Master(rel));
        }
        Action::CueMix => {
            let _ = cmd.send(Command::CueMix(rel));
        }
        Action::Browse => {
            let _ = cmd.send(Command::Browse(rel));
        }
        Action::LoadA if pressed => {
            let _ = cmd.send(Command::DeckLoadSelected { deck: 0 });
        }
        Action::LoadB if pressed => {
            let _ = cmd.send(Command::DeckLoadSelected { deck: 1 });
        }
        Action::Scene if pressed => {
            let _ = cmd.send(Command::LaunchScene {
                scene: b.extra.min((SCENES - 1) as u8),
            });
        }
        Action::Clip if pressed => {
            let _ = cmd.send(Command::LaunchClip {
                track: b.deck.min((TRACKS - 1) as u8),
                scene: b.extra.min((SCENES - 1) as u8),
            });
        }
        Action::TrackFader => {
            let _ = cmd.send(Command::TrackGain {
                track: b.extra.min((TRACKS - 1) as u8),
                value: rel,
            });
        }
        Action::TrackMute if pressed => {
            let _ = cmd.send(Command::Mute {
                track: b.extra.min((TRACKS - 1) as u8),
            });
        }
        Action::Play if pressed => {
            let _ = cmd.send(Command::TogglePlay);
        }
        Action::Stop if pressed => {
            let _ = cmd.send(Command::Stop);
        }
        Action::Record if pressed => {
            let _ = cmd.send(Command::Record);
        }
        Action::Tap if pressed => {
            let _ = cmd.send(Command::Tap(Instant::now()));
        }
        Action::FxWet => {
            let _ = cmd.send(Command::FxWet {
                slot: b.extra,
                value: rel,
            });
        }
        Action::FxSelect if pressed => {
            let _ = cmd.send(Command::FxSelect { slot: b.extra });
        }
        _ => {}
    }
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
        extra,
        relative: None,
    }
}
fn cbind(ch: u8, cc: u8, action: Action, deck: u8, extra: u8) -> Binding {
    Binding {
        kind: MsgKind::Cc,
        ch,
        data: cc,
        action,
        deck,
        extra,
        relative: None,
    }
}
fn rbind(ch: u8, cc: u8, action: Action, deck: u8, extra: u8, relative: RelativeSpec) -> Binding {
    Binding {
        kind: MsgKind::CcRel,
        ch,
        data: cc,
        action,
        deck,
        extra,
        relative: Some(relative),
    }
}

pub fn builtin_maps() -> anyhow::Result<Vec<MidiMap>> {
    let mut maps = Vec::new();
    maps.push(pioneer_ddj_fx());
    maps.push(numark_ns7(true));
    maps.push(numark_ns7(false));
    maps.push(akai_apc_mini());
    // The specific MkII name must precede the original's broader matcher.
    maps.push(akai_apc40_mk2());
    maps.push(akai_apc40());
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
        b.push(nbind(ch, 0x0C, Action::DeckCue, deck, 0));
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
    let mut b = Vec::new();
    for deck in 0..2u8 {
        let ch = deck;
        b.push(nbind(ch, 0x0C, Action::DeckPlay, deck, 0));
        b.push(nbind(ch, 0x0D, Action::DeckCue, deck, 0));
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
    // Akai APC40 Mk2 protocol v1.2, pp. 30-34: forty distinct clip notes,
    // with the same eight per-channel CC7 track faders as the original.
    for scene in 0..5u8 {
        for track in 0..8u8 {
            b.push(nbind(0xff, scene * 8 + track, Action::Clip, track, scene));
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
        assert_eq!(map_for("Numark NS7"), "Numark NS7 (legacy; wheels unmapped)");
        assert!(map_for("APC Mini mk2").contains("APC Mini"));
        assert!(map_for("Akai APC40 mk2").contains("APC40"));
        assert!(map_for("MPK Mini Plus").contains("Akai"));
    }

    #[test]
    fn identical_named_connections_preserve_source_channel_and_zero_velocity_release() {
        let first = next_source_id();
        let second = next_source_id();
        assert_ne!(first, second);
        let (tx, rx) = crate::engine::CommandPort::channel(16);
        let snapshot = Arc::new(Mutex::new(super::super::Snapshot::default()));
        let mut rt = super::super::RtEngine::new(48_000.0, rx, snapshot);
        let map = class_compliant();
        let log = Arc::new(Mutex::new(Vec::new()));
        let learn = Arc::new(Mutex::new(None));
        let shift = Arc::new(Mutex::new([false; 4]));
        rt.selected_track = 1;
        handle_msg(&[0x93, 60, 100], first, &map, &tx, &log, &learn, &shift, "USB MIDI keyboard");
        rt.process(&mut []);
        rt.selected_track = 2;
        handle_msg(&[0x93, 60, 100], second, &map, &tx, &log, &learn, &shift, "USB MIDI keyboard");
        rt.process(&mut []);
        let first_key = super::super::dsp::InputKey::Midi { source: first, ch: 3, note: 60 };
        let second_key = super::super::dsp::InputKey::Midi { source: second, ch: 3, note: 60 };
        assert!(rt.tracks[1].poly.voices.iter().any(|voice| voice.input == Some(first_key) && voice.env.stage == 1));
        assert!(rt.tracks[2].poly.voices.iter().any(|voice| voice.input == Some(second_key) && voice.env.stage == 1));
        handle_msg(&[0x83, 60, 0], first, &map, &tx, &log, &learn, &shift, "USB MIDI keyboard");
        rt.process(&mut []);
        assert!(rt.tracks[1].poly.voices.iter().filter(|voice| voice.input == Some(first_key)).all(|voice| voice.env.stage == 4));
        assert!(rt.tracks[2].poly.voices.iter().any(|voice| voice.input == Some(second_key) && voice.env.stage == 1));
        rt.selected_track = 3;
        handle_msg(&[0x93, 60, 0], second, &map, &tx, &log, &learn, &shift, "USB MIDI keyboard");
        rt.process(&mut []);
        assert!(rt.tracks[2].poly.voices.iter().filter(|voice| voice.input == Some(second_key)).all(|voice| voice.env.stage == 4));
    }
}
