use super::*;
use crate::ui::test_support::Fixture;
use egui::accesskit::{Action,ActionData,ActionRequest,Node,NodeId};
struct Gui {fixture:Fixture,ctx:egui::Context,time:f64,nodes:Vec<(NodeId,Node)>,focused:bool}
impl Gui {
    fn new()->Self {let ctx=egui::Context::default();ctx.enable_accesskit();let mut g=Self {fixture:Fixture::new(128),ctx,time:0.0,nodes:Vec::new(),focused:true};g.frame(vec![]);g.frame(vec![]);g}
    fn frame(&mut self,events:Vec<egui::Event>) {self.time+=0.02;let o=self.ctx.run(egui::RawInput {screen_rect:Some(Rect::from_min_size(Pos2::ZERO,Vec2::new(1800.0,1600.0))),time:Some(self.time),focused:self.focused,events,..Default::default()},|ctx|self.fixture.app.update_frame(ctx));self.nodes=o.platform_output.accesskit_update.unwrap().nodes;self.fixture.rt.process(&mut []);self.fixture.rt.publish_for_test();}
    fn target(&self,label:&str)->NodeId {self.nodes.iter().find(|(_,n)|n.label()==Some(label)).unwrap_or_else(||panic!("missing {label}: {:?}",self.nodes.iter().filter_map(|(_,n)|n.label()).collect::<Vec<_>>())).0}
    fn access(&mut self,label:&str,action:Action,data:Option<ActionData>) {self.frame(vec![egui::Event::AccessKitActionRequest(ActionRequest {target:self.target(label),action,data})]);self.frame(vec![]);}
    fn bend(&self,deck:usize)->i8 {(self.fixture.app.engine.snapshot().decks[deck].controls.bend / 0.08).round() as i8}
}
fn key(key:Key,pressed:bool)->egui::Event {egui::Event::Key {key,physical_key:Some(key),pressed,repeat:false,modifiers:egui::Modifiers::NONE}}
#[test]
fn native_bend_pointer_keyboard_assistive_and_focus_release_return_to_the_base_without_starting_playback() {
    let mut g=Gui::new();let pitch=g.fixture.rt.decks[0].pitch;
    g.access("Deck A: Bend up",Action::CustomAction,Some(ActionData::CustomAction(0)));assert_eq!(g.bend(0),1);assert_eq!(g.bend(1),0);
    g.access("Deck A: Bend up",Action::CustomAction,Some(ActionData::CustomAction(1)));assert_eq!(g.bend(0),0);
    g.access("Deck A: Bend down",Action::Focus,None);g.frame(vec![key(Key::Space,true)]);assert_eq!(g.bend(0),-1);
    g.frame(vec![key(Key::Space,false)]);assert_eq!(g.bend(0),0);
    g.access("Deck A: Bend up",Action::Focus,None);g.frame(vec![key(Key::Enter,true)]);assert_eq!(g.bend(0),1);
    g.focused=false;g.frame(vec![]);assert_eq!(g.bend(0),0);g.focused=true;g.frame(vec![key(Key::Enter,false)]);
    let node=g.nodes.iter().find(|(_,n)|n.label()==Some("Deck A: Bend down")).unwrap().1.bounds().unwrap();let pos=Pos2::new(((node.x0+node.x1)*0.5) as f32,((node.y0+node.y1)*0.5) as f32);
    for on in [true,false] {g.frame(vec![egui::Event::PointerMoved(pos),egui::Event::PointerButton {pos,button:PointerButton::Primary,pressed:on,modifiers:egui::Modifiers::NONE}]);assert_eq!(g.bend(0),if on {-1} else {0});}
    assert_eq!(g.fixture.rt.decks[0].pitch,pitch);assert!(!g.fixture.rt.decks[0].playing && !g.fixture.rt.decks[1].playing);
    g.access("Deck A: Bend up",Action::Click,None);assert_eq!(g.bend(0),1);
    g.fixture.app.release_workspace_panel(crate::preferences::workspaces::Panel::Decks);g.fixture.rt.process(&mut []);g.fixture.rt.publish_for_test();assert_eq!(g.bend(0),0);
    g.frame(vec![]);g.frame(vec![]);
    g.access("Deck A: Bend up",Action::Click,None);assert_eq!(g.bend(0),1);
    g.fixture.rt.apply(Command::DeckControl {source:887,deck:0,control:Control::Hold {button:Button::BendUp,on:true}});
    g.focused=false;g.frame(vec![]);assert_eq!(g.bend(0),1);assert!(!g.fixture.app.pitch_inputs.held(0,1));
    g.focused=true;g.frame(vec![]);g.fixture.rt.apply(Command::DeckControl {source:887,deck:0,control:Control::Hold {button:Button::BendUp,on:false}});g.fixture.rt.publish_for_test();assert_eq!(g.bend(0),0);
    g.access("Deck A: Bend up",Action::Click,None);assert_eq!(g.bend(0),1);
    g.fixture.app.library_playlist.open=true;g.frame(vec![]);assert_eq!(g.bend(0),0);assert!(!g.fixture.app.pitch_inputs.held(0,1));
    g.fixture.app.library_playlist.open=false;g.frame(vec![]);g.frame(vec![]);
    g.access("Deck A: Bend down",Action::Click,None);assert_eq!(g.bend(0),-1);
    let audio=g.fixture.rt.decks[0].audio.clone().unwrap();g.fixture.rt.apply(Command::DeckAudio {deck:0,audio});g.fixture.rt.publish_for_test();g.frame(vec![]);assert_eq!(g.bend(0),0);assert!(!g.fixture.app.pitch_inputs.held(0,0));
    g.access("Deck B: Bend up",Action::Click,None);assert_eq!(g.bend(1),1);
    g.fixture.app.engine.send(Command::SafetyStop(crate::engine::performance::Safety::Stop)).unwrap();g.fixture.rt.process(&mut []);g.fixture.rt.publish_for_test();g.frame(vec![]);assert_eq!(g.bend(1),0);assert!(!g.fixture.app.pitch_inputs.held(1,1));
    g.fixture.app.engine.send(Command::RecoverPerformance).unwrap();g.fixture.rt.process(&mut []);g.fixture.rt.publish_for_test();g.frame(vec![]);assert_eq!(g.bend(1),0);
}
#[test]
fn native_pitch_target_and_separate_bpm_readouts_follow_the_actual_snapshot_and_sync_settings() {
    let mut g=Gui::new();g.fixture.rt.apply(Command::DeckPitch {deck:0,value:0.75});g.fixture.rt.publish_for_test();g.frame(vec![]);
    g.fixture.rt.apply(Command::MidiPitch(crate::engine::pitch_pickup::Input {source:2190,context:[0;3],channel:0,binding:crate::engine::midi::Binding {kind:crate::engine::midi::MsgKind::Cc,ch:0,data:7,action:crate::engine::midi::Action::DeckPitch,deck:0,extra:0,relative:None,controls:None,pair_order:None},value:0.1}));
    g.fixture.rt.publish_for_test();g.frame(vec![]);
    let pitch=g.nodes.iter().find(|(_,n)|n.label()==Some("Deck A: Pitch")).unwrap().1.clone();
    assert!(pitch.description().unwrap().contains("Pickup +4.00% · move up"));
    g.access("Deck A: Sync settings",Action::Click,None);
    let o=g.ctx.run(egui::RawInput {screen_rect:Some(Rect::from_min_size(Pos2::ZERO,Vec2::new(1800.0,1600.0))),time:Some(g.time+0.02),focused:true,..Default::default()},|ctx|g.fixture.app.update_frame(ctx));
    let texts=o.shapes.iter().filter_map(|s|match &s.shape {egui::Shape::Text(t)=>Some(t.galley.text()),_=>None}).collect::<Vec<_>>();
    assert!(texts.iter().any(|s|s.starts_with("Original ") && s.contains("local grid")),"{texts:?}");
    assert!(texts.iter().any(|s|s.starts_with("Effective ") && s.contains("pitch range ±8%")),"{texts:?}");
    let bounds=pitch.bounds().unwrap();
    assert!(o.shapes.iter().any(|s|match &s.shape {egui::Shape::LineSegment {points,stroke}=>stroke.color==g.fixture.app.theme.orange && stroke.width==2.0 && points.iter().all(|p|f64::from(p.x)>=bounds.x0 && f64::from(p.x)<=bounds.x1 && f64::from(p.y)>=bounds.y0 && f64::from(p.y)<=bounds.y1),_=>false}),"actual pickup marker missing");
    let status=crate::engine::pitch_pickup::Status {physical:Some(0.1),target:0.75,acquired:false,sync:false};
    let text=pickup_label(status,8.0);assert!(text.contains("Pickup +4.00%") && text.contains("move up"));
    assert!(pickup_label(crate::engine::pitch_pickup::Status {sync:true,..status},8.0).contains("choose Off"));
}
