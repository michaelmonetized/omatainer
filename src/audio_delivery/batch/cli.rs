use super::*;
use crate::{engine::project, project_file};
use sha2::{Digest, Sha256};
use std::{
    fs::OpenOptions,
    io::{Read, Seek, SeekFrom},
    os::unix::fs::OpenOptionsExt,
    sync::atomic::{AtomicBool, Ordering},
};
#[derive(Deserialize)]
struct AudioDocument {
    engine: project::State,
    mapping_schema: u32,
}

/// Load the existing native container without starting a device or the GUI.
/// Args: saved .omatainer document and cancellation. Returns the validated audio state/media plus a digest of the exact open inode, rejecting modification during load.
fn capture(path: &Path, cancel: &AtomicBool) -> Result<(Captured, String), String> {
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .map_err(|e| e.to_string())?;
    let hash = |file: &mut std::fs::File| -> Result<String, String> {
        file.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
        let mut hash = Sha256::new();
        let mut bytes = [0; 65536];
        loop {
            super::super::active(cancel)?;
            let n = file.read(&mut bytes).map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            hash.update(&bytes[..n]);
        }
        Ok(format!("{:x}", hash.finalize()))
    };
    if !file.metadata().map_err(|e| e.to_string())?.is_file() {
        return Err("Delivery needs a regular native project".into());
    }
    let before = hash(&mut file)?;
    file.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
    let bundle = project_file::load_from_file::<AudioDocument>(
        file.try_clone().map_err(|e| e.to_string())?,
        &Default::default(),
        cancel,
    )
    .map_err(|e| e.to_string())?;
    if before != hash(&mut file)? {
        return Err("Native project changed during delivery load".into());
    }
    if bundle.state.mapping_schema != 1 {
        return Err("Unsupported native document mapping schema".into());
    }
    bundle.state.engine.validate(&bundle.media)?;
    Ok((
        Captured {
            checkpoint: Default::default(),
            state: bundle.state.engine,
            media: bundle.media,
            revision: 0,
            playback_receipts: Default::default(),
        },
        before,
    ))
}

pub(crate) fn run(args: &[String]) -> Result<bool, String> {
    let Some(command) = args.first().map(String::as_str) else {
        return Ok(false);
    };
    match command {
        "delivery-plan" => {
            if args.len() != 3 {
                return Err(
                    "usage: omatainer delivery-plan PROJECT.omatainer NEW_RECIPE.json".into(),
                );
            }
            let (capture, hash) = capture(Path::new(&args[1]), &AtomicBool::new(false))?;
            let mut export = Export::default();
            if let Some(model) = &capture.state.routing {
                let outputs = model
                    .ports
                    .iter()
                    .filter(|p| {
                        p.direction == crate::engine::audio::routing::model::Direction::Output
                            && model.monitor_output != Some(p.id)
                    })
                    .collect::<Vec<_>>();
                if outputs.len() == 1 {
                    export.output_alias = Some(outputs[0].id);
                }
            }
            let mut recipe = Recipe::aligned(&capture, export, Config::default())?;
            recipe.project_sha256 = Some(hash);
            recipe.save(Path::new(&args[2]))?;
            println!(
                "{}",
                serde_json::json!({"schema":1,"recipe":args[2],"project_sha256":recipe.project_sha256,"catalog":Recipe::catalog(&capture)?,"note":"Review/edit common range/output/metadata/targets and explicit pass IDs in the recipe before rendering"})
            );
            Ok(true)
        }
        "delivery-batch" => {
            if args.len() != 4 {
                return Err(
                    "usage: omatainer delivery-batch PROJECT.omatainer RECIPE.json NEW_FOLDER"
                        .into(),
                );
            }
            let handle = crate::engine::performance::Handle::default();
            let permit = handle.optional_work().map_err(|e| e.to_string())?;
            let cancel = permit.cancel();
            let _interrupt = Interrupt::new(cancel.clone());
            let (capture, hash) = capture(Path::new(&args[1]), &cancel)?;
            let recipe = Recipe::load(Path::new(&args[2]), &cancel)?;
            if recipe.project_sha256.as_deref() != Some(&hash) {
                return Err(
                    "Project bytes differ from the reviewed CLI recipe; create/review a new plan"
                        .into(),
                );
            }
            let review = recipe.review(&capture)?;
            let outcome = super::run(
                capture,
                &review,
                Path::new(&args[3]),
                &permit,
                &Default::default(),
                &Progress::default(),
            )?;
            println!(
                "{}",
                serde_json::json!({"schema":1,"status":"saved","folder":outcome.folder,"passes":outcome.files,"frames_per_pass":outcome.frames,"warning":outcome.warning})
            );
            Ok(true)
        }
        _ => Ok(false),
    }
}
static INTERRUPTED: AtomicBool = AtomicBool::new(false);
extern "C" fn interrupt(_: libc::c_int) {
    INTERRUPTED.store(true, Ordering::Release);
}
struct Interrupt {
    done: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
    previous: libc::sighandler_t,
}
impl Interrupt {
    fn new(cancel: Arc<AtomicBool>) -> Self {
        INTERRUPTED.store(false, Ordering::Release);
        let previous = unsafe { libc::signal(libc::SIGINT, interrupt as *const () as libc::sighandler_t) };
        let done = Arc::new(AtomicBool::new(false));
        let stop = done.clone();
        let thread = std::thread::spawn(move || {
            while !stop.load(Ordering::Acquire) {
                if INTERRUPTED.load(Ordering::Acquire) {
                    cancel.store(true, Ordering::Release);
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        });
        Self {
            done,
            thread: Some(thread),
            previous,
        }
    }
}
impl Drop for Interrupt {
    fn drop(&mut self) {
        unsafe {
            libc::signal(libc::SIGINT, self.previous);
        }
        self.done.store(true, Ordering::Release);
        let _ = self.thread.take().unwrap().join();
    }
}
