use super::policy::PolicyStatus;
use crate::engine::{CommandPort, Snapshot};
use midir::{MidiOutput, MidiOutputConnection};
use parking_lot::Mutex;
use std::collections::BTreeMap;
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering::Relaxed},
    Arc, Weak,
};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize)]
pub(crate) struct ChannelEffect {
    pub deck: u8,
    pub kind: crate::engine::channel_fx::Kind,
    pub name: &'static str,
    pub knob: f32,
    pub neutral: bool,
}
impl ChannelEffect {
    /// Expose the applied deck assignment and label to controller feedback consumers.
    /// Takes renderer-confirmed deck, type and knob; returns fixed feedback data without sending a device command.
    pub(crate) fn applied(deck: u8, kind: crate::engine::channel_fx::Kind, knob: f32) -> Self {
        Self { deck, kind, name: kind.name(), knob, neutral: (0.47..=0.53).contains(&knob) }
    }
}
impl Default for ChannelEffect {
    fn default() -> Self { Self::applied(0, crate::engine::channel_fx::Kind::Filter, 0.5) }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Surface {
    Sp1,
    Ns7,
    ApcMk2,
    Apc,
    Mpd232,
}

impl Surface {
    /// Prepare the controller connection.
    /// Takes no arguments; returns identity requests and the MkII host-mode introduction before LED updates.
    fn initialization(self) -> Vec<Vec<u8>> {
        let mut messages = Vec::new();
        if matches!(self, Self::Apc | Self::ApcMk2 | Self::Mpd232) {
            messages.push(vec![0xf0, 0x7e, 0x7f, 6, 1, 0xf7]);
        }
        if self == Self::ApcMk2 {
            let mut introduction = vec![0xf0, 0x47, 0x7f, 0x29, 0x60, 0, 4, 0x41];
            for version in [
                env!("CARGO_PKG_VERSION_MAJOR"),
                env!("CARGO_PKG_VERSION_MINOR"),
                env!("CARGO_PKG_VERSION_PATCH"),
            ] {
                introduction.push(version.parse::<u64>().unwrap_or(0).min(127) as u8);
            }
            introduction.push(0xf7);
            messages.push(introduction);
        }
        messages
    }

    #[cfg(test)]
    fn named(name: &str) -> Option<Self> {
        if super::connections::application_port(name) {
            return None;
        }
        let name = name.to_lowercase();
        if name.contains("ddj-sp1") {
            Some(Self::Sp1)
        } else if name.contains("ns7") && !name.contains("ns7fx") {
            Some(Self::Ns7)
        } else if name.contains("mpd232") || name.contains("mpd 232") {
            Some(Self::Mpd232)
        } else if name.contains("apc40") && (name.contains("mkii") || name.contains("mk2")) {
            Some(Self::ApcMk2)
        } else if name.contains("apc40") {
            Some(Self::Apc)
        } else {
            None
        }
    }

    fn messages(self, snapshot: &Snapshot) -> Vec<[u8; 3]> {
        let mut messages = Vec::new();
        match self {
            Self::Sp1 => {
                messages.push([0x9b, 9, 127]);
                for deck in 0..4 {
                    let Some(state) = snapshot.decks.get(deck % 2) else {
                        continue;
                    };
                    messages.push([0x90 + deck as u8, 0x58, if state.sync { 127 } else { 0 }]);
                    messages.push([0x90 + deck as u8, 0x5c, if state.sync { 127 } else { 0 }]);
                    messages.push([0x90 + deck as u8, 0x63, if state.keylock { 127 } else { 0 }]);
                    messages.push([0x90 + deck as u8, 0x55, if state.loop_on { 127 } else { 0 }]);
                    messages.push([0x90 + deck as u8, 0x40, if state.controls.slip { 127 } else { 0 }]);
                    messages.push([0x90 + deck as u8, 0x15, if state.controls.bleep { 127 } else { 0 }]);
                    messages.push([0x90 + deck as u8, 0x38, if state.controls.reverse { 127 } else { 0 }]);
                    messages.push([0x9b, deck as u8, if state.media_active { 127 } else { 0 }]);
                    for (pad, set) in state.hotcues.iter().take(8).enumerate() {
                        messages.push([0x97 + deck as u8, pad as u8, if *set { 127 } else { 0 }]);
                        messages.push([0x97 + deck as u8, 0x10 + pad as u8, if state.controls.roll == Some(pad as u8) { 127 } else { 0 }]);
                        messages.push([0x97 + deck as u8, 0x20 + pad as u8, if state.controls.slice == Some(pad as u8) { 127 } else { 0 }]);
                        let slot = deck % 2 * 8 + pad;
                        let loaded = snapshot.sampler_instances.get(snapshot.sampler_bank).is_some_and(|bank| bank.data.audio[slot].is_some());
                        let brightness = if snapshot.surfaces.sampler_playing[slot] { 127 } else if loaded { 63 } else { 0 };
                        for mode in [0x30, 0x70] { messages.push([0x97 + deck as u8, mode + pad as u8, brightness]); }
                        messages.push([0x97 + deck as u8, 0x40 + pad as u8, if state.controls.hotloops[pad] { 127 } else { 0 }]);
                        let beats = state.loop_len / f64::from(state.source_sample_rate.max(1)) * f64::from(state.bpm) / 60.0;
                        messages.push([0x97 + deck as u8, 0x50 + pad as u8, if state.loop_on && (beats - 2_f64.powi(pad as i32 - 5 + i32::from(state.controls.roll_scale))).abs() < 0.01 { 127 } else { 0 }]);
                        let manual = match pad { 1 | 6 => state.loop_on, 2 => state.controls.hotloops[usize::from(state.controls.loop_slot)], 4 | 5 => state.controls.loop_edit == (pad - 3) as u8, _ => false };
                        messages.push([0x97 + deck as u8, 0x60 + pad as u8, if manual { 127 } else { 0 }]);
                    }
                }
                for (bank, effects) in snapshot.surfaces.fx.iter().enumerate() {
                    for slot in 0..3 { messages.push([0x94 + bank as u8, 0x47 + slot as u8, if effects.on[slot] { 127 } else { 0 }]); }
                    for deck in 0..2 {
                        messages.push([0x96, 0x4c + (bank * 4 + deck) as u8, if effects.assigned[deck] { 127 } else { 0 }]);
                        messages.push([0x96, 0x5a + (bank * 2 + deck) as u8, if effects.assigned[deck] { 127 } else { 0 }]);
                    }
                }
            }
            Self::Ns7 => {
                for (deck, state) in snapshot.decks.iter().take(2).enumerate() {
                    messages.push([
                        0xb0,
                        [0x09, 0x1f][deck],
                        if state.playing { 127 } else { 0 },
                    ]);
                    messages.push([
                        0xb0,
                        [0x08, 0x1e][deck],
                        if state.media_active && !state.playing {
                            127
                        } else {
                            0
                        },
                    ]);
                    messages.push([0xb0, [0x07, 0x1d][deck], if state.sync { 127 } else { 0 }]);
                    messages.push([0xb0, [0x12, 0x29][deck], if state.vinyl { 0 } else { 127 }]);
                    for (address, on) in [
                        ([16, 39][deck], state.keylock),
                        ([10, 32][deck], state.controls.delete),
                        ([21, 44][deck], state.loop_on),
                        ([24, 47][deck], state.controls.auto_loop),
                    ] { messages.push([0xb0, address, if on { 127 } else { 0 }]); }
                    let loop_beats = state.loop_len / f64::from(state.source_sample_rate.max(1)) * f64::from(state.bpm) / 60.0;
                    for button in 0..4 {
                        let on = if state.controls.auto_loop { state.loop_on && (loop_beats - f64::from(1 << button)).abs() < 0.01 }
                            else { match button { 0 | 1 => state.loop_len > 1.0, 2 => state.loop_len > 1.0, _ => state.loop_on } };
                        messages.push([0xb0, [25, 48][deck] + button, if on { 127 } else { 0 }]);
                    }
                    for (pad, set) in state.hotcues.iter().take(5).enumerate() {
                        messages.push([0xb0, [11, 33][deck] + pad as u8, if *set { 127 } else { 0 }]);
                    }
                    let meter = if snapshot.meter_master { snapshot.master_meters[deck] } else { state.meter };
                    messages.push([0xb0, 52 + deck as u8, (meter * 127.0).clamp(0.0, 127.0) as u8]);
                }
                for (address, on) in [(0, snapshot.fader_start[0]), (1, snapshot.fader_start[1]), (2, snapshot.xfader_reverse)] {
                    messages.push([0xb0, address, if on { 127 } else { 0 }]);
                }
            }
            Self::ApcMk2 | Self::Apc => {
                for scene in 0..5 {
                    for track in 0..8 {
                        let state = snapshot.tracks.get(track + if self == Self::ApcMk2 { snapshot.surfaces.track_offset } else { 0 });
                        let scene_index = scene + if self == Self::ApcMk2 { snapshot.surfaces.scene_offset } else { 0 };
                        let occupied = state
                            .and_then(|state| state.clips.get(scene_index))
                            .is_some_and(|clip| clip.kind != 0 && !clip.properties.disabled);
                        let playing = occupied
                            && state.is_some_and(|state| state.playing_scene == scene_index as i16);
                        let color = if playing {
                            if self == Self::ApcMk2 { 21 } else { 1 }
                        } else if occupied {
                            if self == Self::ApcMk2 { 13 } else { 5 }
                        } else {
                            0
                        };
                        messages.push(if self == Self::ApcMk2 {
                            [0x90, ((4 - scene) * 8 + track) as u8, color]
                        } else {
                            [0x90 + track as u8, 0x35 + scene as u8, color]
                        });
                    }
                }
                if self == Self::ApcMk2 {
                    let status = snapshot.surfaces;
                    for track in 0..8 {
                        let state = snapshot.tracks.get(status.track_offset + track);
                        for (note, on) in [(0x30, state.is_some_and(|state| state.armed)), (0x31, state.is_some_and(|state| state.solo)),
                            (0x32, state.is_some_and(|state| !state.mute)), (0x33, snapshot.selected_track == status.track_offset + track)] {
                            messages.push([0x90 + track as u8, note, u8::from(on)]);
                        }
                        messages.push([0x90 + track as u8, 0x42, status.assignments[track]]);
                        let value = match status.knob_mode {
                            1 => status.sends[track][usize::from(status.send)],
                            2 => state.map_or(0.0, |state| state.gain / 1.5),
                            _ => state.map_or(0.5, |state| (state.pan + 1.0) * 0.5),
                        };
                        messages.push([0xb0, 0x38 + track as u8, if status.knob_mode == 0 { 3 } else { 2 }]);
                        messages.push([0xb0, 0x30 + track as u8, (value.clamp(0.0, 1.0) * 127.0).round() as u8]);
                        messages.push([0xb0, 0x18 + track as u8, 2]);
                        messages.push([0xb0, 0x10 + track as u8, (status.device_values[track].clamp(0.0, 1.0) * 127.0).round() as u8]);
                    }
                    for (note, on) in [(0x3e, status.device_on), (0x3f, status.device_lock.is_some()), (0x40, snapshot.view == 2),
                        (0x50, status.device_master),
                        (0x41, snapshot.fx_view >= 0), (0x57, status.knob_mode == 0), (0x58, status.knob_mode == 1),
                        (0x59, status.knob_mode == 2), (0x5a, snapshot.metronome), (0x5b, snapshot.playing),
                        (0x5d, snapshot.recording), (0x66, snapshot.recording), (0x67, status.bank_lock)] {
                        messages.push([0x90, note, u8::from(on)]);
                    }
                    for scene in 0..5 {
                        let scene_index = status.scene_offset + scene;
                        let playing = snapshot.tracks.iter().any(|track| track.playing_scene == scene_index as i16);
                        messages.push([0x90, 0x52 + scene as u8, u8::from(playing)]);
                    }
                }
            }
            Self::Mpd232 => {}
        }
        messages
    }
}

#[derive(Default)]
struct Counters {
    sent: AtomicU64,
    failed: AtomicU64,
    connected: AtomicU64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub struct Stats {
    pub sent: u64,
    pub failed: u64,
    pub connected: u64,
}

struct Output {
    incarnation:String,
    profile_hash:String,
    port_id:String,
    registry:Arc<super::catalog::runtime::Registry>,
    surface: Surface,
    connection: MidiOutputConnection,
    values: BTreeMap<(u8, u8), u8>,
    motors: Motors,
}

#[derive(Default)]
struct Motors([Option<bool>; 2]);

impl Motors {
    /// Follow renderer-confirmed deck transport with motor edges.
    /// Takes the current snapshot; returns start or stop commands only for changed deck states.
    fn pending(&self, snapshot: &Snapshot) -> ([bool; 2], Vec<[u8; 3]>) {
        let desired = std::array::from_fn(|deck| {
            !snapshot.performance.recovery && !snapshot.performance.stopped
                && snapshot.decks.get(deck).is_some_and(|state| state.media_active && state.playing && state.vinyl)
        });
        let messages = desired.iter().enumerate().filter_map(|(deck, running)| {
            (self.0[deck] != Some(*running)).then_some([0xb0, if *running { 65 } else { 66 } + deck as u8 * 10, 127])
        }).collect();
        (desired, messages)
    }

    fn applied(&mut self, desired: [bool; 2]) {
        self.0 = desired.map(Some);
    }
}

impl Drop for Output {
    fn drop(&mut self) {
        self.registry.output_result(&self.port_id,false,false,false);
        match self.surface {
            Surface::Ns7 => {
                for controller in [66, 76] { let _ = self.connection.send(&[0xb0, controller, 127]); }
            }
            Surface::Sp1 => { let _ = self.connection.send(&[0x9b, 9, 0]); }
            _ => {}
        }
    }
}

pub(super) struct Manager {
    stopped: Arc<AtomicBool>,
    worker: Option<std::thread::JoinHandle<()>>,
    counters: Arc<Counters>,
}

impl Manager {
    pub(super) fn start(
        snapshot: Weak<Mutex<Snapshot>>,
        cmd: CommandPort,
        input_counters: Arc<super::handoff::InputCounters>,
        policy: impl Fn() -> Arc<PolicyStatus> + Send + 'static,
        registry:Arc<super::catalog::runtime::Registry>,
    ) -> std::io::Result<Self> {
        let stopped = Arc::new(AtomicBool::new(false));
        let stop = stopped.clone();
        let counters = Arc::new(Counters::default());
        let counts = counters.clone();
        let worker = std::thread::Builder::new()
            .name("omatainer-midi-feedback".into())
            .spawn(move || {
                let mut outputs: BTreeMap<String, Output> = BTreeMap::new();
                let mut discover = Instant::now();
                let mut profile_view=None;
                while !stop.load(Relaxed) {
                    let Some(snapshot) = snapshot.upgrade() else {
                        break;
                    };
                    let policy = policy();
                    outputs.retain(|id, output| registry.output(id).is_some_and(|d|d.profile_hash==output.profile_hash && d.device.connection==output.incarnation && (policy.requested_policy.allows(&d.name)||policy.requested_policy.allows(&d.endpoint_name))));
                    if Instant::now() >= discover && !policy.pending() {
                        discover = Instant::now() + Duration::from_secs(2);
                        if let Ok(_permit) = cmd.performance().project_change() {
                            if let Ok(probe) = MidiOutput::new("omatainer-feedback-discover") {
                                let ports = probe.ports();
                                outputs.retain(|id, _| ports.iter().any(|port| port.id() == *id));
                                for port in ports {
                                    if outputs.contains_key(&port.id()) {
                                        continue;
                                    }

                                    let Some(device)=registry.output(&port.id()) else{continue;};
                                    let profile=device.profile.as_ref().unwrap();
                                    let surface=match profile.feedback{super::catalog::Driver::Ns7=>Surface::Ns7,super::catalog::Driver::Sp1=>Surface::Sp1,super::catalog::Driver::Apc40Mk2=>Surface::ApcMk2,super::catalog::Driver::Mpd232=>Surface::Mpd232,super::catalog::Driver::Generic=>continue};
                                    if !policy.requested_policy.allows(&device.name) && !policy.requested_policy.allows(&device.endpoint_name) {
                                        continue;
                                    }
                                    let Ok(midi) = MidiOutput::new("omatainer-feedback") else {
                                        continue;
                                    };
                                    match midi.connect(&port, "omatainer-feedback-out") {
                                        Ok(mut connection) => {
                                            let mut ready = true;
                                            let initialization=match profile.initialization{super::catalog::Initialization::None=>Vec::new(),super::catalog::Initialization::Inquiry=>vec![vec![0xf0,0x7e,0x7f,6,1,0xf7]],super::catalog::Initialization::Apc40Mk2Host41=>surface.initialization()};
                                            let initialization_sent = !initialization.is_empty();
                                            for message in initialization {
                                                if connection.send(&message).is_ok() {
                                                    counts.sent.fetch_add(1, Relaxed);
                                                } else {
                                                    counts.failed.fetch_add(1, Relaxed);
                                                    ready = false;
                                                    break;
                                                }
                                            }
                                            registry.output_result(&port.id(),ready,ready&&initialization_sent,!ready);
                                            if !ready {
                                                continue;
                                            }
                                            outputs.insert(
                                                port.id(),
                                                Output {
                                                    incarnation:device.device.connection,profile_hash:device.profile_hash,port_id:port.id(),registry:registry.clone(),
                                                    surface,
                                                    connection,
                                                    values: BTreeMap::new(),
                                                    motors: Motors::default(),
                                                },
                                            );
                                        }
                                        Err(_) => {
                                            counts.failed.fetch_add(1, Relaxed);
                                        }
                                    }
                                }
                            }
                        }
                    }
                    counts.connected.store(outputs.len() as u64, Relaxed);
                    let updates = {
                        let snapshot = snapshot.lock();
                        outputs
                            .iter()
                            .map(|(id, output)| {
                                let (desired, motors) = output.motors.pending(&snapshot);
                                (id.clone(), output.surface.messages(&snapshot), desired,
                                    if output.surface == Surface::Ns7 { motors } else { Vec::new() })
                            })
                            .collect::<Vec<_>>()
                    };
                    for (id, messages, desired, motors) in updates {
                        let Some(output) = outputs.get_mut(&id) else {
                            continue;
                        };
                        let mut failed = false;
                        for [status, data, value] in messages {
                            if output.values.get(&(status, data)) == Some(&value) {
                                continue;
                            }
                            if output.connection.send(&[status, data, value]).is_err() {
                                counts.failed.fetch_add(1, Relaxed);
                                failed = true;
                                break;
                            }
                            counts.sent.fetch_add(1, Relaxed);
                            output.values.insert((status, data), value);
                        }
                        if !failed {
                            for message in motors {
                                if output.connection.send(&message).is_err() {
                                    counts.failed.fetch_add(1, Relaxed);
                                    failed = true;
                                    break;
                                }
                                counts.sent.fetch_add(1, Relaxed);
                            }
                            if !failed { output.motors.applied(desired); }
                        }
                        if failed {
                            registry.output_result(&id,false,false,true);
                            outputs.remove(&id);
                        }
                    }
                    snapshot.lock().midi_feedback = Stats {
                        sent: counts.sent.load(Relaxed),
                        failed: counts.failed.load(Relaxed),
                        connected: outputs.len() as u64,
                    };
                    snapshot.lock().midi_input = input_counters.snapshot();
                    let view=registry.view();if profile_view.as_ref().is_none_or(|old|!Arc::ptr_eq(old,&view)){snapshot.lock().midi_profiles=registry.receipt();profile_view=Some(view);}
                    std::thread::park_timeout(Duration::from_millis(16));
                }
                outputs.clear();
                counts.connected.store(0, Relaxed);
            })?;
        Ok(Self {
            stopped,
            worker: Some(worker),
            counters,
        })
    }

    pub(super) fn stats(&self) -> Stats {
        Stats {
            sent: self.counters.sent.load(Relaxed),
            failed: self.counters.failed.load(Relaxed),
            connected: self.counters.connected.load(Relaxed),
        }
    }
}

impl Drop for Manager {
    fn drop(&mut self) {
        self.stopped.store(true, Relaxed);
        if let Some(worker) = self.worker.take() {
            worker.thread().unpark();
            let _ = worker.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn channel_effect_feedback_exposes_actual_assignment_name_position_and_detent_without_wire_commands() {
        use crate::engine::{channel_fx::Kind, Command, Engine};
        let (engine, mut renderer) = Engine::headless_for_test(48000, 128);
        for (deck, kind, knob) in [(0, Kind::Echo, 0.12), (1, Kind::Room, 0.51)] {
            engine.send(Command::DeckChannelEffect { deck, effect: kind }).unwrap();
            engine.send(Command::DeckFilter { deck, value: knob }).unwrap();
        }
        renderer.process(&mut []); renderer.publish_for_test(); let snapshot = engine.snapshot();
        for (deck, kind, knob) in [(0, Kind::Echo, 0.12), (1, Kind::Room, 0.51)] {
            let frame = snapshot.decks[deck].channel_effect_feedback;
            assert_eq!(frame.deck, deck as u8); assert_eq!(frame.kind, kind); assert_eq!(frame.name, kind.name()); assert_eq!(frame.knob, knob); assert_eq!(frame.neutral, deck == 1);
            let json = serde_json::to_value(frame).unwrap(); assert_eq!(json["name"], kind.name());
        }
        assert_eq!(snapshot.midi_feedback.connected, 0); assert_eq!(snapshot.midi_feedback.sent, 0);
    }
    #[test]
    fn ns7_controls_feedback_tracks_keylock_hotcues_loop_and_hardware_switches() {
        let mut snapshot = Snapshot::default(); snapshot.decks = vec![Default::default(), Default::default()];
        snapshot.fader_start = [true, false]; snapshot.xfader_reverse = true;
        for deck in &mut snapshot.decks { deck.keylock = true; deck.hotcues[0] = true; deck.loop_on = true; deck.controls.auto_loop = true; deck.controls.delete = true; }
        let messages = Surface::Ns7.messages(&snapshot);
        for cc in [0,2,16,39,11,33,21,44,24,47,10,32] { assert!(messages.contains(&[0xb0,cc,127]), "CC {cc}"); }
        assert!(messages.contains(&[0xb0,1,0]));
        snapshot.decks[0].keylock = false; snapshot.decks[0].hotcues[0] = false;
        let messages = Surface::Ns7.messages(&snapshot); assert!(messages.contains(&[0xb0,16,0])); assert!(messages.contains(&[0xb0,11,0]));
        assert!(messages.iter().all(|message| !matches!(message[1], 65..=80)), "LED refresh must never contain motor actions");
    }
    #[test]
    fn existing_sp1_direction_feedback_uses_actual_renderer_status_without_output_ports() {
        use crate::engine::{Command,deck_controls::{Button,Control}};
        let(engine,mut renderer)=crate::engine::Engine::headless_for_test(48000,64);
        renderer.apply(Command::DeckControl {source:71,deck:0,control:Control::Reverse {enabled:true}});
        renderer.apply(Command::DeckControl {source:72,deck:1,control:Control::Hold {button:Button::Bleep,on:true}});
        renderer.publish_for_test();
        let messages=Surface::Sp1.messages(&engine.snapshot());
        for expected in [[0x90,0x38,127],[0x90,0x15,0],[0x91,0x38,0],[0x91,0x15,127]] {assert!(messages.contains(&expected),"Missing {expected:?}");}
        renderer.apply(Command::DeckControl {source:71,deck:0,control:Control::Reverse {enabled:false}});
        renderer.apply(Command::DeckControl {source:72,deck:1,control:Control::Hold {button:Button::Bleep,on:false}});
        renderer.publish_for_test();let messages=Surface::Sp1.messages(&engine.snapshot());
        assert!(messages.contains(&[0x90,0x38,0]));assert!(messages.contains(&[0x91,0x15,0]));
        assert!(messages.iter().all(|message|message[0]&0xf0==0x90 && message[1]<128 && message[2]<128));
    }
    #[test]
    fn ns7_motors_repeat_play_pause_edges_and_stop_for_vinyl_off_or_recovery() {
        let mut snapshot = Snapshot::default();
        snapshot.decks = vec![Default::default(), Default::default()];
        for deck in &mut snapshot.decks { deck.media_active = true; deck.vinyl = true; }
        let mut motors = Motors::default();
        let (desired, messages) = motors.pending(&snapshot);
        assert_eq!(messages, [[0xb0,66,127], [0xb0,76,127]]);
        motors.applied(desired);
        for _ in 0..3 {
            snapshot.decks[0].playing = true;
            let (desired, messages) = motors.pending(&snapshot);
            assert_eq!(messages, [[0xb0,65,127]]);
            motors.applied(desired);
            assert!(motors.pending(&snapshot).1.is_empty());
            snapshot.decks[0].playing = false;
            let (desired, messages) = motors.pending(&snapshot);
            assert_eq!(messages, [[0xb0,66,127]]);
            motors.applied(desired);
        }
        snapshot.decks[1].playing = true;
        let (desired, messages) = motors.pending(&snapshot);
        assert_eq!(messages, [[0xb0,75,127]]);
        motors.applied(desired);
        snapshot.decks[1].vinyl = false;
        let (desired, messages) = motors.pending(&snapshot);
        assert_eq!(messages, [[0xb0,76,127]]);
        motors.applied(desired);
        snapshot.decks[1].vinyl = true;
        let (desired, messages) = motors.pending(&snapshot);
        assert_eq!(messages, [[0xb0,75,127]]);
        motors.applied(desired);
        snapshot.performance.recovery = true;
        assert_eq!(motors.pending(&snapshot).1, [[0xb0,76,127]]);
    }
    #[test]
    fn apc_mk2_initialization_selects_host_button_control_before_led_updates() {
        let messages = Surface::ApcMk2.initialization();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0], [0xf0, 0x7e, 0x7f, 6, 1, 0xf7]);
        let introduction = &messages[1];
        assert_eq!(introduction.len(), 12);
        assert_eq!(introduction[..8], [0xf0, 0x47, 0x7f, 0x29, 0x60, 0, 4, 0x41]);
        assert!(introduction[8..11].iter().all(|value| *value < 128));
        assert_eq!(introduction[11], 0xf7);
        assert_eq!(Surface::Apc.initialization(), messages[..1]);
        assert_eq!(Surface::Mpd232.initialization(), messages[..1]);
        assert!(Surface::Ns7.initialization().is_empty());
        assert!(Surface::Sp1.initialization().is_empty());
    }

    #[test]
    fn apc_clip_feedback_preserves_each_generations_colors_and_addresses() {
        let mut snapshot = Snapshot::default();
        snapshot.tracks = (0..8)
            .map(|track| crate::engine::TrackSnap {
                playing_scene: (track % 5) as i16,
                clips: (0..5)
                    .map(|scene| crate::engine::ClipSnap {
                        kind: ((track + scene) % 3) as u8,
                        ..Default::default()
                    })
                    .collect(),
                ..Default::default()
            })
            .collect();
        for (surface, playing_color, loaded_color) in [(Surface::Apc, 1, 5), (Surface::ApcMk2, 21, 13)] {
            let messages = surface.messages(&snapshot);
            let first_notes = [32, 24, 16, 8, 0];
            assert_eq!(messages.iter().filter(|message| if surface == Surface::ApcMk2 { message[0] == 0x90 && message[1] < 40 } else { (0x90..=0x97).contains(&message[0]) && (0x35..=0x39).contains(&message[1]) }).count(), 40);
            for track in 0..8 {
                for scene in 0..5 {
                    let state = &snapshot.tracks[track];
                    let expected = if state.clips[scene].kind == 0 { 0 }
                        else if state.playing_scene == scene as i16 { playing_color }
                        else { loaded_color };
                    let address = if surface == Surface::ApcMk2 {
                        [0x90, first_notes[scene] + track as u8, expected]
                    } else { [0x90 + track as u8, 0x35 + scene as u8, expected] };
                    assert!(messages.contains(&address), "{surface:?}: track {track}, scene {scene}");
                }
            }
        }
    }

    #[test]
    fn own_ports_never_become_hardware_feedback_targets() {
        for name in [
            "omatainer-input:omatainer-in-Pioneer DDJ-SP1 128:0",
            "omatainer-feedback:Numark NS7 129:0",
        ] {
            assert!(super::super::connections::application_port(name));
            assert_eq!(Surface::named(name), None);
        }
        assert_eq!(
            Surface::named("Pioneer DDJ-SP1:Pioneer DDJ-SP1 MIDI 1 24:0"),
            Some(Surface::Sp1)
        );
        assert_eq!(
            Surface::named("Numark NS7:Numark NS7 MIDI 32:0"),
            Some(Surface::Ns7)
        );
    }

    #[test]
    fn apc_mk2_grid_feedback_keeps_top_rows_at_high_notes_after_banking() {
        let mut snapshot = Snapshot::default();
        snapshot.surfaces.track_offset = 8;
        snapshot.surfaces.scene_offset = 6;
        snapshot.tracks = (0..16)
            .map(|track| crate::engine::TrackSnap {
                playing_scene: 7,
                clips: (0..8)
                    .map(|scene| crate::engine::ClipSnap {
                        kind: if scene == 6 && track % 2 == 0 || scene == 7 { 1 } else { 0 },
                        ..Default::default()
                    })
                    .collect(),
                ..Default::default()
            })
            .collect();
        let messages = Surface::ApcMk2.messages(&snapshot);
        let first_notes = [32, 24, 16, 8, 0];
        for (row, first_note) in first_notes.iter().enumerate() {
            for column in 0..8 {
                let expected = match row {
                    0 if column % 2 == 0 => 13,
                    1 => 21,
                    _ => 0,
                };
                assert!(messages.contains(&[0x90, first_note + column, expected]));
            }
        }
    }
    #[test]
    fn feedback_uses_documented_led_addresses_and_mirrors_the_sp1_banks() {
        let mut snapshot = Snapshot::default();
        snapshot.decks = vec![Default::default(), Default::default()];
        snapshot.decks[0].sync = true;
        snapshot.decks[1].hotcues[7] = true;
        let messages = Surface::Sp1.messages(&snapshot);
        assert!(messages.contains(&[0x9b, 9, 127]));
        assert!(messages.contains(&[0x90, 0x58, 127]));
        assert!(messages.contains(&[0x92, 0x58, 127]));
        assert!(messages.contains(&[0x98, 7, 127]));
        assert!(messages.contains(&[0x9a, 7, 127]));
        snapshot.decks[0].playing = true;
        assert!(Surface::Ns7
            .messages(&snapshot)
            .contains(&[0xb0, 0x09, 127]));
        assert!(!Surface::Ns7
            .messages(&snapshot)
            .iter()
            .any(|message| message[0] != 0xb0));
        snapshot.decks[1].media_active = true;
        assert!(Surface::Ns7
            .messages(&snapshot)
            .contains(&[0xb0, 0x1e, 127]));
        let before = Surface::Ns7.messages(&snapshot);
        snapshot.decks[0].pfl = true;
        snapshot.decks[1].pfl = true;
        assert_eq!(before, Surface::Ns7.messages(&snapshot));
    }

    #[test]
    fn surfaces_feedback_follows_modes_rings_and_held_pad_states() {
        let mut snapshot = Snapshot::default();
        snapshot.tracks = (0..16).map(|_| crate::engine::TrackSnap::default()).collect();
        snapshot.surfaces.track_offset = 8;
        snapshot.tracks[9].solo = true;
        snapshot.tracks[9].armed = true;
        snapshot.surfaces.assignments[1] = 2;
        snapshot.surfaces.knob_mode = 1;
        snapshot.surfaces.send = 1;
        snapshot.surfaces.sends[1][1] = 1.0;
        snapshot.surfaces.device_values[0] = 0.5;
        let messages = Surface::ApcMk2.messages(&snapshot);
        for message in [[0x91, 0x30, 1], [0x91, 0x31, 1], [0x91, 0x42, 2], [0xb0, 0x39, 2], [0xb0, 0x31, 127], [0xb0, 0x10, 64]] {
            assert!(messages.contains(&message), "{message:?}");
        }
        snapshot.decks = vec![Default::default(), Default::default()];
        snapshot.decks[0].controls.roll = Some(5);
        snapshot.decks[1].controls.slice = Some(7);
        snapshot.decks[0].controls.slip = true;
        snapshot.surfaces.sampler_playing[8] = true;
        snapshot.surfaces.fx[1].on[2] = true;
        snapshot.surfaces.fx[1].assigned[0] = true;
        let messages = Surface::Sp1.messages(&snapshot);
        for message in [[0x97, 0x15, 127], [0x98, 0x27, 127], [0x90, 0x40, 127], [0x98, 0x70, 127], [0x95, 0x49, 127], [0x96, 0x50, 127]] {
            assert!(messages.contains(&message), "{message:?}");
        }
        assert!(Surface::Mpd232.messages(&snapshot).is_empty());
    }
}
