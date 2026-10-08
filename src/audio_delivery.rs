use crate::engine::{
    performance::WorkPermit,
    project::{Captured, Prepared},
    Command,
};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};
mod render;
pub(crate) mod recovery;
#[cfg(test)]
mod tests;
pub(crate) mod wav;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Format {
    #[default]
    Float32,
    Pcm16,
    Pcm24,
    Flac16,
    Flac24,
    Mp3,
}
impl Format {
    pub const ALL: [Self; 6] = [
        Self::Float32,
        Self::Pcm16,
        Self::Pcm24,
        Self::Flac16,
        Self::Flac24,
        Self::Mp3,
    ];
    pub fn title(self) -> &'static str {
        match self {
            Self::Float32 => "WAV · 32-bit float",
            Self::Pcm16 => "WAV · 16-bit PCM",
            Self::Pcm24 => "WAV · 24-bit PCM",
            Self::Flac16 => "FLAC · 16-bit",
            Self::Flac24 => "FLAC · 24-bit",
            Self::Mp3 => "MP3 · 320 kbps",
        }
    }
    pub fn extension(self) -> &'static str {
        match self {
            Self::Flac16 | Self::Flac24 => "flac",
            Self::Mp3 => "mp3",
            _ => "wav",
        }
    }
    pub fn encoding(self) -> wav::Encoding {
        match self {
            Self::Pcm16 | Self::Flac16 => wav::Encoding::Pcm16,
            Self::Pcm24 | Self::Flac24 => wav::Encoding::Pcm24,
            _ => wav::Encoding::Float32,
        }
    }
    pub fn dither(self) -> bool {
        matches!(
            self,
            Self::Pcm16 | Self::Pcm24 | Self::Flac16 | Self::Flac24
        )
    }
    fn external(self) -> bool {
        matches!(self, Self::Flac16 | Self::Flac24 | Self::Mp3)
    }
}
#[derive(Clone, Debug)]
pub(crate) struct Options {
    pub format: Format,
    pub rate: u32,
    pub channels: u16,
    pub dither: bool,
    pub normalize: bool,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            format: Format::Float32,
            rate: 48000,
            channels: 2,
            dither: false,
            normalize: false,
        }
    }
}
impl Options {
    /// Validate an explicit output format.
    /// Takes format, rate, width and processing choices; refuses unsupported codec combinations before creating files.
    pub fn validate(&self) -> Result<(), String> {
        if !(8000..=192000).contains(&self.rate)
            || !(1..=26).contains(&self.channels)
            || self.dither && !self.format.dither()
        {
            return Err("Unsupported output rate, width or dither choice".into());
        }
        if matches!(self.format, Format::Flac16 | Format::Flac24) && self.channels > 8 {
            return Err("FLAC supports up to eight channels; choose WAV for wider sources".into());
        }
        if self.format == Format::Mp3
            && (self.channels > 2 || ![32000, 44100, 48000].contains(&self.rate))
        {
            return Err("320 kbps MP3 requires mono/stereo at 32, 44.1 or 48 kHz".into());
        }
        Ok(())
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Source {
    #[default]
    Session,
    Scene,
    Arrangement,
}
#[derive(Clone, Debug)]
pub(crate) struct Export {
    pub source: Source,
    pub scene: u16,
    pub decks: bool,
    pub output_alias: Option<u64>,
    pub start: f64,
    pub end: f64,
    pub repeats: u16,
    pub tail: f64,
    pub options: Options,
}
impl Default for Export {
    fn default() -> Self {
        Self {
            source: Source::Scene,
            scene: 0,
            decks: false,
            output_alias: None,
            start: 0.0,
            end: 30.0,
            repeats: 1,
            tail: 2.0,
            options: Options::default(),
        }
    }
}
impl Export {
    /// Resolve exact render frame bounds.
    /// Takes a reviewed range and format; returns preroll, body, tail and final frame counts or an explicit size refusal.
    pub fn frames(&self) -> Result<(u64, u64, u64, u64), String> {
        self.options.validate()?;
        if self.options.channels > 2
            || ![self.start, self.end, self.tail]
                .iter()
                .all(|v| v.is_finite())
            || self.start < 0.0
            || self.end <= self.start
            || self.end > 28800.0
            || !(0.0..=120.0).contains(&self.tail)
            || !(1..=64).contains(&self.repeats)
        {
            return Err("Export needs a positive range within eight hours, mono/stereo, 1–64 repeats and a tail of at most 120 seconds".into());
        }
        let frame = |v: f64| (v * f64::from(self.options.rate)).round() as u64;
        let start = frame(self.start);
        let body = frame(self.end) - start;
        let tail = frame(self.tail);
        let total = body
            .checked_mul(u64::from(self.repeats))
            .and_then(|n| n.checked_add(tail))
            .ok_or("Export duration overflow")?;
        let bytes = total
            .checked_mul(
                u64::from(self.options.channels) * self.options.format.encoding().bytes() as u64,
            )
            .ok_or("Export size overflow")?;
        if body == 0 || bytes > u64::from(u32::MAX) - 36 {
            return Err("Export exceeds the supported single-file size; shorten the range or reduce repeats/rate".into());
        }
        Ok((start, body, tail, total))
    }
}
#[derive(Clone, Debug)]
pub(crate) struct Outcome {
    pub folder: PathBuf,
    pub frames: u64,
    pub rate: u32,
    pub channels: u16,
    pub peak: f32,
    pub gain: f64,
}
fn active(cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::Acquire) {
        Err("Audio export cancelled; no output was published".into())
    } else {
        Ok(())
    }
}
fn create(path: &Path) -> Result<File, String> {
    OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .map_err(|e| e.to_string())
}

/// Convert a bounded raw float spool into one explicit delivery format.
/// Takes complete source frames, loop count, tail, output choices, cancellation and progress; returns peak and applied gain without loading the whole recording.
pub(crate) fn encode(
    raw: &Path,
    destination: &Path,
    body: u64,
    tail: u64,
    repeats: u16,
    options: &Options,
    cancel: &AtomicBool,
    progress: &crate::background::Reporter,
) -> Result<(f32, f64), String> {
    options.validate()?;
    active(cancel)?;
    let width = usize::from(options.channels);
    let expected = (body + tail)
        .checked_mul(width as u64 * 4)
        .ok_or("Audio spool size overflow")?;
    let mut input = File::open(raw).map_err(|e| e.to_string())?;
    if input.metadata().map_err(|e| e.to_string())?.len() != expected {
        return Err("Audio spool duration differs from the reviewed range".into());
    }
    let mut bytes = [0_u8; 16384];
    let mut samples = [0_f32; 4096];
    let mut peak = 0_f32;
    let mut left = expected;
    while left > 0 {
        active(cancel)?;
        let n = left.min(bytes.len() as u64) as usize;
        input
            .read_exact(&mut bytes[..n])
            .map_err(|e| e.to_string())?;
        for b in bytes[..n].chunks_exact(4) {
            let s = f32::from_le_bytes(b.try_into().unwrap());
            if !s.is_finite() {
                return Err("Audio spool contains a non-finite sample".into());
            }
            peak = peak.max(s.abs());
        }
        left -= n as u64;
    }
    let gain = if options.normalize && peak > 0.0 {
        10_f64.powf(-1.0 / 20.0) / f64::from(peak)
    } else {
        1.0
    };
    let temporary = destination.with_extension("encode.wav");
    let native = if options.format.external() {
        temporary.as_path()
    } else {
        destination
    };
    let mut writer = wav::Writer::new(
        native,
        options.channels,
        options.rate,
        options.format.encoding(),
        options.dither,
    )?;
    let total = body * u64::from(repeats) + tail;
    let mut done = 0;
    progress.progress(0, Some(total));
    for repeat in 0..repeats {
        input.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
        let mut frames = body + if repeat + 1 == repeats { tail } else { 0 };
        while frames > 0 {
            active(cancel)?;
            let n = frames.min((samples.len() / width) as u64) as usize;
            input
                .read_exact(&mut bytes[..n * width * 4])
                .map_err(|e| e.to_string())?;
            for (s, b) in samples[..n * width].iter_mut().zip(bytes.chunks_exact(4)) {
                *s = f32::from_le_bytes(b.try_into().unwrap());
            }
            writer.append(&samples[..n * width], gain)?;
            frames -= n as u64;
            done += n as u64;
            progress.progress(done, Some(total));
        }
    }
    if writer.finish()? != total {
        return Err("Encoded frame count differs".into());
    }
    active(cancel)?;
    if options.format.external() {
        progress.progress(0, None);
        let mut command = std::process::Command::new("ffmpeg");
        command
            .args([
                "-v",
                "error",
                "-nostdin",
                "-threads",
                "1",
                "-protocol_whitelist",
                "file,pipe",
                "-format_whitelist",
                "wav",
                "-i",
            ])
            .arg(&temporary)
            .args([
                "-map",
                "0:a:0",
                "-vn",
                "-map_metadata",
                "-1",
                "-threads",
                "1",
            ]);
        if options.format == Format::Mp3 {
            command.args(["-c:a", "libmp3lame", "-b:a", "320k"]);
        } else {
            command.args(["-c:a", "flac", "-compression_level", "5"]);
        }
        command.arg("-n").arg(destination);
        crate::video::decoder::Process::start(&mut command)?
            .capture(1024, std::time::Duration::from_secs(3600), cancel)
            .map_err(|e| format!("Audio encoding failed: {e}"))?;
        File::open(destination)
            .and_then(|f| f.sync_all())
            .map_err(|e| e.to_string())?;
        std::fs::remove_file(temporary).map_err(|e| e.to_string())?;
    }
    active(cancel)?;
    Ok((peak, gain))
}

/// Export a coherent session without opening devices or changing live playback.
/// Takes project capture, reviewed request, new folder and admitted work; renders and publishes only after every format/range check succeeds.
pub(crate) fn run(
    captured: Captured,
    request: &Export,
    destination: &Path,
    permit: &WorkPermit,
    progress: &crate::background::Reporter,
) -> Result<Outcome, String> {
    let cancel = permit.cancel();
    active(&cancel)?;
    let bounds = request.frames()?;
    if !destination.is_absolute() || destination.as_os_str().len() > 4096 || destination.exists() {
        return Err("Choose a new absolute export folder".into());
    }
    let revision = captured.revision;
    let stage = crate::portable_project::Stage::new(
        destination
            .parent()
            .ok_or("Export folder needs an existing parent")?,
    )?;
    let parent = std::ffi::CString::new(stage.path.as_os_str().as_encoded_bytes())
        .map_err(|e| e.to_string())?;
    let mut space = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    if unsafe { libc::statvfs(parent.as_ptr(), space.as_mut_ptr()) } != 0 {
        return Err(format!(
            "Cannot inspect export disk space: {}",
            std::io::Error::last_os_error()
        ));
    }
    let space = unsafe { space.assume_init() };
    let available = (space.f_bavail as u64).saturating_mul(space.f_frsize as u64);
    let spool = (bounds.1 + bounds.2) * u64::from(request.options.channels) * 4;
    let encoded = bounds.3
        * u64::from(request.options.channels)
        * request.options.format.encoding().bytes() as u64;
    let needed = spool
        + encoded
            * if request.options.format.external() {
                2
            } else {
                1
            }
        + 16 * 1024 * 1024;
    if available < needed {
        return Err(format!(
            "Export needs approximately {} MiB of free disk space; {} MiB is available",
            needed / (1024 * 1024),
            available / (1024 * 1024)
        ));
    }
    let raw = stage.path.join("render.f32");
    render::run(captured, request, &raw, bounds, &cancel, progress)?;
    let file = stage
        .path
        .join(format!("master.{}", request.options.format.extension()));
    let (peak, gain) = encode(
        &raw,
        &file,
        bounds.1,
        bounds.2,
        request.repeats,
        &request.options,
        &cancel,
        progress,
    )?;
    std::fs::remove_file(raw).map_err(|e| e.to_string())?;
    let mut receipt = create(&stage.path.join("export.json"))?;
    let metadata = serde_json::json!({"schema":1,"project_revision":revision,"source":format!("{:?}",request.source),"scene":request.scene,"decks_included":request.decks,"output_alias":request.output_alias,"range_seconds":[request.start,request.end],"repeats":request.repeats,"tail_seconds":request.tail,"rate":request.options.rate,"channels":request.options.channels,"format":request.options.format.title(),"frames":bounds.3,"source_peak":peak,"normalization_gain":gain,"dither":request.options.dither,"render":"independent native graph; fresh DSP state; actual preroll; source channels in saved order; no physical input or device opened"});
    receipt
        .write_all(&serde_json::to_vec_pretty(&metadata).map_err(|e| e.to_string())?)
        .and_then(|_| receipt.sync_all())
        .map_err(|e| e.to_string())?;
    let _commit = permit.commit().map_err(|e| e.to_string())?;
    crate::portable_project::publish(stage, destination, &cancel)?;
    Ok(Outcome {
        folder: destination.to_owned(),
        frames: bounds.3,
        rate: request.options.rate,
        channels: request.options.channels,
        peak,
        gain,
    })
}
