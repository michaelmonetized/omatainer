//! Cue drafts are GUI-owned. Applied values and their identity always come
//! from renderer snapshots; library durability comes from its worker receipt.
use super::*;
use crate::engine::cue_metadata::{Name, Style};
mod replacement;

#[derive(Default)]
pub(super) struct Cues {
    pub editor: Option<Editor>,
    pub project_receipts: [Option<Receipt>; DECKS],
    relocation: Option<Relocation>,
}
pub(super) struct Editor {
    deck: usize,
    receipt: Receipt,
    drafts: [Draft; 8],
    message: String,
}
struct Draft {
    name: String,
    color: String,
}
impl Draft {
    fn from_style(style: Style) -> Self {
        Self {
            name: style.name.as_str().into(),
            color: style
                .color
                .map(|[r, g, b]| format!("#{r:02X}{g:02X}{b:02X}"))
                .unwrap_or_default(),
        }
    }
    fn style(&self) -> Result<Style, &'static str> {
        let name = Name::new(&self.name)?;
        let text = self.color.trim();
        let color = if text.is_empty() {
            None
        } else {
            let text = text.strip_prefix('#').unwrap_or(text);
            if text.len() != 6 || !text.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err("Use six hexadecimal color digits, or leave empty for the theme color");
            }
            Some(std::array::from_fn(|i| {
                u8::from_str_radix(&text[i * 2..i * 2 + 2], 16).unwrap()
            }))
        };
        Ok(Style { name, color })
    }
}
struct Relocation {
    search: replacement::Search,
    request: crate::library::Relocate,
    path: String,
    title: String,
    message: String,
    pending: bool,
    saved: bool,
}

pub(super) fn color(theme: &Theme, style: Style, slot: usize) -> Color32 {
    style
        .color
        .map(|[r, g, b]| Color32::from_rgb(r, g, b))
        .unwrap_or_else(|| theme.track_color(slot))
}
pub(super) fn short_name(name: &str, count: usize) -> String {
    let mut chars = name.chars();
    let mut result: String = chars.by_ref().take(count).collect();
    if chars.next().is_some() {
        result.push('…');
    }
    result
}
pub(super) fn label(slot: usize, style: Style) -> String {
    if style.name.as_str().is_empty() {
        format!("Hot cue {}", slot + 1)
    } else {
        format!("Hot cue {}: {}", slot + 1, style.name.as_str())
    }
}
pub(super) fn description(snap: &crate::engine::DeckSnap, slot: usize) -> String {
    let position = snap.hotcue_positions[slot]
        .filter(|_| snap.source_sample_rate > 0)
        .map(|p| format!("{:.3} seconds", p / snap.source_sample_rate as f64))
        .unwrap_or_else(|| "empty".into());
    let color = snap.cue_styles[slot]
        .color
        .map(|[r, g, b]| format!("#{r:02X}{g:02X}{b:02X}"))
        .unwrap_or_else(|| "theme color".into());
    let linked = snap.saved_loops.cue_loops[slot].map_or_else(String::new, |id| format!(", saved loop {id}; cue-only override available"));
    format!(
        "{}, {position}, {color}{linked}",
        label(slot, snap.cue_styles[slot])
    )
}
impl App {
    pub(super) fn open_cue_editor(&mut self, deck: usize) {
        let Some(snap) = self.snap.decks.get(deck) else {
            return;
        };
        let Some(receipt) = self.cue_receipt(snap.receipt_key) else {
            self.status = "Wait for the loaded track identity before editing cue names".into();
            return;
        };
        self.cue_editor.editor = Some(Editor {
            deck,
            receipt,
            drafts: std::array::from_fn(|i| Draft::from_style(snap.cue_styles[i])),
            message: String::new(),
        });
    }
    pub(super) fn open_cue_relocation(&mut self) {
        let Some(item) = self.selected_library_item().cloned() else {
            return;
        };
        let Some(track) = self.library_metadata.catalog.track(&item.source) else {
            self.status = "Wait for this track to be saved in the DJ library".into();
            return;
        };
        let Some(fingerprint) = item
            .fingerprint
            .filter(|_| matches!(item.source, LibSource::File(_) | LibSource::Removable {..}))
        else {
            self.status = "Relocation requires a previously inspected local file track".into();
            return;
        };
        self.cue_editor.relocation = Some(Relocation {
            search: replacement::Search::default(),
            request: crate::library::Relocate {
                id: track.id.clone(),
                source: item.source.clone(),
                fingerprint,
                destination: PathBuf::new(),
            },
            path: String::new(),
            title: item.title,
            pending: false,
            saved: false,
            message: if track.versions[track.current].content_hash.is_some() {
                "Original track content is verified.".into()
            } else {
                "The original file must still be available for content verification.".into()
            },
        });
    }
    pub(super) fn cue_editor_ui(&mut self, ctx: &egui::Context) {
        if let Some(mut editor) = self.cue_editor.editor.take() {
            let snap = self
                .snap
                .decks
                .get(editor.deck)
                .cloned()
                .unwrap_or_default();
            if snap.receipt_key != editor.receipt.snapshot_key()
                || editor.receipt.state() != crate::engine::load_receipt::State::Current
            {
                self.status = "Cue editor closed because the loaded track changed".into();
            } else {
                keyboard::block_for_dialog(ctx);
                let mut open = true;
                let mut close = false;
                egui::Window::new({ let __omatainer_args = (&((b'A' + editor.deck as u8) as char),); crate::localization::format("Deck {} cues", &[format!("{}", __omatainer_args.0)]) }).open(&mut open).resizable(true).vscroll(true).show(ctx, |ui| {
                    if self.project.committing() || !self.project.dialog_is_closed() { ui.disable(); }
                    ui.heading(&snap.title);
                    ui.label(self.cue_storage_status(&editor.receipt));
                    ui.label(tr!("Cue names: up to 64 UTF-8 bytes. Color: #RRGGBB; empty follows the theme."));
                    ui.label(tr!("Main cue is separate. Empty hot-cue slots must be set before naming them."));
                    for i in 0..8 {
                        ui.push_id(i, |ui| {
                            ui.horizontal(|ui| {
                                let (rect, _) = ui.allocate_exact_size(Vec2::splat(12.0), Sense::hover());
                                ui.painter().rect_filled(rect, 2.0, color(&self.theme, snap.cue_styles[i], i));
                                ui.painter().rect_stroke(rect, 2.0, st(1.0, self.theme.fg_dim), egui::StrokeKind::Inside);
                                ui.label(description(&snap, i));
                            });
                            if let Some(id) = snap.saved_loops.cue_loops[i] {
                                ui.horizontal(|ui| {
                                    ui.label(format!("Cue {} triggers saved loop {id}", i + 1));
                                    let response = ui.small_button(format!("Jump {} (cue only)", i + 1));
                                    accessibility::button(ui, &response, &format!("Cue {} only", i + 1), None);
                                    if response.clicked() { self.send(Command::DeckControl { source: 0, deck: editor.deck as u8, control: crate::engine::deck_controls::Control::CueOnly { media_key: snap.media_key, pad: i as u8 } }); }
                                });
                            }
                            ui.horizontal(|ui| {
                                let set = snap.hotcues[i];
                                let action = ui.button(if set { { let __omatainer_args = (&(i+1),); crate::localization::format("Jump {}", &[format!("{}", __omatainer_args.0)]) } } else { { let __omatainer_args = (&(i+1),); crate::localization::format("Set {}", &[format!("{}", __omatainer_args.0)]) } });
                                help::annotate(ui, &action, HelpControl::HotCue);
                                if action.clicked() { self.send(Command::DeckCuePoint { deck: editor.deck as u8, pad: i as u8, del: false, receipt: editor.receipt.clone() }); }
                                ui.add_enabled_ui(set, |ui| {
                                    let name = ui.add(egui::TextEdit::singleline(&mut editor.drafts[i].name).desired_width(180.0).char_limit(64));
                                    name.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, format!("Cue {} name", i+1)));
                                    help::annotate(ui, &name, HelpControl::CueName);
                                    let rgb = ui.add(egui::TextEdit::singleline(&mut editor.drafts[i].color).desired_width(80.0).char_limit(7).hint_text(tr!("theme")));
                                    rgb.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, format!("Cue {} color", i+1)));
                                    help::annotate(ui, &rgb, HelpControl::CueColor);
                                    let apply = ui.button({ let __omatainer_args = (&(i+1),); crate::localization::format("Apply {}", &[format!("{}", __omatainer_args.0)]) });
                                    help::annotate(ui, &apply, HelpControl::CueApply);
                                    if apply.clicked() {
                                        match editor.drafts[i].style() {
                                            Err(error) => editor.message = error.into(),
                                            Ok(style) => editor.message = if self.submit(Command::DeckCueStyle { deck: editor.deck as u8, pad: i as u8, style, receipt: editor.receipt.clone() }) {
                                                format!("Cue {} edit queued. Applied values appear in the cue row and pads.", i+1)
                                            } else { "Cue edit was not accepted; your draft is still here.".into() },
                                        }
                                    }
                                    let delete = ui.button({ let __omatainer_args = (&(i+1),); crate::localization::format("Delete {}", &[format!("{}", __omatainer_args.0)]) });
                                    help::annotate(ui, &delete, HelpControl::HotCue);
                                    if delete.clicked() { self.send(Command::DeckCuePoint { deck: editor.deck as u8, pad: i as u8, del: true, receipt: editor.receipt.clone() }); }
                                });
                            });
                        });
                    }
                    let reset = ui.button(tr!("Reload cue values"));
                    help::annotate(ui, &reset, HelpControl::CueReload);
                    if reset.clicked() { editor.drafts = std::array::from_fn(|i| Draft::from_style(snap.cue_styles[i])); editor.message.clear(); }
                    if ui.button(tr!("Close cue editor")).help(ui, HelpControl::CueClose).clicked() { close = true; }
                    if !editor.message.is_empty() { ui.label(&editor.message); }
                });
                if open && !close {
                    self.cue_editor.editor = Some(editor);
                }
            }
        }
        if let Some(mut relocation) = self.cue_editor.relocation.take() {
            if relocation.pending {
                if let Some(outcome) = self.library_metadata.relocation_result(&relocation.request)
                {
                    relocation.pending = false;
                    relocation.saved = outcome.is_ok();
                    relocation.message = match outcome {
                        Ok(()) => format!(
                            "Relocation saved: {}. Track identity and cues were preserved.",
                            relocation.request.destination.display()
                        ),
                        Err(error) => error.clone(),
                    };
                }
            }
            keyboard::block_for_dialog(ctx);
            let mut open = true;
            egui::Window::new(tr!("Relocate library track")).id(egui::Id::new("Relocate library track")).open(&mut open).show(ctx, |ui| {
                if self.project.committing() || !self.project.dialog_is_closed() { ui.disable(); }
                ui.heading(&relocation.title);
                ui.label(tr!("Choose the new location after moving or copying the same file. Verification compares every byte before preserving its track identity and cues. This does not move or delete files."));
                ui.label(&relocation.message);
                let path = ui.add_enabled(!relocation.pending && !relocation.saved, egui::TextEdit::singleline(&mut relocation.path).desired_width(420.0).hint_text(tr!("/new/location/track.wav")));
                path.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, "Relocated track path"));
                help::annotate(ui, &path, HelpControl::CueRelocatePath);
                let submit = ui.push_id((&relocation.request.id, &relocation.path), |ui| ui.add_enabled(!relocation.pending && !relocation.saved, egui::Button::new(tr!("Verify and relocate track")))).inner;
                help::annotate(ui, &submit, HelpControl::CueRelocateApply);
                if submit.clicked() {
                    relocation.search.discard();
                    relocation.request.destination = PathBuf::from(relocation.path.trim());
                    relocation.message = if self.library_metadata.relocate(relocation.request.clone()) {
                        relocation.pending = true;
                        "Verifying relocation on the library worker…".into()
                    } else { format!("Relocation was not queued. {}", self.library_metadata.label()) };
                }
                replacement::panel(self, &mut relocation, ui);
                ui.label(self.library_metadata.label());
            });
            if open {
                self.cue_editor.relocation = Some(relocation);
            }
        }
    }
}

#[cfg(test)]
mod tests;
