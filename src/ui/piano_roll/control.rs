use super::*;
use crate::engine::midi_data::{ControlKind, Label, Lanes};
use crate::midi_file::{Message, Meta};

#[derive(Clone)]
pub(super) struct Controls {
    ppqn: u16,
    end_tick: u64,
    messages: Vec<Message>,
    meta: Vec<Meta>,
    labels: Vec<Label>,
    lane: usize,
    channel: u8,
    cc: u8,
    beat: f64,
    value: u16,
    bank: [u8; 2],
    name: String,
    selected: Option<Message>,
    points: Vec<usize>,
    pub dirty: bool,
}
impl Controls {
    pub fn new(source: Option<&Lanes>) -> Self {
        let mut result = Self {
            ppqn: source.map_or(960, |l| l.ppqn),
            end_tick: source.map_or(0, |l| l.end_tick),
            messages: source.map_or_else(Vec::new, |l| l.messages.clone()),
            meta: source.map_or_else(Vec::new, |l| l.meta.clone()),
            labels: source.map_or_else(Vec::new, |l| l.labels.clone()),
            lane: 0,
            channel: 0,
            cc: 1,
            beat: 0.0,
            value: 0,
            bank: [0, 0],
            name: String::new(),
            selected: None,
            points: Vec::new(),
            dirty: false,
        };
        result.refresh();
        result
    }
    fn kind(&self) -> ControlKind {
        match self.lane {
            1 => ControlKind::Bend,
            2 => ControlKind::Pressure,
            3 => ControlKind::Program {
                bank_msb: self.bank[0],
                bank_lsb: self.bank[1],
                program: self.value.min(127) as u8,
            },
            _ => ControlKind::Cc {
                controller: self.cc,
            },
        }
    }
    fn maximum(&self) -> u16 {
        if self.lane == 1 {
            16383
        } else {
            127
        }
    }
    fn accepts(&self, m: &Message) -> bool {
        m.bytes[0] & 15 == self.channel
            && match self.lane {
                1 => m.bytes[0] & 0xf0 == 0xe0,
                2 => m.bytes[0] & 0xf0 == 0xd0,
                3 => m.bytes[0] & 0xf0 == 0xc0,
                _ => m.bytes[0] & 0xf0 == 0xb0 && m.bytes[1] == self.cc,
            }
    }
    fn refresh(&mut self) {
        self.points = self
            .messages
            .iter()
            .enumerate()
            .filter(|(_, m)| self.accepts(m))
            .map(|(i, _)| i)
            .collect();
        self.selected = None;
        self.value = self.value.min(self.maximum());
        self.name = self
            .labels
            .iter()
            .find(|l| l.channel == self.channel && l.control == self.kind())
            .map_or_else(String::new, |l| l.name.clone());
    }
    fn value_of(&self, m: Message) -> u16 {
        match m.bytes[0] & 0xf0 {
            0xe0 => u16::from(m.bytes[1]) | u16::from(m.bytes[2]) << 7,
            0xc0 | 0xd0 => u16::from(m.bytes[1]),
            _ => u16::from(m.bytes[2]),
        }
    }
    fn select(&mut self, index: usize) {
        let m = self.messages[index];
        self.selected = Some(m);
        self.beat = m.tick as f64 / f64::from(self.ppqn);
        self.value = self.value_of(m);
        if self.lane == 3 {
            self.bank = [0, 0];
            for prior in &self.messages[..index] {
                if prior.bytes[0] == 0xb0 | self.channel {
                    if prior.bytes[1] == 0 {
                        self.bank[0] = prior.bytes[2];
                    }
                    if prior.bytes[1] == 32 {
                        self.bank[1] = prior.bytes[2];
                    }
                }
            }
            self.name = self
                .labels
                .iter()
                .find(|l| l.channel == self.channel && l.control == self.kind())
                .map_or_else(String::new, |l| l.name.clone());
        }
    }
    fn tick(&self) -> Result<u64, String> {
        if !self.beat.is_finite() || !(0.0..=262144.0).contains(&self.beat) {
            return Err("Controller position must be within 0–262144 beats".into());
        }
        Ok((self.beat * f64::from(self.ppqn)).round() as u64)
    }
    fn next_order(&self, notes: &[MidiNote], count: u32) -> Result<u32, String> {
        let maximum = self
            .messages
            .iter()
            .map(|m| m.order)
            .chain(self.meta.iter().map(|m| m.order))
            .chain(
                notes
                    .iter()
                    .filter_map(|n| n.source_timing)
                    .map(|t| t.end_order),
            )
            .max();
        let first = maximum
            .map_or(Some(0), |m| m.checked_add(1))
            .ok_or("MIDI event ordering is exhausted")?;
        first
            .checked_add(count)
            .ok_or("MIDI event ordering is exhausted")?;
        Ok(first)
    }
    fn packet(&self, tick: u64, order: u32) -> Result<Message, String> {
        if self.channel >= 16
            || self.cc >= 120
            || self.lane > 3
            || self.value > self.maximum()
            || self.bank.iter().any(|v| *v > 127)
        {
            return Err("Choose a valid MIDI channel, controller and value".into());
        }
        let (bytes, length) = match self.lane {
            1 => (
                [
                    0xe0 | self.channel,
                    (self.value & 127) as u8,
                    (self.value >> 7) as u8,
                ],
                3,
            ),
            2 => ([0xd0 | self.channel, self.value as u8, 0], 2),
            3 => ([0xc0 | self.channel, self.value as u8, 0], 2),
            _ => ([0xb0 | self.channel, self.cc, self.value as u8], 3),
        };
        Ok(Message {
            tick,
            order,
            bytes,
            length,
        })
    }
    fn insert(&mut self, notes: &[MidiNote]) -> Result<(), String> {
        let count = if self.lane == 3 { 3 } else { 1 };
        if self.messages.len() + self.meta.len() + count >= crate::midi_file::MAX_EVENTS {
            return Err("This MIDI clip has reached its event limit".into());
        }
        let tick = self.tick()?;
        let order = self.next_order(notes, count as u32)?;
        let message = self.packet(tick, order + count as u32 - 1)?;
        if self.lane == 3 {
            for (i, cc) in [0, 32].into_iter().enumerate() {
                self.messages.push(Message {
                    tick,
                    order: order + i as u32,
                    bytes: [0xb0 | self.channel, cc, self.bank[i]],
                    length: 3,
                });
            }
        }
        self.messages.push(message);
        self.end_tick = self.end_tick.max(tick);
        self.changed();
        self.selected = Some(message);
        Ok(())
    }
    fn changed(&mut self) {
        self.messages.sort_unstable_by_key(|m| (m.tick, m.order));
        self.dirty = true;
        let name = self.name.clone();
        self.refresh();
        self.name = name;
    }
    fn replace(&mut self, notes: &[MidiNote]) -> Result<(), String> {
        let old = self.selected.ok_or("Select a controller point first")?;
        let index = self
            .messages
            .iter()
            .position(|m| *m == old)
            .ok_or("The selected event has changed; select it again")?;
        let tick = self.tick()?;
        let order = if self.lane == 3 {
            self.next_order(notes, 3)? + 2
        } else {
            old.order
        };
        let next = self.packet(tick, order)?;
        if self.lane == 3 {
            if self.messages.len() + self.meta.len() + 2 >= crate::midi_file::MAX_EVENTS {
                return Err("This MIDI clip has reached its event limit".into());
            }
            for (i, cc) in [0, 32].into_iter().enumerate() {
                self.messages.push(Message {
                    tick,
                    order: order - 2 + i as u32,
                    bytes: [0xb0 | self.channel, cc, self.bank[i]],
                    length: 3,
                });
            }
        }
        self.messages[index] = next;
        self.end_tick = self.end_tick.max(next.tick);
        self.changed();
        self.selected = Some(next);
        Ok(())
    }
    fn delete(&mut self) -> Result<(), String> {
        let old = self.selected.ok_or("Select a controller point first")?;
        let index = self
            .messages
            .iter()
            .position(|m| *m == old)
            .ok_or("The selected event has changed; select it again")?;
        self.messages.remove(index);
        self.changed();
        Ok(())
    }
    fn label(&mut self) -> Result<(), String> {
        let kind = self.kind();
        let name = self.name.trim().to_owned();
        if name.len() > 256 || name.chars().any(char::is_control) {
            return Err("Device labels support 256 bytes without control characters".into());
        }
        let old = self
            .labels
            .iter()
            .position(|l| l.channel == self.channel && l.control == kind);
        if !name.is_empty() && old.is_none() && self.labels.len() >= 4096 {
            return Err("This clip already contains 4096 device labels".into());
        }
        if let Some(index) = old {
            self.labels.remove(index);
        }
        if !name.is_empty() {
            self.labels.push(Label {
                channel: self.channel,
                control: kind,
                name,
            });
        }
        self.dirty = true;
        Ok(())
    }
    /// Capture editable MIDI content.
    /// Takes draft notes; returns an owned producer payload with the current controller data.
    pub(super) fn content(&self, notes:&[MidiNote]) -> crate::engine::midi_tools::Content {
        crate::engine::midi_tools::Content {notes:notes.to_vec(),ppqn:self.ppqn,end_tick:self.end_tick,messages:self.messages.clone(),meta:self.meta.clone(),labels:self.labels.clone()}
    }
    /// Guard controller content.
    /// Takes a captured payload; returns whether current source data still matches.
    pub(super) fn matches(&self, content:&crate::engine::midi_tools::Content) -> bool {
        self.ppqn==content.ppqn && self.end_tick==content.end_tick && self.messages==content.messages && self.meta==content.meta && self.labels==content.labels
    }
    /// Receive prepared controller content.
    /// Takes an owned payload and original dirty decision; moves its lanes into the draft and rebuilds the visible point list.
    pub(super) fn install(&mut self, content:&mut crate::engine::midi_tools::Content, dirty:bool) {
        self.ppqn=content.ppqn;self.end_tick=content.end_tick;self.messages=std::mem::take(&mut content.messages);self.meta=std::mem::take(&mut content.meta);self.labels=std::mem::take(&mut content.labels);self.dirty=dirty;self.refresh();
    }
    pub fn prepared(
        &self,
        notes: &[MidiNote],
        end: f64,
        cancel: &AtomicBool,
    ) -> Result<Option<Arc<Lanes>>, String> {
        if self.messages.is_empty() && self.meta.is_empty() && self.labels.is_empty() {
            return Ok(None);
        }
        let note_end = notes
            .iter()
            .map(|n| {
                ((n.source_start() + n.source_duration()) * f64::from(self.ppqn)).ceil() as u64
            })
            .max()
            .unwrap_or(0);
        let end_tick = self
            .end_tick
            .max(note_end)
            .max((end * f64::from(self.ppqn)).ceil() as u64);
        Lanes::named_with_cancel(
            self.ppqn,
            end_tick,
            self.messages.clone(),
            self.meta.clone(),
            self.labels.clone(),
            &mut || cancel.load(Ordering::Acquire),
        )
        .map(Some)
    }
}

pub(super) fn show(ui: &mut Ui, draft: &mut Draft, theme: &Theme) -> Result<bool, String> {
    let mut outcome = Ok(false);
    let shown = egui::CollapsingHeader::new("MIDI controller lanes").id_salt("midi-controllers").show(ui, |ui| {
        let controls = &mut draft.controls;
        let before = (controls.lane, controls.channel, controls.cc);
        ui.horizontal_wrapped(|ui| {
            egui::ComboBox::from_id_salt("controller-kind").selected_text(["CC lane", "Pitch bend lane", "Pressure lane", "Bank/program lane"][controls.lane]).show_ui(ui, |ui| {
                for (i, label) in ["CC lane", "Pitch bend lane", "Pressure lane", "Bank/program lane"].iter().enumerate() { ui.selectable_value(&mut controls.lane, i, *label); }
            });
            let mut channel = f64::from(controls.channel + 1); if number(ui, "Controller channel", &mut channel, 1.0, 16.0) { controls.channel = channel.round() as u8 - 1; }
            if controls.lane == 0 { let mut cc = f64::from(controls.cc); if number(ui, "CC number", &mut cc, 0.0, 119.0) { controls.cc = cc.round() as u8; } }
            ui.label(controls.kind().name());
        });
        if before != (controls.lane, controls.channel, controls.cc) {
            if before.0 != controls.lane { controls.value = if controls.lane == 1 { 8192 } else { 0 }; }
            controls.refresh();
        }
        ui.horizontal_wrapped(|ui| {
            number(ui, "Controller beat", &mut controls.beat, 0.0, 262144.0);
            let mut value = f64::from(controls.value); if number(ui, "Controller value", &mut value, 0.0, f64::from(controls.maximum())) { controls.value = value.round() as u16; }
            if controls.lane == 3 {
                for (index, label) in ["Bank MSB", "Bank LSB"].into_iter().enumerate() {
                    let mut value = f64::from(controls.bank[index]); if number(ui, label, &mut value, 0.0, 127.0) { controls.bank[index] = value.round() as u8; }
                }
            }
        });
        ui.horizontal_wrapped(|ui| {
            if button(ui, "Insert controller point").clicked() { outcome = controls.insert(&draft.notes).map(|_| true); }
            if button(ui, "Set selected controller point").clicked() { outcome = controls.replace(&draft.notes).map(|_| true); }
            if button(ui, "Delete controller point").clicked() { outcome = controls.delete().map(|_| true); }
            ui.label("Device label");
            let text = ui.add(egui::TextEdit::singleline(&mut controls.name).char_limit(256));
            text.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, "Controller device label"));
            if button(ui, "Save device label").clicked() { outcome = controls.label().map(|_| true); }
        });
        ui.label(format!("{} points · source PPQN {} · pitch bend center 8192. Bank/program insertion sends CC 0, CC 32, then Program Change. Imported messages and metadata keep their wire values.", controls.points.len(), controls.ppqn));
        let (response, painter) = ui.allocate_painter(Vec2::new(ui.available_width().max(1.0), 100.0), egui::Sense::click());
        let rect = response.rect;
        painter.rect_filled(rect, 0.0, theme.bg_dark);
        let visible_end = draft.view_beat + f64::from(rect.width() / draft.beat_pixels);
        let start = controls.points.partition_point(|i| controls.messages[*i].tick as f64 / f64::from(controls.ppqn) < draft.view_beat);
        let end = controls.points.partition_point(|i| controls.messages[*i].tick as f64 / f64::from(controls.ppqn) <= visible_end);
        let mut last: Option<Pos2> = None;
        let stride = (end - start.saturating_sub(1)).div_ceil(4096).max(1);
        for index in (start.saturating_sub(1)..end).step_by(stride) {
            let message = controls.messages[controls.points[index]];
            let beat = message.tick as f64 / f64::from(controls.ppqn);
            let point = Pos2::new((rect.left() + (beat - draft.view_beat) as f32 * draft.beat_pixels).clamp(rect.left(), rect.right()), rect.bottom() - f32::from(controls.value_of(message)) / f32::from(controls.maximum()) * rect.height());
            if let Some(previous) = last { painter.line_segment([previous, Pos2::new(point.x, previous.y)], egui::Stroke::new(1.0_f32, theme.cyan)); }
            painter.circle_filled(point, 2.5, if controls.selected == Some(message) { theme.yellow } else { theme.cyan });
            last = Some(point);
        }
        if let Some(previous) = last { painter.line_segment([previous, Pos2::new(rect.right(), previous.y)], egui::Stroke::new(1.0_f32, theme.cyan)); }
        if stride > 1 { ui.label("Dense lane overview is sampled; the point list retains every event."); }
        if response.clicked() {
            if let Some(position) = response.interact_pointer_pos() {
                let beat = draft.view_beat + f64::from((position.x - rect.left()) / draft.beat_pixels);
                if let Some(index) = controls.points[start..end].iter().copied().min_by(|a, b| {
                    (controls.messages[*a].tick as f64 / f64::from(controls.ppqn) - beat).abs().total_cmp(&(controls.messages[*b].tick as f64 / f64::from(controls.ppqn) - beat).abs())
                }).filter(|i| (controls.messages[*i].tick as f64 / f64::from(controls.ppqn) - beat).abs() * f64::from(draft.beat_pixels) <= 8.0) {
                    controls.select(index);
                } else {
                    let grid = GRIDS[draft.grid].1;
                    controls.beat = if grid > 0.0 { (beat / grid).round() * grid } else { beat };
                    controls.value = ((rect.bottom() - position.y) / rect.height() * f32::from(controls.maximum())).round().clamp(0.0, f32::from(controls.maximum())) as u16;
                    outcome = controls.insert(&draft.notes).map(|_| true);
                }
            }
        }
        let mut select = None;
        let rows = egui::ScrollArea::vertical().id_salt("controller-points").max_height(110.0).show_rows(ui, 22.0, controls.points.len(), |ui, range| {
            for row in range {
                let index = controls.points[row]; let m = controls.messages[index];
                let label = format!("Controller point {}: {:.9} beats · value {} · tick {} · order {}", row + 1, m.tick as f64 / f64::from(controls.ppqn), controls.value_of(m), m.tick, m.order);
                let response = ui.selectable_label(controls.selected == Some(m), &label);
                accessibility::button(ui, &response, &label, Some(controls.selected == Some(m)));
                if response.clicked() { select = Some(index); }
            }
        });
        accessibility::scrollbars(ui, "Controller point list", &rows);
        if let Some(index) = select { controls.select(index); }
        draft.dirty |= controls.dirty;
    });
    help::annotate(ui, &shown.header_response, HelpControl::MidiControllers);
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;
    fn document() -> Arc<Document> {
        Arc::new(Document {
            track_identity: None,
            scene_identity: None,
            track: 2,
            scene: 7,
            epoch: 1,
            kind: crate::engine::ClipKind::Empty,
            name: String::new(),
            bars: 4.0,
            region: None,
            notes: vec![],
            lanes: None,
        })
    }
    #[test]
    fn controller_points_keep_source_ticks_widths_channels_metadata_and_patch_order() {
        let imported = Lanes::new(
            32767,
            8589672448,
            vec![Message {
                tick: 8589672447,
                order: 40,
                bytes: [0xbf, 74, 19],
                length: 3,
            }],
            vec![],
        )
        .unwrap();
        let mut controls = Controls::new(Some(&imported));
        controls.channel = 15;
        controls.cc = 74;
        controls.refresh();
        controls.select(0);
        assert_eq!(controls.tick().unwrap(), 8589672447);
        controls.value = 123;
        controls.replace(&[]).unwrap();
        assert_eq!(controls.messages[0].tick, 8589672447);
        controls.lane = 1;
        controls.value = 16383;
        controls.beat = 1.25;
        controls.insert(&[]).unwrap();
        assert_eq!(controls.selected.unwrap().bytes, [0xef, 127, 127]);
        controls.lane = 2;
        controls.value = 99;
        controls.insert(&[]).unwrap();
        assert_eq!(controls.selected.unwrap().length, 2);
        controls.lane = 3;
        controls.bank = [3, 12];
        controls.value = 57;
        controls.beat = 4.5;
        controls.insert(&[]).unwrap();
        controls.name = "My stage piano".into();
        controls.label().unwrap();
        let lanes = controls
            .prepared(&[], 262143.0, &AtomicBool::new(false))
            .unwrap()
            .unwrap();
        let patch: Vec<_> = lanes
            .messages
            .iter()
            .filter(|m| m.tick == (4.5 * 32767.0) as u64 + 1)
            .map(|m| &m.bytes[..usize::from(m.length)])
            .collect();
        assert_eq!(
            patch,
            vec![&[0xbf, 0, 3][..], &[0xbf, 32, 12][..], &[0xcf, 57][..]]
        );
        assert_eq!(lanes.labels[0].name, "My stage piano");
        let mut draft = Draft::new(document());
        draft.controls = controls;
        let (_, _, next) = Request::with_lanes(
            draft.baseline,
            "Controller test".into(),
            Region::full(65536.0),
            vec![],
            Some(lanes.clone()),
        )
        .unwrap();
        assert_eq!(next.lanes, Some(lanes));
    }
    #[test]
    fn invalid_controller_edits_and_cancelled_preparation_preserve_source() {
        let mut controls = Controls::new(None);
        controls.insert(&[]).unwrap();
        let before = controls.messages.clone();
        controls.value = 128;
        assert!(controls.replace(&[]).is_err());
        assert_eq!(controls.messages, before);
        controls.beat = f64::NAN;
        assert!(controls.insert(&[]).is_err());
        assert_eq!(controls.messages, before);
        assert!(controls
            .prepared(&[], 16.0, &AtomicBool::new(true))
            .is_err());
        controls.selected = Some(Message {
            order: 999,
            ..before[0]
        });
        assert!(controls.delete().is_err());
        assert_eq!(controls.messages, before);
    }
    #[test]
    fn native_controller_editor_apply_undo_redo_and_cancel_keep_exact_lanes() {
        let mut gui = super::super::tests::Gui::new();
        gui.app.open_piano_roll();
        gui.settle();
        gui.click("MIDI controller lanes");
        gui.number("Controller channel", 16.0);
        gui.number("CC number", 74.0);
        gui.number("Controller beat", 2.125);
        gui.number("Controller value", 99.0);
        gui.click("MIDI piano roll: Insert controller point");
        gui.app.piano_roll.draft.as_mut().unwrap().controls.name = "Filter cutoff".into();
        gui.click("MIDI piano roll: Save device label");
        gui.apply();
        let applied = gui.rt.tracks[2].clips[7].lanes.clone().unwrap();
        assert_eq!(applied.messages[0].bytes, [0xbf, 74, 99]);
        assert_eq!(applied.messages[0].tick, 2040);
        assert_eq!(applied.labels[0].name, "Filter cutoff");
        gui.app.engine.send(Command::Undo).unwrap();
        gui.settle();
        assert!(gui.rt.tracks[2].clips[7].lanes.is_none());
        gui.app.engine.send(Command::Redo).unwrap();
        gui.settle();
        assert_eq!(gui.rt.tracks[2].clips[7].lanes, Some(applied.clone()));
        gui.number("Controller value", 12.0);
        gui.click("MIDI piano roll: Insert controller point");
        gui.click("MIDI piano roll: Cancel / close MIDI editor");
        gui.click("MIDI piano roll: Discard MIDI draft");
        gui.settle();
        assert_eq!(gui.rt.tracks[2].clips[7].lanes, Some(applied));
    }
}
