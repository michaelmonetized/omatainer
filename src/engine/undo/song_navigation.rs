use super::super::song_navigation;
use super::*;
impl RtEngine {
    /// Publish one reviewed song-section edit and its retained inverse.
    /// Takes a worker-prepared request; applies and acknowledges it atomically or retires the complete rejected command without changing navigation.
    pub(in crate::engine) fn history_song_navigation(
        &mut self,
        mut request: Box<song_navigation::Request>,
    ) {
        if !request.current(self) || !request.ack.claim() {
            self.history_reject(Command::SongNavigationEdit(request), Failure::Invalid);
            return;
        }
        if self.undo.enabled {
            if let Err(error) = self.undo.preflight(request.bytes()) {
                self.history_reject(Command::SongNavigationEdit(request), error);
                return;
            }
        }
        self.navigation.cancel();
        std::mem::swap(&mut self.navigation.saved, &mut request.replacement);
        if self.undo.enabled {
            self.undo.begin(Name::SongNavigation, 0, self.frames_done);
            self.undo.append(Patch::SongNavigation {
                saved: request.inverse.take(),
                bytes: request.bytes(),
            });
            self.undo.recount();
        }
        if let Some((beat, grid)) = request.jump {
            self.song_navigation_queue(beat, grid, request.loop_after);
        }
        self.project.edited();
        request.ack.applied();
        self.undo
            .retire_command(Command::SongNavigationEdit(request));
    }
    /// Toggle the saved loop switch through native Undo admission.
    /// Takes the current renderer; records one inverse or preserves the switch when braces or storage are unavailable.
    pub(in crate::engine) fn history_song_loop(&mut self) {
        let Some(saved) = self
            .navigation
            .saved
            .as_ref()
            .filter(|saved| saved.model.loop_region.is_some())
            .cloned()
        else {
            self.navigation.error = Some(song_navigation::Error::MissingLoop);
            return;
        };
        let bytes = 2 * saved.model.bytes();
        if self.undo.enabled {
            if let Err(error) = self.undo.preflight(bytes) {
                self.undo.reject(error);
                return;
            }
            self.undo.begin(Name::SongNavigation, 0, self.frames_done);
            self.undo.append(Patch::SongNavigation {
                saved: Some(saved.clone()),
                bytes,
            });
            self.undo.recount();
        }
        self.navigation.cancel();
        self.navigation.saved.as_mut().unwrap().looping = !saved.looping;
        self.project.edited();
    }
}
