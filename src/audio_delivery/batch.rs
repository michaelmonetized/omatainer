use super::{finishing::Config, Export, Outcome};
use crate::engine::{
    audio::routing::selection::{Policy, Selection, Target},
    performance::WorkPermit,
    project::Captured,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    io::Write,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
pub(crate) mod cli;
#[cfg(test)]
mod tests;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Kind {
    #[default]
    Stem,
    Version,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Item {
    pub id: u64,
    pub name: String,
    pub kind: Kind,
    pub selection: Selection,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Recipe {
    pub schema: u32,
    pub namespace: [u64; 2],
    pub next_id: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_sha256: Option<String>,
    pub export: Export,
    pub config: Config,
    pub items: Vec<Item>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Review {
    pub revision: u64,
    pub generation: u64,
    pub recipe: Recipe,
}
#[derive(Clone, Debug, Default)]
pub(crate) struct Status {
    pub item: usize,
    pub total: usize,
    pub name: String,
    pub phase: String,
}
pub(crate) type Progress = Arc<Mutex<Status>>;
#[derive(Debug)]
pub(crate) struct BatchOutcome {
    pub folder: PathBuf,
    pub files: usize,
    pub frames: u64,
    pub warning: Option<String>,
}

/// Duplicate immutable captured values for a fresh render graph.
/// Args: one owned review capture. Returns shared immutable media and cloned saved state, with unchanged revision/checkpoint and processor receipts.
fn copy(c: &Captured) -> Captured {
    Captured {
        checkpoint: c.checkpoint,
        state: c.state.clone(),
        media: c.media.clone(),
        revision: c.revision,
        playback_receipts: c.playback_receipts.clone(),
    }
}
impl Recipe {
    /// Enumerate real saved contribution targets.
    /// Args: immutable capture. Returns namespaced track/group/named-return/fixed-return identities without guessing vocal roles.
    pub fn catalog(c: &Captured) -> Result<Vec<(Target, String)>, String> {
        let layout = c
            .state
            .session
            .as_ref()
            .ok_or("Batch delivery requires saved project identities")?;
        let mut out = layout
            .tracks
            .iter()
            .filter(|t| t.active)
            .map(|t| (Target::Track(t.id), format!("Track · {}", t.name)))
            .chain(
                layout
                    .scenes
                    .iter()
                    .filter(|s| s.active)
                    .map(|s| (Target::Scene(s.id), format!("Group · {}", s.name))),
            )
            .collect::<Vec<_>>();
        if let Some(model) = &c.state.routing {
            out.extend(
                model
                    .buses
                    .iter()
                    .map(|b| (Target::Return(b.id), format!("Return · {}", b.alias))),
            );
        }
        out.extend([
            (Target::FixedReturn(0), "Return · Reverb".into()),
            (Target::FixedReturn(1), "Return · Echo".into()),
        ]);
        Ok(out)
    }
    /// Create professional aligned group and return passes.
    /// Args: one capture, common export range and metadata. Returns explicit stable pass IDs, fresh DSP per pass, no independent normalization/dither, and default post-device boundaries.
    pub fn aligned(c: &Captured, mut export: Export, config: Config) -> Result<Self, String> {
        let layout = c
            .state
            .session
            .as_ref()
            .ok_or("Batch delivery requires project identities")?;
        export.options.normalize = false;
        export.options.dither = false;
        if export.options.format == super::Format::Mp3 {
            export.options.format = super::Format::Rf64Float32;
        }
        let mut items = Vec::new();
        for (target, label) in Self::catalog(c)? {
            if matches!(target, Target::Track(_)) {
                continue;
            }
            let id = items.len() as u64 + 1;
            items.push(Item {
                id,
                name: format!("{id:03}-{}", filename(&label)),
                kind: Kind::Stem,
                selection: Selection {
                    namespace: layout.namespace,
                    targets: vec![target],
                    policy: Policy::PostDevice,
                },
            });
        }
        Ok(Self {
            schema: 1,
            project_sha256: None,
            namespace: layout.namespace,
            next_id: items.len() as u64 + 1,
            export,
            config,
            items,
        })
    }
    /// Validate and bind saved pass identities to this exact captured revision.
    /// Args: immutable capture. Returns a review that rejects stale captures and overlapping/removed/ambiguous routes before final publication.
    pub fn review(&self, c: &Captured) -> Result<Review, String> {
        self.config.validate()?;
        self.export.frames()?;
        let layout = c
            .state
            .session
            .as_ref()
            .ok_or("Batch delivery requires project identities")?;
        if self.schema != 1
            || self.namespace != layout.namespace
            || self.items.is_empty()
            || self.items.len() > 128
        {
            return Err("Batch needs 1–128 passes from this project namespace".into());
        }
        let mut ids = BTreeSet::new();
        let mut names = BTreeSet::new();
        for item in &self.items {
            if item.id == 0
                || item.id >= self.next_id
                || !ids.insert(item.id)
                || item.name.is_empty()
                || item.name.len() > 80
                || !item
                    .name
                    .bytes()
                    .all(|v| v.is_ascii_alphanumeric() || b"-_.".contains(&v))
                || item.name == "."
                || item.name == ".."
                || ["batch", "publication-warning"]
                    .contains(&item.name.to_ascii_lowercase().as_str())
                || !names.insert(item.name.to_ascii_lowercase())
            {
                return Err(
                    "Batch pass IDs and safe file names must be unique (including case)".into(),
                );
            }
            if item.selection.namespace != self.namespace {
                return Err("Batch selection belongs to a different project".into());
            }
            item.selection.plan(c, self.export.output_alias)?;
        }
        super::output_contract(&c.state, &self.export)?;
        Ok(Review {
            revision: c.revision,
            generation: layout.generation,
            recipe: self.clone(),
        })
    }
    /// Save a reusable reviewed recipe without replacing a file.
    /// Args: a new absolute JSON path. Returns after durable file write; saved IDs are revalidated against a fresh capture when reopened.
    pub fn save(&self, path: &Path) -> Result<(), String> {
        if !path.is_absolute() {
            return Err("Choose an absolute new batch recipe path".into());
        }
        let bytes = serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?;
        let mut f = super::create(path)?;
        f.write_all(&bytes)
            .and_then(|_| f.sync_all())
            .map_err(|e| e.to_string())
    }
    /// Reopen a bounded native batch recipe.
    /// Args: existing JSON path. Returns authored IDs and choices; caller must review a fresh capture before rendering.
    pub fn load(path: &Path, cancel: &std::sync::atomic::AtomicBool) -> Result<Self, String> {
        let bytes = super::process::read_regular(path, 1024 * 1024, cancel)?;
        let recipe = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
        super::active(cancel)?;
        Ok(recipe)
    }
}
fn filename(s: &str) -> String {
    let out: String = s
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .take(65)
        .collect();
    out.trim_matches('-').to_owned()
}
fn update(progress: &Progress, item: usize, total: usize, name: &str, phase: &str) {
    *progress.lock().unwrap() = Status {
        item,
        total,
        name: name.into(),
        phase: phase.into(),
    };
}

/// Render all reviewed passes from one immutable capture and publish one complete folder.
/// Args: capture, revision-bound review, new destination, admitted work and measured progress. Returns only after all encoded files/metadata/QC succeed; failures retain a small per-pass receipt outside the absent final folder.
pub(crate) fn run(
    captured: Captured,
    review: &Review,
    destination: &Path,
    permit: &WorkPermit,
    reporter: &crate::background::Reporter,
    progress: &Progress,
) -> Result<BatchOutcome, String> {
    let cancel = permit.cancel();
    super::active(&cancel)?;
    let fresh = review.recipe.review(&captured)?;
    if fresh.revision != review.revision || fresh.generation != review.generation {
        return Err("Project changed since batch review; refresh before rendering".into());
    }
    if !destination.is_absolute() || destination.as_os_str().len() > 4096 || destination.exists() {
        return Err("Choose a new absolute batch folder; existing files are preserved".into());
    }
    let parent = destination
        .parent()
        .ok_or("Batch folder needs an existing parent")?;
    super::measurement::preflight(&cancel)?;
    let stage = crate::portable_project::Stage::new(parent)?;
    space(&stage.path, &review.recipe)?;
    let recipe = &review.recipe;
    let bounds = recipe.export.frames()?;
    let mut cases=recipe.items.iter().map(|i|serde_json::json!({"id":i.id,"name":i.name,"kind":i.kind,"selection":i.selection,"status":"skipped"})).collect::<Vec<_>>();
    let mut warnings = Vec::new();
    let mut manifest = serde_json::json!({"schema":1,"review":review,"range_start_frame":bounds.0,"body_frames":bounds.1,"tail_frames":bounds.2,"frames_per_pass":bounds.3,"alignment":"same immutable capture/revision, source origin, range, sample rate, output channel order, repeats and tail; fresh graph per pass; original PDC trimmed once","semantics":"Track post-device before shared scene FX; Group includes shared scene FX; named/fixed Return carries its actual wet tap. All feeders/sidechains continue. Dry uses pre-FX taps. MasteredSolo includes normal master/limiter and is non-exact-sum. No inferred vocal separation. Independent normalization, dither, lossy formats and nonlinear shared/master processing cannot promise null-sum.","status":"running"});
    let result = (|| {
        for (index, item) in recipe.items.iter().enumerate() {
            update(
                progress,
                index + 1,
                recipe.items.len(),
                &item.name,
                "Rendering and measuring final file",
            );
            cases[index]["status"] = serde_json::json!("running");
            let folder = stage.path.join(format!("pass-{}", item.id));
            let result = super::run_selected(
                copy(&captured),
                &recipe.export,
                Some(&item.selection),
                &recipe.config,
                &folder,
                permit,
                reporter,
            );
            let Outcome { warning, .. } = match result {
                Ok(v) => v,
                Err(e) => {
                    cases[index]["status"] = serde_json::json!("failed");
                    cases[index]["error"] = serde_json::json!(&e);
                    return Err(e);
                }
            };
            if let Some(w) = warning {
                warnings.push(format!("{}: {w}", item.name));
            }
            let output = format!("{}.{}", item.name, recipe.export.options.format.extension());
            std::fs::rename(
                folder.join(format!(
                    "master.{}",
                    recipe.export.options.format.extension()
                )),
                stage.path.join(&output),
            )
            .map_err(|e| e.to_string())?;
            let metadata = format!("{}.json", item.name);
            std::fs::rename(folder.join("export.json"), stage.path.join(&metadata))
                .map_err(|e| e.to_string())?;
            if folder.join("publication-warning.json").exists() {
                std::fs::remove_file(folder.join("publication-warning.json"))
                    .map_err(|e| e.to_string())?;
            }
            std::fs::remove_dir(&folder).map_err(|e| e.to_string())?;
            cases[index]["status"] = serde_json::json!("passed");
            cases[index]["file"] = serde_json::json!(output);
            cases[index]["metadata"] = serde_json::json!(metadata);
        }
        super::active(&cancel)?;
        manifest["status"] = serde_json::json!("passed");
        manifest["cases"] = serde_json::json!(&cases);
        manifest["warnings"] = serde_json::json!(&warnings);
        let mut f = super::create(&stage.path.join("batch.json"))?;
        f.write_all(&serde_json::to_vec_pretty(&manifest).map_err(|e| e.to_string())?)
            .and_then(|_| f.sync_all())
            .map_err(|e| e.to_string())?;
        update(
            progress,
            recipe.items.len(),
            recipe.items.len(),
            "Batch",
            "Publishing complete folder",
        );
        let _commit = permit.commit().map_err(|e| e.to_string())?;
        let warning = super::publication::retain(
            destination,
            super::publication::warning(crate::portable_project::publish(
                stage,
                destination,
                &cancel,
            )?),
        );
        if let Some(w) = warning {
            warnings.push(w);
        }
        Ok(())
    })();
    match result {
        Ok(()) => {
            update(
                progress,
                recipe.items.len(),
                recipe.items.len(),
                "Batch",
                "Saved",
            );
            Ok(BatchOutcome {
                folder: destination.to_owned(),
                files: recipe.items.len(),
                frames: bounds.3,
                warning: (!warnings.is_empty()).then(|| warnings.join("; ")),
            })
        }
        Err(e) => {
            update(
                progress,
                0,
                recipe.items.len(),
                "Batch",
                "Cancelled or failed; no final folder published",
            );
            for case in &mut cases {
                if case["status"] == "running" {
                    case["status"] = serde_json::json!("failed");
                    case["error"] = serde_json::json!(&e);
                }
            }
            manifest["status"] = serde_json::json!("failed");
            manifest["error"] = serde_json::json!(&e);
            manifest["cases"] = serde_json::json!(cases);
            let name = format!(
                ".delivery-failure-{}.json",
                crate::sampler_bank::BankId::new().map_err(|e| e.to_string())?
            );
            let receipt = parent.join(name);
            let retained = (|| {
                let mut f = super::create(&receipt)?;
                f.write_all(&serde_json::to_vec_pretty(&manifest).map_err(|e| e.to_string())?)
                    .and_then(|_| f.sync_all())
                    .map_err(|e| e.to_string())
            })();
            match retained {
                Ok(()) => Err(format!("{e}. Per-pass receipt: {}", receipt.display())),
                Err(f) => Err(format!("{e}. Could not retain per-pass receipt: {f}")),
            }
        }
    }
}

fn space(path: &Path, recipe: &Recipe) -> Result<(), String> {
    let bounds = recipe.export.frames()?;
    let width = u64::from(recipe.export.options.channels);
    let per = bounds
        .3
        .checked_mul(width * recipe.export.options.format.encoding().bytes() as u64)
        .ok_or("Batch size overflow")?;
    let needed = per
        .checked_mul(recipe.items.len() as u64 + 2)
        .and_then(|v| v.checked_add((bounds.1 + bounds.2) * width * 4))
        .and_then(|v| v.checked_add(32 * 1024 * 1024))
        .ok_or("Batch size overflow")?;
    let path =
        std::ffi::CString::new(path.as_os_str().as_encoded_bytes()).map_err(|e| e.to_string())?;
    let mut disk = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    if unsafe { libc::statvfs(path.as_ptr(), disk.as_mut_ptr()) } != 0 {
        return Err("Cannot inspect batch disk space".into());
    }
    let disk = unsafe { disk.assume_init() };
    let available = (disk.f_bavail as u64).saturating_mul(disk.f_frsize as u64);
    if needed > available {
        return Err(format!(
            "Batch needs approximately{}MiB;{}MiB available",
            needed / 1048576,
            available / 1048576
        ));
    }
    Ok(())
}
