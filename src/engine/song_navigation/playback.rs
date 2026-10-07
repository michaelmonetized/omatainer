use super::super::{clip_launch::Grid, midi_schedule::BEAT_EPSILON, RtEngine};
use super::*;

impl RtEngine {
    /// Leave song looping for an explicitly selected destination.
    /// Takes this renderer; clears the live loop flag without changing saved braces or clip ownership.
    pub(crate) fn song_navigation_leave_loop(&mut self) {
        if self.navigation.saved.as_mut().is_some_and(|saved| {
            let active = saved.looping;
            saved.looping = false;
            active
        }) {
            self.undo.untracked_change();
            self.project.edited();
        }
    }
    /// Schedule one exact musical destination.
    /// Takes its beat, timing and post-jump loop choice; queues against the audio clock or seeks immediately while stopped.
    pub(crate) fn song_navigation_queue(&mut self, beat: f64, grid: Grid, loop_after: bool) {
        let seconds = self
            .conductor
            .as_ref()
            .map_or(beat * 60.0 / f64::from(self.bpm), |map| {
                map.seconds_at(beat)
            });
        if !super::metadata::position(beat)
            || !seconds.is_finite()
            || !(0.0..=86400.0).contains(&seconds)
        {
            self.navigation.cancel();
            self.navigation.error = Some(Error::PositionLimit);
            return;
        }
        self.song_navigation_leave_loop();
        self.navigation.error = None;
        let pending = Pending {
            beat,
            when: self.clip_boundary(grid),
            loop_after,
        };
        self.navigation.pending = Some(pending);
        if !self.playing || pending.when <= self.precise_midi_beat() + BEAT_EPSILON {
            self.song_navigation_tick();
        }
    }
    /// Resolve keyboard, assistive and mapped-controller navigation.
    /// Takes one validated action; uses stable section IDs and chronological neighbors without changing physical input owners.
    pub(crate) fn song_navigation(&mut self, action: Action) {
        if !action.valid() {
            self.navigation.cancel();
            self.navigation.error = Some(Error::PositionLimit);
            return;
        }
        let (destination, grid) = match action {
            Action::Cancel => {
                self.navigation.cancel();
                return;
            }
            Action::ToggleLoop => return,
            Action::Beat { beat, grid } => (Some(beat), grid),
            Action::Seconds(seconds) => {
                let beat = self
                    .conductor
                    .as_ref()
                    .map_or(seconds * f64::from(self.bpm) / 60.0, |map| {
                        map.beat_at_seconds(seconds)
                    });
                self.song_navigation_queue(beat, Grid::Immediate, false);
                return;
            }
            Action::Locator { id, grid } => {
                let beat = self
                    .navigation
                    .saved
                    .as_ref()
                    .and_then(|saved| saved.model.destination(id));
                if beat.is_none() {
                    self.navigation.cancel();
                    self.navigation.error = Some(Error::MissingLocator);
                    return;
                }
                (beat, grid)
            }
            Action::Previous(grid) | Action::Next(grid) => {
                let beat = self.navigation.saved.as_ref().and_then(|saved| {
                    saved
                        .model
                        .adjacent(self.precise_midi_beat(), matches!(action, Action::Next(_)))
                });
                if beat.is_none() {
                    self.navigation.cancel();
                    self.navigation.error = Some(Error::MissingNeighbor);
                    return;
                }
                (beat, grid)
            }
        };
        self.song_navigation_queue(destination.unwrap(), grid, false);
    }
    /// Consume song jumps and loop wraps before this sample advances.
    /// Takes the current audio-owned position; seeks/chases once at a due boundary and preserves fractional loop time through conductor changes.
    pub(crate) fn song_navigation_tick(&mut self) {
        if self.count_in.is_some() {
            return;
        }
        if let Some(pending) = self.navigation.pending {
            if !self.playing || pending.when <= self.precise_midi_beat() + BEAT_EPSILON {
                let seconds = self
                    .conductor
                    .as_ref()
                    .map_or(pending.beat * 60.0 / f64::from(self.bpm), |map| {
                        map.seconds_at(pending.beat)
                    });
                if !seconds.is_finite() || !(0.0..=86400.0).contains(&seconds) {
                    self.navigation.cancel();
                    self.navigation.error = Some(Error::PositionLimit);
                    return;
                }
                self.seek_timeline(seconds);
                if pending.loop_after {
                    if let Some(saved) = &mut self.navigation.saved {
                        saved.looping = saved.model.loop_region.is_some();
                    }
                    self.project.edited();
                }
                self.mapped_clock = self.conductor.as_ref().map(|map| {
                    super::super::midi_data::ConductorClock::new(map, self.precise_midi_beat())
                });
            }
            return;
        }
        if !self.playing {
            return;
        }
        let Some(region) = self
            .navigation
            .saved
            .as_ref()
            .filter(|saved| saved.looping)
            .and_then(|saved| saved.model.loop_region)
        else {
            return;
        };
        let now = self.precise_midi_beat();
        if now + BEAT_EPSILON < region.end {
            return;
        }
        let seconds = |beat| {
            self.conductor
                .as_ref()
                .map_or(beat * 60.0 / f64::from(self.bpm), |map| {
                    map.seconds_at(beat)
                })
        };
        let start = seconds(region.start);
        let end = seconds(region.end);
        let excess = (seconds(now) - end).max(0.0);
        let period = end - start;
        if !period.is_finite() || period <= 0.0 || end > 86400.0 {
            self.song_navigation_leave_loop();
            self.navigation.cancel();
            self.navigation.error = Some(Error::PositionLimit);
            return;
        }
        self.seek_timeline(start + excess.rem_euclid(period));
        self.mapped_clock = self
            .conductor
            .as_ref()
            .map(|map| super::super::midi_data::ConductorClock::new(map, self.precise_midi_beat()));
    }
}
