//! Decide keyboard ownership once, after widgets have handled this frame.
use eframe::egui;

const DIALOG_GUARD: &str = "omatainer-global-shortcut-dialog";
const ACTIVATION_GUARD: &str = "omatainer-focused-key-activation";

pub(super) fn text_is_focused(ctx: &egui::Context) -> bool {
    ctx.memory(|memory| memory.focused())
        .is_some_and(|id| egui::TextEdit::load_state(ctx, id).is_some())
}

fn dialog_is_open(ctx: &egui::Context) -> bool {
    ctx.memory(|memory| memory.top_modal_layer().is_some())
        || egui::Popup::is_any_open(ctx)
        || ctx.data(|data| {
            data.get_temp::<bool>(egui::Id::new(DIALOG_GUARD))
                .unwrap_or(false)
        })
}

/// Keep performance gestures outside text editing and blocking dialogs.
/// Takes the current native context; returns whether a dialog or text field owns input.
pub(super) fn dialogs_block_input(ctx: &egui::Context) -> bool {
    dialog_is_open(ctx) || text_is_focused(ctx)
}

/// Blocking dialogs call this before rendering, including on their first and
/// closing frames. egui's modal-layer query describes the *previous* frame.
pub(super) fn block_for_dialog(ctx: &egui::Context) {
    ctx.data_mut(|data| data.insert_temp(egui::Id::new(DIALOG_GUARD), true));
}

/// Keep a focused keyboard action from also reaching global shortcuts.
/// Takes the native context; returns no value and leaves independent touch gestures available.
pub(super) fn block_for_activation(ctx: &egui::Context) {
    ctx.data_mut(|data| data.insert_temp(egui::Id::new(ACTIVATION_GUARD), true));
}

#[derive(Default)]
pub(super) struct ShortcutFocus {
    text_last_frame: bool,
    blocked_at_start: bool,
}

impl ShortcutFocus {
    pub fn begin_frame(&mut self, ctx: &egui::Context) {
        ctx.data_mut(|data| data.remove::<bool>(egui::Id::new(DIALOG_GUARD)));
        ctx.data_mut(|data| data.remove::<bool>(egui::Id::new(ACTIVATION_GUARD)));
        // Escape can surrender text focus in egui's begin_pass, before App
        // runs. Retain that frame's editing ownership so Escape never leaks
        // through into CloseFx, and Enter/Tab cannot activate global actions.
        self.blocked_at_start = self.text_last_frame || text_is_focused(ctx) || dialog_is_open(ctx);
    }

    pub fn globals_allowed(&mut self, ctx: &egui::Context) -> bool {
        let text_now = text_is_focused(ctx);
        let focused_activation = ctx.memory(|memory| memory.focused().is_some())
            && ctx.input(|input| input.key_pressed(egui::Key::Space) || input.key_pressed(egui::Key::Enter));
        let allowed = !focused_activation && !self.blocked_at_start
            && !ctx.data(|data| data.get_temp::<bool>(egui::Id::new(ACTIVATION_GUARD)).unwrap_or(false))
            && !text_now
            && !dialog_is_open(ctx)
            && ctx.input(|input| input.focused);
        self.text_last_frame = text_now;
        allowed
    }
}
