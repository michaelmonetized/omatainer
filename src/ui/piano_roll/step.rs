use super::*;
use std::collections::BTreeMap;

pub(super) struct Keyboard {
    pub enabled: bool,
    pub octave: i8,
    pub velocity: u8,
    pub chord: BTreeSet<u8>,
    gates: BTreeMap<Key, (u64, u8)>,
}
impl Default for Keyboard {
    fn default() -> Self {
        Self {
            enabled: false,
            octave: 4,
            velocity: 100,
            chord: BTreeSet::new(),
            gates: BTreeMap::new(),
        }
    }
}
pub(super) struct Edit {
    start: f32,
    end: f32,
    inserted: Vec<MidiNote>,
    extended: Vec<(MidiNote, MidiNote)>,
    before: Region,
    after: Region,
    chord: BTreeSet<NoteId>,
    dirty: bool,
}
#[derive(Clone, Copy)]
pub(super) enum Action {
    Pitch,
    Advance,
    Rest,
    Tie,
    Delete,
}

impl Draft {
    /// Insert one musical step.
    /// Takes distinct pitches and velocity; returns an error without changing the draft on invalid input.
    pub(super) fn step_insert(
        &mut self,
        pitches: &BTreeSet<u8>,
        velocity: u8,
    ) -> Result<(), String> {
        let (duration, end) = self.step_bounds()?;
        if self.notes.len() + pitches.len() > crate::engine::project::MAX_NOTES_PER_CLIP
            || velocity == 0
            || velocity > 127
            || pitches.iter().any(|p| *p > 127)
        {
            return Err(
                "The step exceeds the supported note count, pitch or velocity range".into(),
            );
        }
        let mut notes = Vec::with_capacity(pitches.len());
        for pitch in pitches {
            let id = NoteId::new();
            if !id.valid() {
                return Err("A stable step note identity could not be created".into());
            }
            notes.push(MidiNote {
                id,
                channel: 0,
                release_vel: 64,
                source_timing: None,
                pitch: *pitch,
                start: self.cursor.start,
                len: duration,
                vel: velocity,
                muted: false,
            });
        }
        let after = self.step_region(end);
        let mut checked = self.notes.clone();
        checked.extend(notes.iter().cloned());
        if !after.allows(&checked) {
            return Err("The step would exceed the loop's supported note density".into());
        }
        let edit = self.step_edit(end, after, notes.clone(), Vec::new());
        self.step_chord = notes.iter().map(|n| n.id).collect();
        self.selected = self.step_chord.clone();
        self.notes.extend(notes);
        self.step_commit(edit);
        Ok(())
    }
    fn step_bounds(&self) -> Result<(f32, f32), String> {
        let duration = GRIDS.get(self.step_grid).map_or(0.0, |g| g.1) as f32;
        let end = self.cursor.start + duration;
        if !self.cursor.start.is_finite()
            || self.cursor.start < self.region.start as f32
            || duration < 1.0 / 1024.0
            || !end.is_finite()
            || end > 262_144.0
            || end <= self.cursor.start
        {
            return Err("The step cursor or duration is outside the supported clip range".into());
        }
        Ok((duration, end))
    }
    fn step_region(&self, end: f32) -> Region {
        let mut region = self.region;
        if end as f64 > region.end {
            if region.loop_end == region.end {
                region.loop_end = end as f64;
            }
            region.end = end as f64;
        }
        region
    }
    fn step_edit(
        &self,
        end: f32,
        after: Region,
        inserted: Vec<MidiNote>,
        extended: Vec<(MidiNote, MidiNote)>,
    ) -> Edit {
        Edit {
            start: self.cursor.start,
            end,
            inserted,
            extended,
            before: self.region,
            after,
            chord: self.step_chord.clone(),
            dirty: self.dirty,
        }
    }
    fn step_commit(&mut self, edit: Edit) {
        self.cursor.start = edit.end;
        self.region = edit.after;
        self.dirty |=
            !edit.inserted.is_empty() || !edit.extended.is_empty() || edit.before != edit.after;
        if self.steps.len() == 256 {
            self.steps.remove(0);
        }
        self.steps.push(edit);
        if self.cursor.start as f64 >= self.view_beat + 12.0 {
            self.view_beat = (self.cursor.start as f64 - 4.0).max(0.0);
        }
    }
    pub(super) fn step_tie(&mut self) -> Result<(), String> {
        let (duration, end) = self.step_bounds()?;
        let mut extended = Vec::new();
        for note in self
            .notes
            .iter()
            .filter(|n| self.step_chord.contains(&n.id))
        {
            if (note.start + note.len - self.cursor.start).abs() > 1.0 / 1024.0 {
                return Err("The previous chord no longer ends at the step cursor".into());
            }
            let mut next = note.clone();
            next.len += duration;
            next.reconcile_timing();
            extended.push((note.clone(), next));
        }
        if extended.is_empty() || extended.len() != self.step_chord.len() {
            return Err("Enter a chord before tying its next step".into());
        }
        let after = self.step_region(end);
        let edit = self.step_edit(end, after, Vec::new(), extended);
        for (_, next) in &edit.extended {
            *self.notes.iter_mut().find(|n| n.id == next.id).unwrap() = next.clone();
        }
        self.step_commit(edit);
        Ok(())
    }
    pub(super) fn step_delete(&mut self) -> Result<(), String> {
        let Some(edit) = self.steps.last() else {
            return Err("There is no previous step to delete".into());
        };
        if self.cursor.start != edit.end
            || self.region != edit.after
            || edit
                .inserted
                .iter()
                .any(|note| !self.notes.iter().any(|n| n == note))
            || edit
                .extended
                .iter()
                .any(|(_, note)| !self.notes.iter().any(|n| n == note))
        {
            return Err(
                "The last step was edited elsewhere; use the note actions to revise it".into(),
            );
        }
        let edit = self.steps.pop().unwrap();
        self.notes
            .retain(|n| !edit.inserted.iter().any(|i| i.id == n.id));
        for (old, _) in edit.extended {
            let target = self.notes.iter_mut().find(|n| n.id == old.id).unwrap();
            *target = old;
        }
        self.cursor.start = edit.start;
        self.region = edit.before;
        self.step_chord = edit.chord;
        self.selected = self.step_chord.clone();
        self.dirty = true;
        if self.steps.is_empty()
            && self.notes == self.baseline.notes
            && self.name == self.baseline.name
            && self.region == self.baseline.playback_region()
        {
            self.dirty = edit.dirty;
        }
        Ok(())
    }
}

fn offset(key: Key) -> Option<u8> {
    [
        Key::A,
        Key::W,
        Key::S,
        Key::E,
        Key::D,
        Key::F,
        Key::T,
        Key::G,
        Key::Y,
        Key::H,
        Key::U,
        Key::J,
        Key::K,
    ]
    .iter()
    .position(|k| *k == key)
    .map(|i| i as u8)
}
impl Editor {
    pub(super) fn step_action(&mut self, action: Action) {
        let Some(draft) = &mut self.draft else {
            return;
        };
        let result = match action {
            Action::Pitch => {
                draft.step_insert(&BTreeSet::from([draft.cursor.pitch]), draft.cursor.velocity)
            }
            Action::Advance => {
                let pitches = self.keyboard.gates.values().map(|(_, p)| *p).collect();
                draft.step_insert(&pitches, self.keyboard.velocity)
            }
            Action::Rest => draft.step_insert(&BTreeSet::new(), self.keyboard.velocity),
            Action::Tie => draft.step_tie(),
            Action::Delete => draft.step_delete(),
        };
        match result {
            Ok(()) => {
                self.keyboard.chord.clear();
                self.error = None;
            }
            Err(error) => self.error = Some(error),
        }
    }
    pub(super) fn stop_keyboard(&mut self, engine: &Engine) {
        let mut failed = None;
        let mut disconnected = false;
        self.keyboard.gates.retain(|_, (id, _)| {
            match engine.send(Command::MidiAudition {
                id: *id,
                track: 0,
                note: 0,
                vel: 0,
                on: false,
            }) {
                Ok(_) => false,
                Err(crate::engine::SubmissionError::Disconnected) => {
                    disconnected = true;
                    false
                }
                Err(error) => {
                    failed = Some(error);
                    true
                }
            }
        });
        self.stop_requested =
            (self.stop_requested && self.audition.is_some()) || !self.keyboard.gates.is_empty();
        if let Some(error) = failed {
            self.error = Some(format!(
                "Keyboard note release not accepted: {error}; retrying."
            ));
        } else if disconnected {
            self.error = Some("Renderer disconnected; keyboard release outcome is unknown.".into());
        }
    }
    /// Handle the explicitly focused musical keyboard.
    /// Takes the engine, frame context and input-area focus; returns no value and releases notes on focus loss.
    pub(super) fn keyboard_input(
        &mut self,
        engine: &Engine,
        ctx: &egui::Context,
        focused: bool,
        events: Vec<egui::Event>,
    ) {
        if !focused
            || !self.keyboard.enabled
            || !ctx.input(|i| i.focused)
            || keyboard::text_is_focused(ctx)
            || egui::Popup::is_any_open(ctx)
            || self.confirm_discard
            || self.stop_requested
        {
            self.keyboard.chord.clear();
            self.stop_keyboard(engine);
            return;
        }
        for event in events {
            let egui::Event::Key {
                key,
                physical_key,
                pressed,
                repeat,
                modifiers,
            } = event
            else {
                continue;
            };
            let key = physical_key.unwrap_or(key);
            if offset(key).is_some()
                || matches!(
                    key,
                    Key::Z | Key::X | Key::C | Key::V | Key::Space | Key::Backspace
                )
            {
                ctx.input_mut(|i| i.events.retain(|e| !matches!(e, egui::Event::Key { key: k, physical_key: p, .. } if p.unwrap_or(*k) == key)));
                keyboard::block_for_activation(ctx);
            }
            if !pressed {
                if let Some((id, _)) = self.keyboard.gates.get(&key).copied() {
                    match engine.send(Command::MidiAudition {
                        id,
                        track: 0,
                        note: 0,
                        vel: 0,
                        on: false,
                    }) {
                        Ok(_) => {
                            self.keyboard.gates.remove(&key);
                        }
                        Err(error) => {
                            self.stop_requested = true;
                            self.error =
                                Some(format!("Keyboard release not accepted: {error}; retrying."));
                        }
                    }
                    if self.keyboard.gates.is_empty() {
                        if !self.keyboard.chord.is_empty()
                            && self.draft.as_ref().is_some_and(|d| d.step_record)
                        {
                            let pitches = self.keyboard.chord.clone();
                            if let Some(draft) = &mut self.draft {
                                match draft.step_insert(&pitches, self.keyboard.velocity) {
                                    Ok(()) => self.error = None,
                                    Err(error) => self.error = Some(error),
                                }
                            }
                        }
                        self.keyboard.chord.clear();
                    }
                }
                continue;
            }
            if repeat
                || modifiers.command
                || modifiers.ctrl
                || modifiers.alt
                || self.keyboard.gates.contains_key(&key)
            {
                continue;
            }
            if let Some(offset) = offset(key) {
                if modifiers.shift {
                    continue;
                }
                let pitch = (self.keyboard.octave as i16 + 1) * 12 + offset as i16;
                if !(0..=127).contains(&pitch) {
                    self.error =
                        Some("This key is above the MIDI pitch range; lower the octave".into());
                    continue;
                }
                let Some(next) = self.next_audition.checked_add(1) else {
                    self.error = Some("Audition identity exhausted; reopen the application".into());
                    continue;
                };
                let Some(draft) = &self.draft else {
                    continue;
                };
                let id = self.next_audition;
                self.next_audition = next;
                match engine.send(Command::MidiAudition {
                    id,
                    track: draft.baseline.track,
                    note: pitch as u8,
                    vel: self.keyboard.velocity,
                    on: true,
                }) {
                    Ok(_) => {
                        self.keyboard.gates.insert(key, (id, pitch as u8));
                        self.keyboard.chord.insert(pitch as u8);
                    }
                    Err(error) => {
                        self.error = Some(format!("Keyboard note was not accepted: {error}"))
                    }
                }
            } else {
                match key {
                    Key::Space => self.step_action(if modifiers.shift {
                        Action::Tie
                    } else {
                        Action::Advance
                    }),
                    Key::Backspace => self.step_action(Action::Delete),
                    Key::Z | Key::X => {
                        self.stop(engine);
                        self.keyboard.octave = (self.keyboard.octave
                            + if key == Key::Z { -1 } else { 1 })
                        .clamp(-1, 9);
                    }
                    Key::C => {
                        self.keyboard.velocity = self.keyboard.velocity.saturating_sub(10).max(1)
                    }
                    Key::V => {
                        self.keyboard.velocity = self.keyboard.velocity.saturating_add(10).min(127)
                    }
                    _ => {}
                }
            }
        }
    }
}
