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
                    messages.push([0x90 + deck as u8, 0x55, if state.loop_on { 127 } else { 0 }]);
                    for (pad, set) in state.hotcues.iter().take(8).enumerate() {
                        messages.push([0x97 + deck as u8, pad as u8, if *set { 127 } else { 0 }]);
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
                }
            }
            Self::ApcMk2 | Self::Apc => {
                for scene in 0..5 {
                    for track in 0..8 {
                        let state = snapshot.tracks.get(track);
                        let occupied = state
                            .and_then(|state| state.clips.get(scene))
                            .is_some_and(|clip| clip.kind != 0);
                        let playing = occupied
                            && state.is_some_and(|state| state.playing_scene == scene as i16);
                        let color = if playing {
                            if self == Self::ApcMk2 { 21 } else { 1 }
                        } else if occupied {
                            if self == Self::ApcMk2 { 13 } else { 5 }
                        } else {
                            0
                        };
                        messages.push(if self == Self::ApcMk2 {
                            [0x90, (scene * 8 + track) as u8, color]
                        } else {
                            [0x90 + track as u8, 0x35 + scene as u8, color]
                        });
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
    name: String,
    surface: Surface,
    connection: MidiOutputConnection,
    values: BTreeMap<(u8, u8), u8>,
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
                while !stop.load(Relaxed) {
                    let Some(snapshot) = snapshot.upgrade() else {
                        break;
                    };
                    let policy = policy();
                    outputs.retain(|_, output| policy.requested_policy.allows(&output.name));
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
                                    let Ok(name) = probe.port_name(&port) else {
                                        continue;
                                    };
                                    let Some(surface) = Surface::named(&name) else {
                                        continue;
                                    };
                                    if !policy.requested_policy.allows(&name) {
                                        continue;
                                    }
                                    let Ok(midi) = MidiOutput::new("omatainer-feedback") else {
                                        continue;
                                    };
                                    match midi.connect(&port, "omatainer-feedback-out") {
                                        Ok(mut connection) => {
                                            let mut ready = true;
                                            for message in surface.initialization() {
                                                if connection.send(&message).is_ok() {
                                                    counts.sent.fetch_add(1, Relaxed);
                                                } else {
                                                    counts.failed.fetch_add(1, Relaxed);
                                                    ready = false;
                                                    break;
                                                }
                                            }
                                            if !ready {
                                                continue;
                                            }
                                            outputs.insert(
                                                port.id(),
                                                Output {
                                                    name,
                                                    surface,
                                                    connection,
                                                    values: BTreeMap::new(),
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
                            .map(|(id, output)| (id.clone(), output.surface.messages(&snapshot)))
                            .collect::<Vec<_>>()
                    };
                    for (id, messages) in updates {
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
                        if failed {
                            outputs.remove(&id);
                        }
                    }
                    snapshot.lock().midi_feedback = Stats {
                        sent: counts.sent.load(Relaxed),
                        failed: counts.failed.load(Relaxed),
                        connected: outputs.len() as u64,
                    };
                    snapshot.lock().midi_input = input_counters.snapshot();
                    std::thread::park_timeout(Duration::from_millis(67));
                }
                for output in outputs
                    .values_mut()
                    .filter(|output| output.surface == Surface::Sp1)
                {
                    let _ = output.connection.send(&[0x9b, 9, 0]);
                }
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
            assert_eq!(messages.len(), 40);
            for track in 0..8 {
                for scene in 0..5 {
                    let state = &snapshot.tracks[track];
                    let expected = if state.clips[scene].kind == 0 { 0 }
                        else if state.playing_scene == scene as i16 { playing_color }
                        else { loaded_color };
                    let address = if surface == Surface::ApcMk2 {
                        [0x90, (scene * 8 + track) as u8, expected]
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
}
