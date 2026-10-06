use crate::engine::RtEngine;
pub mod calibration;
pub mod config;
pub(crate) mod graph;
#[cfg(target_os = "linux")]
pub(crate) mod jack;
pub mod owner;
pub mod recovery;
pub(crate) mod routing;
use cpal::traits::{DeviceTrait, StreamTrait};
pub use owner::AudioOut;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

#[derive(Clone, Debug)]
pub struct OutputInfo {
    pub backend: String,
    pub format: String,
    pub plan: config::Plan,
}
impl From<config::Plan> for OutputInfo {
    fn from(plan: config::Plan) -> Self {
        Self {
            backend: plan.backend.clone(),
            format: plan.format.to_string(),
            plan,
        }
    }
}
pub fn start(rt: RtEngine) -> anyhow::Result<AudioOut> {
    start_with_settings(rt, &crate::preferences::Audio::default())
}
pub fn start_with_settings(
    rt: RtEngine,
    settings: &crate::preferences::Audio,
) -> anyhow::Result<AudioOut> {
    // The ALSA implementation joins its callback worker before Stream::drop
    // returns. Other backends need their own verified ownership contract.
    anyhow::ensure!(
        cfg!(target_os = "linux"),
        "Live audio ownership currently requires a supported Linux ALSA or JACK backend"
    );
    owner::start_with(rt, settings.clone(), || Native)
}
struct Native;
impl owner::Backend for Native {
    type Stream = NativeStream;
    fn select(&mut self, settings: &crate::preferences::Audio) -> Result<config::Plan, String> {
        #[cfg(target_os = "linux")]
        if settings.backend.as_deref() == Some(jack::BACKEND) { return jack::select(settings); }
        config::select(settings)
            .map(|(_, plan)| plan)
            .map_err(|e| e.to_string())
    }
    fn open(
        &mut self,
        plan: &config::Plan,
        callback: OutputCallback,
        fault: Arc<AtomicBool>,
        identity: Option<&str>,
    ) -> Result<Self::Stream, String> {
        #[cfg(target_os = "linux")]
        if plan.backend == jack::BACKEND { return jack::output(plan, callback, fault).map(NativeStream::Jack); }
        let device = config::select_exact(plan).map_err(|e| e.to_string())?;
        if identity
            .is_some_and(|expected| recovery::identity(&plan.device).as_deref() != Some(expected))
        {
            return Err(
                "Physical output changed before opening; no other output was activated".into(),
            );
        }
        let cfg = plan.config();
        let errors = callback.rt.telemetry.clone();
        let discontinuity = Arc::new(AtomicBool::new(false));
        let output_discontinuity = discontinuity.clone();
        let error = move |e: cpal::Error| {
            if e.kind() == cpal::ErrorKind::Xrun {
                discontinuity.store(true, Ordering::Release);
            }
            if errors.error(&e) && !fault.swap(true, Ordering::AcqRel) {
                eprintln!("Audio backend error: {e}");
            } else if e.kind() == cpal::ErrorKind::RealtimeDenied {
                eprintln!("Audio scheduling warning: {e}");
            }
        };
        macro_rules! build {
            ($type:ty) => {
                build::<$type>(&device, &cfg, callback, error, output_discontinuity)
                    .map(NativeStream::Alsa)
                    .map_err(|e| e.to_string())
            };
        }
        let stream = match plan.format {
            cpal::SampleFormat::F32 => build!(f32),
            cpal::SampleFormat::F64 => build!(f64),
            cpal::SampleFormat::I8 => build!(i8),
            cpal::SampleFormat::I16 => build!(i16),
            cpal::SampleFormat::I32 => build!(i32),
            cpal::SampleFormat::I64 => build!(i64),
            cpal::SampleFormat::U8 => build!(u8),
            cpal::SampleFormat::U16 => build!(u16),
            cpal::SampleFormat::U32 => build!(u32),
            cpal::SampleFormat::U64 => build!(u64),
            other => Err(format!("Unsupported sample format {other}")),
        }?;
        if identity.is_some_and(|expected| recovery::identity(&plan.device).as_deref() != Some(expected)) {
            drop(stream);
            return Err("Physical output changed while opening; activation refused".into());
        }
        Ok(stream)
    }
    fn identity(&mut self, plan: &config::Plan) -> Option<String> {
        #[cfg(target_os = "linux")]
        if plan.backend == jack::BACKEND { return jack::identity(); }
        recovery::identity(&plan.device)
    }
    fn reconnect(&mut self, target: &recovery::Target) -> Result<config::Plan, String> {
        #[cfg(target_os = "linux")]
        if target.plan.backend == jack::BACKEND {
            if target.identity.as_ref() != jack::identity().as_ref() {
                return Err(
                    "Retained graph server identity changed; preview another output explicitly"
                        .into(),
                );
            }
            let mut saved = recovery::settings(&target.plan);
            saved.sample_rate = None;
            saved.buffer_frames = None;
            return jack::select(&saved);
        }
        recovery::discover(target)
    }
    fn calibrate(
        &mut self,
        request: &calibration::Request,
        cancel: &AtomicBool,
        stopped: &Arc<AtomicBool>,
    ) -> Result<calibration::Measurement, String> {
        #[cfg(target_os = "linux")]
        if request.output.backend == jack::BACKEND { return Err("Choose the ALSA workflow for physical loopback calibration".into()); }
        calibration::native::run(request, cancel, stopped.clone())
    }
    fn maintain(&mut self, stream: &Self::Stream) -> Result<Option<u32>, String> {
        match stream {
            NativeStream::Alsa(_) => Ok(None),
            #[cfg(target_os = "linux")] NativeStream::Jack(stream) => stream.maintain().map(Some),
        }
    }
    fn play(&mut self, stream: &Self::Stream) -> Result<(), String> {
        match stream { NativeStream::Alsa(stream) => stream.play().map_err(|e| e.to_string()),
            #[cfg(target_os = "linux")] NativeStream::Jack(stream) => stream.maintain().map(|_| ()),
        }
    }
}
enum NativeStream {
    Alsa(cpal::Stream),
    #[cfg(target_os = "linux")]
    Jack(jack::Stream),
}
fn build<T>(
    device: &cpal::Device,
    cfg: &cpal::StreamConfig,
    mut callback: OutputCallback,
    err_fn: impl Fn(cpal::Error) + Send + 'static,
    discontinuity: Arc<AtomicBool>,
) -> Result<cpal::Stream, cpal::Error>
where
    T: cpal::SizedSample + cpal::FromSample<f32>,
    f64: cpal::FromSample<T>,
{
    device.build_output_stream(
        cfg.clone(),
        move |data: &mut [T], info| {
            if discontinuity.swap(false, Ordering::AcqRel) {
                callback.rt.audible.restart();
                callback.resume_ramp = Some((0, 0, 1));
            }
            let timestamp = info.timestamp();
            callback.render_timed(
                data,
                timestamp
                    .playback
                    .checked_duration_since(timestamp.callback),
            );
        },
        err_fn,
        None,
    )
}

/// A single preallocated return slot receives the unique graph when CPAL drops
/// its closure. The receiver lives through stream destruction on the owner.
struct GraphLease {
    graph: Option<Box<RtEngine>>,
    returned: Option<crossbeam_channel::Sender<Box<RtEngine>>>,
}
impl std::ops::Deref for GraphLease {
    type Target = RtEngine;
    fn deref(&self) -> &RtEngine {
        self.graph.as_deref().unwrap()
    }
}
impl std::ops::DerefMut for GraphLease {
    fn deref_mut(&mut self) -> &mut RtEngine {
        self.graph.as_deref_mut().unwrap()
    }
}
impl Drop for GraphLease {
    fn drop(&mut self) {
        if let Some(graph) = &self.graph { graph.audible.invalidate(); }
        if let Some(returned) = &self.returned {
            if let Some(graph) = self.graph.take() {
                if let Err(error) = returned.try_send(graph) {
                    // This is an invariant failure (one producer/one empty slot,
                    // receiver retained until join). Fail closed without retiring
                    // an entire render graph on a backend callback thread.
                    std::mem::forget(error.into_inner());
                }
            }
        }
    }
}

/// The CPAL closure owns this state. Producers can submit commands and read
/// snapshots, but cannot lock, inspect or mutate the renderer between blocks.
pub(crate) struct OutputCallback {
    rt: GraphLease,
    enabled: Option<Arc<AtomicBool>>,
    stopped: Option<Arc<AtomicBool>>,
    channels: usize,
    buffer: Vec<f32>,
    resume_ramp: Option<(u8, u32, u32)>,
    #[cfg(test)]
    conversion_delay: std::time::Duration,
    #[cfg(test)]
    stall_once: Option<(u32, std::time::Duration)>,
}

impl OutputCallback {
    #[cfg(test)]
    pub(crate) fn renderer_for_test(&self) -> &RtEngine { &self.rt }
    #[cfg(test)]
    pub(crate) fn renderer_mut_for_test(&mut self) -> &mut RtEngine { &mut self.rt }

    pub(crate) fn new(mut rt: RtEngine, channels: usize) -> Self {
        rt.audible.restart();
        Self {
            rt: GraphLease {
                graph: Some(Box::new(rt)),
                returned: None,
            },
            enabled: None,
            stopped: None,
            channels,
            buffer: vec![0.0; 32768 * channels.max(1)],
            resume_ramp: None,
            #[cfg(test)]
            conversion_delay: std::time::Duration::ZERO,
            #[cfg(test)]
            stall_once: None,
        }
    }

    fn managed(
        mut rt: Box<RtEngine>,
        channels: usize,
        returned: crossbeam_channel::Sender<Box<RtEngine>>,
        enabled: Arc<AtomicBool>,
        stopped: Arc<AtomicBool>,
    ) -> Self {
        rt.audible.restart();
        Self {
            rt: GraphLease {
                graph: Some(rt),
                returned: Some(returned),
            },
            enabled: Some(enabled),
            stopped: Some(stopped),
            channels,
            buffer: vec![0.0; 32768 * channels.max(1)],
            resume_ramp: Some((0, 0, 1)),
            #[cfg(test)]
            conversion_delay: std::time::Duration::ZERO,
            #[cfg(test)]
            stall_once: None,
        }
    }

    pub(crate) fn render<T>(&mut self, data: &mut [T])
    where
        T: cpal::SizedSample + cpal::FromSample<f32>,
    f64: cpal::FromSample<T>,
    {
        self.render_timed(data, None);
    }

    pub(crate) fn render_timed<T>(&mut self, data: &mut [T], latency: Option<std::time::Duration>)
    where
        T: cpal::SizedSample + cpal::FromSample<f32>,
    f64: cpal::FromSample<T>,
    {
        let playback_ns = latency.and_then(|duration| {
            self.rt
                .audible
                .now_ns()
                .checked_add(super::audio_metrics::nanoseconds(duration))
        });
        self.render_at(data, latency, playback_ns);
    }

    /// Render a block against its scheduled first output frame.
    /// Takes destination samples, reported delay and absolute playback time; returns after bounded conversion and timing publication.
    fn render_at<T>(
        &mut self,
        data: &mut [T],
        latency: Option<std::time::Duration>,
        playback_ns: Option<u64>,
    ) where
        T: cpal::SizedSample + cpal::FromSample<f32>,
        f64: cpal::FromSample<T>,
    {
        if self
            .enabled
            .as_ref()
            .is_some_and(|enabled| !enabled.load(Ordering::Acquire))
            || self
                .stopped
                .as_ref()
                .is_some_and(|stopped| stopped.load(Ordering::Acquire))
        {
            self.rt.audible.restart();
            for sample in data {
                *sample = T::from_sample(0.0);
            }
            return;
        }
        let started = std::time::Instant::now();
        if data.len() > self.buffer.len() {
            let width = self.buffer.len();
            let mut offset = 0u64;
            for block in data.chunks_mut(width) {
                let start = playback_ns.and_then(|ns| {
                    ns.checked_add(
                        offset.saturating_mul(1_000_000_000) / (self.rt.sr as u64).max(1),
                    )
                });
                self.render_at(block, latency, start);
                offset += (block.len() / self.channels.max(1)) as u64;
            }
            return;
        }
        let slice = &mut self.buffer[..data.len()];
        slice.fill(0.0);
        if let Some(history) = &mut self.rt.history_measurement { history.begin_output(data.len() / self.channels.max(1)); }
        let sample_rate = self.rt.sr as u32;
        self.rt.audible.begin(sample_rate, playback_ns);
        self.rt.process_interleaved(slice, self.channels);
        if let Some((was_playing, remaining, total)) = &mut self.resume_ramp {
            let playing = u8::from(self.rt.playing)
                | self
                    .rt
                    .decks
                    .iter()
                    .enumerate()
                    .fold(0, |bits, (index, deck)| {
                        bits | (u8::from(deck.playing) << (index + 1))
                    });
            if playing != 0 && *was_playing == 0 && *total == 1 {
                *total = (self.rt.sr as u32).saturating_mul(2).div_ceil(1000).max(1);
                *remaining = *total;
            }
            *was_playing = playing;
            for frame in slice.chunks_mut(self.channels.max(1)) {
                if *remaining == 0 { break; }
                let gain = 1.0 - *remaining as f32 / *total as f32;
                for sample in frame { *sample *= gain; }
                *remaining -= 1;
            }
        }
        if self.resume_ramp.is_some_and(|(playing, remaining, total)| playing != 0 && remaining == 0 && total > 1) {
            self.resume_ramp = None;
        }
        #[cfg(test)]
        std::thread::sleep(self.conversion_delay);
        #[cfg(test)]
        if let Some((remaining, delay)) = &mut self.stall_once {
            if *remaining == 0 {
                std::thread::sleep(*delay);
                self.stall_once = None;
            } else {
                *remaining -= 1;
            }
        }
        for (destination, source) in data.iter_mut().zip(slice) {
            *destination = T::from_sample(*source);
        }
        self.rt.routing_pipe.meters(data, self.channels);
        if let Some(history) = &mut self.rt.history_measurement { history.converted(data, self.channels); }
        self.rt.audible.finish();
        self.rt.telemetry.record_output(
            started.elapsed(),
            data.len() / self.channels.max(1),
            self.rt.sr as u32,
            self.rt.render_cpu_ns,
            self.channels as u16,
            latency,
        );
    }
}

#[cfg(test)]
#[path = "audio_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "audio/audible_tests.rs"]
mod audible_tests;
#[cfg(test)]
#[path = "audio_live_set_tests.rs"]
mod live_set_tests;
#[cfg(test)]
#[path = "audio_metrics_tests.rs"]
mod metrics_tests;
