//! Recovery happens before audio ownership exists. A selected retry execs a fresh
//! process, avoiding CPAL Stream transfer or a second winit event-loop instance.
use std::sync::{
    atomic::{AtomicU8, Ordering},
    Arc,
};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Choice {
    Exit,
    Retry,
    DefaultsOnce,
}
struct Recovery {
    message: String,
    invalid_preferences: bool,
    choice: Arc<AtomicU8>,
}
impl eframe::App for Recovery {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx,|ui|{
            ui.heading("Setup needs attention"); ui.label(&self.message);
            ui.label("No alternate audio route was selected automatically. Your saved file is preserved.");
            if self.invalid_preferences { ui.label("Continue with defaults to inspect Preferences. Saving remains blocked until Reload succeeds or you explicitly preserve the old file and reset."); }
            let choices=[("Retry saved configuration",1),(if self.invalid_preferences {"Continue with defaults this time"}else{"Use system default audio this time"},2),("Exit",0)];
            for (label,value) in choices {
                if ui.button(label).clicked(){self.choice.store(value,Ordering::Release);ctx.send_viewport_cmd(egui::ViewportCommand::Close);}
            }
        });
    }
}
pub fn show(message: &str, invalid_preferences: bool) -> anyhow::Result<Choice> {
    let choice = Arc::new(AtomicU8::new(0));
    let selected = choice.clone();
    let message = message.to_owned();
    eframe::run_native(
        "omatainer · setup recovery",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_inner_size([680.0, 360.0])
                .with_title("omatainer · setup recovery"),
            ..Default::default()
        },
        Box::new(move |_cc| {
            Ok(Box::new(Recovery {
                message,
                invalid_preferences,
                choice: selected,
            }))
        }),
    )
    .map_err(|error| anyhow::anyhow!("{error}"))?;
    Ok(match choice.load(Ordering::Acquire) {
        1 => Choice::Retry,
        2 => Choice::DefaultsOnce,
        _ => Choice::Exit,
    })
}
pub fn restart(choice: Choice) -> anyhow::Result<()> {
    use std::os::unix::process::CommandExt;
    if choice == Choice::Exit {
        return Ok(());
    }
    let mut command = std::process::Command::new(std::env::current_exe()?);
    if choice == Choice::DefaultsOnce {
        command.arg("--defaults-once");
    }
    Err(command.exec().into())
}
