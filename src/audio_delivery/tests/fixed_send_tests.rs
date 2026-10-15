use super::*;
use crate::engine::{ClipKind, project, test_alloc};

fn fixture(rate:u32,sends:[f32;2])->(Engine,RtEngine) {
    let (engine,mut rt)=Engine::headless_for_test(rate,256);
    rt.apply(Command::Stop);
    for track in &mut rt.tracks {track.clips.iter_mut().for_each(|clip|*clip=crate::engine::Clip::empty());track.fx.slots.clear();}
    for rack in &mut rt.scene_fx {rack.slots.clear();}
    rt.quantize=false;rt.bpm=120.;rt.master=0.25;rt.xfader=0.25;
    let mut data=vec![0.;8192*2];
    for (i,left,right) in [(0,0.11,-0.07),(2048,-0.05,0.13),(8000,0.09,0.03)] {data[i*2]=left;data[i*2+1]=right;}
    let sample=Arc::new(Sample {spectrum:None,name:"Unequal impulses".into(),sr:rate,ch:2,data,peaks:Vec::new().into(),bpm:120.,path:String::new()});
    let region=crate::engine::audio_clip::Region::full(&sample,120.).unwrap().prepare(&sample).unwrap();
    let track=&mut rt.tracks[0];track.kind=4;track.gain=1.;track.eq.low_g=1.;track.eq.mid_g=1.;track.eq.high_g=1.;
    track.clips[0].audio_region=Some(region);track.clips[0].kind=ClipKind::Audio;track.clips[0].audio=Some(sample);track.clips[0].bars=1.;
    rt.apply(Command::TrackSend {track:0,send:0,value:sends[0]});
    rt.apply(Command::TrackSend {track:0,send:1,value:sends[1]});
    rt.apply(Command::TrackCrossfader {track:0,assignment:1});
    (engine,rt)
}
fn pcm(rt:&mut RtEngine,rate:u32,block:usize)->Vec<f32>{
    rt.apply(Command::LaunchScene {scene:0});
    let mut output=vec![0.;(8192+rate as usize)*2];
    let (body,tail)=output.split_at_mut(8192*2);
    for chunk in body.chunks_mut(block*2){rt.process(chunk);}
    rt.stop_export_sources();
    for chunk in tail.chunks_mut(block*2){rt.process(chunk);}
    output
}

#[test]
fn fixed_send_fresh_process_renderer() {
    let Some(path)=std::env::var_os("OMATAINER_FIXED_SEND_ARCHIVE") else {return};
    let bundle=crate::project_file::load::<project::State>(Path::new(&path),&Default::default(),&AtomicBool::new(false)).unwrap();
    let rate=std::env::var("OMATAINER_FIXED_SEND_RATE").unwrap().parse().unwrap();
    let mut prepared=project::Prepared::from_state(bundle.state,bundle.media,rate).unwrap().into_offline();
    let output=pcm(&mut prepared,rate,127);
    std::fs::write(Path::new(&path).with_extension("f32"),output.iter().flat_map(|v|v.to_le_bytes()).collect::<Vec<_>>()).unwrap();
}

#[test]
fn fixed_send_capture_native_fresh_process_and_encoded_export_null_with_wet_tails() {
    let files=Files::new();
    for rate in [44100,48000,96000] {
        let (_,mut dry)=fixture(rate,[0.;2]);let dry=pcm(&mut dry,rate,64);
        for (case,sends) in [[0.43,0.],[0.,0.71],[0.29,0.67]].into_iter().enumerate() {
            let (engine,mut live)=fixture(rate,sends);
            let saved=capture(&engine,&mut live);
            let path=files.0.join(format!("sends-{rate}-{case}.omat"));
            crate::project_file::save(&path,&crate::project_file::Bundle {state:saved.state.clone(),media:saved.media.clone()},crate::project_file::Overwrite::Never,&Default::default(),&AtomicBool::new(false)).unwrap();
            let request=Export {end:8192.0/f64::from(rate),tail:1.,options:Options {rate,..Options::default()},..Export::default()};
            let output=files.0.join(format!("encoded-{rate}-{case}"));
            run(saved,&request,&output,&engine.cmd.performance().optional_work().unwrap(),&crate::background::Reporter::default()).unwrap();
            let expected=pcm(&mut live,rate,256);
            let decoded=crate::engine::decode::decode_audio(&output.join("master.wav")).unwrap();
            assert_eq!(decoded.sample.data,expected,"export differs at {rate}/{case}");
            let wet:f64=expected[8192*2..].iter().zip(&dry[8192*2..]).map(|(a,b)|f64::from(a-b).abs()).sum();
            assert!(wet>0.0001,"missing wet tail {rate}/{case}: {wet}");
            let status=std::process::Command::new(std::env::current_exe().unwrap()).args(["--exact","audio_delivery::tests::fixed_send_tests::fixed_send_fresh_process_renderer","--test-threads=1"]).env("OMATAINER_FIXED_SEND_ARCHIVE",&path).env("OMATAINER_FIXED_SEND_RATE",rate.to_string()).status().unwrap();
            assert!(status.success());
            let independent=std::fs::read(path.with_extension("f32")).unwrap();
            assert_eq!(independent,expected.iter().flat_map(|v|v.to_le_bytes()).collect::<Vec<_>>(),"fresh process differs at {rate}/{case}");
            assert_eq!(test_alloc::measure(||live.process(&mut [0.;128])),test_alloc::Counts::default());
            eprintln!("fixed-send {rate}/{case}: exact fresh-process/live/encoded null; wet tail L1={wet}");
        }
    }
}
