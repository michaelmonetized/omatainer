use super::{Empty, Properties, Timing};
use crate::engine::{
    clip_launch::Grid,
    midi_data::TimingSettings,
    midi_schedule::BEAT_EPSILON,
    session::{Axis, Reference},
    RtEngine,
};
use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub(crate) enum Error {
    InvalidScene,
    ChangedScene,
    PositionLimit,
    Arrangement,
}
impl Error {
    /// Explain a refused scene launch.
    /// Takes the bounded error code; returns a static native status message.
    pub(crate) fn text(self) -> &'static str {
        match self {
            Self::InvalidScene => "Scene launch was refused: choose an active scene with valid properties.",
            Self::ChangedScene => "Queued scene changed or was deleted; launch it again after reviewing its properties.",
            Self::PositionLimit => "Scene launch exceeds the song's musical or time limit.",
            Self::Arrangement => "Scene launch needs Session playback. Disable Arrangement playback first.",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub(crate) struct Pending {
    pub scene: Reference,
    pub properties: Properties,
    pub when: f64,
    pub additive: bool,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize)]
pub(crate) struct State {
    pub timing: Option<Timing>,
    pub pending: Option<Pending>,
    pub active: Option<Reference>,
    pub active_properties: Option<Properties>,
    pub error: Option<Error>,
}
impl State {
    /// Cancel a pending whole-scene change.
    /// Takes this audio-owned scene state; clears its pending launch and refusal without changing sounding clips.
    pub(crate) fn cancel(&mut self) {
        self.pending = None;
        self.error = None;
    }
}
impl RtEngine {
    /// Queue all scene content and clock properties at one musical boundary.
    /// Takes an active storage slot; retains identity and reviewed properties, with immediate application while stopped.
    pub(crate) fn scene_queue(&mut self, scene: usize, additive: bool) {
        self.scenes.cancel();
        if self.arrangement.enabled() {
            self.scenes.error = Some(Error::Arrangement);
            return;
        }
        let Some(reference) = self.session.reference(Axis::Scene, scene) else {
            self.scenes.error = Some(Error::InvalidScene);
            return;
        };
        let properties = self.session.scenes[scene].scene;
        if !properties.valid() {
            self.scenes.error = Some(Error::InvalidScene);
            return;
        }
        let when = self.clip_boundary(properties.grid);
        let seconds = self
            .conductor
            .as_ref()
            .map_or(when * 60.0 / f64::from(self.bpm), |map| {
                map.seconds_at(when)
            });
        if !when.is_finite()
            || !(0.0..=262144.0).contains(&when)
            || !seconds.is_finite()
            || !(0.0..=86400.0).contains(&seconds)
        {
            self.scenes.error = Some(Error::PositionLimit);
            return;
        }
        self.scenes.pending = Some(Pending {
            scene: reference,
            properties,
            when,
            additive,
        });
        if !self.playing || when <= self.precise_midi_beat() + BEAT_EPSILON {
            self.scene_launch_tick();
        }
    }
    /// Apply a due whole-scene launch before the output sample advances.
    /// Takes the exact song position; changes its flat clock and every participating track without allocation or source-owner replacement.
    pub(crate) fn scene_launch_tick(&mut self) {
        let Some(pending) = self.scenes.pending else {
            return;
        };
        if self.count_in.is_some()
            || self.playing && pending.when > self.precise_midi_beat() + BEAT_EPSILON
        {
            return;
        }
        self.scenes.pending = None;
        let Some(scene) = self
            .session
            .resolve(Axis::Scene, pending.scene.id)
            .filter(|slot| self.session.resolves(Axis::Scene, *slot, pending.scene))
        else {
            self.scenes.error = Some(Error::ChangedScene);
            return;
        };
        if self.session.scenes[scene].scene != pending.properties {
            self.scenes.error = Some(Error::ChangedScene);
            return;
        }
        if self.arrangement.enabled() {
            self.scenes.error = Some(Error::Arrangement);
            return;
        }
        let properties = pending.properties;
        if properties.tempo_micros.is_some() || properties.meter.is_some() {
            let now = self.precise_midi_beat();
            let inherited = self.scenes.timing.unwrap_or_else(|| {
                self.conductor.as_ref().map_or(
                    Timing {
                        signature: Default::default(),
                        anchor: 0.0,
                        first_bar: 1,
                        click: TimingSettings::default(),
                    },
                    |map| {
                        let (bar, _, meter) = map.position(now);
                        let origin = map
                            .meters
                            .iter()
                            .rev()
                            .find(|meter| meter.tick as f64 / f64::from(map.ppqn) <= now)
                            .map_or(0.0, |meter| meter.tick as f64 / f64::from(map.ppqn));
                        let mut click = map.native.unwrap_or_default();
                        let origin = if origin == 0.0 { click.pickup } else { origin };
                        click.pickup = 0.0;
                        let signature = super::Signature {
                            numerator: meter.numerator,
                            denominator_power: meter.denominator_power,
                        };
                        let anchor = if now < origin {
                            origin
                        } else {
                            origin
                                + ((now - origin) / signature.length()).floor() * signature.length()
                        };
                        Timing {
                            signature,
                            anchor,
                            first_bar: bar.max(1),
                            click,
                        }
                    },
                )
            });
            let previous_bar = inherited.position(now).0.max(1);
            let timing = Timing {
                signature: properties.meter.unwrap_or(inherited.signature),
                anchor: if properties.meter.is_some() {
                    pending.when
                } else {
                    inherited.anchor
                },
                first_bar: if properties.meter.is_some() {
                    previous_bar
                } else {
                    inherited.first_bar
                },
                click: inherited.click,
            };
            let bpm = properties.bpm().unwrap_or_else(|| {
                self.conductor.as_ref().map_or(f64::from(self.bpm), |map| {
                    60_000_000.0 / map.micros_exact_at(now)
                })
            });
            self.retire_conductor();
            self.bpm = bpm as f32;
            self.scenes.timing = Some(timing);
            self.metro.reset();
        }
        let selected_track = self.selected_track;
        for track in 0..self.tracks.len() {
            if !self.session.tracks[track].active {
                continue;
            }
            let clip = &self.tracks[track].clips[scene];
            if clip.properties.disabled {
                continue;
            }
            if clip.occupied() {
                self.clip_queue_at(track, scene as u16, true, pending.when);
            } else if properties.empty == Empty::Stop && !pending.additive {
                self.finish_recording_track(track);
                self.tracks[track].stop_clip();
                self.midi_routing.clear_clip(track as u8);
            }
        }
        self.selected_track = selected_track;
        self.scenes.active = Some(pending.scene);
        self.scenes.active_properties = Some(pending.properties);
        self.scenes.error = None;
        self.selected_scene = scene;
        self.project.edited();
        self.undo.untracked_change();
    }
    /// Recheck queued and active identities after an atomic metadata swap.
    /// Takes the current layout; cancels changed/deleted work while preserving reordered identities.
    pub(crate) fn scene_metadata_changed(&mut self) {
        if self.scenes.pending.is_some_and(|pending| {
            self.session
                .resolve(Axis::Scene, pending.scene.id)
                .is_none_or(|slot| {
                    !self.session.resolves(Axis::Scene, slot, pending.scene)
                        || self.session.scenes[slot].scene != pending.properties
                })
        }) {
            self.scenes.pending = None;
            self.scenes.error = Some(Error::ChangedScene);
        }
        if self.scenes.active.is_some_and(|active| {
            self.session
                .resolve(Axis::Scene, active.id)
                .is_none_or(|slot| !self.session.resolves(Axis::Scene, slot, active))
        }) {
            self.scenes.active = None;
            self.scenes.active_properties = None;
        }
    }
    /// Resolve a group of bars against the current flat scene meter.
    /// Takes a launch grid and current beat; returns a scene boundary only for explicitly bar-based grids.
    pub(crate) fn scene_bar_boundary(&self, grid: Grid, beat: f64) -> Option<f64> {
        let bars = match grid {
            Grid::Bar => 1,
            Grid::TwoBars => 2,
            Grid::FourBars => 4,
            Grid::EightBars => 8,
            _ => return None,
        };
        self.scenes.timing.map(|timing| timing.boundary(beat, bars))
    }
}

impl crate::engine::Snapshot {
    /// Read the applied or queued scene name from the prepared snapshot.
    /// Takes the queued flag; returns a borrowed name only while its namespace and stable identity resolve.
    pub(crate) fn scene_name(&self, queued: bool) -> &str {
        let reference = if queued {
            self.scenes.pending.map(|p| p.scene)
        } else {
            self.scenes.active
        };
        let Some((layout, reference)) = self.session.as_ref().zip(reference) else {
            return "";
        };
        layout
            .resolve(Axis::Scene, reference.id)
            .filter(|slot| layout.resolves(Axis::Scene, *slot, reference))
            .map_or("", |slot| layout.scenes[slot].name.as_str())
    }
}
