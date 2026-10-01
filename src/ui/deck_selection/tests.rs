use super::*;
use crate::ui::test_support::{label_center, Fixture};

struct Gui {
    f: Fixture,
    ctx: egui::Context,
    time: f64,
    cover: Option<Pos2>,
}
impl Gui {
    fn new(capacity: usize) -> Self {
        let mut result = Self { f: Fixture::new(capacity), ctx: egui::Context::default(), time: 0.0, cover: None };
        result.frame(vec![]);
        result.frame(vec![]);
        result
    }
    fn frame(&mut self, events: Vec<egui::Event>) -> egui::FullOutput {
        self.time += 0.02;
        self.ctx.run(egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1440.0, 1000.0))),
            time: Some(self.time), events, ..Default::default()
        }, |ctx| {
            if let Some(center) = self.cover {
                egui::Area::new(egui::Id::new("deck-test-cover"))
                    .order(egui::Order::Foreground)
                    .fixed_pos(center - Vec2::splat(25.0))
                    .show(ctx, |ui| { ui.allocate_exact_size(Vec2::splat(50.0), Sense::click()); });
            }
            self.f.app.update_frame(ctx)
        })
    }
    fn pointer(&mut self, pos: Pos2, pressed: bool) {
        self.frame(vec![egui::Event::PointerMoved(pos), egui::Event::PointerButton {
            pos, button: PointerButton::Primary, pressed, modifiers: Default::default(),
        }]);
    }
    fn click(&mut self, pos: Pos2) {
        self.pointer(pos, true);
        self.pointer(pos, false);
    }
    fn double_row(&mut self, title: &str) {
        let out = self.frame(vec![]);
        let pos = label_center(&out, title);
        self.click(pos);
        self.click(pos);
    }
    fn apply(&mut self) {
        self.f.rt.process(&mut []);
        self.f.rt.publish_for_test();
    }
    fn platters(&mut self) -> [Pos2; 2] {
        let output = self.frame(vec![]);
        let mut centers: Vec<_> = output.shapes.iter().filter_map(|shape| match &shape.shape {
            egui::epaint::Shape::Circle(circle) if circle.radius > 50.0 && circle.stroke.width == 2.0 => Some(circle.center),
            _ => None,
        }).collect();
        centers.sort_by(|a, b| a.x.total_cmp(&b.x));
        centers.dedup();
        assert_eq!(centers.len(), 2);
        [centers[0], centers[1]]
    }
}

#[test]
fn pointer_deck_selection_routes_double_click_before_snapshot_ack_to_only_that_deck() {
    for (deck, title) in [(1, "Drums (session)"), (0, "Harmony (session)")] {
        let mut g = Gui::new(128);
        if deck == 0 {
            g.f.rt.apply(Command::SelectDeck(1));
            g.f.rt.publish_for_test();
            g.frame(vec![]);
        }
        let other = g.f.rt.decks[1-deck].audio.clone().unwrap();
        let pos = g.platters()[deck];
        g.click(pos);
        assert_eq!(g.f.app.load_target(), deck);
        assert_ne!(g.f.rt.selected_deck, deck, "test must retain a lagging renderer");
        let out = g.frame(vec![]);
        label_center(&out, "load target (queued)");
        g.double_row(title);
        g.apply();
        assert_eq!(g.f.rt.selected_deck, deck);
        assert_eq!(g.f.rt.decks[deck].title, title);
        assert!(Arc::ptr_eq(&other, g.f.rt.decks[1-deck].audio.as_ref().unwrap()));
        assert!(!g.f.rt.decks[deck].playing);
        let out = g.frame(vec![]);
        label_center(&out, "load target");
        assert!(out.shapes.iter().any(|shape| matches!(&shape.shape,
            egui::epaint::Shape::Rect(rect) if rect.stroke.width == 1.5 && rect.stroke.color == g.f.app.theme.accent)));
        assert!(g.f.decoder_jobs.try_recv().is_err());
    }
}

#[test]
fn visible_selectors_control_f_and_explicit_buttons_keep_their_own_destinations() {
    for (selected, explicit) in [(0, 1), (1, 0)] {
        let mut g = Gui::new(128);
        g.f.app.lib_filter = "Harmony".into();
        let out = g.frame(vec![]);
        let label = if selected == 0 { "Deck A" } else { "Deck B" };
        g.click(label_center(&out, label));
        assert_eq!(g.f.app.load_target(), selected);
        let out = g.frame(vec![]);
        g.click(label_center(&out, if explicit == 0 { "→ A" } else { "→ B" }));
        g.apply();
        assert!(g.f.rt.decks[explicit].title.contains("Harmony"));
        assert_eq!(g.f.rt.selected_deck, selected);
        let other = g.f.rt.decks[1-selected].audio.clone().unwrap();
        g.f.app.lib_filter = "Drums".into();
        g.frame(vec![]);
        for pressed in [true, false] {
            g.frame(vec![egui::Event::Key { key: Key::F, physical_key: Some(Key::F),
                pressed, repeat: false, modifiers: Default::default() }]);
        }
        g.apply();
        assert!(g.f.rt.decks[selected].title.contains("Drums"));
        assert!(Arc::ptr_eq(&other, g.f.rt.decks[1-selected].audio.as_ref().unwrap()));
    }
}

#[test]
fn acknowledged_selection_revision_handles_a_b_a_and_later_renderer_selection() {
    let mut g = Gui::new(128);
    g.f.app.select_deck(1);
    g.f.app.select_deck(0);
    assert_eq!(g.f.app.load_target(), 0);
    let first = g.f.rt.cmd_rx.try_recv().unwrap();
    assert!(matches!(first, Command::SelectDeckRequested { deck: 1, request: 1 }));
    let counts = crate::engine::test_alloc::measure(|| g.f.rt.apply(first));
    assert_eq!((counts.allocations, counts.frees), (0, 0));
    g.f.rt.publish_for_test();
    g.frame(vec![]);
    assert_eq!(g.f.app.snap.selected_deck, 1);
    assert_eq!(g.f.app.snap.selected_deck_request, 1);
    assert_eq!(g.f.app.load_target(), 0, "old A snapshot and intermediate B cannot acknowledge requested A");
    let second = g.f.rt.cmd_rx.try_recv().unwrap();
    assert!(matches!(second, Command::SelectDeckRequested { deck: 0, request: 2 }));
    g.f.rt.apply(second);
    g.f.rt.publish_for_test();
    g.frame(vec![]);
    assert_eq!(g.f.app.snap.selected_deck_request, 2);
    assert_eq!(g.f.app.load_target(), 0);
    g.f.rt.apply(Command::SelectDeck(1));
    g.f.rt.publish_for_test();
    g.frame(vec![]);
    assert_eq!(g.f.app.load_target(), 1, "after acknowledgment renderer order wins");
}

#[test]
fn rejected_selection_hover_and_release_over_another_deck_cannot_retarget_loads() {
    let mut full = Gui::new(16);
    while full.f.app.engine.send(Command::Master(0.5)).is_ok() {}
    full.f.app.select_deck(1);
    assert_eq!(full.f.app.load_target(), 0);
    assert!(full.f.app.deck_selection.pending.is_none());
    assert!(full.f.app.submission_error.get().is_some());

    let mut g = Gui::new(128);
    let [a, b] = g.platters();
    g.frame(vec![egui::Event::PointerMoved(b)]);
    assert_eq!(g.f.app.load_target(), 0);
    g.pointer(b, true);
    assert_eq!(g.f.app.load_target(), 1);
    g.pointer(a, false);
    assert_eq!(g.f.app.load_target(), 1);
    g.apply();
    assert_eq!(g.f.rt.selected_deck, 1);
}

#[test]
fn actual_waveform_presses_and_deck_control_presses_use_the_same_selection_policy() {
    let mut g = Gui::new(128);
    let output = g.frame(vec![]);
    let mut waves: Vec<_> = output.shapes.iter().filter_map(|shape| match &shape.shape {
        egui::epaint::Shape::Rect(rect) if rect.fill == g.f.app.theme.bg_darker
            && rect.rect.width() > 55.0 && rect.rect.height() > 120.0 => Some(rect.rect.center()),
        _ => None,
    }).collect();
    waves.sort_by(|a, b| a.x.total_cmp(&b.x));
    assert_eq!(waves.len(), 2);
    for deck in [1, 0] {
        g.click(waves[deck]);
        assert_eq!(g.f.app.load_target(), deck);
        g.apply();
    }
    let output = g.frame(vec![]);
    let bass_b = output.shapes.iter().filter_map(|shape| match &shape.shape {
        egui::epaint::Shape::Text(text) if text.galley.text() == "b" => Some(text.visual_bounding_rect().center()),
        _ => None,
    }).max_by(|a,b| a.x.total_cmp(&b.x)).unwrap();
    let cut = g.f.rt.decks[1].eq_cut[0];
    assert!(bass_b.x < 1440.0);
    let knob = output.shapes.iter().filter_map(|shape| match &shape.shape {
        egui::epaint::Shape::Circle(circle) if circle.radius > 8.0 && circle.radius < 20.0
            && circle.center.y < bass_b.y && (circle.center.x - bass_b.x).abs() < 20.0 => Some(circle.center),
        _ => None,
    }).min_by(|a,b| a.distance_sq(bass_b).total_cmp(&b.distance_sq(bass_b))).unwrap();
    g.click(knob);
    assert_eq!(g.f.app.load_target(), 1);
    g.apply();
    assert_eq!(g.f.rt.selected_deck, 1);
    assert_ne!(g.f.rt.decks[1].eq_cut[0], cut, "selection must preserve the control's own action");
}

#[test]
fn same_frame_pointer_events_follow_press_positions_and_order_not_final_hover_or_ui_order() {
    let mut g = Gui::new(128);
    let [a, b] = g.platters();
    let event = |pos, pressed| egui::Event::PointerButton {
        pos, button: PointerButton::Primary, pressed, modifiers: Default::default(),
    };
    g.frame(vec![egui::Event::PointerMoved(b), event(b, true),
        egui::Event::PointerMoved(a), event(a, false)]);
    assert_eq!(g.f.app.load_target(), 1, "a quick tap must select its press location");
    g.apply();
    g.frame(vec![event(b, true), event(b, false), event(a, true), event(a, false)]);
    assert_eq!(g.f.app.load_target(), 0, "later left-deck press wins despite being visited first");
    g.apply();
    assert_eq!(g.f.rt.selected_deck, 0);
}

#[test]
fn foreground_dialog_occlusion_prevents_a_deck_press_from_changing_the_load_target() {
    let mut g = Gui::new(128);
    let [a, b] = g.platters();
    g.cover = Some(b);
    g.frame(vec![]);
    g.frame(vec![]);
    g.pointer(b, true);
    g.pointer(a, false);
    assert_eq!(g.f.app.load_target(), 0);
    assert!(g.f.app.deck_selection.pending.is_none());
    g.apply();
    assert_eq!(g.f.rt.selected_deck, 0);
}

#[test]
fn later_covered_press_does_not_hide_an_earlier_visible_deck_press_in_the_same_frame() {
    let mut g = Gui::new(128);
    let [a, b] = g.platters();
    g.cover = Some(a);
    g.frame(vec![]);
    g.frame(vec![]);
    let visible_a = a + Vec2::new(70.0, 0.0);
    let event = |pos, pressed| egui::Event::PointerButton {
        pos, button: PointerButton::Primary, pressed, modifiers: Default::default(),
    };
    g.frame(vec![event(b, true), event(b, false), event(visible_a, true),
        event(visible_a, false), event(a, true), event(a, false)]);
    assert_eq!(g.f.app.load_target(), 0, "last visible eligible press was A, not earlier B");
    g.apply();
    assert_eq!(g.f.rt.selected_deck, 0);
}
