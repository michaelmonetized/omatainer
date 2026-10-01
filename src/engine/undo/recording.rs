use super::*;
impl RtEngine {
    /// Reserve the original clip plus a separate writable note buffer before
    /// recording's first mutation. A take can include every one of 64 cells.
    pub(in crate::engine) fn history_record_target(&mut self, track: usize, scene: usize) -> bool {
        if !self.undo.enabled || self.undo.replaying {
            return true;
        }
        let clip = &self.tracks[track].clips[scene];
        if clip.kind == ClipKind::Audio {
            // The existing recorder only writes MIDI clips. Monitoring already
            // started, but this input must not create an unchanged inverse.
            return false;
        }
        if clip.notes.len() >= NOTE_LIMIT {
            self.undo.reject(Failure::Notes);
            return false;
        }
        if clip.name.len() > TEXT_LIMIT {
            self.undo.reject(Failure::Text);
            return false;
        }
        let grouped = self.undo.cursor == self.undo.entries.len()
            && self
                .undo
                .entries
                .last()
                .and_then(Option::as_ref)
                .is_some_and(|e| e.name == Name::RecordNotes && e.gesture == self.undo.record_take);
        if grouped && self.undo.entries.last().unwrap().as_ref().unwrap().patches.iter().flatten()
            .any(|p|matches!(p,Patch::Clip {track:t,scene:s,..} if *t as usize==track && *s as usize==scene)) {return true;}
        let bytes = clip.name.capacity().max(TEXT_LIMIT)
            + 2 * NOTE_LIMIT * std::mem::size_of::<MidiNote>()
            + clip.audio.as_ref().map_or(0, |a| sample_bytes(a));
        if let Err(reason) = self.undo.preflight(bytes) {
            self.undo.reject(reason);
            return false;
        }
        let Ok(mut scratch) = self.undo.scratch.as_ref().unwrap().try_recv() else {
            self.undo.reject(Failure::Capacity);
            return false;
        };
        scratch.name.push_str(&clip.name);
        scratch.notes.extend_from_slice(&clip.notes);
        let clip = &mut self.tracks[track].clips[scene];
        let notes = std::mem::replace(&mut clip.notes, scratch.notes);
        let name = std::mem::replace(&mut clip.name, scratch.name);
        let patch = Patch::Clip {
            track: track as u8,
            scene: scene as u8,
            value: Clip {
                kind: clip.kind,
                name,
                bars: clip.bars,
                notes,
                gain: clip.gain,
                audio: clip.audio.clone(),
            },
            spare_notes: Vec::new(),
        };
        if !grouped {
            let gesture = self.undo.gesture;
            self.undo.gesture = self.undo.record_take;
            self.undo.begin(Name::RecordNotes, 0, self.frames_done);
            self.undo.gesture = gesture;
        }
        self.undo.append(patch);
        self.undo.recount();
        true
    }
    pub(in crate::engine) fn history_finish_take(&mut self) {
        self.undo.record_take = self.undo.record_take.wrapping_add(1) | (1 << 63);
    }
}

impl RtEngine {
    pub(in crate::engine) fn history_record_changed(&mut self, track: usize, scene: usize) {
        if !self.undo.enabled || self.undo.replaying {
            return;
        }
        if let Some(index)=self.undo.entries[..self.undo.cursor].iter().rposition(|e|e.as_ref().is_some_and(|e|e.name==Name::RecordNotes && e.patches[..e.len].iter().flatten().any(|p|matches!(p,Patch::Clip {track:t,scene:s,..} if *t as usize==track && *s as usize==scene)))) {self.undo.changed_from(index);}
    }
    pub(in crate::engine) fn history_held_changed(&mut self) {
        if !self.undo.enabled || self.undo.replaying {
            return;
        }
        let held = self.note_recording.held_targets();
        if let Some(index)=self.undo.entries[..self.undo.cursor].iter().position(|e|e.as_ref().is_some_and(|e|e.name==Name::RecordNotes && e.patches[..e.len].iter().flatten().any(|p|matches!(p,Patch::Clip {track,scene,..} if held & (1u64<<(*track as usize*8+*scene as usize))!=0)))) {self.undo.changed_from(index);}
    }
}

impl RtEngine {
    pub(in crate::engine) fn refresh_history_protection(&mut self) {
        if !self.undo.enabled || self.undo.replaying {
            return;
        }
        let held = self.note_recording.held_targets();
        self.undo.protected=self.undo.entries[..self.undo.cursor].iter().flatten().find(|e|e.name==Name::RecordNotes && e.patches[..e.len].iter().flatten().any(|p|matches!(p,Patch::Clip {track,scene,..} if held&(1u64<<(*track as usize*8+*scene as usize))!=0))).map(|e|e.id);
    }
}

impl RtEngine {
    pub(in crate::engine) fn active_recording_history(&self) -> [u64; MAX_PATCHES] {
        let mut owners = [0; MAX_PATCHES];
        let held = self.note_recording.held_targets();
        for entry in self.undo.entries[..self.undo.cursor].iter().rev().flatten() {
            if entry.name != Name::RecordNotes {
                continue;
            }
            for patch in entry.patches.iter().flatten() {
                if let Patch::Clip { track, scene, .. } = patch {
                    let cell = *track as usize * SCENES + *scene as usize;
                    if held & (1 << cell) != 0 && owners[cell] == 0 {
                        owners[cell] = entry.id;
                    }
                }
            }
        }
        owners
    }
}
