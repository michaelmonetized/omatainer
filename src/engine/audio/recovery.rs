//! Recovery retains the accepted route and verifies physical identity off the callback.
use super::config::{self, Plan};
use cpal::traits::{DeviceTrait, HostTrait};
use std::path::{Path, PathBuf};
use std::time::Duration;

#[derive(Clone, Debug)]
pub struct Target {
    pub plan: Plan,
    pub identity: Option<String>,
}

/// Retain explicit output settings.
/// Takes an accepted plan; returns its exact rate, channels, format and buffer.
pub(super) fn settings(plan: &Plan) -> crate::preferences::Audio {
    crate::preferences::Audio {
        graph: plan.graph.clone(),
        backend: Some(plan.backend.clone()),
        device: Some(plan.device.clone()),
        sample_rate: Some(plan.rate),
        channels: Some(plan.channels),
        buffer_frames: plan.buffer,
        format: crate::preferences::AudioFormat::ALL
            .into_iter()
            .find(|format| format.cpal() == plan.format),
        ..Default::default()
    }
}

fn text(path: PathBuf) -> Option<String> {
    let bytes = std::fs::read(path).ok()?;
    if bytes.len() > 1024 {
        return None;
    }
    let value = String::from_utf8(bytes).ok()?.trim().to_owned();
    (!value.is_empty() && !value.chars().any(char::is_control)).then_some(value)
}

fn physical(device: &Path) -> Option<Vec<String>> {
    let device = device.canonicalize().ok()?;
    for parent in device.ancestors() {
        if parent.join("idVendor").exists() {
            return Some(vec![
                "usb".into(),
                text(parent.join("idVendor"))?,
                text(parent.join("idProduct"))?,
                text(parent.join("serial"))?,
                device
                    .ancestors()
                    .take_while(|path| *path != parent)
                    .find_map(|path| text(path.join("bInterfaceNumber")))?,
            ]);
        }
    }
    Some(vec![
        "fixed".into(),
        device.to_str()?.into(),
        text(device.join("vendor")).unwrap_or_default(),
        text(device.join("device")).unwrap_or_default(),
    ])
}

fn identity_at(name: &str, sound: &Path) -> Option<String> {
    let (kind, parameters) = name.split_once(':')?;
    if !matches!(
        kind,
        "hw" | "plughw"
            | "sysdefault"
            | "front"
            | "rear"
            | "center_lfe"
            | "side"
            | "surround21"
            | "surround40"
            | "surround41"
            | "surround50"
            | "surround51"
            | "surround71"
            | "iec958"
            | "hdmi"
            | "dmix"
    ) {
        return None;
    }
    let mut parts = parameters.split(',');
    let card = parts
        .next()?
        .strip_prefix("CARD=")
        .unwrap_or(parameters.split(',').next()?);
    if card.is_empty() {
        return None;
    }
    let route: Vec<_> = parts.collect();
    if route.iter().any(|part| part.starts_with("CARD=")) {
        return None;
    }
    let mut matching = std::fs::read_dir(sound)
        .ok()?
        .take(257)
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name();
            let name = name.to_str()?;
            let index = name.strip_prefix("card")?;
            if index.is_empty() || !index.bytes().all(|byte| byte.is_ascii_digit()) {
                return None;
            }
            let path = entry.path();
            (card == index || text(path.join("id")).as_deref() == Some(card)).then_some(path)
        });
    let card = matching.next()?;
    if matching.next().is_some() {
        return None;
    }
    let physical = physical(&card.join("device"))?;
    serde_json::to_string(&("alsa-physical-v1", physical, kind, route)).ok()
}

/// Resolve a physical ALSA endpoint.
/// Takes its exact PCM name; returns a USB serial or fixed-device identity and route.
/// Default/server aliases and USB devices without a serial return no identity.
pub(super) fn identity(name: &str) -> Option<String> {
    identity_at(name, Path::new("/sys/class/sound"))
}

/// Rediscover the retained physical output.
/// Takes its accepted identity and route; returns the current name or an error.
pub(super) fn discover(target: &Target) -> Result<Plan, String> {
    let expected = target.identity.as_ref().ok_or(
        "This output has no verifiable physical identity; preview and confirm a fallback explicitly")?;
    let host = cpal::default_host();
    if host.id().name() != target.plan.backend {
        return Err("Retained backend is unavailable".into());
    }
    let devices = host
        .output_devices()
        .map_err(|error| error.to_string())?
        .take(257)
        .collect::<Vec<_>>();
    if devices.len() > 256 {
        return Err("Output inventory is truncated; reconnect refused".into());
    }
    let mut names = devices
        .iter()
        .filter_map(config::name)
        .filter(|name| identity(name).as_ref() == Some(expected));
    let name = names
        .next()
        .ok_or("Retained physical output is unavailable; no other output was activated")?;
    if names.next().is_some() {
        return Err("Retained identity is ambiguous; no output was activated".into());
    }
    let mut settings = settings(&target.plan);
    settings.device = Some(name);
    super::config::select(&settings)
        .map(|(_, plan)| plan)
        .map_err(|error| error.to_string())
}

/// Read elapsed boot time including system suspend.
/// Takes no arguments; returns a monotonic duration when the Linux clock is available.
pub(super) fn boot_time() -> Option<Duration> {
    let mut time = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    if unsafe { libc::clock_gettime(libc::CLOCK_BOOTTIME, &mut time) } != 0 {
        return None;
    }
    Some(Duration::new(
        time.tv_sec.try_into().ok()?,
        time.tv_nsec.try_into().ok()?,
    ))
}

pub(super) struct Watchdog {
    checked: Duration,
    progressed: Duration,
    callbacks: u64,
    timeout: Duration,
}
impl Watchdog {
    /// Start callback liveness observation.
    /// Takes boot time, callback count and nominal buffer duration; returns bounded state.
    pub fn new(now: Duration, callbacks: u64, budget: Duration) -> Self {
        Self {
            checked: now,
            progressed: now,
            callbacks,
            timeout: Duration::from_secs(5).max(budget.saturating_mul(4)),
        }
    }
    /// Detect stalled output or a long owner/suspend gap.
    /// Takes current boot time and callback count; returns whether output must retire.
    pub fn lost(&mut self, now: Duration, callbacks: u64) -> bool {
        let gap = now.saturating_sub(self.checked) > Duration::from_secs(5);
        self.checked = now;
        if callbacks != self.callbacks {
            self.callbacks = callbacks;
            self.progressed = now;
        }
        gap || now.saturating_sub(self.progressed) > self.timeout
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn callback_stall_and_suspend_gap_fail_closed_without_treating_regular_progress_as_loss() {
        let mut watch = Watchdog::new(Duration::ZERO, 0, Duration::from_millis(10));
        for second in 1..10 {
            assert!(!watch.lost(Duration::from_secs(second), second));
        }
        for second in 10..=14 {
            assert!(!watch.lost(Duration::from_secs(second), 9));
        }
        assert!(watch.lost(Duration::from_secs(15), 9));
        let mut watch = Watchdog::new(Duration::ZERO, 0, Duration::from_secs(4));
        for second in 1..=16 {
            assert!(!watch.lost(Duration::from_secs(second), 0));
        }
        assert!(watch.lost(Duration::from_secs(17), 0));
        let mut watch = Watchdog::new(Duration::ZERO, 0, Duration::ZERO);
        assert!(watch.lost(Duration::from_secs(6), 100));
    }
    #[test]
    fn physical_identity_survives_card_renumbering_and_refuses_replaced_or_serialless_usb() {
        use std::os::unix::fs::symlink;
        let root =
            std::env::temp_dir().join(format!("omatainer-physical-id-{}", std::process::id()));
        std::fs::create_dir(&root).unwrap();
        let sound = root.join("sound");
        let usb = root.join("devices/usb");
        let interface = usb.join("1-1:1.0");
        std::fs::create_dir_all(&interface).unwrap();
        std::fs::create_dir(&sound).unwrap();
        std::fs::write(interface.join("bInterfaceNumber"), "00").unwrap();
        for (name, value) in [
            ("idVendor", "1234"),
            ("idProduct", "abcd"),
            ("serial", "unit-1"),
        ] {
            std::fs::write(usb.join(name), value).unwrap();
        }
        let card = sound.join("card2");
        std::fs::create_dir(&card).unwrap();
        std::fs::write(card.join("id"), "Studio").unwrap();
        symlink(&interface, card.join("device")).unwrap();
        let first = identity_at("hw:CARD=Studio,DEV=0", &sound).unwrap();
        std::fs::rename(&card, sound.join("card4")).unwrap();
        std::fs::write(sound.join("card4/id"), "Renamed").unwrap();
        assert_eq!(
            identity_at("hw:CARD=Renamed,DEV=0", &sound).as_ref(),
            Some(&first)
        );
        assert_ne!(
            identity_at("hw:CARD=Renamed,DEV=1", &sound).as_ref(),
            Some(&first)
        );
        std::fs::write(usb.join("serial"), "unit-2").unwrap();
        assert_ne!(
            identity_at("hw:CARD=Renamed,DEV=0", &sound).as_ref(),
            Some(&first)
        );
        std::fs::remove_file(usb.join("serial")).unwrap();
        assert!(identity_at("hw:CARD=Renamed,DEV=0", &sound).is_none());
        assert!(identity_at("default", &sound).is_none());
        assert!(identity_at("pipewire", &sound).is_none());
        assert!(identity_at("hw:CARD=Renamed,CARD=Other", &sound).is_none());
        std::fs::remove_dir_all(root).unwrap();
    }
}
