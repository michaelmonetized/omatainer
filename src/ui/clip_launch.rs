use super::*;
use crate::engine::clip_launch::{Grid, Mode, Policy, Press, Release, Target};

const SOURCE: u64 = u64::MAX;
#[derive(Clone, Copy, Debug)]
struct Held {
    key: u32,
    id: egui::Id,
}
#[derive(Default, Debug)]
pub(super) struct Holds {
    held: [Option<Held>; 4],
    released: [Option<u32>; 3],
    activated: Vec<u32>,
}
impl App {
    fn set_clip_hold(
        &mut self,
        index: usize,
        key: u32,
        id: egui::Id,
        track: u8,
        scene: u16,
        on: bool,
    ) {
        if self.clip_launch_holds.held[index].is_some_and(|h| h.key == key) == on {
            return;
        }
        if let Some(old) = self.clip_launch_holds.held[index].take() {
            self.send(Command::ClipRelease(Release {
                source: SOURCE,
                key: old.key,
            }));
        }
        if on
            && self.submit(Command::ClipPress(Press {
                source: SOURCE,
                key,
                target: Target::Slot {
                    track,
                    scene,
                    looping: false,
                },
            }))
        {
            self.clip_launch_holds.held[index] = Some(Held { key, id });
        }
    }
    pub(super) fn clip_launch_releases(&mut self, ctx: &egui::Context) {
        if ctx.current_pass_index() == 0 {
            self.clip_launch_holds.released = [None; 3];
            self.clip_launch_holds.activated.clear();
        }
        for index in 0..4 {
            let Some(held) = self.clip_launch_holds.held[index] else {
                continue;
            };
            let release = ctx.input(|i| {
                !i.focused
                    || match index {
                        0 => {
                            i.pointer.button_released(egui::PointerButton::Primary)
                                || !i.pointer.button_down(egui::PointerButton::Primary)
                        }
                        1 => !i.key_down(Key::Space),
                        2 => !i.key_down(Key::Enter),
                        _ => false,
                    }
            }) || matches!(index, 1 | 2)
                && ctx.memory(|m| m.focused() != Some(held.id));
            if release {
                self.clip_launch_holds.held[index] = None;
                self.send(Command::ClipRelease(Release {
                    source: SOURCE,
                    key: held.key,
                }));
                if index < 3 {
                    self.clip_launch_holds.released[index] = Some(held.key);
                }
            }
        }
    }
    /// Apply held native clip input through the same source-owned engine path as controllers.
    /// Takes one visible clip response and alternate action; returns whether configured input consumed its activation, while releases remain independent of scrolling and selection.
    pub(super) fn clip_launch_input(
        &mut self,
        response: &egui::Response,
        track: usize,
        scene: usize,
        mode: Mode,
        enabled: bool,
        action: Option<usize>,
    ) -> bool {
        let base = (track * crate::engine::session::MAX_SCENES + scene) as u32 * 8;
        let ctx = &response.ctx;
        let normal = ctx.input(|i| {
            !i.modifiers.shift
                && !i.modifiers.alt
                && !i.modifiers.ctrl
                && !i.modifiers.command
                && i.focused
        });
        let pointer = response.is_pointer_button_down_on()
            && ctx.input(|i| i.pointer.button_down(egui::PointerButton::Primary));
        let mut handled = self
            .clip_launch_holds
            .released
            .iter()
            .flatten()
            .any(|k| *k / 8 == base / 8);
        if enabled && normal && pointer {
            self.set_clip_hold(0, base, response.id, track as u8, scene as u16, true);
            handled = true;
            response.request_focus();
        }
        for (index, key) in [(1, Key::Space), (2, Key::Enter)] {
            if enabled&&normal&&response.has_focus()&&ctx.input(|i|i.key_pressed(key)&&!i.events.iter().any(|e|matches!(e,egui::Event::Key{key:k,pressed:true,repeat:true,..}if *k==key))){self.set_clip_hold(index,base+index as u32,response.id,track as u8,scene as u16,true);keyboard::block_for_activation(ctx);handled=true;}
            if self.clip_launch_holds.held[index].is_some_and(|h| h.key / 8 == base / 8) {
                handled = true;
            }
        }
        let assisted = ctx.input(|i| {
            i.num_accesskit_action_requests(response.id, egui::accesskit::Action::Click) > 0
        });
        if action == Some(7) {
            self.set_clip_hold(3, base + 3, response.id, track as u8, scene as u16, false);
            handled = true;
        }
        if enabled
            && (action == Some(6) || assisted)
            && !self.clip_launch_holds.activated.contains(&base)
        {
            self.clip_launch_holds.activated.push(base);
            if action == Some(6) || matches!(mode, Mode::Gate | Mode::Repeat) {
                let on = action == Some(6)
                    || !self.clip_launch_holds.held[3].is_some_and(|h| h.key == base + 3);
                self.set_clip_hold(3, base + 3, response.id, track as u8, scene as u16, on);
            } else {
                self.send(Command::ClipPress(Press {
                    source: SOURCE,
                    key: base + 4,
                    target: Target::Slot {
                        track: track as u8,
                        scene: scene as u16,
                        looping: false,
                    },
                }));
                self.send(Command::ClipRelease(Release {
                    source: SOURCE,
                    key: base + 4,
                }));
            }
            handled = true;
        }
        if action == Some(8) {
            self.send(Command::ClipCancel { track: track as u8 });
            handled = true;
        }
        handled |= self.clip_launch_holds.activated.contains(&base);
        handled
    }
}
/// Edit persisted launch choices in the native clip manager.
/// Takes the UI and policy draft; updates mode, inherited or musical timing, and legato without changing playback before Apply.
pub(super) fn policy_ui(ui: &mut Ui, policy: &mut Policy) {
    ui.horizontal(|ui| {
        ui.label("Launch mode");
        let mode = egui::ComboBox::from_id_salt("Clip launch mode")
            .selected_text(policy.mode.label())
            .show_ui(ui, |ui| {
                for mode in Mode::ALL {
                    if ui
                        .selectable_value(&mut policy.mode, mode, mode.label())
                        .clicked()
                    {
                        ui.close();
                    }
                }
            });
        accessibility::button(ui, &mode.response, "Clip launch mode", None);
        ui.label("Launch timing");
        let timing = egui::ComboBox::from_id_salt("Clip launch timing")
            .selected_text(policy.grid.label())
            .show_ui(ui, |ui| {
                for grid in Grid::ALL {
                    if ui
                        .selectable_value(&mut policy.grid, grid, grid.label())
                        .clicked()
                    {
                        ui.close();
                    }
                }
            });
        accessibility::button(ui, &timing.response, "Clip launch timing", None);
        let response = ui.checkbox(&mut policy.legato, "Legato clip switching");
        accessibility::button(ui, &response, "Legato clip switching", Some(policy.legato));
    });
    ui.label("Trigger: press starts; release leaves playing. Gate: hold to play. Toggle: press starts, next press stops. Repeat: hold to retrigger on the chosen beat grid; Immediate repeats at the clip length. Legato carries the previous musical phase.");
}
