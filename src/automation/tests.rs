use super::*;
use crate::engine::{Engine, RtEngine};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};

#[test]
fn ns7_controls_api_jobs_apply_and_invalid_targets_or_schedules_refuse() {
    let mut service = Service::new(); let namespace = service.state()["expected"]["namespace"].clone();
    for control in [json!({"op":"keylock"}), json!({"op":"hold","button":"reverse","on":true}), json!({"op":"hold","button":"reverse","on":false})] {
        let response = service.query(json!({"op":"command","namespace":namespace,"action":{"op":"deck_control","deck":0,"control":control}}));
        assert_eq!(response["ok"], true, "{response}");
        assert_eq!(service.complete(&response["result"]["job"])["result"]["status"], "applied");
    }
    assert!(service.rt.decks[0].keylock); service.publish(); assert_eq!(service.state()["decks"][0]["controls"]["reverse"], false);
    for action in [json!({"op":"deck_control","deck":2,"control":{"op":"keylock"}}), json!({"op":"deck_control","deck":0,"control":{"op":"strip","value":2.0}})] {
        assert_eq!(service.query(json!({"op":"command","namespace":namespace,"action":action}))["ok"], false);
    }
    assert_eq!(service.query(json!({"op":"schedule","namespace":namespace,"beat":4.0,"action":{"op":"deck_control","deck":0,"control":{"op":"keylock"}}}))["error_code"], "invalid_schedule");
}
use std::path::PathBuf;
use std::time::Instant;

#[test]
fn beat_jump_api_jobs_report_applied_positions_sizes_and_strict_immediate_targets() {
    let mut service = Service::new();
    let namespace = service.state()["expected"]["namespace"].clone();
    for deck in 0..2 {
        service.rt.decks[deck].pos = service.rt.decks[deck].audio.as_ref().unwrap().frames() as f64 * 0.5;
        let old = service.rt.decks[deck].pos; let other = service.rt.decks[1 - deck].pos;
        for control in [json!({"op":"beat_jump_size","index":3}), json!({"op":"beat_jump","forward":true})] {
            let response = service.query(json!({"op":"command","namespace":namespace,"action":{"op":"deck_control","deck":deck,"control":control}}));
            assert_eq!(response["ok"], true); assert_eq!(service.complete(&response["result"]["job"])["result"]["status"], "applied");
        }
        assert!(service.rt.decks[deck].pos > old); assert_eq!(service.rt.decks[1 - deck].pos, other);
        let response = service.query(json!({"op":"command","namespace":namespace,"action":{"op":"deck_control","deck":deck,"control":{"op":"beat_jump","forward":false}}}));
        assert_eq!(service.complete(&response["result"]["job"])["result"]["status"], "applied");
        assert!((service.rt.decks[deck].pos - old).abs() < 1.0e-6); assert!(!service.rt.decks[deck].playing);
        service.publish(); assert_eq!(service.state()["decks"][deck]["controls"]["beat_jump_size"], 3);
    }
    for control in [json!({"op":"beat_jump_size","index":10}), json!({"op":"beat_jump_size","index":-1}), json!({"op":"beat_jump","forward":0}), json!({"op":"beat_jump","forward":true,"extra":1})] {
        assert_eq!(service.query(json!({"op":"command","namespace":namespace,"action":{"op":"deck_control","deck":0,"control":control}}))["ok"], false);
    }
    assert_eq!(service.query(json!({"op":"schedule","namespace":namespace,"beat":4.0,"action":{"op":"deck_control","deck":0,"control":{"op":"beat_jump","forward":true}}}))["error_code"], "invalid_schedule");
}

#[test]
fn loop_edit_api_reports_stale_or_unusable_regions_as_rejected_and_applied_exact_bounds() {
    let mut service=Service::new();let namespace=service.state()["expected"]["namespace"].clone();
    let key=service.state()["decks"][0]["media_key"].clone();
    let request=|control|json!({"op":"command","namespace":namespace,"action":{"op":"deck_control","deck":0,"control":control}});
    let response=service.query(request(json!({"op":"loop_bounds","media_key":key,"start_seconds":0.1,"end_seconds":0.2})));
    assert_eq!(response["ok"],true);assert_eq!(service.complete(&response["result"]["job"])["result"]["status"],"applied");
    let old=(service.rt.decks[0].loop_start,service.rt.decks[0].loop_len);
    for control in [json!({"op":"loop_bounds","media_key":key,"start_seconds":0.1,"end_seconds":100000}),
        json!({"op":"loop_length","media_key":(key.as_str().unwrap().parse::<u64>().unwrap()+1).to_string(),"beats":4})] {
        let response=service.query(request(control));assert_eq!(response["ok"],true);
        assert_eq!(service.complete(&response["result"]["job"])["result"]["status"],"rejected");
        assert_eq!((service.rt.decks[0].loop_start,service.rt.decks[0].loop_len),old);
    }
    for control in [json!({"op":"loop_bounds","media_key":key,"start_seconds":0.2,"end_seconds":0.1}),
        json!({"op":"loop_length","media_key":key,"beats":0}),
        json!({"op":"loop_move","media_key":key,"beats":0}),
        json!({"op":"loop_move","media_key":key,"beats":1,"unknown":1}),
        json!({"op":"loop_move","media_key":9223372036854775809_u64,"beats":1}),
        json!({"op":"loop_move","media_key":"01","beats":1})] {
        assert_eq!(service.query(request(control))["ok"],false);
    }
    service.publish();assert!(!service.state()["decks"][0]["playing"].as_bool().unwrap());
    assert_eq!(service.state()["decks"][1]["loop_region"],serde_json::Value::Null);
}

#[test]
fn deck_quantization_api_reports_pending_onsets_separately_from_accepted_gestures() {
    let mut service = Service::new();
    let namespace = service.state()["expected"]["namespace"].clone();
    let request = |control| json!({"op":"command","namespace":namespace,"action":{"op":"deck_control","deck":0,"control":control}});
    for invalid in [json!({"op":"quantize","enabled":true,"division":6}), json!({"op":"quantize","enabled":1,"division":3}),
        json!({"op":"quantize","enabled":true,"division":3,"unknown":1})] {
        assert_eq!(service.query(request(invalid))["ok"], false);
    }
    let response = service.query(request(json!({"op":"quantize","enabled":true,"division":1})));
    assert_eq!(service.complete(&response["result"]["job"])["result"]["status"], "applied");
    service.publish(); let state = service.state();
    assert_eq!(state["decks"][0]["controls"]["quantize"], true);
    assert_eq!(state["decks"][0]["controls"]["quantize_division"], 1);
    assert_eq!(state["decks"][1]["controls"]["quantize"], false);
    service.rt.apply(Command::DeckSeek { deck: 0, frac: 0.2 });
    service.rt.apply(Command::DeckHotCue { deck: 0, pad: 0, del: false });
    service.rt.apply(Command::DeckPlay { deck: 0 });
    service.rt.apply(Command::DeckSeek { deck: 0, frac: 0.031 });
    let response = service.query(request(json!({"op":"hold","button":{"hot_cue":0},"on":true})));
    assert_eq!(response["ok"], true, "{response}");
    assert_eq!(service.complete(&response["result"]["job"])["result"]["status"], "applied");
    service.publish(); assert_eq!(service.state()["decks"][0]["controls"]["pending"]["action"]["kind"], "hot_cue");
    let response = service.query(request(json!({"op":"hold","button":{"hot_cue":0},"on":false})));
    assert_eq!(service.complete(&response["result"]["job"])["result"]["status"], "applied");
    service.publish(); assert!(service.state()["decks"][0]["controls"]["pending"].is_null());
}

#[test]
fn shipped_cli_builds_typed_versioned_envelopes_before_connecting() {
    let args = vec!["api".into(), r#"{"op":"discover"}"#.into()];
    assert_eq!(
        crate::automation_payload(&args).unwrap(),
        json!({"op":"api","version":1,"request":{"op":"discover"}})
    );
    assert!(crate::automation_payload(&args[..1]).is_err());
    assert!(crate::automation_payload(&["api".into(), r#"{"op":"unknown"}"#.into()]).is_err());
    assert!(crate::automation_payload(&["api".into(), "{".into()]).is_err());
}

#[test]
fn shipped_cli_exchange_and_subscription_use_the_actual_api_socket() {
    let service = Service::new();
    let path = service.root.join("api.sock");
    let payload =
        crate::automation_payload(&["api".into(), r#"{"op":"discover"}"#.into()]).unwrap();
    let reply =
        crate::exchange_request(UnixStream::connect(&path).unwrap(), &payload.to_string()).unwrap();
    let reply: Value = serde_json::from_str(&reply).unwrap();
    assert_eq!(reply["version"], 1);
    assert_eq!(reply["result"]["version"], 1);
    let rejected = crate::exchange_request(
        UnixStream::connect(&path).unwrap(),
        r#"{"op":"api","version":99,"request":{"op":"discover"}}"#,
    )
    .unwrap_err();
    assert!(rejected.to_string().contains("rejected"));
    struct TwoFrames(Vec<u8>);
    impl Write for TwoFrames {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.extend_from_slice(bytes);
            if self.0.iter().filter(|byte| **byte == b'\n').count() >= 2 {
                return Err(std::io::Error::other("consumer finished"));
            }
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut output = TwoFrames(Vec::new());
    let payload =
        crate::automation_payload(&["api".into(), r#"{"op":"subscribe"}"#.into()]).unwrap();
    assert!(follow(&path, &payload, &mut output)
        .unwrap_err()
        .to_string()
        .contains("consumer finished"));
    let frames: Vec<Value> = std::str::from_utf8(&output.0)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(frames.len(), 2);
    assert!(frames.iter().all(|frame| frame["ok"] == true
        && frame["version"] == 1
        && frame["result"]["expected"].is_object()));
    assert_eq!(frames[1]["event"], "state");
}

#[test]
fn shipped_subscription_rejects_malformed_success_fields_before_emitting_state() {
    let service = Service::new();
    for (index, invalid) in [json!(null), json!("yes"), json!(1)]
        .into_iter()
        .enumerate()
    {
        let path = service.root.join(format!("malformed-{index}.sock"));
        let listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut line = String::new();
            BufReader::new(stream.try_clone().unwrap())
                .read_line(&mut line)
                .unwrap();
            let request: Value = serde_json::from_str(&line).unwrap();
            writeln!(
                stream,
                "{}",
                json!({"version":1,"id":request["id"],"ok":invalid,"result":{}})
            )
            .unwrap();
        });
        let mut output = Vec::new();
        let error = follow(
            &path,
            &json!({"op":"api","version":1,"request":{"op":"subscribe"}}),
            &mut output,
        )
        .unwrap_err();
        assert!(error.to_string().contains("subscription rejected"));
        assert!(output.is_empty());
        server.join().unwrap();
    }
}

struct Service {
    engine: Engine,
    rt: RtEngine,
    _server: crate::ipc_server::IpcServer,
    root: PathBuf,
}
impl Service {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "api-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&root)
            .unwrap();
        let (engine, mut rt) = Engine::headless_for_test(48000, 256);
        rt.process(&mut []);
        let server = crate::ipc_server::start_at(
            &root.join("api.sock"),
            engine.cmd.clone(),
            engine.snap.clone(),
        )
        .unwrap();
        let mut service = Self {
            engine,
            rt,
            _server: server,
            root,
        };
        service.publish();
        service
    }
    fn publish(&mut self) {
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            self.rt.process(&mut []);
            self.rt.publish_for_test();
            let s = self.engine.snapshot();
            if s.sample_rate == 48000
                && s.project_revision == self.engine.project.revision()
                && s.sampler_epoch == self.engine.undo.checkpoint().epoch
                && s.transport_epoch == self.rt.transport_epoch
                && s.playing == self.rt.playing
            {
                break;
            }
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    fn client(&self) -> Client {
        let socket = UnixStream::connect(self.root.join("api.sock")).unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        Client(BufReader::new(socket))
    }
    fn query(&self, request: Value) -> Value {
        self.client().query(request)
    }
    fn state(&self) -> Value {
        self.query(json!({"op":"state"}))["result"].clone()
    }
    fn complete(&mut self, id: &Value) -> Value {
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            self.rt.process(&mut []);
            let response = self.query(json!({"op":"job","id":id}));
            if response["result"]["status"] != "pending" {
                return response;
            }
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
    }
}
impl Drop for Service {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
struct Client(BufReader<UnixStream>);
impl Client {
    fn query(&mut self, request: Value) -> Value {
        self.envelope(json!({"op":"api","version":1,"id":"fixture","request":request}))
    }
    fn envelope(&mut self, request: Value) -> Value {
        writeln!(self.0.get_mut(), "{request}").unwrap();
        self.read()
    }
    fn read(&mut self) -> Value {
        let mut line = String::new();
        self.0.read_line(&mut line).unwrap();
        serde_json::from_str(&line).unwrap()
    }
}

#[test]
fn discover_state_and_commands_work_over_the_real_private_socket_and_reconnect() {
    let mut service = Service::new();
    assert_eq!(
        std::fs::metadata(service.root.join("api.sock"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    let discovery = service.query(json!({"op":"discover"}));
    assert_eq!(discovery["result"]["version"], 1);
    assert!(discovery["result"]["actions"]["track_gain"].is_object());
    let state = service.state();
    let target = state["objects"][2]["target"].clone();
    assert_eq!(state["expected"]["namespace"].as_str().unwrap().len(), 32);
    assert_eq!(target["id"].as_str().unwrap().len(), 16);
    let accepted=service.query(json!({"op":"command","namespace":state["expected"]["namespace"],"action":{"op":"track_gain","target":target,"value":0.17}}));
    assert_eq!(accepted["ok"], true);
    assert_eq!(accepted["result"]["status"], "pending");
    let completed = service.complete(&accepted["result"]["job"]);
    assert_eq!(completed["result"]["status"], "applied");
    assert_eq!(service.rt.tracks[2].gain, 0.17);
    let again = service.query(json!({"op":"job","id":accepted["result"]["job"]}));
    assert_eq!(again["result"]["status"], "applied");
}

#[test]
fn concurrent_atomic_edits_reject_stale_state_and_preserve_native_undo() {
    let mut service = Service::new();
    let state = service.state();
    let target = state["objects"][2]["target"].clone();
    let name = service.rt.session.tracks[2].name.clone();
    let mut first = service.client();
    let mut second = service.client();
    let edit = |name| json!({"op":"edit","expected":state["expected"],"target":target,"action":{"op":"rename","name":name}});
    let one = first.query(edit("Remote one"));
    let two = second.query(edit("Remote two"));
    assert_eq!(one["ok"], true);
    assert_eq!(two["ok"], true);
    service.rt.process(&mut []);
    assert_eq!(
        service.complete(&one["result"]["job"])["result"]["status"],
        "applied"
    );
    assert_eq!(
        service.complete(&two["result"]["job"])["result"]["status"],
        "rejected"
    );
    assert_eq!(service.rt.session.tracks[2].name, "Remote one");
    service.engine.send(Command::Undo).unwrap();
    service.rt.process(&mut []);
    assert_eq!(service.rt.session.tracks[2].name, name);
    service.publish();
    let stale = service.query(edit("Stale"));
    assert_eq!(stale["error_code"], "conflict");
}

#[test]
fn musical_jobs_survive_disconnect_and_cancel_without_applying() {
    let mut service = Service::new();
    service.rt.apply(Command::Play);
    service.publish();
    let state = service.state();
    let before = service.rt.master;
    let accepted=service.query(json!({"op":"schedule","namespace":state["expected"]["namespace"],"beat":state["beat"].as_f64().unwrap()+4.0,"action":{"op":"master_gain","value":0.2}}));
    assert_eq!(accepted["ok"], true);
    service.rt.process(&mut []);
    let cancelled = service.query(json!({"op":"cancel","id":accepted["result"]["job"]}));
    assert_eq!(cancelled["result"]["status"], "cancelled");
    service.rt.beat += 8.0;
    service.rt.process(&mut [0.0; 2]);
    assert_eq!(service.rt.master, before);
    assert_eq!(
        service.query(json!({"op":"cancel","id":accepted["result"]["job"]}))["error_code"],
        "cancel_conflict"
    );
}

#[test]
fn malformed_versions_values_targets_confirmation_and_blocked_snapshots_fail_explicitly() {
    let service = Service::new();
    let state = service.state();
    let mut client = service.client();
    assert_eq!(
        client.envelope(json!({"op":"api","version":2,"request":{"op":"discover"}}))["error_code"],
        "unsupported_version"
    );
    for request in [
        json!({"op":"future_operation"}),
        json!({"op":"discover","future_field":true}),
    ] {
        assert_eq!(
            service
                .client()
                .envelope(json!({"op":"api","version":99,"request":request}))["error_code"],
            "unsupported_version"
        );
    }
    assert_eq!(
        service.query(json!({"op":"state","extra":1}))["error_code"],
        "invalid_operation"
    );
    assert_eq!(
        service.query(json!({"op":"state","page":{"limit":17}}))["error_code"],
        "invalid_operation"
    );
    assert_eq!(
        service.query(json!({"op":"emergency_silence","confirm":false}))["error_code"],
        "invalid_operation"
    );
    assert_eq!(service.query(json!({"op":"command","namespace":state["expected"]["namespace"],"action":{"op":"master_gain","value":1.6}}))["error_code"],"invalid_operation");
    let mut target = state["objects"][0]["target"].clone();
    target["id"] = json!("ffffffffffffffff");
    assert_eq!(service.query(json!({"op":"command","namespace":state["expected"]["namespace"],"action":{"op":"track_gain","target":target,"value":0.2}}))["error_code"],"invalid_target");
    let locked = service.engine.snap.lock();
    assert_eq!(
        service.query(json!({"op":"state"}))["error_code"],
        "snapshot_unavailable"
    );
    drop(locked);
    assert_eq!(
        service.query(json!({"op":"job","id":"missing"}))["error_code"],
        "job_expired"
    );
}

#[test]
fn state_subscription_reconnects_and_updates_after_native_changes() {
    let mut service = Service::new();
    let mut client = service.client();
    assert_eq!(client.query(json!({"op":"subscribe"}))["ok"], true);
    service.rt.apply(Command::Play);
    service.publish();
    assert_eq!(client.read()["result"]["playing"], true);
    drop(client);
    let mut reconnected = service.client();
    assert_eq!(
        reconnected.query(json!({"op":"subscribe"}))["result"]["playing"],
        true
    );
    service.rt.apply(Command::Stop);
    service.publish();
    assert_eq!(reconnected.read()["result"]["playing"], false);
}

#[test]
fn state_pages_remain_within_the_wire_budget_for_escaped_names_and_maximal_counters() {
    let service = Service::new();
    let snapshot = Arc::new(Mutex::new(service.engine.snapshot()));
    {
        let mut snapshot = snapshot.lock();
        let layout = snapshot.session.as_mut().unwrap();
        *layout = session::Layout::legacy((0..128).map(|_| "\u{0001}".repeat(4096)), 512);
        for item in &mut layout.scenes {
            item.name = "\u{0001}".repeat(4096);
            item.color = Some([255, 255, 255]);
        }
    }
    for axis in ["track", "scene"] {
        let request = json!({"op":"api","version":1,"id":"\u{0001}".repeat(128),"request":{"op":"state","page":{"axis":axis,"limit":16}}});
        let (mut response, _) = reply(
            &request,
            &service.engine.cmd,
            &snapshot,
            Limits::default(),
            true,
        );
        assert_eq!(response["ok"], true);
        for value in response["result"]["performance"]
            .as_object_mut()
            .unwrap()
            .values_mut()
        {
            if value.is_u64() {
                *value = json!(u64::MAX);
            }
        }
        assert!(serde_json::to_vec(&response).unwrap().len() < ipc_transport::RESPONSE_BYTES);
        assert_eq!(response["result"]["objects"].as_array().unwrap().len(), 16);
    }
}

#[test]
fn now_playing_api_bounds_worst_case_labels_and_drops_expired_or_disabled_data() {
    let service=Service::new();let feed=service.engine.cmd.now_playing();
    assert_eq!(service.query(json!({"op":"now_playing"}))["result"]["status"],"disabled");
    let config=crate::performance_history::now_playing::Config{enabled:true,title:true,artist:true,identity:true};feed.configure(config);
    let (generation,_)=feed.config();
    let source=crate::performance_history::Source::Catalog{track_id:"a".repeat(32),version:u32::MAX,title:"\\".repeat(1024),artist:"音".repeat(341)};
    feed.publish(generation,true,[Some(source.clone()),Some(source)]);
    let value=service.query(json!({"op":"now_playing"}));assert_eq!(value["ok"],true);assert!(value.to_string().len()<crate::ipc_transport::RESPONSE_BYTES);
    assert_eq!(value["result"]["decks"][0]["labels_truncated"],true);assert_eq!(value["result"]["decks"][0]["title"].as_str().unwrap().len(),512);assert_eq!(value["result"]["decks"][0]["artist"].as_str().unwrap().len(),510);
    assert!(service.query(json!({"op":"discover"}))["result"]["requests"].get("now_playing").is_some());
    std::thread::sleep(Duration::from_millis(1050));
    let value=service.query(json!({"op":"now_playing"}));assert_eq!(value["result"]["status"],"stale");assert_eq!(value["result"]["decks"],json!([]));
    feed.configure(Default::default());feed.publish(generation,true,[Some(crate::performance_history::Source::Unresolved),None]);
    assert_eq!(service.query(json!({"op":"now_playing"}))["result"]["status"],"disabled");
}

#[test]
fn headphone_api_validates_values_and_rejects_unavailable_checks_without_changing_program() {
    let mut service=Service::new();let namespace=service.state()["expected"]["namespace"].clone();
    let request=|control|json!({"op":"command","namespace":namespace,"action":{"op":"monitor","control":control}});
    let master=service.rt.master;
    for control in [json!({"op":"source","value":"pfl"}),json!({"op":"volume","value":0.5}),
        json!({"op":"blend","value":0.75}),json!({"op":"split","value":true}),
        json!({"op":"pfl","value":{"deck":0,"enabled":true}})] {
        let response=service.query(request(control));assert_eq!(response["ok"],true,"{response}");
        assert_eq!(service.complete(&response["result"]["job"])["result"]["status"],"applied");
    }
    service.publish();let state=service.state();assert_eq!(state["monitor"]["source"],"pfl");
    assert_eq!(state["monitor"]["volume"],0.5);assert_eq!(state["monitor"]["blend"],0.75);assert_eq!(state["monitor"]["split"],true);
    assert_eq!(service.rt.master,master);
    let revision=state["expected"]["revision"].clone();
    service.rt.apply(Command::Undo);service.publish();
    assert_eq!(service.state()["monitor"]["blend"],0.0);
    assert_ne!(service.state()["expected"]["revision"],revision);
    service.rt.apply(Command::Redo);service.publish();assert_eq!(service.state()["monitor"]["blend"],0.75);
    for control in [json!({"op":"volume","value":1.1}),json!({"op":"split","value":1}),json!({"op":"tone","value":2}),
        json!({"op":"pfl","value":{"deck":2,"enabled":true}}),json!({"op":"volume","value":0.5,"extra":1})] {
        assert_eq!(service.query(request(control))["ok"],false);
    }
    let response=service.query(request(json!({"op":"tone","value":0})));assert_eq!(response["ok"],true);
    assert_eq!(service.complete(&response["result"]["job"])["result"]["status"],"rejected");
    assert_eq!(service.query(json!({"op":"schedule","namespace":namespace,"beat":4,"action":{"op":"monitor","control":{"op":"volume","value":0.5}}}))["error_code"],"invalid_schedule");
}

#[test]
fn track_input_api_retains_exact_targets_modes_arm_cue_and_saved_undo() {
    use crate::engine::input_monitor::Mode;
    let mut service = Service::new(); let state = service.state();
    let target = state["objects"][2]["target"].clone(); let namespace = state["expected"]["namespace"].clone();
    for action in [json!({"op":"track_monitor","target":target,"mode":"auto"}),json!({"op":"track_arm","target":target,"value":true}),json!({"op":"track_cue","target":target,"value":true})] {
        let response = service.query(json!({"op":"command","namespace":namespace,"action":action})); assert_eq!(response["ok"],true,"{response}"); assert_eq!(service.complete(&response["result"]["job"])["result"]["status"],"applied");
    }
    assert_eq!(service.rt.tracks[2].input_monitor,Some(Mode::Auto)); assert!(service.rt.tracks[2].armed && service.rt.tracks[2].pfl);
    service.publish(); let input=service.state()["objects"][2]["input"].clone(); assert_eq!(input["mode"],"auto"); assert_eq!(input["armed"],true); assert_eq!(input["cue"],true);
    service.rt.apply(Command::Undo); assert!(!service.rt.tracks[2].armed); service.rt.apply(Command::Undo); assert_eq!(service.rt.tracks[2].input_monitor,None);
    service.rt.apply(Command::Redo); assert_eq!(service.rt.tracks[2].input_monitor,Some(Mode::Auto));
    for action in [json!({"op":"track_monitor","target":target,"mode":"sideways"}), json!({"op":"track_monitor","target":target,"mode":0}), json!({"op":"track_arm","target":target,"value":1}), json!({"op":"track_cue","target":target,"value":true,"extra":true})] { assert_eq!(service.query(json!({"op":"command","namespace":namespace,"action":action}))["ok"],false); }
    let response = service.query(json!({"op":"command","namespace":namespace,"action":{"op":"track_monitor","target":target,"mode":"off"}})); assert_eq!(response["ok"],true);
    let id = service.rt.session.tracks[2].id;
    let (remove, _) = session::Request::metadata(&service.rt.session, service.engine.undo.checkpoint().epoch, session::Action::Delete { axis: session::Axis::Track, id }).unwrap();
    service.rt.apply(Command::session_edit(remove));
    assert_eq!(service.complete(&response["result"]["job"])["result"]["status"],"rejected"); assert_eq!(service.rt.tracks[2].input_monitor,Some(Mode::Auto)); assert!(!service.rt.session.tracks[2].active);
}
