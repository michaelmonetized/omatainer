//! Forced reloads acknowledge the frame after egui installs font/zoom changes.
use super::*;
use crate::theme::requests::{Applied, Failure, Request};

pub(super) struct Pending {
    request: Request,
    installed: Option<(u64, crate::preferences::Appearance)>,
}
impl App {
    pub(super) fn poll_theme_requests(&mut self, ctx: &egui::Context) -> bool {
        if let Some(mut pending) = self.theme_request.take() {
            if let Some((frame, appearance)) = &pending.installed {
                let current = &self.settings.profile().appearance;
                if current != appearance {
                    // A newer applied profile remains authoritative. Fonts and
                    // zoom requested in this frame install at the next begin-pass.
                    let appearance = current.clone();
                    self.apply_appearance(ctx);
                    pending.installed = Some((ctx.cumulative_frame_nr(), appearance));
                } else if ctx.cumulative_frame_nr() > *frame {
                    pending.request.finish(Ok(Applied {
                        generation: pending.request.generation,
                        follow_theme: current.follow_theme,
                        font: self.theme.font.chars().take(128).collect(),
                        font_size: self.theme.font_size,
                        scale: ctx.zoom_factor(),
                    }));
                    return true;
                }
            } else if let Some(done) = self
                .theme_reload
                .as_ref()
                .and_then(|loader| loader.poll_forced())
            {
                debug_assert_eq!(done.generation, pending.request.generation);
                match done.result {
                    Err(error) => {
                        pending
                            .request
                            .finish(Err(Failure::new("invalid_theme", error)));
                        return true;
                    }
                    Ok(update) if pending.request.begin_apply() => {
                        self.settings.theme_update = Some(update);
                        let appearance = self.settings.profile().appearance.clone();
                        self.apply_appearance(ctx);
                        pending.installed = Some((ctx.cumulative_frame_nr(), appearance));
                    }
                    Ok(_) => return true, // cancelled before application
                }
            }
            // Keep the single worker lane occupied after timeout until its
            // result arrives. No abandoned jobs accumulate behind a slow OS read.
            self.theme_request = Some(pending);
            ctx.request_repaint_after(std::time::Duration::from_millis(16));
            return true;
        }
        for _ in 0..crate::theme::requests::CAPACITY {
            let Some(request) = self
                .theme_requests
                .as_ref()
                .and_then(|endpoint| endpoint.next())
            else {
                return false;
            };
            if request.cancelled() {
                continue;
            }
            match self.theme_reload.as_ref() {
                Some(loader) => match loader.force(request.generation) {
                    Ok(()) => {
                        self.theme_request = Some(Pending {
                            request,
                            installed: None,
                        });
                        ctx.request_repaint_after(std::time::Duration::from_millis(16));
                        return true;
                    }
                    Err(error) => request.finish(Err(Failure::new("theme_busy", error))),
                },
                None => request.finish(Err(Failure::new(
                    "theme_unavailable",
                    "Theme resource worker is unavailable",
                ))),
            }
        }
        true
    }
}

#[cfg(test)]
mod tests;
