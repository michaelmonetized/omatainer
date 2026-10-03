use super::*;
use crate::engine::{dsp::InputKey, RtEngine, SamplerInstrument, SynthInstrument};
use egui::accesskit::{Action, ActionRequest, Node, NodeId};

struct Gui {
    app: App,
    rt: RtEngine,
    ctx: egui::Context,
    nodes: Vec<(NodeId, Node)>,
    time: f64,
    size: Vec2,
    focused: bool,
    discard: bool,
}
impl Gui {
    fn new() -> Self {
        let (engine, mut rt) = Engine::headless_for_test(48_000, 256);
        rt.apply(Command::SamplerInst(SamplerInstrument::Synth(
            SynthInstrument::Keys,
        )));
        rt.publish_for_test();
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut gui = Self {
            app: App::with_loader(engine, Theme::default(), None),
            rt,
            ctx,
            nodes: Vec::new(),
            time: 0.0,
            size: Vec2::new(1600.0, 1200.0),
            focused: true,
            discard: false,
        };
        gui.frame(vec![]);
        gui.frame(vec![]);
        gui
    }
    fn frame(&mut self, events: Vec<egui::Event>) {
        self.time += 0.02;
        let output = self.ctx.run(
            egui::RawInput {
                focused: self.focused,
                time: Some(self.time),
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, self.size)),
                events,
                ..Default::default()
            },
            |ctx| {
                if self.discard && ctx.current_pass_index() == 0 {
                    ctx.request_discard("touch regression");
                }
                self.app.update_frame(ctx);
            },
        );
        self.discard = false;
        self.nodes = output.platform_output.accesskit_update.unwrap().nodes;
        self.rt.process(&mut [0.0; 128]);
        self.rt.publish_for_test();
    }
    fn hit(&self, target: Target) -> Hit {
        self.ctx
            .data(|data| data.get_temp::<Frame>(egui::Id::new(FRAME)))
            .unwrap()
            .hits
            .into_iter()
            .find(|hit| hit.target == target && hit.enabled)
            .expect("actual rendered touch target")
    }
    fn held(&self, pad: u8) -> bool {
        self.rt.sampler_poly.voices.iter().any(|voice| {
            voice.input == Some(InputKey::Pad(pad)) && matches!(voice.env.stage, 1..=3)
        })
    }
    fn click(&mut self, label: &str) {
        let target = self
            .nodes
            .iter()
            .find(|(_, node)| node.label() == Some(label))
            .expect(label)
            .0;
        self.frame(vec![egui::Event::AccessKitActionRequest(ActionRequest {
            target,
            action: Action::Click,
            data: None,
        })]);
        self.frame(vec![]);
    }
    fn start(&self, device: u64, id: u64, target: Target, force: Option<f32>) -> egui::Event {
        touch(
            device,
            id,
            egui::TouchPhase::Start,
            self.hit(target).rect.center(),
            force,
        )
    }
}
fn touch(
    device: u64,
    id: u64,
    phase: egui::TouchPhase,
    pos: Pos2,
    force: Option<f32>,
) -> egui::Event {
    egui::Event::Touch {
        device_id: egui::TouchDeviceId(device),
        id: egui::TouchId(id),
        phase,
        pos,
        force,
    }
}
fn release(device: u64, id: u64, phase: egui::TouchPhase) -> egui::Event {
    touch(
        device,
        id,
        phase,
        Pos2::new(f32::NAN, f32::NAN),
        Some(f32::NAN),
    )
}

#[test]
fn native_contacts_hold_independent_pads_and_share_same_pad_across_devices() {
    let mut gui = Gui::new();
    let pos = gui.hit(Target::Pad(0)).rect.center();
    gui.frame(vec![
        gui.start(10, 1, Target::Pad(0), Some(0.2)),
        gui.start(10, 2, Target::Pad(1), None),
        egui::Event::PointerMoved(pos),
        egui::Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed: true,
            modifiers: Default::default(),
        },
    ]);
    assert!(gui.held(0) && gui.held(1));
    assert_eq!(
        gui.rt.sampler_poly.note_on_events, 2,
        "emulated primary mouse must not duplicate touch"
    );
    assert_eq!(gui.app.pad_inputs[0], 16);
    let voice = gui
        .rt
        .sampler_poly
        .voices
        .iter()
        .find(|voice| voice.input == Some(InputKey::Pad(0)))
        .unwrap();
    assert!((voice.vel - 0.18).abs() < 1e-6);
    gui.frame(vec![
        gui.start(20, 1, Target::Pad(0), Some(0.8)),
        touch(10, 1, egui::TouchPhase::Move, Pos2::ZERO, None),
    ]);
    assert_eq!(
        gui.rt.sampler_poly.note_on_events, 2,
        "a second owner must not retrigger an already held pad"
    );
    gui.frame(vec![release(10, 1, egui::TouchPhase::End)]);
    assert!(gui.held(0) && gui.held(1));
    gui.frame(vec![release(10, 2, egui::TouchPhase::Cancel)]);
    assert!(gui.held(0) && !gui.held(1));
    gui.frame(vec![
        release(20, 1, egui::TouchPhase::End),
        egui::Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed: false,
            modifiers: Default::default(),
        },
    ]);
    assert!(!gui.held(0));
    assert!(gui.app.pad_inputs.iter().all(|mask| *mask == 0));
    assert!(gui.app.touch_input.contacts.is_empty());
}

#[test]
fn native_faders_move_independently_and_conflicting_contacts_cannot_steal() {
    let mut gui = Gui::new();
    gui.size.x = 2600.0;
    gui.frame(vec![]);
    gui.frame(vec![]);
    let pitch = gui.hit(Target::Pitch(0));
    let pitch_b = gui.hit(Target::Pitch(1));
    assert!(
        pitch_b.rect.contains(pitch_b.track.center()),
        "the full fader track must be visible"
    );
    assert!(pitch_b.rect.width() >= gui.app.theme.target_size(22.0));
    let cross = gui.hit(Target::Crossfader);
    gui.frame(vec![
        touch(
            1,
            1,
            egui::TouchPhase::Start,
            Pos2::new(pitch.track.center().x, pitch.track.top()),
            None,
        ),
        touch(
            1,
            2,
            egui::TouchPhase::Start,
            Pos2::new(cross.track.left(), cross.track.center().y),
            None,
        ),
        gui.start(1, 3, Target::Pad(3), None),
        touch(
            1,
            5,
            egui::TouchPhase::Start,
            Pos2::new(
                pitch_b.track.center().x,
                pitch_b.track.top() + pitch_b.track.height() * 0.25,
            ),
            None,
        ),
    ]);
    assert!((gui.rt.decks[0].pitch - 1.0).abs() < 1e-6);
    assert!(gui.rt.xfader.abs() < 1e-6 && gui.held(3));
    assert!(
        (gui.rt.decks[1].pitch - 0.75).abs() < 1e-6,
        "actual pitch {}, hit {:?}, track {:?}",
        gui.rt.decks[1].pitch,
        pitch_b.rect,
        pitch_b.track
    );
    gui.frame(vec![
        touch(
            1,
            1,
            egui::TouchPhase::Move,
            Pos2::new(pitch.track.center().x, pitch.track.bottom()),
            Some(0.2),
        ),
        touch(
            1,
            2,
            egui::TouchPhase::Move,
            Pos2::new(cross.track.right(), cross.track.center().y),
            None,
        ),
        gui.start(1, 4, Target::Pitch(0), None),
    ]);
    assert!(gui.rt.decks[0].pitch.abs() < 1e-6);
    assert!((gui.rt.xfader - 1.0).abs() < 1e-6 && gui.held(3));
    assert_eq!(gui.app.touch_input.rejected, 1);
    gui.frame(vec![
        release(1, 1, egui::TouchPhase::End),
        touch(
            1,
            2,
            egui::TouchPhase::Move,
            Pos2::new(cross.track.center().x, cross.track.center().y),
            None,
        ),
    ]);
    assert!(gui.rt.decks[0].pitch.abs() < 1e-6);
    assert!((gui.rt.xfader - 0.5).abs() < 1e-6 && gui.held(3));
    assert!((gui.rt.decks[1].pitch - 0.75).abs() < 1e-6);
    gui.frame(vec![
        release(1, 2, egui::TouchPhase::End),
        release(1, 3, egui::TouchPhase::End),
    ]);
    assert!(!gui.held(3));
    gui.frame(vec![touch(
        1,
        5,
        egui::TouchPhase::Move,
        Pos2::new(pitch_b.track.center().x, pitch_b.track.bottom()),
        None,
    )]);
    assert!(gui.rt.decks[1].pitch.abs() < 1e-6);
    gui.frame(vec![release(1, 5, egui::TouchPhase::End)]);
}

#[test]
fn native_short_taps_and_restarted_ids_keep_event_order_once_across_layout_passes() {
    let mut gui = Gui::new();
    gui.discard = true;
    gui.frame(vec![
        gui.start(1, 1, Target::Pad(0), None),
        release(1, 1, egui::TouchPhase::End),
    ]);
    assert_eq!(gui.rt.sampler_poly.note_on_events, 1);
    assert!(!gui.held(0) && gui.app.touch_input.contacts.is_empty());
    gui.frame(vec![gui.start(1, 1, Target::Pad(0), None)]);
    gui.frame(vec![
        release(1, 1, egui::TouchPhase::End),
        gui.start(1, 1, Target::Pad(0), Some(0.4)),
    ]);
    assert_eq!(gui.rt.sampler_poly.note_on_events, 3);
    assert!(gui.held(0));
    gui.frame(vec![gui.start(1, 1, Target::Pad(1), None)]);
    assert!(!gui.held(0) && gui.held(1));
    assert_eq!(gui.rt.sampler_poly.note_on_events, 4);
    gui.frame(vec![release(1, 1, egui::TouchPhase::End)]);
}

#[test]
fn native_focus_resize_dialog_and_recovery_cancel_without_resuming_old_contacts() {
    let mut gui = Gui::new();
    for mode in 0..5 {
        gui.frame(vec![gui.start(1, 1, Target::Pad(0), None)]);
        assert!(gui.held(0));
        match mode {
            0 => gui.focused = false,
            1 => gui.size.x += 100.0,
            2 => gui.app.touch_input.open = true,
            4 => gui.ctx.set_zoom_factor(1.25),
            _ => gui
                .app
                .engine
                .cmd
                .performance()
                .request_safety(crate::engine::performance::Safety::Stop),
        }
        gui.frame(vec![]);
        assert!(!gui.held(0));
        assert_eq!(gui.app.pad_inputs[0], 0);
        assert!(gui.app.touch_input.contacts.is_empty());
        gui.focused = true;
        gui.app.touch_input.open = false;
        if mode == 3 {
            gui.app
                .engine
                .cmd
                .performance()
                .acknowledge_inputs_released()
                .unwrap();
        }
        gui.frame(vec![]);
        gui.frame(vec![]);
        let pos = gui.hit(Target::Pad(0)).rect.center();
        gui.frame(vec![touch(1, 1, egui::TouchPhase::Move, pos, None)]);
        assert!(!gui.held(0), "only a new press may resume");
    }
    gui.click("Touch & pen");
    assert!(gui.app.touch_input.open);
}

#[test]
fn native_invalid_contacts_and_overflow_release_holds_and_never_resume() {
    let mut gui = Gui::new();
    for force in [f32::NAN, f32::INFINITY, -0.1, 1.1] {
        gui.frame(vec![gui.start(1, 1, Target::Pad(0), Some(force))]);
        assert!(!gui.held(0));
    }
    gui.frame(vec![gui.start(1, 1, Target::Pad(0), None)]);
    assert!(gui.held(0));
    let pos = gui.hit(Target::Pad(0)).rect.center();
    gui.frame(vec![touch(
        1,
        1,
        egui::TouchPhase::Move,
        pos,
        Some(f32::NAN),
    )]);
    assert!(!gui.held(0));
    gui.frame(vec![gui.start(1, 1, Target::Pad(0), None)]);
    gui.frame(
        (0..=MAX_EVENTS)
            .map(|_| touch(1, 1, egui::TouchPhase::Move, pos, None))
            .collect(),
    );
    assert!(!gui.held(0) && gui.app.touch_input.contacts.is_empty());
    gui.frame(vec![touch(1, 1, egui::TouchPhase::Move, pos, None)]);
    assert!(!gui.held(0));
    let events = (0..MAX_CONTACTS + 1)
        .map(|id| gui.start(1, id as u64, Target::Pad(0), None))
        .collect();
    gui.frame(events);
    assert_eq!(gui.app.touch_input.contacts.len(), MAX_CONTACTS);
    assert!(gui.held(0));
    gui.frame(
        (0..MAX_CONTACTS)
            .map(|id| release(1, id as u64, egui::TouchPhase::End))
            .collect(),
    );
    assert!(!gui.held(0));
    gui.frame(vec![touch(
        1,
        80,
        egui::TouchPhase::Start,
        Pos2::new(-100.0, -100.0),
        None,
    )]);
    assert!(gui.app.touch_input.contacts.is_empty());
}

#[test]
fn native_touch_and_keyboard_owners_release_only_their_own_pad_gate() {
    let mut gui = Gui::new();
    let key = |pressed| egui::Event::Key {
        key: egui::Key::Space,
        physical_key: None,
        pressed,
        repeat: false,
        modifiers: Default::default(),
    };
    let target = gui
        .nodes
        .iter()
        .find(|(_, node)| {
            node.label()
                .is_some_and(|label| label.starts_with("Sampler: Pad 1:"))
        })
        .expect("first synth pad")
        .0;
    gui.frame(vec![egui::Event::AccessKitActionRequest(ActionRequest {
        target,
        action: Action::Focus,
        data: None,
    })]);
    gui.frame(vec![key(true)]);
    assert!(gui.held(0));
    assert_ne!(gui.app.pad_inputs[0], 0);
    gui.frame(vec![gui.start(1, 1, Target::Pad(0), Some(0.2))]);
    gui.frame(vec![release(1, 1, egui::TouchPhase::End)]);
    assert!(gui.held(0));
    gui.frame(vec![key(false)]);
    assert!(!gui.held(0));
    gui.frame(vec![gui.start(1, 1, Target::Pad(0), None), key(true)]);
    assert_eq!(
        gui.app.touch_input.contacts.len(),
        1,
        "simultaneous touch and keyboard press must retain both owners"
    );
    gui.frame(vec![key(false)]);
    assert!(
        gui.held(0),
        "touch contact count {}, mask {}",
        gui.app.touch_input.contacts.len(),
        gui.app.pad_inputs[0]
    );
    gui.frame(vec![release(1, 1, egui::TouchPhase::End)]);
    assert!(!gui.held(0));
}

#[test]
fn native_rejected_touch_attack_never_keeps_an_owner_or_resumes_after_queue_drain() {
    let mut gui = Gui::new();
    while gui.app.engine.cmd.send(Command::Master(0.8)).is_ok() {}
    gui.frame(vec![gui.start(1, 1, Target::Pad(0), None)]);
    assert!(!gui.held(0));
    assert_eq!(gui.app.pad_inputs[0], 0);
    assert!(gui.app.touch_input.contacts.is_empty());
    assert!(gui.app.submission_error.get().is_some());
    for _ in 0..8 {
        gui.frame(vec![]);
    }
    gui.click("Dismiss");
    let pos = gui.hit(Target::Pad(0)).rect.center();
    gui.frame(vec![touch(1, 1, egui::TouchPhase::Move, pos, None)]);
    assert!(!gui.held(0));
    gui.frame(vec![gui.start(1, 2, Target::Pad(0), None)]);
    assert!(gui.held(0));
    gui.frame(vec![release(1, 2, egui::TouchPhase::End)]);
    assert!(!gui.held(0));
}

#[test]
fn native_pad_drag_does_not_scroll_and_preferences_cancel_the_hold() {
    let mut gui = Gui::new();
    let hit = gui.hit(Target::Pad(0));
    let pos = hit.rect.center();
    gui.frame(vec![
        gui.start(1, 1, Target::Pad(0), None),
        egui::Event::PointerMoved(pos),
        egui::Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed: true,
            modifiers: Default::default(),
        },
    ]);
    let away = pos - Vec2::new(0.0, 100.0);
    gui.frame(vec![
        touch(1, 1, egui::TouchPhase::Move, away, None),
        egui::Event::PointerMoved(away),
    ]);
    assert!(gui.held(0));
    assert_eq!(
        gui.hit(Target::Pad(0)).rect,
        hit.rect,
        "a performance drag must not scroll the panel"
    );
    gui.frame(vec![egui::Event::PointerButton {
        pos: away,
        button: PointerButton::Primary,
        pressed: false,
        modifiers: Default::default(),
    }]);
    gui.click("Preferences");
    assert!(gui.app.settings.open);
    assert!(!gui.held(0) && gui.app.touch_input.contacts.is_empty());
}
