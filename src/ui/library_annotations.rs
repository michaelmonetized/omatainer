use super::*;
use crate::library::{
    annotations::{Annotations, Patch, Rule},
    crates::Edit,
    TrackId,
};
use library_metadata::CollectionAction;

#[derive(Default)]
pub(super) struct Panel {
    pub open: bool,
    ids: Vec<TrackId>,
    expected: u64,
    fields: Annotations,
    change: [bool; 5],
    color: String,
    tags: String,
    reviewed: Option<Patch>,
    rule: Rule,
    rule_crate: Option<crate::library::crates::CrateId>,
    rule_color: String,
    message: String,
}
impl Panel {
    /// Build exactly the fields selected in the editor.
    /// Takes this draft; returns a validated sidecar patch without any embedded tag operation.
    fn patch(&self) -> Result<Patch, String> {
        let patch = Patch {
            rating: self.change[0].then_some(self.fields.rating),
            color: if self.change[1] {
                Some(parse_color(&self.color)?)
            } else {
                None
            },
            group: self.change[2].then(|| self.fields.group.clone()),
            tags: self.change[3].then(|| {
                self.tags
                    .lines()
                    .filter(|line| !line.is_empty())
                    .map(str::to_owned)
                    .collect()
            }),
            notes: self.change[4].then(|| self.fields.notes.clone()),
        };
        if patch.is_empty() {
            return Err("Select at least one field to change".into());
        }
        patch.apply(&Annotations::default())?;
        Ok(patch)
    }
}

/// Parse a user color or an explicit clear.
/// Takes empty text or #RRGGBB; returns no color, RGB bytes or a visible error.
fn parse_color(value: &str) -> Result<Option<[u8; 3]>, String> {
    if value.is_empty() {
        return Ok(None);
    }
    let hex = value
        .strip_prefix('#')
        .ok_or("Use #RRGGBB or leave empty to clear")?;
    if hex.len() != 6 {
        return Err("Use #RRGGBB or leave empty to clear".into());
    }
    let number = u32::from_str_radix(hex, 16).map_err(|_| "Use #RRGGBB or leave empty to clear")?;
    Ok(Some([
        (number >> 16) as u8,
        (number >> 8) as u8,
        number as u8,
    ]))
}

impl App {
    /// Freeze the selected stable IDs before reviewing a batch.
    /// Takes selected/all-filtered choice; fills a draft without reading or changing audio files.
    fn capture_annotations(&mut self, filtered: bool) {
        self.refresh_library_view();
        self.library_annotations.ids.clear();
        self.library_annotations.reviewed = None;
        let rows: Vec<_> = if filtered {
            self.library_view.indices.iter().copied().collect()
        } else {
            self.library_view
                .indices
                .get(self.lib_sel)
                .copied()
                .into_iter()
                .collect()
        };
        let targets: Vec<_> = rows
            .iter()
            .filter_map(|&row| {
                self.library_metadata
                    .catalog
                    .track(&self.library[row].source)
            })
            .collect();
        if targets.len() != rows.len() || targets.is_empty() {
            self.library_annotations.message = "Every selected row needs a saved stable track ID; finish the library scan/save first".into();
            return;
        }
        let panel = &mut self.library_annotations;
        panel.ids = targets.iter().map(|track| track.id.clone()).collect();
        panel.expected = self.library_metadata.catalog.crates.revision();
        panel.fields = targets[0].annotations.clone();
        panel.color = panel
            .fields
            .color
            .map(|c| format!("#{:02X}{:02X}{:02X}", c[0], c[1], c[2]))
            .unwrap_or_default();
        panel.tags = panel.fields.tags.join("\n");
        panel.change = [false; 5];
        panel.reviewed = None;
        panel.message = format!("Captured {} stable track IDs. Values shown come from the first captured track. Unselected fields keep each track's own values.", panel.ids.len());
    }

    /// Edit durable annotations and the same predicates used by smart crates.
    /// Takes the frame context; submits reviewed changes through the existing catalog owner.
    pub(super) fn library_annotations_ui(&mut self, ctx: &egui::Context) {
        if !self.library_annotations.open {
            return;
        }
        keyboard::block_for_dialog(ctx);
        let mut open = true;
        let mut capture = None;
        let mut action = None;
        let available = self.library_metadata.ready()
            && self.library_crates.pending.is_none()
            && !self.engine.cmd.performance().protected()
            && !self.project.committing();
        let selected_crate = self.library_crates.selected.clone();
        let panel = &mut self.library_annotations;
        if panel.rule_crate != selected_crate {
            panel.rule = selected_crate
                .as_ref()
                .and_then(|id| self.library_metadata.catalog.crates.node(id))
                .and_then(|node| node.annotation_rule.clone())
                .unwrap_or_default();
            panel.rule_color = panel
                .rule
                .color
                .map(|c| format!("#{:02X}{:02X}{:02X}", c[0], c[1], c[2]))
                .unwrap_or_default();
            panel.rule_crate = selected_crate.clone();
        }
        egui::Window::new(tr!("Track ratings, colors and annotations")).id(egui::Id::new("track-annotations-window"))
            .open(&mut open).default_width(660.0).vscroll(true).max_height(self.theme.window_height(ctx)).show(ctx, |ui| {
                ui.label(tr!("These fields live in library metadata. Annotation edits never write embedded audio tags or change loaded sound."));
                ui.horizontal(|ui| {
                    if ui.add_enabled(available,egui::Button::new(tr!("Capture selected annotations"))).help(ui,HelpControl::TrackAnnotations).clicked() { capture = Some(false); }
                    if ui.add_enabled(available,egui::Button::new(tr!("Capture filtered annotation batch"))).help(ui,HelpControl::TrackAnnotations).clicked() { capture = Some(true); }
                });
                ui.label(&panel.message);
                let previous = (panel.fields.clone(), panel.change, panel.color.clone(), panel.tags.clone());
                ui.add_enabled_ui(available, |ui| {
                    ui.horizontal(|ui| { ui.checkbox(&mut panel.change[0],tr!("Change rating")); ui.add(egui::Slider::new(&mut panel.fields.rating,0..=5).text("Rating (0 = unrated)")); });
                    annotation_text(ui,"Color (#RRGGBB; empty clears)",&mut panel.color,&mut panel.change[1],false);
                    annotation_text(ui,"Performance group",&mut panel.fields.group,&mut panel.change[2],false);
                    annotation_text(ui,"Performance tags (one per line)",&mut panel.tags,&mut panel.change[3],true);
                    annotation_text(ui,"Performance notes",&mut panel.fields.notes,&mut panel.change[4],true);
                });
                if previous != (panel.fields.clone(), panel.change, panel.color.clone(), panel.tags.clone()) { panel.reviewed = None; }
                if ui.add_enabled(available && !panel.ids.is_empty(),egui::Button::new(tr!("Review annotation changes"))).help(ui,HelpControl::TrackAnnotations).clicked() {
                    match panel.patch() { Ok(patch) => { panel.reviewed = Some(patch); panel.message = format!("Reviewed {} captured tracks. Apply replaces only selected fields, including explicit clears.",panel.ids.len()); }, Err(error) => panel.message = error }
                }
                if ui.add_enabled(available && panel.reviewed.is_some(),egui::Button::new(tr!("Apply reviewed annotations"))).help(ui,HelpControl::TrackAnnotations).clicked() {
                    action = Some((panel.expected,CollectionAction::Annotate { ids: panel.ids.clone(), patch: panel.reviewed.take().unwrap() }));
                }
                if let Some((token,_)) = &self.library_crates.pending {
                    if ui.button(tr!("Cancel pending annotation or crate edit")).clicked() { token.cancel(); }
                }
                ui.label(&self.library_crates.message);
                ui.separator(); ui.heading(tr!("Automatic annotation crate rule"));
                ui.label(tr!("Create and select an empty named crate first. All configured conditions are combined. Its membership follows current annotations automatically."));
                ui.add(egui::Slider::new(&mut panel.rule.minimum_rating,0..=5).text("Minimum crate rating"));
                for (name,value) in [("Rule color (#RRGGBB; empty ignores)",&mut panel.rule_color),("Rule group contains",&mut panel.rule.group),("Rule tag equals",&mut panel.rule.tag),("Rule notes contain",&mut panel.rule.notes)] {
                    let label = ui.label(name); let response = ui.add(egui::TextEdit::singleline(value).char_limit(4096)).labelled_by(label.id);
                    ui.ctx().accesskit_node_builder(response.id, |node|node.set_label(name));
                }
                if ui.add_enabled(available && selected_crate.is_some(),egui::Button::new(tr!("Save annotation rule on selected crate"))).help(ui,HelpControl::TrackAnnotationRule).clicked() {
                    match parse_color(&panel.rule_color).and_then(|color| { let mut rule = panel.rule.clone(); rule.color = color; rule.validate()?; Ok(rule) }) {
                        Ok(rule) => action = Some((self.library_metadata.catalog.crates.revision(),CollectionAction::Edit(Edit::SetAnnotationRule { id:selected_crate.clone().unwrap(),rule:Some(rule) }))),
                        Err(error) => panel.message = error,
                    }
                }
                if ui.add_enabled(available && selected_crate.is_some(),egui::Button::new(tr!("Remove annotation rule"))).help(ui,HelpControl::TrackAnnotationRule).clicked() {
                    action = Some((self.library_metadata.catalog.crates.revision(),CollectionAction::Edit(Edit::SetAnnotationRule { id:selected_crate.clone().unwrap(),rule:None })));
                }
                ui.label(tr!("Search examples: rating>=4 tag:clean color:#FF6600 group:peak note:request. Ordinary text also searches tags, groups and notes. Metadata exports include all annotations and saved rules."));
                ui.label(tr!("Closing this editor preserves any already admitted transaction; use Cancel before its publication claim to stop it."));
            });
        self.library_annotations.open = open;
        if let Some(filtered) = capture {
            self.capture_annotations(filtered);
        }
        if let Some((revision, action)) = action {
            self.submit_crate_edit(revision, action);
        }
    }
}

/// Edit one annotation with an explicit field-selection checkbox.
/// Takes label, draft text, changed flag and line mode; updates only the draft and exposes native labels.
fn annotation_text(
    ui: &mut Ui,
    label: &str,
    value: &mut String,
    changed: &mut bool,
    multiline: bool,
) {
    ui.checkbox(changed, crate::localization::format("Change {label}", &[format!("{}", label)]));
    let name = label;
    let label = ui.label(name);
    let edit = if multiline {
        egui::TextEdit::multiline(value).desired_rows(3)
    } else {
        egui::TextEdit::singleline(value)
    };
    let response = ui.add(edit.char_limit(4096)).labelled_by(label.id);
    ui.ctx()
        .accesskit_node_builder(response.id, |node| node.set_label(name));
}

#[cfg(test)]
pub(super) mod tests;
