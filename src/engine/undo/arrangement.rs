use super::*;
impl RtEngine {
    pub(in crate::engine) fn history_arrangement(
        &mut self,
        mut request: Box<super::super::arrangement::edit::Request>,
    ) {
        if !request.current(self) || !request.ack.claim() {
            self.history_reject(Command::ArrangementEdit(request), Failure::Invalid);
            return;
        }
        if !self.undo.enabled || self.undo.replaying {
            self.history_reject(Command::ArrangementEdit(request), Failure::Unavailable);
            return;
        }
        let mut assets = 0;
        for plan in self.arrangement.plan.iter().chain(request.reserved.iter()) {
            for sample in plan.media() {
                if self
                    .undo
                    .assets
                    .binary_search_by_key(&(Arc::as_ptr(sample) as usize), |a| a.0)
                    .is_err()
                {
                    assets += 1;
                }
            }
        }
        let room = self.undo.assets.len().saturating_add(assets) <= self.undo.assets.capacity();
        let retained_bytes = request.bytes() + self.arrangement.storage_bytes();
        if let Err(reason) = if room {
            self.undo.preflight(retained_bytes)
        } else {
            Err(Failure::Budget)
        } {
            self.history_reject(Command::ArrangementEdit(request), reason);
            return;
        }
        self.undo.begin(Name::Arrangement, 0, self.frames_done);
        for (slot, track) in self.tracks.iter_mut().enumerate() {
            track.release_clip_notes();
            self.midi_routing.clear_clip(slot as u8);
        }
        let mut replacement = request.replacement.take().unwrap();
        std::mem::swap(&mut self.arrangement, &mut replacement);
        self.arrangement.reset(self.precise_midi_beat());
        let bytes = retained_bytes;
        let original = replacement.plan.clone();
        self.undo.append(Patch::Arrangement {
            value: replacement,
            reserved: [original, request.reserved.take()],
            bytes,
        });
        self.undo.recount();
        self.project.edited();
        request.ack.applied();
        self.undo.retire_command(Command::ArrangementEdit(request));
    }
}
