use super::*;
impl RtEngine {
    pub(in crate::engine) fn history_clip_management(
        &mut self,
        mut request: Box<super::super::clip_management::edit::Request>,
    ) {
        if !request.current(self) || !request.ack.claim() {
            self.history_reject(Command::ClipManage(request), Failure::Invalid);
            return;
        }
        if !self.undo.enabled || self.undo.replaying {
            self.history_reject(Command::ClipManage(request), Failure::Unavailable);
            return;
        }
        let inverse = request.inverse.as_mut().unwrap();
        inverse.reserve(self);
        let assets = inverse
            .media()
            .filter(|sample| {
                self.undo
                    .assets
                    .binary_search_by_key(&(Arc::as_ptr(sample) as usize), |a| a.0)
                    .is_err()
            })
            .count();
        let room = self.undo.assets.len().saturating_add(assets) <= self.undo.assets.capacity();
        if let Err(reason) = if room {
            self.undo.preflight(
                inverse
                    .bytes()
                    .saturating_add(inverse.media().map(|s| sample_bytes(s)).sum::<usize>()),
            )
        } else {
            Err(Failure::Budget)
        } {
            self.history_reject(Command::ClipManage(request), reason);
            return;
        }
        self.undo.begin(Name::ClipManagement, 0, self.frames_done);
        let mut inverse = request.inverse.take().unwrap();
        inverse.swap(self);
        self.undo.append(Patch::ClipManagement(inverse));
        self.undo.recount();
        self.project.edited();
        request.ack.applied();
        self.undo.retire_command(Command::ClipManage(request));
    }
}
