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
    summary.truncated |= matches!(reason, SkipReason::DepthLimit | SkipReason::Capacity | SkipReason::Unreadable | SkipReason::MissingRoot | SkipReason::InvalidPath | SkipReason::Symlink);
    if summary.samples.len() < MAX_SKIP_SAMPLES {
        summary.samples.push(Skipped {
            path: bounded(path.display(), 1024),
            reason,
            detail: bounded(detail, 256),
        });
    }
}

pub(super) fn scan_with_inventory(
    request: &Request,
    previous_fingerprints: &HashMap<PathBuf, Fingerprint>,
    inventory: &mut impl FnMut() -> Result<crate::media_location::Snapshot, crate::media_location::Failure>,
) -> Result<(Vec<LibItem>, HashMap<PathBuf, Fingerprint>, Summary, Option<ScanRoots>), ScanFailure> {
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
    let snapshot = if request.watch.is_some() || request.kind==Kind::Import { Some(inventory().map_err(|e| io_error(Path::new("watched media discovery"),e))?) } else {None};
    let mut batch = request.watch.as_ref().filter(|_|request.kind==Kind::Roots).map(|watch| crate::library::watch_roots::Batch {
        expected:watch.catalog.watched_roots.revision(), profile:watch.profile.clone(), configured:request.roots.clone(),observed:Vec::new() });
    let mut adoptions=Vec::new();let mut directories=Vec::new();
    let mut guards = Vec::new();
    let mut roots = Vec::new();
    for configured in &request.roots {
        check_cancel(request)?;
        let location = if let Some(snapshot)=&snapshot {
            let known=request.watch.as_ref().filter(|_|request.kind==Kind::Roots).and_then(|watch|watch.catalog.watched_roots.binding(&watch.profile,configured));
            let observed = match known {
                Some(binding) if matches!(binding.source, LibSource::Removable {..}) => snapshot.resolve(&binding.source),
                _ => snapshot.identify(configured),
            };
            match observed {
                Ok(location) => {
                    if let Err(error)=snapshot.inspect(&location) { skip(request,&mut summary,configured,SkipReason::MissingRoot,error);continue; }
                    if let Some(batch)=&mut batch {batch.observed.push(crate::library::watch_roots::Binding {
                        profile:batch.profile.clone(),configured:configured.clone(),source:location.source.clone() });}
                    guards.push(location.clone());Some(location)
                },
                Err(error) => { skip(request,&mut summary,configured,SkipReason::MissingRoot,error);continue; }
            }
        } else {None};
        let input = location.as_ref().map_or(configured.as_path(),|l|l.path.as_path());
        let root = match input.canonicalize() {
            Ok(path) => path,
            Err(error) => {
                skip(
                    request,
                    &mut summary,
                    configured,
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
        roots.push((root,location,configured.clone()));
    }
    // Validate the whole profile batch before touching its candidate rows.
    if let (Some(watch),Some(batch))=(&request.watch,&batch) {
        let mut book=watch.catalog.watched_roots.clone();book.apply(batch).map_err(|e|io_error(Path::new("watched roots"),e))?;
    }
    roots.sort_by_key(|(p,_,_)| p.components().count());
    let mut selected: Vec<(PathBuf,Option<crate::media_location::Location>,PathBuf)> = Vec::new();
    for (root,location,configured) in roots {
        if selected.iter().any(|(parent,old,_)| root.starts_with(parent) || match (old.as_ref().map(|l|&l.source),location.as_ref().map(|l|&l.source)) {
            (Some(LibSource::Removable {volume_id:a,relative_path:pa}),Some(LibSource::Removable {volume_id:b,relative_path:pb})) => a==b && pb.starts_with(pa),_=>false }) {
            skip(
                request,
                &mut summary,
                &root,
                SkipReason::Duplicate,
                "already covered by another input",
            );
        } else {
            selected.push((root,location,configured));
        }
    }
    'roots: for (root,location,configured) in selected {
        let mut walk=WalkDir::new(&root).max_depth(MAX_DEPTH).max_open(8).follow_links(false).into_iter();
        while let Some(entry)=walk.next() {
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
            let child = if let (Some(location),Some(snapshot))=(&location,&snapshot) {
                match location.child(path).and_then(|child|snapshot.inspect(&child).map(|_|child)) {
                    Ok(child)=>Some(child),Err(error)=> {
                        if entry.file_type().is_dir() { walk.skip_current_dir(); }
                        skip(request,&mut summary,path,SkipReason::Unreadable,error);continue;
                    }
                }
            } else {None};
            if entry.file_type().is_dir() {
                if request.watch_enabled && request.kind==Kind::Roots {
                    if directories.len()<watch::MAX_DIRECTORIES {directories.push(path.into());} else {summary.watch_limited=true;}
                }
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
            let source=child.map_or_else(||LibSource::File(path.to_path_buf()),|l|l.source);
            if !seen.insert(source.clone()) {
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
                source,
            };
            let mut alias=None;
            if matches!(item.source,LibSource::Removable {..}) {
                let tail=path.strip_prefix(&root).map_err(|e|io_error(path,e))?;
                let old_root=request.watch.as_ref().and_then(|w|w.catalog.watched_roots.binding(&w.profile,&configured))
                    .and_then(|b|if let LibSource::File(p)=&b.source {Some(p.as_path())} else {None});
                let mut paths=vec![path.to_path_buf(),configured.join(tail)];
                if let Some(old)=old_root {paths.push(old.join(tail));}
                for old in paths.into_iter().map(LibSource::File) {
                    let stored=request.watch.as_ref().and_then(|w|w.catalog.track(&old)).map(|t|t.versions[t.current].fingerprint);
                    let expected=previous.get(&old).and_then(|i|i.fingerprint).or(stored.flatten());
                    if let Some(expected)=expected {
                        adoptions.push(crate::library::watch_roots::Adoption {old:old.clone(),expected,source:item.source.clone()});
                        alias=Some(old);break;
                    }
                }
            }
            if previous_fingerprints
                .get(path)
                .is_none_or(|previous| *previous == fingerprint)
            {
                if let Some(old) = previous.get(&item.source).or_else(||alias.as_ref().and_then(|old|previous.get(old))) {
                    preserve_metadata(&mut item, old);
                }
            }
            if let Some(position)=positions.get(&item.source).copied().or_else(||alias.as_ref().and_then(|old|positions.remove(old))) {
                positions.insert(item.source.clone(),position);
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
    if snapshot.is_some() {
        let after=inventory().map_err(|e|io_error(Path::new("watched media recheck"),e))?;
        for guard in guards { guard.recheck_with(&after).map_err(|e|io_error(&guard.path,e))?; }
        for item in items.iter().chain(request.baseline.iter()).filter(|i|matches!(i.source,LibSource::Removable {..})) {
            check_cancel(request)?;
            if summary.availability.contains_key(&item.source) {continue;}
            if summary.availability.len()==MAX_ITEMS {summary.truncated=true;break;}
            let state=after.resolve(&item.source).and_then(|l|after.inspect(&l).map(|_|())).map_or_else(|e|e.to_string(),|_|"Available on the mounted volume".into());
            summary.availability.insert(item.source.clone(),state);
        }
    }
    request.progress.phase.store(1, Ordering::Relaxed);
    sort_crate(&mut items);
    check_cancel(request)?;
    let receipt=(batch.is_some() || !adoptions.is_empty()).then(||ScanRoots {book:batch,adoptions,directories});
    Ok((items, fingerprints, summary, receipt))
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
