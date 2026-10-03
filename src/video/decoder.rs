//! Installed FFmpeg tools decode only allowlisted local containers on a worker.
use super::*;
use std::{
    io::Read,
    os::fd::AsRawFd,
    process::{Child, ChildStderr, ChildStdout, Command, Stdio},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

pub(crate) struct Process {
    child: Child,
    stdout: ChildStdout,
    stderr: ChildStderr,
    errors: Vec<u8>,
}
impl Process {
    /// Start an argument-based child with bounded nonblocking output pipes.
    /// Takes an already constructed command; returns a child killed and reaped on every exit path.
    pub fn start(command: &mut Command) -> Result<Self, String> {
        let mut child = command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("Install ffmpeg and ffprobe for local video decoding: {e}"))?;
        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();
        for fd in [stdout.as_raw_fd(), stderr.as_raw_fd()] {
            if unsafe { libc::fcntl(fd, libc::F_SETFL, libc::O_NONBLOCK) } < 0 {
                let _ = child.kill();
                let _ = child.wait();
                return Err("Cannot configure video decoder pipes".into());
            }
        }
        Ok(Self {
            child,
            stdout,
            stderr,
            errors: vec![],
        })
    }
    fn check(&mut self, cancel: &AtomicBool, deadline: Instant) -> Result<(), String> {
        if cancel.load(Ordering::Acquire) {
            return Err("Video operation cancelled".into());
        }
        if Instant::now() > deadline {
            return Err("Video decoder timed out".into());
        }
        let mut bytes = [0u8; 1024];
        loop {
            match self.stderr.read(&mut bytes) {
                Ok(0) => break,
                Ok(n) => {
                    if self.errors.len() + n > 4096 {
                        return Err("Video decoder error output exceeded its bound".into());
                    }
                    self.errors.extend_from_slice(&bytes[..n]);
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e.to_string()),
            }
        }
        if !self.errors.is_empty() {
            return Err(format!(
                "Video decoder: {}",
                String::from_utf8_lossy(&self.errors)
            ));
        }
        Ok(())
    }
    /// Collect bounded metadata while honoring cancellation and a wall deadline.
    /// Takes byte limit, duration and cancellation flag; returns successful child output.
    pub fn capture(
        &mut self,
        maximum: usize,
        timeout: Duration,
        cancel: &AtomicBool,
    ) -> Result<Vec<u8>, String> {
        let deadline = Instant::now() + timeout;
        let mut output = Vec::new();
        let mut bytes = [0u8; 16384];
        loop {
            self.check(cancel, deadline)?;
            match self.stdout.read(&mut bytes) {
                Ok(0) => {
                    if let Some(status) = self.child.try_wait().map_err(|e| e.to_string())? {
                        self.check(cancel, deadline)?;
                        if !status.success() {
                            return Err(format!("Video decoder exited with {status}"));
                        }
                        return Ok(output);
                    }
                }
                Ok(n) => {
                    if output.len() + n > maximum {
                        return Err("Video metadata exceeded its byte bound".into());
                    }
                    output.extend_from_slice(&bytes[..n]);
                    continue;
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e.to_string()),
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    /// Read one complete RGB frame from the sequential decoder.
    /// Takes the known bounded byte count and cancellation flag; returns exact frame bytes or a decoder failure.
    fn frame(&mut self, size: usize, cancel: &AtomicBool) -> Result<Vec<u8>, String> {
        let deadline = Instant::now() + Duration::from_secs(15);
        let mut bytes = vec![0; size];
        let mut read = 0;
        while read < size {
            self.check(cancel, deadline)?;
            match self.stdout.read(&mut bytes[read..]) {
                Ok(0) => {
                    if let Some(status) = self.child.try_wait().map_err(|e| e.to_string())? {
                        return Err(format!("Video ended before its indexed frame ({status})"));
                    }
                }
                Ok(n) => {
                    read += n;
                    continue;
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e.to_string()),
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        self.check(cancel, deadline)?;
        Ok(bytes)
    }
}
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn command(tool: &str) -> Command {
    let mut command = Command::new(tool);
    command.args([
        "-v",
        "error",
        "-threads",
        "1",
        "-protocol_whitelist",
        "file,pipe",
        "-format_whitelist",
        "mov,matroska,webm,avi,mpegts",
    ]);
    command
}
fn rate(value: &str) -> Result<Rate, String> {
    let (a, b) = value
        .split_once('/')
        .ok_or("Missing exact video frame rate")?;
    let mut numerator: u32 = a.parse().map_err(|_| "Invalid video frame rate")?;
    let mut denominator: u32 = b.parse().map_err(|_| "Invalid video frame rate")?;
    if numerator == 0 || denominator == 0 {
        return Err("Invalid video frame rate".into());
    }
    let (mut x, mut y) = (numerator, denominator);
    while y != 0 {
        let next = x % y;
        x = y;
        y = next;
    }
    numerator /= x;
    denominator /= x;
    let rate = Rate {
        numerator,
        denominator,
    };
    if rate.nominal().is_ok() {
        return Ok(rate);
    }
    for (numerator, denominator) in [
        (24, 1),
        (25, 1),
        (30, 1),
        (50, 1),
        (60, 1),
        (24000, 1001),
        (30000, 1001),
        (60000, 1001),
    ] {
        let candidate = Rate {
            numerator,
            denominator,
        };
        if (f64::from(rate.numerator) / f64::from(rate.denominator)
            - f64::from(numerator) / f64::from(denominator))
        .abs()
            <= 0.00001
        {
            return Ok(candidate);
        }
    }
    rate.nominal()?;
    Ok(rate)
}
#[derive(Deserialize)]
struct Probe {
    streams: Vec<Stream>,
    frames: Vec<ProbeFrame>,
}
#[derive(Deserialize)]
struct Stream {
    codec_name: String,
    width: u32,
    height: u32,
    avg_frame_rate: String,
    r_frame_rate: String,
    pix_fmt: String,
    sample_aspect_ratio: Option<String>,
    color_transfer: Option<String>,
    #[serde(default)]
    side_data_list: Vec<serde_json::Value>,
}
#[derive(Deserialize)]
struct ProbeFrame {
    best_effort_timestamp_time: String,
    width: u32,
    height: u32,
}

/// Decode and validate every picture timestamp before importing a local clip.
/// Takes an absolute file and cancellation flag; returns constant-rate metadata and exact file identity.
pub(crate) fn probe(path: PathBuf, cancel: &AtomicBool) -> Result<Clip, String> {
    if !path.is_absolute() || path.as_os_str().len() > 4096 {
        return Err("Choose one absolute local video path".into());
    }
    let fingerprint =
        FileFingerprint::read(&path).ok_or("Video source is missing or is not a regular file")?;
    let mut command = command("ffprobe");
    command.args(["-select_streams","v:0","-show_streams","-show_frames","-show_entries",
        "stream=codec_name,width,height,avg_frame_rate,r_frame_rate,pix_fmt,sample_aspect_ratio,color_transfer:stream_side_data=rotation:frame=best_effort_timestamp_time,width,height",
        "-of","json=compact=1"]).arg(&path);
    let bytes = Process::start(&mut command)?.capture(
        128 * 1024 * 1024,
        Duration::from_secs(120),
        cancel,
    )?;
    let data: Probe =
        serde_json::from_slice(&bytes).map_err(|e| format!("Invalid video metadata: {e}"))?;
    if data.streams.len() != 1 || data.frames.is_empty() || data.frames.len() as u64 > MAX_FRAMES {
        return Err(
            "Video needs one supported picture stream with at most one million decoded frames"
                .into(),
        );
    }
    let stream = &data.streams[0];
    let rate = rate(&stream.avg_frame_rate)?;
    if rate != self::rate(&stream.r_frame_rate)?
        || stream
            .sample_aspect_ratio
            .as_deref()
            .is_some_and(|sar| !matches!(sar, "1:1" | "N/A"))
        || stream
            .side_data_list
            .iter()
            .any(|s| s.get("rotation").is_some_and(|r| r.as_i64() != Some(0)))
        || !matches!(
            stream.pix_fmt.as_str(),
            "yuv420p"
                | "yuv422p"
                | "yuv444p"
                | "yuvj420p"
                | "yuvj422p"
                | "yuvj444p"
                | "rgb24"
                | "rgba"
                | "bgr0"
                | "bgra"
                | "yuv420p10le"
                | "yuv422p10le"
                | "yuv444p10le"
                | "yuv444p12le"
                | "gbrp10le"
                | "gbrp12le"
        )
        || stream
            .color_transfer
            .as_deref()
            .is_some_and(|name| matches!(name, "smpte2084" | "arib-std-b67"))
    {
        return Err("Video requires constant rate, square pixels, no rotation and supported SDR pixels; normalize other sources first".into());
    }
    let first: f64 = data.frames[0]
        .best_effort_timestamp_time
        .parse()
        .map_err(|_| "Missing video timestamp")?;
    let info = Info {
        rate,
        frames: data.frames.len() as u64,
        width: stream.width,
        height: stream.height,
        first_pts: first,
        codec: stream.codec_name.clone(),
    };
    info.validate()?;
    for (index, frame) in data.frames.iter().enumerate() {
        let timestamp: f64 = frame
            .best_effort_timestamp_time
            .parse()
            .map_err(|_| "Invalid video timestamp")?;
        if !timestamp.is_finite()
            || (timestamp - first - rate.seconds(index as u64)).abs() > 0.0011
            || frame.width != info.width
            || frame.height != info.height
        {
            return Err("Variable-rate timestamps, discontinuities or changing video dimensions are unsupported; source was preserved".into());
        }
    }
    if cancel.load(Ordering::Acquire) {
        return Err("Video import cancelled".into());
    }
    if FileFingerprint::read(&path) != Some(fingerprint) {
        return Err("Video changed during import; inspect it again".into());
    }
    Ok(Clip {
        path,
        fingerprint,
        trim_in: 0,
        trim_out: info.frames,
        placement: 0,
        timecode_offset: 0,
        drop_frame: false,
        preview_offset_ms: 0,
        locators: vec![],
        detached: false,
        info,
    })
}
#[derive(Debug)]
pub(crate) struct Frame {
    pub index: u64,
    pub width: usize,
    pub height: usize,
    pub rgba: Vec<u8>,
}
/// Decode sequential frames from an exact constant-rate source position.
/// Takes validated clip, initial source frame, cancellation flag and bounded frame consumer; stops on cancellation or consumer refusal.
pub(crate) fn stream(
    clip: &Clip,
    start: u64,
    cancel: &AtomicBool,
    mut consume: impl FnMut(Frame) -> Result<(), String>,
) -> Result<(), String> {
    clip.validate()?;
    if start >= clip.trim_out || FileFingerprint::read(&clip.path) != Some(clip.fingerprint) {
        return Err("Video source changed or requested frame is outside its trim".into());
    }
    let scale = (1280.0 / f64::from(clip.info.width))
        .min(720.0 / f64::from(clip.info.height))
        .min(1.0);
    let width = (f64::from(clip.info.width) * scale).round().max(1.0) as usize;
    let height = (f64::from(clip.info.height) * scale).round().max(1.0) as usize;
    let seek = ((clip.info.first_pts + clip.info.rate.seconds(start) - 0.0011) * 1e6).floor() / 1e6;
    let mut command = command("ffmpeg");
    command
        .args([
            "-nostdin",
            "-seek_timestamp",
            "1",
            "-ss",
            &format!("{seek:.6}"),
            "-i",
        ])
        .arg(&clip.path)
        .args([
            "-map",
            "0:v:0",
            "-an",
            "-sn",
            "-dn",
            "-filter_threads",
            "1",
            "-vf",
            &format!("scale={width}:{height}"),
            "-fps_mode",
            "passthrough",
            "-pix_fmt",
            "rgba",
            "-f",
            "rawvideo",
            "pipe:1",
        ]);
    let mut process = Process::start(&mut command)?;
    for index in start..clip.trim_out {
        let rgba = process.frame(width * height * 4, cancel)?;
        if FileFingerprint::read(&clip.path) != Some(clip.fingerprint) {
            return Err("Video source changed during decoding".into());
        }
        consume(Frame {
            index,
            width,
            height,
            rgba,
        })?;
    }
    Ok(())
}
#[cfg(test)]
pub(crate) mod tests;
