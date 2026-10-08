//! Isolated migration reads and source verification over private native containers.
use super::*;
use crate::{
    filesystem_worker,
    project_file::{self, Bundle, Limits},
};
use std::{
    fs::File,
    io::{Seek, SeekFrom},
    os::unix::io::{AsRawFd, FromRawFd},
    process::Command,
    time::Duration,
};

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) enum Edit {
    Render {
        path: PathBuf,
        options: renders::Options,
    },
    Restore,
    Relink {
        source: usize,
        device: usize,
        binary: crate::plugin_host::BinaryIdentity,
        class: crate::plugin_host::Class,
        reviewed: bool,
    },
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
enum Job {
    Review {
        path: PathBuf,
        options: Options,
        native: bool,
    },
    Verify {
        proof: Option<NativeProof>,
    },
    Edit {
        proof: Option<NativeProof>,
        operation: Edit,
    },
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Reply {
    project: bool,
    proof: Option<NativeProof>,
}
fn limits() -> Limits {
    Limits {
        max_pcm_bytes: MAX_PCM,
        ..Default::default()
    }
}
fn memory_file() -> Result<File, String> {
    let fd = unsafe {
        libc::memfd_create(
            c"omatainer-migration".as_ptr(),
            libc::MFD_CLOEXEC | libc::MFD_ALLOW_SEALING,
        )
    };
    if fd < 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    Ok(unsafe { File::from_raw_fd(fd) })
}
fn seal(file: &File) -> Result<(), String> {
    if unsafe {
        libc::fcntl(
            file.as_raw_fd(),
            libc::F_ADD_SEALS,
            libc::F_SEAL_WRITE | libc::F_SEAL_GROW | libc::F_SEAL_SHRINK | libc::F_SEAL_SEAL,
        )
    } < 0
    {
        return Err(std::io::Error::last_os_error().to_string());
    }
    Ok(())
}
fn write(file: &mut File, draft: &Imported, cancel: &AtomicBool) -> Result<(), String> {
    project_file::write_to_file(
        file,
        &Bundle {
            state: draft.state.clone(),
            media: draft.media.clone(),
        },
        &limits(),
        cancel,
    )
    .map_err(|e| e.to_string())
}
fn read(
    mut file: File,
    proof: Option<NativeProof>,
    cancel: &AtomicBool,
) -> Result<Imported, String> {
    file.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
    let bundle = project_file::load_from_file::<project::State>(file, &limits(), cancel)
        .map_err(|e| e.to_string())?;
    bundle.state.validate(&bundle.media)?;
    if bundle.state.migration.is_none() {
        return Err("Migration worker returned no source archive".into());
    }
    Ok(Imported {
        state: bundle.state,
        media: bundle.media,
        reviewed_native: proof,
    })
}
fn executable() -> Result<PathBuf, String> {
    #[cfg(test)]
    return std::env::var_os("OMATAINER_TEST_BIN")
        .map(PathBuf::from)
        .ok_or_else(|| "Native migration validation requires OMATAINER_TEST_BIN".into());
    #[cfg(not(test))]
    std::env::current_exe().map_err(|e| e.to_string())
}
fn exchange(
    job: Job,
    draft: Option<&Imported>,
    cancel: &AtomicBool,
) -> Result<Option<Imported>, String> {
    active(cancel)?;
    let mut input = memory_file()?;
    let mut output = memory_file()?;
    if let Some(draft) = draft {
        write(&mut input, draft, cancel)?;
    }
    seal(&input)?;
    let returns_project = !matches!(job, Job::Verify { .. });
    let mut command = Command::new(executable()?);
    command.arg("ableton-worker");
    let reply: Result<Reply, String> = filesystem_worker::invoke(
        command,
        &job,
        &[&input, &output],
        &|| !cancel.load(Ordering::Acquire),
        Duration::from_secs(120),
    )?;
    let reply = reply?;
    if reply.project != returns_project {
        return Err("Migration worker returned the wrong operation result".into());
    }
    active(cancel)?;
    if !returns_project {
        return Ok(None);
    }
    seal(&output)?;
    output.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
    read(output, reply.proof, cancel).map(Some)
}

/// Review an owned Set or saved migration without performing source I/O in the app.
/// Takes a source, explicit maps, archive choice and cancellation; returns the validated isolated draft with retained proof.
pub(crate) fn review(
    path: &Path,
    options: &Options,
    native: bool,
    cancel: &AtomicBool,
) -> Result<Imported, String> {
    exchange(
        Job::Review {
            path: path.into(),
            options: options.clone(),
            native,
        },
        None,
        cancel,
    )?
    .ok_or_else(|| "Migration worker omitted its project".into())
}
/// Verify the reviewed source in a disposable filesystem worker.
/// Takes a draft and cancellation; returns before publication only if its original Set or native archive remains unchanged.
pub(crate) fn verify(draft: &Imported, cancel: &AtomicBool) -> Result<(), String> {
    exchange(
        Job::Verify {
            proof: draft.reviewed_native.clone(),
        },
        Some(draft),
        cancel,
    )
    .map(|_| ())
}
/// Apply one reviewed migration edit off the app's source I/O threads.
/// Takes the previous draft, explicit render/restore/relink operation and cancellation; returns a complete replacement or preserves the caller's draft on failure.
pub(crate) fn edit(
    draft: &Imported,
    operation: Edit,
    cancel: &AtomicBool,
) -> Result<Imported, String> {
    exchange(
        Job::Edit {
            proof: draft.reviewed_native.clone(),
            operation,
        },
        Some(draft),
        cancel,
    )?
    .ok_or_else(|| "Migration worker omitted its project".into())
}

/// Serve one inherited migration transfer before audio or MIDI startup.
/// Reads strict job JSON and private descriptors 3/4; verifies and writes one bounded native result without publishing a destination project.
pub(crate) fn worker() -> Result<(), String> {
    filesystem_worker::serve(|job: Job| {
        let cancel = AtomicBool::new(false);
        let mut output = unsafe { File::from_raw_fd(4) };
        let draft = match job {
            Job::Review {
                path,
                options,
                native,
            } => {
                if native {
                    load_native(&path, &cancel)?
                } else {
                    load(&path, &options, &cancel)?
                }
            }
            Job::Verify { proof } => {
                let input = unsafe { File::from_raw_fd(3) };
                let draft = read(input, proof, &cancel)?;
                verify_draft(&draft, &cancel)?;
                return Ok(Reply {
                    project: false,
                    proof: None,
                });
            }
            Job::Edit { proof, operation } => {
                let input = unsafe { File::from_raw_fd(3) };
                let draft = read(input, proof, &cancel)?;
                verify_draft(&draft, &cancel)?;
                match operation {
                    Edit::Render { path, options } => {
                        renders::attach(draft, &path, &options, &cancel)?
                    }
                    Edit::Restore => renders::restore(draft, &cancel)?,
                    Edit::Relink {
                        source,
                        device,
                        binary,
                        class,
                        reviewed,
                    } => plugins::relink(draft, source, device, binary, class, reviewed, &cancel)?,
                }
            }
        };
        write(&mut output, &draft, &cancel)?;
        Ok(Reply {
            project: true,
            proof: draft.reviewed_native,
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "Requires a separately built native app with the isolated migration entry point"]
    fn actual_worker_transfers_source_state_and_refuses_corruption_mutation_and_cancel() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!(
            "target/validation/ableton-isolated-{}",
            crate::sampler_bank::BankId::new().unwrap()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("Original 東京.als");
        std::fs::write(&path, super::super::tests::document(11)).unwrap();
        let cancel = AtomicBool::new(false);
        let draft = review(&path, &Options::default(), false, &cancel).unwrap();
        assert_eq!(
            draft.state.migration.as_ref().unwrap().sources[0].xml,
            super::super::tests::document(11)
        );
        verify(&draft, &cancel).unwrap();
        std::fs::write(&path, super::super::tests::document(10)).unwrap();
        assert!(verify(&draft, &cancel)
            .unwrap_err()
            .contains("changed after review"));
        std::fs::write(&path, "<broken").unwrap();
        assert!(review(&path, &Options::default(), false, &cancel).is_err());
        cancel.store(true, Ordering::Release);
        assert!(review(&path, &Options::default(), false, &cancel)
            .err()
            .unwrap()
            .contains("cancelled"));
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn sealed_transfers_reject_changes_and_use_the_real_project_codec() {
        let mut file = memory_file().unwrap();
        let cancel = AtomicBool::new(false);
        let (state, media) = project::State::empty().unwrap();
        project_file::write_to_file(&mut file, &Bundle { state, media }, &limits(), &cancel)
            .unwrap();
        seal(&file).unwrap();
        assert!(file.set_len(0).is_err());
        file.seek(SeekFrom::Start(0)).unwrap();
        let decoded =
            project_file::load_from_file::<project::State>(file, &limits(), &cancel).unwrap();
        decoded.state.validate(&decoded.media).unwrap();
    }
}
