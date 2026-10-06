use super::*;
use crate::audio_delivery::{wav, Format, Options};
use std::time::Instant;

pub(crate) const STANDARD_OUTPUT: u64 = u64::MAX;
const SEGMENT_BYTES: u64 = 256 * 1024 * 1024;
#[derive(Clone, Debug)]
pub(crate) struct Request {
    pub alias: u64,
    pub output_channels: Option<Vec<u16>>,
    pub options: Options,
    pub seconds: u32,
    pub epoch: u64,
}
#[derive(Clone, Debug)]
pub(crate) struct Outcome {
    pub folder: PathBuf,
    pub files: Vec<PathBuf>,
    pub frames: u64,
    pub rate: u32,
    pub channels: u16,
    pub warning: Option<String>,
}

impl Recorder {
    pub(super) fn fail(&self, kind: u8) {
        let _ = self
            .shared
            .failure
            .compare_exchange(0, kind, Ordering::Relaxed, Ordering::Relaxed);
        self.shared.fault.store(true, Ordering::Release);
        self.stop();
    }
    /// Apply one recording activation at a graph boundary.
    /// Takes the current graph rate; installs a bounded channel plan and acknowledges only the current source epoch without heap work.
    pub(crate) fn begin_delivery(&self, rate: u32) {
        let Some(mut activation) = self.starting.try_lock() else {
            return;
        };
        let Ok(start) = activation.pop() else {
            return;
        };
        let accepted = start.request == self.shared.request.load(Ordering::Acquire)
            && self.shared.busy.load(Ordering::Acquire)
            && !self.shared.stop.load(Ordering::Acquire)
            && !self.shared.preview.load(Ordering::Acquire)
            && start.epoch == self.epoch()
            && start.rate == rate;
        if accepted {
            self.shared.generation.fetch_add(1, Ordering::AcqRel);
            self.shared.count.store(0, Ordering::Release);
            self.shared.fault.store(false, Ordering::Release);
            self.shared.failure.store(0, Ordering::Release);
            self.shared.limit.store(start.limit, Ordering::Release);
            for (index, packed) in self.shared.output_channels.iter().enumerate() {
                let mut value = 0_u64;
                for byte in 0..8 {
                    if let Some(channel) = start.channels.get(index * 8 + byte) {
                        value |= u64::from(*channel) << (byte * 8);
                    }
                }
                packed.store(value, Ordering::Relaxed);
            }
            self.shared
                .output_width
                .store(start.width, Ordering::Release);
            self.shared.alias.store(start.alias, Ordering::Release);
        }
        self.shared.accepted.store(accepted, Ordering::Release);
        self.shared
            .acknowledged
            .store(start.request, Ordering::Release);
    }
    /// Capture the actual converted output.
    /// Takes a final device buffer and physical width; submits exact selected channels after ramps, limiting, mapping and sample conversion.
    pub(crate) fn converted<T>(&self, data: &[T], channels: usize)
    where
        T: cpal::Sample + Copy,
        f64: cpal::FromSample<T>,
    {
        let alias = self.alias();
        let width = usize::from(self.shared.output_width.load(Ordering::Acquire));
        if alias == 0 || width == 0 || self.shared.stop.load(Ordering::Acquire) {
            return;
        }
        if channels == 0 || data.len() % channels != 0 {
            self.fail(1);
            return;
        }
        let indices: [usize; 26] = std::array::from_fn(|i| {
            ((self.shared.output_channels[i / 8].load(Ordering::Relaxed) >> (i % 8 * 8)) & 255)
                as usize
        });
        if indices[..width].iter().any(|&i| i >= channels) {
            self.fail(1);
            return;
        }
        for physical in data.chunks_exact(channels) {
            let mut frame = [0.0; 32];
            for (slot, &index) in indices[..width].iter().enumerate() {
                frame[slot] = cpal::Sample::to_sample::<f64>(physical[index]) as f32;
            }
            if frame[..width].iter().any(|v| !v.is_finite()) {
                self.fail(4);
                return;
            }
            self.capture(alias, frame, true);
        }
    }
    /// Read and reset the recorded-source peak.
    /// Takes this recorder; returns the maximum submitted finite sample magnitude since the last native meter read.
    pub(crate) fn delivery_peak(&self) -> f32 {
        f32::from_bits(self.shared.peak.swap(0, Ordering::AcqRel))
    }
    /// Record a live output or raw routed alias.
    /// Takes reviewed source/format, a new folder and a stop flag; writes bounded segments on a worker and preserves every finalized prefix on stop or source failure.
    pub(crate) fn write_delivery(
        &self,
        request: &Request,
        destination: &Path,
        stop: &AtomicBool,
    ) -> Result<Outcome, String> {
        request.options.validate()?;
        if stop.load(Ordering::Acquire) {
            return Err("Recording cancelled before activation".into());
        }
        if request.alias == 0
            || request.options.normalize
            || !(1..=43200).contains(&request.seconds)
            || !destination.is_absolute()
            || destination.exists()
        {
            return Err("Recording needs a reviewed source, 1 second–12 hour duration and a new absolute folder; normalization belongs to offline export".into());
        }
        let width = usize::from(request.options.channels);
        let limit = u64::from(request.options.rate) * u64::from(request.seconds);
        let bytes = limit * width as u64 * request.options.format.encoding().bytes() as u64;
        if bytes > SEGMENT_BYTES * 256 {
            return Err(
                "Recording exceeds the 256-segment limit; reduce duration, rate or width".into(),
            );
        }
        let mut indices = [0_u16; 26];
        if let Some(channels) = &request.output_channels {
            if channels.len() != width
                || channels
                    .iter()
                    .any(|&c| usize::from(c) >= super::super::model::MAX_PHYSICAL_CHANNELS)
            {
                return Err("Reviewed recording output channels are unavailable".into());
            }
            indices[..width].copy_from_slice(channels);
        }
        if self.shared.preview.load(Ordering::Acquire) || self.epoch() != request.epoch {
            return Err(
                "Recording source changed or licensed preview is active; review again".into(),
            );
        }
        if self.shared.busy.swap(true, Ordering::AcqRel) {
            return Err("An audio recording is already active".into());
        }
        struct Guard<'a>(&'a Recorder);
        impl Drop for Guard<'_> {
            fn drop(&mut self) {
                let _activation = self.0.starting.lock();
                self.0.stop();
                self.0.shared.alias.store(0, Ordering::Release);
                self.0.shared.output_width.store(0, Ordering::Release);
                self.0.shared.limit.store(0, Ordering::Release);
                self.0.shared.busy.store(false, Ordering::Release);
            }
        }
        let _guard = Guard(self);
        let stage = crate::portable_project::Stage::new(
            destination
                .parent()
                .ok_or("Recording folder needs an existing parent")?,
        )?;
        let current_first = destination.join("take-0001.wav");
        write_metadata(
            &stage.path,
            request,
            &[current_first.clone()],
            0,
            Some("Preparing recording"),
        )?;
        let _ownership = crate::audio_delivery::recovery::lock_recording(&stage.path)?;
        let parent = std::ffi::CString::new(stage.path.as_os_str().as_encoded_bytes())
            .map_err(|e| e.to_string())?;
        let mut space = std::mem::MaybeUninit::<libc::statvfs>::uninit();
        if unsafe { libc::statvfs(parent.as_ptr(), space.as_mut_ptr()) } != 0 {
            return Err(format!(
                "Cannot inspect recording disk space: {}",
                std::io::Error::last_os_error()
            ));
        }
        let space = unsafe { space.assume_init() };
        let available = (space.f_bavail as u64).saturating_mul(space.f_frsize as u64);
        let external = matches!(
            request.options.format,
            Format::Flac16 | Format::Flac24 | Format::Mp3
        );
        let needed = bytes * if external { 2 } else { 1 } + 16 * 1024 * 1024;
        if available < needed {
            return Err(format!(
                "Recording needs approximately {} MiB of free disk space; {} MiB is available",
                needed / (1024 * 1024),
                available / (1024 * 1024)
            ));
        }
        crate::portable_project::publish(stage, destination, &AtomicBool::new(false))?;
        let first_writer = wav::Writer::new(
            &current_first,
            request.options.channels,
            request.options.rate,
            request.options.format.encoding(),
            request.options.dither,
        )?;
        while self.receiver.lock().pop().is_ok() {}
        {
            let mut activation = self.starting.lock();
            while activation.pop().is_ok() {}
        }
        let id = self.shared.request.fetch_add(1, Ordering::AcqRel) + 1;
        self.shared.stop.store(false, Ordering::Release);
        self.shared.peak.store(0, Ordering::Release);
        self.starts
            .lock()
            .push(DeliveryStart {
                request: id,
                alias: request.alias,
                epoch: request.epoch,
                rate: request.options.rate,
                limit,
                width: if request.output_channels.is_some() {
                    width as u8
                } else {
                    0
                },
                channels: indices,
            })
            .map_err(|_| "Recording activation slot is busy")?;
        let deadline = Instant::now() + Duration::from_secs(10);
        while self.shared.acknowledged.load(Ordering::Acquire) != id {
            if stop.load(Ordering::Acquire)
                || self.epoch() != request.epoch
                || Instant::now() > deadline
            {
                return Err(
                    "Recording activation cancelled or the audio callback is unavailable".into(),
                );
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        if !self.shared.accepted.load(Ordering::Acquire) {
            return Err("Recording source/rate changed before activation; review again".into());
        }
        let generation = self.shared.generation.load(Ordering::Acquire);
        let per_segment =
            SEGMENT_BYTES / (width as u64 * request.options.format.encoding().bytes() as u64);
        let mut files = Vec::new();
        let mut total = 0_u64;
        let mut writer = Some(first_writer);
        let mut current = current_first;
        let mut segment = 0_u64;
        let mut closed_frames = 0_u64;
        let mut buffered = 0_usize;
        let mut buffer = [0.0_f32; 26 * 1024];
        let mut warning = None;
        let mut last = Instant::now();
        loop {
            if self.epoch() != request.epoch {
                warning.get_or_insert_with(|| {
                    "Recording source changed; valid prefix retained".into()
                });
                self.stop();
            }
            if stop.load(Ordering::Acquire) {
                self.stop();
            }
            if self.shared.fault.load(Ordering::Acquire) {
                warning = Some(
                    match self.shared.failure.load(Ordering::Acquire) {
                        1 => "Selected source lost complete input or output channels",
                        2 => {
                            "Recording queue overflowed; saved files end before the missing frames"
                        }
                        3 => "Recording source changed",
                        4 => "Selected source produced invalid samples",
                        _ => "Recording source failed",
                    }
                    .to_owned(),
                );
                self.stop();
            }
            if total >= limit {
                self.stop();
            }
            let sample = match self.next_sample(Duration::from_millis(20)) {
                Some(s) if s.generation != generation => continue,
                Some(s) => Some(s),
                None => {
                    if self.shared.stop.load(Ordering::Acquire) {
                        break;
                    }
                    if last.elapsed() > Duration::from_secs(5) {
                        warning = Some("Audio source stopped producing frames".into());
                        break;
                    }
                    continue;
                }
            };
            let Some(sample) = sample else {
                break;
            };
            if total >= limit {
                break;
            }
            if sample.index != total || sample.frame[..width].iter().any(|v| !v.is_finite()) {
                warning =
                    Some("Recording has missing or invalid frames; valid prefix retained".into());
                break;
            }
            if writer.is_none() {
                current = destination.join(format!("take-{:04}.wav", files.len() + 1));
                match wav::Writer::new(
                    &current,
                    request.options.channels,
                    request.options.rate,
                    request.options.format.encoding(),
                    request.options.dither,
                ) {
                    Ok(w) => writer = Some(w),
                    Err(e) => {
                        warning = Some(format!("Cannot open recording segment: {e}"));
                        break;
                    }
                }
                segment = 0;
            }
            buffer[buffered * width..(buffered + 1) * width]
                .copy_from_slice(&sample.frame[..width]);
            buffered += 1;
            segment += 1;
            total += 1;
            last = Instant::now();
            if buffered == 1024 || segment == per_segment {
                if let Err(e) = writer
                    .as_mut()
                    .unwrap()
                    .append(&buffer[..buffered * width], 1.0)
                {
                    warning = Some(e);
                    buffered = 0;
                    break;
                }
                buffered = 0;
            }
            if segment == per_segment {
                let w = writer.take().unwrap();
                match w.finish() {
                    Ok(frames) => {
                        closed_frames += frames;
                        files.push(current.clone());
                        if let Err(e) =
                            write_metadata(destination, request, &files, total, Some("Recording"))
                        {
                            warning = Some(e);
                            break;
                        }
                    }
                    Err(e) => {
                        files.push(current.clone());
                        warning = Some(e);
                        break;
                    }
                }
            }
        }
        self.stop();
        self.shared.alias.store(0, Ordering::Release);
        if let Some(mut w) = writer {
            if buffered > 0 {
                if let Err(e) = w.append(&buffer[..buffered * width], 1.0) {
                    warning = Some(format!("{}; {e}", warning.unwrap_or_default()));
                }
            }
            let complete = w.frames();
            total = closed_frames + complete;
            match w.finish() {
                Ok(_) => {
                    if complete > 0 {
                        files.push(current.clone());
                    } else {
                        let _ = std::fs::remove_file(&current);
                    }
                }
                Err(e) => {
                    files.push(current.clone());
                    warning = Some(format!("{}; {e}", warning.unwrap_or_default()));
                }
            }
        }
        if files.is_empty() {
            return Err(format!(
                "No complete audio was recorded in {}: {}",
                destination.display(),
                warning.unwrap_or_else(|| "capture stopped before its first frame".into())
            ));
        }
        if matches!(
            request.options.format,
            Format::Flac16 | Format::Flac24 | Format::Mp3
        ) {
            for file in &mut files {
                let target = file.with_extension(request.options.format.extension());
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
                    .arg(&*file)
                    .args([
                        "-map",
                        "0:a:0",
                        "-vn",
                        "-map_metadata",
                        "-1",
                        "-threads",
                        "1",
                    ]);
                if request.options.format == Format::Mp3 {
                    command.args(["-c:a", "libmp3lame", "-b:a", "320k"]);
                } else {
                    command.args(["-c:a", "flac"]);
                }
                command.arg("-n").arg(&target);
                match crate::video::decoder::Process::start(&mut command).and_then(|mut p| {
                    p.capture(1024, Duration::from_secs(3600), &AtomicBool::new(false))
                }) {
                    Ok(_) => {
                        if let Err(e) = std::fs::File::open(&target).and_then(|f| f.sync_all()) {
                            warning = Some(format!(
                                "Encoded file could not flush: {e}; original WAV retained"
                            ));
                            let _ = std::fs::remove_file(&target);
                            break;
                        }
                        *file = target;
                    }
                    Err(e) => {
                        let _ = std::fs::remove_file(&target);
                        warning = Some(format!(
                            "Encoding failed: {e}; original finalized WAV files retained"
                        ));
                        break;
                    }
                }
            }
        }
        if let Err(e) = write_metadata(destination, request, &files, total, warning.as_deref()) {
            warning = Some(format!(
                "{}; recording metadata could not flush: {e}",
                warning.unwrap_or_default()
            ));
        }
        Ok(Outcome {
            folder: destination.to_owned(),
            files,
            frames: total,
            rate: request.options.rate,
            channels: request.options.channels,
            warning,
        })
    }
}
fn write_metadata(
    folder: &Path,
    request: &Request,
    files: &[PathBuf],
    frames: u64,
    warning: Option<&str>,
) -> Result<(), String> {
    let metadata = serde_json::json!({"schema":1,"source_alias":request.alias.to_string(),"output_channels":request.output_channels,"rate":request.options.rate,"channels":request.options.channels,"format":request.options.format.title(),"frames":frames,"files":files.iter().map(|p|p.file_name().unwrap().to_string_lossy()).collect::<Vec<_>>(),"status":warning.unwrap_or("Complete"),"recoverable_wav_prefixes":true});
    let mut metadata = metadata;
    metadata["wav_encoding"] = match request.options.format.encoding() {
        wav::Encoding::Float32 => "float32",
        wav::Encoding::Pcm16 => "pcm16",
        wav::Encoding::Pcm24 => "pcm24",
    }
    .into();
    crate::audio_delivery::recovery::persist(folder, &metadata)
}

#[cfg(test)]
mod tests;
