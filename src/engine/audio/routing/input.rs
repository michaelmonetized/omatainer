//! A bounded native input pipe never waits, allocates or opens devices in the renderer.
use super::model::{InputConfig, MAX_PHYSICAL_CHANNELS};
use crate::engine::audio::{config, owner, recovery};
use arc_swap::ArcSwap;
use cpal::traits::{DeviceTrait, StreamTrait};
use crossbeam_channel::{bounded, Receiver, Sender};
use std::sync::{
    atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
    Arc,
};
use std::time::Duration;

const CAPACITY: usize = 8192;
#[derive(Clone, Copy)]
struct Sample {
    generation: u64,
    frame: [f32; MAX_PHYSICAL_CHANNELS],
}
pub(crate) struct Shared {
    pub(crate) explicit: AtomicBool,
    pub(crate) probe: AtomicU32,
    generation: AtomicU64,
    rate: AtomicU32,
    enabled: AtomicBool,
    pub(crate) fault: AtomicBool,
    pub(crate) captured: AtomicU64,
    pub(crate) overflow: AtomicU64,
    pub(crate) underrun: AtomicU64,
    pub(crate) input: [AtomicU32; MAX_PHYSICAL_CHANNELS],
    pub(crate) output: [AtomicU32; MAX_PHYSICAL_CHANNELS],
}
#[derive(Clone)]
pub(crate) struct Pipe {
    pub(crate) recorder: super::record::Recorder,
    sender: Sender<Sample>,
    receiver: Receiver<Sample>,
    pub(crate) shared: Arc<Shared>,
}
impl Default for Pipe {
    fn default() -> Self {
        let (sender, receiver) = bounded(CAPACITY);
        Self {
            sender,
            receiver,
            recorder: super::record::Recorder::default(),
            shared: Arc::new(Shared {
                explicit: AtomicBool::new(false),
                probe: AtomicU32::new(0),
                generation: AtomicU64::new(0),
                rate: AtomicU32::new(0),
                enabled: AtomicBool::new(false),
                fault: AtomicBool::new(false),
                captured: AtomicU64::new(0),
                overflow: AtomicU64::new(0),
                underrun: AtomicU64::new(0),
                input: std::array::from_fn(|_| AtomicU32::new(0)),
                output: std::array::from_fn(|_| AtomicU32::new(0)),
            }),
        }
    }
}
impl Pipe {
    /// Receive one nominal-rate input frame.
    /// Takes the active output rate; returns current-generation channels or silence, without waiting.
    pub(crate) fn frame(&self, rate: u32) -> [f32; MAX_PHYSICAL_CHANNELS] {
        if !self.shared.enabled.load(Ordering::Acquire)
            || self.shared.fault.load(Ordering::Acquire)
            || self.shared.rate.load(Ordering::Acquire) != rate
        {
            return [0.0; MAX_PHYSICAL_CHANNELS];
        }
        let generation = self.shared.generation.load(Ordering::Acquire);
        match self.receiver.try_recv() {
            Ok(sample) if sample.generation == generation => sample.frame,
            _ => {
                self.shared.underrun.fetch_add(1, Ordering::Relaxed);
                [0.0; MAX_PHYSICAL_CHANNELS]
            }
        }
    }
    /// Publish actual converted output levels.
    /// Takes normalized interleaved samples and width; updates independent physical meters.
    pub(crate) fn meters<T>(&self, data: &[T], channels: usize)
    where
        T: cpal::SizedSample,
        f64: cpal::FromSample<T>,
    {
        let mut peaks = [0.0_f32; MAX_PHYSICAL_CHANNELS];
        for frame in data.chunks_exact(channels.max(1)) {
            for (peak, value) in peaks.iter_mut().zip(frame) {
                *peak = peak.max(value.to_sample::<f64>().abs() as f32);
            }
        }
        for (meter, peak) in self.shared.output.iter().zip(peaks) {
            decay(meter, peak);
        }
    }
    fn stop(&self) {
        self.shared.enabled.store(false, Ordering::Release);
        self.shared.generation.fetch_add(1, Ordering::AcqRel);
        while self.receiver.try_recv().is_ok() {}
        for meter in &self.shared.input {
            meter.store(0, Ordering::Relaxed);
        }
    }
    pub(crate) fn capture<T>(&self, data: &[T], channels: usize, generation: u64)
    where
        T: cpal::SizedSample,
        f32: cpal::FromSample<T>,
    {
        if !self.shared.enabled.load(Ordering::Acquire)
            || generation != self.shared.generation.load(Ordering::Acquire)
        {
            return;
        }
        for source in data.chunks_exact(channels) {
            let mut frame = [0.0; MAX_PHYSICAL_CHANNELS];
            for (index, (target, value)) in frame.iter_mut().zip(source).enumerate() {
                let value = value.to_sample::<f32>();
                if !value.is_finite() {
                    self.shared.fault.store(true, Ordering::Release);
                    return;
                }
                *target = value;
                decay(&self.shared.input[index], value.abs());
            }
            if self.sender.try_send(Sample { generation, frame }).is_err() {
                self.shared.overflow.fetch_add(1, Ordering::Relaxed);
            }
            self.shared.captured.fetch_add(1, Ordering::Relaxed);
        }
    }
}

/// Update a bounded physical-channel peak.
/// Takes an atomic meter and current magnitude; retains the larger decayed or current value.
fn decay(meter: &AtomicU32, peak: f32) {
    let old = f32::from_bits(meter.load(Ordering::Relaxed));
    meter.store((old * 0.999).max(peak).to_bits(), Ordering::Relaxed);
}

#[derive(Clone, Debug)]
pub(crate) struct Status {
    pub generation: u64,
    pub active: Option<config::Plan>,
    pub message: String,
}
struct Request {
    plan: Option<config::Plan>,
    output_generation: u64,
    cancel: Arc<AtomicBool>,
    result: Sender<Result<Arc<Status>, String>>,
}
#[derive(Clone)]
pub(crate) struct Handle {
    requests: Sender<Request>,
    status: Arc<ArcSwap<Status>>,
    pipe: Pipe,
    output: owner::Handle,
}
impl Handle {
    /// Read immutable input status.
    /// Takes this handle; returns the latest accepted input route and notice.
    pub(crate) fn status(&self) -> Arc<Status> {
        self.status.load_full()
    }
    pub(crate) fn shared(&self) -> &Arc<Shared> {
        &self.pipe.shared
    }
    /// Confirm a reviewed input plan.
    /// Takes its exact plan, output generation and cancellation; returns committed status after an exclusive stopped boundary.
    pub(crate) fn apply(
        &self,
        plan: Option<config::Plan>,
        output_generation: u64,
        cancel: Arc<AtomicBool>,
    ) -> Result<Arc<Status>, String> {
        let permit = self.output.performance_permit()?;
        let _seal = self
            .output
            .project_handle()
            .seal_for_audio_permitted(permit, &cancel)
            .map_err(|error| error.to_string())?;
        if cancel.load(Ordering::Acquire) {
            return Err("Input change cancelled".into());
        }
        let (result, receipt) = bounded(1);
        self.requests
            .try_send(Request {
                plan,
                output_generation,
                cancel,
                result,
            })
            .map_err(|_| "An input operation is still pending".to_string())?;
        receipt
            .recv()
            .map_err(|_| "Input owner closed".to_string())?
    }
}

/// Preview an exact nominal-rate input.
/// Takes saved input choices, fresh discovery and the active output; returns a matching advertised plan or refusal.
pub(crate) fn preview(
    saved: &InputConfig,
    inventory: &config::Inventory,
    output: &config::Plan,
) -> Result<config::Plan, String> {
    if saved.backend != output.backend || inventory.backend != output.backend || inventory.truncated
    {
        return Err("Input/output backend changed or discovery was incomplete".into());
    }
    if let Some(error) = &inventory.input_error {
        return Err(error.clone());
    }
    let settings = crate::preferences::Audio {
        backend: Some(saved.backend.clone()),
        device: Some(saved.device.clone()),
        channels: Some(saved.channels),
        sample_rate: Some(output.rate),
        format: Some(saved.format),
        buffer_frames: saved.buffer_frames,
        ..Default::default()
    };
    let inputs = config::Inventory {
        graph_ports: Vec::new(),
        backend: inventory.backend.clone(),
        devices: inventory.inputs.clone(),
        inputs: Vec::new(),
        input_error: None,
        truncated: false,
    };
    let mut plan = config::plan(&settings, &inputs).map_err(|error| error.replace("output", "input"))?;
    plan.graph = output.graph.clone();
    Ok(plan)
}

fn build<T>(
    device: &cpal::Device,
    plan: &config::Plan,
    pipe: Pipe,
    generation: u64,
) -> Result<cpal::Stream, String>
where
    T: cpal::SizedSample,
    f32: cpal::FromSample<T>,
{
    let channels = usize::from(plan.channels);
    let fault = pipe.shared.clone();
    device
        .build_input_stream(
            &plan.config(),
            move |data: &[T], _| pipe.capture(data, channels, generation),
            move |_| fault.fault.store(true, Ordering::Release),
            None,
        )
        .map_err(|error| error.to_string())
}
enum NativeInput {
    Alsa(cpal::Stream),
    #[cfg(target_os = "linux")]
    Jack(super::super::jack::Stream),
}
fn open(plan: &config::Plan, pipe: &Pipe, cancel: &AtomicBool) -> Result<NativeInput, String> {
    #[cfg(target_os = "linux")]
    if plan.backend == super::super::jack::BACKEND {
        pipe.stop(); pipe.shared.fault.store(false, Ordering::Release);
        let generation = pipe.shared.generation.load(Ordering::Acquire);
        let stream = super::super::jack::capture(plan, pipe.clone(), generation, Arc::new(AtomicBool::new(false)))?;
        if cancel.load(Ordering::Acquire) || pipe.shared.fault.load(Ordering::Acquire) { return Err("Graph input activation cancelled or faulted".into()); }
        pipe.shared.rate.store(plan.rate, Ordering::Release); pipe.shared.enabled.store(true, Ordering::Release);
        return Ok(NativeInput::Jack(stream));
    }
    if !(1..=MAX_PHYSICAL_CHANNELS).contains(&usize::from(plan.channels)) {
        return Err("Input supports one through 64 channels".into());
    }
    let identity = recovery::identity(&plan.device);
    let device = config::select_input_exact(plan).map_err(|error| error.to_string())?;
    pipe.stop();
    pipe.shared.fault.store(false, Ordering::Release);
    let generation = pipe.shared.generation.load(Ordering::Acquire);
    let stream = match plan.format {
        cpal::SampleFormat::F32 => build::<f32>(&device, plan, pipe.clone(), generation),
        cpal::SampleFormat::F64 => build::<f64>(&device, plan, pipe.clone(), generation),
        cpal::SampleFormat::I8 => build::<i8>(&device, plan, pipe.clone(), generation),
        cpal::SampleFormat::I16 => build::<i16>(&device, plan, pipe.clone(), generation),
        cpal::SampleFormat::I32 => build::<i32>(&device, plan, pipe.clone(), generation),
        cpal::SampleFormat::I64 => build::<i64>(&device, plan, pipe.clone(), generation),
        cpal::SampleFormat::U8 => build::<u8>(&device, plan, pipe.clone(), generation),
        cpal::SampleFormat::U16 => build::<u16>(&device, plan, pipe.clone(), generation),
        cpal::SampleFormat::U32 => build::<u32>(&device, plan, pipe.clone(), generation),
        cpal::SampleFormat::U64 => build::<u64>(&device, plan, pipe.clone(), generation),
        _ => Err("Unsupported input sample format".into()),
    }?;
    stream.play().map_err(|error| error.to_string())?;
    if cancel.load(Ordering::Acquire)
        || pipe.shared.fault.load(Ordering::Acquire)
        || identity
            .as_ref()
            .is_some_and(|key| recovery::identity(&plan.device).as_ref() != Some(key))
    {
        return Err("Input changed, faulted or operation was cancelled before activation".into());
    }
    pipe.shared.rate.store(plan.rate, Ordering::Release);
    pipe.shared.fault.store(false, Ordering::Release);
    pipe.shared.enabled.store(true, Ordering::Release);
    Ok(NativeInput::Alsa(stream))
}

/// Start the sole native input owner.
/// Takes a fixed pipe and output owner; returns a request handle without enumerating or opening any input.
pub(crate) fn start(pipe: Pipe, output: owner::Handle) -> std::io::Result<(Handle, std::thread::JoinHandle<()>)> {
    let status = Arc::new(ArcSwap::from_pointee(Status {
        generation: 0,
        active: None,
        message: "Input is off. Saved choices require explicit activation.".into(),
    }));
    let (requests, receiver) = bounded::<Request>(1);
    let handle = Handle {
        requests,
        status: status.clone(),
        pipe: pipe.clone(),
        output: output.clone(),
    };
    let worker = std::thread::Builder::new().name("omatainer-input-owner".into()).spawn(move || {
        let mut active: Option<(NativeInput, config::Plan, u64)> = None;
        let mut last_frame = (pipe.shared.captured.load(Ordering::Relaxed), std::time::Instant::now());
        loop {
            #[cfg(target_os = "linux")]
            if let Some((NativeInput::Jack(stream), plan, _)) = &mut active {
                match stream.maintain() {
                    Ok(quantum) if plan.buffer != Some(quantum) => {
                        plan.buffer = Some(quantum);
                        let mut current = (*status.load_full()).clone(); current.active = Some(plan.clone()); status.store(Arc::new(current));
                    }
                    Err(_) => pipe.shared.fault.store(true, Ordering::Release),
                    _ => {}
                }
            }
            let captured = pipe.shared.captured.load(Ordering::Relaxed);
            if captured != last_frame.0 { last_frame = (captured, std::time::Instant::now()); }
            let output_status = output.status();
            if active.as_ref().is_some_and(|(_, plan, generation)| pipe.shared.fault.load(Ordering::Acquire) || last_frame.1.elapsed() > Duration::from_secs(5)
                || output_status.phase != owner::Phase::Running || output_status.generation != *generation
                || output_status.active.as_ref().is_none_or(|value| value.plan.rate != plan.rate)) {
                pipe.recorder.invalidate();
                pipe.stop(); active = None;
                status.store(Arc::new(Status { generation: status.load().generation + 1, active: None, message: "Input stopped after an input fault or output change. Saved routes are retained; preview and enable input explicitly.".into() }));
            }
            match receiver.recv_timeout(Duration::from_millis(20)) {
                Ok(request) => {
                    let result = (|| -> Result<Arc<Status>, String> {
                        if request.cancel.load(Ordering::Acquire) { return Err("Input change cancelled".into()); }
                        let current = output.status();
                        if current.generation != request.output_generation || current.phase != owner::Phase::Running { return Err("Output changed since input preview; preview again".into()); }
                        if request.plan.as_ref().is_some_and(|plan| current.active.as_ref().is_none_or(|value| value.plan.backend != plan.backend || value.plan.rate != plan.rate)) { return Err("Input must use the active output backend and nominal rate".into()); }
                        pipe.stop(); active = None;
                        pipe.recorder.invalidate();
                        if let Some(plan) = request.plan {
                            let stream = open(&plan, &pipe, &request.cancel)?;
                            let after = output.status();
                            if after.generation != current.generation || request.cancel.load(Ordering::Acquire) { pipe.stop(); drop(stream); return Err("Output changed or input activation was cancelled".into()); }
                            active = Some((stream, plan, current.generation));
                            last_frame = (pipe.shared.captured.load(Ordering::Relaxed), std::time::Instant::now());
                        }
                        let value = Arc::new(Status { generation: status.load().generation + 1, active: active.as_ref().map(|(_, plan, _)| plan.clone()), message: if active.is_some() { "Input enabled at the output's nominal rate. Drift, missing frames and queue overflow are reported separately.".into() } else { "Input disabled; saved aliases and routes are retained.".into() } });
                        status.store(value.clone()); Ok(value)
                    })();
                    if let Err(error) = &result { status.store(Arc::new(Status { generation: status.load().generation + 1, active: active.as_ref().map(|(_, plan, _)| plan.clone()), message: error.clone() })); }
                    let _ = request.result.send(result);
                }
                Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
                Err(_) => {}
            }
            if output.closed() { break; }
        }
        pipe.stop(); drop(active);
    })?;
    Ok((handle, worker))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn input_pipe_bounds_generation_rate_conversion_and_overflow_without_allocation() {
        let pipe = Pipe::default();
        pipe.shared.rate.store(48000, Ordering::Release);
        pipe.shared.enabled.store(true, Ordering::Release);
        let sample = [16384_i16; 64];
        assert_eq!(
            crate::engine::test_alloc::measure(|| pipe.capture(&sample, 64, 0)),
            crate::engine::test_alloc::Counts::default()
        );
        assert_eq!(pipe.frame(44100), [0.0; 64]);
        assert_eq!(pipe.frame(48000), [0.5; 64]);
        pipe.capture(&[f32::NAN], 1, 0);
        assert!(pipe.shared.fault.load(Ordering::Acquire));
        for _ in 0..CAPACITY + 7 {
            pipe.capture(&[0.25], 1, 0);
        }
        assert_eq!(pipe.shared.overflow.load(Ordering::Relaxed), 7);
        pipe.stop();
        pipe.shared.enabled.store(true, Ordering::Release);
        pipe.capture(&[0.9], 1, 0);
        assert_eq!(pipe.frame(48000), [0.0; 64]);
    }
}
