
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
