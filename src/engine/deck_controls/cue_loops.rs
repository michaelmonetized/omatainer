use crate::engine::{DeckRt, DeckTransition};

impl DeckRt {
    /// Keep linked cue markers at the saved region's exact start.
    /// Takes this deck; updates existing markers or clears dangling links without allocating or starting playback.
    pub(in crate::engine) fn sync_cue_loop_positions(&mut self) {
        for cue in 0..8 {
            let Some(id) = self.controls.cue_loops[cue] else {
                continue;
            };
            if let Some((start, _)) =
                self.controls.loops[usize::from(id - 1)].filter(|_| self.hotcues[cue].set)
            {
                self.hotcues[cue].pos = start;
            } else {
                self.controls.cue_loops[cue] = None;
            }
        }
    }
    /// Release the association when a cue is deleted.
    /// Takes its validated zero-based cue index; retains the saved loop region and other cue links.
    pub(in crate::engine) fn clear_cue_loop(&mut self, cue: usize) {
        self.controls.cue_loops[cue] = None;
    }
    /// Keep an associated cue and region on the same retained source frame.
    /// Takes cue index, output sample rate and fallback tempo; returns its exact saved position for linked cues or the ordinary quantized target for other cues.
    pub(in crate::engine) fn cue_trigger_position(
        &self,
        cue: usize,
        sample_rate: f32,
        tempo: f32,
    ) -> f64 {
        if self.controls.cue_loops[cue].is_some() {
            self.hotcues[cue].pos
        } else {
            self.cue_quantized_position(self.hotcues[cue].pos, sample_rate, tempo)
        }
    }
    /// Start an associated saved region at the same instant as its cue.
    /// Takes cue index and output sample rate; returns true after a single exact-source loop jump, or false for an ordinary cue or explicit cue-only override.
    pub(in crate::engine) fn jump_cue_loop(&mut self, cue: usize, sample_rate: f32) -> bool {
        if self.controls.cue_only {
            return false;
        }
        let Some(id) = self.controls.cue_loops[cue] else {
            return false;
        };
        let Some((start, length)) = self.controls.loops[usize::from(id - 1)] else {
            return false;
        };
        self.controls.selected = usize::from(id - 1);
        self.loop_start = start;
        self.loop_len = length;
        self.loop_on = true;
        self.transition_to(start, sample_rate, DeckTransition::Jump);
        true
    }
}
