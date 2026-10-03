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
    EmptyOnce,
}
struct Recovery {
    message: String,
    invalid_preferences: bool,
    template_failure: bool,
    choice: Arc<AtomicU8>,
}
impl eframe::App for Recovery {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx,|ui|{
            ui.heading("Setup needs attention"); ui.label(&self.message);
            ui.label("No alternate audio route was selected automatically. Your saved file is preserved.");
            if self.invalid_preferences { ui.label("Continue with defaults to inspect Preferences. Saving remains blocked until Reload succeeds or you explicitly preserve the old file and reset."); }
            let choices=[("Retry saved configuration",1),(if self.template_failure {"Start an empty session this time"}else if self.invalid_preferences {"Continue with defaults this time"}else{"Use system default audio this time"},2),("Exit",0)];
            for (label,value) in choices {
                if ui.button(label).clicked(){self.choice.store(if value==2 && self.template_failure {3}else{value},Ordering::Release);ctx.send_viewport_cmd(egui::ViewportCommand::Close);}
            }
        });
    }
}
pub fn show(message: &str, invalid_preferences: bool) -> anyhow::Result<Choice> {
    show_kind(message, invalid_preferences, false)
}
pub fn show_template(message: &str) -> anyhow::Result<Choice> {
    show_kind(message, false, true)
}
fn show_kind(
    message: &str,
    invalid_preferences: bool,
    template_failure: bool,
) -> anyhow::Result<Choice> {
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
                template_failure,
                choice: selected,
            }))
        }),
    )
    .map_err(|error| anyhow::anyhow!("{error}"))?;
    Ok(match choice.load(Ordering::Acquire) {
        1 => Choice::Retry,
        2 => Choice::DefaultsOnce,
        3 => Choice::EmptyOnce,
        _ => Choice::Exit,
    })
}
fn retry_arguments(choice: Choice, launch: crate::startup::Launch) -> Vec<&'static str> {
    let defaults =
        choice == Choice::DefaultsOnce || choice == Choice::EmptyOnce && launch.defaults_once;
    let empty = choice == Choice::EmptyOnce || choice == Choice::DefaultsOnce && launch.empty_once;
    let mut args = Vec::new();
    if defaults {
        args.push("--defaults-once");
    }
    if empty {
        args.push("--empty-once");
    }
    args
}
pub fn restart(choice: Choice, launch: crate::startup::Launch) -> anyhow::Result<()> {
    use std::os::unix::process::CommandExt;
    if choice == Choice::Exit {
        return Ok(());
    }
    let mut command = std::process::Command::new(std::env::current_exe()?);
    command.args(retry_arguments(choice, launch));
    Err(command.exec().into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn explicit_recovery_choices_survive_a_second_independent_setup_failure() {
        let empty = crate::startup::Launch {
            empty_once: true,
            ..Default::default()
        };
        assert_eq!(
            retry_arguments(Choice::DefaultsOnce, empty),
            ["--defaults-once", "--empty-once"]
        );
        let audio = crate::startup::Launch {
            defaults_once: true,
            ..Default::default()
        };
        assert_eq!(
            retry_arguments(Choice::EmptyOnce, audio),
            ["--defaults-once", "--empty-once"]
        );
        assert!(retry_arguments(Choice::Retry, empty).is_empty());
    }
}
