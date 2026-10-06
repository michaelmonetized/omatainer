use super::*;

#[derive(Clone)]
struct Lane {
    enabled: bool,
    pitch: u8,
    steps: u16,
    pulses: u16,
    rotation: u16,
    grid: usize,
    density: f64,
    velocity: u8,
    accent: u8,
    accent_every: u16,
}
impl Default for Lane {
    fn default() -> Self {
        Self {
            enabled: true,
            pitch: 36,
            steps: 16,
            pulses: 4,
            rotation: 0,
            grid: 3,
            density: 1.0,
            velocity: 90,
            accent: 110,
            accent_every: 4,
        }
    }
}
pub(super) struct Generator {
    lanes: Vec<Lane>,
    seed: u64,
    seed_text: String,
    swing: f64,
    gate: f64,
    replace: bool,
    preview: Option<Preview>,
    message: String,
}
struct Preview {
    original: Vec<MidiNote>,
    region: Region,
    selected: BTreeSet<NoteId>,
    dirty: bool,
    generated: Vec<MidiNote>,
    resulting_region: Region,
}
impl Default for Generator {
    fn default() -> Self {
        Self {
            lanes: vec![Lane::default()],
            seed: 1,
            seed_text: "1".into(),
            swing: 0.0,
            gate: 0.5,
            replace: false,
            preview: None,
            message:
                "Preview writes editable notes into this draft. Apply commits them to the clip."
                    .into(),
        }
    }
}
fn gcd(mut a: u64, mut b: u64) -> u64 {
    while b != 0 {
        let remainder = a % b;
        a = b;
        b = remainder;
    }
    a
}
fn random(seed: u64, lane: usize, step: u64) -> f64 {
    let mut value = seed
        .wrapping_add((lane as u64).wrapping_mul(0x9e3779b97f4a7c15))
        .wrapping_add(step);
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d049bb133111eb);
    ((value ^ (value >> 31)) >> 11) as f64 / ((1u64 << 53) as f64)
}
impl Generator {
    /// Generate a complete polymetric rhythm.
    /// Takes the source-beat origin; returns ordinary notes and the least-common pattern end, or refuses excessive work.
    fn generate(&self, start: f64) -> Result<(Vec<MidiNote>, f64), String> {
        if !start.is_finite()
            || !(0.0..=262_144.0).contains(&start)
            || self.lanes.is_empty()
            || self.lanes.len() > 8
            || !self.swing.is_finite()
            || !(0.0..=0.49).contains(&self.swing)
            || !self.gate.is_finite()
            || !(0.01..=1.0).contains(&self.gate)
        {
            return Err(
                "Rhythm needs a valid cursor, up to eight voices, swing 0–49% and gate 1–100%"
                    .into(),
            );
        }
        let mut prepared = Vec::new();
        let mut period = 1u64;
        for (index, lane) in self.lanes.iter().enumerate().filter(|(_, l)| l.enabled) {
            if lane.pitch > 127
                || lane.steps == 0
                || lane.steps > 64
                || lane.pulses > lane.steps
                || lane.grid == 0
                || lane.grid >= GRIDS.len()
                || !lane.density.is_finite()
                || !(0.0..=1.0).contains(&lane.density)
                || lane.velocity == 0
                || lane.velocity > 127
                || lane.accent == 0
                || lane.accent > 127
                || lane.accent_every > 64
            {
                return Err(format!(
                    "Rhythm voice {} has invalid pitch, steps, pulses, density or velocity",
                    index + 1
                ));
            }
            let step_ticks = (GRIDS[lane.grid].1 * 960.0).round() as u64;
            let ticks = u64::from(lane.steps) * step_ticks;
            period = (period / gcd(period, ticks))
                .checked_mul(ticks)
                .ok_or("Pattern lengths exceed the supported common loop")?;
            if period > 960 * 262_144 {
                return Err("The common pattern would exceed 262144 beats".into());
            }
            prepared.push((index, lane, step_ticks));
        }
        if prepared.is_empty() {
            return Err("Enable at least one rhythm voice".into());
        }
        let work: u64 = prepared.iter().map(|(_, _, step)| period / step).sum();
        if work > 65_536 {
            return Err("These independent lengths need more than 65536 generated steps; choose shorter common periods".into());
        }
        let end = start + period as f64 / 960.0;
        if end > 262_144.0 {
            return Err("The generated rhythm would leave the supported beat range".into());
        }
        let mut notes = Vec::new();
        for (index, lane, step_ticks) in prepared {
            for step in 0..period / step_ticks {
                let local = step % u64::from(lane.steps);
                let phase = (local + u64::from(lane.steps) - u64::from(lane.rotation % lane.steps))
                    % u64::from(lane.steps);
                if phase * u64::from(lane.pulses) % u64::from(lane.steps) >= u64::from(lane.pulses)
                    || lane.density == 0.0
                    || random(self.seed, index, step) >= lane.density
                {
                    continue;
                }
                if notes.len() == crate::engine::project::MAX_NOTES_PER_CLIP {
                    return Err("Generated rhythm exceeds 8192 notes; nothing changed".into());
                }
                let delay = if local % 2 == 1 { self.swing } else { 0.0 };
                let beat = start + (step as f64 + delay) * step_ticks as f64 / 960.0;
                let length = (self.gate * step_ticks as f64 / 960.0).min(end - beat);
                let id = NoteId::new();
                if !id.valid() {
                    return Err("Rhythm note identities are unavailable".into());
                }
                notes.push(MidiNote {
                    id,
                    channel: 0,
                    release_vel: 64,
                    source_timing: None,
                    pitch: lane.pitch,
                    start: beat as f32,
                    len: length as f32,
                    vel: if lane.accent_every != 0 && local % u64::from(lane.accent_every) == 0 {
                        lane.accent
                    } else {
                        lane.velocity
                    },
                    muted: false,
                });
            }
        }
        notes.sort_by(|a, b| {
            a.start
                .total_cmp(&b.start)
                .then_with(|| a.pitch.cmp(&b.pitch))
        });
        Ok((notes, end))
    }
    pub(super) fn committed(&mut self) {
        self.preview = None;
    }
    fn preview(&mut self, draft: &mut Draft) -> Result<(), String> {
        self.ensure_unedited(draft)?;
        let (mut generated, end) = self.generate(draft.cursor.start as f64)?;
        let (original, region, selected, dirty) = self.preview.as_ref().map_or_else(
            || {
                (
                    draft.notes.clone(),
                    draft.region,
                    draft.selected.clone(),
                    draft.dirty,
                )
            },
            |p| (p.original.clone(), p.region, p.selected.clone(), p.dirty),
        );
        let mut result = if self.replace {
            Vec::new()
        } else {
            original.clone()
        };
        if result.len() + generated.len() > crate::engine::project::MAX_NOTES_PER_CLIP {
            return Err("Existing and generated notes would exceed 8192; nothing changed".into());
        }
        let generated_ids = generated.iter().map(|n| n.id).collect();
        result.append(&mut generated);
        let mut next_region = region;
        if end > region.end {
            next_region.end = end;
        }
        next_region.loop_start = draft.cursor.start as f64;
        next_region.loop_end = end;
        if next_region.loop_start < next_region.start || !next_region.allows(&result) {
            return Err("Generated rhythm does not fit the clip's loop or note-density limits; nothing changed".into());
        }
        self.message = format!("{} generated notes · common loop {:.6} beats · seed {}. Apply MIDI edit commits this preview.", result.len() - if self.replace { 0 } else { original.len() }, end - draft.cursor.start as f64, self.seed);
        self.preview = Some(Preview {
            original,
            region,
            selected,
            dirty,
            generated: result.clone(),
            resulting_region: next_region,
        });
        draft.notes = result;
        draft.region = next_region;
        draft.selected = generated_ids;
        draft.steps.clear();
        draft.step_chord.clear();
        draft.dirty = true;
        Ok(())
    }
    fn ensure_unedited(&self, draft: &Draft) -> Result<(), String> {
        if self
            .preview
            .as_ref()
            .is_some_and(|p| p.generated != draft.notes || p.resulting_region != draft.region)
        {
            return Err("Preview notes or loop bounds were edited. Apply those edits before starting another preview; nothing was replaced.".into());
        }
        Ok(())
    }
    fn restore(&mut self, draft: &mut Draft) -> Result<(), String> {
        self.ensure_unedited(draft)?;
        if let Some(preview) = self.preview.take() {
            draft.notes = preview.original;
            draft.region = preview.region;
            draft.selected = preview.selected;
            draft.dirty = preview.dirty || draft.name != draft.baseline.name;
            self.message = "Original draft notes and loop bounds restored.".into();
        }
        Ok(())
    }
}
fn integer(ui: &mut Ui, label: &str, value: &mut u16, min: u16, max: u16) {
    let mut number_value = f64::from(*value);
    if number(ui, label, &mut number_value, f64::from(min), f64::from(max)) {
        *value = number_value.round() as u16;
    }
}
fn value(ui: &mut Ui, label: &str, value: &mut u8) {
    let mut number_value = f64::from(*value);
    if number(ui, label, &mut number_value, 1.0, 127.0) {
        *value = number_value.round() as u8;
    }
}
fn enabled_button(ui: &mut Ui, label: &str, enabled: bool) -> egui::Response {
    let response = ui.add_enabled(enabled, egui::Button::new(label));
    accessibility::button(ui, &response, label, None);
    help::annotate(ui, &response, HelpControl::MidiRhythm);
    response
}
pub(super) fn show(ui: &mut Ui, draft: &mut Draft) -> Result<bool, String> {
    let mut generator = std::mem::take(&mut draft.rhythm);
    let mut preview = false;
    let mut restore = false;
    egui::CollapsingHeader::new("Rhythm generator").id_salt("midi-rhythm").show(ui, |ui| {
        ui.label(&generator.message);
        ui.label("Each voice repeats at its own step length. Preview spans their least-common loop in quarter-note beats; meter changes do not rescale the notes. Pitch stays explicit when scale context changes.");
        ui.horizontal_wrapped(|ui| {
            let label = ui.label("Seed");
            let seed = ui.add(egui::TextEdit::singleline(&mut generator.seed_text).char_limit(20).desired_width(140.0)).labelled_by(label.id);
            seed.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, "Rhythm seed"));
            help::annotate(ui, &seed, HelpControl::MidiRhythm);
            number(ui, "Rhythm swing", &mut generator.swing, 0.0, 0.49);
            number(ui, "Rhythm gate", &mut generator.gate, 0.01, 1.0);
            let replace = ui.checkbox(&mut generator.replace, "Replace draft notes");
            accessibility::button(ui, &replace, "Replace draft notes", Some(generator.replace));
            help::annotate(ui, &replace, HelpControl::MidiRhythm);
        });
        for (index, lane) in generator.lanes.iter_mut().enumerate() {
            ui.push_id(("rhythm-voice", index), |ui| accessibility::scope(ui, &format!("Rhythm voice {}", index + 1), |ui| {
                ui.horizontal_wrapped(|ui| {
                    let enabled = ui.checkbox(&mut lane.enabled, format!("Voice {}", index + 1));
                    accessibility::button(ui, &enabled, "Enabled", Some(lane.enabled));
                    let mut pitch = lane.pitch as f64; if number(ui, "Rhythm pitch", &mut pitch, 0.0, 127.0) { lane.pitch = pitch.round() as u8; }
                    integer(ui, "Rhythm steps", &mut lane.steps, 1, 64);
                    integer(ui, "Rhythm pulses", &mut lane.pulses, 0, lane.steps);
                    integer(ui, "Rhythm rotation", &mut lane.rotation, 0, 63);
                    let grid = egui::ComboBox::from_id_salt("rhythm-grid").selected_text(GRIDS[lane.grid].0)
                        .show_ui(ui, |ui| { for (i, (name, _)) in GRIDS.iter().enumerate().skip(1) { ui.selectable_value(&mut lane.grid, i, *name); } });
                    help::annotate(ui, &grid.response, HelpControl::MidiRhythm);
                    number(ui, "Rhythm density", &mut lane.density, 0.0, 1.0);
                    value(ui, "Rhythm velocity", &mut lane.velocity);
                    value(ui, "Rhythm accent", &mut lane.accent);
                    integer(ui, "Accent every", &mut lane.accent_every, 0, 64);
                });
            }));
        }
        ui.horizontal_wrapped(|ui| {
            if enabled_button(ui, "Add rhythm voice", generator.lanes.len() < 8).clicked() {
                let pitch = 36 + generator.lanes.len() as u8;
                generator.lanes.push(Lane { pitch, ..Lane::default() });
            }
            if enabled_button(ui, "Remove last rhythm voice", generator.lanes.len() > 1).clicked() { generator.lanes.pop(); }
            if button(ui, "Preview rhythm").clicked() { preview = true; }
            if button(ui, "Restore before rhythm preview").clicked() { restore = true; }
        });
    });
    let result = if preview {
        match generator.seed_text.parse::<u64>() {
            Ok(seed) => {
                generator.seed = seed;
                generator.preview(draft)
            }
            Err(_) => {
                Err("Rhythm seed must be a whole number from 0 through 18446744073709551615".into())
            }
        }
    } else if restore {
        generator.restore(draft)
    } else {
        Ok(())
    };
    draft.rhythm = generator;
    result.map(|_| preview || restore)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn content(notes: &[MidiNote]) -> Vec<(u8, f32, f32, u8)> {
        notes
            .iter()
            .map(|n| (n.pitch, n.start, n.len, n.vel))
            .collect()
    }
    #[test]
    fn independent_odd_lengths_rotation_accents_and_swing_match_the_score() {
        let generator = Generator {
            lanes: vec![
                Lane {
                    steps: 5,
                    pulses: 2,
                    accent_every: 3,
                    ..Lane::default()
                },
                Lane {
                    pitch: 42,
                    steps: 7,
                    pulses: 3,
                    rotation: 1,
                    accent_every: 0,
                    ..Lane::default()
                },
            ],
            swing: 0.2,
            ..Generator::default()
        };
        let (notes, end) = generator.generate(2.0).unwrap();
        assert_eq!(end, 10.75);
        assert_eq!(notes.len(), 29);
        let kick = notes.iter().filter(|n| n.pitch == 36).collect::<Vec<_>>();
        assert_eq!(kick.len(), 14);
        for (index, note) in kick.iter().enumerate() {
            let step = (index / 2) * 5 + if index % 2 == 0 { 0 } else { 3 };
            let local = step % 5;
            let expected = 2.0 + (step as f32 + if local % 2 == 1 { 0.2 } else { 0.0 }) * 0.25;
            assert!((note.start - expected).abs() < 1e-6);
            assert_eq!(note.vel, 110);
        }
        let hat = notes.iter().filter(|n| n.pitch == 42).collect::<Vec<_>>();
        for (index, note) in hat.iter().enumerate() {
            let step = (index / 3) * 7 + [1, 4, 6][index % 3];
            let expected = 2.0 + (step as f32 + if step % 7 % 2 == 1 { 0.2 } else { 0.0 }) * 0.25;
            assert!((note.start - expected).abs() < 1e-6);
            assert_eq!(note.vel, 90);
        }
    }
    #[test]
    fn empty_full_and_seeded_density_are_defined_and_bounded() {
        let mut generator = Generator::default();
        generator.lanes[0].pulses = 0;
        assert!(generator.generate(0.0).unwrap().0.is_empty());
        generator.lanes[0].pulses = 16;
        assert_eq!(generator.generate(0.0).unwrap().0.len(), 16);
        generator.lanes[0].density = 0.5;
        assert_eq!(
            content(&generator.generate(0.0).unwrap().0),
            content(&generator.generate(0.0).unwrap().0)
        );
        let original = content(&generator.generate(0.0).unwrap().0);
        generator.seed = 999;
        assert_ne!(original, content(&generator.generate(0.0).unwrap().0));
        generator.lanes[0].density = 0.0;
        assert!(generator.generate(0.0).unwrap().0.is_empty());
        generator.lanes = vec![
            Lane {
                steps: 61,
                ..Lane::default()
            },
            Lane {
                steps: 59,
                ..Lane::default()
            },
            Lane {
                steps: 53,
                ..Lane::default()
            },
        ];
        assert!(generator.generate(0.0).is_err());
        assert!(generator.generate(f64::NAN).is_err());
    }
}
