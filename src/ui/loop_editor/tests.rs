use super::*;
use crate::ui::test_support::Fixture;
use egui::accesskit::{Action, ActionData, ActionRequest};

struct Gui {
    fixture: Fixture,
    ctx: egui::Context,
    time: f64,
}
impl Gui {
    fn new() -> Self {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut gui = Self {
            fixture: Fixture::new(256),
            ctx,
            time: 0.0,
        };
        for deck in 0..2 {
            gui.fixture.rt.apply(Command::DeckSeek { deck, frac: 0.2 });
        }
        gui.fixture.rt.publish_for_test();
        gui.frame(vec![]);
        gui.frame(vec![]);
        gui
    }
    fn frame(&mut self, events: Vec<egui::Event>) -> egui::FullOutput {
        self.time += 0.02;
        let output = self.ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1800.0, 1600.0))),
                time: Some(self.time),
                focused: true,
                events,
                ..Default::default()
            },
            |ctx| self.fixture.app.update_frame(ctx),
        );
        self.fixture.rt.process(&mut []);
        self.fixture.rt.publish_for_test();
        output
    }
    fn click(&mut self, label: &str) {
        let nodes = self
            .frame(vec![])
            .platform_output
            .accesskit_update
            .unwrap()
            .nodes;
        let target = nodes
            .iter()
            .find(|(_, n)| n.label() == Some(label))
            .map(|(id, _)| *id)
            .unwrap_or_else(|| {
                panic!(
                    "Missing {label}: {:?}",
                    nodes
                        .iter()
                        .filter_map(|(_, n)| n.label())
                        .collect::<Vec<_>>()
                )
            });
        self.frame(vec![egui::Event::AccessKitActionRequest(ActionRequest {
            target,
            action: Action::Click,
            data: None,
        })]);
        self.frame(vec![]);
    }
    fn value(&mut self, label: &str, value: f64) {
        let nodes = self
            .frame(vec![])
            .platform_output
            .accesskit_update
            .unwrap()
            .nodes;
        let target = nodes
            .iter()
            .find(|(_, node)| node.label() == Some(label))
            .unwrap()
            .0;
        self.frame(vec![egui::Event::AccessKitActionRequest(ActionRequest {
            target,
            action: Action::SetValue,
            data: Some(ActionData::NumericValue(value)),
        })]);
        self.frame(vec![]);
    }
}

#[test]
fn native_loop_lengths_moves_and_one_frame_edges_apply_to_the_named_deck_without_starting() {
    let mut gui = Gui::new();
    let positions = gui.fixture.rt.decks.each_ref().map(|d| d.pos);
    gui.click("Deck A: Loop editor");
    gui.click("Deck A: Set loop length");
    let initial = gui.fixture.app.engine.snapshot().decks[0].clone();
    assert!(initial.loop_len > 64.0);
    assert!(!initial.loop_on && !initial.playing);
    gui.click("Deck A: Loop start seconds one frame later");
    let edited = gui.fixture.app.engine.snapshot().decks[0].clone();
    assert!((edited.loop_start - initial.loop_start - 1.0).abs() < 1e-6);
    assert!(
        (edited.loop_start + edited.loop_len - initial.loop_start - initial.loop_len).abs() < 1e-6
    );
    gui.click("Deck A: Loop start seconds one frame earlier");
    gui.click("Deck A: Move loop forward");
    let moved = gui.fixture.app.engine.snapshot().decks[0].clone();
    assert!(moved.loop_start > initial.loop_start);
    gui.click("Deck A: Move loop backward");
    assert!(
        (gui.fixture.app.engine.snapshot().decks[0].loop_start - initial.loop_start).abs() < 1e-6
    );
    let edited_start = gui.fixture.app.engine.snapshot().decks[0].loop_start;
    let rate = gui.fixture.app.engine.snapshot().decks[0].source_sample_rate;
    gui.value(
        "Deck A: Loop start seconds",
        edited_start / f64::from(rate) + 0.001,
    );
    assert!(
        (gui.fixture.app.engine.snapshot().decks[0].loop_start
            - edited_start
            - f64::from(rate) * 0.001)
            .abs()
            < 1e-6
    );
    gui.click("Deck A: Enable loop");
    assert!(gui.fixture.app.engine.snapshot().decks[0].loop_on);
    gui.click("Deck A: Disable loop");
    assert!(!gui.fixture.app.engine.snapshot().decks[0].loop_on);
    gui.click("Deck A: Loop length");
    gui.click("0.125 beats");
    gui.click("Deck A: Set loop length");
    let short = gui.fixture.app.engine.snapshot().decks[0].clone();
    assert!(short.loop_len < initial.loop_len && short.loop_len >= 64.0);
    gui.click("Deck B: Loop editor");
    gui.click("Deck B: Set loop length");
    assert!(gui.fixture.app.engine.snapshot().decks[1].loop_len > short.loop_len);
    assert_eq!(gui.fixture.rt.decks.each_ref().map(|d| d.pos), positions);
    assert!(gui.fixture.rt.decks.iter().all(|d| !d.playing));
}

#[test]
fn native_loop_edits_retain_waveform_markers_and_refuse_a_queued_edit_after_media_replacement() {
    let mut gui = Gui::new();
    gui.click("Deck A: Loop editor");
    gui.click("Deck A: Set loop length");
    let snap = gui.fixture.app.engine.snapshot().decks[0].clone();
    let output = gui.frame(vec![]);
    for label in ["loop in", "loop out"] {
        assert!(output.shapes.iter().any(|shape| matches!(&shape.shape,egui::epaint::Shape::Text(text) if text.galley.text()==label)),"missing waveform {label}");
    }
    let old_audio = gui.fixture.rt.decks[0].audio.clone().unwrap();
    gui.fixture.app.send(Command::DeckControl {
        source: 0,
        deck: 0,
        control: Control::LoopMove {
            media_key: snap.media_key,
            beats: 4.0,
        },
    });
    gui.fixture.rt.apply(Command::DeckAudio {
        deck: 0,
        audio: old_audio,
    });
    gui.fixture.rt.process(&mut []);
    assert_eq!(gui.fixture.rt.decks[0].loop_len, 0.0);
    assert!(!gui.fixture.rt.decks[0].playing);
    gui.fixture.rt.publish_for_test();
    gui.frame(vec![]);
    assert_ne!(
        gui.fixture.app.engine.snapshot().decks[0].media_key,
        snap.media_key
    );
}
impl Gui {
    fn text(&mut self,label:&str,value:&str) {
        let target=self.frame(vec![]).platform_output.accesskit_update.unwrap().nodes.into_iter().find(|(_,node)|node.label()==Some(label)).unwrap().0;
        self.frame(vec![egui::Event::AccessKitActionRequest(ActionRequest {target,action:Action::Focus,data:None})]);
        for pressed in [true,false] {self.frame(vec![egui::Event::Key {key:Key::A,physical_key:None,pressed,repeat:false,modifiers:egui::Modifiers {ctrl:true,command:true,..Default::default()}}]);}
        self.frame(vec![egui::Event::Text(value.into())]);self.frame(vec![]);
    }
}
#[test]
fn native_saved_loop_save_rename_reorder_recall_activate_delete_and_undo_use_the_actual_deck_handlers() {
    let mut gui=Gui::new();gui.click("Deck A: Loop editor");gui.click("Deck A: Set loop length");
    gui.click("Deck A: Save saved loop 3");
    let slot=gui.fixture.app.engine.snapshot().decks[0].saved_loops.slots[2].unwrap();
    gui.text("Deck A: Saved loop 3 name","Drop / 東京 🎵");gui.click("Deck A: Rename saved loop 3");
    assert_eq!(gui.fixture.app.engine.snapshot().decks[0].saved_loops.slots[2].unwrap().style.name.as_str(),"Drop / 東京 🎵");
    gui.click("Deck A: Move up saved loop 3");
    assert_eq!(gui.fixture.app.engine.snapshot().decks[0].saved_loops.order,[1,3,2,4,5,6,7,8]);
    gui.click("Deck A: Recall saved loop 3");assert!(!gui.fixture.rt.decks[0].loop_on && !gui.fixture.rt.decks[0].playing);
    gui.click("Deck A: Activate saved loop 3");assert!(gui.fixture.rt.decks[0].loop_on && !gui.fixture.rt.decks[0].playing);
    let rate=f64::from(gui.fixture.rt.decks[0].audio.as_ref().unwrap().sr);
    assert!((gui.fixture.rt.decks[0].pos/rate-slot.start).abs()<1e-9);
    gui.fixture.rt.clear_undo_for_test();
    gui.click("Deck A: Delete saved loop 3");assert!(gui.fixture.app.engine.snapshot().decks[0].saved_loops.slots[2].is_none());
    gui.fixture.app.send(Command::Undo);gui.frame(vec![]);gui.frame(vec![]);
    assert_eq!(gui.fixture.app.engine.snapshot().decks[0].saved_loops.slots[2].unwrap().style.name.as_str(),"Drop / 東京 🎵");
    assert!(gui.fixture.rt.decks[0].loop_on);
    assert!(gui.fixture.app.engine.snapshot().decks[1].saved_loops.is_default());
}
