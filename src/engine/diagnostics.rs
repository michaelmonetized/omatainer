//! Optional bounded attribution. One real sample frame in 2048 is timed; these
//! wall costs are estimates, not per-track CPU or additive callback accounting.
use super::{DECKS, session::{MAX_SCENES as SCENES, MAX_TRACKS as TRACKS}};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Instant;

pub const SLOTS: usize = 16;
pub const STRIDE: u64 = 2048;
const TRACK_BASE: usize = 0;
const SCENE_BASE: usize = TRACK_BASE + TRACKS;
const DECK_BASE: usize = SCENE_BASE + SCENES;
const MASTER_BASE: usize = DECK_BASE + DECKS;
const PAD: usize = MASTER_BASE + 3;
const TRACK_FX: usize = PAD + 1;
const SCENE_FX: usize = TRACK_FX + TRACKS * SLOTS;
pub const POINTS: usize = SCENE_FX + SCENES * SLOTS;

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct Cost {
    pub point: usize,
    pub effect: Option<u8>,
    pub elapsed_ns: u64,
}
impl Cost {
    pub fn valid(self) -> bool {
        if self.point >= POINTS {
            return false;
        }
        if (MASTER_BASE..PAD).contains(&self.point) {
            self.effect.is_some_and(|id| id < 3)
        } else if self.point >= TRACK_FX {
            self.effect
                .is_some_and(|id| (id as usize) < super::fx::FxId::all().len())
        } else {
            self.effect.is_none()
        }
    }
    pub fn label(self) -> String {
        let n = self.point;
        if n < SCENE_BASE {
            format!("Track {} total", n + 1)
        } else if n < DECK_BASE {
            format!("Scene {} bus total", n - SCENE_BASE + 1)
        } else if n < MASTER_BASE {
            format!("Deck {} total", ['A', 'B'][n - DECK_BASE])
        } else if n < PAD {
            format!(
                "Master device {} ({})",
                n - MASTER_BASE + 1,
                self.effect
                    .and_then(|id| ["Echo", "Reverb", "Filter"].get(id as usize))
                    .unwrap_or(&"unknown")
            )
        } else if n == PAD {
            "Shared pad sources".into()
        } else {
            let (owner, index) = if n < SCENE_FX {
                ("Track", n - TRACK_FX)
            } else {
                ("Scene", n - SCENE_FX)
            };
            let name = self
                .effect
                .and_then(|id| super::fx::FxId::all().get(id as usize))
                .map(|id| id.name())
                .unwrap_or("unknown");
            format!(
                "{owner} {} device {} ({name})",
                index / SLOTS + 1,
                index % SLOTS + 1
            )
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Profile {
    pub frame: u64,
    pub sample_rate: u32,
    pub omitted_devices: u64,
    pub costs: Vec<Cost>,
}

/// Callback-owned fixed storage; no strings, allocation, or destruction.
pub(super) struct FrameProfile {
    pub active: bool,
    #[cfg(test)]
    pub fault: Option<(bool, usize, usize, std::time::Duration)>,
    pub frame: u64,
    pub sample_rate: u32,
    omitted: u64,
    ns: [u64; POINTS],
    kinds: [u8; POINTS],
}
impl Default for FrameProfile {
    fn default() -> Self {
        Self {
            active: false,
            #[cfg(test)]
            fault: None,
            frame: 0,
            sample_rate: 0,
            omitted: 0,
            ns: [0; POINTS],
            kinds: [255; POINTS],
        }
    }
}
impl FrameProfile {
    pub fn begin(&mut self, enabled: bool, frame: u64, sr: f32) {
        self.active = enabled && frame % STRIDE == 0;
        if self.active {
            self.frame = frame;
            self.sample_rate = sr as u32;
            self.omitted = 0;
            self.ns.fill(0);
            self.kinds.fill(255);
        }
    }
    pub fn start(&self) -> Option<Instant> {
        self.active.then(Instant::now)
    }
    fn end(&mut self, point: usize, start: Option<Instant>, effect: u8) {
        if let Some(start) = start {
            self.ns[point] = super::audio_metrics::nanoseconds(start.elapsed());
            self.kinds[point] = effect;
        }
    }
    pub fn track(&mut self, i: usize, start: Option<Instant>) {
        self.end(TRACK_BASE + i, start, 254);
    }
    pub fn scene(&mut self, i: usize, start: Option<Instant>) {
        self.end(SCENE_BASE + i, start, 254);
    }
    pub fn deck(&mut self, i: usize, start: Option<Instant>) {
        self.end(DECK_BASE + i, start, 254);
    }
    pub fn master(&mut self, i: usize, start: Option<Instant>, kind: super::FxKind) {
        self.end(MASTER_BASE + i, start, kind as u8);
    }
    pub fn pads(&mut self, start: Option<Instant>) {
        self.end(PAD, start, 254);
    }
    pub fn chain(
        &mut self,
        chain: &mut super::fx::FxChain,
        frame: [f32; 2],
        sr: f32,
        scene: bool,
        target: usize,
    ) -> [f32; 2] {
        if !self.active {
            return chain.process_stereo(frame, sr);
        }
        let base = if scene { SCENE_FX } else { TRACK_FX } + target * SLOTS;
        self.omitted += chain.slots.len().saturating_sub(SLOTS) as u64;
        let mut frame = frame;
        for (i, slot) in chain.slots.iter_mut().enumerate() {
            let start = (i < SLOTS).then(Instant::now);
            #[cfg(test)]
            if let Some((is_scene, owner, index, delay)) = self.fault {
                if (scene, target, i) == (is_scene, owner, index) {
                    std::thread::sleep(delay);
                }
            }
            frame = slot.tick_stereo(frame, sr);
            if i < SLOTS {
                self.end(base + i, start, slot.id() as u8);
            }
        }
        frame
    }
}

pub(super) struct Profiler {
    pub enabled: AtomicBool,
    sequence: AtomicU64,
    frame: AtomicU64,
    sr: AtomicU64,
    omitted: AtomicU64,
    ns: Box<[AtomicU64]>,
    kinds: Box<[AtomicU64]>,
}
impl Default for Profiler {
    fn default() -> Self {
        Self {
            enabled: AtomicBool::new(false),
            sequence: AtomicU64::new(0),
            frame: AtomicU64::new(0),
            sr: AtomicU64::new(0),
            omitted: AtomicU64::new(0),
            ns: (0..POINTS).map(|_| AtomicU64::new(0)).collect(),
            kinds: (0..POINTS).map(|_| AtomicU64::new(255)).collect(),
        }
    }
}
impl Profiler {
    pub fn publish(&self, frame: &FrameProfile) {
        self.sequence.fetch_add(1, Ordering::AcqRel);
        self.frame.store(frame.frame, Ordering::Relaxed);
        self.sr.store(frame.sample_rate as u64, Ordering::Relaxed);
        self.omitted.store(frame.omitted, Ordering::Relaxed);
        for i in 0..POINTS {
            self.ns[i].store(frame.ns[i], Ordering::Relaxed);
            self.kinds[i].store(frame.kinds[i] as u64, Ordering::Relaxed);
        }
        self.sequence.fetch_add(1, Ordering::Release);
    }
    /// Called only from GUI/diagnostic readers; materializing labels/Vec never
    /// runs on the audio callback. A raced read returns None without spinning.
    pub fn read(&self) -> Option<Profile> {
        let before = self.sequence.load(Ordering::Acquire);
        if before == 0 || before & 1 != 0 {
            return None;
        }
        let mut result = Profile {
            frame: self.frame.load(Ordering::Relaxed),
            sample_rate: self.sr.load(Ordering::Relaxed) as u32,
            omitted_devices: self.omitted.load(Ordering::Relaxed),
            costs: Vec::with_capacity(POINTS),
        };
        for i in 0..POINTS {
            let kind = self.kinds[i].load(Ordering::Relaxed) as u8;
            let ns = self.ns[i].load(Ordering::Relaxed);
            if kind != 255 {
                result.costs.push(Cost {
                    point: i,
                    effect: (kind != 254).then_some(kind),
                    elapsed_ns: ns,
                });
            }
        }
        std::sync::atomic::fence(Ordering::Acquire);
        (before == self.sequence.load(Ordering::Relaxed)).then_some(result)
    }
}

#[cfg(test)]
mod tests;
