use super::*;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Digest of application-owned identity, never of user paths, titles or media.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Id(pub [u8; 16]);
impl Id {
    pub fn digest(bytes: &[u8]) -> Self {
        let hash = Sha256::digest(bytes);
        Self(hash[..16].try_into().expect("fixed digest prefix"))
    }
    pub fn hex(self) -> String {
        self.0.iter().map(|b| format!("{b:02x}")).collect()
    }
    pub fn from_hex(text: &str) -> Option<Self> {
        if text.len() != 32
            || !text
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return None;
        }
        let mut bytes = [0; 16];
        for (i, byte) in bytes.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&text[i * 2..i * 2 + 2], 16).ok()?;
        }
        Some(Self(bytes))
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Exit {
    #[default]
    Running,
    Clean,
    ObservedRustPanic,
    Unclean,
    StartupFailed,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureClass {
    Invalid,
    NotFound,
    PermissionDenied,
    NoSpace,
    Busy,
    Cancelled,
    Unavailable,
    Io,
    Unknown,
}
impl FailureClass {
    pub fn from_io(error: &std::io::Error) -> Self {
        match error.kind() {
            std::io::ErrorKind::NotFound => Self::NotFound,
            std::io::ErrorKind::PermissionDenied => Self::PermissionDenied,
            std::io::ErrorKind::InvalidData | std::io::ErrorKind::InvalidInput => Self::Invalid,
            _ if error.raw_os_error() == Some(libc::ENOSPC) => Self::NoSpace,
            _ => Self::Io,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Code {
    Startup,
    SafeModeStartup,
    AudioOpenFailed,
    EngineStartupFailed,
    PreferencesReadFailed,
    IpcStartupFailed,
    GuiStartupFailed,
    AudioBackendFailed,
    AudioOutputOffline,
    AudioSwitchFailed,
    AudioRollbackFailed,
    MidiConnectionFailed,
    ParserRejected,
    ProjectReadFailed,
    ProjectWriteFailed,
    RecoveryWriteFailed,
    RecoveryReadFailed,
    ThemeReadFailed,
    PluginHostingUnavailable,
    SupportStorageFailed,
    CleanExit,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Event {
    pub elapsed_ms: u64,
    pub code: Code,
    pub failure: Option<FailureClass>,
    /// Number of additional identical consecutive observations, not lost events.
    pub repeats: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Backend {
    SystemDefault,
    Alsa,
    Other,
    Unavailable,
}
impl Backend {
    pub fn from_name(name: &str) -> Self {
        if name.eq_ignore_ascii_case("alsa") {
            Self::Alsa
        } else {
            Self::Other
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Format {
    I8,
    I16,
    I32,
    I64,
    U8,
    U16,
    U32,
    U64,
    F32,
    F64,
    Default,
}
impl Format {
    pub fn from_cpal(format: cpal::SampleFormat) -> Self {
        match format {
            cpal::SampleFormat::I8 => Self::I8,
            cpal::SampleFormat::I16 => Self::I16,
            cpal::SampleFormat::I32 => Self::I32,
            cpal::SampleFormat::I64 => Self::I64,
            cpal::SampleFormat::U8 => Self::U8,
            cpal::SampleFormat::U16 => Self::U16,
            cpal::SampleFormat::U32 => Self::U32,
            cpal::SampleFormat::U64 => Self::U64,
            cpal::SampleFormat::F32 => Self::F32,
            cpal::SampleFormat::F64 => Self::F64,
            _ => Self::Default,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceChoice {
    SystemDefault,
    ExplicitRedacted,
    ResolvedRedacted,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputRoute {
    Unresolved,
    MonoSumToOne,
    MainLeftRightToOneTwo,
    MainLeftRightToOneTwoOthersSilent,
}
impl OutputRoute {
    fn from_channels(channels: Option<u16>) -> Self {
        match channels {
            None => Self::Unresolved,
            Some(1) => Self::MonoSumToOne,
            Some(2) => Self::MainLeftRightToOneTwo,
            Some(_) => Self::MainLeftRightToOneTwoOthersSilent,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Route {
    pub backend: Backend,
    pub device: DeviceChoice,
    pub sample_rate: Option<u32>,
    pub channels: Option<u16>,
    pub format: Format,
    pub requested_buffer_frames: Option<u32>,
    pub output_route: OutputRoute,
}
impl Route {
    pub fn requested(settings: &crate::preferences::Audio) -> Self {
        Self {
            backend: settings
                .backend
                .as_deref()
                .map_or(Backend::SystemDefault, Backend::from_name),
            device: if settings.device.is_some() {
                DeviceChoice::ExplicitRedacted
            } else {
                DeviceChoice::SystemDefault
            },
            sample_rate: settings.sample_rate,
            channels: settings.channels,
            format: settings
                .format
                .map_or(Format::Default, |format| Format::from_cpal(format.cpal())),
            requested_buffer_frames: settings.buffer_frames,
            output_route: OutputRoute::from_channels(settings.channels),
        }
    }
    pub fn active(plan: &crate::engine::audio::config::Plan) -> Self {
        Self {
            backend: Backend::from_name(&plan.backend),
            device: DeviceChoice::ResolvedRedacted,
            sample_rate: Some(plan.rate),
            channels: Some(plan.channels),
            format: Format::from_cpal(plan.format),
            requested_buffer_frames: plan.buffer,
            output_route: OutputRoute::from_channels(Some(plan.channels)),
        }
    }
    fn validate(&self) -> Result<(), Error> {
        if self.output_route != OutputRoute::from_channels(self.channels) {
            return Err(Error::Invalid("route does not match its channel count"));
        }
        if self
            .sample_rate
            .is_some_and(|n| !(8000..=384000).contains(&n))
            || self.channels.is_some_and(|n| n == 0 || n > 256)
            || self
                .requested_buffer_frames
                .is_some_and(|n| n == 0 || n > 1_048_576)
        {
            return Err(Error::Invalid("unsupported route dimensions"));
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecoveryRef {
    pub session: [u8; 32],
    pub epoch: u64,
    pub sequence: u64,
    pub revision: u64,
    pub view_revision: u64,
    pub captured_unix_ms: u64,
    pub committed_unix_ms: u64,
}
/// These copies are an explicit privacy allowlist. Adding fields to engine
/// snapshots/counters cannot silently add fields to a support export.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Audio {
    pub last_callback: Option<Callback>,
    pub callbacks: u64,
    pub deadline_overruns: u64,
    pub max_elapsed_ns: u64,
    pub max_overrun_ns: u64,
    pub backend_errors: u64,
    pub device_lost: u64,
    pub dropped_buffers: Option<u64>,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Callback {
    pub elapsed_ns: u64,
    pub budget_ns: u64,
    pub render_cpu_ns: Option<u64>,
    pub overrun_ns: u64,
    pub output_latency_ns: Option<u64>,
    pub sample_rate: u32,
    pub channels: u16,
    pub frames: usize,
}
impl From<crate::engine::audio_metrics::AudioMetrics> for Audio {
    fn from(value: crate::engine::audio_metrics::AudioMetrics) -> Self {
        Self {
            last_callback: value.last_callback.map(|m| Callback {
                elapsed_ns: m.elapsed_ns,
                budget_ns: m.budget_ns,
                render_cpu_ns: m.render_cpu_ns,
                overrun_ns: m.overrun_ns,
                output_latency_ns: m.output_latency_ns,
                sample_rate: m.sample_rate,
                channels: m.channels,
                frames: m.frames,
            }),
            callbacks: value.callbacks,
            deadline_overruns: value.deadline_overruns,
            max_elapsed_ns: value.max_elapsed_ns,
            max_overrun_ns: value.max_overrun_ns,
            backend_errors: value.backend_errors,
            device_lost: value.device_lost,
            dropped_buffers: None,
        }
    }
}
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Commands {
    pub pending: usize,
    pub capacity: usize,
    pub observed_high_water: u64,
    pub reserved_releases: u64,
    pub full_rejections: u64,
    pub rejected: u64,
    pub accepted: u64,
    pub coalesced: u64,
}
impl From<crate::engine::QueuePressure> for Commands {
    fn from(q: crate::engine::QueuePressure) -> Self {
        Self {
            pending: q.pending,
            capacity: q.capacity,
            observed_high_water: q.observed_high_water,
            reserved_releases: q.reserved_releases,
            full_rejections: q.full_rejections,
            rejected: q.rejected,
            accepted: q.accepted,
            coalesced: q.coalesced,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Sample {
    pub elapsed_ms: u64,
    pub audio: Audio,
    pub commands: Commands,
    pub midi: [u64; 8],
    pub ui_update_ns: Option<u64>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Build {
    pub application: String,
    pub source_manifest_sha256: [u8; 32],
    pub target_os: String,
    pub architecture: String,
    /// Runtime backend versions are not exposed by CPAL; these are linked crates.
    pub dependencies: Vec<Dependency>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Dependency {
    pub name: String,
    pub version: String,
}
const DEPENDENCIES: &[&str] = &[
    "cpal",
    "midir",
    "symphonia",
    "eframe",
    "egui",
    "accesskit",
    "alsa",
    "alsa-sys",
];
fn version(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= 64
        && text
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b".-+_".contains(&b))
}
impl Build {
    /// Parsing the embedded source inventory is worker-only and never reads a
    /// user Cargo file, environment value, or filesystem path.
    pub fn current() -> Self {
        let value: serde_json::Value =
            serde_json::from_str(crate::licenses::MANIFEST).unwrap_or(serde_json::Value::Null);
        let dependencies = value
            .get("cargo")
            .and_then(|v| v.as_array())
            .into_iter()
            .flatten()
            .filter_map(|entry| {
                let name = entry.get("name")?.as_str()?;
                let version = entry.get("version")?.as_str()?;
                (DEPENDENCIES.contains(&name) && self::version(version)).then(|| Dependency {
                    name: name.into(),
                    version: version.into(),
                })
            })
            .take(16)
            .collect();
        Self {
            application: env!("CARGO_PKG_VERSION").into(),
            source_manifest_sha256: Sha256::digest(crate::licenses::MANIFEST.as_bytes()).into(),
            target_os: std::env::consts::OS.into(),
            architecture: std::env::consts::ARCH.into(),
            dependencies,
        }
    }
    fn validate(&self) -> Result<(), Error> {
        if !version(&self.application)
            || !["linux"].contains(&self.target_os.as_str())
            || !["aarch64", "x86_64"].contains(&self.architecture.as_str())
            || self.dependencies.len() > 16
            || self
                .dependencies
                .iter()
                .any(|d| !DEPENDENCIES.contains(&d.name.as_str()) || !version(&d.version))
        {
            return Err(Error::Invalid("unrecognized build metadata"));
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Report {
    pub schema: u32,
    pub run: Id,
    pub build: Option<Build>,
    pub exit: Exit,
    pub safe_mode: bool,
    pub started_unix_ms: u64,
    pub collected_unix_ms: u64,
    pub requested_audio: Option<Route>,
    pub active_audio: Option<Route>,
    /// Explicit capability boundary; this build never hosts third-party plugins.
    pub audio_plugin_host_available: bool,
    pub backend_runtime_version: Option<String>,
    pub events: Vec<Event>,
    pub dropped_events: u64,
    pub dropped_observations: u64,
    pub samples: Vec<Sample>,
    pub dropped_samples: u64,
    pub recovery: Vec<RecoveryRef>,
}
impl Report {
    pub fn new(run: Id, safe_mode: bool, unix_ms: u64) -> Self {
        Self {
            schema: 1,
            run,
            build: Some(Build::current()),
            exit: Exit::Running,
            safe_mode,
            started_unix_ms: unix_ms,
            collected_unix_ms: unix_ms,
            requested_audio: None,
            active_audio: None,
            audio_plugin_host_available: false,
            backend_runtime_version: None,
            events: Vec::new(),
            dropped_events: 0,
            dropped_observations: 0,
            samples: Vec::new(),
            dropped_samples: 0,
            recovery: Vec::new(),
        }
    }
    pub fn validate(&self) -> Result<(), Error> {
        if self.schema != 1
            || self.events.len() > MAX_EVENTS
            || self.samples.len() > MAX_SAMPLES
            || self.recovery.len() > MAX_RECOVERIES
        {
            return Err(Error::Invalid("unsupported report schema or record limit"));
        }
        if let Some(build) = &self.build {
            build.validate()?;
        }
        for route in [&self.requested_audio, &self.active_audio]
            .into_iter()
            .flatten()
        {
            route.validate()?;
        }
        if self.audio_plugin_host_available || self.backend_runtime_version.is_some() {
            return Err(Error::Invalid(
                "unsupported capability or unreported backend runtime version",
            ));
        }
        if self
            .events
            .windows(2)
            .any(|w| w[0].elapsed_ms > w[1].elapsed_ms)
            || self
                .samples
                .windows(2)
                .any(|w| w[0].elapsed_ms > w[1].elapsed_ms)
        {
            return Err(Error::Invalid("non-monotonic report observations"));
        }
        if self
            .samples
            .iter()
            .any(|sample| sample.audio.dropped_buffers.is_some())
        {
            return Err(Error::Invalid(
                "backend dropped-buffer counts are unavailable",
            ));
        }
        Ok(())
    }
    pub fn record(&mut self, mut event: Event) {
        // Multiple non-RT producers may enqueue in a different order from
        // their clock reads. Times describe monotonic collector observations.
        if let Some(last) = self.events.last() {
            event.elapsed_ms = event.elapsed_ms.max(last.elapsed_ms);
        }
        if let Some(last) = self.events.last_mut() {
            if last.code == event.code && last.failure == event.failure {
                last.repeats = last.repeats.saturating_add(1);
                return;
            }
        }
        if self.events.len() == MAX_EVENTS {
            self.events.remove(0);
            self.dropped_events = self.dropped_events.saturating_add(1);
        }
        self.events.push(event);
    }
    pub fn sample(&mut self, mut sample: Sample) {
        if let Some(last) = self.samples.last() {
            sample.elapsed_ms = sample.elapsed_ms.max(last.elapsed_ms);
        }
        if self.samples.len() == MAX_SAMPLES {
            self.samples.remove(0);
            self.dropped_samples = self.dropped_samples.saturating_add(1);
        }
        self.samples.push(sample);
    }
}
