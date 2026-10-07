//! Touch contacts own one performance control until release or cancellation.
use super::*;

const FRAME: &str = "performance-touch-frame";
const MAX_CONTACTS: usize = 32;
const MAX_HITS: usize = 64;
const MAX_EVENTS: usize = 256;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Target {
    Pad(u8),
    DeckPad { deck: u8, pad: u8 },
    Cue(u8),
    Pitch(u8),
    Crossfader,
}

#[derive(Clone, Copy)]
struct Hit {
    target: Target,
    rect: Rect,
    track: Rect,
    layer: egui::LayerId,
    enabled: bool,
}

#[derive(Clone, Default)]
struct Frame {
    hits: Vec<Hit>,
    mouse: Mouse,
}

#[derive(Clone, Copy, Default)]
pub(super) struct Mouse {
    pos: Option<Pos2>,
    origin: Option<Pos2>,
    pub down: bool,
    pub pressed: bool,
    pub released: bool,
    moved: bool,
}
impl Mouse {
    /// Check whether the physical pointer started on this visible widget.
    /// Takes its response; returns whether the press belongs to its current layer.
    pub fn starts_here(&self, response: &egui::Response) -> bool {
        response.enabled()
            && self.origin.is_some_and(|pos| {
                response.interact_rect.contains(pos)
                    && response.ctx.layer_id_at(pos) == Some(response.layer_id)
            })
    }
    /// Read an independent physical mouse click or drag on this widget.
    /// Takes its response; returns the latest position only during its own gesture.
    pub fn position(&self, response: &egui::Response) -> Option<Pos2> {
        if (self.pressed || self.down && self.moved || self.released) && self.starts_here(response)
        {
            self.pos
        } else {
            None
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Key {
    device: egui::TouchDeviceId,
    contact: egui::TouchId,
}

struct Contact {
    key: Key,
    hit: Hit,
    pos: Pos2,
    pressure: f32,
    changed: bool,
}

#[derive(Default)]
pub(super) struct Input {
    contacts: Vec<Contact>,
    events: Vec<egui::Event>,
    mouse: Mouse,
    pointer_touch: Option<egui::TouchId>,
    focused: bool,
    viewport: Option<(Rect, f32, Option<Vec2>)>,
    pub open: bool,
    pub cancelled: u64,
    pub rejected: u64,
}

/// Register a current performance target and read its independent physical pointer.
/// Takes the rendered response, target and value track; returns mouse input without touch emulation.
pub(super) fn register(
    ui: &Ui,
    response: &egui::Response,
    target: Target,
    track: Rect,
    enabled: bool,
) -> Mouse {
    let hit = Hit {
        target,
        rect: response.interact_rect.intersect(ui.clip_rect()),
        track,
        layer: response.layer_id,
        enabled: response.enabled() && enabled,
    };
    let frame_key = viewport_key(ui.ctx().viewport_id(), FRAME);
    ui.ctx().data_mut(|data| {
        let frame = data.get_temp_mut_or_default::<Frame>(frame_key);
        if hit.rect.is_positive() && frame.hits.len() < MAX_HITS {
            frame.hits.push(hit);
        }
        frame.mouse
    })
}

impl Input {
    /// Start collecting current widget geometry without opening a renderer gate.
    /// Takes the native frame; returns no value and marks touch-owned mouse emulation.
    pub fn begin(&mut self, ctx: &egui::Context) {
        if ctx.current_pass_index() == 0 {
            self.events.clear();
            ctx.input(|input| {
                self.focused = input.focused;
                self.events.extend(
                    input
                        .events
                        .iter()
                        .filter(|event| matches!(event, egui::Event::Touch { .. }))
                        .take(MAX_EVENTS + 1)
                        .cloned(),
                );
                self.read_mouse(&input.events);
                if !input.focused {
                    self.mouse = Mouse::default();
                    self.pointer_touch = None;
                }
            });
        }
        let frame_key = viewport_key(ctx.viewport_id(), FRAME);
        ctx.data_mut(|data| {
            data.insert_temp(
                frame_key,
                Frame {
                    hits: Vec::with_capacity(20),
                    mouse: self.mouse,
                },
            )
        });
    }

    /// Separate the pinned native backend's adjacent touch emulation from physical mouse events.
    /// Takes ordered native events; returns no value and retains independent mouse button ownership.
    fn read_mouse(&mut self, events: &[egui::Event]) {
        self.mouse.pressed = false;
        self.mouse.released = false;
        self.mouse.moved = false;
        let mut index = 0;
        while let Some(event) = events.get(index) {
            index += 1;
            match event {
                egui::Event::Touch { id, phase, pos, .. }
                    if self.pointer_touch.is_none() || self.pointer_touch == Some(*id) =>
                {
                    let start = *phase == egui::TouchPhase::Start;
                    let end = *phase == egui::TouchPhase::End;
                    if start {
                        self.pointer_touch = Some(*id);
                    }
                    if matches!(phase, egui::TouchPhase::End | egui::TouchPhase::Cancel) {
                        self.pointer_touch = None;
                    }
                    let moved = matches!(events.get(index), Some(egui::Event::PointerMoved(next)) if next == pos);
                    let pressed = matches!(events.get(index + 1), Some(egui::Event::PointerButton {
                        button: PointerButton::Primary, pressed: true, pos: next, ..
                    }) if next == pos);
                    if start && moved && pressed {
                        index += 2;
                    } else if *phase == egui::TouchPhase::Move && moved {
                        index += 1;
                    }
                    if end
                        && matches!(
                            events.get(index),
                            Some(egui::Event::PointerButton {
                                button: PointerButton::Primary,
                                pressed: false,
                                ..
                            })
                        )
                        && matches!(events.get(index + 1), Some(egui::Event::PointerGone))
                    {
                        index += 1;
                    }
                    if matches!(phase, egui::TouchPhase::End | egui::TouchPhase::Cancel)
                        && matches!(events.get(index), Some(egui::Event::PointerGone))
                    {
                        index += 1;
                    }
                }
                egui::Event::PointerMoved(pos) => {
                    self.mouse.pos = Some(*pos);
                    self.mouse.moved = true;
                }
                egui::Event::PointerButton {
                    pos,
                    button: PointerButton::Primary,
                    pressed,
                    ..
                } => {
                    self.mouse.pos = Some(*pos);
                    self.mouse.down = *pressed;
                    if *pressed {
                        self.mouse.origin = Some(*pos);
                        self.mouse.pressed = true;
                    } else {
                        self.mouse.released = true;
                    }
                }
                egui::Event::PointerGone => {
                    self.mouse.pos = None;
                }
                _ => {}
            }
        }
    }

    /// Forget input ownership when the project, focus or input device cancels it.
    /// Takes no arguments; returns no value and leaves no resumed touch gestures.
    pub fn clear(&mut self) {
        self.cancelled = self.cancelled.saturating_add(self.contacts.len() as u64);
        self.contacts.clear();
    }

    /// Retire pointer ownership when a workspace moves or replaces its controls.
    /// Takes no arguments; returns no value and requires a fresh mouse or touch press.
    pub(super) fn retire(&mut self) {
        self.clear();
        self.mouse = Mouse::default();
    }

    /// Read the pads still owned by at least one contact.
    /// Takes this input state; returns one bit per held pad.
    fn gates(&self) -> u64 {
        self.contacts
            .iter()
            .fold(0, |mask, contact| match contact.hit.target {
                Target::Pad(pad) => mask | (1 << pad),
                Target::Cue(deck) => mask | (1 << (16 + deck)),
                Target::DeckPad { deck, pad } => mask | (1 << (18 + deck * 8 + pad)),
                _ => mask,
            })
    }

    /// Preserve pad press and release transitions in native event order.
    /// Takes the earlier mask and output list; appends changed gates with their attack pressure.
    fn edges(&self, before: u64, changes: &mut Vec<(Target, bool, f32)>) {
        let after = self.gates();
        for index in 0..18 + DECKS * 8 {
            if (before ^ after) & (1 << index) == 0 {
                continue;
            }
            let target = if index < 16 { Target::Pad(index as u8) } else if index < 18 { Target::Cue((index - 16) as u8) } else { Target::DeckPad { deck: ((index - 18) / 8) as u8, pad: ((index - 18) % 8) as u8 } };
            let on = after & (1 << index) != 0;
            let pressure = self
                .contacts
                .iter()
                .find(|contact| contact.hit.target == target)
                .map_or(1.0, |contact| contact.pressure);
            changes.push((target, on, pressure));
        }
    }

    /// Resolve contacts against this frame's layers and independent control ownership.
    /// Takes native events and blocking state; returns ordered pad edges and changed faders.
    fn finish(
        &mut self,
        ctx: &egui::Context,
        blocked: bool,
    ) -> (Vec<(Target, bool, f32)>, Vec<(Target, f32)>) {
        if ctx.will_discard() {
            return (Vec::new(), Vec::new());
        }
        let frame_key = viewport_key(ctx.viewport_id(), FRAME);
        let frame = ctx
            .data(|data| data.get_temp::<Frame>(frame_key))
            .unwrap_or_default();
        let viewport = (
            ctx.screen_rect(),
            ctx.pixels_per_point(),
            ctx.input(|input| input.viewport().monitor_size),
        );
        let focused = self.focused;
        let events = std::mem::take(&mut self.events);
        let mut changes = Vec::new();
        let before = self.gates();
        if blocked
            || !focused
            || self.viewport.is_some_and(|old| old != viewport)
            || events.len() > MAX_EVENTS
        {
            self.clear();
            self.viewport = Some(viewport);
            self.edges(before, &mut changes);
            return (changes, Vec::new());
        }
        self.viewport = Some(viewport);
        let old_count = self.contacts.len();
        self.contacts.retain(|contact| {
            frame.hits.iter().any(|hit| {
                hit.target == contact.hit.target
                    && hit.enabled
                    && hit.rect == contact.hit.rect
                    && hit.layer == contact.hit.layer
            })
        });
        self.cancelled = self
            .cancelled
            .saturating_add((old_count - self.contacts.len()) as u64);
        self.edges(before, &mut changes);
        for contact in &mut self.contacts {
            contact.changed = false;
        }
        for event in events {
            let egui::Event::Touch {
                device_id,
                id,
                phase,
                pos,
                force,
            } = event
            else {
                continue;
            };
            let key = Key {
                device: device_id,
                contact: id,
            };
            let mut before = self.gates();
            let index = self.contacts.iter().position(|contact| contact.key == key);
            if matches!(phase, egui::TouchPhase::End | egui::TouchPhase::Cancel) {
                if let Some(index) = index {
                    self.contacts.remove(index);
                    if phase == egui::TouchPhase::Cancel {
                        self.cancelled = self.cancelled.saturating_add(1);
                    }
                }
            } else if !pos.x.is_finite()
                || !pos.y.is_finite()
                || force.is_some_and(|force| !force.is_finite() || !(0.0..=1.0).contains(&force))
            {
                if let Some(index) = index {
                    self.contacts.remove(index);
                }
                self.rejected = self.rejected.saturating_add(1);
            } else if phase == egui::TouchPhase::Move {
                if let Some(index) = index {
                    self.contacts[index].pos = pos;
                    self.contacts[index].changed = true;
                }
            } else if phase == egui::TouchPhase::Start {
                if let Some(index) = index {
                    self.contacts.remove(index);
                    self.edges(before, &mut changes);
                    before = self.gates();
                }
                let layer = ctx.layer_id_at(pos);
                if let Some(hit) = frame
                    .hits
                    .iter()
                    .rev()
                    .find(|hit| hit.enabled && hit.rect.contains(pos) && Some(hit.layer) == layer)
                    .copied()
                {
                    if self.contacts.len() == MAX_CONTACTS
                        || (!matches!(hit.target, Target::Pad(_) | Target::Cue(_) | Target::DeckPad { .. })
                            && self
                                .contacts
                                .iter()
                                .any(|contact| contact.hit.target == hit.target))
                    {
                        self.rejected = self.rejected.saturating_add(1);
                    } else {
                        self.contacts.push(Contact {
                            key,
                            hit,
                            pos,
                            pressure: force.unwrap_or(1.0),
                            changed: true,
                        });
                    }
                }
            }
            self.edges(before, &mut changes);
        }
        let mut faders = Vec::with_capacity(3);
        for contact in &self.contacts {
            if !contact.changed {
                continue;
            }
            let value = match contact.hit.target {
                Target::Pitch(_) => {
                    1.0 - (contact.pos.y - contact.hit.track.top()) / contact.hit.track.height()
                }
                Target::Crossfader => {
                    (contact.pos.x - contact.hit.track.left()) / contact.hit.track.width()
                }
                Target::Pad(_) | Target::Cue(_) | Target::DeckPad { .. } => continue,
            };
            if value.is_finite() {
                faders.push((contact.hit.target, value.clamp(0.0, 1.0)));
            }
        }
        (changes, faders)
    }
}

impl App {
    /// Show the input guide and pause performance gestures while it owns input.
    /// Takes the native context; returns no value and keeps only the guide visibility.
    pub(super) fn touch_input_ui(&mut self, ctx: &egui::Context) {
        if !self.touch_input.open {
            return;
        }
        keyboard::block_for_dialog(ctx);
        let mut open = true;
        egui::Window::new(tr!("Touch and pen gestures")).id(egui::Id::new("touch-input-guide")).open(&mut open)
            .vscroll(true).max_height(self.theme.window_height(ctx)).show(ctx,|ui| {
                ui.label(tr!("Hold multiple pads, Cue buttons, pitch faders and the crossfader independently. Each touch keeps its control until lifted or cancelled; moving off a pad or Cue keeps it held. One touch owns each fader."));
                ui.label(tr!("Reported pressure changes pad attack loudness. Faders ignore pressure; no pressure information uses full attack. Mouse, keyboard and assistive controls remain available."));
                ui.label(tr!("Opening an editor, losing focus or changing visible controls releases touch holds. A new press is required. Touches outside performance controls scroll normally; use UI scale in Preferences for larger targets."));
                ui.label(tr!("On Linux, pressure is unavailable; pad attacks use full loudness."));
                ui.label(tr!("Close this guide before performing. Other controls use the platform's ordinary single-pointer behavior."));
                ui.label(crate::localization::format("Cancelled contacts: {0}; rejected contacts: {1}",&[self.touch_input.cancelled.to_string(),self.touch_input.rejected.to_string()]));
            });
        self.touch_input.open = open;
    }

    /// Submit independently owned touch controls through ordinary command admission.
    /// Takes the rendered native frame; returns no value and releases cancelled pad gates.
    pub(super) fn finish_touch_input(&mut self, ctx: &egui::Context) {
        let blocked = self.engine.safe_mode()
            || self.engine.cmd.performance().status().recovery
            || self.project.committing()
            || !self.project.dialog_is_closed()
            || keyboard::dialogs_block_input(ctx);
        self.guard_cue_inputs(ctx, blocked);
        self.guard_deck_pad_inputs(ctx, blocked);
        let (gates, faders) = self.touch_input.finish(ctx, blocked);
        for (target, on, pressure) in gates {
            match target {
                Target::Pad(pad) => self.set_pad_input_pressure(pad as usize, 16, on, Some(pressure)),
                Target::Cue(deck) => self.set_cue_input(deck, 4, on, ctx.viewport_id()),
                Target::DeckPad { deck, pad } => self.set_deck_pad_input(deck, pad, 4, on, pressure, ctx.input(|i| i.modifiers.shift), ctx.viewport_id()),
                _ => {}
            }
        }
        let before = self.touch_input.contacts.len();
        self.touch_input
            .contacts
            .retain(|contact| match contact.hit.target {
                Target::Pad(pad) => self.pad_inputs[pad as usize] & 16 != 0,
                Target::Cue(deck) => self.cue_audition.owners[usize::from(deck)][4].is_some(),
                Target::DeckPad { deck, pad } => self.deck_pad_inputs.owners[usize::from(deck)][usize::from(pad)][4].is_some(),
                _ => true,
            });
        self.touch_input.rejected = self
            .touch_input
            .rejected
            .saturating_add((before - self.touch_input.contacts.len()) as u64);
        for (target, value) in faders {
            match target {
                Target::Pitch(deck) => self.send(Command::DeckPitch { deck, value }),
                Target::Crossfader => self.send(Command::Xfader(value)),
                Target::Pad(_) | Target::Cue(_) | Target::DeckPad { .. } => {}
            }
        }
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
/// Inspect a performance target rendered in a particular native window.
/// Takes context, viewport and target; returns its current clipped rectangle when present.
pub(super) fn target_rect(
    ctx: &egui::Context,
    viewport: egui::ViewportId,
    target: Target,
) -> Option<Rect> {
    ctx.data(|data| data.get_temp::<Frame>(viewport_key(viewport, FRAME)))?
        .hits
        .iter()
        .find(|hit| hit.target == target && hit.enabled)
        .map(|hit| hit.rect)
}
