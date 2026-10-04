//! Device discovery and configuration planning stay outside the render callback.
use super::*;
use crate::preferences::Audio;
use anyhow::Context;
use cpal::traits::HostTrait;

#[derive(Clone, Debug, PartialEq)]
pub struct Range {
    pub channels: u16,
    pub min_rate: u32,
    pub max_rate: u32,
    pub format: cpal::SampleFormat,
    pub buffer: Option<(u32, u32)>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Device {
    pub name: String,
    pub default: bool,
    pub defaults: Option<(u16, u32, cpal::SampleFormat)>,
    pub ranges: Vec<Range>,
    pub error: Option<String>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Inventory {
    pub(crate) graph_ports: Vec<(String, bool)>,
    pub backend: String,
    pub devices: Vec<Device>,
    pub inputs: Vec<Device>,
    pub input_error: Option<String>,
    pub truncated: bool,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Plan {
    pub(crate) graph: graph::Routes,
    pub backend: String,
    pub device: String,
    pub channels: u16,
    pub rate: u32,
    pub format: cpal::SampleFormat,
    pub buffer: Option<u32>,
    pub warning: Option<String>,
}
impl Plan {
    pub fn route(&self) -> String {
        if self.channels == 1 {
            "Main L+R summed to output 1".into()
        } else if self.channels == 2 {
            "Main left/right to outputs 1/2".into()
        } else {
            format!(
                "Main left/right to outputs 1/2; outputs 3–{} silent",
                self.channels
            )
        }
    }
    pub fn config(&self) -> cpal::StreamConfig {
        cpal::StreamConfig {
            channels: self.channels,
            sample_rate: cpal::SampleRate(self.rate),
            buffer_size: self
                .buffer
                .map(cpal::BufferSize::Fixed)
                .unwrap_or(cpal::BufferSize::Default),
        }
    }
}
fn supported(format: cpal::SampleFormat) -> bool {
    matches!(
        format,
        cpal::SampleFormat::F32
            | cpal::SampleFormat::F64
            | cpal::SampleFormat::I8
            | cpal::SampleFormat::I16
            | cpal::SampleFormat::I32
            | cpal::SampleFormat::I64
            | cpal::SampleFormat::U8
            | cpal::SampleFormat::U16
            | cpal::SampleFormat::U32
            | cpal::SampleFormat::U64
    )
}
fn inspect(device: &cpal::Device, is_default: bool, input: bool) -> Device {
    let name = device.name().unwrap_or_else(|_| "Unnamed output".into());
    let defaults = (if input {
        device.default_input_config()
    } else {
        device.default_output_config()
    })
    .ok()
    .map(|config| {
        (
            config.channels(),
            config.sample_rate().0,
            config.sample_format(),
        )
    });
    let (ranges, error) = match if input {
        device
            .supported_input_configs()
            .map(|items| items.take(4097).collect::<Vec<_>>())
    } else {
        device
            .supported_output_configs()
            .map(|items| items.take(4097).collect::<Vec<_>>())
    } {
        Ok(configs) => (
            configs
                .into_iter()
                .take(4096)
                .map(|config| Range {
                    channels: config.channels(),
                    min_rate: config.min_sample_rate().0,
                    max_rate: config.max_sample_rate().0,
                    format: config.sample_format(),
                    buffer: match config.buffer_size() {
                        cpal::SupportedBufferSize::Range { min, max } => Some((*min, *max)),
                        cpal::SupportedBufferSize::Unknown => None,
                    },
                })
                .collect(),
            None,
        ),
        Err(error) => (Vec::new(), Some(error.to_string())),
    };
    Device {
        name,
        default: is_default,
        defaults,
        ranges,
        error,
    }
}

pub fn discover() -> Result<Inventory, String> {
    let host = cpal::default_host();
    let outputs = host
        .output_devices()
        .map_err(|e| e.to_string())?
        .take(257)
        .collect::<Vec<_>>();
    let inputs = host
        .input_devices()
        .map(|devices| devices.take(257).collect::<Vec<_>>())
        .map_err(|e| e.to_string());
    let mut truncated =
        outputs.len() > 256 || inputs.as_ref().is_ok_and(|devices| devices.len() > 256);
    fn collect(
        devices: Vec<cpal::Device>,
        default: Option<cpal::Device>,
        input: bool,
    ) -> Vec<Device> {
        let mut found = devices
            .into_iter()
            .take(256)
            .map(|device| inspect(&device, false, input))
            .collect::<Vec<_>>();
        if let Some(default) = default {
            let default = inspect(&default, true, input);
            if let Some(index) = found.iter().position(|d| d.name == default.name) {
                found[index] = default;
            } else {
                if found.len() == 256 {
                    found.pop();
                }
                found.push(default);
            }
        }
        found
    }
    let devices = collect(outputs, host.default_output_device(), false);
    let (inputs, input_error) = match inputs {
        Ok(inputs) => (collect(inputs, host.default_input_device(), true), None),
        Err(error) => (Vec::new(), Some(error)),
    };
    truncated |= devices
        .iter()
        .chain(&inputs)
        .any(|device| device.ranges.len() == 4096);
    Ok(Inventory {
        graph_ports: Vec::new(),
        backend: host.id().name().into(),
        devices,
        inputs,
        input_error,
        truncated,
    })
}

pub fn plan(settings: &Audio, inventory: &Inventory) -> Result<Plan, String> {
    if settings
        .backend
        .as_ref()
        .is_some_and(|backend| backend != &inventory.backend)
    {
        return Err(
            "Saved audio backend is unavailable; select the current backend explicitly".into(),
        );
    }

    let matches: Vec<_> = inventory
        .devices
        .iter()
        .filter(|device| {
            settings
                .device
                .as_ref()
                .map(|name| device.name == *name)
                .unwrap_or(device.default)
        })
        .collect();
    if matches.is_empty() {
        return Err(format!(
            "Audio output {} is unavailable",
            settings.device.as_deref().unwrap_or("System default")
        ));
    }
    if matches.len() != 1 {
        return Err(
            "Audio device name is ambiguous; choose System default or rename duplicate devices"
                .into(),
        );
    }
    let device = matches[0];
    if let Some(error) = &device.error {
        return Err(format!("{}: {error}", device.name));
    }
    let (default_channels, default_rate, default_format) = match device.defaults {
        Some(defaults)=>defaults,
        None=>match (settings.channels,settings.sample_rate,settings.format) {
            (Some(channels),Some(rate),Some(format))=>(channels,rate,format.cpal()),
            _=>return Err("Device has no default configuration; choose channels, rate and sample format explicitly".into()),
        }
    };
    let channels = settings.channels.unwrap_or(default_channels);
    let rate = settings.sample_rate.unwrap_or(default_rate);
    let mut matches: Vec<_> = device
        .ranges
        .iter()
        .filter(|range| {
            supported(range.format)
                && settings
                    .format
                    .is_none_or(|format| format.cpal() == range.format)
                && range.channels == channels
                && (range.min_rate..=range.max_rate).contains(&rate)
                && settings.buffer_frames.is_none_or(|frames| {
                    range
                        .buffer
                        .is_none_or(|(min, max)| (min..=max).contains(&frames))
                })
        })
        .collect();
    matches.sort_by_key(|range| {
        if range.format == default_format {
            0
        } else {
            match range.format {
                cpal::SampleFormat::F32 => 1,
                cpal::SampleFormat::I16 => 2,
                _ => 3,
            }
        }
    });
    let range = matches.first().ok_or_else(|| {
        format!(
            "{} does not advertise {rate} Hz, {channels} channels and the requested buffer",
            device.name
        )
    })?;
    settings.graph.validate()?;
    if inventory.backend == "JACK" && settings.graph.outputs.iter().any(|link| link.channel >= channels) { return Err("Saved graph output exceeds the selected channel count".into()); }
    Ok(Plan {
        graph: settings.graph.clone(), backend: inventory.backend.clone(), device: device.name.clone(), channels, rate, format: range.format,
        buffer: if inventory.backend == "JACK" { range.buffer.map(|(quantum, _)| quantum) } else { settings.buffer_frames },
        warning: if inventory.backend == "JACK" { Some("The graph server owns rate and quantum. No system connections are automatic; missing saved endpoints stay disconnected.".into()) } else { (settings.buffer_frames.is_some() && range.buffer.is_none()).then(|| "The device does not advertise buffer limits; opening the stream may still reject this buffer".into()) },
    })
}

/// Revalidate immediately before opening; the preview may have become stale.
pub(super) fn select(settings: &Audio) -> anyhow::Result<(cpal::Device, Plan)> {
    let host = cpal::default_host();
    let device = if let Some(name) = &settings.device {
        let mut matches: Vec<_> = host
            .output_devices()?
            .filter(|device| device.name().ok().as_ref() == Some(name))
            .collect();
        anyhow::ensure!(
            matches.len() == 1,
            "configured audio output {name:?} is missing or ambiguous"
        );
        matches.remove(0)
    } else {
        host.default_output_device()
            .context("no default audio output (PipeWire/ALSA)")?
    };
    let inventory = Inventory {
        graph_ports: Vec::new(),
        backend: host.id().name().into(),
        devices: vec![inspect(&device, true, false)],
        inputs: Vec::new(),
        input_error: None,
        truncated: false,
    };
    let plan = plan(settings, &inventory).map_err(anyhow::Error::msg)?;
    Ok((device, plan))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    pub(crate) fn inventory() -> Inventory {
        Inventory {
            graph_ports: Vec::new(),
            backend: "fixture".into(),
            inputs: Vec::new(),
            input_error: None,
            truncated: false,
            devices: vec![Device {
                name: "Studio interface".into(),
                default: true,
                defaults: Some((2, 48000, cpal::SampleFormat::F32)),
                error: None,
                ranges: vec![
                    Range {
                        channels: 2,
                        min_rate: 44100,
                        max_rate: 96000,
                        format: cpal::SampleFormat::F32,
                        buffer: Some((64, 1024)),
                    },
                    Range {
                        channels: 4,
                        min_rate: 48000,
                        max_rate: 48000,
                        format: cpal::SampleFormat::I16,
                        buffer: None,
                    },
                ],
            }],
        }
    }
    #[test]
    fn exact_configuration_forwarding_and_routes_never_silently_fall_back() {
        let inventory = inventory();
        let mut settings = Audio::default();
        let default = plan(&settings, &inventory).unwrap();
        assert_eq!(default.rate, 48000);
        assert_eq!(default.channels, 2);
        settings.device = Some("Studio interface".into());
        settings.sample_rate = Some(96000);
        settings.buffer_frames = Some(512);
        let chosen = plan(&settings, &inventory).unwrap();
        let native = chosen.config();
        assert_eq!(native.sample_rate.0, 96000);
        assert_eq!(native.buffer_size, cpal::BufferSize::Fixed(512));
        assert_eq!(native.channels, 2);
        for bad in [32, 2048] {
            settings.buffer_frames = Some(bad);
            assert!(plan(&settings, &inventory).is_err());
        }
        settings.buffer_frames = Some(256);
        settings.sample_rate = Some(192000);
        assert!(plan(&settings, &inventory).is_err());
        settings.sample_rate = Some(48000);
        settings.channels = Some(4);
        let chosen = plan(&settings, &inventory).unwrap();
        assert!(chosen.warning.is_some());
        assert!(chosen.route().contains("3–4 silent"));
        assert_eq!(chosen.format, cpal::SampleFormat::I16);
        settings.device = Some("Missing controller".into());
        assert!(plan(&settings, &inventory)
            .unwrap_err()
            .contains("Missing controller"));
        settings.device = Some("Studio interface".into());
        let mut ambiguous = inventory.clone();
        ambiguous.devices.push(ambiguous.devices[0].clone());
        assert!(plan(&settings, &ambiguous).is_err());
        let mut mono = default;
        mono.channels = 1;
        assert!(mono.route().contains("summed"));
    }
}

/// Re-open the exact accepted device/configuration for rollback; do not silently
/// follow a changed system default to a different interface.
pub(super) fn select_exact(plan: &Plan) -> anyhow::Result<cpal::Device> {
    let host = cpal::default_host();
    anyhow::ensure!(
        host.id().name() == plan.backend,
        "Audio backend changed since preview"
    );
    let mut matching = host
        .output_devices()?
        .filter(|device| device.name().ok().as_deref() == Some(plan.device.as_str()));
    let device = matching
        .next()
        .context("Previously selected output is unavailable")?;
    anyhow::ensure!(
        matching.next().is_none(),
        "Previously selected output name became ambiguous"
    );
    let found = inspect(&device, true, false);
    anyhow::ensure!(
        found.ranges.iter().any(|r| r.channels == plan.channels
            && r.format == plan.format
            && (r.min_rate..=r.max_rate).contains(&plan.rate)
            && plan
                .buffer
                .is_none_or(|b| r.buffer.is_none_or(|(lo, hi)| (lo..=hi).contains(&b)))),
        "Output capabilities changed since preview"
    );
    Ok(device)
}

pub fn calibration_plan(
    settings: &Audio,
    inventory: &Inventory,
    output: &Plan,
) -> Result<Plan, String> {
    if output.rate > super::calibration::MAX_RATE {
        return Err("Calibration supports rates up to 192 kHz".into());
    }
    if let Some(error) = &inventory.input_error {
        return Err(format!("Input enumeration failed: {error}"));
    }
    let input = &settings.calibration;
    let selected = Audio {
        backend: Some(output.backend.clone()),
        device: input.device.clone(),
        sample_rate: Some(output.rate),
        channels: input.channels,
        buffer_frames: input.buffer_frames,
        format: input.format,
        calibration: Default::default(),
        graph: Default::default(),
    };
    let input_inventory = Inventory {
        graph_ports: Vec::new(),
        backend: inventory.backend.clone(),
        devices: inventory.inputs.clone(),
        inputs: Vec::new(),
        input_error: None,
        truncated: false,
    };
    let plan =
        plan(&selected, &input_inventory).map_err(|error| error.replace("output", "input"))?;
    if input.channel >= plan.channels || input.output_channel >= output.channels {
        return Err("Calibration channel is outside the selected input/output layout".into());
    }
    Ok(plan)
}
pub(super) fn select_input_exact(plan: &Plan) -> anyhow::Result<cpal::Device> {
    let host = cpal::default_host();
    anyhow::ensure!(
        host.id().name() == plan.backend,
        "Input backend changed since preview"
    );
    let mut devices = host
        .input_devices()?
        .filter(|device| device.name().ok().as_deref() == Some(plan.device.as_str()));
    let device = devices
        .next()
        .context("Selected calibration input is unavailable")?;
    anyhow::ensure!(
        devices.next().is_none(),
        "Selected calibration input name is ambiguous"
    );
    let found = inspect(&device, true, true);
    anyhow::ensure!(
        found
            .ranges
            .iter()
            .any(|range| range.channels == plan.channels
                && range.format == plan.format
                && (range.min_rate..=range.max_rate).contains(&plan.rate)
                && plan
                    .buffer
                    .is_none_or(|b| range.buffer.is_none_or(|(lo, hi)| (lo..=hi).contains(&b)))),
        "Input capabilities changed since preview"
    );
    Ok(device)
}

#[cfg(test)]
mod professional_tests {
    use super::*;
    #[test]
    fn advertised_rates_formats_channels_and_input_routes_are_exact_and_missing_modes_fail() {
        let mut inventory = tests::inventory();
        inventory.devices[0].ranges.clear();
        for rate in [44100, 48000, 96000, 192000] {
            for format in crate::preferences::AudioFormat::ALL {
                inventory.devices[0].ranges.push(Range {
                    channels: 2,
                    min_rate: rate,
                    max_rate: rate,
                    format: format.cpal(),
                    buffer: Some((64, 512)),
                });
            }
        }
        inventory.inputs = inventory.devices.clone();
        let mut settings = Audio::default();
        settings.backend = Some("fixture".into());
        settings.channels = Some(2);
        settings.buffer_frames = Some(128);
        settings.calibration.channels = Some(2);
        settings.calibration.buffer_frames = Some(128);
        for rate in [44100, 48000, 96000, 192000] {
            for format in crate::preferences::AudioFormat::ALL {
                settings.sample_rate = Some(rate);
                settings.format = Some(format);
                settings.calibration.format = Some(format);
                let output = plan(&settings, &inventory).unwrap();
                assert_eq!(output.rate, rate);
                assert_eq!(output.format, format.cpal());
                assert_eq!(output.config().buffer_size, cpal::BufferSize::Fixed(128));
                let input = calibration_plan(&settings, &inventory, &output).unwrap();
                assert_eq!(input.rate, rate);
                assert_eq!(input.format, format.cpal());
                settings.calibration.channel = 2;
                assert!(calibration_plan(&settings, &inventory, &output).is_err());
                settings.calibration.channel = 0;
            }
        }
        settings.sample_rate = Some(88200);
        assert!(plan(&settings, &inventory).is_err());
        settings.sample_rate = Some(48000);
        settings.backend = Some("Missing backend".into());
        assert!(plan(&settings, &inventory).is_err());
        settings.backend = None;
        inventory.devices[0].defaults = None;
        assert!(plan(&settings, &inventory).is_ok());
        settings.format = None;
        assert!(plan(&settings, &inventory).is_err());
    }
    #[test]
    #[ignore = "read-only native driver capability inventory; no stream playback or probe"]
    fn native_readonly_inventory() {
        let inventory = discover().unwrap();
        eprintln!("NATIVE READ-ONLY INVENTORY {inventory:#?}");
        assert!(!inventory.backend.is_empty());
    }
}

/// Discover the selected Linux workflow.
/// Takes an optional exact backend name; returns ALSA capabilities or the existing JACK server.
pub(crate) fn discover_for(backend: Option<&str>) -> Result<Inventory, String> {
    #[cfg(target_os = "linux")]
    if backend == Some(super::jack::BACKEND) { return super::jack::discover(); }
    let inventory = discover()?;
    if backend.is_some_and(|name| name != inventory.backend) { return Err("Saved audio backend is unavailable".into()); }
    Ok(inventory)
}
