//! Record aliases feed a bounded queue; a worker writes and publishes complete WAV files.
use super::prepared::Frame;
use parking_lot::Mutex;
use std::io::{Seek, SeekFrom, Write};
use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicU8, Ordering},
        Arc,
    },
    time::Duration,
};

const CAPACITY: usize = 32768;
const MAX_BYTES: u64 = 128 * 1024 * 1024;
pub(crate) mod delivery;
pub(crate) mod placement;
#[derive(Clone, Copy)]
struct DeliveryStart {
    request: u64,
    alias: u64,
    epoch: u64,
    rate: u32,
    limit: u64,
    width: u8,
    channels: [u16; 26],
}
#[derive(Clone, Copy)]
struct Sample {
    frame: Frame,
    index: u64,
    generation: u64,
}
struct Shared {
    alias: AtomicU64,
    count: AtomicU64,
    fault: AtomicBool,
    stop: AtomicBool,
    busy: AtomicBool,
    preview: AtomicBool,
    generation: AtomicU64,
    epoch: AtomicU64,
    output_width: AtomicU8,
    output_channels: [AtomicU64; 4],
    limit: AtomicU64,
    failure: AtomicU8,
    peak: AtomicU32,
    request: AtomicU64,
    acknowledged: AtomicU64,
    accepted: AtomicBool,
    alignment_pending: AtomicU32,
    delay_frames: AtomicU32,
    origin_seconds: AtomicU64,
    origin_available: AtomicBool,
    origin_rate: AtomicU32,
    placement_valid: AtomicBool,
    clock_bound: AtomicBool,
    clock_pointer: AtomicU64,
    clock_bpm: AtomicU32,
}
#[derive(Clone)]
pub(crate) struct Recorder {
    sender: Arc<Mutex<rtrb::Producer<Sample>>>,
    receiver: Arc<Mutex<rtrb::Consumer<Sample>>>,
    shared: Arc<Shared>,
    starts: Arc<Mutex<rtrb::Producer<DeliveryStart>>>,
    starting: Arc<Mutex<rtrb::Consumer<DeliveryStart>>>,
}
impl Default for Recorder {
    fn default() -> Self {
        let (sender, receiver) = rtrb::RingBuffer::new(CAPACITY);
        let (starts, starting) = rtrb::RingBuffer::new(1);
        Self {
            sender: Arc::new(Mutex::new(sender)),
            receiver: Arc::new(Mutex::new(receiver)),
            starts: Arc::new(Mutex::new(starts)),
            starting: Arc::new(Mutex::new(starting)),
            shared: Arc::new(Shared {
                alias: AtomicU64::new(0),
                count: AtomicU64::new(0),
                fault: AtomicBool::new(false),
                stop: AtomicBool::new(false),
                busy: AtomicBool::new(false),
                preview: AtomicBool::new(false),
                generation: AtomicU64::new(0),
                epoch: AtomicU64::new(0),
                output_width: AtomicU8::new(0),
                output_channels: std::array::from_fn(|_| AtomicU64::new(0)),
                limit: AtomicU64::new(0),
                failure: AtomicU8::new(0),
                peak: AtomicU32::new(0),
                request: AtomicU64::new(0),
                acknowledged: AtomicU64::new(0),
                accepted: AtomicBool::new(false),
                alignment_pending: AtomicU32::new(0),
                delay_frames: AtomicU32::new(0),
                origin_seconds: AtomicU64::new(0),
                origin_available: AtomicBool::new(false),
                origin_rate: AtomicU32::new(0),
                placement_valid: AtomicBool::new(true),
                clock_bound: AtomicBool::new(false),
                clock_pointer: AtomicU64::new(0),
                clock_bpm: AtomicU32::new(0),
            }),
        }
    }
}
impl Recorder {
    /// Confirm the time map used by an active raw capture.
    /// Takes the renderer's retained conductor and tempo; invalidates automatic placement after a clock change while keeping recorded audio.
    pub(crate) fn mark_clock(&self, conductor: Option<&Arc<crate::engine::midi_data::Conductor>>, bpm: f32) {
        if self.alias() == 0 || self.shared.stop.load(Ordering::Acquire) {
            return;
        }
        if self.shared.clock_bound.load(Ordering::Acquire) {
            let pointer = conductor.map_or(0, |clock| Arc::as_ptr(clock) as u64);
            if pointer != self.shared.clock_pointer.load(Ordering::Relaxed)
                || pointer == 0 && bpm.to_bits() != self.shared.clock_bpm.load(Ordering::Relaxed)
            { self.shared.placement_valid.store(false, Ordering::Release); }
        }
    }
    /// Publish whether the current graph is still warming or changing its delay.
    /// Takes scalar timing from the audio boundary; refuses new recordings until alignment is settled without blocking existing audio.
    pub(crate) fn alignment_pending(&self, frames: u32) {
        self.shared
            .alignment_pending
            .store(frames, Ordering::Release);
    }
    /// Retain the first captured frame's transport origin and graph delay.
    /// Takes the selected alias, software delay and transport seconds; publishes timing before the first numbered frame without callback allocation.
    pub(crate) fn mark_origin(&self, alias: u64, delay: u32, seconds: f64) {
        if self.alias() != alias || self.shared.stop.load(Ordering::Acquire) {
            return;
        }
        if self.alias() == alias && self.shared.origin_available.load(Ordering::Acquire) {
            let rate = self.shared.origin_rate.load(Ordering::Relaxed);
            let origin = f64::from_bits(self.shared.origin_seconds.load(Ordering::Relaxed));
            let expected = origin + self.frames() as f64 / f64::from(rate);
            if rate == 0 || delay != self.shared.delay_frames.load(Ordering::Relaxed)
                || !seconds.is_finite() || (seconds - expected).abs() > 0.25 / f64::from(rate) {
                self.shared.placement_valid.store(false, Ordering::Release);
            }
        }
        if self.alias() == alias
            && self.frames() == 0
            && !self.shared.origin_available.load(Ordering::Acquire)
            && seconds.is_finite()
        {
            self.shared.delay_frames.store(delay, Ordering::Relaxed);
            self.shared
                .origin_seconds
                .store(seconds.to_bits(), Ordering::Relaxed);
            self.shared.origin_available.store(true, Ordering::Release);
        }
    }
    /// Read recording placement after its first frame was accepted.
    /// Takes the source rate; returns optional transport origin, exact graph delay and aligned source time for the recording manifest.
    fn placement(&self, rate: u32) -> serde_json::Value {
        if !self.shared.origin_available.load(Ordering::Acquire)
            || !self.shared.placement_valid.load(Ordering::Acquire)
        {
            return serde_json::Value::Null;
        }
        let seconds = f64::from_bits(self.shared.origin_seconds.load(Ordering::Relaxed));
        let delay = self.shared.delay_frames.load(Ordering::Relaxed);
        serde_json::json!({ "transport_seconds_at_start": seconds, "graph_delay_frames": delay, "source_seconds_at_start": seconds - f64::from(delay) / f64::from(rate) })
    }
    /// Wait for one numbered audio frame on the writer.
    /// Takes a short timeout; returns the next published SPSC frame without making its producer wait for this consumer.
    fn next_sample(&self, timeout: Duration) -> Option<Sample> {
        let deadline = std::time::Instant::now() + timeout;
        loop {
            if let Ok(sample) = self.receiver.lock().pop() {
                return Some(sample);
            }
            if std::time::Instant::now() >= deadline {
                return None;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    /// Report a live-input recording policy.
    /// Takes this recorder; returns true for active raw alias capture, while recording final program outputs leaves track monitoring unchanged.
    pub(crate) fn monitoring_inputs(&self) -> bool {
        self.alias() != 0
            && !self.shared.stop.load(Ordering::Acquire)
            && self.shared.output_width.load(Ordering::Acquire) == 0
    }
    /// Check capture or recording delivery ownership.
    /// Takes this recorder; returns true through activation, capture and final encoding.
    pub(crate) fn busy(&self) -> bool {
        self.shared.busy.load(Ordering::Acquire) || self.alias() != 0
    }
    /// Read the selected record source.
    /// Takes this recorder; returns its stable alias ID, or zero while idle.
    pub(crate) fn alias(&self) -> u64 {
        self.shared.alias.load(Ordering::Acquire)
    }
    /// Capture one record-source frame.
    /// Takes its alias, rendered channels and source continuity; refuses incomplete input before submitting a numbered frame without waiting or allocation.
    pub(crate) fn capture(&self, alias: u64, frame: Frame, complete: bool) {
        let generation = self.shared.generation.load(Ordering::Acquire);
        if self.alias() != alias
            || self.shared.stop.load(Ordering::Acquire)
            || self.shared.preview.load(Ordering::Acquire)
        {
            return;
        }
        if !complete {
            self.fail(1);
            return;
        }
        let limit = self.shared.limit.load(Ordering::Relaxed);
        if limit != 0 && self.frames() >= limit {
            self.stop();
            return;
        }
        let index = self.shared.count.fetch_add(1, Ordering::Relaxed);
        if !self.sender.try_lock().is_some_and(|mut sender| {
            sender
                .push(Sample {
                    frame,
                    index,
                    generation,
                })
                .is_ok()
        }) {
            self.fail(2);
        }
        let peak = frame
            .iter()
            .filter(|s| s.is_finite())
            .fold(0.0_f32, |peak, s| peak.max(s.abs()))
            .to_bits();
        self.shared.peak.fetch_max(peak, Ordering::Relaxed);
    }
    /// Finish a capture without discarding completed audio.
    /// Takes this recorder; signals the writer to drain and finalize its bounded queue.
    pub(crate) fn stop(&self) {
        self.shared.stop.store(true, Ordering::Release);
    }
    /// Retire a record source boundary.
    /// Takes this recorder; invalidates pending writer activation and finishes an existing capture.
    pub(crate) fn invalidate(&self) {
        self.shared.epoch.fetch_add(1, Ordering::AcqRel);
        if self.alias() != 0 {
            let _ =
                self.shared
                    .failure
                    .compare_exchange(0, 3, Ordering::Relaxed, Ordering::Relaxed);
        }
        self.stop();
    }
    /// Exclude transient licensed preview audio.
    /// Takes preview ownership at a block boundary; prevents new captures and finishes existing audio before preview frames render.
    pub(crate) fn preview(&self, active: bool) {
        self.shared.preview.store(active, Ordering::Release);
        if active {
            self.invalidate();
        }
    }
    /// Read the source boundary.
    /// Takes this recorder; returns the epoch a writer must confirm before activation.
    pub(crate) fn epoch(&self) -> u64 {
        self.shared.epoch.load(Ordering::Acquire)
    }
    /// Read captured frame count.
    /// Takes this recorder; returns frames submitted by the active source.
    pub(crate) fn frames(&self) -> u64 {
        self.shared.count.load(Ordering::Relaxed)
    }
    /// Write a selected record alias.
    /// Takes source identity, width, rate, duration, new destination, cancellation and inspected epoch; returns a complete WAV path or removes the unpublished partial file.
    pub(crate) fn write(
        &self,
        alias: u64,
        channels: u16,
        rate: u32,
        seconds: u32,
        destination: &Path,
        cancel: &AtomicBool,
        epoch: u64,
    ) -> Result<PathBuf, String> {
        self.write_internal(alias, channels, rate, seconds, destination, cancel, epoch, None)
    }
    /// Capture audio with its retained musical time map.
    /// Takes the selected source, file bounds, cancellation, epoch and original conductor/tempo; returns a complete WAV and guarded timing receipt without changing the project.
    pub(crate) fn write_timed(&self, alias: u64, channels: u16, rate: u32, seconds: u32, destination: &Path, cancel: &AtomicBool, epoch: u64, conductor: Option<Arc<crate::engine::midi_data::Conductor>>, bpm: f32) -> Result<PathBuf, String> {
        self.write_internal(alias, channels, rate, seconds, destination, cancel, epoch, Some((conductor, bpm)))
    }
    fn write_internal(&self, alias: u64, channels: u16, rate: u32, seconds: u32, destination: &Path, cancel: &AtomicBool, epoch: u64, clock: Option<(Option<Arc<crate::engine::midi_data::Conductor>>, f32)>) -> Result<PathBuf, String> {
        let conductor = clock.as_ref().map(|(original, bpm)| placement::conductor(original.as_ref(), *bpm)).transpose()?;
        if alias == 0
            || !(1..=super::model::MAX_RECORD_CHANNELS as u16).contains(&channels)
            || !(8000..=384000).contains(&rate)
            || !(1..=600).contains(&seconds)
        {
            return Err("Unsupported record source, rate or duration".into());
        }
        if self.shared.preview.load(Ordering::Acquire) {
            return Err("Stop licensed provider preview before capturing a record source".into());
        }
        if self.shared.alignment_pending.load(Ordering::Acquire) != 0 {
            return Err(
                "Wait for latency histories and their transition to settle before recording".into(),
            );
        }
        let limit = u64::from(rate) * u64::from(seconds);
        if limit * u64::from(channels) * 4 > MAX_BYTES {
            return Err("Capture exceeds the 128 MiB file limit; choose a shorter duration".into());
        }
        let mut metadata_name = destination
            .file_name()
            .ok_or("Capture needs a file name")?
            .to_os_string();
        metadata_name.push(".omatainer.json");
        let metadata = destination.with_file_name(metadata_name);
        if destination.exists() || metadata.exists() {
            return Err("Capture destination already exists; choose a new file".into());
        }
        if self.shared.busy.swap(true, Ordering::AcqRel) {
            return Err("A record-source capture is already active".into());
        }
        struct Guard<'a>(&'a Recorder);
        impl Drop for Guard<'_> {
            fn drop(&mut self) {
                let _activation = self.0.starting.lock();
                self.0.shared.alias.store(0, Ordering::Release);
                self.0.shared.stop.store(true, Ordering::Release);
                self.0.shared.busy.store(false, Ordering::Release);
                self.0.shared.output_width.store(0, Ordering::Release);
                self.0.shared.limit.store(0, Ordering::Release);
            }
        }
        let _guard = Guard(self);
        let temporary = destination.with_extension(format!(
            "omatainer-record-{}.part",
            crate::sampler_bank::BankId::new().map_err(|error| error.to_string())?
        ));
        let temporary_metadata = temporary.with_extension("timing.part");
        let result = (|| -> Result<PathBuf, String> {
            let file = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .create_new(true)
                .open(&temporary)
                .map_err(|error| error.to_string())?;
            let durable = file.try_clone().map_err(|error| error.to_string())?;
            let mut writer = std::io::BufWriter::new(file);
            header(&mut writer, channels, rate, 0).map_err(|error| error.to_string())?;
            while self.receiver.lock().pop().is_ok() {}
            let mut activation = self.starting.lock();
            while activation.pop().is_ok() {}
            self.shared.request.fetch_add(1, Ordering::AcqRel);
            let generation = self.shared.generation.fetch_add(1, Ordering::AcqRel) + 1;
            self.shared.count.store(0, Ordering::Release);
            self.shared.origin_rate.store(rate, Ordering::Relaxed);
            self.shared.placement_valid.store(true, Ordering::Release);
            self.shared.clock_pointer.store(clock.as_ref().and_then(|(original, _)| original.as_ref()).map_or(0, |original| Arc::as_ptr(original) as u64), Ordering::Relaxed);
            self.shared.clock_bpm.store(clock.as_ref().map_or(0, |(_, bpm)| bpm.to_bits()), Ordering::Relaxed);
            self.shared.clock_bound.store(clock.is_some(), Ordering::Release);
            self.shared.fault.store(false, Ordering::Release);
            self.shared.failure.store(0, Ordering::Release);
            self.shared.output_width.store(0, Ordering::Release);
            self.shared.origin_available.store(false, Ordering::Release);
            self.shared.limit.store(0, Ordering::Release);
            self.shared.stop.store(false, Ordering::Release);
            if self.shared.preview.load(Ordering::Acquire) {
                return Err(
                    "Stop licensed provider preview before capturing a record source".into(),
                );
            }
            if cancel.load(Ordering::Acquire) || self.epoch() != epoch {
                return Err("Record-source capture cancelled".into());
            }
            if self.shared.alignment_pending.load(Ordering::Acquire) != 0 {
                return Err("Latency alignment changed before recording activation".into());
            }
            self.shared.alias.store(alias, Ordering::Release);
            drop(activation);
            if self.epoch() != epoch || self.shared.preview.load(Ordering::Acquire) {
                self.stop();
                return Err("Record source changed before activation".into());
            }
            let mut frames = 0_u64;
            let mut last_frame = std::time::Instant::now();
            while frames < limit {
                if self.epoch() != epoch {
                    self.stop();
                }
                if cancel.load(Ordering::Acquire) {
                    return Err("Record-source capture cancelled; no file was published".into());
                }
                if self.shared.fault.load(Ordering::Acquire) {
                    return Err("Record-source capture has missing input, invalid frames or queue overflow; no incomplete file was published".into());
                }
                match self.next_sample(Duration::from_millis(20)) {
                    Some(sample) if sample.generation != generation => {},
                    Some(sample) => {
                        if sample.index != frames || sample.frame[..usize::from(channels)].iter().any(|value| !value.is_finite()) { return Err("Record-source capture has missing or invalid frames".into()); }
                        for value in &sample.frame[..usize::from(channels)] { writer.write_all(&value.to_le_bytes()).map_err(|error| error.to_string())?; }
                        frames += 1; last_frame = std::time::Instant::now();
                    }
                    None if self.shared.stop.load(Ordering::Acquire) => break,
                    None if last_frame.elapsed() > Duration::from_secs(5) => return Err("Record source stopped producing frames; no empty or incomplete file was published".into()),
                    None => {},
                }
            }
            self.shared.alias.store(0, Ordering::Release);
            if self.shared.fault.load(Ordering::Acquire) {
                return Err("Record-source capture has missing input, invalid frames or queue overflow; no incomplete file was published".into());
            }
            if frames == 0 || cancel.load(Ordering::Acquire) {
                return Err("Record-source capture ended without complete audio".into());
            }
            writer
                .seek(SeekFrom::Start(0))
                .map_err(|error| error.to_string())?;
            header(
                &mut writer,
                channels,
                rate,
                (frames * u64::from(channels) * 4) as u32,
            )
            .map_err(|error| error.to_string())?;
            writer.flush().map_err(|error| error.to_string())?;
            durable.sync_all().map_err(|error| error.to_string())?;
            let audio_sha256 = placement::hash_file(&durable, cancel)?;
            let exact_bpm = clock.as_ref().filter(|(original, _)| original.is_none()).map(|(_, bpm)| f64::from(*bpm));
            let timing = serde_json::json!({"schema": 1, "source_alias": alias.to_string(), "rate": rate, "channels": channels, "frames": frames, "audio_sha256": audio_sha256, "conductor": conductor, "exact_bpm": exact_bpm, "placement": self.placement(rate)});
            let mut metadata_file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary_metadata)
                .map_err(|error| error.to_string())?;
            metadata_file
                .write_all(&serde_json::to_vec_pretty(&timing).map_err(|error| error.to_string())?)
                .map_err(|error| error.to_string())?;
            metadata_file
                .sync_all()
                .map_err(|error| error.to_string())?;
            if cancel.load(Ordering::Acquire) {
                return Err("Record-source capture cancelled; no file was published".into());
            }
            std::fs::hard_link(&temporary_metadata, &metadata)
                .map_err(|error| error.to_string())?;
            if let Err(error) = std::fs::hard_link(&temporary, destination) {
                let _ = std::fs::remove_file(&metadata);
                return Err(error.to_string());
            }
            if let Some(parent) = destination
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                std::fs::File::open(parent)
                    .and_then(|file| file.sync_all())
                    .map_err(|error| {
                        format!(
                            "Capture was published to {}; directory sync failed: {error}",
                            destination.display()
                        )
                    })?;
            }
            Ok(destination.to_owned())
        })();
        let _ = std::fs::remove_file(&temporary);
        let _ = std::fs::remove_file(&temporary_metadata);
        result
    }
}

/// Write a bounded floating-point WAV header.
/// Takes a seekable writer, channel width, rate and data bytes; writes the 44-byte IEEE float header without inferring speaker positions.
fn header(writer: &mut impl Write, channels: u16, rate: u32, bytes: u32) -> std::io::Result<()> {
    writer.write_all(b"RIFF")?;
    writer.write_all(&(36 + bytes).to_le_bytes())?;
    writer.write_all(b"WAVEfmt ")?;
    writer.write_all(&16_u32.to_le_bytes())?;
    writer.write_all(&3_u16.to_le_bytes())?;
    writer.write_all(&channels.to_le_bytes())?;
    writer.write_all(&rate.to_le_bytes())?;
    writer.write_all(&(rate * u32::from(channels) * 4).to_le_bytes())?;
    writer.write_all(&(channels * 4).to_le_bytes())?;
    writer.write_all(&32_u16.to_le_bytes())?;
    writer.write_all(b"data")?;
    writer.write_all(&bytes.to_le_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn incomplete_input_refuses_publication_and_the_next_complete_capture_can_finish() {
        let recorder = Recorder::default();
        let directory = std::env::temp_dir().join(format!(
            "omatainer-input-record-{}",
            crate::sampler_bank::BankId::new().unwrap()
        ));
        std::fs::create_dir(&directory).unwrap();
        for complete in [false, true] {
            let path = directory.join(if complete {
                "complete.wav"
            } else {
                "incomplete.wav"
            });
            let capture = recorder.clone();
            let destination = path.clone();
            let epoch = recorder.epoch();
            let writer = std::thread::spawn(move || {
                capture.write(7, 2, 48000, 1, &destination, &AtomicBool::new(false), epoch)
            });
            let deadline = std::time::Instant::now() + Duration::from_secs(3);
            while recorder.alias() != 7 {
                assert!(std::time::Instant::now() < deadline);
                std::thread::sleep(Duration::from_millis(1));
            }
            recorder.capture(7, [0.25; 32], true);
            assert_eq!(
                crate::engine::test_alloc::measure(|| recorder.capture(7, [0.0; 32], complete)),
                crate::engine::test_alloc::Counts::default()
            );
            recorder.stop();
            let result = writer.join().unwrap();
            if complete {
                assert_eq!(result.unwrap(), path);
                let decoded = crate::engine::decode::decode_audio(&path).unwrap();
                assert_eq!(decoded.sample.frames(), 2);
                assert_eq!(&decoded.sample.data[..4], &[0.25, 0.25, 0.0, 0.0]);
            } else {
                assert!(result.unwrap_err().contains("missing input"));
                assert!(!path.exists());
                assert_eq!(std::fs::read_dir(&directory).unwrap().count(), 0);
            }
        }
        std::fs::remove_dir_all(directory).unwrap();
    }
    #[test]
    fn record_source_writes_every_channel_and_cancel_preserves_existing_files() {
        let recorder = Recorder::default();
        let path = std::env::temp_dir().join(format!(
            "omatainer-record-{}.wav",
            crate::sampler_bank::BankId::new().unwrap()
        ));
        let capture = recorder.clone();
        let epoch = recorder.epoch();
        let destination = path.clone();
        let writer = std::thread::spawn(move || {
            capture.write(
                7,
                26,
                48000,
                1,
                &destination,
                &AtomicBool::new(false),
                epoch,
            )
        });
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        while recorder.alias() != 7 {
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
        for frame in 0..512 {
            recorder.mark_origin(7, 336, 4.0 + f64::from(frame) / 48000.0);
            let samples =
                std::array::from_fn(|channel| (channel + 1) as f32 / 64.0 + frame as f32 / 65536.0);
            assert_eq!(
                crate::engine::test_alloc::measure(|| recorder.capture(7, samples, true)),
                crate::engine::test_alloc::Counts::default()
            );
        }
        recorder.invalidate();
        assert_eq!(writer.join().unwrap().unwrap(), path);
        let wav = std::fs::read(&path).unwrap();
        assert_eq!(&wav[..4], b"RIFF");
        assert_eq!(u16::from_le_bytes(wav[22..24].try_into().unwrap()), 26);
        assert_eq!(wav.len(), 44 + 512 * 26 * 4);
        let metadata_path = path.with_file_name(format!(
            "{}.omatainer.json",
            path.file_name().unwrap().to_string_lossy()
        ));
        let metadata: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&metadata_path).unwrap()).unwrap();
        assert_eq!(metadata["placement"]["transport_seconds_at_start"], 4.0);
        assert_eq!(metadata["placement"]["graph_delay_frames"], 336);
        assert_eq!(metadata["placement"]["source_seconds_at_start"], 3.993);
        let decoded = crate::engine::decode::decode_audio(&path).unwrap();
        assert_eq!(decoded.sample.ch, 26);
        assert_eq!(decoded.sample.frames(), 512);
        for (index, sample) in wav[44..].chunks_exact(4).enumerate() {
            let actual = f32::from_le_bytes(sample.try_into().unwrap());
            assert_eq!(
                actual,
                (index % 26 + 1) as f32 / 64.0 + (index / 26) as f32 / 65536.0
            );
            assert_eq!(decoded.sample.data[index], actual);
        }
        assert!(recorder
            .write(
                7,
                26,
                48000,
                1,
                &path,
                &AtomicBool::new(false),
                recorder.epoch()
            )
            .is_err());
        assert_eq!(std::fs::read(&path).unwrap(), wav);
        std::fs::remove_file(&path).unwrap();
        std::fs::remove_file(path.with_file_name(format!(
            "{}.omatainer.json",
            path.file_name().unwrap().to_string_lossy()
        )))
        .unwrap();
        assert!(recorder
            .write(
                7,
                2,
                48000,
                1,
                &path,
                &AtomicBool::new(true),
                recorder.epoch()
            )
            .unwrap_err()
            .contains("cancel"));
        assert!(!path.exists());
        let retired = recorder.epoch();
        recorder.invalidate();
        assert!(recorder
            .write(7, 2, 48000, 1, &path, &AtomicBool::new(false), retired)
            .is_err());
        assert_eq!(recorder.alias(), 0);
        assert!(!path.exists());
    }
}
