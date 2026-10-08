use super::{model::*,plugins::{Instance,Parameter},prepared::Prepared};
use crate::{engine::{self,Command,Engine,RtEngine,project},plugin_host::{self,Class,Saved,Request,Response}};
use std::{path::PathBuf,sync::{Arc,atomic::AtomicBool},time::{Duration,Instant}};

pub(crate) fn fixture(instrument:bool)->(Class,Saved) {
    let root=PathBuf::from(std::env::var_os("OMATAINER_VST3_FIXTURES").expect("Native fixture directory is required"));
    probe(root.join("Contract.vst3"),instrument)
}
pub(crate) fn probe(bundle:PathBuf,instrument:bool)->(Class,Saved){
    let exe=PathBuf::from(std::env::var_os("OMATAINER_TEST_BIN").expect("Fresh native app executable is required"));
    let mut worker=plugin_host::process::Process::start(&exe).unwrap();
    let cancel=AtomicBool::new(false);
    let binary=match worker.exchange(&Request::Inspect{path:bundle},&cancel,Duration::from_secs(10)).unwrap(){Response::Identity{binary}=>binary,_=>panic!("Missing fixture identity")};
    let classes=match worker.exchange(&Request::Probe{binary:binary.clone()},&cancel,Duration::from_secs(10)).unwrap(){Response::Classes{classes}=>classes,_=>panic!("Missing fixture classes")};
    let class=classes.into_iter().find(|c| c.info.category.contains("Instrument")==instrument).expect("Both original fixture classes must be present");
    let saved=Saved{schema:1,binary,class_id:class.info.uid.clone(),plugin_version:class.info.version.clone(),state_codec:"vst3-host-0.9-state".into(),state:vec![]};
    (class,saved)
}
fn instance(class:&Class,saved:Saved,track:Option<engine::session::Id>)->Instance{
    Instance{id:2,name:"Contract".into(),saved,inputs:class.layout.inputs.iter().map(|b|b.channel_count as u8).collect(),outputs:class.layout.outputs.iter().map(|b|b.channel_count as u8).collect(),midi_track:track,scene_track:None,instrument:class.info.category.contains("Instrument"),bypass:false,latency:class.latency+768,parameters:vec![],automation:vec![],unavailable:None}
}
fn link(source:Group,destination:Group,map:&[(u8,u8,f32)])->Connection{
    Connection{source:Source{group:source,tap:Tap::PostMixer},destination,map:map.iter().map(|(source,destination,gain)|ChannelMap{source:*source,destination:*destination,gain:*gain}).collect()}
}
fn effect(class:&Class,saved:Saved,rt:&RtEngine)->Model{
    let mut model=Model::default();model.version=3;model.next_id=6;model.plugins.push(instance(class,saved,None));
    model.ports.push(Port{id:3,alias:"Contract input".into(),direction:Direction::Input,channels:vec![0,1,2]});
    model.ports.push(Port{id:4,alias:"Context output".into(),direction:Direction::Output,channels:vec![2]});
    model.connections=vec![link(Group::Input(3),Group::Plugin(2),&[(0,0,1.),(1,1,1.),(2,2,1.)]),link(Group::Plugin(2),Group::Output(1),&[(0,0,1.),(1,1,1.)]),link(Group::Plugin(2),Group::Output(4),&[(2,0,1.)])];
    model.order(&rt.session).unwrap();model
}
fn capture(engine:&Engine,rt:&mut RtEngine)->project::Captured{
    let handle=engine.project.clone();let task=std::thread::spawn(move||handle.capture(&AtomicBool::new(false)));
    let deadline=Instant::now()+Duration::from_secs(15);
    while !task.is_finished(){rt.process_interleaved(&mut[0.;512],2);assert!(Instant::now()<deadline);std::thread::sleep(Duration::from_millis(1));}
    task.join().unwrap().unwrap()
}

#[test]
#[ignore="requires the freshly compiled native worker and original SDK contract fixture"]
fn native_graph_buses_context_bypass_latency_and_callback_storage(){
    let (class,saved)=fixture(false);assert_eq!(class.layout.inputs.iter().map(|b|b.channel_count).collect::<Vec<_>>(),[2,1]);assert_eq!(class.layout.outputs.iter().map(|b|b.channel_count).collect::<Vec<_>>(),[2,1]);
    let (_,mut rt)=Engine::headless_for_test(48000,256);let model=effect(&class,saved.clone(),&rt);
    let mut graph=Prepared::at_rate(Arc::new(model.clone()),&rt.session,48000).unwrap();assert!(graph.plugins[0].error.is_none(),"{:?}",graph.plugins[0].error);graph.offline=true;
    assert_eq!(graph.output_delay(1),768);
    rt.playing=true;rt.bpm=120.;
    for frame in 0..2048u64{
        rt.timeline_frames=frame+1;rt.beat=(frame+1)as f64/24000.;rt.midi_beat=rt.beat;rt.midi_beat_reference=rt.beat;rt.last_midi_step=1./24000.;
        rt.routing_input_frame.fill(0.);if frame==0{rt.routing_input_frame[..3].copy_from_slice(&[0.1,0.01,0.02]);}
        let output=graph.render(&mut rt,false,0.,3);
        let expected=if frame==768{[engine::limiter(0.07),engine::limiter(0.025)]}else{[0.;2]};
        for channel in 0..2{assert!((output[channel]-expected[channel]).abs()<1e-6,"frame {frame}, channel {channel}: {} != {}",output[channel],expected[channel]);}
        if frame>=768 {
            let source=frame-768;let block=source/256*256;let context=(source as f64)*0.0000001+(block as f64/24000.)*0.001+120.*0.00001+4.*0.0001+4.*0.00001+0.01;
            assert!((output[2]-engine::limiter(context as f32)).abs()<1e-6,"Wrong actual SDK context at {frame}: {}, nodes {:?}, class {:?}",output[2],graph.nodes.iter().map(|n| (n.group,n.input,n.taps)).collect::<Vec<_>>(),class);
        }
    }
    assert_eq!(graph.plugins[0].endpoint.as_ref().unwrap().control.missed_blocks(),0);
    graph.offline=false;
    let counts=engine::test_alloc::measure(||{for _ in 0..256{graph.render(&mut rt,false,0.,3);}});assert_eq!(counts,Default::default());
    let mut bypass=model;bypass.plugins[0].bypass=true;
    let mut graph=Prepared::at_rate(Arc::new(bypass),&rt.session,48000).unwrap();graph.offline=true;
    for frame in 0..1024 {rt.routing_input_frame.fill(0.);if frame==0 {rt.routing_input_frame[..3].copy_from_slice(&[0.1,0.02,0.25]);}let out=graph.render(&mut rt,false,0.,3);if frame==768{assert!((out[0]-engine::limiter(0.1)).abs()<1e-6);assert!((out[1]-engine::limiter(0.02)).abs()<1e-6);assert_eq!(out[2],0.,"A sidechain must never become a dry auxiliary output");}}
}

#[test]
#[ignore="requires the freshly compiled native worker and original SDK contract fixture"]
fn native_parallel_null_and_dynamic_latency_keep_exact_offline_output_time(){
    let (class,saved)=fixture(false);let (_,mut rt)=Engine::headless_for_test(48000,256);let mut model=effect(&class,saved,&rt);
    model.buses.push(Bus{id:5,alias:"Parallel difference".into(),channels:1,gain:1.,mute:false});
    model.connections.retain(|c|c.destination!=Group::Output(1));
    model.connections.extend([link(Group::Plugin(2),Group::Bus(5),&[(0,0,1.)]),link(Group::Input(3),Group::Bus(5),&[(0,0,-0.5),(2,0,-1.)]),link(Group::Bus(5),Group::Output(1),&[(0,0,1.)])]);
    let mut graph=Prepared::at_rate(Arc::new(model),&rt.session,48000).unwrap();graph.offline();let floor=graph.output_delay(1);assert_eq!(floor,4800);
    let mut peak=0f32;
    for frame in 0..8192u64{
        if frame==1024{assert!(graph.plugin_parameter(2,1,1.));}
        rt.routing_input_frame.fill(0.);
        if [0,512,1536,2048].contains(&frame){rt.routing_input_frame[..3].copy_from_slice(&[0.25,-0.125,0.03125]);}
        let output=graph.render(&mut rt,false,0.,3);peak=peak.max(output[0].abs());assert_eq!(graph.output_delay(1),floor);
    }
    assert_eq!(peak,0.,"Parallel plugin and dry routes must cancel, including a native latency change");
    assert_eq!(graph.plugins[0].endpoint.as_ref().unwrap().control.latency(),832);assert_eq!(graph.plugins[0].endpoint.as_ref().unwrap().control.missed_blocks(),0);
}

#[test]
#[ignore="requires the freshly compiled native worker and original SDK contract fixture"]
fn native_plugin_instrument_release_capture_container_reopen_and_missing_identity(){
    let (class,saved)=fixture(true);let (engine,mut rt)=Engine::headless_for_test(48000,256);let track=rt.session.tracks[0].id;
    let mut model=Model::default();model.version=3;model.next_id=3;model.plugins.push(instance(&class,saved,Some(track)));model.connections.push(link(Group::Plugin(2),Group::Track(track),&[(0,0,1.),(1,1,1.)]));
    rt.tracks[0].input_monitor=Some(engine::input_monitor::Mode::Off);rt.tracks[0].gain=1.;rt.tracks[0].kind=1;
    let mut graph=Prepared::at_rate(Arc::new(model),&rt.session,48000).unwrap();graph.offline=true;rt.routing=Some(Box::new(graph));
    rt.apply(Command::LiveNoteOn{source:19,ch:3,note:69,vel:127});
    let mut block=[0.;512];for _ in 0..6{rt.process_interleaved(&mut block,2);}assert!(block.iter().any(|s|s.abs()>0.01),"A plugin instrument must sound even with physical input monitoring off");
    rt.selected_track=1;rt.apply(Command::LiveNoteOff{source:19,ch:3,note:69});for _ in 0..6{rt.process_interleaved(&mut block,2);}assert!(block.iter().all(|s|s.abs()<1e-6),"A changed selection must not steal the original plugin note off");
    let namespace=rt.session.namespace;
    let edit=Command::PluginParameter{namespace,id:2,parameter:0,value:0.25};assert_eq!(engine::test_alloc::measure(||rt.apply(edit)),Default::default());
    assert_eq!(rt.routing.as_ref().unwrap().plugin_parameter_value(2,0),Some(0.25));rt.apply(Command::Undo);assert_eq!(rt.routing.as_ref().unwrap().plugin_parameter_value(2,0),Some(0.5));rt.apply(Command::Redo);assert_eq!(rt.routing.as_ref().unwrap().plugin_parameter_value(2,0),Some(0.25));rt.process_interleaved(&mut block,2);
    let captured=capture(&engine,&mut rt);let retained=captured.state.routing.as_ref().unwrap().plugins[0].clone();assert!(!retained.saved.state.is_empty());assert_eq!(retained.parameters,[Parameter{id:0,value:0.25}]);assert!(retained.unavailable.is_none());
    let root=PathBuf::from(std::env::var_os("OMATAINER_VST3_FIXTURES").unwrap()).join(format!("graph-project-{}",std::process::id()));std::fs::create_dir_all(&root).unwrap();let path=root.join("processor.omatainer");
    let bundle=crate::project_file::Bundle{state:captured.state,media:captured.media};let cancel=AtomicBool::new(false);crate::project_file::save(&path,&bundle,crate::project_file::Overwrite::Never,&Default::default(),&cancel).unwrap();
    let reopened:crate::project_file::Bundle<project::State>=crate::project_file::load(&path,&Default::default(),&cancel).unwrap();assert_eq!(reopened.state.routing.as_ref().unwrap().plugins[0],retained);
    let mut loaded=project::Prepared::from_state(reopened.state.clone(),reopened.media.clone(),48000).unwrap().into_offline();assert!(loaded.routing.as_ref().unwrap().plugins[0].error.is_none());
    loaded.apply(Command::Select{track:0,scene:0});loaded.apply(Command::LiveNoteOn{source:20,ch:3,note:60,vel:127});for _ in 0..24{loaded.process_interleaved(&mut block,2);}assert!(block.iter().any(|s|s.abs()>0.005));
    let mut missing=reopened.state.clone();let graph=Arc::make_mut(missing.routing.as_mut().unwrap());graph.plugins[0].saved.binary.bundle=root.join("Missing.vst3");
    let prepared=project::Prepared::from_state(missing,reopened.media.clone(),48000).unwrap();let saved=&prepared.rt.routing.as_ref().unwrap().model.plugins[0];assert_eq!(saved.saved.state,retained.saved.state);assert!(saved.unavailable.as_ref().unwrap().contains("unavailable"));
    let mut silent=prepared.into_offline();silent.apply(Command::Select{track:0,scene:0});silent.apply(Command::LiveNoteOn{source:21,ch:3,note:69,vel:127});for _ in 0..24{silent.process_interleaved(&mut block,2);}assert!(block.iter().all(|s|s.abs()<1e-6),"An unavailable plugin instrument must never substitute the native synth");
    let mut wrong=reopened.state;Arc::make_mut(wrong.routing.as_mut().unwrap()).plugins[0].saved.plugin_version="99".into();let prepared=project::Prepared::from_state(wrong,reopened.media,48000).unwrap();assert!(prepared.rt.routing.as_ref().unwrap().model.plugins[0].unavailable.as_ref().unwrap().contains("version"));
    rt.set_sample_rate(96000).unwrap();
    let processor=&rt.routing.as_ref().unwrap().plugins[0];assert!((processor.endpoint.as_ref().unwrap().control.class.parameters.iter().find(|p|p.id==0).unwrap().value-0.25).abs()<1e-9,"Changing the output rate must retain the latest opaque processor state");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn plugin_automation_bounds_legacy_injection_and_physical_note_ownership(){
    use super::plugins::Automation;
    let lane=Automation{id:1,points:vec![[0.,0.],[2.,1.]]};assert_eq!(lane.at(1.),0.5);assert_eq!(lane.at(4.),1.);
    let mut root=crate::engine::plugin_midi::Routing::default();let key=engine::InputKey::Midi{source:9,ch:2,note:60};root.on(key,0,2,60,127);root.clip(0,[0x92,60,90],2);root.next();root.clip(0,[0x82,60,0],1);assert!(root.events(0).is_empty());root.clear_clip(0);assert!(root.events(0).is_empty());root.release(key);assert_eq!(root.events(0),[[0x82,60,0]]);
    let (engine,mut rt)=Engine::headless_for_test(48000,256); let mut raw=serde_json::to_value(capture(&engine,&mut rt).state).unwrap();raw["version"]=28.into();raw["routing"]=serde_json::json!({"plugins":[]});assert!(serde_json::from_value::<project::State>(raw).is_err());
}

#[test]
#[ignore="requires the freshly compiled native worker and original SDK contract fixture"]
fn native_automation_corner_and_individual_panic_release_reach_the_processor() {
    let (class,saved)=fixture(true);let (_,mut rt)=Engine::headless_for_test(48000,256);let track=rt.session.tracks[0].id;
    let mut model=Model::default();model.version=3;model.next_id=3;let mut p=instance(&class,saved,Some(track));p.automation.push(super::plugins::Automation{id:0,points:vec![[0.,0.5],[128./24000.,1.],[256./24000.,0.5]]});model.plugins.push(p);
    model.connections=vec![link(Group::Plugin(2),Group::Output(1),&[(0,0,1.),(1,1,1.)])];
    let mut graph=Prepared::at_rate(Arc::new(model),&rt.session,48000).unwrap();graph.offline=true;rt.playing=true;
    rt.plugin_midi.on(engine::InputKey::Midi{source:8,ch:7,note:72},0,7,72,127);
    for frame in 0..1536u64{
        rt.timeline_frames=frame+1;rt.beat=(frame+1)as f64/24000.;rt.midi_beat=rt.beat;rt.midi_beat_reference=rt.beat;rt.last_midi_step=1./24000.;
        if frame==512{graph.reset_latency();}
        let output=graph.render(&mut rt,false,0.,2);
        if frame==768+128{assert!((output[0]-engine::limiter(0.1)).abs()<1e-6,"An envelope corner inside a block must reach VST3 at its exact sample offset");}
        if frame>=768+512{assert!(output[..2].iter().all(|s|s.abs()<1e-6),"Panic must release an actual VST note without a CC123 MIDI mapping");}
    }
    assert!(!graph.plugins[0].endpoint.as_ref().unwrap().faulted());
}

#[test]
#[ignore="requires native workers and the locally built licensed DISTRHO Nekobi and MVerb fixtures"]
fn native_independent_nekobi_instrument_and_mverb_tail_use_the_same_session_graph() {
    let root=PathBuf::from(std::env::var_os("OMATAINER_VST3_FIXTURES").unwrap()).join("dpf-plugins/bin");
    let (synth,saved)=probe(root.join("Nekobi.vst3"),true);let (fx,effect_saved)=probe(root.join("MVerb.vst3"),false);
    let (_,mut rt)=Engine::headless_for_test(48000,256);let track=rt.session.tracks[0].id;
    let mut model=Model::default();model.version=3;model.next_id=4;model.plugins.push(instance(&synth,saved,Some(track)));let mut effect=instance(&fx,effect_saved,None);effect.id=3;effect.name="MVerb".into();model.plugins.push(effect);model.connections=vec![link(Group::Plugin(2),Group::Plugin(3),&[(0,0,1.),(if model.plugins[0].output_width()>1 {1} else {0},1,1.)]),link(Group::Plugin(3),Group::Output(1),&[(0,0,1.),(1,1,1.)])];
    let mut graph=Prepared::at_rate(Arc::new(model),&rt.session,48000).unwrap();graph.offline();assert!(graph.plugins.iter().all(|p|p.error.is_none()),"{:?}",graph.plugins.iter().map(|p|&p.error).collect::<Vec<_>>());
    rt.plugin_midi.on(engine::InputKey::Midi{source:3,ch:2,note:69},0,2,69,127);let mut peak=0f32;let mut tail=0f32;
    for frame in 0..48000{if frame==12000 {rt.plugin_midi.release(engine::InputKey::Midi{source:3,ch:2,note:69});}let out=graph.render(&mut rt,false,0.,2);assert!(out.iter().all(|s|s.is_finite()));peak=peak.max(out[0].abs());if frame>=16000{tail=tail.max(out[0].abs());}}
    assert!(peak>1e-4,"The independent native synth must produce audio through its effect chain");assert!(tail>1e-6,"The independent native reverb must keep processing its source-release tail");assert!(graph.plugins.iter().all(|p|p.endpoint.as_ref().unwrap().control.missed_blocks()==0));
}

#[test]
#[ignore="requires the freshly compiled native worker and original SDK contract fixture"]
fn native_cancelled_graph_and_oversized_histories_refuse_without_replacing_the_session() {
    let (class,saved)=fixture(false);let (_,rt)=Engine::headless_for_test(48000,256);let model=effect(&class,saved.clone(),&rt);
    let cancel=AtomicBool::new(true);assert!(Prepared::with_cancel(Arc::new(model),&rt.session,48000,&cancel).unwrap_err().contains("cancelled"));assert!(rt.routing.is_none());
    let mut model=Model::default();model.version=3;model.latency=Some(super::latency::Configuration{reserve_micros:2_000_000,..Default::default()});model.next_id=34;
    for id in 2..34{let mut p=instance(&class,saved.clone(),None);p.id=id;p.name=format!("Processor {id}");p.inputs=vec![32];p.outputs=vec![32];model.plugins.push(p);}
    assert!(Prepared::at_rate(Arc::new(model),&rt.session,48000).unwrap_err().contains("64 MiB"));assert!(rt.routing.is_none());
}

#[test]
#[ignore="requires the freshly compiled native worker and original SDK contract fixture"]
fn native_editor_refusal_preserves_the_running_processor_and_its_state() {
    let (_,saved)=fixture(false);let cancel=AtomicBool::new(false);let exe=PathBuf::from(std::env::var_os("OMATAINER_TEST_BIN").unwrap());let mut worker=plugin_host::process::Process::start(&exe).unwrap();
    assert!(matches!(worker.exchange(&Request::Load{saved,rate:48000},&cancel,Duration::from_secs(10)).unwrap(),Response::Loaded{..}));
    assert!(worker.exchange(&Request::Editor{open:true},&cancel,Duration::from_secs(10)).unwrap_err().contains("GUI"));
    assert!(matches!(worker.exchange(&Request::State,&cancel,Duration::from_secs(10)).unwrap(),Response::State{..}),"A normal editor refusal must retain the plugin process and checkpoint");
}

#[test]
fn encoded_plugin_states_refuse_json_expansion_before_graph_install() {
    let binary=plugin_host::BinaryIdentity {bundle:"/missing/Contract.vst3".into(),binary:"/missing/Contract.so".into(),sha256:"0".repeat(64),arch:"aarch64".into()};
    let saved=Saved {schema:1,binary,class_id:"0".repeat(32),plugin_version:"1.0.0".into(),state_codec:"vst3-host-0.9-state".into(),state:vec![255;8*1024*1024]};
    let p=Instance {id:2,name:"Large state".into(),saved,inputs:vec![],outputs:vec![2],midi_track:None,scene_track:None,instrument:false,bypass:false,latency:768,parameters:vec![],automation:vec![],unavailable:None};
    let mut model=Model::default();model.version=3;model.next_id=3;model.plugins.push(p);
    let layout=engine::session::Layout::legacy((0..8).map(|n|format!("Track {n}")),8);
    assert!(model.order(&layout).unwrap_err().contains("after encoding"));
}

#[test]
#[ignore="requires fresh native workers and the original SDK contract instrument; writes private WAV exports"]
fn native_plugin_export_trims_bridge_delay_refuses_missing_processing_and_keeps_cancel_atomic() {
    use crate::audio_delivery::{self, Export};
    let (class,saved)=fixture(true); let (engine,mut rt)=Engine::headless_for_test(48000,256);
    let track=rt.session.tracks[0].id; rt.tracks[0].kind=1; rt.tracks[0].gain=1.;rt.master=0.5;
    for other in rt.tracks.iter_mut().skip(1) {other.mute=true;}
    let clip=&mut rt.tracks[0].clips[0];clip.kind=engine::ClipKind::Midi;clip.bars=1.;clip.notes=vec![engine::MidiNote { variation: None,id:engine::midi_edit::NoteId::new(),pitch:60,start:0.,len:1.,vel:127,channel:4,release_vel:0,source_timing:None,muted:false}];
    let mut model=Model::default();model.version=3;model.next_id=3;model.plugins.push(instance(&class,saved,Some(track)));model.connections.push(link(Group::Plugin(2),Group::Track(track),&[(0,0,1.),(1,1,1.)]));
    rt.routing=Some(Box::new(Prepared::at_rate(Arc::new(model),&rt.session,48000).unwrap()));
    let root=PathBuf::from(std::env::var_os("OMATAINER_VST3_FIXTURES").unwrap()).join(format!("plugin-exports-{}",std::process::id()));std::fs::create_dir_all(&root).unwrap();
    let request=Export {output_alias:Some(1),start:0.01,end:0.04,tail:0.,..Default::default()};
    let result=audio_delivery::run(capture(&engine,&mut rt),&request,&root.join("playable"),&engine.cmd.performance().optional_work().unwrap(),&Default::default()).unwrap();
    let rendered=engine::decode::decode_audio(&root.join("playable/master.wav")).unwrap();assert_eq!(result.frames,1440);assert_eq!(rendered.sample.frames(),1440);assert!(rendered.sample.data.iter().all(|v|(*v-engine::limiter(0.025)).abs()<1e-6),"Offline publication must trim the plugin and compensation startup exactly once");
    let mut captured=capture(&engine,&mut rt);Arc::make_mut(captured.state.routing.as_mut().unwrap()).plugins[0].saved.binary.bundle=root.join("Absent.vst3");
    assert!(audio_delivery::run(captured,&request,&root.join("missing"),&engine.cmd.performance().optional_work().unwrap(),&Default::default()).unwrap_err().contains("unavailable"));assert!(!root.join("missing").exists());
    let cancelled=engine.cmd.performance().optional_work().unwrap();cancelled.cancel().store(true,std::sync::atomic::Ordering::Release);
    assert!(audio_delivery::run(capture(&engine,&mut rt),&request,&root.join("cancelled"),&cancelled,&Default::default()).is_err());assert!(!root.join("cancelled").exists());
    assert!(!rt.playing);assert!(rt.routing.as_ref().unwrap().plugins[0].endpoint.as_ref().unwrap().control.error().is_none());std::fs::remove_dir_all(root).unwrap();
}
