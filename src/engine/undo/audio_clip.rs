use super::*;
impl RtEngine {
    pub(in crate::engine) fn history_audio_clip(
        &mut self,
        mut request: Box<super::super::audio_clip::edit::Request>,
    ) {
        if !request.current(self) || !request.ack.claim() {
            self.history_reject(Command::AudioClipEdit(request), Failure::Invalid);
            return;
        }
        if !self.undo.enabled || self.undo.replaying {
            self.history_reject(Command::AudioClipEdit(request), Failure::Unavailable);
            return;
        }
        let t = usize::from(request.baseline.track);
        let s = usize::from(request.baseline.scene);
        let original = self.tracks[t].clips[s].audio.clone();
        let reservation = [original, Some(request.reserved_source.clone())];
        let mut assets = 0;
        for audio in reservation.iter().flatten() {
            if self
                .undo
                .assets
                .binary_search_by_key(&(Arc::as_ptr(audio) as usize), |a| a.0)
                .is_err()
            {
                assets += 1;
            }
        }
        let room = self.undo.assets.len().saturating_add(assets) <= self.undo.assets.capacity();
        let bytes = request
            .bytes()
            .saturating_add(2 * NOTE_LIMIT * std::mem::size_of::<MidiNote>());
        if let Err(reason) = if room {
            self.undo.preflight(bytes)
        } else {
            Err(Failure::Budget)
        } {
            self.history_reject(Command::AudioClipEdit(request), reason);
            return;
        }
        self.undo.begin(Name::AudioClip, 0, self.frames_done);
        self.cancel_recording_clip(t, s);
        std::mem::swap(&mut self.tracks[t].clips[s], &mut request.replacement);
        let value = std::mem::replace(&mut request.replacement, Clip::empty());
        self.undo.append(Patch::Clip {
            track: t as u8,
            scene: s as u16,
            value,
            spare_notes: std::mem::take(&mut request.spare_notes),
            reserved_midi_bytes: request
                .baseline
                .clip
                .lanes
                .as_ref()
                .map_or(0, |l| l.bytes()),
            reserved_audio: reservation,
        });
        let midi_beat = self.precise_midi_beat();
        self.tracks[t].clip_notes_changed(s, self.beat, midi_beat);
        self.undo.recount();
        self.project.edited();
        request.ack.applied();
        self.undo.retire_command(Command::AudioClipEdit(request));
    }
}
