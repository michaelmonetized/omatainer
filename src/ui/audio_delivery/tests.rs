use super::*;
use crate::engine::audio::OutputCallback;
use egui::accesskit::{Action, ActionRequest, Node, NodeId};
struct Files(PathBuf);
impl Files {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!(
            "omatainer-native-audio-export-{}",
            crate::sampler_bank::BankId::new().unwrap()
        ));
        std::fs::create_dir(&p).unwrap();
        Self(p)
    }
}
impl Drop for Files {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
struct Gui {
    app: App,
    callback: OutputCallback,
    ctx: egui::Context,
    nodes: Vec<(NodeId, Node)>,
    time: f64,
}
impl Gui {
    fn new(files: &Files) -> Self {
        let (engine, mut rt) = Engine::headless_for_test(48000, 256);
        rt.publish_for_test();
        let mut app = App::with_loader(engine, Theme::default(), None);
        app.library_metadata =
            library_metadata::Metadata::with_hook(files.0.join("catalog.json"), || {});
        app.library_metadata
            .set_performance(app.engine.cmd.performance().clone());
        app.library_initialized = false;
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut g = Self {
            app,
            callback: OutputCallback::new(rt, 2),
            ctx,
            nodes: vec![],
            time: 0.0,
        };
        g.wait();
        g.click("Audio export & recording");
        g
    }
    fn frame(&mut self, events: Vec<egui::Event>) {
        self.time += 0.02;
        self.callback.renderer_mut_for_test().publish_for_test();
        let out = self.ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1800.0, 1600.0))),
                time: Some(self.time),
                events,
                focused: true,
                ..Default::default()
            },
            |ctx| self.app.update_frame(ctx),
        );
        self.nodes = out.platform_output.accesskit_update.unwrap().nodes;
        self.callback.render(&mut [0.0_f32; 512]);
    }
    fn click(&mut self, label: &str) {
        for _ in 0..2 {
            if let Some(events) = test_support::workspace_menu_events(&self.nodes, label) {
                self.frame(events);
                self.frame(vec![]);
            } else { break; }
        }
        let target = self
            .nodes
            .iter()
            .find(|(_, n)| n.label() == Some(label))
            .map(|(id, _)| *id)
            .unwrap_or_else(|| panic!("Missing {label}"));
        self.frame(vec![egui::Event::AccessKitActionRequest(ActionRequest {
            target,
            action: Action::Click,
            data: None,
        })]);
        self.frame(vec![]);
    }
    fn capture(&mut self)->crate::engine::project::Captured{
        let handle=self.app.engine.project.clone();let capture=std::thread::spawn(move||handle.capture(&AtomicBool::new(false)).unwrap());let end=Instant::now()+std::time::Duration::from_secs(15);while !capture.is_finished(){self.frame(vec![]);assert!(Instant::now()<end);std::thread::sleep(std::time::Duration::from_millis(1));}capture.join().unwrap()
    }
    fn type_named(&mut self,label:&str,value:&str){
        let labels=self.nodes.iter().filter(|(_,n)|n.label()==Some(label)||(n.role()==egui::accesskit::Role::Label&&n.value()==Some(label))).map(|(id,_)|*id).collect::<Vec<_>>();
        let target=self.nodes.iter().find(|(_,n)|n.role()==egui::accesskit::Role::TextInput&&(n.label()==Some(label)||labels.iter().any(|id|n.labelled_by().contains(id)))).map(|(id,_)|*id).unwrap_or_else(||panic!("Missing named text field {label}"));
        self.frame(vec![egui::Event::AccessKitActionRequest(ActionRequest{target,action:Action::Focus,data:None})]);
        let modifiers=egui::Modifiers{ctrl:true,command:true,..Default::default()};
        self.frame(vec![egui::Event::Key{key:Key::A,physical_key:None,pressed:true,repeat:false,modifiers}]);
        self.frame(vec![egui::Event::Key{key:Key::A,physical_key:None,pressed:false,repeat:false,modifiers},egui::Event::Text(value.into())]);
        self.frame(vec![]);
    }
    fn wait(&mut self) {
        let end = Instant::now() + std::time::Duration::from_secs(15);
        loop {
            self.frame(vec![]);
            if self.app.audio_delivery.job.is_none() && !self.app.library_metadata.active() {
                break;
            }
            assert!(Instant::now() < end, "{}", self.app.audio_delivery.message);
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    }
}
#[test]
fn native_source_review_export_decode_and_stale_review_preserve_live_session() {
    let files = Files::new();
    let mut g = Gui::new(&files);
    g.click("Review export source");
    g.wait();
    assert!(g.app.audio_delivery.plan.is_some());
    let before = g.app.snap.project_revision;
    g.app.audio_delivery.request.end = 0.04;
    g.app.audio_delivery.request.tail = 0.0;
    let folder = files.0.join("delivery");
    g.app.audio_delivery.path = folder.to_string_lossy().into_owned();
    g.frame(vec![]);
    g.click("Render master audio");
    g.wait();
    assert!(
        g.app.audio_delivery.message.starts_with("Saved"),
        "{}",
        g.app.audio_delivery.message
    );
    let decoded = crate::engine::decode::decode_audio(&folder.join("master.wav")).unwrap();
    assert_eq!(decoded.sample.frames(), 1920);
    assert_eq!(g.app.snap.project_revision, before);
    assert!(!g.callback.renderer_for_test().playing);
    assert!(g
        .callback
        .renderer_for_test()
        .decks
        .iter()
        .all(|d| !d.playing));
    g.callback
        .renderer_mut_for_test()
        .apply(Command::SetBpm(130.0));
    let refused = files.0.join("stale");
    g.app.audio_delivery.path = refused.to_string_lossy().into_owned();
    g.frame(vec![]);
    g.click("Render master audio");
    g.wait();
    assert!(
        g.app
            .audio_delivery
            .message
            .contains("changed since export review"),
        "{}",
        g.app.audio_delivery.message
    );
    assert!(!refused.exists());
    assert!(folder.join("master.wav").exists());
}

#[test]
fn native_performance_record_stop_decode_and_reviewed_recovery_work_in_performance_mode() {
    use std::io::{Seek, SeekFrom, Write};
    let files = Files::new();
    let mut g = Gui::new(&files);
    g.click("Review export source");
    g.wait();
    g.callback
        .renderer_mut_for_test()
        .apply(Command::PerformanceMode(true));
    g.callback
        .renderer_mut_for_test()
        .apply(Command::DeckPlay { deck: 0 });
    let revision = g.app.engine.project.revision();
    let folder = files.0.join("performance");
    g.app.audio_delivery.record_path = folder.to_string_lossy().into_owned();
    g.app.audio_delivery.record_seconds = 60;
    g.frame(vec![]);
    g.click("Start performance recording");
    let end = Instant::now() + std::time::Duration::from_secs(15);
    while g.app.engine.routing.recorder.frames() < 2048 {
        g.frame(vec![]);
        assert!(Instant::now() < end, "{}", g.app.audio_delivery.message);
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    g.click("Stop performance recording");
    g.wait();
    assert!(
        g.app.audio_delivery.message.starts_with("Recorded"),
        "{}",
        g.app.audio_delivery.message
    );
    let audio = folder.join("take-0001.wav");
    let decoded = crate::engine::decode::decode_audio(&audio).unwrap();
    assert!(decoded.sample.frames() >= 2048);
    assert!(decoded.sample.data.iter().any(|v| v.abs() > 0.001));
    assert_eq!(g.app.engine.project.revision(), revision);
    assert!(g.callback.renderer_for_test().decks[0].playing);
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .open(&audio)
        .unwrap();
    file.seek(SeekFrom::Start(40)).unwrap();
    file.write_all(&0_u32.to_le_bytes()).unwrap();
    drop(file);
    g.app.audio_delivery.recovery_path = folder.to_string_lossy().into_owned();
    g.frame(vec![]);
    g.click("Review recording recovery");
    g.wait();
    assert_eq!(g.app.audio_delivery.recovery.as_ref().unwrap().repairs, 1);
    g.click("Recover reviewed recording");
    g.wait();
    assert!(
        g.app.audio_delivery.message.starts_with("Recovered"),
        "{}",
        g.app.audio_delivery.message
    );
    assert_eq!(
        crate::engine::decode::decode_audio(&audio)
            .unwrap()
            .sample
            .data,
        decoded.sample.data
    );
    g.app.audio_delivery.path = files
        .0
        .join("protected-export")
        .to_string_lossy()
        .into_owned();
    g.frame(vec![]);
    g.click("Render master audio");
    g.wait();
    assert!(!files.0.join("protected-export").exists());
}

#[test]
fn native_final_file_policy_blocks_before_publication_then_warns_with_metadata() {
    let files=Files::new();let mut g=Gui::new(&files);
    for track in &mut g.callback.renderer_mut_for_test().tracks {for clip in &mut track.clips {*clip=crate::engine::Clip::empty();}}
    g.frame(vec![]);g.click("Review export source");g.wait();
    g.app.audio_delivery.request.end=0.5;g.app.audio_delivery.request.tail=0.;
    g.app.audio_delivery.config.metadata.title="Keyboard reachable delivery".into();
    g.app.audio_delivery.config.targets.integrated_lufs=Some(-23.);
    g.app.audio_delivery.config.targets.policy=data::measurement::Policy::Block;
    let blocked=files.0.join("blocked");g.app.audio_delivery.path=blocked.to_string_lossy().into_owned();g.frame(vec![]);g.click("Render master audio");g.wait();
    assert!(g.app.audio_delivery.message.contains("blocked publication"),"{}",g.app.audio_delivery.message);assert!(!blocked.exists());
    g.app.audio_delivery.config.targets.policy=data::measurement::Policy::Warn;
    let warned=files.0.join("warned");g.app.audio_delivery.path=warned.to_string_lossy().into_owned();g.frame(vec![]);g.click("Render master audio");g.wait();
    assert!(g.app.audio_delivery.message.contains("Delivery QC:"),"{}",g.app.audio_delivery.message);
    let value:serde_json::Value=serde_json::from_slice(&std::fs::read(warned.join("export.json")).unwrap()).unwrap();
    assert_eq!(value["metadata"]["title"],"Keyboard reachable delivery");assert!(value["qc"]["integrated_lufs"].is_null());assert_eq!(value["qc"]["frames"],24000);assert_eq!(value["file_sha256"].as_str().unwrap().len(),64);
}

#[test]
fn native_aligned_batch_review_recipe_reopen_render_and_stale_guard_use_real_worker() {
    let files=Files::new();let mut g=Gui::new(&files);g.click("Review export source");g.wait();
    g.app.audio_delivery.request.end=0.5;g.app.audio_delivery.request.tail=0.;
    g.app.audio_delivery.batch.as_mut().unwrap().items.truncate(2);
    g.app.audio_delivery.batch.as_mut().unwrap().items[0].name="preserved-custom".into();
    g.app.audio_delivery.request.options.normalize=true;
    g.click("Aligned stems & versions");
    let recipe=files.0.join("reviewed.json");g.app.audio_delivery.recipe_path=recipe.to_string_lossy().into_owned();g.frame(vec![]);g.click("Save batch recipe to new file");g.wait();assert!(recipe.is_file());assert!(!data::batch::Recipe::load(&recipe,&AtomicBool::new(false)).unwrap().export.options.normalize);
    g.app.audio_delivery.batch.as_mut().unwrap().items.clear();g.click("Reopen batch recipe");g.wait();assert_eq!(g.app.audio_delivery.batch.as_ref().unwrap().items.len(),2);
    let folder=files.0.join("batch");g.app.audio_delivery.path=folder.to_string_lossy().into_owned();let before=g.app.snap.project_revision;g.frame(vec![]);g.click("Render aligned batch");g.wait();assert!(g.app.audio_delivery.message.starts_with("Saved aligned batch"),"{}",g.app.audio_delivery.message);assert!(folder.join("batch.json").is_file());assert_eq!(g.app.snap.project_revision,before);
    g.callback.renderer_mut_for_test().apply(Command::SetBpm(131.));let stale=files.0.join("stale-batch");g.app.audio_delivery.path=stale.to_string_lossy().into_owned();g.frame(vec![]);g.click("Render aligned batch");g.wait();assert!(g.app.audio_delivery.message.contains("changed since batch review"),"{}",g.app.audio_delivery.message);assert!(!stale.exists());
    g.click("Review export source");g.wait();let refreshed=g.app.audio_delivery.batch.as_ref().unwrap();assert_eq!(refreshed.items.len(),2);assert_eq!(refreshed.items[0].name,"preserved-custom");
}

#[test]
fn native_document_cli_plan_reopen_and_batch_use_same_encoded_delivery_path(){
    for delete_first in [false,true] {
    let files=Files::new();let mut g=Gui::new(&files);let mut captured=g.capture();
    if delete_first {let layout=captured.state.session.as_ref().unwrap();let(request,_)=crate::engine::session::Request::metadata(layout,captured.checkpoint.epoch,crate::engine::session::Action::Delete{axis:crate::engine::session::Axis::Scene,id:layout.scenes[0].id}).unwrap();g.callback.renderer_mut_for_test().apply(Command::session_edit(request));captured=g.capture();assert!(!captured.state.session.as_ref().unwrap().scenes[0].active);}
    let layout=captured.state.session.as_ref().unwrap();let source_slot=layout.scene_order[0];let source_id=layout.scenes[usize::from(source_slot)].id;
let document=project::Document{engine:captured.state,view:project::UiState::default(),mapping_schema:project::FACTORY_MAPPING_SCHEMA};document.validate().unwrap();let bundle=crate::project_file::Bundle{state:document,media:captured.media};let native=files.0.join("source.omat");crate::project_file::save(&native,&bundle,crate::project_file::Overwrite::Never,&Default::default(),&AtomicBool::new(false)).unwrap();
    let recipe=files.0.join("cli.json");assert!(data::cli::run(&["delivery-plan".into(),native.to_string_lossy().into_owned(),recipe.to_string_lossy().into_owned()]).unwrap());let mut r=data::batch::Recipe::load(&recipe,&AtomicBool::new(false)).unwrap();assert_eq!(r.export.scene,source_slot);assert_eq!(r.source_scene.unwrap().id,source_id);r.export.end=0.5;r.export.tail=0.;r.items.truncate(2);std::fs::remove_file(&recipe).unwrap();r.save(&recipe).unwrap();
    let destination=files.0.join("cli-output");assert!(data::cli::run(&["delivery-batch".into(),native.to_string_lossy().into_owned(),recipe.to_string_lossy().into_owned(),destination.to_string_lossy().into_owned()]).unwrap());let report:serde_json::Value=serde_json::from_slice(&std::fs::read(destination.join("batch.json")).unwrap()).unwrap();assert_eq!(report["cases"].as_array().unwrap().len(),2);assert_eq!(report["frames_per_pass"],24000);assert_eq!(report["review"]["recipe"]["project_sha256"],r.project_sha256.as_ref().unwrap().as_str());
    let untouched=files.0.join("changed-plan");r.project_sha256=Some("0".repeat(64));let changed=files.0.join("changed.json");r.save(&changed).unwrap();assert!(data::cli::run(&["delivery-batch".into(),native.to_string_lossy().into_owned(),changed.to_string_lossy().into_owned(),untouched.to_string_lossy().into_owned()]).unwrap_err().contains("Project bytes differ"));assert!(!untouched.exists());
    }
}

#[test]
fn native_delivery_named_fields_accept_keyboard_text_and_keep_live_music_unchanged(){
    let files=Files::new();let mut g=Gui::new(&files);g.click("Review export source");g.wait();let revision=g.app.engine.project.revision();
    g.click("Delivery metadata & final-file QC");g.type_named("Title","Keyboard mix");g.type_named("ISRC","USAAA2600001");
    assert_eq!(g.app.audio_delivery.config.metadata.title,"Keyboard mix");assert_eq!(g.app.audio_delivery.config.metadata.isrc,"USAAA2600001");
    g.click("Delivery metadata & final-file QC");g.click("Aligned stems & versions");g.type_named("New pass name","keyboard-version");
    assert_eq!(g.app.audio_delivery.version_name,"keyboard-version");
    let path=files.0.join("keyboard-recipe.json");g.type_named("Batch recipe JSON path",path.to_str().unwrap());g.click("Save batch recipe to new file");g.wait();
    let reopened=data::batch::Recipe::load(&path,&AtomicBool::new(false)).unwrap();assert_eq!(reopened.config.metadata.title,"Keyboard mix");assert_eq!(reopened.config.metadata.isrc,"USAAA2600001");assert_eq!(g.app.engine.project.revision(),revision);assert!(!g.callback.renderer_for_test().playing);
}
