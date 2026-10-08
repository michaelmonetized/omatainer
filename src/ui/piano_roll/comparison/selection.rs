use super::*;

#[derive(Clone, Copy, Debug)]
pub(super) struct Criteria {
    pub pitch: [u8; 2],
    pub time: [f64; 2],
    pub velocity: [u8; 2],
    pub overlap: bool,
    pub invert: bool,
}
impl Default for Criteria {
    fn default() -> Self {
        Self {
            pitch: [0, 127],
            time: [-524288.0, 524288.0],
            velocity: [1, 127],
            overlap: false,
            invert: false,
        }
    }
}
impl Criteria {
    /// Select exact source notes in the shared ruler.
    /// Takes notes, their original playback region and display-only offset; returns stable identities without changing source coordinates.
    pub(super) fn select(
        &self,
        notes: &[MidiNote],
        region: Region,
        offset: f64,
    ) -> Result<BTreeSet<NoteId>, String> {
        if self.pitch[0] > self.pitch[1]
            || self.pitch[1] > 127
            || self.velocity[0] == 0
            || self.velocity[0] > self.velocity[1]
            || self.velocity[1] > 127
            || self
                .time
                .iter()
                .any(|v| !v.is_finite() || !(-524288.0..=524288.0).contains(v))
            || self.time[0] > self.time[1]
            || !region.valid()
            || !offset.is_finite()
            || !(-262144.0..=262144.0).contains(&offset)
        {
            return Err(
                "Set ordered finite pitch, time and velocity ranges before selecting notes".into(),
            );
        }
        let mut identities = BTreeSet::new();
        let mut selected = BTreeSet::new();
        for note in notes {
            if !note.id.valid()
                || !identities.insert(note.id)
                || !note.interchange_valid()
                || !note.start.is_finite()
                || !note.len.is_finite()
                || note.pitch > 127
                || note.vel == 0
                || note.vel > 127
            {
                return Err(
                    "This clip needs valid unique source notes before multi-clip selection".into(),
                );
            }
            let start = note.source_start() - region.start + offset;
            let end = start + note.source_duration();
            let time = if self.overlap {
                start <= self.time[1] && end >= self.time[0]
            } else {
                start >= self.time[0] && start <= self.time[1]
            };
            let matches = (self.pitch[0]..=self.pitch[1]).contains(&note.pitch)
                && (self.velocity[0]..=self.velocity[1]).contains(&note.vel)
                && time;
            if matches ^ self.invert {
                selected.insert(note.id);
            }
        }
        Ok(selected)
    }
}
