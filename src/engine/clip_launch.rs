use super::{clip_management::edit::Slot, PlayingClip, RtEngine};
use serde::{Deserialize, Serialize};
#[cfg(test)]
mod tests;

/// Read a current clip identity without creating an error payload.
/// Takes layout and storage slots; returns both retained references only while they are active.
fn address(layout: &super::session::Layout, track: usize, scene: usize) -> Option<Slot> {
    Some(Slot {
        track: layout.reference(super::session::Axis::Track, track)?,
        scene: layout.reference(super::session::Axis::Scene, scene)?,
    })
}
/// Recheck a held or queued identity without allocating.
/// Takes current layout and the original references; returns current slots only while namespace and identities agree.
fn resolve(layout: &super::session::Layout, slot: Slot) -> Option<(usize, usize)> {
    let track = layout
        .resolve(super::session::Axis::Track, slot.track.id)
        .filter(|t| layout.resolves(super::session::Axis::Track, *t, slot.track))?;
    let scene = layout
        .resolve(super::session::Axis::Scene, slot.scene.id)
        .filter(|s| layout.resolves(super::session::Axis::Scene, *s, slot.scene))?;
    Some((track, scene))
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum Mode {
    #[default]
    Trigger,
    Gate,
    Toggle,
    Repeat,
}
impl Mode {
    pub(crate) const ALL: [Self; 4] = [Self::Trigger, Self::Gate, Self::Toggle, Self::Repeat];
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Trigger => "Trigger",
            Self::Gate => "Gate",
            Self::Toggle => "Toggle",
            Self::Repeat => "Repeat",
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum Grid {
    #[default]
    Global,
    Immediate,
    Sixteenth,
    Eighth,
    Quarter,
    Half,
    Bar,
    TwoBars,
    FourBars,
    EightBars,
}
impl Grid {
    pub(crate) const ALL: [Self; 10] = [
        Self::Global,
        Self::Immediate,
        Self::Sixteenth,
        Self::Eighth,
        Self::Quarter,
        Self::Half,
        Self::Bar,
        Self::TwoBars,
        Self::FourBars,
        Self::EightBars,
    ];
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Global => "Global",
            Self::Immediate => "Immediate",
            Self::Sixteenth => "1/16",
            Self::Eighth => "1/8",
            Self::Quarter => "1/4",
            Self::Half => "1/2",
            Self::Bar => "1 bar",
            Self::TwoBars => "2 bars",
            Self::FourBars => "4 bars",
            Self::EightBars => "8 bars",
        }
    }
    fn beats(self, global: f32) -> f64 {
        match self {
            Self::Global => {
                if global.is_finite() {
                    f64::from(global.max(0.0))
                } else {
                    0.0
                }
            }
            Self::Immediate => 0.0,
            Self::Sixteenth => 0.25,
            Self::Eighth => 0.5,
            Self::Quarter => 1.0,
            Self::Half => 2.0,
            Self::Bar => 4.0,
            Self::TwoBars => 8.0,
            Self::FourBars => 16.0,
            Self::EightBars => 32.0,
        }
    }
    fn bars(self) -> Option<u32> {
        match self {
            Self::Bar => Some(1),
            Self::TwoBars => Some(2),
            Self::FourBars => Some(4),
            Self::EightBars => Some(8),
            _ => None,
        }
    }
}
/// Keep musical launch behavior with the clip content.
/// Takes a mode, inherited or explicit musical grid and phase option; persists defaults without changing older clips.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Policy {
    pub mode: Mode,
    pub grid: Grid,
    pub legato: bool,
}
impl Policy {
    pub(crate) fn is_default(&self) -> bool {
        *self == Self::default()
    }
}
#[derive(Clone, Copy, Debug)]
pub(crate) enum Target {
    Slot {
        track: u8,
        scene: u16,
        looping: bool,
    },
    Apc(u8),
}
#[derive(Clone, Copy, Debug)]
pub(crate) struct Press {
    pub source: u64,
    pub key: u32,
    pub target: Target,
}
#[derive(Clone, Copy, Debug)]
pub(crate) struct Release {
    pub source: u64,
    pub key: u32,
}
#[derive(Clone, Copy, Debug)]
struct Held {
    source: u64,
    key: u32,
    slot: Slot,
    id: u64,
    policy: Policy,
}
#[derive(Clone, Copy, Debug)]
pub(crate) struct Start {
    slot: Slot,
    scene: u16,
    when: f64,
    id: u64,
    looping: bool,
    legato: bool,
    repeat_period: Option<RepeatPeriod>,
}
#[derive(Clone, Copy, Debug)]
enum RepeatPeriod {
    Beats(f64),
    BeatGrid(f64),
    Bars(u32),
}
#[derive(Clone, Copy, Debug)]
struct Stop {
    when: f64,
    id: u64,
}
#[derive(Clone, Copy, Debug)]
struct Repeating {
    start: Start,
    next: f64,
}
#[derive(Clone, Debug, Default)]
pub(crate) struct State {
    pub(crate) queued: Option<Start>,
    stopping: Option<Stop>,
    repeat: Option<Repeating>,
    active_id: u64,
}
impl State {
    pub(crate) fn queued_scene(&self) -> Option<u16> {
        self.queued.map(|q| q.scene)
    }
    pub(crate) fn stopping(&self) -> bool {
        self.stopping.is_some()
    }
    pub(crate) fn cancel(&mut self) {
        self.queued = None;
        self.stopping = None;
    }
    pub(crate) fn clear(&mut self) {
        *self = Self::default();
    }
    pub(crate) fn seek(&mut self) {
        self.cancel();
        self.repeat = None;
    }
    pub(crate) fn cancel_scene(&mut self, scene: u16) {
        if self.queued_scene() == Some(scene) {
            self.queued = None;
        }
        if self.repeat.is_some_and(|r| r.start.scene == scene) {
            self.repeat = None;
        }
    }
    pub(crate) fn set_looping(&mut self, scene: u16, looping: bool) {
        if let Some(q) = self.queued.as_mut().filter(|q| q.scene == scene) {
            q.looping = looping;
        }
    }
}
pub(crate) struct Inputs {
    held: [Option<Held>; super::control::MAX_COMMANDS],
    next: u64,
}
impl Default for Inputs {
    fn default() -> Self {
        Self {
            held: [None; super::control::MAX_COMMANDS],
            next: 0,
        }
    }
}
impl Inputs {
    pub(crate) fn clear(&mut self) {
        self.held.fill(None);
    }
}

/// Match a pad release before consulting the current mapping or layer.
/// Takes a complete channel message and connection owner; returns the original wire key for note-off, zero-velocity note-on or zero CC.
pub(crate) fn wire_release(message: &[u8; 3], source: u64) -> Option<Release> {
    let status = message[0] & 0xf0;
    (status == 0x80 || matches!(status, 0x90 | 0xb0) && message[2] == 0).then(|| Release {
        source,
        key: wire_key(message),
    })
}
pub(crate) fn wire_key(message: &[u8; 3]) -> u32 {
    u32::from(message[0] & 15) * 128
        + u32::from(message[1])
        + if message[0] & 0xf0 == 0xb0 { 2048 } else { 0 }
}

impl RtEngine {
    pub(crate) fn clip_queue_explicit(&mut self, track: usize, scene: u16, looping: bool) {
        let Some(slot) = address(&self.session, track, usize::from(scene)) else {
            return;
        };
        let clip = &self.tracks[track].clips[usize::from(scene)];
        if !clip.occupied() || clip.properties.disabled {
            return;
        }
        let policy = clip.properties.launch;
        let when = self.clip_boundary(policy.grid);
        self.clip_launch_inputs.next = self.clip_launch_inputs.next.wrapping_add(1).max(1);
        let start = Start {
            slot,
            scene,
            when,
            id: self.clip_launch_inputs.next,
            looping,
            legato: policy.legato,
            repeat_period: None,
        };
        self.tracks[track].launch.queued = Some(start);
        self.start_count_in();
        self.playing = true;
        self.selected_track = track;
        self.selected_scene = usize::from(scene);
        if when <= self.precise_midi_beat() + super::midi_schedule::BEAT_EPSILON
            && self.count_in.is_none()
        {
            self.tracks[track].launch.queued = None;
            self.commit_clip_start(track, start);
        }
    }
    pub(crate) fn clip_boundary(&self, grid: Grid) -> f64 {
        let now = self.precise_midi_beat();
        let quantum = grid.beats(self.quant);
        if quantum == 0.0 || !self.playing {
            return now;
        }
        if let (Some(bars), Some(map)) = (grid.bars(), self.conductor.as_ref()) {
            return map.next_bar_boundary(now, bars);
        }
        let nearest = (now / quantum).round() * quantum;
        if (now - nearest).abs() < super::midi_schedule::BEAT_EPSILON {
            nearest
        } else {
            (now / quantum).ceil() * quantum
        }
    }
    pub(crate) fn clip_press(&mut self, input: Press) {
        if self.arrangement.enabled()
            || self
                .clip_launch_inputs
                .held
                .iter()
                .flatten()
                .any(|h| h.source == input.source && h.key == input.key)
        {
            return;
        }
        let (track, scene, looping) = match input.target {
            Target::Slot {
                track,
                scene,
                looping,
            } => (usize::from(track), usize::from(scene), looping),
            Target::Apc(pad) if pad < 40 => {
                let status = &self.surface.status;
                let track = status.track_offset + usize::from(pad % 8);
                let scene = status.scene_offset + usize::from(4 - pad / 8);
                if status.shift {
                    if track < self.tracks.len() && scene < self.scene_fx.len() {
                        self.apply(super::Command::Select { track, scene });
                    }
                    return;
                }
                (track, scene, true)
            }
            _ => return,
        };
        let Some(slot) = address(&self.session, track, scene) else {
            return;
        };
        let clip = &self.tracks[track].clips[scene];
        if clip.properties.disabled {
            return;
        }
        if !clip.occupied() {
            self.apply(super::Command::StopTrack { track: track as u8 });
            return;
        }
        let policy = clip.properties.launch;
        let Some(index) = self
            .clip_launch_inputs
            .held
            .iter()
            .position(Option::is_none)
        else {
            return;
        };
        self.clip_launch_inputs.next = self.clip_launch_inputs.next.wrapping_add(1).max(1);
        let id = self.clip_launch_inputs.next;
        self.clip_launch_inputs.held[index] = Some(Held {
            source: input.source,
            key: input.key,
            slot,
            id,
            policy,
        });
        if policy.mode == Mode::Toggle {
            if self.tracks[track]
                .launch
                .queued
                .is_some_and(|q| q.slot == slot)
            {
                self.tracks[track].launch.queued = None;
                return;
            }
            if self.tracks[track]
                .playing
                .is_some_and(|p| usize::from(p.scene) == scene)
            {
                if self.tracks[track].launch.stopping.is_some() {
                    self.tracks[track].launch.stopping = None;
                    return;
                }
                let when = self.clip_boundary(policy.grid);
                self.tracks[track].launch.stopping = Some(Stop {
                    when,
                    id: self.tracks[track].launch.active_id,
                });
                return;
            }
        }
        let when = self.clip_boundary(policy.grid);
        let repeat_period = if policy.mode == Mode::Repeat {
            let duration = self.clip_period(track, scene);
            let quantum = policy.grid.beats(self.quant);
            Some(policy.grid.bars().map_or_else(
                || {
                    if quantum > 0.0 {
                        RepeatPeriod::BeatGrid(quantum.max(0.0625))
                    } else {
                        RepeatPeriod::Beats(duration)
                    }
                },
                RepeatPeriod::Bars,
            ))
        } else {
            None
        };
        let start = Start {
            slot,
            scene: scene as u16,
            when,
            id,
            looping: looping && policy.mode != Mode::Repeat,
            legato: policy.legato,
            repeat_period,
        };
        self.tracks[track].launch.queued = Some(start);
        self.start_count_in();
        self.playing = true;
        self.selected_track = track;
        self.selected_scene = scene;
        if when <= self.precise_midi_beat() + super::midi_schedule::BEAT_EPSILON
            && self.count_in.is_none()
        {
            self.tracks[track].launch.queued = None;
            self.commit_clip_start(track, start);
        }
    }
    fn clip_period(&self, track: usize, scene: usize) -> f64 {
        let clip = &self.tracks[track].clips[scene];
        clip.audio_region
            .map_or_else(
                || {
                    clip.region
                        .map_or(f64::from(clip.bars.max(0.25)) * 4.0, |r| r.period())
                },
                |r| r.duration_beats,
            )
            .max(0.0625)
    }
    pub(crate) fn clip_release(&mut self, input: Release) {
        let Some(index) = self
            .clip_launch_inputs
            .held
            .iter()
            .position(|h| h.is_some_and(|h| h.source == input.source && h.key == input.key))
        else {
            return;
        };
        let held = self.clip_launch_inputs.held[index].take().unwrap();
        if !matches!(held.policy.mode, Mode::Gate | Mode::Repeat) {
            return;
        }
        let Some((track, _)) = resolve(&self.session, held.slot) else {
            return;
        };
        let state = &mut self.tracks[track].launch;
        if state.queued.is_some_and(|q| q.id == held.id) {
            state.queued = None;
        }
        if state.repeat.is_some_and(|r| r.start.id == held.id) {
            state.repeat = None;
        }
        if state.active_id == held.id {
            let when = self.clip_boundary(held.policy.grid);
            self.tracks[track].launch.stopping = Some(Stop { when, id: held.id });
            if when <= self.precise_midi_beat() + super::midi_schedule::BEAT_EPSILON {
                self.clip_launch_tick(track);
            }
        }
    }
    pub(crate) fn clip_launch_tick(&mut self, track: usize) {
        if !self.playing || self.count_in.is_some() {
            return;
        }
        let now = self.precise_midi_beat();
        if let Some(stop) = self.tracks[track]
            .launch
            .stopping
            .filter(|s| now > s.when + super::midi_schedule::BEAT_EPSILON)
        {
            self.tracks[track].launch.stopping = None;
            if stop.id == self.tracks[track].launch.active_id {
                let pending = self.tracks[track].launch.queued;
                self.finish_recording_track(track);
                self.tracks[track].stop_clip();
                self.midi_routing.clear_clip(track as u8);
                self.tracks[track].launch.queued = pending;
            }
        }
        if let Some(start) = self.tracks[track]
            .launch
            .queued
            .filter(|s| now > s.when + super::midi_schedule::BEAT_EPSILON)
        {
            self.tracks[track].launch.queued = None;
            self.commit_clip_start(track, start);
        } else if let Some(repeat) = self.tracks[track]
            .launch
            .repeat
            .filter(|r| now > r.next + super::midi_schedule::BEAT_EPSILON)
        {
            let mut start = repeat.start;
            start.when = repeat.next;
            start.legato = false;
            self.commit_clip_start(track, start);
        }
    }
    fn commit_clip_start(&mut self, track: usize, start: Start) {
        let Some((slot, scene)) = resolve(&self.session, start.slot) else {
            self.tracks[track].launch.repeat = None;
            return;
        };
        if slot != track
            || !self.tracks[track].clips[scene].occupied()
            || self.tracks[track].clips[scene].properties.disabled
        {
            self.tracks[track].launch.repeat = None;
            return;
        }
        let phase = if start.legato {
            self.tracks[track]
                .playing
                .map_or(0.0, |p| (start.when - p.midi_start_beat).max(0.0))
                .rem_euclid(self.clip_period(track, scene))
        } else {
            0.0
        };
        let queued = self.tracks[track].launch.queued;
        let repeat = start.repeat_period.map(|period| Repeating {
            start,
            next: match (period, self.conductor.as_ref()) {
                (RepeatPeriod::Bars(bars), Some(map)) => map
                    .next_bar_boundary(start.when + super::midi_schedule::BEAT_EPSILON * 2.0, bars),
                (RepeatPeriod::Bars(bars), None) => {
                    (start.when / (f64::from(bars) * 4.0) + 1.0).floor() * f64::from(bars) * 4.0
                }
                (RepeatPeriod::Beats(beats), _) => start.when + beats,
                (RepeatPeriod::BeatGrid(beats), _) => {
                    ((start.when + super::midi_schedule::BEAT_EPSILON) / beats).floor() * beats
                        + beats
                }
            },
        });
        let offset = self.precise_midi_beat() - self.beat;
        self.finish_recording_track(track);
        self.tracks[track].stop_clip();
        self.midi_routing.clear_clip(track as u8);
        self.tracks[track].launch.active_id = start.id;
        self.tracks[track].launch.queued = queued;
        self.tracks[track].launch.repeat = repeat;
        self.tracks[track].playing = Some(PlayingClip {
            scene: scene as u16,
            start_beat: start.when - phase - offset,
            midi_start_beat: start.when - phase,
            last_beat: if phase > 0.0 { phase } else { -0.0001 },
            looping: start.looping,
        });
        let now = self.precise_midi_beat();
        self.tracks[track].rebuild_midi_schedule(self.beat, now);
    }
}
