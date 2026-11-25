This comment contains the exact standalone probes behind the reproduced findings in this backlog. References in individual issues to `docs/evidence/*` mean the files below; they do not require an unpublished branch or a repository push. Source-only findings remain source-only findings.

The audited product baseline is [`cae546c7bc840ec1a643e4befe18f91e86d0e4c7`](https://github.com/michaelmonetized/omatainer/tree/cae546c7bc840ec1a643e4befe18f91e86d0e4c7), dated September 30, 2026. The recorded toolchain was `rustc 1.98.0 (88d9e12ae 2026-08-18)` on Linux, with a CPU inventory reporting 10 M1 Pro cores. The unchanged baseline passed all 13 tests; its warm local run took 0.30 seconds and emitted 19 compiler warnings.

These are headless, synthetic probes of actual source, not physical controller, audio-device, native-window, listening, desktop-FPS or full-application acceptance tests. The GUI probe substitutes only the hardware-facing Engine wrapper and drives original UI methods with egui input. The snapshot tests deliberately hold a mutex for 50/80 ms; they do not establish a measured live UI stall or device underrun. Fields named `cpu_wall_ms` and `median_cpu_ms` measure elapsed wall time, not consumed process CPU time. Timings vary with machine, scheduling and build cache. Allocated bytes are allocation traffic, not retained memory. The 192,000 allocations/second figure is the extrapolation `4096 / 1024 * 48000` for the demonstrated fixture.

Passing the integration/core defect assertions confirms the audited bug remains present; it is not a regression-test pass for repaired code. Other probe outputs are observations to inspect. MIDI learn is exercised through deliberately forced internal learn state; no available GUI activation or hardware behavior is implied.

To reproduce from a clean checkout, use Bash on Linux with Git, Python 3, Cargo/rustc and the native build prerequisites required by the locked Cargo dependencies (including a native C toolchain, pkg-config and ALSA development files). Python scripts use only the standard library. The initial Cargo build may need network access; the integration copy then builds offline.

```bash
set -euo pipefail
audit_checkout="$(mktemp -d -t omatainer-repro.XXXXXX)"
git clone https://github.com/michaelmonetized/omatainer.git "$audit_checkout"
cd "$audit_checkout"
git checkout --detach cae546c7bc840ec1a643e4befe18f91e86d0e4c7
mkdir -p docs/evidence
```

Save each of the nine source blocks below verbatim at its stated destination, retaining the filenames. All nine belong in the same `docs/evidence` directory. Then run these commands sequentially from the checkout root:

```bash
export CARGO_TARGET_DIR="$PWD/target"
cargo test --locked
python3 docs/evidence/run_audio_dsp.py "$PWD"
python3 docs/evidence/run_core.py "$PWD"
python3 docs/evidence/ui_probe.py "$PWD"
python3 docs/evidence/integration_probe.py "$PWD" --abort-check
```

`CARGO_TARGET_DIR` is set explicitly because the standalone runners expect `target/debug/deps` and choose the newest cached dependency artifacts. A fresh checkout/cache avoids mixed feature or compiler artifacts. Audio/core/UI runners print their temporary output directories and preserve generated binaries/logs there. The integration runner copies `src`, `Cargo.toml` and `Cargo.lock` to a temporary directory, appends probes only to that copy, and reuses this checkout's Cargo target cache. Run the probes sequentially to avoid concurrent Cargo/cache interference. No runner edits product source, runs the installer or opens an audio device/application window.

`--abort-check` intentionally aborts only an isolated engine harness, disables core dumps, and expects Python return code `-6` (SIGABRT) on Linux. The caught scene-index panic also prints an expected panic message during the passing integration test. Omit `--abort-check` to skip only the separate abort harness. IPC tests use socket pairs and a private temporary runtime directory; no running application socket is targeted. After review, the fresh checkout and printed temporary output directories can be removed.

The executable files are included in full; the output blocks following them preserve the recorded numeric results. SHA-256 values permit checking copied source bytes.

<details>
<summary>Source: docs/evidence/run_audio_dsp.py</summary>

Destination: `docs/evidence/run_audio_dsp.py`  
SHA-256: `fa26845aa252a735fa5e532163a68a4e4f8d16bcf2c25e3e0dda93885fc02fa7`

```python
#!/usr/bin/env python3
"""Compile an audit harness against unchanged repository source and cached Cargo deps.

Usage: python3 run_audio_dsp.py /path/to/omatainer [--out-dir /path/to/evidence]
Run `cargo test --locked` in that repository first if target/debug/deps is absent.
This runner does not invoke Cargo, open audio devices, or edit repository source.
"""
import argparse
import os
from pathlib import Path
import subprocess
import tempfile

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('repo', type=Path)
parser.add_argument('--out-dir', type=Path)
args = parser.parse_args()
repo = args.repo.resolve()
harness = Path(__file__).resolve().with_name('audio_dsp.rs')
deps = repo / 'target/debug/deps'
output = args.out_dir or Path(tempfile.mkdtemp(prefix='omatainer-audio-audit-'))
output.mkdir(parents=True, exist_ok=True)
output = output.resolve()
command = ['rustc', '--edition=2021', '-C', 'opt-level=3', '-L', f'dependency={deps}', str(harness), '-o', str(output / 'audio_dsp')]
for dep in ['anyhow', 'cpal', 'parking_lot', 'crossbeam_channel', 'serde', 'symphonia', 'midir']:
    candidates = sorted(deps.glob(f'lib{dep}-*.rlib'), key=lambda p: p.stat().st_mtime, reverse=True)
    if not candidates:
        parser.error(f'Missing cached dependency {dep}; run cargo test --locked first')
    command += ['--extern', f'{dep}={candidates[0]}']
environment = dict(os.environ, OMATAINER_AUDIT_REPO=str(repo))
subprocess.run(command, env=environment, check=True)
result = subprocess.run([str(output / 'audio_dsp')], check=True, text=True, capture_output=True)
(output / 'audio_dsp-results.txt').write_text(result.stdout)
print(result.stdout, end='')
print(f'Raw results: {output / "audio_dsp-results.txt"}')
```

</details>

<details>
<summary>Source: docs/evidence/audio_dsp.rs</summary>

Destination: `docs/evidence/audio_dsp.rs`  
SHA-256: `3027d17d81a7dd2a3c9389ed895c0f6619685986b750b7547cd4b90f3673a43f`

```rust
#![allow(dead_code)]
mod engine { include!(concat!(env!("OMATAINER_AUDIT_REPO"), "/src/engine/mod.rs")); }
use engine::{Command, RtEngine, Snapshot};
use engine::dsp::Sample;
use engine::fx::{FxId, FxSlot, FxChain};
use std::sync::{Arc,atomic::{AtomicBool,AtomicUsize, Ordering}};
use std::alloc::{GlobalAlloc,Layout,System};
use std::time::{Instant,Duration};
use parking_lot::Mutex;

static COUNT: AtomicBool = AtomicBool::new(false);
static ALLOCS: AtomicUsize = AtomicUsize::new(0);
static BYTES: AtomicUsize = AtomicUsize::new(0);
struct Counted;
unsafe impl GlobalAlloc for Counted {
 unsafe fn alloc(&self,l:Layout)->*mut u8 { if COUNT.load(Ordering::Relaxed) { ALLOCS.fetch_add(1,Ordering::Relaxed); BYTES.fetch_add(l.size(),Ordering::Relaxed); } System.alloc(l) }
 unsafe fn alloc_zeroed(&self,l:Layout)->*mut u8 { if COUNT.load(Ordering::Relaxed) { ALLOCS.fetch_add(1,Ordering::Relaxed); BYTES.fetch_add(l.size(),Ordering::Relaxed); } System.alloc_zeroed(l) }
 unsafe fn realloc(&self,p:*mut u8,l:Layout,n:usize)->*mut u8 { if COUNT.load(Ordering::Relaxed) { ALLOCS.fetch_add(1,Ordering::Relaxed); BYTES.fetch_add(n,Ordering::Relaxed); } System.realloc(p,l,n) }
 unsafe fn dealloc(&self,p:*mut u8,l:Layout) { System.dealloc(p,l) }
}
#[global_allocator] static A: Counted = Counted;
fn make()->RtEngine { let (_tx,rx)=crossbeam_channel::bounded(256); RtEngine::new(48000.,rx,Arc::new(Mutex::new(Snapshot::default()))) }
fn sample(v:Vec<f32>,ch:u16)->Arc<Sample> { Arc::new(Sample{name:"audit".into(),sr:48000,ch,data:v,peaks:vec![],bpm:120.,path:String::new()}) }
fn measure(name:&str, rt:&mut RtEngine, frames:usize) {
 let mut out=vec![0.;frames*2];
 ALLOCS.store(0,Ordering::Relaxed);BYTES.store(0,Ordering::Relaxed);COUNT.store(true,Ordering::Relaxed);
 let t=Instant::now();rt.process(&mut out);let elapsed=t.elapsed();COUNT.store(false,Ordering::Relaxed);
 println!("{name}: frames={frames} allocations={} allocated_bytes={} cpu_wall_ms={:.3}",ALLOCS.load(Ordering::Relaxed),BYTES.load(Ordering::Relaxed),elapsed.as_secs_f64()*1000.);
}
fn main() {
 let mut rt=make();let mut idle=[0.;2048];rt.process(&mut idle);println!("default_stopped_output: last_left={} last_right={} playing={} decks_playing={:?}",idle[2046],idle[2047],rt.playing,rt.decks.each_ref().map(|d|d.playing)); measure("idle",&mut rt,1024);
 rt.apply(Command::LaunchScene{scene:0}); let n:usize=rt.tracks.iter().filter_map(|t|t.playing.map(|p|t.clips[p.scene as usize].notes.len())).sum();println!("demo active_clips={} note_count={n}",rt.tracks.iter().filter(|t|t.playing.is_some()).count()); measure("demo",&mut rt,1024);
 let mut rt=make();for t in &mut rt.tracks{t.fx.slots.push(FxSlot::new(FxId::Comp));}measure("one_fx_each_track",&mut rt,1024);
 let mut rt=make();rt.apply(Command::LaunchClip{track:2,scene:0});rt.tracks[2].fx.slots.push(FxSlot::new(FxId::Arp));measure("one_arp_chord",&mut rt,1024);
 let mut rt=make();rt.apply(Command::DeckAudio{deck:0,audio:sample(vec![0.25;96000],2)}); rt.xfader=0.;rt.master=1.; let mut out=vec![0.;2048];rt.process(&mut out);println!("paused_deck: playing={} pos={} final_left={} final_right={} max_abs={}",rt.decks[0].playing,rt.decks[0].pos,out[2046],out[2047],out.iter().map(|x|x.abs()).fold(0.,f32::max));
 let mut rt=make();rt.apply(Command::DeckAudio{deck:0,audio:sample((0..96000).map(|i|if i%2==0{0.25}else{0.}).collect(),2)});rt.xfader=0.;rt.master=1.;rt.decks[0].playing=true;rt.decks[0].eq.low_g=0.;rt.process(&mut out);println!("left_only_after_deck_eq: final_left={} final_right={}",out[2046],out[2047]);
 for vel in [1,127] {let mut rt=make();rt.decks[0].audio=None;rt.decks[1].audio=None;rt.apply(Command::LiveNoteOn{ch:0,note:36,vel});rt.process(&mut out);println!("drum_vel={vel} abs_sum={:.9}",out.iter().map(|x|x.abs()).sum::<f32>());}
 let mut chain=FxChain::new(48000.);println!("empty_fx_impulse={:?},{:?}",chain.tick_stereo(1.,48000.),chain.tick_stereo(0.,48000.));
 for param in [0.,1.] {let mut chain=FxChain::new(48000.);let mut slot=FxSlot::new(FxId::Delay);slot.p[0]=param;slot.mix=1.;slot.p[1]=0.;chain.slots.push(slot);let mut first=None;for i in 0..24001{let y=chain.tick(if i==0{1.}else{0.},48000.);if y.abs()>0.01&&first.is_none(){first=Some(i);}}println!("delay_param0={param} first_echo_sample={first:?}");}
 for count in [1,2] {let mut chain=FxChain::new(48000.);for _ in 0..count{let mut slot=FxSlot::new(FxId::Delay);slot.mix=1.;slot.p[1]=0.;chain.slots.push(slot);}let first=(0..48001).find(|i|chain.tick(if *i==0{1.}else{0.},48000.).abs()>0.01);println!("serial_delay_slots={count} first_echo={first:?}");}
 let mut rt=make();rt.decks[1].audio=None;let mut v=vec![0.;96000];v[2]=1.;rt.apply(Command::DeckAudio{deck:0,audio:sample(v,2)});rt.decks[0].playing=true;rt.xfader=0.;rt.bpm=120.;rt.fx_wet[0]=1.;rt.delay.fb=0.;let mut v=vec![0.;22000];rt.process(&mut v);println!("master_echo_120_bpm: configured_delay_samples={} first_echo_frame={:?}",rt.delay.time_samples,v.chunks_exact(2).position(|s|s[0].abs()>0.01||s[1].abs()>0.01));
 let mut chain=FxChain::new(48000.);let mut slot=FxSlot::new(FxId::Balance);slot.mix=0.;slot.p[0]=0.;chain.slots.push(slot);for _ in 0..2{chain.tick_stereo(0.8,48000.);}println!("balance_mix_zero={:?}",chain.tick_stereo(0.8,48000.));
 let mut rt=make();rt.decks[0].audio=None;rt.decks[1].audio=None;rt.process(&mut vec![0.;11998]);let snap=rt.snap.clone();let (tx,rx)=std::sync::mpsc::channel();let handle=std::thread::spawn(move||{let guard=snap.lock();tx.send(()).unwrap();std::thread::sleep(Duration::from_millis(50));drop(guard);});rx.recv().unwrap();let t=Instant::now();rt.process(&mut [0.;2]);let elapsed=t.elapsed();handle.join().unwrap();println!("snapshot_lock_contention: one_frame_wall_ms={:.3} reported_cpu_fraction={}",elapsed.as_secs_f64()*1000.,rt.snap.lock().cpu);
}
```

</details>

<details>
<summary>Source: docs/evidence/run_core.py</summary>

Destination: `docs/evidence/run_core.py`  
SHA-256: `2ba393d35edec6a122299f466a8876db7629c7055399dfd1d4186a56c09c3219`

```python
#!/usr/bin/env python3
"""Compile an audit harness against unchanged repository source and cached Cargo deps.

Usage: python3 run_core.py /path/to/omatainer [--out-dir /path/to/evidence]
Run `cargo test --locked` in that repository first if target/debug/deps is absent.
This runner does not invoke Cargo, open audio devices, or edit repository source.
"""
import argparse
import os
from pathlib import Path
import subprocess
import tempfile

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('repo', type=Path)
parser.add_argument('--out-dir', type=Path)
args = parser.parse_args()
repo = args.repo.resolve()
harness = Path(__file__).resolve().with_name('core_probe.rs')
deps = repo / 'target/debug/deps'
output = args.out_dir or Path(tempfile.mkdtemp(prefix='omatainer-core-audit-'))
output.mkdir(parents=True, exist_ok=True)
output = output.resolve()
command = ['rustc', '--edition=2021', '-C', 'opt-level=3', '-L', f'dependency={deps}', str(harness), '-o', str(output / 'core_probe')]
for dep in ['anyhow', 'cpal', 'parking_lot', 'crossbeam_channel', 'serde', 'symphonia', 'midir']:
    candidates = sorted(deps.glob(f'lib{dep}-*.rlib'), key=lambda p: p.stat().st_mtime, reverse=True)
    if not candidates:
        parser.error(f'Missing cached dependency {dep}; run cargo test --locked first')
    command += ['--extern', f'{dep}={candidates[0]}']
environment = dict(os.environ, OMATAINER_AUDIT_REPO=str(repo))
subprocess.run(command, env=environment, check=True)
result = subprocess.run([str(output / 'core_probe')], check=True, text=True, capture_output=True)
(output / 'core-results.txt').write_text(result.stdout)
print(result.stdout, end='')
print(f'Raw results: {output / "core-results.txt"}')
```

</details>

<details>
<summary>Source: docs/evidence/core_probe.rs</summary>

Destination: `docs/evidence/core_probe.rs`  
SHA-256: `f90d9c2843fdbd05c1efdd24ba048924055d21d722002ee9e35805481e982419`

```rust
#![allow(dead_code)]
mod engine { include!(concat!(env!("OMATAINER_AUDIT_REPO"), "/src/engine/mod.rs")); }
use engine::*;
use std::sync::Arc;
use parking_lot::Mutex;
fn fresh() -> RtEngine {
 let (_,rx)=crossbeam_channel::bounded(256);
 let mut rt=RtEngine::new(48000., rx, Arc::new(Mutex::new(Snapshot::default())));
 rt.apply(Command::DeckUnload{deck:0}); rt.apply(Command::DeckUnload{deck:1});
 rt
}
fn render(rt:&mut RtEngine,n:usize) { rt.process(&mut vec![0.;n*2]); }
fn active(rt:&RtEngine,t:usize)->Vec<(u8,u8)> { rt.tracks[t].poly.voices.iter().filter(|v|v.env.active()).map(|v|(v.note,v.env.stage)).collect() }
fn main() {
 let mut rt=fresh(); rt.beat=0.25; rt.playing=true;
 rt.tracks[1].clips[2]=Clip{kind:ClipKind::Midi,name:"probe".into(),bars:1.,notes:vec![MidiNote{pitch:60,start:0.,len:3.9,vel:100}],gain:1.,audio:None};
 rt.apply(Command::FireClip{track:1,scene:2,looping:false});
 let scheduled=rt.tracks[1].playing.unwrap().start_beat;
 render(&mut rt,1);
 println!("QUANT first frame: beat={:.6}, scheduled={}, active={:?}",rt.beat,scheduled,active(&rt,1));
 assert!(rt.beat<scheduled && !active(&rt,1).is_empty());
 render(&mut rt,20000);
 println!("QUANT one-shot after scheduled start: beat={:.6}, playing_clip={:?}, active={:?}",rt.beat,rt.tracks[1].playing,active(&rt,1));
 assert!(rt.tracks[1].playing.is_none());
 let mut rt=fresh(); rt.apply(Command::Select{track:1,scene:2}); rt.apply(Command::LiveNoteOn{ch:0,note:60,vel:100}); render(&mut rt,128);
 rt.apply(Command::Select{track:2,scene:2}); rt.apply(Command::LiveNoteOff{ch:0,note:60}); render(&mut rt,96000);
 println!("NOTE OFF after changing selection: old_track={:?}, new_track={:?}",active(&rt,1),active(&rt,2));
 assert!(active(&rt,1).contains(&(60,3)));
 let mut rt=fresh(); rt.tracks[1].clips[2]=Clip{kind:ClipKind::Midi,name:"held".into(),bars:1.,notes:vec![MidiNote{pitch:64,start:0.,len:3.,vel:100}],gain:1.,audio:None};
 rt.apply(Command::LaunchClip{track:1,scene:2}); render(&mut rt,1000); rt.apply(Command::ToggleScene{scene:2}); render(&mut rt,96000);
 println!("SCENE OFF after 2 s: playing_clip={:?}, active={:?}",rt.tracks[1].playing,active(&rt,1));
 assert!(rt.tracks[1].playing.is_none() && active(&rt,1).contains(&(64,3)));
 let mut rt=fresh(); render(&mut rt,5500); let snap=rt.snap.clone(); let held=snap.lock();
 let (tx,rx)=std::sync::mpsc::channel(); let (done_tx,done_rx)=std::sync::mpsc::channel();
 let thread=std::thread::spawn(move||{tx.send(()).unwrap(); let t=std::time::Instant::now(); render(&mut rt,512); done_tx.send(t.elapsed()).unwrap();});
 rx.recv().unwrap(); std::thread::sleep(std::time::Duration::from_millis(80));
 let blocked=done_rx.try_recv().is_err(); drop(held); let elapsed=done_rx.recv().unwrap(); thread.join().unwrap();
 println!("SNAPSHOT lock: render blocked={}, total_ms={:.3}, simulated_block_budget_ms=10.667",blocked,elapsed.as_secs_f64()*1000.); assert!(blocked);
}
```

</details>

<details>
<summary>Source: docs/evidence/ui_probe.py</summary>

Destination: `docs/evidence/ui_probe.py`  
SHA-256: `97460b8cc406655987469aba1baadff775fdc6389cad4be168e4df75bcbff900`

```python
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
```

</details>

<details>
<summary>Source: docs/evidence/integration_probe.py</summary>

Destination: `docs/evidence/integration_probe.py`  
SHA-256: `94b5c725ced0d7ebd5d5c5e4a2c18326dcc27ec2e37f331f280980084ca8d2ea`

```python
#!/usr/bin/env python3
"""Copy the current repo, append invariant probes, run without audio or desktop startup.
Usage: python3 reproduce.py /path/to/omatainer [--abort-check]
Probes pass when the audited defect remains present. They are evidence, not fixes.
"""
import os, resource, shutil, signal, subprocess, sys, tempfile
from pathlib import Path
repo = Path(sys.argv[1]).resolve()
evidence = Path(__file__).resolve().parent
cache = repo / 'target'
with tempfile.TemporaryDirectory(prefix='omatainer-integration-audit-') as tmp:
    work = Path(tmp)
    shutil.copytree(repo / 'src', work / 'src')
    for name in ('Cargo.toml', 'Cargo.lock'):
        shutil.copy2(repo / name, work / name)
    for target, snippet in (('src/main.rs','ipc_audit_tests.rs'),('src/engine/midi.rs','midi_audit_tests.rs')):
        with (work / target).open('a') as f:
            f.write((evidence / snippet).read_text())
    cargo = ['cargo','test','--offline','--manifest-path',str(work/'Cargo.toml'),'--target-dir',str(cache),'--bin','omatainer','audit_','--','--nocapture','--test-threads=1']
    subprocess.run(cargo,check=True)
    if '--abort-check' in sys.argv[2:]:
        shutil.copy2(evidence/'audit_scene_crash.rs',work/'src/audit_scene_crash.rs')
        with (work/'Cargo.toml').open('a') as f:
            f.write('\n[[bin]]\nname="audit_scene_crash"\npath="src/audit_scene_crash.rs"\n')
        subprocess.run(['cargo','rustc','--offline','--manifest-path',str(work/'Cargo.toml'),'--target-dir',str(cache),'--bin','audit_scene_crash','--','-C','panic=abort'],check=True)
        resource.setrlimit(resource.RLIMIT_CORE,(0,0))
        result = subprocess.run([str(cache/'debug/audit_scene_crash')])
        print('scene 9 panic=abort return code:',result.returncode)
        assert result.returncode == -signal.SIGABRT
```

</details>

<details>
<summary>Source: docs/evidence/ipc_audit_tests.rs</summary>

Destination: `docs/evidence/ipc_audit_tests.rs`  
SHA-256: `744ea1f53161a1ddf319aedf474aa0bcf4dc0539f65d78aa6cb42308ddbe29be`

```rust

#[cfg(test)]
mod adversarial_ipc_audit {
    use super::*;
    #[test]
    fn audit_unknown_and_invalid_requests_report_success() {
        let (_tx,rx) = crossbeam_channel::bounded(16);
        let snap = std::sync::Arc::new(parking_lot::Mutex::new(engine::Snapshot::default()));
        let rt = std::sync::Arc::new(parking_lot::Mutex::new(engine::RtEngine::new(48000.0,rx,snap.clone())));
        for payload in ["not JSON at all", r#"{"op":"definitelyNotAnOperation"}"#, "status"] {
            let (mut client,server) = UnixStream::pair().unwrap();
            client.set_read_timeout(Some(Duration::from_secs(1))).unwrap();
            let rc = rt.clone(); let sc = snap.clone();
            let thread = std::thread::spawn(move || handle_client(server,rc,sc));
            writeln!(client,"{payload}").unwrap();
            let mut response = String::new();
            BufReader::new(client.try_clone().unwrap()).read_line(&mut response).unwrap();
            let response:serde_json::Value=serde_json::from_str(&response).unwrap();
            assert_eq!(response["ok"],true);
            println!("IPC payload {payload:?}: ok={}",response["ok"]);
            drop(client);
            thread.join().unwrap().unwrap();
        }
    }
    #[test]
    fn audit_slow_live_socket_is_unlinked_as_stale() {
        let path = std::env::temp_dir().join(format!("omatainer-audit-runtime-{}",std::process::id()));
        std::fs::create_dir_all(&path).unwrap();
        let prev = std::env::var_os("XDG_RUNTIME_DIR");
        std::env::set_var("XDG_RUNTIME_DIR",&path);
        let p=socket_path(); let _=std::fs::remove_file(&p);
        let listener=UnixListener::bind(&p).unwrap();
        let accept=std::thread::spawn(move || {
            let (stream,_)=listener.accept().unwrap();
            std::thread::sleep(Duration::from_millis(450));
            drop(stream); drop(listener);
        });
        let found=single_instance_running();
        let exists=p.exists();
        println!("Live listener with >200ms response: running={found}, socket_path_exists={exists}");
        accept.join().unwrap();
        match prev {Some(v)=>std::env::set_var("XDG_RUNTIME_DIR",v),None=>std::env::remove_var("XDG_RUNTIME_DIR")};
        assert!(!found); assert!(!exists);
    }
}

#[cfg(test)]
mod adversarial_scene_audit {
    use super::*;
    #[test]
    fn audit_cli_scene_nine_panics_in_engine() {
        let (_tx,rx)=crossbeam_channel::bounded(16);
        let snap=std::sync::Arc::new(parking_lot::Mutex::new(engine::Snapshot::default()));
        let mut rt=engine::RtEngine::new(48000.0,rx,snap);
        let command_line_scene: u8 = "9".parse().unwrap();
        let result=std::panic::catch_unwind(std::panic::AssertUnwindSafe(||
            rt.apply(Command::LaunchScene {scene:command_line_scene.saturating_sub(1)})));
        println!("ctl scene 9 -> zero-based index8 -> engine panicked={}",result.is_err());
        assert!(result.is_err());
    }
}
```

</details>

<details>
<summary>Source: docs/evidence/midi_audit_tests.rs</summary>

Destination: `docs/evidence/midi_audit_tests.rs`  
SHA-256: `13828a54b35a17bf71abd85373fb6cb669f52ee5c0c545d4b04dc20ba0e816cb`

```rust

#[cfg(test)]
mod adversarial_midi_audit {
    use super::*;
    fn observe(bytes: &[u8], map: &MidiMap) -> Vec<Command> {
        let (tx, rx) = crossbeam_channel::bounded(256);
        handle_msg(bytes, map, &tx, &Arc::new(Mutex::new(Vec::new())),
            &Arc::new(Mutex::new(None)), &Arc::new(Mutex::new([false; 4])), "audit");
        rx.try_iter().collect()
    }
    #[test]
    fn audit_one_byte_clock_and_transport_are_discarded() {
        for byte in [0xFA, 0xFC, 0xF8] {
            let cmds = observe(&[byte], &class_compliant());
            println!("MIDI realtime {byte:#04x}: {} commands", cmds.len());
            assert!(cmds.is_empty());
        }
    }
    #[test]
    fn audit_apc40_first_fader_fans_out_to_every_track() {
        let cmds = observe(&[0xB0, 7, 127], &akai_apc40());
        let tracks: Vec<u8> = cmds.iter().filter_map(|c| match c {
            Command::TrackGain {track, ..} => Some(*track), _ => None
        }).collect();
        println!("APC40 B0 07 7F changes tracks {tracks:?}");
        assert_eq!(tracks, vec![0,0,1,2,3,4,5,6,7]);
    }
    #[test]
    fn audit_relative_jog_decoder_has_no_reverse_value() {
        let deltas: Vec<f32> = (0..128).flat_map(|v| observe(&[0xB0, 0x21, v], &pioneer_ddj_fx()))
            .filter_map(|c| match c { Command::DeckJog {delta,..} => Some(delta), _ => None }).collect();
        println!("All 128 relative jog bytes: min={} max={} negative_count={}",
            deltas.iter().copied().fold(f32::INFINITY, f32::min),
            deltas.iter().copied().fold(f32::NEG_INFINITY, f32::max),
            deltas.iter().filter(|v| **v < 0.0).count());
        assert_eq!(deltas.len(),128);
        assert!(deltas.iter().all(|d| *d >= 0.0));
    }
    #[test]
    fn audit_pioneer_load_reaches_empty_engine_arm() {
        let cmds = observe(&[0x90, 0x02, 127], &pioneer_ddj_fx());
        assert!(matches!(cmds.as_slice(), [Command::DeckLoadSelected {deck:0}]));
        let (_tx, rx) = crossbeam_channel::bounded(1);
        let mut rt = crate::engine::RtEngine::new(48000.0, rx,
            Arc::new(Mutex::new(crate::engine::Snapshot::default())));
        let before = rt.decks[0].audio.as_ref().map(Arc::as_ptr);
        for c in cmds { rt.apply(c); }
        assert_eq!(before, rt.decks[0].audio.as_ref().map(Arc::as_ptr));
        println!("Pioneer load button emits DeckLoadSelected; engine keeps same audio allocation");
    }
    #[test]
    fn audit_learning_swallows_every_message_without_finishing() {
        let (tx, rx) = crossbeam_channel::bounded(32);
        let learn = Arc::new(Mutex::new(Some("master".to_string())));
        let log = Arc::new(Mutex::new(Vec::new()));
        let shift = Arc::new(Mutex::new([false;4]));
        for bytes in [[0xB0,7,64], [0x90,60,100], [0x80,60,0]] {
            handle_msg(&bytes, &class_compliant(), &tx, &log, &learn, &shift, "audit");
        }
        let cmds: Vec<_> = rx.try_iter().collect();
        assert_eq!(cmds.len(),3);
        assert!(cmds.iter().all(|c| matches!(c,Command::LearnCapture{..})));
        assert!(learn.lock().is_some());
        println!("Forced learn mode converts CC, NoteOn and NoteOff into 3 discarded LearnCapture commands; stays enabled");
    }
}
```

</details>

<details>
<summary>Source: docs/evidence/audit_scene_crash.rs</summary>

Destination: `docs/evidence/audit_scene_crash.rs`  
SHA-256: `e006c2657c426b1d5e1bd6f950c1fc6f1edbd2b55f1f4fd9cda8b13dbfdf9995`

```rust
mod engine;
mod theme;
fn main() {
 let (_tx,rx)=crossbeam_channel::bounded(16);
 let snap=std::sync::Arc::new(parking_lot::Mutex::new(engine::Snapshot::default()));
 let mut rt=engine::RtEngine::new(48000.0,rx,snap);
 rt.apply(engine::Command::LaunchScene {scene:8});
}
```

</details>

<details>
<summary>Recorded output: audio-results.txt</summary>

```text
default_stopped_output: last_left=0.065097496 last_right=0.065097496 playing=false decks_playing=[false, false]
idle: frames=1024 allocations=0 allocated_bytes=0 cpu_wall_ms=0.371
demo active_clips=4 note_count=30
demo: frames=1024 allocations=4096 allocated_bytes=368640 cpu_wall_ms=0.910
one_fx_each_track: frames=1024 allocations=8192 allocated_bytes=196608 cpu_wall_ms=0.429
one_arp_chord: frames=1024 allocations=3072 allocated_bytes=106496 cpu_wall_ms=0.464
paused_deck: playing=false pos=0 final_left=0.20935817 final_right=0.20935817 max_abs=0.20935817
left_only_after_deck_eq: final_left=0.10413283 final_right=-0.104132734
drum_vel=1 abs_sum=788.015258789
drum_vel=127 abs_sum=788.015258789
empty_fx_impulse=(1.0, 0.0),(0.0, 1.0)
delay_param0=0 first_echo_sample=Some(12000)
delay_param0=1 first_echo_sample=Some(12000)
serial_delay_slots=1 first_echo=Some(12000)
serial_delay_slots=2 first_echo=Some(12000)
master_echo_120_bpm: configured_delay_samples=18000 first_echo_frame=Some(9000)
balance_mix_zero=(0.8, 0.0)
snapshot_lock_contention: one_frame_wall_ms=50.117 reported_cpu_fraction=0.02275596
```

</details>

<details>
<summary>Recorded output: core-results.txt</summary>

```text
QUANT first frame: beat=0.250043, scheduled=1, active=[(60, 1)]
QUANT one-shot after scheduled start: beat=1.111154, playing_clip=None, active=[(60, 2)]
NOTE OFF after changing selection: old_track=[(60, 3)], new_track=[]
SCENE OFF after 2 s: playing_clip=None, active=[(64, 3)]
SNAPSHOT lock: render blocked=true, total_ms=80.105, simulated_block_budget_ms=10.667
```

</details>

<details>
<summary>Recorded output: ui-results.txt</summary>

```text
builtin load builtin:drums: engine command count=0 decoder job=Some((0, "builtin:drums"))
builtin load builtin:harmony: engine command count=0 decoder job=Some((0, "builtin:harmony"))
search focus confirmed=true
type q in focused search: text="q" engine commands=[DeckPlay { deck: 0 }]
scene2 chain after adding only to scene1: fx_view=101 fx_slots=1
compose permanent: initial_notes=1 arm_after_Stop=true selected_after_play=(3, 0) added_notes_target=4
headless crate count=100 warm_frames=10 median_cpu_ms=0.170 min_cpu_ms=0.167 max_cpu_ms=0.175
headless crate count=10000 warm_frames=10 median_cpu_ms=22.074 min_cpu_ms=19.865 max_cpu_ms=29.138
headless crate count=50000 warm_frames=10 median_cpu_ms=116.694 min_cpu_ms=111.969 max_cpu_ms=118.972
```

</details>

<details>
<summary>Integration results: concise extract of the recorded run</summary>

Compiler warnings and verbose build output are omitted. These are the recorded result lines, not an additional run. The eight integration probes passed while asserting the audited defects; the separate engine harness terminated with the expected SIGABRT.

```text
running 8 tests
test adversarial_ipc_audit::audit_slow_live_socket_is_unlinked_as_stale ... Live listener with >200ms response: running=false, socket_path_exists=false
test adversarial_ipc_audit::audit_unknown_and_invalid_requests_report_success ... IPC payload "not JSON at all": ok=true
IPC payload "{\"op\":\"definitelyNotAnOperation\"}": ok=true
IPC payload "status": ok=true
ctl scene 9 -> zero-based index8 -> engine panicked=true
test engine::midi::adversarial_midi_audit::audit_apc40_first_fader_fans_out_to_every_track ... APC40 B0 07 7F changes tracks [0, 0, 1, 2, 3, 4, 5, 6, 7]
test engine::midi::adversarial_midi_audit::audit_learning_swallows_every_message_without_finishing ... Forced learn mode converts CC, NoteOn and NoteOff into 3 discarded LearnCapture commands; stays enabled
test engine::midi::adversarial_midi_audit::audit_one_byte_clock_and_transport_are_discarded ... MIDI realtime 0xfa: 0 commands
MIDI realtime 0xfc: 0 commands
MIDI realtime 0xf8: 0 commands
test engine::midi::adversarial_midi_audit::audit_pioneer_load_reaches_empty_engine_arm ... Pioneer load button emits DeckLoadSelected; engine keeps same audio allocation
test engine::midi::adversarial_midi_audit::audit_relative_jog_decoder_has_no_reverse_value ... All 128 relative jog bytes: min=0 max=22.05 negative_count=0
test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 13 filtered out; finished in 0.67s
scene 9 panic=abort return code: -6
```

</details>
