use super::*;
use crate::engine::{session::{Action, Axis, Request, Structure}, test_alloc, surface_controls::TrackMix};

#[test]
fn fixed_send_identity_controls_undo_defaults_and_structural_recall() {
    let (engine, mut rt) = Engine::headless_for_test(48000, 144);
    let mix = TrackMix { sends: [0.37, 0.81], crossfader: 2 };
    for command in [Command::TrackSend {track:2,send:0,value:mix.sends[0]}, Command::TrackSend {track:2,send:1,value:mix.sends[1]}, Command::TrackCrossfader {track:2,assignment:2}] {
        assert_eq!(test_alloc::measure(||rt.apply(command)),test_alloc::Counts::default());
    }
    assert_eq!(rt.tracks[2].surface,mix);
    for expected in [TrackMix {sends:mix.sends,crossfader:0},TrackMix {sends:[mix.sends[0],0.],crossfader:0},TrackMix::default()] {
        rt.apply(Command::Undo);
        assert_eq!(rt.tracks[2].surface,expected);
    }
    for _ in 0..3 {rt.apply(Command::Redo);}
    assert_eq!(rt.tracks[2].surface,mix);
    let id=rt.session.tracks[2].id;
    let (request,ack)=Request::metadata(&rt.session,captured(&rt).checkpoint.epoch,Action::Move {axis:Axis::Track,id,position:0}).unwrap();
    rt.apply(Command::session_edit(request));
    assert_eq!(ack.state(),midi_edit::Outcome::Applied);
    assert_eq!(rt.session.track_order[0],2);
    assert_eq!(rt.tracks[2].surface,mix);
    let (request,ack)=Request::structural(captured(&rt),48000,Structure::Duplicate {axis:Axis::Track,id,name:"Return source copy".into(),position:1}).unwrap();
    assert_eq!(test_alloc::measure(||rt.apply(Command::session_edit(request))),test_alloc::Counts::default());
    assert_eq!(ack.state(),midi_edit::Outcome::Applied);
    assert_eq!(rt.tracks[8].surface,mix);
    assert_ne!(rt.session.tracks[8].id,id);
    let (request,_) = Request::metadata(&rt.session,captured(&rt).checkpoint.epoch,Action::Delete {axis:Axis::Track,id}).unwrap();
    rt.apply(Command::session_edit(request));
    let (request,ack)=Request::structural(captured(&rt),48000,Structure::Track {name:"Unrelated new track".into(),audio:true,position:0}).unwrap();
    rt.apply(Command::session_edit(request));
    assert_eq!(ack.state(),midi_edit::Outcome::Applied);
    let slot=usize::from(rt.session.track_order[0]);
    assert_ne!(rt.session.tracks[slot].id,id);
    assert_eq!(rt.tracks[slot].surface,TrackMix::default());
    let saved=captured(&rt);
    let reopened=Prepared::from_state(serde_json::from_slice(&serde_json::to_vec(&saved.state).unwrap()).unwrap(),saved.media,48000).unwrap();
    assert_eq!(reopened.rt.tracks[8].surface,mix);
    assert_eq!(reopened.rt.tracks[slot].surface,TrackMix::default());
    let mut blank=Prepared::empty(48000).unwrap();
    assert_eq!(test_alloc::measure(||blank.swap_into(&mut rt)),test_alloc::Counts::default());
    assert!(rt.tracks.iter().all(|track|track.surface.is_default()));
}

#[test]
fn fixed_send_legacy_defaults_invalid_packets_and_saved_values_are_atomic() {
    let mut original=rt();
    let saved=captured(&original);
    let raw=serde_json::to_value(&saved.state).unwrap();
    assert!(raw["tracks"].as_array().unwrap().iter().all(|t|t.get("surface").is_none()));
    let legacy: State=serde_json::from_value(raw.clone()).unwrap();
    assert!(legacy.tracks.iter().all(|t|t.surface.is_default()));
    for command in [Command::TrackSend {track:0,send:2,value:0.5},Command::TrackSend {track:0,send:0,value:f32::NAN},Command::TrackCrossfader {track:0,assignment:3},Command::TrackSend {track:127,send:0,value:1.0}] {
        assert_eq!(test_alloc::measure(||original.apply(command)),test_alloc::Counts::default());
        assert!(original.tracks.iter().all(|t|t.surface.is_default()));
    }
    for surface in [serde_json::json!({"sends":[-0.1,0.0],"crossfader":0}),serde_json::json!({"sends":[0.0,1.1],"crossfader":0}),serde_json::json!({"sends":[0.0,0.0],"crossfader":3})] {
        let mut invalid=raw.clone();invalid["tracks"][0]["surface"]=surface;
        let state:State=serde_json::from_value(invalid).unwrap();
        assert!(Prepared::from_state(state,saved.media.clone(),48000).is_err());
    }
}

#[test]
fn fixed_send_queued_controls_never_rebind_after_track_identity_reuse(){
    let(engine,mut rt)=Engine::headless_for_test(48000,144);
    let id=rt.session.tracks[2].id;
    engine.send(Command::TrackSend{track:2,send:0,value:0.8}).unwrap();
    engine.send(Command::TrackCrossfader{track:2,assignment:1}).unwrap();
    let(request,_)=Request::metadata(&rt.session,captured(&rt).checkpoint.epoch,Action::Delete{axis:Axis::Track,id}).unwrap();rt.apply(Command::session_edit(request));
    let(request,_)=Request::structural(captured(&rt),48000,Structure::Track{name:"New stable target".into(),audio:true,position:2}).unwrap();rt.apply(Command::session_edit(request));
    rt.process(&mut []);
    assert_ne!(rt.session.tracks[2].id,id);assert_eq!(rt.tracks[2].surface,TrackMix::default());
}

