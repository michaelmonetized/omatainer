//! Bounded preference files and atomic publication; called by startup or a worker.
use super::*;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

const MAX_BYTES: u64 = 1_048_576;
static NEXT_TEMP: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Revision {
    dev: u64,
    ino: u64,
    len: u64,
    modified: (i64, i64),
    changed: (i64, i64),
}
impl Revision {
    fn of(meta: &fs::Metadata) -> Self {
        Self {
            dev: meta.dev(),
            ino: meta.ino(),
            len: meta.len(),
            modified: (meta.mtime(), meta.mtime_nsec()),
            changed: (meta.ctime(), meta.ctime_nsec()),
        }
    }
}
#[derive(Debug)]
pub struct Loaded {
    pub preferences: Preferences,
    pub revision: Option<Revision>,
    pub migrated: bool,
}
#[derive(Debug)]
pub enum Error {
    Cancelled,
    Conflict,
    Invalid(String),
    Io(String),
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Cancelled => write!(f, "Cancelled; existing preferences were preserved"),
            Self::Conflict => write!(f, "Preferences changed on disk; reload before saving"),
            Self::Invalid(message) | Self::Io(message) => f.write_str(message),
        }
    }
}
impl std::error::Error for Error {}
fn io(stage: &str, error: std::io::Error) -> Error {
    Error::Io(format!("{stage}: {error}"))
}
fn check(cancel: &AtomicBool) -> Result<(), Error> {
    if cancel.load(Ordering::Acquire) {
        Err(Error::Cancelled)
    } else {
        Ok(())
    }
}

pub fn read_raw(path: &Path, cancel: &AtomicBool) -> Result<(Vec<u8>, Revision), Error> {
    check(cancel)?;
    if !path.is_absolute() {
        return Err(Error::Invalid(
            "Use an absolute preference file path".into(),
        ));
    }
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .map_err(|error| io("Open preferences", error))?;
    let before = file
        .metadata()
        .map_err(|error| io("Inspect preferences", error))?;
    if !before.is_file() || before.len() > MAX_BYTES {
        return Err(Error::Invalid(
            "Preferences must be a regular file no larger than 1 MiB".into(),
        ));
    }
    let revision = Revision::of(&before);
    let mut bytes = Vec::new();
    (&mut file)
        .take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| io("Read preferences", error))?;
    check(cancel)?;
    if bytes.len() as u64 > MAX_BYTES
        || Revision::of(
            &file
                .metadata()
                .map_err(|error| io("Recheck preferences", error))?,
        ) != revision
    {
        return Err(Error::Conflict);
    }
    Ok((bytes, revision))
}

pub fn decode(bytes: &[u8]) -> Result<(Preferences, bool), Error> {
    if bytes.len() as u64 > MAX_BYTES {
        return Err(Error::Invalid("Preferences exceed 1 MiB".into()));
    }
    let value = serde_json::from_slice::<serde_json::Value>(bytes)
        .map_err(|error| Error::Invalid(format!("Malformed preferences: {error}")))?;
    let version = value
        .get("version")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| Error::Invalid("Preferences need an integer version".into()))?;
    let profiles: Vec<_> = if version == 1 { value.get("profile").into_iter().collect() }
        else { value.get("profiles").and_then(|p| p.as_object()).map_or(Vec::new(), |p| p.values().collect()) };
    if version < 21 && profiles.iter().any(|profile|profile.get("midi_clock").is_some()) {
        return Err(Error::Invalid("MIDI clock output requires preferences version 21".into()));
    }
    if version < 20 && profiles.iter().any(|profile| {
        let navigation = |binding: &serde_json::Value| binding.get("action").and_then(|a|a.as_str()).is_some_and(|a|matches!(a,"SongLocator"|"SongPrevious"|"SongNext"|"SongLoop"|"SongCancel"));
        profile.get("midi_learn").and_then(|v|v.get("mappings")).and_then(|v|v.as_array()).is_some_and(|rows|rows.iter().any(|row|row.get("binding").is_some_and(navigation)))
        || profile.get("midi_presets").and_then(|v|v.as_array()).is_some_and(|rows|rows.iter().any(|row|row.get("version").and_then(|v|v.as_u64()).is_some_and(|v|v>2) || row.get("bindings").and_then(|v|v.as_array()).is_some_and(|rows|rows.iter().any(navigation))))
    }) { return Err(Error::Invalid("Song navigation requires preferences version 20".into())); }
    if version < 19 && profiles.iter().any(|profile| {
        let assignments = profile.get("midi_learn").and_then(|v| v.get("mappings")).and_then(|v| v.as_array());
        let presets = profile.get("midi_presets").and_then(|v| v.as_array());
        assignments.is_some_and(|rows| rows.iter().any(|row| row.get("binding").is_some_and(crate::engine::midi::controls::new_fields)))
            || presets.is_some_and(|rows| rows.iter().any(|row| row.get("version").and_then(|v| v.as_u64()).is_some_and(|version| version > 1)
                || row.get("bindings").and_then(|v| v.as_array()).is_some_and(|bindings| bindings.iter().any(crate::engine::midi::controls::new_fields))))
    }) { return Err(Error::Invalid("Encoder controls require preferences version 19".into())); }
    if version < 17 && profiles.iter().any(|p| p.get("midi_learn").and_then(|v| v.get("mappings")).and_then(|v| v.as_array()).is_some_and(|rows| rows.iter().any(|row|
        row.get("binding").and_then(|v| v.get("action")).and_then(|v| v.as_str()) == Some("DeckCueHold")))) {
        return Err(Error::Invalid("Held Cue assignments require preferences version 17".into()));
    }
    if version < 16 && profiles.iter().any(|p| {
        p.get("shortcuts").and_then(|v| v.as_object()).is_some_and(|v| v.keys().any(|key| key.starts_with("beat_jump_")))
            || p.get("midi_learn").and_then(|v| v.get("mappings")).and_then(|v| v.as_array()).is_some_and(|rows| rows.iter().any(|row|
                row.get("binding").and_then(|v| v.get("action")).and_then(|v| v.as_str()).is_some_and(|v| v.starts_with("DeckBeatJump"))))
    }) {
        return Err(Error::Invalid("Beat jump assignments require preferences version 16".into()));
    }
    if version < 15 && profiles.iter().any(|p|p.get("now_playing").is_some()) {
        return Err(Error::Invalid("Now-playing publication requires preferences version 15".into()));
    }
    if version < 14 && profiles.iter().any(|p| p.get("waveforms").is_some()) {
        return Err(Error::Invalid("Waveform views require preferences version 14".into()));
    }
    if version < 13 && profiles.iter().any(|p|p.get("library_layout").is_some()) {
        return Err(Error::Invalid("Library layouts require preferences version 13".into()));
    }
    if version < 12 && profiles.iter().any(|p|p.get("midi_learn").is_some()) {
        return Err(Error::Invalid("MIDI assignments require preferences version 12".into()));
    }
    if version < 11 && profiles.iter().any(|p|p.get("workspaces").is_some()) {
        return Err(Error::Invalid("Workspaces require preferences version 11".into()));
    }
    if version < 10 && profiles.iter().any(|p| p.get("appearance").is_some_and(|a| a.get("locale").is_some())) {
        return Err(Error::Invalid("Display language requires preferences version 10".into()));
    }
    if version < 9 && profiles.iter().any(|p|p.get("automation").is_some()) {
        return Err(Error::Invalid("Automation configuration requires preferences version 9; older versions cannot carry newer fields".into()));
    }
    if version < 8 && profiles.iter().any(|p|p.get("appearance").is_some_and(|a|["contrast", "reduced_motion", "waveform_contrast", "level_contrast"].iter().any(|field|a.get(field).is_some()))) {
        return Err(Error::Invalid("Display contrast and motion require preferences version 8; older versions cannot carry newer fields".into()));
    }
    if version < 7 && profiles.iter().any(|p|p.get("startup").and_then(|s|s.get("session")).is_some()) {
        return Err(Error::Invalid("Startup templates require preferences version7; older versions cannot carry newer fields".into()));
    }
    if version < 6 && profiles.iter().any(|p|p.get("midi_routing").is_some()) {
        return Err(Error::Invalid("MIDI routing requires preferences version6; an older version cannot carry newer fields".into()));
    }
    if version < 18 && profiles.iter().any(|profile| profile.get("midi_presets").is_some()) {
        return Err(Error::Invalid("MIDI presets require preferences version 18".into()));
    }
    let (mut preferences, migrated) = match version {
        21 => (
            serde_json::from_slice::<Preferences>(bytes)
                .map_err(|error| Error::Invalid(format!("Invalid preferences: {error}")))?,
            false,
        ),
        2 | 3 | 4 | 5 | 6 | 7 | 8 | 9 | 10 | 11 | 12 | 13 | 14 | 15 | 16 | 17 | 18 | 19 | 20 => {
            let mut preferences: Preferences = serde_json::from_slice(bytes).map_err(|error| {
                Error::Invalid(format!("Invalid version {version} preferences: {error}"))
            })?;
            preferences.version = VERSION;
            (preferences, true)
        }
        1 => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Single {
                version: u32,
                profile: Profile,
            }
            let old: Single = serde_json::from_slice(bytes).map_err(|error| {
                Error::Invalid(format!("Invalid version 1 preferences: {error}"))
            })?;
            debug_assert_eq!(old.version, 1);
            (
                Preferences {
                    version: VERSION,
                    active: "Studio".into(),
                    profiles: BTreeMap::from([
                        ("Studio".into(), old.profile.clone()),
                        ("Performance".into(), old.profile),
                    ]),
                },
                true,
            )
        }
        other => {
            return Err(Error::Invalid(format!(
                "Preferences version {other} is unsupported; the original file was preserved"
            )))
        }
    };
    if version < 16 { for profile in preferences.profiles.values_mut() { crate::ui::migrate_beat_jump_shortcuts(profile); } }
    preferences.validate().map_err(Error::Invalid)?;
    Ok((preferences, migrated))
}

pub fn load(path: &Path, cancel: &AtomicBool) -> Result<Loaded, Error> {
    let (bytes, revision) = read_raw(path, cancel)?;
    let (preferences, migrated) = decode(&bytes)?;
    Ok(Loaded {
        preferences,
        revision: Some(revision),
        migrated,
    })
}

#[derive(Clone, Debug)]
pub struct Saved {
    pub revision: Option<Revision>,
    pub warning: Option<String>,
}
#[derive(Clone, Debug)]
pub enum Overwrite {
    New,
    Exact(Option<Revision>),
}
fn revision(path: &Path) -> Result<Option<Revision>, Error> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.is_file() => Ok(Some(Revision::of(&meta))),
        Ok(_) => Err(Error::Invalid(
            "Destination must be a regular file; symlinks and special files are not replaced"
                .into(),
        )),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(io("Inspect destination", error)),
    }
}
struct Temp(PathBuf);
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

pub fn save(
    path: &Path,
    preferences: &Preferences,
    overwrite: Overwrite,
    cancel: &AtomicBool,
) -> Result<Saved, Error> {
    save_before_commit(path, preferences, overwrite, cancel, || {})
}

fn save_before_commit(
    path: &Path,
    preferences: &Preferences,
    overwrite: Overwrite,
    cancel: &AtomicBool,
    before_commit: impl FnOnce(),
) -> Result<Saved, Error> {
    save_with_gate(path, preferences, overwrite, cancel, before_commit, None)
}
pub(crate) fn save_protected(path: &Path, preferences: &Preferences, overwrite: Overwrite, cancel: &AtomicBool, permit: &crate::engine::performance::WorkPermit) -> Result<Saved, Error> {
    save_with_gate(path, preferences, overwrite, cancel, || {}, Some(permit))
}
fn save_with_gate(path: &Path, preferences: &Preferences, overwrite: Overwrite, cancel: &AtomicBool, before_commit: impl FnOnce(), permit: Option<&crate::engine::performance::WorkPermit>) -> Result<Saved, Error> {
    check(cancel)?;
    preferences.validate().map_err(Error::Invalid)?;
    let bytes = serde_json::to_vec_pretty(preferences)
        .map_err(|error| Error::Invalid(error.to_string()))?;
    publish_bytes(path, &bytes, overwrite, cancel, before_commit, permit)
}

/// Publish a new bounded settings export without replacing an existing file.
/// Takes an absolute path, validated bytes, cancellation and work permit; returns the committed file receipt.
pub(crate) fn publish_new(path: &Path, bytes: &[u8], cancel: &AtomicBool, permit: &crate::engine::performance::WorkPermit) -> Result<Saved, Error> {
    publish_bytes(path, bytes, Overwrite::New, cancel, || {}, Some(permit))
}

fn publish_bytes(path: &Path, bytes: &[u8], overwrite: Overwrite, cancel: &AtomicBool, before_commit: impl FnOnce(), permit: Option<&crate::engine::performance::WorkPermit>) -> Result<Saved, Error> {
    check(cancel)?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err(Error::Invalid("Preferences exceed 1 MiB".into()));
    }
    let previous = revision(path)?;
    match &overwrite {
        Overwrite::New if previous.is_some() => {
            return Err(Error::Invalid(
                "Destination exists; choose a new file or explicitly replace it".into(),
            ))
        }
        Overwrite::Exact(expected) if *expected != previous => return Err(Error::Conflict),
        _ => {}
    }
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .ok_or_else(|| Error::Invalid("Use an absolute preference file path".into()))?;
    if !path.is_absolute() {
        return Err(Error::Invalid(
            "Use an absolute preference file path".into(),
        ));
    }
    fs::create_dir_all(parent).map_err(|error| io("Create preferences directory", error))?;
    let serial = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
    let temp = Temp(parent.join(format!(
        ".omatainer-preferences-{}-{serial}.tmp",
        std::process::id()
    )));
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temp.0)
        .map_err(|error| io("Create preference staging file", error))?;
    output
        .write_all(bytes)
        .map_err(|error| io("Write preferences", error))?;
    output
        .sync_all()
        .map_err(|error| io("Synchronize preferences", error))?;
    before_commit();
    check(cancel)?;
    let _commit = permit.map(|permit| permit.commit().map_err(|error| Error::Invalid(error.to_string()))).transpose()?;
    if revision(path)? != previous {
        return Err(Error::Conflict);
    }
    let mut cleanup_failed = false;
    if previous.is_none() {
        // Creation must never clobber a destination created after our check.
        fs::hard_link(&temp.0, path).map_err(|error| io("Publish new preferences", error))?;
        cleanup_failed = fs::remove_file(&temp.0).is_err();
    } else {
        fs::rename(&temp.0, path).map_err(|error| io("Publish preferences", error))?;
    }
    // Publication has committed: late cancellation and directory-sync failure
    // must not report a failed save that would invite a misleading retry.
    let mut warning = File::open(parent)
        .and_then(|directory| directory.sync_all())
        .err()
        .map(|error| format!("Saved, but directory synchronization failed: {error}"));
    let revision = if cleanup_failed {
        warning =
            Some("Saved, but staging cleanup is incomplete; reload before saving again".into());
        None
    } else {
        match output.metadata() {
            Ok(metadata) => Some(Revision::of(&metadata)),
            Err(error) => {
                warning = Some(format!("Saved, but the saved file identity could not be checked: {error}; reload before saving again"));
                None
            }
        }
    };
    Ok(Saved { revision, warning })
}

/// Explicit recovery keeps the unreadable prior file beside the new defaults.
pub fn backup_and_reset(
    path: &Path,
    preferences: &Preferences,
    cancel: &AtomicBool,
) -> Result<Saved, Error> {
    preferences.validate().map_err(Error::Invalid)?;
    let previous = revision(path)?;
    let mut backup = None;
    if previous.is_some() {
        let (bytes, before) = read_raw(path, cancel)?;
        if previous.as_ref() != Some(&before) {
            return Err(Error::Conflict);
        }
        let name = path
            .file_name()
            .ok_or_else(|| Error::Invalid("Missing preferences filename".into()))?
            .to_string_lossy();
        let target = path.with_file_name(format!(
            "{name}.preserved-{}-{}",
            std::process::id(),
            NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
        ));
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&target)
            .map_err(|error| io("Preserve old preferences", error))?;
        output
            .write_all(&bytes)
            .and_then(|_| output.sync_all())
            .map_err(|error| io("Save preferences backup", error))?;
        backup = Some(target);
    }
    let mut saved = save(path, preferences, Overwrite::Exact(previous), cancel)?;
    if let Some(backup) = backup {
        let message = format!("Previous file preserved at {}", backup.display());
        saved.warning = Some(
            saved
                .warning
                .map_or(message.clone(), |warning| format!("{warning}; {message}")),
        );
    }
    Ok(saved)
}

pub fn default_path(config: Option<&std::ffi::OsStr>, home: &Path) -> PathBuf {
    config
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .unwrap_or_else(|| home.join(".config"))
        .join("omatainer/preferences.json")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn version_eight_migrates_disabled_osc_without_changing_existing_intent() {
        let mut original = Preferences::defaults(Path::new("/private/api-user"));
        original
            .profiles
            .get_mut("Studio")
            .unwrap()
            .audio
            .sample_rate = Some(96000);
        original
            .profiles
            .get_mut("Studio")
            .unwrap()
            .appearance
            .contrast = crate::theme::Contrast::Light;
        let mut value = serde_json::to_value(&original).unwrap();
        value["version"] = 8.into(); strip_language(&mut value);
        assert!(decode(&serde_json::to_vec(&value).unwrap())
            .unwrap_err()
            .to_string()
            .contains("Automation"));
        for profile in value["profiles"].as_object_mut().unwrap().values_mut() {
            profile.as_object_mut().unwrap().remove("automation");
        }
        let (loaded, migrated) = decode(&serde_json::to_vec(&value).unwrap()).unwrap();
        assert!(migrated);
        assert_eq!(loaded, original);
        original.profiles.get_mut("Studio").unwrap().automation = crate::automation::osc::Config {
            enabled: true,
            port: 0,
        };
        let (reopened, migrated) = decode(&serde_json::to_vec(&original).unwrap()).unwrap();
        assert!(!migrated);
        assert_eq!(reopened, original);
        original.profiles.get_mut("Studio").unwrap().automation.port = 80;
        assert!(decode(&serde_json::to_vec(&original).unwrap()).is_err());
    }
    fn strip_language(value: &mut serde_json::Value) {
        for profile in value["profiles"].as_object_mut().unwrap().values_mut() { profile.as_object_mut().unwrap().remove("workspaces");profile.as_object_mut().unwrap().remove("midi_learn");profile.as_object_mut().unwrap().remove("library_layout");profile.as_object_mut().unwrap().remove("waveforms"); profile["appearance"].as_object_mut().unwrap().remove("locale"); }
    }
    fn strip_appearance(profile: &mut serde_json::Value) {
        profile.as_object_mut().unwrap().remove("workspaces");profile.as_object_mut().unwrap().remove("midi_learn");profile.as_object_mut().unwrap().remove("library_layout");profile.as_object_mut().unwrap().remove("waveforms");
        profile.as_object_mut().unwrap().remove("automation");
        for field in ["locale", "contrast", "reduced_motion", "waveform_contrast", "level_contrast"] { profile["appearance"].as_object_mut().unwrap().remove(field); }
    }
    fn strip_display(value: &mut serde_json::Value) {
        for profile in value["profiles"].as_object_mut().unwrap().values_mut() { strip_appearance(profile); }
    }
    #[test]
    fn version_seven_migrates_display_defaults_and_rejects_new_fields() {
        let mut original = Preferences::defaults(Path::new("/private/display-user"));
        original.profiles.get_mut("Studio").unwrap().startup.session = crate::project_template::Startup::Empty;
        original.profiles.get_mut("Studio").unwrap().audio.sample_rate = Some(96000);
        let mut value = serde_json::to_value(&original).unwrap(); value["version"] = 7.into(); strip_language(&mut value);
        for profile in value["profiles"].as_object_mut().unwrap().values_mut() {profile.as_object_mut().unwrap().remove("automation");}
        assert!(decode(&serde_json::to_vec(&value).unwrap()).unwrap_err().to_string().contains("Display contrast"));
        strip_display(&mut value);
        let (loaded, migrated) = decode(&serde_json::to_vec(&value).unwrap()).unwrap();
        assert!(migrated); assert_eq!(loaded, original);
        let display = &mut original.profiles.get_mut("Studio").unwrap().appearance;
        display.contrast = crate::theme::Contrast::Light; display.reduced_motion = true;
        display.waveform_contrast = 2.5; display.level_contrast = 3.0;
        let (reopened, migrated) = decode(&serde_json::to_vec(&original).unwrap()).unwrap();
        assert!(!migrated); assert_eq!(reopened, original);
        for field in ["waveform_contrast", "level_contrast"] {
            for invalid in [serde_json::json!(0.99),serde_json::json!(3.01),serde_json::Value::Null] {
                let mut value = serde_json::to_value(&original).unwrap(); value["profiles"]["Studio"]["appearance"][field] = invalid;
                assert!(decode(&serde_json::to_vec(&value).unwrap()).is_err());
            }
        }
    }
    struct Directory(PathBuf);
    impl Directory {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "omatainer-preferences-{}-{}",
                std::process::id(),
                NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
        fn file(&self) -> PathBuf {
            self.0.join("preferences.json")
        }
    }
    impl Drop for Directory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn version_six_migrates_without_changing_startup_or_audio_and_rejects_new_fields() {
        let original = Preferences::defaults(Path::new("/private/template-user"));
        let mut value = serde_json::to_value(&original).unwrap(); value["version"] = 6.into(); strip_language(&mut value);
        strip_display(&mut value);
        assert!(decode(&serde_json::to_vec(&value).unwrap()).is_err());
        for profile in value["profiles"].as_object_mut().unwrap().values_mut() { profile["startup"].as_object_mut().unwrap().remove("session"); }
        let (loaded, migrated) = decode(&serde_json::to_vec(&value).unwrap()).unwrap();
        assert!(migrated); assert_eq!(loaded, original);
        let mut changed = loaded; changed.profiles.get_mut("Studio").unwrap().startup.session = crate::project_template::Startup::Empty;
        let (reopened, migrated) = decode(&serde_json::to_vec(&changed).unwrap()).unwrap();
        assert!(!migrated); assert_eq!(reopened, changed);
    }

    #[test]
    fn version_four_migration_adds_recovery_limits_without_rewriting_audio_or_performance() {
        let dir = Directory::new();
        let mut original = Preferences::defaults(&dir.0);
        original.profiles.get_mut("Studio").unwrap().audio.sample_rate = Some(96000);
        original.profiles.get_mut("Studio").unwrap().startup.performance_mode = true;
        let mut json = serde_json::to_value(&original).unwrap();
        json["version"] = 4.into();
        strip_display(&mut json);
        for profile in json["profiles"].as_object_mut().unwrap().values_mut() {
            profile.as_object_mut().unwrap().remove("midi_routing");
            profile.as_object_mut().unwrap().remove("recovery");
            profile["startup"].as_object_mut().unwrap().remove("session");
        }
        let bytes = serde_json::to_vec(&json).unwrap();
        fs::write(dir.file(), &bytes).unwrap();
        let loaded = load(&dir.file(), &AtomicBool::new(false)).unwrap();
        assert!(loaded.migrated);
        assert_eq!(loaded.preferences, original);
        assert_eq!(fs::read(dir.file()).unwrap(), bytes);
        for limit in [0_u64, 63 * crate::recovery::MIB, 17 * 1024 * crate::recovery::MIB] {
            let mut invalid = original.clone();
            invalid.profiles.get_mut("Studio").unwrap().recovery.max_bytes = limit;
            assert!(decode(&serde_json::to_vec(&invalid).unwrap()).is_err());
        }
    }

    #[test]
    fn atomic_roundtrip_defaults_profiles_and_conflict_preserve_exact_values() {
        let dir = Directory::new();
        let cancel = AtomicBool::new(false);
        let mut prefs = Preferences::defaults(Path::new("/private/user"));
        prefs
            .profiles
            .get_mut("Performance")
            .unwrap()
            .appearance
            .scale = 1.25;
        prefs.active = "Performance".into();
        let saved = save(&dir.file(), &prefs, Overwrite::New, &cancel).unwrap();
        let loaded = load(&dir.file(), &cancel).unwrap();
        assert_eq!(loaded.preferences, prefs);
        assert_eq!(loaded.revision, saved.revision);
        let original = fs::read(dir.file()).unwrap();
        assert!(save(&dir.file(), &prefs, Overwrite::New, &cancel).is_err());
        assert!(matches!(
            save(&dir.file(), &prefs, Overwrite::Exact(None), &cancel),
            Err(Error::Conflict)
        ));
        assert_eq!(fs::read(dir.file()).unwrap(), original);
        prefs.active = "Studio".into();
        save(
            &dir.file(),
            &prefs,
            Overwrite::Exact(saved.revision),
            &cancel,
        )
        .unwrap();
        assert_eq!(load(&dir.file(), &cancel).unwrap().preferences, prefs);
        assert_eq!(fs::metadata(dir.file()).unwrap().mode() & 0o777, 0o600);
    }
    #[test]
    fn version_two_audio_migrates_and_version_three_retains_exact_input_output_settings() {
        let prefs = Preferences::defaults(Path::new("/private/user"));
        let mut value = serde_json::to_value(&prefs).unwrap();
        value["version"] = 2.into(); strip_language(&mut value);
        strip_display(&mut value);
        for profile in value["profiles"].as_object_mut().unwrap().values_mut() {
            profile.as_object_mut().unwrap().remove("midi_routing");
            profile["startup"].as_object_mut().unwrap().remove("session");
            for field in ["backend", "format", "calibration"] {
                profile["audio"].as_object_mut().unwrap().remove(field);
            }
        }
        let (mut migrated, changed) = decode(&serde_json::to_vec(&value).unwrap()).unwrap();
        assert!(changed);
        assert_eq!(migrated, prefs);
        let audio = &mut migrated.profiles.get_mut("Studio").unwrap().audio;
        audio.backend = Some("ALSA".into());
        audio.device = Some("External USB".into());
        audio.format = Some(AudioFormat::I32);
        audio.sample_rate = Some(192000);
        audio.buffer_frames = Some(64);
        audio.calibration.device = Some("Loopback input".into());
        audio.calibration.format = Some(AudioFormat::F32);
        audio.calibration.channel = 3;
        audio.calibration.channels = Some(4);
        audio.calibration.output_channel = 1;
        audio.calibration.level_db = -48.0;
        let dir = Directory::new();
        save(
            &dir.file(),
            &migrated,
            Overwrite::New,
            &AtomicBool::new(false),
        )
        .unwrap();
        let read = load(&dir.file(), &AtomicBool::new(false)).unwrap();
        assert_eq!(read.preferences, migrated);
        assert!(!read.migrated);
        let mut invalid = serde_json::to_value(&migrated).unwrap();
        invalid["profiles"]["Studio"]["audio"]["format"] = "packed-24".into();
        assert!(decode(&serde_json::to_vec(&invalid).unwrap()).is_err());
    }
    #[test]
    fn migration_is_explicit_and_unknown_or_duplicate_fields_never_disappear() {
        let profile = Profile::defaults(Path::new("/private/user"));
        let mut old_profile = serde_json::to_value(&profile).unwrap();
        strip_appearance(&mut old_profile);
        old_profile.as_object_mut().unwrap().remove("midi_routing");
        old_profile["startup"].as_object_mut().unwrap().remove("session");
        let raw = serde_json::to_vec(&serde_json::json!({"version":1,"profile":old_profile})).unwrap();
        let (loaded, migrated) = decode(&raw).unwrap();
        assert!(migrated);
        assert_eq!(loaded.current(), Some(&profile));
        assert_eq!(loaded.profiles["Performance"], profile);
        assert!(decode(br#"{"version":999,"secret":"preserve original"}"#).is_err());
        let value = serde_json::to_string(&old_profile).unwrap();
        let duplicate = format!(
            r#"{{"version":2,"active":"Studio","profiles":{{"Studio":{value},"Studio":{value}}}}}"#
        );
        assert!(decode(duplicate.as_bytes()).is_err());
        let mut unknown = serde_json::to_value(loaded).unwrap();
        unknown["credentials"] = "do not export".into();
        assert!(decode(&serde_json::to_vec(&unknown).unwrap()).is_err());
    }
    #[test]
    fn delayed_cancellation_and_external_changes_leave_the_destination_untouched() {
        let dir = Directory::new();
        let cancel = AtomicBool::new(false);
        let prefs = Preferences::defaults(Path::new("/private/user"));
        let saved = save(&dir.file(), &prefs, Overwrite::New, &cancel).unwrap();
        let original = fs::read(dir.file()).unwrap();
        assert!(matches!(
            save_before_commit(
                &dir.file(),
                &prefs,
                Overwrite::Exact(saved.revision.clone()),
                &cancel,
                || cancel.store(true, Ordering::Release)
            ),
            Err(Error::Cancelled)
        ));
        assert_eq!(fs::read(dir.file()).unwrap(), original);
        cancel.store(false, Ordering::Release);
        assert!(matches!(
            save_before_commit(
                &dir.file(),
                &prefs,
                Overwrite::Exact(saved.revision),
                &cancel,
                || fs::write(dir.file(), b"external edit").unwrap()
            ),
            Err(Error::Conflict)
        ));
        assert_eq!(fs::read(dir.file()).unwrap(), b"external edit");
        assert_eq!(
            fs::read_dir(&dir.0).unwrap().count(),
            1,
            "staging files retired"
        );
    }
    #[test]
    fn special_and_oversized_files_fail_without_following_or_blocking() {
        use std::os::unix::fs::symlink;
        let dir = Directory::new();
        let cancel = AtomicBool::new(false);
        let target = dir.0.join("original");
        fs::write(&target, b"keep").unwrap();
        symlink(&target, dir.file()).unwrap();
        assert!(load(&dir.file(), &cancel).is_err());
        assert!(save(
            &dir.file(),
            &Preferences::defaults(Path::new("/private/user")),
            Overwrite::New,
            &cancel
        )
        .is_err());
        assert_eq!(fs::read(target).unwrap(), b"keep");
        fs::remove_file(dir.file()).unwrap();
        let path = std::ffi::CString::new(dir.file().as_os_str().as_encoded_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(path.as_ptr(), 0o600) }, 0);
        assert!(load(&dir.file(), &cancel).is_err());
        fs::remove_file(dir.file()).unwrap();
        File::create(dir.file())
            .unwrap()
            .set_len(MAX_BYTES + 1)
            .unwrap();
        assert!(load(&dir.file(), &cancel).is_err());
    }
    #[test]
    fn performance_publication_race_preserves_prior_file_and_startup_flag_roundtrips() {
        let dir = Directory::new();
        let path = dir.0.join("preferences.json");
        let preferences = Preferences::defaults(&dir.0);
        let original = save(&path, &preferences, Overwrite::New, &AtomicBool::new(false)).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        let performance = crate::engine::performance::Handle::default();
        let permit = performance.optional_work().unwrap();
        let mut changed = preferences.clone();
        changed.profiles.get_mut("Studio").unwrap().startup.performance_mode = true;
        let result = save_with_gate(&path, &changed, Overwrite::Exact(original.revision), &permit.cancel(), || performance.set_enabled(true).unwrap(), Some(&permit));
        assert!(result.is_err());
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        performance.set_enabled(false).unwrap();
        let saved = save(&path, &changed, Overwrite::Exact(revision(&path).unwrap()), &AtomicBool::new(false)).unwrap();
        assert!(saved.revision.is_some());
        assert!(load(&path, &AtomicBool::new(false)).unwrap().preferences.current().unwrap().startup.performance_mode);
        let mut legacy = serde_json::to_value(preferences).unwrap();
        legacy["version"] = 3.into();
        strip_display(&mut legacy);
        for profile in legacy["profiles"].as_object_mut().unwrap().values_mut() { profile.as_object_mut().unwrap().remove("midi_routing"); profile["startup"].as_object_mut().unwrap().remove("performance_mode"); profile["startup"].as_object_mut().unwrap().remove("session"); }
        let (legacy, migrated) = decode(&serde_json::to_vec(&legacy).unwrap()).unwrap();
        assert!(migrated);
        assert_eq!(legacy.version, VERSION);
        assert!(legacy.profiles.values().all(|profile| !profile.startup.performance_mode));
    }

    #[test]
    fn version_nine_migrates_english_and_unknown_language_never_rewrites_preferences() {
        let original = Preferences::defaults(Path::new("/private/international"));
        let mut value=serde_json::to_value(&original).unwrap();value["version"]=9.into();strip_language(&mut value);
        let (loaded,migrated)=decode(&serde_json::to_vec(&value).unwrap()).unwrap();assert!(migrated);assert_eq!(loaded,original);
        let mut invalid=serde_json::to_value(&original).unwrap();invalid["profiles"]["Studio"]["appearance"]["locale"]="unsupported".into();
        assert!(decode(&serde_json::to_vec(&invalid).unwrap()).is_err());
        value["profiles"]["Studio"]["appearance"]["locale"]="spanish".into();assert!(decode(&serde_json::to_vec(&value).unwrap()).is_err());
    }

}


#[cfg(test)]
mod workspace_migration_tests {
    use super::*;
    #[test]
    fn version_ten_migrates_to_default_layouts_and_cannot_smuggle_new_workspace_fields() {
        let current=Preferences::defaults(Path::new("/tmp"));
        let mut legacy=serde_json::to_value(&current).unwrap();
        legacy["version"]=10.into();
        for profile in legacy["profiles"].as_object_mut().unwrap().values_mut() {
            profile.as_object_mut().unwrap().remove("workspaces");profile.as_object_mut().unwrap().remove("midi_learn");profile.as_object_mut().unwrap().remove("library_layout");profile.as_object_mut().unwrap().remove("waveforms");
        }
        let (migrated,changed)=decode(&serde_json::to_vec(&legacy).unwrap()).unwrap();
        assert!(changed); assert_eq!(migrated,current);
        legacy["profiles"]["Studio"]["workspaces"]=serde_json::to_value(&current.current().unwrap().workspaces).unwrap();
        assert!(decode(&serde_json::to_vec(&legacy).unwrap()).is_err());
        let mut invalid=serde_json::to_value(&current).unwrap();
        invalid["profiles"]["Studio"]["workspaces"]["active"]="missing".into();
        assert!(decode(&serde_json::to_vec(&invalid).unwrap()).is_err());
    }
}


#[cfg(test)]
mod midi_learn_migration_tests {
    use super::*;
    #[test]
    fn version_eleven_migrates_empty_assignments_and_cannot_smuggle_new_bindings() {
        let current=Preferences::defaults(Path::new("/tmp"));let mut old=serde_json::to_value(&current).unwrap();old["version"]=11.into();
        for profile in old["profiles"].as_object_mut().unwrap().values_mut(){profile.as_object_mut().unwrap().remove("midi_learn");profile.as_object_mut().unwrap().remove("library_layout");profile.as_object_mut().unwrap().remove("waveforms");}
        let (migrated,changed)=decode(&serde_json::to_vec(&old).unwrap()).unwrap();assert!(changed);assert_eq!(migrated,current);
        old["profiles"]["Studio"]["midi_learn"]=serde_json::to_value(&current.current().unwrap().midi_learn).unwrap();assert!(decode(&serde_json::to_vec(&old).unwrap()).is_err());
        old["version"]=(VERSION+1).into();assert!(decode(&serde_json::to_vec(&old).unwrap()).is_err());
    }
}

#[cfg(test)]
mod now_playing_migration_tests {
    use super::*;
    #[test]
    fn version_fourteen_migrates_feed_disabled_and_cannot_carry_new_publication_intent() {
        let current=Preferences::defaults(Path::new("/home/test"));let mut legacy=serde_json::to_value(&current).unwrap();legacy["version"]=14.into();
        let (loaded,migrated)=decode(&serde_json::to_vec(&legacy).unwrap()).unwrap();assert!(migrated);assert_eq!(loaded,current);assert!(!loaded.current().unwrap().now_playing.enabled);
        legacy["profiles"]["Studio"]["now_playing"]=serde_json::json!({"enabled":true,"title":false,"artist":false,"identity":true});
        assert!(decode(&serde_json::to_vec(&legacy).unwrap()).is_err());legacy["version"]=15.into();
        let (loaded,migrated)=decode(&serde_json::to_vec(&legacy).unwrap()).unwrap();assert!(migrated);assert!(loaded.current().unwrap().now_playing.enabled);
        legacy["profiles"]["Studio"]["now_playing"]["endpoint"]="https://example.com".into();assert!(decode(&serde_json::to_vec(&legacy).unwrap()).is_err());
    }
}

#[cfg(test)]
mod beat_jump_migration_tests {
    use super::*;
    #[test]
    fn version_fifteen_preserves_owned_keys_and_refuses_new_actions_under_old_headers() {
        let current = Preferences::defaults(Path::new("/home/test"));
        let mut legacy = serde_json::to_value(&current).unwrap(); legacy["version"] = 15.into();
        legacy["profiles"]["Studio"]["shortcuts"]["play_a"] = serde_json::json!({"key":"[","ctrl":false,"shift":true,"alt":false});
        let (migrated, changed) = decode(&serde_json::to_vec(&legacy).unwrap()).unwrap();
        assert!(changed); assert_eq!(migrated.version, VERSION);
        assert_eq!(migrated.profiles["Studio"].shortcuts["play_a"], Some(Shortcut { key: "[".into(), ctrl: false, shift: true, alt: false }));
        assert_eq!(migrated.profiles["Studio"].shortcuts["beat_jump_back"], None);
        assert!(!migrated.profiles["Studio"].shortcuts.contains_key("beat_jump_forward"));
        assert_eq!(decode(&serde_json::to_vec(&migrated).unwrap()).unwrap(), (migrated, false));
        for key in ["beat_jump_back", "beat_jump_forward", "beat_jump_smaller", "beat_jump_larger"] {
            let mut invalid = legacy.clone(); invalid["profiles"]["Studio"]["shortcuts"][key] = serde_json::Value::Null;
            assert!(decode(&serde_json::to_vec(&invalid).unwrap()).is_err());
        }
        for action in ["DeckBeatJumpBack", "DeckBeatJumpForward", "DeckBeatJumpSmaller", "DeckBeatJumpLarger"] {
            let mut invalid = legacy.clone();
            invalid["profiles"]["Studio"]["midi_learn"] = serde_json::json!({"mappings":[{"endpoint":{"name":"Test","id":"port"},"binding":{"kind":"Note","ch":0,"data":60,"action":action,"deck":0,"extra":0,"relative":null}}]});
            assert!(decode(&serde_json::to_vec(&invalid).unwrap()).is_err());
        }
    }
}

#[cfg(test)]
mod cue_audition_migration_tests {
    use super::*;
    #[test]
    fn version_sixteen_preserves_custom_jump_keys_and_one_shot_cue_while_requiring_new_hold_headers() {
        let mut old=Preferences::defaults(Path::new("/home/test")); old.version=16;
        let profile=old.profiles.get_mut("Studio").unwrap();
        profile.shortcuts.insert("play_a".into(),Some(Shortcut {key:"[".into(),ctrl:false,shift:true,alt:false}));
        profile.shortcuts.insert("beat_jump_back".into(),Some(Shortcut {key:"J".into(),ctrl:false,shift:false,alt:false}));
        profile.midi_learn.mappings.push(crate::engine::midi::learn::Mapping {endpoint:crate::engine::midi::learn::Endpoint {name:"Test".into(),id:"port".into()},binding:crate::engine::midi::Binding {kind:crate::engine::midi::MsgKind::Note,ch:0,data:60,action:crate::engine::midi::Action::DeckCue,deck:1,extra:0,relative:None, controls: None,
         pair_order: None,
        }});
        let (loaded,migrated)=decode(&serde_json::to_vec(&old).unwrap()).unwrap(); assert!(migrated);
        old.version=VERSION; assert_eq!(loaded,old);
        let mut newer=loaded.clone(); newer.profiles.get_mut("Studio").unwrap().midi_learn.mappings[0].binding.action=crate::engine::midi::Action::DeckCueHold;
        assert_eq!(decode(&serde_json::to_vec(&newer).unwrap()).unwrap(),(newer.clone(),false));
        newer.version=16; assert!(decode(&serde_json::to_vec(&newer).unwrap()).is_err());
        let mut too_new=loaded; too_new.version=VERSION+1; assert!(decode(&serde_json::to_vec(&too_new).unwrap()).is_err());
    }
}
