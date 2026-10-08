use super::*;
#[cfg(test)] mod tests;
use crate::{
    dj_library::{Candidate, Discovery, Purpose},
    producer_library::Pack,
};
#[derive(Default)]
pub(super) struct Panel {
    pub(super) open: bool,
    pub(super) selected: Vec<Pack>,
    discovery: Discovery,
    custom_root: String,
    configured_only: bool,
    events: Option<Receiver<Result<Pack, String>>>,
    cancel: Option<Arc<AtomicBool>>,
    reviewed: Option<Pack>,
    accepted: bool,
    message: String,
}
impl Panel {
    pub(super) fn poll(&mut self) {
        self.discovery.poll();
        let Some(events) = &self.events else {
            return;
        };
        let result = match events.try_recv() {
            Ok(result) => result,
            Err(crossbeam_channel::TryRecvError::Empty) => return,
            Err(_) => Err("Producer metadata worker ended without a result".into()),
        };
        let cancelled = self
            .cancel
            .as_ref()
            .is_some_and(|c| c.load(Ordering::Acquire));
        self.events = None;
        self.cancel = None;
        if cancelled {
            self.message = "Producer review cancelled; previous libraries retained".into();
            return;
        }
        match result {
            Ok(pack) => {
                self.reviewed = Some(pack);
                self.accepted = false;
                self.message="Review installation identity and applicable content rights before resolving assets".into();
            }
            Err(error) => self.message = error,
        }
    }
    pub(super) fn cancel(&mut self) {
        self.discovery.cancel();
        if let Some(cancel) = &self.cancel {
            cancel.store(true, Ordering::Release);
        }
    }
    fn review(&mut self, candidate: Candidate, engine: &Engine) {
        if self.events.is_some() {
            return;
        }
        let result = (|| -> Result<(), String> {
            let work = engine
                .cmd
                .performance()
                .optional_work()
                .map_err(|e| e.to_string())?;
            let cancel = work.cancel();
            let (done, events) = bounded(1);
            let ticket = work.background(
                crate::background::Kind::Index,
                "Producer metadata review".into(),
                32 * crate::background::MIB,
            )?;
            std::thread::Builder::new()
                .name("producer-library-review".into())
                .spawn(move || {
                    let result = (|| {
                        let _running = ticket.enter(|| work.cancelled())?;
                        crate::producer_library::isolated(&candidate, &work.cancel())
                    })();
                    let _ = done.send(result);
                })
                .map_err(|e| e.to_string())?;
            self.events = Some(events);
            self.cancel = Some(cancel);
            self.reviewed = None;
            self.accepted = false;
            Ok(())
        })();
        if let Err(error) = result {
            self.message = error;
        }
    }
}
impl App {
    pub(super) fn producer_library_ui(&mut self, ctx: &egui::Context) {
        if !self.ableton.library.open {
            return;
        }
        let mut open = true;
        let mut scan = None;
        let mut review = None;
        let mut preset = None;
        let mut sample_folder = None;
        let enabled = !self.engine.cmd.performance().protected()
            && !self.ableton.busy()
            && !self.project.busy();
        egui::Window::new("Producer libraries").id(egui::Id::new("producer-libraries")).open(&mut open).default_width(680.).vscroll(true).show(ctx,|ui|{
            let panel=&mut self.ableton.library;
            ui.label("Discover samples, user presets and installed Pack manifests. Source installations stay unchanged.");
            let label=ui.label("Custom producer library directory");ui.add(egui::TextEdit::singleline(&mut panel.custom_root).char_limit(4096)).labelled_by(label.id);
            ui.checkbox(&mut panel.configured_only,"Search home and configured directories only");
            ui.horizontal(|ui|{if ui.add_enabled(enabled&&!panel.discovery.active(),egui::Button::new("Search producer libraries")).clicked(){scan=Some(false);}if ui.add_enabled(enabled&&!panel.discovery.active()&&panel.discovery.cursor.is_some(),egui::Button::new("Continue producer discovery")).clicked(){scan=Some(true);}if ui.add_enabled(panel.discovery.active()||panel.events.is_some(),egui::Button::new("Cancel producer work")).clicked(){panel.cancel();}});
            ui.label(&panel.discovery.message);
            ui.label("Pages inspect at most 10000 entries. Continue until finished; exclusions and capacity limits are reported. Results are snapshots and checked again on review.");
            egui::ScrollArea::vertical().id_salt("producer-candidates").max_height(230.).show_rows(ui,65.,panel.discovery.candidates.len(),|ui,range|{for index in range{let candidate=&panel.discovery.candidates[index];ui.horizontal(|ui|{
                if candidate.format.starts_with("Installed Pack") {if ui.add_enabled(enabled&&panel.events.is_none(),egui::Button::new("Review Pack manifest")).clicked(){review=Some(candidate.clone());}}
                else if candidate.reviewable {if ui.add_enabled(enabled,egui::Button::new("Review user source")).clicked(){preset=Some(candidate.path.clone());}}
                else if candidate.format.starts_with("Ordinary audio") && ui.add_enabled(enabled,egui::Button::new("Use sample folder for relink")).clicked(){sample_folder=candidate.path.parent().map(PathBuf::from);}
                ui.add(egui::Label::new(&candidate.format).truncate());
            });ui.add(egui::Label::new(candidate.path.display().to_string()).truncate());}});
            for notice in &panel.discovery.notices{ui.add(egui::Label::new(format!("{}: {}",notice.path.display(),notice.state)).truncate()).on_hover_text(&notice.state);}
            if let Some(pack)=&panel.reviewed {
                ui.separator();for key in ["PackDisplayName","PackUniqueID","PackVendor","PackMajorVersion","PackMinorVersion","PackRevision"] {ui.label(format!("{key}: {}",pack.fields.get(key).map_or("absent",String::as_str)));}
                ui.label("A manifest identifies installed content. It supplies neither a native device engine nor redistribution rights. Only verified referenced audio is embedded; original device state stays retained.");
                ui.checkbox(&mut panel.accepted,"I can use this installed content in the imported project");
                if ui.add_enabled(enabled&&panel.accepted&&panel.selected.len()<32,egui::Button::new("Use reviewed Pack for next import")).clicked(){let pack=pack.clone();panel.selected.retain(|p|p.fields.get("PackUniqueID")!=pack.fields.get("PackUniqueID"));panel.selected.push(pack);panel.message="Pack selected. Review the source Set again to resolve its declared Pack asset references.".into();}
            }
            ui.label(format!("{} explicit Pack installations selected",panel.selected.len()));if ui.add_enabled(enabled&&!panel.selected.is_empty(),egui::Button::new("Clear selected Pack installations")).clicked(){panel.selected.clear();}
            ui.label(&panel.message);
        });
        self.ableton.library.open = open;
        if !open {
            self.ableton.library.cancel();
        }
        if let Some(continuation) = scan {
            let mut roots = self.settings.profile().library_roots.clone();
            if let Some(home) = std::env::var_os("HOME") {
                roots.insert(0, PathBuf::from(home));
            }
            if !self.ableton.library.custom_root.trim().is_empty() {
                roots.insert(0, PathBuf::from(self.ableton.library.custom_root.trim()));
            }
            roots.dedup();
            let panel = &mut self.ableton.library;
            if let Err(error) = panel.discovery.start_for(
                Purpose::Producer,
                roots,
                !panel.configured_only,
                continuation,
                self.engine.cmd.performance(),
            ) {
                panel.discovery.message = error;
            }
        }
        if let Some(candidate) = review {
            self.ableton.library.review(candidate, &self.engine);
        }
        if let Some(path) = preset {
            self.ableton.path = path.display().to_string();
            self.ableton.native_review = false;
            self.ableton.start(&self.engine, false);
        }
        if let Some(folder) = sample_folder {
            self.ableton.to = folder.display().to_string();
            self.ableton.library.message="Replacement folder selected. Supply the source path prefix and review the Set again; filename-only matches are never inferred.".into();
        }
    }
}
