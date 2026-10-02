//! Persisted exact source ticks. Projection into the legacy visible f32 fields
//! is checked, so a stale tick record cannot override an intentional edit.
use serde::{Deserialize, Serialize};
mod lanes;
mod timeline;
pub(crate) use timeline::Settings as TimingSettings;
pub(crate) use lanes::{Conductor, Lanes, Meter, Tempo, MAX_CONDUCTOR_POINTS, MAX_LANE_BYTES};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TickTiming {
    pub ppqn: u16,
    pub start: u64,
    pub duration: u64,
    pub start_order: u32,
    pub end_order: u32,
}
impl TickTiming {
    pub fn valid(self) -> bool {
        self.end_order > self.start_order
            && self.ppqn > 0
            && self.ppqn <= 32767
            && self
                .start
                .checked_add(self.duration)
                .is_some_and(|end| end <= u64::from(self.ppqn) * 262144)
    }
    pub fn start_beats(self) -> f64 {
        self.start as f64 / f64::from(self.ppqn)
    }
    pub fn duration_beats(self) -> f64 {
        self.duration as f64 / f64::from(self.ppqn)
    }
    pub fn matches(self, start: f32, len: f32) -> bool {
        self.valid() && self.start_beats() as f32 == start && self.duration_beats() as f32 == len
    }
}
pub(super) fn release_velocity() -> u8 {
    64
}
impl super::MidiNote {
    pub(crate) fn source_start(&self) -> f64 {
        self.source_timing
            .filter(|timing| timing.matches(self.start, self.len))
            .map_or(self.start as f64, TickTiming::start_beats)
    }
    pub(crate) fn source_duration(&self) -> f64 {
        self.source_timing
            .filter(|timing| timing.matches(self.start, self.len))
            .map_or(self.len as f64, TickTiming::duration_beats)
    }
    pub(crate) fn interchange_valid(&self) -> bool {
        self.channel < 16
            && self.release_vel <= 127
            && self
                .source_timing
                .is_none_or(|timing| timing.matches(self.start, self.len))
    }
    /// Edits retain channel/release values; changed coordinates become native
    /// musical values until an export explicitly chooses its tick resolution.
    pub(crate) fn reconcile_timing(&mut self) {
        if self
            .source_timing
            .is_some_and(|timing| !timing.matches(self.start, self.len))
        {
            self.source_timing = None;
        }
    }
    pub(crate) fn from_smf(note: &crate::midi_file::Note, ppqn: u16) -> Result<Self, String> {
        let timing = TickTiming {
            ppqn,
            start: note.start_tick,
            duration: note.duration_ticks,
            start_order: note.start_order,
            end_order: note.end_order,
        };
        if !timing.valid()
            || note.channel > 15
            || note.pitch > 127
            || note.velocity == 0
            || note.velocity > 127
            || note.release_velocity > 127
        {
            return Err("MIDI note is outside supported musical/value bounds".into());
        }
        let id = super::midi_edit::NoteId::new();
        if !id.valid() {
            return Err("MIDI note identity is unavailable".into());
        }
        Ok(Self {
            id,
            muted: false,
            pitch: note.pitch,
            vel: note.velocity,
            channel: note.channel,
            release_vel: note.release_velocity,
            source_timing: Some(timing),
            start: timing.start_beats() as f32,
            len: timing.duration_beats() as f32,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn late_high_resolution_import_keeps_ticks_beyond_visible_float_precision() {
        super::super::midi_edit::initialize().unwrap();
        let raw = crate::midi_file::Note {
            channel: 15,
            pitch: 100,
            velocity: 80,
            release_velocity: 7,
            start_tick: 32767 * 262144 - 2,
            duration_ticks: 1,
            start_order: 1,
            end_order: 2,
        };
        let mut note = super::super::MidiNote::from_smf(&raw, 32767).unwrap();
        assert_eq!(note.start, 262144.0);
        assert!(note.source_start() < 262144.0);
        assert_eq!(note.source_duration(), 1.0 / 32767.0);
        assert!(note.interchange_valid());
        note.pitch -= 1;
        note.reconcile_timing();
        assert!(note.source_timing.is_some());
        note.start -= 1.0;
        note.reconcile_timing();
        assert!(note.source_timing.is_none());
        assert_eq!((note.channel, note.release_vel), (15, 7));
    }
    #[test]
    fn exact_source_ticks_drive_renderer_without_projection_boundary_shift() {
        use super::super::{midi_edit::Region, test_alloc, Command, Engine};
        let (engine, mut rt) = Engine::headless_for_test(48000, 256);
        rt.bpm = 120.0;
        rt.quant = 0.0;
        let raw = crate::midi_file::Note {
            channel: 2,
            pitch: 64,
            velocity: 80,
            release_velocity: 7,
            start_tick: 1,
            duration_ticks: 719,
            start_order: 1,
            end_order: 2,
        };
        let other = crate::midi_file::Note {
            channel: 15,
            pitch: 67,
            velocity: 100,
            release_velocity: 13,
            start_tick: 1001,
            duration_ticks: 123,
            start_order: 3,
            end_order: 4,
        };
        let notes = vec![
            super::super::MidiNote::from_smf(&raw, 960).unwrap(),
            super::super::MidiNote::from_smf(&other, 960).unwrap(),
        ];
        rt.tracks[2].clips[7].region = Some(Region::full(16.0));
        rt.tracks[2].clips[7].bars = 16.0;
        engine
            .send(Command::SetNotes {
                track: 2,
                scene: 7,
                notes,
            })
            .unwrap();
        rt.process(&mut []);
        rt.begin_midi_trace_for_test(2);
        engine
            .send(Command::FireClip {
                track: 2,
                scene: 7,
                looping: false,
            })
            .unwrap();
        rt.process(&mut []);
        let mut out = vec![0.0; 257 * 2];
        let counts = test_alloc::measure(|| {
            for _ in 0..110 {
                rt.process(&mut out);
            }
        });
        assert_eq!(counts, test_alloc::Counts::default());
        assert_eq!(
            rt.take_midi_trace_for_test(2),
            vec![
                (25, true, 64, 80),
                (18000, false, 64, 0),
                (25025, true, 67, 100),
                (28100, false, 67, 0)
            ]
        );
        assert_eq!(rt.tracks[2].clips[7].notes[1].channel, 15);
        assert_eq!(rt.tracks[2].clips[7].notes[1].release_vel, 13);
    }
    #[test]
    fn corrupt_tick_records_fail_instead_of_overriding_editor_coordinates() {
        let good = TickTiming {
            ppqn: 960,
            start: 1,
            duration: 719,
            start_order: 1,
            end_order: 2,
        };
        assert!(good.matches(1.0 / 960.0, 719.0 / 960.0));
        assert!(!good.matches(0.0, 719.0 / 960.0));
        for bad in [
            TickTiming { ppqn: 0, ..good },
            TickTiming {
                ppqn: 32768,
                ..good
            },
            TickTiming {
                start: u64::MAX,
                ..good
            },
            TickTiming {
                start: 960 * 262144,
                duration: 1,
                ..good
            },
        ] {
            assert!(!bad.valid());
        }
    }
}

impl super::RtEngine {
    pub(super) fn retire_conductor(&mut self) {
        if let Some(conductor) = self.conductor.take() {
            let bytes = conductor.bytes();
            self.undo.retire_midi_conductor(conductor, bytes);
        }
    }
}
