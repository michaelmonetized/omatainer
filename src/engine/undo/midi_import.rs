//! One atomic bounded transaction for all mapped cells and the conductor.
use super::*;
impl RtEngine {
    pub(super) fn history_midi_import(&mut self, mut request: midi_interchange::Request) {
        if !self.midi_import_current(&request) || !request.ack.claim() {
            self.history_reject(Command::MidiImport(request), Failure::Invalid);
            return;
        }
        if !self.undo.enabled || self.undo.replaying {
            self.history_reject(Command::MidiImport(request), Failure::Unavailable);
            return;
        }
        // Admission must cover the inverse's fixed note reservations, not just
        // the incoming command's allocated payload. Even an empty old cell
        // reserves two full note buffers for future allocation-free swaps.
        let retained = request
            .targets
            .iter()
            .map(|target| {
                let old = &self.tracks[target.baseline.track as usize].clips
                    [target.baseline.scene as usize];
                old.name.capacity()
                    + target.reserved_lane_bytes
                    + (old.notes.capacity() + target.spare_notes.capacity()).max(2 * NOTE_LIMIT)
                        * std::mem::size_of::<MidiNote>()
            })
            .sum::<usize>()
            + if request.change_conductor {
                self.conductor.as_ref().map_or(0, |c| c.bytes())
                    + request.conductor.as_ref().map_or(0, |c| c.bytes())
            } else {
                0
            };
        if let Err(reason) = self.undo.preflight(retained) {
            self.history_reject(Command::MidiImport(request), reason);
            return;
        }
        let timing_only = request.targets.is_empty() && (request.conductor.as_ref().is_some_and(|c| c.native.is_some()) || request.baseline_conductor.as_ref().is_some_and(|c| c.native.is_some()));
        self.undo.begin(if timing_only { Name::Timing } else { Name::Multiple }, 0, self.frames_done);
        if request.change_conductor {
            let bytes = self.conductor.as_ref().map_or(0, |c| c.bytes())
                + request.conductor.as_ref().map_or(0, |c| c.bytes());
            let value = std::mem::replace(&mut self.conductor, request.conductor.take());
            self.undo.append(Patch::Conductor {
                bpm: self.bpm,
                value,
                reserved_bytes: bytes,
            });
        }
        let midi_beat = self.precise_midi_beat();
        if request.change_conductor {
            if let Some(conductor) = &self.conductor {
                self.bpm = (60000000.0 / conductor.micros_exact_at(midi_beat)) as f32;
            }
        }
        for target in &mut request.targets {
            let t = target.baseline.track as usize;
            let s = target.baseline.scene as usize;
            self.cancel_recording_clip(t, s);
            let track = &mut self.tracks[t];
            if track
                .playing
                .or(track.project_resume)
                .is_some_and(|p| p.scene as usize == s)
            {
                track.release_clip_notes();
            }
            // Preserve a mixer gesture made since the worker capture.
            target.replacement.gain = track.clips[s].gain;
            let value = std::mem::replace(
                &mut track.clips[s],
                std::mem::replace(&mut target.replacement, Clip::empty()),
            );
            self.undo.append(Patch::Clip {
                track: t as u8,
                scene: s as u16,
                value,
                spare_notes: std::mem::take(&mut target.spare_notes),
                reserved_midi_bytes: target.reserved_lane_bytes,
            });
            track.clip_notes_changed(s, self.beat, midi_beat);
        }
        self.undo.recount();
        self.project.edited();
        request.ack.applied();
        self.undo.retire_command(Command::MidiImport(request));
    }
}
