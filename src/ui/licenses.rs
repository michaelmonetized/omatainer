use super::*;
use crate::licenses::Catalog;
use std::sync::mpsc::{self, Receiver};

#[derive(Default)]
pub(super) struct Licenses {
    pub open: bool,
    pending: Option<Receiver<Result<Catalog, String>>>,
    catalog: Option<Catalog>,
    error: Option<String>,
    query: String,
    selected: usize,
}
impl Licenses {
    fn start(&mut self, manifest: &'static str, notices: &'static str) {
        let (send, receive) = mpsc::sync_channel(1);
        match std::thread::Builder::new()
            .name("omatainer-licenses".into())
            .spawn(move || {
                let _ = send.send(Catalog::parse(manifest, notices));
            }) {
            Ok(_) => self.pending = Some(receive),
            Err(error) => self.error = Some(format!("Could not open license records: {error}")),
        }
    }
    fn poll(&mut self) {
        let Some(pending) = &self.pending else { return };
        match pending.try_recv() {
            Ok(result) => {
                self.pending = None;
                match result {
                    Ok(catalog) => {
                        self.catalog = Some(catalog);
                        self.error = None;
                    }
                    Err(error) => {
                        self.error = Some(format!("License records unavailable: {error}"))
                    }
                }
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                self.pending = None;
                self.error = Some("License record worker disconnected".into());
            }
            Err(mpsc::TryRecvError::Empty) => {}
        }
    }
}
impl App {
    pub(super) fn licenses_panel(&mut self, ctx: &egui::Context) {
        self.licenses.poll();
        if !self.licenses.open {
            return;
        }
        keyboard::block_for_dialog(ctx);
        if self.licenses.catalog.is_none()
            && self.licenses.pending.is_none()
            && self.licenses.error.is_none()
        {
            self.licenses
                .start(crate::licenses::MANIFEST, crate::licenses::NOTICES);
        }
        let mut open = true;
        let mut close_requested = ctx.input(|input| input.key_pressed(egui::Key::Escape));
        egui::Window::new("Content & licenses").id(egui::Id::new("licenses-window"))
            .open(&mut open).default_size(egui::vec2(840.0, 580.0)).show(ctx, |ui| {
            let response = ui.button("Close license viewer");
            help::annotate(ui, &response, help::Control::ClosePanel);
            if response.clicked() { close_requested = true; }
            if let Some(error) = &self.licenses.error {
                ui.label(error);
                ui.label("No license permission can be inferred from an unavailable record. Reinstall a verified package.");
            }
            let Some(catalog) = &self.licenses.catalog else {
                if self.licenses.pending.is_some() {
                    ui.label("Opening offline license records…");
                    ctx.request_repaint_after(std::time::Duration::from_millis(20));
                }
                return;
            };
            ui.label(format!("Omatainer {} · {} · {} indexed entries", catalog.manifest.application, catalog.manifest.target, catalog.manifest.entries.len()));
            ui.label(&catalog.manifest.scope);
            ui.label(format!("{} Cargo components · {} integration files · rustc {}",
                catalog.manifest.cargo.as_array().map_or(0, Vec::len), catalog.manifest.package.len(),
                catalog.manifest.toolchain["release"].as_str().unwrap_or("recorded version")));
            let external = egui::CollapsingHeader::new("External media and content not shipped").show(ui, |ui| {
                for text in catalog.manifest.external.iter().chain(&catalog.manifest.absent) { ui.label(text); }
            });
            help::annotate(ui, &external.header_response, help::Control::License);
            let label = ui.label("Find content");
            let search = ui.add(egui::TextEdit::singleline(&mut self.licenses.query).hint_text("Asset ID, font or component")).labelled_by(label.id);
            help::annotate(ui, &search, help::Control::LicenseSearch);
            let query = self.licenses.query.to_lowercase();
            let matches: Vec<_> = catalog.manifest.entries.iter().enumerate().filter(|(_, entry)| {
                entry.id.to_lowercase().contains(&query) || entry.name.to_lowercase().contains(&query)
            }).map(|(index, _)| index).collect();
            ui.label(format!("{} matches", matches.len()));
            egui::ScrollArea::vertical().id_salt("license-index").max_height(130.0).show_rows(ui, 22.0, matches.len(), |ui, rows| {
                for row in rows {
                    let index = matches[row];
                    let entry = &catalog.manifest.entries[index];
                    let response = ui.selectable_label(self.licenses.selected == index, format!("{} — {}", entry.id, entry.name));
                    help::annotate(ui, &response, help::Control::LicenseEntry);
                    if response.clicked() {
                        self.licenses.selected = index;
                    }
                }
            });
            ui.separator();
            let entry = &catalog.manifest.entries[self.licenses.selected];
            egui::ScrollArea::vertical().id_salt(("license-detail", self.licenses.selected)).max_height(300.0).show(ui, |ui| {
                ui.heading(&entry.name);
                ui.monospace(&entry.id);
                ui.label(format!("{} · {}", entry.category, entry.delivery));
                ui.label(format!("License: {}", entry.license));
                ui.label(&entry.commercial_use);
                ui.label(&entry.redistribution);
                if !entry.members.is_empty() { ui.label(format!("Content identities: {}", entry.members.join(", "))); }
                for record in &entry.sources {
                    if record.location.starts_with("https://") {
                        let response = ui.hyperlink_to("Source record (external browser)", &record.location);
                        help::annotate(ui, &response, help::Control::LicenseSource);
                    }
                    else { ui.monospace(&record.location); }
                    let hash = record.sha256.as_ref().or_else(|| catalog.manifest.source_files.get(record.location.split('#').next().unwrap_or("")));
                    if let Some(hash) = hash { ui.monospace(format!("SHA-256: {hash}")); }
                }
                for (index, record) in entry.notices.iter().enumerate() {
                    let notice = egui::CollapsingHeader::new(format!("Full notice {}", index + 1)).id_salt((&entry.id, index)).show(ui, |ui| {
                        ui.label(&record.location);
                        let id = record.sha256.as_ref().unwrap();
                        let text = &catalog.notices[id];
                        let lines = &catalog.notice_lines[id];
                        let height = ui.text_style_height(&egui::TextStyle::Monospace);
                        egui::ScrollArea::both().id_salt((&entry.id, index, "notice-lines"))
                            .max_height(180.0).show_rows(ui, height, lines.len(), |ui, visible| {
                                for line in visible {
                                    ui.add(egui::Label::new(RichText::new(text[lines[line].clone()].trim_end_matches('\n')).monospace()).extend());
                                }
                            });
                    });
                    help::annotate(ui, &notice.header_response, help::Control::LicenseNotice);
                }
            });
            ui.label("Offline records are embedded in this executable. Installed releases also retain manifest.json, notices.json and release.json under ~/.local/share/omatainer/licenses.");
        });
        self.licenses.open = open && !close_requested;
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod contextual_help_tests {
    use super::*;
    use crate::ui::help::Control;

    #[test]
    fn actual_offline_license_viewer_describes_search_selection_source_and_notice_controls() {
        let mut fixture = test_support::Fixture::new(64);
        let catalog = Catalog::parse(crate::licenses::MANIFEST, crate::licenses::NOTICES).unwrap();
        let selected = catalog
            .manifest
            .entries
            .iter()
            .position(|entry| entry.id == "font:ubuntu")
            .unwrap();
        fixture.app.licenses.catalog = Some(catalog);
        fixture.app.licenses.selected = selected;
        fixture.app.licenses.query = "font:ubuntu".into();
        fixture.app.licenses.open = true;
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut output = None;
        for _ in 0..3 {
            output = Some(ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1500.0, 1200.0))),
                    ..Default::default()
                },
                |ctx| fixture.app.update_frame(ctx),
            ));
        }
        let nodes = output
            .unwrap()
            .platform_output
            .accesskit_update
            .unwrap()
            .nodes;
        for control in [
            Control::ClosePanel,
            Control::License,
            Control::LicenseSearch,
            Control::LicenseEntry,
            Control::LicenseSource,
            Control::LicenseNotice,
        ] {
            assert!(
                nodes.iter().any(|(_, node)| node
                    .description()
                    .is_some_and(|text| text.contains(control.definition().purpose))),
                "Missing actual license help for {control:?}"
            );
        }
        assert_eq!(fixture.app.licenses.selected, selected);
        assert!(fixture.app.licenses.pending.is_none());
        assert!(
            fixture.decoder_jobs.try_recv().is_err(),
            "reading help must not request media"
        );
    }
}
