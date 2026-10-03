use super::*;
use crate::music_provider::{self as provider, License, MusicProvider};
use provider::worker::{Job, Operation, Reply};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

pub(super) struct Panel {
    pub open: bool,
    provider: Arc<dyn MusicProvider>,
    license: License,
    query: String,
    page_query: String,
    pending_query: String,
    page: Option<provider::Page>,
    job: Option<Job>,
    epochs: (u64, u64),
    preview: Option<Watch>,
    message: String,
}
struct Watch {
    cancel: Arc<AtomicBool>,
    state: Arc<AtomicU8>,
    credits: String,
}
impl Default for Panel {
    fn default() -> Self {
        Self {
            open: false,
            provider: Arc::new(provider::freetouse::FreeToUse::default()),
            license: License::Missing,
            query: String::new(),
            page_query: String::new(),
            pending_query: String::new(),
            page: None,
            job: None,
            epochs: (0, 0),
            preview: None,
            message: String::new(),
        }
    }
}
impl Panel {
    /// Stop every provider operation on license revocation or panel close.
    /// Takes this panel; cancels worker publication and renderer audio immediately.
    fn stop(&mut self) {
        if let Some(job) = &self.job {
            job.cancel();
        }
        if let Some(preview) = &self.preview {
            preview.cancel.store(true, Ordering::Release);
        }
    }
}
impl Drop for Panel {
    fn drop(&mut self) {
        self.stop();
    }
}

impl App {
    /// Poll provider results without blocking a GUI frame.
    /// Takes current app state; installs only current, explicitly licensed preview requests.
    pub(super) fn poll_music_provider(&mut self) {
        let panel = &mut self.music_provider;
        if self.engine.safe_mode()
            || self.engine.cmd.performance().protected()
            || self.project.committing()
        {
            panel.stop();
        }
        if let Some(result) = panel.job.as_mut().and_then(Job::poll) {
            panel.job = None;
            match result {
                Ok(Reply::Page(page)) if panel.open && panel.license != License::Missing => {
                    panel.page_query = panel.pending_query.clone();
                    panel.page = Some(page);
                    panel.message.clear();
                }
                Ok(Reply::Preview(preview)) if panel.open && panel.license != License::Missing => {
                    let state = Arc::new(AtomicU8::new(0));
                    let request = crate::engine::provider_preview::Request {
                        audio: preview.audio,
                        cancel: preview.cancel.clone(),
                        state: state.clone(),
                        transport_epoch: panel.epochs.0,
                        safety_epoch: panel.epochs.1,
                    };
                    let watch = Watch {
                        cancel: preview.cancel,
                        state,
                        credits: preview.track.attribution(),
                    };
                    if let Err(error) = self.engine.cmd.send(Command::ProviderPreview(request)) {
                        watch.cancel.store(true, Ordering::Release);
                        panel.message = format!("Preview was not admitted: {error}");
                    } else {
                        panel.preview = Some(watch);
                        panel.message = "Preview pending renderer confirmation".into();
                    }
                }
                Ok(_) => panel.message = "Provider request cancelled".into(),
                Err(error) => panel.message = error.to_string(),
            }
        }
        if let Some(preview) = &panel.preview {
            match preview.state.load(Ordering::Acquire) {
                1 => panel.message = "Playing original-pitch preview · 25% preview level".into(),
                2 => panel.message = "Preview rejected: session or access changed".into(),
                3 => panel.message = "Preview ended".into(),
                _ => {}
            }
        }
    }

    /// Show provider access, real catalog pages and current restrictions.
    /// Takes the frame context; only explicit widget actions start network requests or audio.
    pub(super) fn music_provider_ui(&mut self, ctx: &egui::Context) {
        if !self.music_provider.open {
            return;
        }
        keyboard::block_for_dialog(ctx);
        let panel = &mut self.music_provider;
        let identity = panel.provider.identity();
        let mut open = true;
        let mut operation = None;
        egui::Window::new("Music providers").id(egui::Id::new("music-provider-window"))
            .open(&mut open).default_width(640.0).vscroll(true)
            .max_height(self.theme.window_height(ctx)).show(ctx, |ui| {
                ui.heading(identity.name);
                ui.label(format!("Provider: {:?} · authentication: {:?}", identity.id, panel.provider.authentication()));
                ui.hyperlink_to("API authorization and app licensing", identity.authorization);
                ui.hyperlink_to("Read current music license", identity.terms);
                ui.label("No paid provider license is configured. Commercial music discovery and playback need a license for Omatainer.");
                ui.label("Free use is limited to non-commercial projects, non-premium music and visible attribution. Your published content needs its own license.");
                let mut enabled = panel.license == License::NonCommercialAttribution;
                if ui.checkbox(&mut enabled, "Enable for non-commercial use; I accept the current license and attribution requirement")
                    .help_detail(ui, HelpControl::MusicProvider, "Network access is off until enabled. No account or API key is required for the public API. Commercial use and premium music remain unavailable.").changed() {
                    panel.stop(); panel.page = None;
                    panel.license = if enabled { License::NonCommercialAttribution } else { License::Missing };
                }
                let caps = panel.provider.capabilities(panel.license, None);
                restrictions(ui, caps);
                if !identity.supported_platform { ui.label("Provider support is currently qualified on Linux only."); }
                let available = caps.search && panel.job.is_none() && !self.engine.safe_mode() && !self.engine.cmd.performance().protected() && !self.project.committing();
                let label = ui.label("Provider search");
                ui.add(egui::TextEdit::singleline(&mut panel.query).char_limit(1024)).labelled_by(label.id);
                ui.horizontal(|ui| {
                    if ui.add_enabled(available, egui::Button::new("Search provider")).help(ui, HelpControl::MusicProvider).clicked() {
                        operation = Some(Operation::Search { query: panel.query.clone(), offset: 0 });
                    }
                    if panel.job.is_some() && ui.button("Cancel provider request").help(ui, HelpControl::MusicProvider).clicked() { panel.stop(); }
                    if ui.button("Stop provider preview").help(ui, HelpControl::MusicProvider).clicked() { panel.stop(); }
                });
                if let Some(watch) = &panel.preview {
                    ui.label(&watch.credits);
                    if ui.button("Copy preview credits").help(ui, HelpControl::MusicProvider).clicked() { ctx.copy_text(watch.credits.clone()); }
                }
                ui.label(&panel.message);
                if let Some(page) = &panel.page {
                    ui.horizontal(|ui| {
                        ui.label(format!("{}–{} of {}", if page.tracks.is_empty() { 0 } else { page.offset + 1 }, page.offset + page.tracks.len() as u32, page.total));
                        if ui.add_enabled(available && page.offset > 0, egui::Button::new("Previous provider page")).clicked() {
                            operation = Some(Operation::Search { query: panel.page_query.clone(), offset: page.offset.saturating_sub(provider::freetouse::PAGE_SIZE) });
                        }
                        let next = page.offset + page.tracks.len() as u32;
                        if ui.add_enabled(available && !page.tracks.is_empty() && next < page.total, egui::Button::new("Next provider page")).clicked() {
                            operation = Some(Operation::Search { query: panel.page_query.clone(), offset: next });
                        }
                    });
                    for track in &page.tracks {
                        ui.separator();
                        ui.label(format!("{} · {} · {:.0}s{}", track.title, track.artists, track.seconds, if track.premium { " · premium" } else { "" }));
                        ui.label(format!("Remote ID: {}", track.id.item));
                        let caps = panel.provider.capabilities(panel.license, Some(track));
                        if ui.add_enabled(available && caps.preview, egui::Button::new(format!("Preview {}", track.title)))
                            .help_detail(ui, HelpControl::MusicProvider, "One unchanged-pitch preview; no local-file import, deck loading or audio export. Credits remain visible while it plays.").clicked() {
                            operation = Some(Operation::Preview(track.id.clone()));
                        }
                        if track.premium { ui.label(provider::Failure::PremiumLicenseRequired.to_string()); }
                        if track.seconds > 300.0 { ui.label("Preview limit: five minutes."); }
                        if ui.button(format!("Copy credits for {}", track.title)).clicked() { ctx.copy_text(track.attribution()); }
                    }
                }
            });
        panel.open = open;
        if !open {
            panel.stop();
        }
        if let Some(operation) = operation {
            if self.project.committing() {
                panel.stop();
                panel.message = "Provider requests wait for the project commit".into();
                return;
            }
            if let Operation::Search { query, .. } = &operation {
                panel.pending_query = query.clone();
            }
            panel.stop();
            panel.preview = None;
            panel.epochs = (
                self.snap.transport_epoch,
                self.engine.cmd.performance().safety_epoch(),
            );
            match self.engine.cmd.performance().optional_work() {
                Ok(permit) => match Job::start(
                    panel.provider.clone(),
                    operation,
                    panel.license,
                    Some(permit),
                ) {
                    Ok(job) => {
                        panel.job = Some(job);
                        panel.message = "Contacting provider…".into();
                    }
                    Err(error) => panel.message = error.to_string(),
                },
                Err(error) => panel.message = error.to_string(),
            }
        }
        if panel.job.is_some()
            || panel
                .preview
                .as_ref()
                .is_some_and(|p| p.state.load(Ordering::Acquire) < 2)
        {
            ctx.request_repaint_after(std::time::Duration::from_millis(20));
        }
    }
}

/// Describe the current adapter contract without inventing provider permissions.
/// Takes native UI and current capabilities; shows the restrictions enforced by this adapter.
fn restrictions(ui: &mut Ui, caps: provider::Capabilities) {
    ui.label(format!(
        "Current adapter capabilities: search {} · preview {} · preview voices {} · DJ decks {}",
        caps.search, caps.preview, caps.preview_voices, caps.decks
    ));
    ui.label(format!(
        "Offline storage {} · stems {} · recording/export {}",
        caps.offline, caps.stems, caps.recording
    ));
    ui.label("Provider previews are transient and excluded from local crates, sampler banks, project media, recording and portable exports.");
}

#[cfg(test)]
mod tests;
