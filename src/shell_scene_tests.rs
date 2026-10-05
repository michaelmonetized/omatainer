//! Opt-in real Quickshell -> production CLI -> private application protocol
//! fixture. No audio/MIDI device, desktop service or public socket is opened.
use super::*;
use engine::{ClipKind, CommandPort, RtEngine, Snapshot, SCENES};
use parking_lot::Mutex;
use std::os::unix::fs::DirBuilderExt;
use std::path::PathBuf;
use std::process::{Child, Command as ProcessCommand, Stdio};
use std::sync::Arc;
use std::time::Instant;

struct Directory(PathBuf);
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
struct Running(Option<Child>);
impl Drop for Running {
    fn drop(&mut self) {
        if let Some(child) = &mut self.0 {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

#[test]
#[ignore = "real offscreen Quickshell; requires freshly built OMATAINER_TEST_BINARY"]
fn native_shell_scene_arguments_reach_distinct_renderer_scenes() {
    let binary = std::env::var_os("OMATAINER_TEST_BINARY").expect("build production CLI first");
    let directory = Directory(std::env::temp_dir().join(format!(
        "omatainer-shell-scene-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    )));
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&directory.0)
        .unwrap();
    let (commands, accepted) = CommandPort::channel(48);
    let snapshot = Arc::new(Mutex::new(Snapshot::default()));
    let server = ipc_server::start_at(
        &directory.0.join("omatainer.sock"),
        commands.clone(),
        snapshot.clone(),
    )
    .unwrap();
    // The fixture observes each admitted command before applying it to the
    // actual engine, so intermediate scenes cannot collapse in one test block.
    // Normal callback draining already has its separate scene/control tests.
    let (_unused, render_input) = crossbeam_channel::bounded(32);
    let mut rt = RtEngine::new(48_000.0, render_input, snapshot);
    rt.quant = 0.0;
    for track in &mut rt.tracks {
        for clip in &mut track.clips {
            clip.kind = ClipKind::Midi;
        }
    }
    let mut child = Running(Some(
        ProcessCommand::new("python3")
            .arg(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/scripts/check-shell-scenes.py"
            ))
            .arg("--cli")
            .arg(binary)
            .arg("--runtime")
            .arg(&directory.0)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    ));
    let deadline = Instant::now() + Duration::from_secs(45);
    let mut observed = Vec::new();
    loop {
        while let Ok(command) = accepted.try_recv() {
            let Command::LaunchScene { scene } = command else {
                panic!("unexpected shell scene command {command:?}");
            };
            assert!((scene as usize) < SCENES);
            rt.apply(Command::LaunchScene { scene });
            assert_eq!(rt.selected_scene, scene as usize);
            assert!(rt
                .tracks
                .iter()
                .all(|track| track.playing.unwrap().scene == scene));
            observed.push(scene);
        }
        if child.0.as_mut().unwrap().try_wait().unwrap().is_some() && accepted.is_empty() {
            break;
        }
        assert!(Instant::now() < deadline, "shell scene harness timed out");
        std::thread::sleep(Duration::from_millis(2));
    }
    let output = child.0.take().unwrap().wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(observed, [0, 3, 7, 1]);
    assert!(accepted.is_empty());
    assert_eq!(rt.selected_scene, 1);
    drop(server);
    assert!(!directory.0.join("omatainer.sock").exists());
    println!("{}", String::from_utf8_lossy(&output.stdout));
    println!("native application harness applied zero-based scenes {observed:?}");
}
