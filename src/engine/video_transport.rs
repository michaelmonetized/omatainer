use super::*;
impl RtEngine {
    /// Read the audio-owned project position.
    /// Takes this renderer; returns seconds integrated from integer playing frames, independent of tempo changes.
    pub(crate) fn timeline_seconds(&self) -> f64 {
        self.timeline_anchor + self.timeline_frames as f64 / f64::from(self.sr)
    }
    /// Seek musical playback to an explicit picture position.
    /// Takes bounded project seconds; ends clip recording, releases clip notes and rebuilds current launch schedules without changing physical input ownership.
    pub(super) fn seek_timeline(&mut self, seconds: f64) {
        if !seconds.is_finite() || !(0.0..=86400.0).contains(&seconds) {
            return;
        }
        self.navigation.cancel();
        self.finish_recording_all();
        self.recording = false;
        self.compose_target = None;
        self.count_in = None;
        self.metro.reset();
        self.transport_epoch = self.transport_epoch.wrapping_add(1);
        let beat = self
            .conductor
            .as_ref()
            .map_or(seconds * f64::from(self.bpm) / 60.0, |map| {
                map.beat_at_seconds(seconds)
            });
        self.beat = beat;
        self.midi_beat = beat;
        self.midi_beat_reference = beat;
        self.beat_roundoff = 0.0;
        self.mapped_clock = None;
        self.arrangement.reset(beat);
        self.timeline_anchor = seconds;
        self.timeline_frames = 0;
        for (slot, track) in self.tracks.iter_mut().enumerate() {
            track.launch.seek();
            self.midi_routing.clear_clip(slot as u8);
            track.release_clip_notes();
            track.drum_pos.fill(None);
            if let Some(mut launch) = track.playing.or(track.project_resume) {
                launch.last_beat = 0.0;
                if track.playing.is_some() {
                    track.playing = Some(launch);
                } else {
                    track.project_resume = Some(launch);
                }
                track.rebuild_midi_schedule(beat, beat);
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn playing_sample_clock_survives_tempo_pause_rate_change_and_long_seek() {
        let (engine, mut rt) = Engine::headless_for_test(48000, 144);
        engine.send(Command::Play).unwrap();
        rt.process(&mut vec![0.0; 96000]);
        assert_eq!(rt.timeline_seconds(), 1.0);
        rt.process(&mut [0.0; 512]);
        assert_eq!(engine.snapshot().timeline_seconds, rt.timeline_seconds());
        engine.send(Command::TimelineSeek(1.0)).unwrap();
        rt.process(&mut []);
        engine.send(Command::SetBpm(200.0)).unwrap();
        rt.process(&mut vec![0.0; 96000]);
        assert_eq!(rt.timeline_seconds(), 2.0);
        engine.send(Command::Stop).unwrap();
        rt.process(&mut [0.0; 512]);
        assert_eq!(rt.timeline_seconds(), 2.0);
        rt.set_sample_rate(44100).unwrap();
        engine.send(Command::Play).unwrap();
        rt.process(&mut vec![0.0; 88200]);
        assert_eq!(rt.timeline_seconds(), 3.0);
        engine.send(Command::TimelineSeek(8.0 * 3600.0)).unwrap();
        rt.process(&mut []);
        assert_eq!(rt.timeline_seconds(), 28800.0);
        rt.process(&mut [0.0; 882]);
        assert!((rt.timeline_seconds() - 28800.01).abs() < 1e-10);
        assert!(engine.send(Command::TimelineSeek(f64::NAN)).is_err());
        engine.cmd.performance().set_enabled(true).unwrap();
        assert!(engine.send(Command::TimelineSeek(0.0)).is_err());
    }
}

#[cfg(test)]
mod allocation_tests {
    use super::*;
    #[test]
    fn actual_clip_seek_and_per_callback_clock_publish_do_not_allocate_or_free() {
        let (_, mut rt) = Engine::headless_for_test(48000, 144);
        rt.apply(Command::LaunchScene { scene: 0 });
        rt.process(&mut [0.0; 512]);
        let counts = super::super::test_alloc::measure(|| {
            rt.apply(Command::TimelineSeek(2.25));
            rt.process(&mut [0.0; 512]);
        });
        assert_eq!(counts.allocations, 0);
        assert_eq!(counts.frees, 0);
    }
}
