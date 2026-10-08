use super::*;
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    os::unix::fs::OpenOptionsExt,
    time::Duration,
};
pub(super) fn regular(path: &Path, limit: usize) -> Result<Vec<u8>, String> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .map_err(|e| e.to_string())?;
    if !file.metadata().map_err(|e| e.to_string())?.is_file() {
        return Err("Controller cache must contain regular files".into());
    }
    let mut bytes = Vec::new();
    file.take((limit + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > limit {
        return Err("Controller cache exceeds its size bound".into());
    }
    Ok(bytes)
}
pub(super) fn atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = path.parent().ok_or("Controller cache has no parent")?;
    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let temporary = parent.join(format!(
        ".controller-{}-{}",
        std::process::id(),
        super::super::next_source_id()
    ));
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(&temporary)
        .map_err(|e| e.to_string())?;
    let result = (|| {
        file.write_all(bytes).map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())?;
        std::fs::rename(&temporary, path).map_err(|e| e.to_string())?;
        File::open(parent)
            .and_then(|f| f.sync_all())
            .map_err(|e| e.to_string())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(temporary);
    }
    result
}
#[derive(Clone)]
pub(crate) struct Acquired {
    pub catalog: Catalog,
    pub profiles: Vec<Profile>,
    pub signed: Vec<u8>,
}
impl Acquired {
    /// Read a complete authenticated cache generation.
    /// Takes its directory and accepted generation floor; returns all profiles or keeps the caller's prior generation untouched on any corrupt/missing entry.
    pub fn cached(directory: &Path, minimum: u64) -> Result<Self, String> {
        let signed = regular(&directory.join("catalog.json"), MAX_CATALOG)?;
        let catalog = Catalog::decode(&signed, minimum)?;
        let mut profiles = Vec::new();
        for entry in &catalog.profiles {
            profiles.push(
                catalog.profile(entry, &regular(&directory.join(&entry.file), MAX_PROFILE)?)?,
            );
        }
        Ok(Self {
            catalog,
            profiles,
            signed,
        })
    }
    /// Acquire and stage a complete immutable controller release.
    /// Takes signed metadata URL, private cache directory, generation floor and cancellation; verifies every profile before atomically publishing a generation directory and pointer.
    pub fn download(
        url: &str,
        directory: &Path,
        minimum: u64,
        cancel: &AtomicBool,
    ) -> Result<Self, String> {
        let signed = download(url, MAX_CATALOG, cancel)?;
        let catalog = Catalog::decode(&signed, minimum)?;
        let mut files = Vec::new();
        for entry in &catalog.profiles {
            let url = format!(
                "https://github.com/michaelmonetized/omatainer/releases/download/{}/{}",
                catalog.release, entry.file
            );
            let bytes = download(&url, entry.bytes, cancel)?;
            catalog.profile(entry, &bytes)?;
            files.push((entry.file.clone(), bytes));
        }
        Self::publish(directory, minimum, signed, files, cancel)
    }
    /// Publish complete authenticated data without exposing a partial generation.
    /// Takes private storage and acquired bytes; returns the verified immutable generation after an atomic directory/pointer publication, preserving the previous latest pointer for rollback.
    pub fn publish(
        directory: &Path,
        minimum: u64,
        signed: Vec<u8>,
        files: Vec<(String, Vec<u8>)>,
        cancel: &AtomicBool,
    ) -> Result<Self, String> {
        let catalog = Catalog::decode(&signed, minimum)?;
        if files.len() != catalog.profiles.len() {
            return Err("Incomplete controller generation".into());
        }
        let mut names = BTreeSet::new();
        for (name, bytes) in &files {
            if !names.insert(name) {
                return Err("Duplicate controller file".into());
            }
            let entry = catalog
                .profiles
                .iter()
                .find(|entry| &entry.file == name)
                .ok_or("Unlisted controller file")?;
            catalog.profile(entry, bytes)?;
        }
        if cancel.load(Ordering::Acquire) {
            return Err("Controller acquisition cancelled before cache publication".into());
        }
        if directory.exists() {
            let mut count = 0;
            for entry in std::fs::read_dir(directory).map_err(|e| e.to_string())? {
                let entry = entry.map_err(|e| e.to_string())?;
                let name = entry.file_name();
                let name = name.to_string_lossy();
                if !name.starts_with("generation-") {
                    continue;
                }
                count += 1;
                if count > 64 {
                    return Err("Controller cache retains 64 generations; review instance pins before removing old generations".into());
                }
                if !entry.file_type().map_err(|e| e.to_string())?.is_dir() {
                    return Err("Controller generation history must use real directories".into());
                }
                let prior = Catalog::decode(
                    &regular(&entry.path().join("catalog.json"), MAX_CATALOG)?,
                    0,
                )?;
                for current in &catalog.profiles {
                    if prior.profiles.iter().any(|p| {
                        p.id == current.id
                            && p.version == current.version
                            && (p.sha256 != current.sha256 || p.bytes != current.bytes)
                    }) {
                        return Err(
                            "An immutable controller profile version changed its published bytes"
                                .into(),
                        );
                    }
                }
            }
            if count == 64
                && !directory
                    .join(format!("generation-{}", catalog.generation))
                    .exists()
            {
                return Err("Controller generation cache is full; active pins and last-good generation are preserved".into());
            }
        }
        let generation = directory.join(format!("generation-{}", catalog.generation));
        if generation.exists() {
            let previous = Self::cached(&generation, minimum)?;
            if previous.signed != signed {
                return Err(
                    "Immutable controller generation conflicts with its prior cache".into(),
                );
            }
        } else {
            std::fs::create_dir_all(directory).map_err(|e| e.to_string())?;
            let candidate = directory.join(format!(
                ".candidate-{}-{}",
                std::process::id(),
                super::super::next_source_id()
            ));
            std::fs::create_dir(&candidate).map_err(|e| e.to_string())?;
            let result = (|| {
                for (name, bytes) in &files {
                    atomic(&candidate.join(name), bytes)?;
                }
                atomic(&candidate.join("catalog.json"), &signed)?;
                Self::cached(&candidate, minimum)?;
                if cancel.load(Ordering::Acquire) {
                    return Err("Controller acquisition cancelled before generation commit".into());
                }
                std::fs::rename(&candidate, &generation).map_err(|e| e.to_string())?;
                File::open(directory)
                    .and_then(|f| f.sync_all())
                    .map_err(|e| e.to_string())
            })();
            if result.is_err() {
                let _ = std::fs::remove_dir_all(&candidate);
            }
            result?;
        }
        let acquired = Self::cached(&generation, minimum)?;
        let latest = directory.join("latest.json");
        if latest.exists() {
            let previous = regular(&latest, MAX_CATALOG)?;
            let previous_catalog = Catalog::decode(&previous, 0)?;
            if previous_catalog.generation > catalog.generation {
                return Err("Controller latest pointer would roll back accepted metadata".into());
            }
            if previous_catalog.generation != catalog.generation {
                atomic(&directory.join("last-good.json"), &previous)?;
            }
        }
        atomic(&latest, &signed)?;
        Ok(acquired)
    }
}
fn download(url: &str, limit: usize, cancel: &AtomicBool) -> Result<Vec<u8>, String> {
    if cancel.load(Ordering::Acquire) {
        return Err("Controller acquisition cancelled".into());
    }
    let parsed = url::Url::parse(url).map_err(|e| e.to_string())?;
    if parsed.scheme() != "https"
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.port().is_some()
        || parsed.fragment().is_some()
        || parsed.query().is_some()
        || !matches!(
            parsed.host_str(),
            Some("github.com" | "raw.githubusercontent.com")
        )
        || !(parsed
            .path()
            .starts_with("/michaelmonetized/omatainer/releases/download/controller-profiles-")
            || parsed
                .path()
                .starts_with("/michaelmonetized/omatainer/stack/app-completion/profiles/"))
    {
        return Err(
            "Controller acquisition requires the pinned HTTPS repository/release origin".into(),
        );
    }
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .https_only(true)
        .timeout_global(Some(Duration::from_secs(10)))
        .max_redirects(3)
        .build()
        .into();
    let mut response = agent
        .get(url)
        .call()
        .map_err(|e| format!("Controller download failed: {e}"))?;
    let mut bytes = Vec::new();
    let mut reader = response.body_mut().as_reader();
    let mut buffer = [0; 8192];
    loop {
        if cancel.load(Ordering::Acquire) {
            return Err("Controller acquisition cancelled".into());
        }
        let n = reader.read(&mut buffer).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        if bytes.len() + n > limit {
            return Err("Controller download exceeds declared bytes".into());
        }
        bytes.extend_from_slice(&buffer[..n]);
    }
    Ok(bytes)
}
