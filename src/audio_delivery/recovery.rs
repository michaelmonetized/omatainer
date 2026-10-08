use super::wav::Encoding;
use sha2::{Digest, Sha256};
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Clone, Debug, PartialEq, Eq)]
struct Identity {
    device: u64,
    inode: u64,
    bytes: u64,
    modified: (i64, i64),
    digest: [u8; 32],
}
#[derive(Clone, Debug)]
struct Segment {
    name: String,
    identity: Identity,
    frames: u64,
    repair: bool,
}
#[derive(Clone, Debug)]
pub(crate) struct Plan {
    pub folder: PathBuf,
    pub frames: u64,
    pub rate: u32,
    pub channels: u16,
    pub repairs: usize,
    directory: (u64, u64),
    metadata: Identity,
    owner: Identity,
    encoding: Encoding,
    segments: Vec<Segment>,
}
fn open(path: &Path, write: bool) -> Result<File, String> {
    let f = OpenOptions::new()
        .read(true)
        .write(write)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
        .map_err(|e| format!("Cannot open {}: {e}", path.display()))?;
    if !f.metadata().map_err(|e| e.to_string())?.is_file() {
        return Err("Recovery requires regular recording files".into());
    }
    if unsafe {
        libc::flock(
            f.as_raw_fd(),
            if write { libc::LOCK_EX } else { libc::LOCK_SH } | libc::LOCK_NB,
        )
    } != 0
    {
        return Err("Recording is still open; stop it before recovery".into());
    }
    Ok(f)
}
fn directory(path: &Path) -> Result<(u64, u64), String> {
    let m = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if !path.is_absolute() || !m.is_dir() || m.file_type().is_symlink() {
        return Err("Recovery needs an existing absolute recording folder".into());
    }
    Ok((m.dev(), m.ino()))
}
fn identity(f: &mut File, cancel: &AtomicBool) -> Result<Identity, String> {
    let before = f.metadata().map_err(|e| e.to_string())?;
    f.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
    let mut hash = Sha256::new();
    let mut bytes = [0; 65536];
    loop {
        if cancel.load(Ordering::Acquire) {
            return Err("Recovery cancelled".into());
        }
        let n = f.read(&mut bytes).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        hash.update(&bytes[..n]);
    }
    let after = f.metadata().map_err(|e| e.to_string())?;
    if (before.len(), before.mtime(), before.mtime_nsec())
        != (after.len(), after.mtime(), after.mtime_nsec())
    {
        return Err("Recording changed while it was reviewed".into());
    }
    Ok(Identity {
        device: after.dev(),
        inode: after.ino(),
        bytes: after.len(),
        modified: (after.mtime(), after.mtime_nsec()),
        digest: hash.finalize().into(),
    })
}
fn metadata(f: &mut File) -> Result<serde_json::Value, String> {
    if f.metadata().map_err(|e| e.to_string())?.len() > 65536 {
        return Err("Recording manifest is too large".into());
    }
    f.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
    serde_json::from_reader(f).map_err(|e| e.to_string())
}
fn segments(folder: &Path) -> Result<Vec<String>, String> {
    let mut names = Vec::new();
    for item in std::fs::read_dir(folder).map_err(|e| e.to_string())? {
        let item = item.map_err(|e| e.to_string())?;
        let name = item
            .file_name()
            .into_string()
            .map_err(|_| "Recording filename is not valid text")?;
        if name.ends_with(".wav") {
            if !item.file_type().map_err(|e| e.to_string())?.is_file() {
                return Err("Recovery refuses linked or non-file WAV segments".into());
            }
            names.push(name);
        }
    }
    names.sort();
    if names.is_empty()
        || names.len() > 256
        || names
            .iter()
            .enumerate()
            .any(|(i, n)| *n != format!("take-{:04}.wav", i + 1))
    {
        return Err("Recovery requires a contiguous set of Omatainer take files".into());
    }
    Ok(names)
}
fn inspect(
    f: &mut File,
    channels: u16,
    rate: u32,
    encoding: Encoding,
    cancel: &AtomicBool,
) -> Result<(u64, bool), String> {
    f.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
    let mut header = [0; 44];
    f.read_exact(&mut header).map_err(|e| e.to_string())?;
    let block = channels * encoding.bytes() as u16;
    let expected_format = if encoding == Encoding::Float32 {
        3_u16
    } else {
        1
    };
    if &header[..4] != b"RIFF"
        || &header[8..16] != b"WAVEfmt "
        || header[16..20] != 16_u32.to_le_bytes()
        || header[20..22] != expected_format.to_le_bytes()
        || header[22..24] != channels.to_le_bytes()
        || header[24..28] != rate.to_le_bytes()
        || header[28..32] != (rate * u32::from(block)).to_le_bytes()
        || header[32..34] != block.to_le_bytes()
        || header[34..36] != encoding.bits().to_le_bytes()
        || &header[36..40] != b"data"
    {
        return Err("WAV does not match this recording's native format; no repair was made".into());
    }
    let length = f.metadata().map_err(|e| e.to_string())?.len();
    if length < 44 || length - 44 > u64::from(u32::MAX) - 36 {
        return Err("Recording segment size is unsupported".into());
    }
    let mut frames = (length - 44) / u64::from(block);
    if encoding == Encoding::Float32 {
        let mut buffer = vec![0; usize::from(block) * 1024];
        let mut scanned = 0;
        while scanned < frames {
            if cancel.load(Ordering::Acquire) {
                return Err("Recovery cancelled".into());
            }
            let count = (frames - scanned).min(1024) as usize;
            f.read_exact(&mut buffer[..count * usize::from(block)])
                .map_err(|e| e.to_string())?;
            if let Some(bad) = buffer[..count * usize::from(block)]
                .chunks_exact(usize::from(block))
                .position(|frame| {
                    frame
                        .chunks_exact(4)
                        .any(|sample| !f32::from_le_bytes(sample.try_into().unwrap()).is_finite())
                })
            {
                frames = scanned + bad as u64;
                break;
            }
            scanned += count as u64;
        }
    }
    let data = frames * u64::from(block);
    let repair = length != 44 + data
        || header[4..8] != (36 + data as u32).to_le_bytes()
        || header[40..44] != (data as u32).to_le_bytes();
    Ok((frames, repair))
}
/// Review an interrupted native recording.
/// Takes its folder and cancellation flag; returns exact valid prefixes and fingerprints without changing files or opening audio devices.
pub(crate) fn review(folder: &Path, cancel: &AtomicBool) -> Result<Plan, String> {
    let directory = directory(folder)?;
    let mut ownership = open(&folder.join(".recording.lock"), false)?;
    let owner = identity(&mut ownership, cancel)?;
    let mut manifest = open(&folder.join("recording.json"), false)?;
    let metadata_identity = identity(&mut manifest, cancel)?;
    let metadata = metadata(&mut manifest)?;
    if metadata["schema"] != 1 || metadata["recoverable_wav_prefixes"] != true {
        return Err("Folder is not a recoverable Omatainer recording".into());
    }
    let rate = metadata["rate"]
        .as_u64()
        .filter(|r| (8000..=192000).contains(r))
        .ok_or("Recording rate is invalid")? as u32;
    let channels = metadata["channels"]
        .as_u64()
        .filter(|c| (1..=26).contains(c))
        .ok_or("Recording width is invalid")? as u16;
    let encoding = match metadata["wav_encoding"].as_str() {
        Some("float32") => Encoding::Float32,
        Some("pcm16") => Encoding::Pcm16,
        Some("pcm24") => Encoding::Pcm24,
        _ => return Err("Recording manifest has no native WAV format".into()),
    };
    let mut plan = Plan {
        folder: folder.to_owned(),
        frames: 0,
        rate,
        channels,
        repairs: 0,
        directory,
        metadata: metadata_identity,
        owner,
        encoding,
        segments: Vec::new(),
    };
    for name in segments(folder)? {
        let mut f = open(&folder.join(&name), false)?;
        let fingerprint = identity(&mut f, cancel)?;
        let (frames, repair) = inspect(&mut f, channels, rate, encoding, cancel)?;
        plan.frames += frames;
        plan.repairs += usize::from(repair);
        plan.segments.push(Segment {
            name,
            identity: fingerprint,
            frames,
            repair,
        });
    }
    if directory != self::directory(folder)? {
        return Err("Recording folder changed during review".into());
    }
    Ok(plan)
}
/// Flush a recording manifest atomically.
/// Takes a folder and JSON record; replaces its manifest only after the new file is complete, then syncs the directory.
pub(crate) fn persist(folder: &Path, metadata: &serde_json::Value) -> Result<(), String> {
    let ownership = folder.join(".recording.lock");
    match OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&ownership)
    {
        Ok(f) => f.sync_all().map_err(|e| e.to_string())?,
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            let m = std::fs::symlink_metadata(&ownership).map_err(|e| e.to_string())?;
            if !m.is_file() || m.file_type().is_symlink() {
                return Err("Recording ownership file changed".into());
            }
        }
        Err(e) => return Err(e.to_string()),
    }
    let path = folder.join(format!(
        ".recording-{}.json",
        crate::sampler_bank::BankId::new()?
    ));
    let mut f = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)
        .map_err(|e| format!("Cannot prepare recording manifest: {e}"))?;
    let result = (|| {
        f.write_all(&serde_json::to_vec_pretty(metadata).map_err(|e| e.to_string())?)
            .and_then(|_| f.sync_all())
            .map_err(|e| e.to_string())?;
        std::fs::rename(&path, folder.join("recording.json"))
            .and_then(|_| File::open(folder)?.sync_all())
            .map_err(|e| e.to_string())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(path);
    }
    result
}
/// Repair the exact reviewed recording prefixes.
/// Takes an unchanged review and cancellation flag; locks and validates every file first, then finalizes headers and reports durable WAV frame counts.
pub(crate) fn recover(plan: &Plan, cancel: &AtomicBool) -> Result<Plan, String> {
    let mut ownership = lock_recording(&plan.folder)?;
    if identity(&mut ownership, cancel)? != plan.owner {
        return Err("Recording ownership changed since review".into());
    }
    if directory(&plan.folder)? != plan.directory
        || segments(&plan.folder)?
            != plan
                .segments
                .iter()
                .map(|s| s.name.clone())
                .collect::<Vec<_>>()
    {
        return Err("Recording files changed since recovery review".into());
    }
    let mut manifest = open(&plan.folder.join("recording.json"), true)?;
    if identity(&mut manifest, cancel)? != plan.metadata {
        return Err("Recording manifest changed since review".into());
    }
    let mut metadata = metadata(&mut manifest)?;
    let mut files = Vec::new();
    for segment in &plan.segments {
        let mut f = open(&plan.folder.join(&segment.name), true)?;
        if identity(&mut f, cancel)? != segment.identity {
            return Err("Recording audio changed since review; review again".into());
        }
        files.push(f);
    }
    if cancel.load(Ordering::Acquire) {
        return Err("Recovery cancelled before repairs".into());
    }
    for (segment, f) in plan.segments.iter().zip(&mut files) {
        if segment.repair {
            let bytes =
                (segment.frames * u64::from(plan.channels) * plan.encoding.bytes() as u64) as u32;
            f.set_len(44 + u64::from(bytes))
                .and_then(|_| f.seek(SeekFrom::Start(4)))
                .and_then(|_| f.write_all(&(36 + bytes).to_le_bytes()))
                .and_then(|_| f.seek(SeekFrom::Start(40)))
                .and_then(|_| f.write_all(&bytes.to_le_bytes()))
                .and_then(|_| f.sync_all())
                .map_err(|e| {
                    format!(
                        "Repair stopped at {}: {e}; review the folder again",
                        segment.name
                    )
                })?;
        }
    }
    metadata["frames"] = plan.frames.into();
    metadata["status"] = "Recovered valid WAV prefixes".into();
    metadata["files"] = plan
        .segments
        .iter()
        .map(|s| s.name.clone())
        .collect::<Vec<_>>()
        .into();
    persist(&plan.folder, &metadata)?;
    drop(files);
    drop(manifest);
    drop(ownership);
    review(&plan.folder, &AtomicBool::new(false))
}

/// Hold an entire recording operation against recovery.
/// Takes its prepared folder; returns an exclusive nonblocking ownership file that stays locked through capture, splitting and encoding.
pub(crate) fn lock_recording(folder: &Path) -> Result<File, String> {
    open(&folder.join(".recording.lock"), true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio_delivery::wav::Writer;
    struct Files(PathBuf);
    impl Files {
        fn new() -> Self {
            let p = std::env::temp_dir().join(format!(
                "omatainer-recovery-{}",
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
    fn manifest(folder: &Path, encoding: Encoding) {
        persist(folder,&serde_json::json!({"schema":1,"recoverable_wav_prefixes":true,"rate":48000,"channels":2,"wav_encoding":match encoding{Encoding::Float32=>"float32",Encoding::Pcm16=>"pcm16",Encoding::Pcm24=>"pcm24"},"frames":0,"status":"Recording"})).unwrap();
    }
    #[test]
    fn interrupted_native_formats_recover_exact_prefix_and_refuse_active_or_changed_files() {
        for encoding in [Encoding::Float32, Encoding::Pcm16, Encoding::Pcm24] {
            let files = Files::new();
            manifest(&files.0, encoding);
            let path = files.0.join("take-0001.wav");
            let mut w = Writer::new(&path, 2, 48000, encoding, false).unwrap();
            w.append(&[0.125, -0.25, 0.5, -0.75], 1.0).unwrap();
            assert!(review(&files.0, &AtomicBool::new(false))
                .unwrap_err()
                .contains("still open"));
            drop(w);
            let mut f = OpenOptions::new().append(true).open(&path).unwrap();
            f.write_all(&[1]).unwrap();
            drop(f);
            let plan = review(&files.0, &AtomicBool::new(false)).unwrap();
            assert_eq!((plan.frames, plan.repairs), (2, 1));
            let fixed = recover(&plan, &AtomicBool::new(false)).unwrap();
            assert_eq!((fixed.frames, fixed.repairs), (2, 0));
            let decoded = crate::engine::decode::decode_audio(&path).unwrap();
            assert_eq!(decoded.sample.frames(), 2);
            assert_eq!(&decoded.sample.data[..4], &[0.125, -0.25, 0.5, -0.75]);
            let ownership = lock_recording(&files.0).unwrap();
            assert!(review(&files.0, &AtomicBool::new(false))
                .unwrap_err()
                .contains("still open"));
            drop(ownership);
            let fresh = review(&files.0, &AtomicBool::new(false)).unwrap();
            let mut f = OpenOptions::new().write(true).open(&path).unwrap();
            f.seek(SeekFrom::Start(44)).unwrap();
            f.write_all(&[0]).unwrap();
            drop(f);
            let mut f = OpenOptions::new().write(true).open(&path).unwrap();
            f.seek(SeekFrom::Start(44)).unwrap();
            f.write_all(&[7]).unwrap();
            drop(f);
            let before = std::fs::read(&path).unwrap();
            assert!(recover(&fresh, &AtomicBool::new(false))
                .unwrap_err()
                .contains("changed"));
            assert_eq!(before, std::fs::read(path).unwrap());
        }
    }
    #[test]
    fn recovery_refuses_foreign_headers_links_and_preserves_only_finite_float_frames() {
        let files = Files::new();
        manifest(&files.0, Encoding::Float32);
        let path = files.0.join("take-0001.wav");
        let mut w = Writer::new(&path, 2, 48000, Encoding::Float32, false).unwrap();
        w.append(&[0.1, 0.2], 1.0).unwrap();
        drop(w);
        let mut f = OpenOptions::new().append(true).open(&path).unwrap();
        f.write_all(
            &[
                f32::NAN.to_le_bytes(),
                0_f32.to_le_bytes(),
                0.3_f32.to_le_bytes(),
                0.4_f32.to_le_bytes(),
            ]
            .concat(),
        )
        .unwrap();
        drop(f);
        let p = review(&files.0, &AtomicBool::new(false)).unwrap();
        assert_eq!(p.frames, 1);
        recover(&p, &AtomicBool::new(false)).unwrap();
        assert_eq!(
            crate::engine::decode::decode_audio(&path)
                .unwrap()
                .sample
                .frames(),
            1
        );
        let mut f = OpenOptions::new().write(true).open(&path).unwrap();
        f.seek(SeekFrom::Start(36)).unwrap();
        f.write_all(b"LIST").unwrap();
        drop(f);
        let before = std::fs::read(&path).unwrap();
        assert!(review(&files.0, &AtomicBool::new(false))
            .unwrap_err()
            .contains("native format"));
        assert_eq!(before, std::fs::read(&path).unwrap());
        std::fs::rename(&path, files.0.join("original.bin")).unwrap();
        std::os::unix::fs::symlink(files.0.join("original.bin"), &path).unwrap();
        assert!(review(&files.0, &AtomicBool::new(false)).is_err());
    }
    #[test]
    #[ignore = "isolated child entry exercised by killed_recording_worker_recovers_durable_samples"]
    fn interrupted_recording_child() {
        let path = PathBuf::from(std::env::var_os("OMATAINER_RECOVERY_CHILD").expect("child path"));
        let disk_failure = std::env::var_os("OMATAINER_RECOVERY_DISK_FAILURE").is_some();
        if disk_failure {
            let limit = libc::rlimit {
                rlim_cur: 4140,
                rlim_max: 4140,
            };
            unsafe {
                libc::signal(libc::SIGXFSZ, libc::SIG_IGN);
                assert_eq!(libc::setrlimit(libc::RLIMIT_FSIZE, &limit), 0);
            }
        }
        manifest(&path, Encoding::Float32);
        let mut w = Writer::new(
            &path.join("take-0001.wav"),
            2,
            48000,
            Encoding::Float32,
            false,
        )
        .unwrap();
        let result = w.append(&[0.125, -0.25].repeat(4096), 1.0);
        if disk_failure {
            let error = result.unwrap_err();
            assert_eq!(w.finish().unwrap(), 512);
            std::fs::write(path.join("write-error.txt"), error).unwrap();
            return;
        }
        result.unwrap();
        std::fs::write(path.join("ready"), b"ready").unwrap();
        loop {
            std::thread::park();
        }
    }
    #[test]
    fn killed_recording_worker_recovers_durable_samples() {
        let files = Files::new();
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "audio_delivery::recovery::tests::interrupted_recording_child",
                "--ignored",
                "--nocapture",
            ])
            .env("OMATAINER_RECOVERY_CHILD", &files.0)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let end = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !files.0.join("ready").exists() {
            assert!(std::time::Instant::now() < end);
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        child.kill().unwrap();
        assert!(!child.wait().unwrap().success());
        let p = review(&files.0, &AtomicBool::new(false)).unwrap();
        assert_eq!((p.frames, p.repairs), (4096, 1));
        recover(&p, &AtomicBool::new(false)).unwrap();
        let decoded = crate::engine::decode::decode_audio(&files.0.join("take-0001.wav")).unwrap();
        assert_eq!(decoded.sample.data, [0.125, -0.25].repeat(4096));
    }
    #[test]
    fn real_disk_write_error_finalizes_only_complete_frames_and_reports_the_failure() {
        let files = Files::new();
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "audio_delivery::recovery::tests::interrupted_recording_child",
                "--ignored",
                "--nocapture",
            ])
            .env("OMATAINER_RECOVERY_CHILD", &files.0)
            .env("OMATAINER_RECOVERY_DISK_FAILURE", "1")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap();
        assert!(status.success());
        assert!(std::fs::read_to_string(files.0.join("write-error.txt"))
            .unwrap()
            .contains("Audio write failed after 512 complete frames"));
        let p = review(&files.0, &AtomicBool::new(false)).unwrap();
        assert_eq!((p.frames, p.repairs), (512, 0));
        let decoded = crate::engine::decode::decode_audio(&files.0.join("take-0001.wav")).unwrap();
        assert_eq!(decoded.sample.data, [0.125, -0.25].repeat(512));
    }
}
