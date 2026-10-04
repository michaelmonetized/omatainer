//! JACK and PipeWire's JACK library share an explicit, server-clocked graph path.
use super::{config, graph, routing::input, OutputCallback};
use jack_sys as j;
use std::{
    cell::Cell,
    ffi::{CStr, CString},
    sync::{
        atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

pub(crate) const BACKEND: &str = "JACK";
pub(crate) const DEVICE: &str = "JACK graph";
pub(crate) const MAX_FRAMES: usize = 32768;
static CLIENT_LIFECYCLE: Mutex<()> = Mutex::new(());
const AUDIO: &[u8] = b"32 bit float mono audio\0";

struct Client(*mut j::jack_client_t);
impl Client {
    /// Open an existing graph server.
    /// Takes an exact client name; returns an inactive client without starting a server or renaming duplicates.
    fn open(name: &str) -> Result<Self, String> {
        let _lifecycle = CLIENT_LIFECYCLE
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        j::library().map_err(|error| format!("JACK client library unavailable: {error}"))?;
        let name = CString::new(name).map_err(|_| "Invalid graph client name")?;
        let mut status = 0;
        let raw = unsafe {
            j::jack_client_open(
                name.as_ptr(),
                j::JackNoStartServer | j::JackUseExactName,
                &mut status,
            )
        };
        if raw.is_null() {
            Err(format!("Existing JACK graph unavailable or exact client name already in use (status {status:#x})"))
        } else {
            Ok(Self(raw))
        }
    }
    fn rate(&self) -> u32 {
        unsafe { j::jack_get_sample_rate(self.0) as u32 }
    }
    fn quantum(&self) -> u32 {
        unsafe { j::jack_get_buffer_size(self.0) }
    }
    /// Read bounded graph names.
    /// Takes direction flags; returns exact audio port names or a refused excessive inventory.
    fn ports(&self, flags: u32) -> Result<Vec<String>, String> {
        let raw = unsafe {
            j::jack_get_ports(
                self.0,
                std::ptr::null(),
                AUDIO.as_ptr().cast(),
                flags.into(),
            )
        };
        unsafe { names(raw) }
    }
    fn port(&self, name: &str) -> Result<*mut j::jack_port_t, String> {
        let name = CString::new(name).map_err(|_| "Invalid graph port name")?;
        let port = unsafe { j::jack_port_by_name(self.0, name.as_ptr()) };
        if port.is_null() {
            Err("Exact graph endpoint unavailable".into())
        } else {
            Ok(port)
        }
    }
    /// Check all actual and saved connections before adding any link.
    /// Takes accepted profile routes; returns refusal for feedback or inconsistent endpoint directions.
    fn validate_routes(&self, routes: &graph::Routes) -> Result<(), String> {
        routes.validate()?;
        let mut edges = Vec::new();
        for source in self.ports(j::JackPortIsOutput)? {
            let Ok(port) = self.port(&source) else {
                continue;
            };
            let connected = unsafe { names(j::jack_port_get_all_connections(self.0, port)) }?;
            for destination in connected {
                if let (Ok(source), Ok(destination)) =
                    (self.graph_node(&source), self.graph_node(&destination))
                {
                    edges.push((source, destination));
                }
                if edges.len() > 16384 {
                    return Err("Graph connection inventory exceeds the review limit".into());
                }
            }
        }
        for (links, direction) in [
            (&routes.outputs, j::JackPortIsInput),
            (&routes.inputs, j::JackPortIsOutput),
        ] {
            for link in links {
                if let Ok(port) = self.port(&link.endpoint) {
                    if unsafe { j::jack_port_flags(port) } as u32 & direction == 0
                        || unsafe { CStr::from_ptr(j::jack_port_type(port)) }.to_bytes()
                            != &AUDIO[..AUDIO.len() - 1]
                    {
                        return Err(
                            "Saved endpoint is not an audio port in the required direction".into(),
                        );
                    }
                }
                let external = match self.graph_node(&link.endpoint) {
                    Ok(node) => node,
                    Err(_) => continue,
                };
                edges.push(if direction == j::JackPortIsInput {
                    ("Omatainer".into(), external)
                } else {
                    (external, "OmatainerCapture".into())
                });
            }
        }
        graph::no_feedback(&edges)
    }
    /// Treat physical ports as terminal hardware endpoints.
    /// Takes an exact port name; returns its processing client or independent physical connector identity.
    fn graph_node(&self, name: &str) -> Result<String, String> {
        let port = self.port(name)?;
        let flags = unsafe { j::jack_port_flags(port) } as u32;
        if flags & j::JackPortIsPhysical != 0 && !graph::owned(client_name(name)) {
            Ok(format!("physical:{name}"))
        } else {
            Ok(client_name(name).into())
        }
    }
    /// Restore only accepted, currently present endpoints.
    /// Takes saved routes and direction; returns the linked count and whether exact links still await their endpoints.
    fn restore(&self, routes: &graph::Routes, input: bool) -> Result<(usize, bool), String> {
        self.validate_routes(routes)?;
        let mut count = 0;
        let mut pending = false;
        for link in if input {
            &routes.inputs
        } else {
            &routes.outputs
        } {
            let local = port_name(input, link.channel);
            let (source, destination) = if input {
                (&link.endpoint, &local)
            } else {
                (&local, &link.endpoint)
            };
            if self.port(source).is_err() || self.port(destination).is_err() {
                pending = true;
                continue;
            }
            let source_c = CString::new(source.as_str()).map_err(|_| "Invalid source")?;
            let dest_c = CString::new(destination.as_str()).map_err(|_| "Invalid destination")?;
            let connected =
                unsafe { j::jack_port_connected_to(self.port(source)?, dest_c.as_ptr()) } != 0;
            if !connected {
                let result = unsafe { j::jack_connect(self.0, source_c.as_ptr(), dest_c.as_ptr()) };
                if result != 0 && result != libc::EEXIST {
                    pending = true;
                    continue;
                }
            }
            count += 1;
        }
        Ok((count, pending))
    }
}
impl Drop for Client {
    fn drop(&mut self) {
        let _lifecycle = CLIENT_LIFECYCLE
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        unsafe {
            j::jack_client_close(self.0);
        }
    }
}

/// Copy graph-owned string arrays off the callback.
/// Takes a JACK allocated null-terminated array; returns at most 4096 owned names and always frees the array.
unsafe fn names(raw: *mut *const libc::c_char) -> Result<Vec<String>, String> {
    if raw.is_null() {
        return Ok(Vec::new());
    }
    let result = (|| {
        let mut values = Vec::new();
        for index in 0..=4096 {
            let value = unsafe { *raw.add(index) };
            if value.is_null() {
                return Ok(values);
            }
            if index == 4096 {
                return Err("Graph port inventory exceeds 4096 names".into());
            }
            let value = unsafe { CStr::from_ptr(value) }
                .to_str()
                .map_err(|_| "Graph port name is not UTF-8")?;
            if value.len() > 255 || value.chars().any(char::is_control) {
                return Err("Graph port name cannot be retained safely".into());
            }
            values.push(value.to_owned());
        }
        unreachable!()
    })();
    unsafe {
        j::jack_free(raw.cast());
    }
    result
}
fn client_name(port: &str) -> &str {
    port.split_once(':').map_or(port, |(name, _)| name)
}
pub(crate) fn port_name(input: bool, channel: u16) -> String {
    format!(
        "{}:{}_{:02}",
        if input {
            "OmatainerCapture"
        } else {
            "Omatainer"
        },
        if input { "input" } else { "output" },
        channel + 1
    )
}

/// Discover the currently loaded graph server without playing audio.
/// Takes no arguments; returns server-owned clock capabilities and exact external endpoints.
pub(crate) fn discover() -> Result<config::Inventory, String> {
    let client = Client::open("OmatainerDiscovery")?;
    let _control = CLIENT_LIFECYCLE
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let rate = client.rate();
    let quantum = client.quantum();
    if !(8000..=384000).contains(&rate) || !(16..=MAX_FRAMES as u32).contains(&quantum) {
        return Err("Graph clock is outside the supported rate/quantum bounds".into());
    }
    let device = config::Device {
        name: DEVICE.into(),
        default: true,
        defaults: Some((2, rate, cpal::SampleFormat::F32)),
        error: None,
        ranges: (1..=64)
            .map(|channels| config::Range {
                channels,
                min_rate: rate,
                max_rate: rate,
                format: cpal::SampleFormat::F32,
                buffer: Some((quantum, quantum)),
            })
            .collect(),
    };
    let mut graph_ports = Vec::new();
    for (flags, input) in [(j::JackPortIsInput, true), (j::JackPortIsOutput, false)] {
        graph_ports.extend(
            client
                .ports(flags)?
                .into_iter()
                .filter(|port| !graph::owned(client_name(port)))
                .map(|port| (port, input)),
        );
    }
    Ok(config::Inventory {
        backend: BACKEND.into(),
        devices: vec![device.clone()],
        inputs: vec![device],
        input_error: None,
        truncated: false,
        graph_ports,
    })
}

/// Plan an exact server-clocked stream.
/// Takes saved settings; returns the observed clock and saved links without modifying server settings.
pub(crate) fn select(settings: &crate::preferences::Audio) -> Result<config::Plan, String> {
    let inventory = discover()?;
    let mut plan = config::plan(settings, &inventory)?;
    plan.buffer = inventory.devices[0].ranges[0]
        .buffer
        .map(|(quantum, _)| quantum);
    plan.warning = Some("The graph server owns rate and quantum. No system connections are automatic; missing saved endpoints stay disconnected.".into());
    if plan
        .graph
        .outputs
        .iter()
        .any(|link| link.channel >= plan.channels)
    {
        return Err("Saved graph output exceeds the selected channel count".into());
    }
    Ok(plan)
}

struct Signals {
    #[cfg(test)]
    trace: Arc<native_tests::Trace>,
    fault: Arc<AtomicBool>,
    dirty: AtomicBool,
    rate: u32,
    quantum: AtomicU32,
    xruns: AtomicU64,
    latency_ns: AtomicU64,
}
struct Process {
    ports: Vec<*mut j::jack_port_t>,
    buffer: Vec<f32>,
    output: Option<OutputCallback>,
    input: Option<(input::Pipe, u64)>,
    signals: Arc<Signals>,
}
pub(crate) struct Stream {
    latency_ports: Vec<*mut j::jack_port_t>,
    client: Option<Client>,
    process: Option<Box<Process>>,
    signals: Arc<Signals>,
    routes: graph::Routes,
    input: bool,
    retry_at: Cell<Option<Instant>>,
}
impl Stream {
    /// Open and activate one exact graph client.
    /// Takes a reviewed plan and prepared callback or capture pipe; returns a stream whose owner closes the client before returning callback state.
    fn open(
        plan: &config::Plan,
        output: Option<OutputCallback>,
        capture: Option<(input::Pipe, u64)>,
        fault: Arc<AtomicBool>,
    ) -> Result<Self, String> {
        let input = capture.is_some();
        if plan.backend != BACKEND
            || plan.device != DEVICE
            || plan.format != cpal::SampleFormat::F32
            || !(1..=64).contains(&plan.channels)
        {
            return Err("Invalid native graph stream plan".into());
        }
        if (if input {
            &plan.graph.inputs
        } else {
            &plan.graph.outputs
        })
        .iter()
        .any(|link| link.channel >= plan.channels)
        {
            return Err("Saved graph link exceeds the selected channel count".into());
        }
        let client = Client::open(if input {
            "OmatainerCapture"
        } else {
            "Omatainer"
        })?;
        if client.rate() != plan.rate
            || plan
                .buffer
                .is_some_and(|quantum| quantum != client.quantum())
        {
            return Err("Graph clock changed since preview; preview again".into());
        }
        {
            let _control = CLIENT_LIFECYCLE
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            client.validate_routes(&plan.graph)?;
        }
        let signals = Arc::new(Signals {
            #[cfg(test)]
            trace: native_tests::new_trace(input),
            fault,
            dirty: AtomicBool::new(true),
            rate: plan.rate,
            quantum: AtomicU32::new(client.quantum()),
            xruns: AtomicU64::new(0),
            latency_ns: AtomicU64::new(u64::MAX),
        });
        let mut ports = Vec::with_capacity(plan.channels as usize);
        {
            let _control = CLIENT_LIFECYCLE
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            for channel in 0..plan.channels {
                let name = CString::new(format!(
                    "{}_{:02}",
                    if input { "input" } else { "output" },
                    channel + 1
                ))
                .unwrap();
                let port = unsafe {
                    j::jack_port_register(
                        client.0,
                        name.as_ptr(),
                        AUDIO.as_ptr().cast(),
                        if input {
                            j::JackPortIsInput
                        } else {
                            j::JackPortIsOutput
                        }
                        .into(),
                        0,
                    )
                };
                if port.is_null() {
                    for port in &ports {
                        unsafe {
                            j::jack_port_unregister(client.0, *port);
                        }
                    }
                    return Err("Graph port registration failed; partial client closed".into());
                }
                ports.push(port);
            }
        }
        let mut stream = Self {
            latency_ports: ports.clone(),
            client: Some(client),
            process: Some(Box::new(Process {
                ports,
                buffer: vec![0.0; MAX_FRAMES * usize::from(plan.channels)],
                output,
                input: capture,
                signals: signals.clone(),
            })),
            signals,
            routes: plan.graph.clone(),
            input,
            retry_at: Cell::new(None),
        };
        let raw = stream.client.as_ref().unwrap().0;
        let process_arg = (&mut **stream.process.as_mut().unwrap() as *mut Process).cast();
        let signal_arg = Arc::as_ptr(&stream.signals).cast_mut().cast();
        let registered = {
            let _control = CLIENT_LIFECYCLE
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            unsafe {
                j::jack_set_process_callback(raw, Some(process), process_arg) == 0
                    && j::jack_set_buffer_size_callback(raw, Some(buffer_size), signal_arg) == 0
                    && j::jack_set_sample_rate_callback(raw, Some(sample_rate), signal_arg) == 0
                    && j::jack_set_graph_order_callback(raw, Some(graph_order), signal_arg) == 0
                    && j::jack_set_port_registration_callback(
                        raw,
                        Some(port_registered),
                        signal_arg,
                    ) == 0
                    && j::jack_set_port_connect_callback(raw, Some(port_connected), signal_arg) == 0
                    && j::jack_set_xrun_callback(raw, Some(xrun), signal_arg) == 0
            }
        };
        if !registered {
            return Err("Graph callback registration failed".into());
        }
        unsafe {
            j::jack_on_shutdown(raw, Some(shutdown), signal_arg);
        }
        let activation = {
            let _lifecycle = CLIENT_LIFECYCLE
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            unsafe { j::jack_activate(raw) }
        };
        if activation != 0 {
            return Err("Graph activation failed; client and callback reclaimed".into());
        }
        stream.maintain()?;
        Ok(stream)
    }
    /// Restore accepted graph links on the owner thread.
    /// Takes the live stream; returns current quantum or a fault without allocating on its callback.
    pub(crate) fn maintain(&self) -> Result<u32, String> {
        let _control = CLIENT_LIFECYCLE
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if self.signals.fault.load(Ordering::Acquire) {
            return Err(
                "Graph clock changed or server stopped; session retained for explicit recovery"
                    .into(),
            );
        }
        if self.signals.dirty.swap(false, Ordering::AcqRel)
            || self
                .retry_at
                .get()
                .is_some_and(|when| Instant::now() >= when)
        {
            match self
                .client
                .as_ref()
                .unwrap()
                .restore(&self.routes, self.input)
            {
                Ok((_, pending)) => self
                    .retry_at
                    .set(pending.then(|| Instant::now() + Duration::from_millis(250))),
                Err(error) => {
                    self.signals.fault.store(true, Ordering::Release);
                    return Err(error);
                }
            }
        }
        if !self.input {
            let mut latency = None;
            let mut consistent = true;
            for port in &self.latency_ports {
                if unsafe { j::jack_port_connected(*port) } == 0 { continue; }
                let mut range = j::jack_latency_range_t { min: 0, max: 0 };
                unsafe { j::jack_port_get_latency_range(*port, j::JackPlaybackLatency, &mut range); }
                if range.min != range.max || latency.is_some_and(|frames| frames != range.max) { consistent = false; break; }
                latency = Some(range.max);
            }
            let ns = latency.filter(|_| consistent).map(|frames| u64::from(frames) * 1_000_000_000 / u64::from(self.signals.rate));
            self.signals.latency_ns.store(ns.unwrap_or(u64::MAX), Ordering::Release);
        }
        Ok(self.signals.quantum.load(Ordering::Acquire))
    }
}
impl Drop for Stream {
    fn drop(&mut self) {
        if let Some(client) = self.client.take() {
            {
                let _lifecycle = CLIENT_LIFECYCLE
                    .lock()
                    .unwrap_or_else(|error| error.into_inner());
                unsafe {
                    j::jack_deactivate(client.0);
                    if let Some(process) = &self.process {
                        for port in &process.ports {
                            j::jack_port_unregister(client.0, *port);
                        }
                    }
                    j::jack_set_process_callback(client.0, None, std::ptr::null_mut());
                    j::jack_on_shutdown(client.0, None, std::ptr::null_mut());
                    j::jack_client_close(client.0);
                }
            };
            std::mem::forget(client);
        }
    }
}

pub(crate) fn output(
    plan: &config::Plan,
    callback: OutputCallback,
    fault: Arc<AtomicBool>,
) -> Result<Stream, String> {
    Stream::open(plan, Some(callback), None, fault)
}
pub(crate) fn capture(
    plan: &config::Plan,
    pipe: input::Pipe,
    generation: u64,
    fault: Arc<AtomicBool>,
) -> Result<Stream, String> {
    Stream::open(plan, None, Some((pipe, generation)), fault)
}

/// Render or capture the current planar block.
/// Takes backend-owned frame count and one pinned process context; returns continuation after bounded, allocation-free work.
unsafe extern "C" fn process(frames: u32, arg: *mut libc::c_void) -> libc::c_int {
    #[cfg(test)]
    {
        let trace = unsafe { &*arg.cast::<Process>() }.signals.trace.clone();
        let mut result = 0;
        let counts = crate::engine::test_alloc::measure(|| {
            result = unsafe { process_block(frames, arg) };
        });
        trace.callbacks.fetch_add(1, Ordering::Relaxed);
        trace
            .allocations
            .fetch_add(counts.allocations as u64, Ordering::Relaxed);
        trace
            .frees
            .fetch_add(counts.frees as u64, Ordering::Relaxed);
        return result;
    }
    #[cfg(not(test))]
    unsafe {
        process_block(frames, arg)
    }
}
unsafe fn process_block(frames: u32, arg: *mut libc::c_void) -> libc::c_int {
    let process = unsafe { &mut *arg.cast::<Process>() };
    let width = process.ports.len();
    let frames = frames as usize;
    if frames > MAX_FRAMES {
        process.signals.fault.store(true, Ordering::Release);
        return 1;
    }
    if process.signals.fault.load(Ordering::Acquire) {
        process.signals.fault.store(true, Ordering::Release);
        if process.output.is_some() {
            for port in &process.ports {
                let raw = unsafe { j::jack_port_get_buffer(*port, frames as u32) }.cast::<f32>();
                if !raw.is_null() {
                    unsafe { std::slice::from_raw_parts_mut(raw, frames) }.fill(0.0);
                }
            }
        }
        return 0;
    }
    let data = &mut process.buffer[..frames * width];
    if let Some(output) = &mut process.output {
        let latency = process.signals.latency_ns.load(Ordering::Acquire);
        output.render_timed(data, (latency != u64::MAX).then(|| Duration::from_nanos(latency)));
        #[cfg(test)]
        native_tests::observe(&process.signals.trace, data, width, process.signals.rate);
        for (channel, port) in process.ports.iter().enumerate() {
            let raw = unsafe { j::jack_port_get_buffer(*port, frames as u32) }.cast::<f32>();
            if raw.is_null() {
                process.signals.fault.store(true, Ordering::Release);
                continue;
            }
            for (frame, value) in unsafe { std::slice::from_raw_parts_mut(raw, frames) }
                .iter_mut()
                .enumerate()
            {
                *value = data[frame * width + channel];
            }
        }
    } else if let Some((pipe, generation)) = &process.input {
        for (channel, port) in process.ports.iter().enumerate() {
            let raw = unsafe { j::jack_port_get_buffer(*port, frames as u32) }.cast::<f32>();
            if raw.is_null() {
                process.signals.fault.store(true, Ordering::Release);
                return 0;
            }
            for (frame, value) in unsafe { std::slice::from_raw_parts(raw, frames) }
                .iter()
                .enumerate()
            {
                data[frame * width + channel] = *value;
            }
        }
        #[cfg(test)]
        native_tests::observe(&process.signals.trace, data, width, process.signals.rate);
        pipe.capture(data, width, *generation);
    }
    0
}
unsafe extern "C" fn buffer_size(frames: u32, arg: *mut libc::c_void) -> libc::c_int {
    let signals = unsafe { &*arg.cast::<Signals>() };
    signals.quantum.store(frames, Ordering::Release);
    if !(16..=MAX_FRAMES as u32).contains(&frames) {
        signals.fault.store(true, Ordering::Release);
    }
    0
}
unsafe extern "C" fn sample_rate(rate: u32, arg: *mut libc::c_void) -> libc::c_int {
    let signals = unsafe { &*arg.cast::<Signals>() };
    if rate != signals.rate {
        signals.fault.store(true, Ordering::Release);
    }
    0
}
unsafe extern "C" fn shutdown(arg: *mut libc::c_void) {
    unsafe { &*arg.cast::<Signals>() }
        .fault
        .store(true, Ordering::Release);
}
unsafe extern "C" fn graph_order(arg: *mut libc::c_void) -> libc::c_int {
    unsafe { &*arg.cast::<Signals>() }
        .dirty
        .store(true, Ordering::Release);
    0
}
unsafe extern "C" fn port_registered(_: u32, _: libc::c_int, arg: *mut libc::c_void) {
    unsafe {
        graph_order(arg);
    }
}
unsafe extern "C" fn port_connected(_: u32, _: u32, _: libc::c_int, arg: *mut libc::c_void) {
    unsafe {
        graph_order(arg);
    }
}
unsafe extern "C" fn xrun(arg: *mut libc::c_void) -> libc::c_int {
    unsafe { &*arg.cast::<Signals>() }
        .xruns
        .fetch_add(1, Ordering::Relaxed);
    0
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn quantum_rate_and_shutdown_notifications_never_allocate() {
        let signals = Signals {
            #[cfg(test)]
            trace: Arc::new(native_tests::Trace::default()),
            fault: Arc::new(AtomicBool::new(false)),
            dirty: AtomicBool::new(false),
            rate: 48000,
            quantum: AtomicU32::new(128),
            xruns: AtomicU64::new(0),
            latency_ns: AtomicU64::new(u64::MAX),
        };
        let arg = (&signals as *const Signals).cast_mut().cast();
        let counts = crate::engine::test_alloc::measure(|| unsafe {
            buffer_size(256, arg);
            sample_rate(48000, arg);
            graph_order(arg);
            xrun(arg);
        });
        assert_eq!(counts, crate::engine::test_alloc::Counts::default());
        assert!(!signals.fault.load(Ordering::Acquire));
        assert_eq!(signals.quantum.load(Ordering::Acquire), 256);
        assert_eq!(
            crate::engine::test_alloc::measure(|| unsafe {
                sample_rate(44100, arg);
                shutdown(arg);
                buffer_size(65536, arg);
            }),
            crate::engine::test_alloc::Counts::default()
        );
        assert!(signals.fault.load(Ordering::Acquire));
    }
}

/// Identify the selected graph server and client library.
/// Takes no arguments; returns an exact retained namespace without identifying physical hardware.
pub(crate) fn identity() -> Option<String> {
    j::library().ok()?;
    let library = j::library().ok()?;
    let version_fn = unsafe {
        library.get::<unsafe extern "C" fn() -> *const libc::c_char>(b"jack_get_version_string\0")
    }
    .ok()?;
    let version = unsafe { CStr::from_ptr(version_fn()) }.to_str().ok()?;
    Some(format!(
        "jack-graph-v1:{}:{}:{}",
        version,
        std::env::var("JACK_DEFAULT_SERVER").unwrap_or_else(|_| "default".into()),
        std::env::var("PIPEWIRE_REMOTE").unwrap_or_else(|_| "pipewire-0".into())
    ))
}

#[cfg(test)]
pub(crate) mod native_tests;
