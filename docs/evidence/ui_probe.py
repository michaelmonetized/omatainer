from pathlib import Path
import argparse
import subprocess
import tempfile
parser = argparse.ArgumentParser(description="Reproduce Omatainer UI audit findings without audio devices or product edits")
parser.add_argument("root", type=Path, help="Omatainer checkout with existing target/debug/deps build artifacts")
parser.add_argument("--output", type=Path, help="Preserve generated harness and logs here (default: new temporary directory)")
args = parser.parse_args()
root = args.root.resolve()
out = args.output.resolve() if args.output else Path(tempfile.mkdtemp(prefix="omatainer-ui-audit-"))
out.mkdir(parents=True, exist_ok=True)
print(f"Evidence directory: {out}")
engine=(root/'src/engine/mod.rs').read_text().split('pub struct Engine {')[0]
for m in ['audio','dsp','fx','midi']:
    engine=engine.replace(f'pub mod {m};',f'#[path = "{root}/src/engine/{m}.rs"] pub mod {m};')
engine+='''
pub struct Engine { pub rt: Arc<Mutex<RtEngine>>, pub commands: Arc<Mutex<Vec<Command>>> }
impl Engine {
 pub fn headless() -> Self { let (_,rx)=crossbeam_channel::unbounded(); let snap=Arc::new(Mutex::new(Snapshot::default())); Self {rt:Arc::new(Mutex::new(RtEngine::new(48000.0,rx,snap))),commands:Arc::new(Mutex::new(Vec::new()))} }
 pub fn send(&self,c:Command) { self.commands.lock().push(c.clone()); self.rt.lock().apply(c); }
 pub fn snapshot(&self)->Snapshot { let rt=self.rt.lock(); rt.publish(); let snap=rt.snap.lock().clone(); snap }
}
'''
(out/'engine.rs').write_text(engine)
ui=(root/'src/ui.rs').read_text()
ui+='''
pub fn audit() {
 let engine=Engine::headless();
 let (tx,rx)=mpsc::channel(); let (_,done)=mpsc::channel();
 let mut app=App {snap:engine.snapshot(), engine, theme:Theme::default(), fonts_set:true, library:Vec::new(), lib_filter:String::new(), lib_sel:0, keys_open:false, midi_open:false, status:String::new(), last_theme_check:Instant::now(), load_tx:tx, load_rx:done, last_play_idx:0, pad_held:[false;16]};
 for name in ["builtin:drums","builtin:harmony"] {
  app.library=vec![LibItem {title:name.into(),artist:String::new(),bpm:124.,key:"C".into(),length:1.,last_play:None,path:PathBuf::from(name)}];
  app.load_sel(0);
  println!("builtin load {}: engine command count={} decoder job={:?}",name,app.engine.commands.lock().len(),rx.try_recv().ok());
 }
 let ctx=egui::Context::default();
 let search_id=egui::Id::new("audit-search");
 let _=ctx.run(egui::RawInput::default(),|ctx|{egui::CentralPanel::default().show(ctx,|ui| {let r=ui.add(egui::TextEdit::singleline(&mut app.lib_filter).id(search_id));r.request_focus();});});
 println!("search focus confirmed={}",ctx.memory(|m|m.has_focus(search_id)));
 let before=app.engine.commands.lock().len();
 let input=egui::RawInput {events:vec![egui::Event::Key{key:Key::Q,physical_key:Some(Key::Q),pressed:true,repeat:false,modifiers:egui::Modifiers::default()},egui::Event::Text("q".into())],..Default::default()};
 let _=ctx.run(input,|ctx|{app.handle_keys(ctx);egui::CentralPanel::default().show(ctx,|ui|{ui.add(egui::TextEdit::singleline(&mut app.lib_filter).id(search_id));});});
 println!("type q in focused search: text={:?} engine commands={:?}",app.lib_filter,&app.engine.commands.lock()[before..]);
 {
  let mut e=app.engine.rt.lock();
  e.apply(Command::OpenFxScene(0)); e.apply(Command::FxAdd(0));
  e.apply(Command::OpenFxScene(1)); e.publish();
  println!("scene2 chain after adding only to scene1: fx_view={} fx_slots={}",e.fx_view,e.snap.lock().fx_slots.len());
  e.apply(Command::Select{track:1,scene:7}); e.apply(Command::SamplerInst(1));
  e.apply(Command::SamplerPad{pad:0,on:true}); e.apply(Command::SamplerPad{pad:0,on:false});
  let initial=e.tracks[1].clips[7].notes.len();
  e.apply(Command::Stop);e.apply(Command::CloseFx);e.apply(Command::TogglePlay);
  let changed=(e.selected_track,e.selected_scene);
  e.apply(Command::SamplerPad{pad:1,on:true});
  println!("compose permanent: initial_notes={} arm_after_Stop={} selected_after_play={:?} added_notes_target={}",initial,e.compose_armed,changed,e.tracks[changed.0].clips[changed.1].notes.len());
 }
 app.lib_filter.clear();
 let theme=Theme::default();
 for count in [100usize,10000,50000] {
  app.library=(0..count).map(|i|LibItem {title:format!("Track {i}"),artist:format!("Artist {i}"),bpm:124.,key:"C".into(),length:180.,last_play:None,path:PathBuf::from(format!("test-{i}.wav"))}).collect();
  let ctx=egui::Context::default();
  let mut timing=Vec::new();
  for frame in 0..12 {
   let start=Instant::now();
   let input=egui::RawInput {screen_rect:Some(egui::Rect::from_min_size(egui::Pos2::ZERO,egui::vec2(1440.,108.))),..Default::default()};
   let _=ctx.run(input,|ctx| {egui::CentralPanel::default().show(ctx,|ui|app.crate_row(ui,&theme));});
   if frame>=2 {timing.push(start.elapsed().as_secs_f64()*1000.);}
  }
  timing.sort_by(f64::total_cmp);
  println!("headless crate count={} warm_frames={} median_cpu_ms={:.3} min_cpu_ms={:.3} max_cpu_ms={:.3}",count,timing.len(),timing[timing.len()/2],timing[0],timing[timing.len()-1]);
 }
}
'''
(out/'ui.rs').write_text(ui)
(out/'main.rs').write_text(f'#[path="{out}/engine.rs"] mod engine; #[path="{root}/src/theme.rs"] mod theme; #[path="{out}/ui.rs"] mod ui; fn main() {{ui::audit();}}')
args=['rustc','--edition=2021','-Awarnings','-C','opt-level=1',str(out/'main.rs'),'-L',f'dependency={root}/target/debug/deps','-o',str(out/'ui-repro')]
for d in ['anyhow','serde','serde_json','cpal','crossbeam_channel','parking_lot','midir','eframe','egui','toml','walkdir','symphonia']:
 files=sorted((root/'target/debug/deps').glob(f'lib{d}-*.rlib'),key=lambda p:p.stat().st_mtime,reverse=True)
 if not files: raise SystemExit(f'Missing cached Rust dependency {d}; run cargo test in {root} first')
 args+=['--extern',f'{d}={files[0]}']
p=subprocess.run(args,capture_output=True,text=True)
(out/'build-output.txt').write_text(p.stdout+p.stderr)
print(p.stdout+p.stderr)
if p.returncode==0:
 p=subprocess.run([str(out/'ui-repro')],capture_output=True,text=True)
 (out/'repro-output.txt').write_text(p.stdout+p.stderr)
 print(p.stdout+p.stderr)
raise SystemExit(p.returncode)
