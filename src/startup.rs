//! Explicit startup choices shared by the native window and headless diagnostic.
use std::path::{Path, PathBuf};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Launch {
    pub safe_mode: bool,
    pub startup_check: bool,
    pub defaults_once: bool,
}
impl Launch {
    pub fn parse(args: &[String]) -> anyhow::Result<Self> {
        let mut result = Self::default();
        for arg in args {
            let slot=match arg.as_str(){"--safe-mode"=>&mut result.safe_mode,"--startup-check"=>&mut result.startup_check,"--defaults-once"=>&mut result.defaults_once,_=>anyhow::bail!("unknown startup argument; usage: omatainer [--safe-mode [--startup-check] | --defaults-once]")};
            anyhow::ensure!(!*slot, "duplicate startup argument");
            *slot = true;
        }
        anyhow::ensure!(
            !result.startup_check || result.safe_mode,
            "--startup-check requires --safe-mode"
        );
        anyhow::ensure!(
            !result.defaults_once || !result.safe_mode,
            "--defaults-once and --safe-mode cannot be combined"
        );
        Ok(result)
    }
}
pub(crate) struct Paths {
    pub home: PathBuf,
    pub preferences: PathBuf,
    pub support: PathBuf,
}
impl Paths {
    /// Explicit inputs keep native child fixtures isolated without repurposing
    /// HOME or mutating global environment in a parallel test process.
    pub fn resolve(
        home: &Path,
        config: Option<&Path>,
        state: Option<&Path>,
    ) -> anyhow::Result<Self> {
        anyhow::ensure!(home.is_absolute(), "HOME must be absolute");
        Ok(Self {
            home: home.into(),
            preferences: crate::preferences::storage::default_path(
                config.map(Path::as_os_str),
                home,
            ),
            support: crate::support::default_path(state, Some(home))?,
        })
    }
    pub fn environment() -> anyhow::Result<Self> {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .ok_or_else(|| anyhow::anyhow!("HOME is unavailable"))?;
        let config = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from);
        let state = std::env::var_os("XDG_STATE_HOME").map(PathBuf::from);
        Self::resolve(&home, config.as_deref(), state.as_deref())
    }
}
pub(crate) fn check_safe_engine(
    engine: &crate::engine::Engine,
    preferences_readable: bool,
) -> anyhow::Result<serde_json::Value> {
    anyhow::ensure!(
        engine.safe_mode(),
        "startup diagnostic requires the real safe engine"
    );
    let capture = engine
        .project
        .capture(&std::sync::atomic::AtomicBool::new(false))?;
    let snapshot = engine.snapshot();
    anyhow::ensure!(
        !snapshot.playing && snapshot.decks.iter().all(|d| !d.playing),
        "safe transport did not stay stopped"
    );
    anyhow::ensure!(
        engine.output_info().is_none() && !engine.midi.connections_available(),
        "safe startup unexpectedly opened a backend"
    );
    Ok(serde_json::json!({
        "schema":1,
        "scope":"headless_safe_startup_not_native_gui_or_device_qa",
        "safe_mode":true,"audio_output_open":false,"midi_manager_available":false,
        "project_capture_available":true,"project_revision":capture.revision,
        "transport_stopped":true,"backend_callbacks":engine.cmd.audio_metrics().callbacks,
        "preferences_readable":preferences_readable,"audio_plugin_host_available":false,
        "upload_client_available":false,
    }))
}
#[cfg(test)]
mod tests {
    use super::*;
    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|s| (*s).into()).collect()
    }
    #[test]
    fn startup_arguments_reject_unknown_duplicates_and_conflicts() {
        assert_eq!(Launch::parse(&[]).unwrap(), Launch::default());
        assert!(
            Launch::parse(&args(&["--safe-mode", "--startup-check"]))
                .unwrap()
                .startup_check
        );
        for values in [
            &["--wat"][..],
            &["--startup-check"],
            &["--safe-mode", "--safe-mode"],
            &["--defaults-once", "--safe-mode"],
            &["--startup-check", "--defaults-once"],
        ] {
            assert!(Launch::parse(&args(values)).is_err());
        }
        let paths = Paths::resolve(
            Path::new("/unused-home"),
            Some(Path::new("/private/config")),
            Some(Path::new("/private/state")),
        )
        .unwrap();
        assert_eq!(
            paths.preferences,
            Path::new("/private/config/omatainer/preferences.json")
        );
        assert_eq!(paths.support, Path::new("/private/state/omatainer/support"));
    }
}
