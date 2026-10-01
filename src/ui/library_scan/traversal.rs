use super::*;
use std::collections::HashSet;
use walkdir::WalkDir;

fn bounded(text: impl ToString, bytes: usize) -> String {
    let mut text = text.to_string();
    if text.len() > bytes {
        let mut end = bytes;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
    }
    text
}
fn skip(
    request: &Request,
    summary: &mut Summary,
    path: &Path,
    reason: SkipReason,
    detail: impl ToString,
) {
    summary.skipped[reason as usize] += 1;
    request.progress.skipped.fetch_add(1, Ordering::Relaxed);
    request.progress.reasons[reason as usize].fetch_add(1, Ordering::Relaxed);
    summary.truncated |= matches!(reason, SkipReason::DepthLimit | SkipReason::Capacity);
    if summary.samples.len() < MAX_SKIP_SAMPLES {
        summary.samples.push(Skipped {
            path: bounded(path.display(), 1024),
            reason,
            detail: bounded(detail, 256),
        });
    }
}

pub(super) fn scan(
    request: &Request,
    previous_fingerprints: &HashMap<PathBuf, Fingerprint>,
) -> Result<(Vec<LibItem>, HashMap<PathBuf, Fingerprint>, Summary), ScanFailure> {
    check_cancel(request)?;
    let previous: HashMap<_, _> = request
        .baseline
        .iter()
        .map(|item| (&item.source, item))
        .collect();
    let mut items = builtin_crate_items();
    for item in &mut items {
        if let Some(old) = previous.get(&item.source) {
            preserve_metadata(item, old);
        }
    }
    if request.kind == Kind::Import {
        let builtin: HashSet<_> = items.iter().map(|item| item.source.clone()).collect();
        items.extend(
            request
                .baseline
                .iter()
                .filter(|item| !builtin.contains(&item.source))
                .cloned(),
        );
    }
    if items.len() > MAX_ITEMS {
        return Err(ScanFailure::Io("Import baseline plus required built-in rows exceeds 100,000 entries".into()));
    }
    let mut positions: HashMap<_, _> = items
        .iter()
        .enumerate()
        .map(|(i, item)| (item.source.clone(), i))
        .collect();
    let mut fingerprints = HashMap::new();
    let mut seen = HashSet::new();
    let mut summary = Summary::default();
    let mut roots = Vec::new();
    for root in &request.roots {
        check_cancel(request)?;
        let root = match root.canonicalize() {
            Ok(path) => path,
            Err(error) => {
                skip(
                    request,
                    &mut summary,
                    root,
                    if error.kind() == std::io::ErrorKind::NotFound {
                        SkipReason::MissingRoot
                    } else {
                        SkipReason::Unreadable
                    },
                    error,
                );
                continue;
            }
        };
        if root.to_str().is_none() || root.as_os_str().len() > 4096 {
            skip(
                request,
                &mut summary,
                &root,
                SkipReason::InvalidPath,
                "UTF-8 absolute paths within 4096 bytes are required by the catalog",
            );
            continue;
        }
        let metadata = match root.metadata() {
            Ok(metadata) => metadata,
            Err(error) => {
                skip(request, &mut summary, &root, SkipReason::Unreadable, error);
                continue;
            }
        };
        if request.kind == Kind::Roots && !metadata.is_dir() {
            return Err(io_error(&root, "scan root is not a directory"));
        }
        if !metadata.is_dir() && !metadata.is_file() {
            skip(
                request,
                &mut summary,
                &root,
                SkipReason::Unsupported,
                "only regular files and directories can be imported",
            );
            continue;
        }
        roots.push(root);
    }
    roots.sort_by_key(|p| p.components().count());
    let mut selected: Vec<PathBuf> = Vec::new();
    for root in roots {
        if selected.iter().any(|parent| root.starts_with(parent)) {
            skip(
                request,
                &mut summary,
                &root,
                SkipReason::Duplicate,
                "already covered by another input",
            );
        } else {
            selected.push(root);
        }
    }
    'roots: for root in selected {
        for entry in WalkDir::new(&root)
            .max_depth(MAX_DEPTH)
            .max_open(8)
            .follow_links(false)
        {
            check_cancel(request)?;
            if request.progress.visited.load(Ordering::Relaxed) >= MAX_VISITED {
                skip(
                    request,
                    &mut summary,
                    &root,
                    SkipReason::Capacity,
                    "one million visited entries per operation",
                );
                break 'roots;
            }
            request.progress.visited.fetch_add(1, Ordering::Relaxed);
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    skip(
                        request,
                        &mut summary,
                        error.path().unwrap_or(&root),
                        SkipReason::Unreadable,
                        &error,
                    );
                    continue;
                }
            };
            #[cfg(test)]
            if let Some(hook) = &request.options.before_entry {
                hook(entry.path());
            }
            check_cancel(request)?;
            let path = entry.path();
            if entry.file_type().is_symlink() {
                skip(
                    request,
                    &mut summary,
                    path,
                    SkipReason::Symlink,
                    "symlink descendants are not followed",
                );
                continue;
            }
            if entry.file_type().is_dir() {
                if entry.depth() == MAX_DEPTH {
                    skip(
                        request,
                        &mut summary,
                        path,
                        SkipReason::DepthLimit,
                        "64-level traversal boundary; descendants not inspected",
                    );
                }
                continue;
            }
            if !entry.file_type().is_file() {
                skip(
                    request,
                    &mut summary,
                    path,
                    SkipReason::Unsupported,
                    "not a regular file",
                );
                continue;
            }
            if path.to_str().is_none() || path.as_os_str().len() > 4096 {
                skip(
                    request,
                    &mut summary,
                    path,
                    SkipReason::InvalidPath,
                    "path cannot be serialized by this catalog",
                );
                continue;
            }
            let extension = path
                .extension()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            if !matches!(
                extension.as_str(),
                "wav" | "mp3" | "flac" | "ogg" | "aiff" | "aif" | "m4a" | "aac"
            ) {
                skip(
                    request,
                    &mut summary,
                    path,
                    SkipReason::Unsupported,
                    "extension is not a supported audio import type",
                );
                continue;
            }
            if !seen.insert(path.to_path_buf()) {
                skip(
                    request,
                    &mut summary,
                    path,
                    SkipReason::Duplicate,
                    "already inspected in this operation",
                );
                continue;
            }
            let metadata = match readable_metadata(path) {
                Ok(meta) => meta,
                Err(error) => {
                    skip(request, &mut summary, path, SkipReason::Unreadable, error);
                    continue;
                }
            };
            let fingerprint = FileFingerprint::from_metadata(&metadata);
            let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("track");
            let (artist, title) = split_artist_title(stem);
            let (bpm, key) = parse_tags(stem);
            let mut item = LibItem {
                title,
                artist,
                bpm: Bpm::hint(bpm),
                fingerprint: Some(fingerprint),
                key,
                length: None,
                last_play: None,
                source: LibSource::File(path.to_path_buf()),
            };
            if previous_fingerprints
                .get(path)
                .is_none_or(|previous| *previous == fingerprint)
            {
                if let Some(old) = previous.get(&item.source) {
                    preserve_metadata(&mut item, old);
                }
            }
            if let Some(&position) = positions.get(&item.source) {
                items[position] = item;
            } else {
                if items.len() == MAX_ITEMS {
                    skip(
                        request,
                        &mut summary,
                        path,
                        SkipReason::Capacity,
                        "100,000 library entry capacity; remaining inputs not inspected",
                    );
                    break 'roots;
                }
                positions.insert(item.source.clone(), items.len());
                items.push(item);
            }
            fingerprints.insert(path.to_path_buf(), fingerprint);
            request.progress.found.fetch_add(1, Ordering::Relaxed);
        }
    }
    check_cancel(request)?;
    request.progress.phase.store(1, Ordering::Relaxed);
    sort_crate(&mut items);
    check_cancel(request)?;
    Ok((items, fingerprints, summary))
}

fn readable_metadata(path: &Path) -> std::io::Result<std::fs::Metadata> {
    use std::os::unix::fs::OpenOptionsExt;
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file()
        || FileFingerprint::from_metadata(&metadata)
            != FileFingerprint::from_metadata(&path.symlink_metadata()?)
    {
        return Err(std::io::Error::other(
            "entry changed or is not a readable regular file",
        ));
    }
    Ok(metadata)
}
