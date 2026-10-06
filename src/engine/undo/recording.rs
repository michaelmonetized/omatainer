use super::*;
impl RtEngine {
    /// Reserve the original clip plus a separate writable note buffer before
    /// recording's first mutation. A take can include every one of 64 cells.
    pub(in crate::engine) fn history_record_target(&mut self, track: usize, scene: usize) -> bool {
        if self.midi_note_count() >= super::super::project::MAX_TOTAL_NOTES
            || !self.midi_recording_density_available(track, scene) {
            self.undo.reject(Failure::Notes); return false;
        }
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
                .back()
                .and_then(Option::as_ref)
                .is_some_and(|e| e.name == Name::RecordNotes && e.gesture == self.undo.record_take);
        if grouped && self.undo.entries.back().unwrap().as_ref().unwrap().patches.iter().flatten()
            .any(|p|matches!(p,Patch::Clip {track:t,scene:s,..} if *t as usize==track && *s as usize==scene)) {return true;}
        let bytes = clip.name.capacity().max(TEXT_LIMIT)
            + 2 * NOTE_LIMIT * std::mem::size_of::<MidiNote>()
            + clip.audio.as_ref().map_or(0, |a| sample_bytes(a))
            + clip.lanes.as_ref().map_or(0, |l| l.bytes());
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
            scene: scene as u16,
            value: Clip {
                properties: clip.properties,
                audio_region: clip.audio_region, lanes: clip.lanes.clone(),
                        region: clip.region,
                kind: clip.kind,
                name,
                bars: clip.bars,
                notes,
                gain: clip.gain,
                audio: clip.audio.clone(),
            },
            spare_notes: Vec::new(),
            reserved_midi_bytes: 0,
            reserved_audio: [None, None],
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
    /// Called immediately after history_record_target reserved the inverse.
    pub(in crate::engine) fn recording_history_owner(&self, track: usize, scene: usize) -> u64 {
        if !self.undo.enabled || self.undo.replaying {
            return 0;
        }
        self.undo.entries.back().and_then(Option::as_ref).filter(|entry|
            entry.name == Name::RecordNotes && entry.patches[..entry.len].iter().flatten().any(|patch|
                matches!(patch, Patch::Clip { track: t, scene: s, .. } if *t as usize == track && *s as usize == scene)))
            .map_or(0, |entry| entry.id)
    }
    pub(in crate::engine) fn history_record_changed(&mut self, owner: u64) {
        if !self.undo.enabled || self.undo.replaying || owner == 0 {
            return;
        }
        if let Some(index) = self.undo.entries.iter().take(self.undo.cursor)
            .position(|entry| entry.as_ref().is_some_and(|entry| entry.id == owner))
        {
            self.undo.changed_from(index);
        }
    }
    pub(in crate::engine) fn history_record_finished(
        &mut self,
        owner: u64,
        track: usize,
        scene: usize,
        note_index: usize,
        duration: f32,
    ) {
        if !self.undo.enabled || owner == 0 {
            return;
        }
        // Later recording inverses may contain this held note. Finalizing its
        // live duration must also finalize those historical snapshots, otherwise
        // undoing a later note would resurrect a provisional quarter-beat hold.
        if let Some(index) = self.undo.entries.iter().take(self.undo.cursor)
            .position(|entry| entry.as_ref().is_some_and(|entry| entry.id == owner))
        {
            for entry in self.undo.entries.iter_mut().skip(index + 1).flatten() {
                for patch in entry.patches[..entry.len].iter_mut().flatten() {
                    if let Patch::Clip {
                        track: t,
                        scene: s,
                        value,
                        ..
                    } = patch
                    {
                        if *t as usize == track && *s as usize == scene {
                            if let Some(note) = value.notes.get_mut(note_index) {
                                note.len = duration;
                            }
                        }
                    }
                }
            }
        }
        self.history_record_changed(owner);
    }
    fn held_history_entry(&self) -> Option<usize> {
        let (owners, len) = self.note_recording.history_owners();
        if len == 0 {
            return None;
        }
        self.undo.entries.iter().take(self.undo.cursor)
            .position(|entry| {
                entry
                    .as_ref()
                    .is_some_and(|entry| owners[..len].contains(&entry.id))
            })
    }
    pub(in crate::engine) fn history_held_changed(&mut self) {
        if !self.undo.enabled || self.undo.replaying {
            return;
        }
        if let Some(index) = self.held_history_entry() {
            self.undo.changed_from(index);
        }
    }
    pub(in crate::engine) fn refresh_history_protection(&mut self) {
        if !self.undo.enabled || self.undo.replaying {
            return;
        }
        self.undo.protected = self
            .held_history_entry()
            .map(|index| self.undo.entries[index].as_ref().unwrap().id);
    }
    pub(in crate::engine) fn active_recording_history(
        &self,
    ) -> [(u64, usize, usize); super::super::recording::CAPTURES] {
        self.note_recording.active_history()
    }
}
