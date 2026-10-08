use super::*;

pub(super) fn inventory(
    track: &Element,
    id: i64,
    source: &mut Source,
    cancel: &AtomicBool,
) -> Result<(), String> {
    if let Some(chain) = at(track, &["DeviceChain", "DeviceChain"])? {
        visit(chain, id, "DeviceChain", source, cancel)?;
    }
    Ok(())
}
fn visit(
    node: &Element,
    track: i64,
    path: &str,
    source: &mut Source,
    cancel: &AtomicBool,
) -> Result<(), String> {
    active(cancel)?;
    for (index, item) in node.children.iter().enumerate() {
        let identity = if item.attr("Id").is_empty() {
            index.to_string()
        } else {
            item.attr("Id").into()
        };
        let next = format!("{path}/{}[{identity}]", item.name);
        if next.len() > 4096 {
            return Err("Device dependency path exceeds 4096 bytes".into());
        }
        if node.name == "Devices" {
            let mut infos = Vec::new();
            if item.name == "PluginDevice" {
                walk(item, "Vst3PluginInfo", &mut infos);
            }
            let format = if !infos.is_empty() {
                Some("VST3".into())
            } else {
                let mut refs = Vec::new();
                if item.name == "PluginDevice" {
                    walk(item, "VstPluginInfo", &mut refs);
                }
                if !refs.is_empty() {
                    Some("VST2".into())
                } else {
                    if item.name == "AuPluginDevice" {
                        walk(item, "AuPluginInfo", &mut refs);
                    }
                    (!refs.is_empty()).then(|| "AU".into())
                }
            };
            let class_id = infos.first().map(|info| uid(info)).transpose()?.flatten();
            source.devices.push(Device {
                track,
                path: next.clone(),
                kind: item.name.clone(),
                name: {
                    let name = value(item, &["UserName"])?;
                    if name.is_empty() {
                        infos
                            .first()
                            .map(|info| value(info, &["Name"]))
                            .transpose()?
                            .filter(|s| !s.is_empty())
                            .unwrap_or(&item.name)
                            .into()
                    } else {
                        name.into()
                    }
                },
                enabled: boolean(item, &["On", "Manual"], true)?,
                format,
                class_id,
                version: infos
                    .first()
                    .map(|info| value(info, &["Version"]))
                    .transpose()?
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned),
                resolution: None,
            });
            source.difference(format!("track:{track}/{next}"),"Device",format!("{} retained with chain, branches, macro values and source state; requires compatible relink, explicit replacement or authorized render",item.name))?;
        }
        if item.name == "FileRef" {
            let (original, relative, pack) = reference(item)?;
            source.assets.push(Asset{id:format!("track:{track}/{next}"),original,relative,pack,source:None,availability:Availability::Missing,detail:"Device asset retained; choose a compatible engine or a render before loading it".into(),audio_sha256:None});
        }
        visit(item, track, &next, source, cancel)?;
    }
    Ok(())
}
fn uid(info: &Element) -> Result<Option<String>, String> {
    let Some(uid) = child(info, "Uid")? else {
        return Ok(None);
    };
    if !uid.attr("Value").is_empty() {
        let value = uid.attr("Value");
        if value.len() != 32 || !value.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err("Invalid VST3 class identity".into());
        }
        return Ok(Some(value.to_ascii_uppercase()));
    }
    let mut result = String::new();
    for name in ["Uid0", "Uid1", "Uid2", "Uid3"] {
        let raw = value(uid, &[name])?;
        if raw.is_empty() {
            return Ok(None);
        }
        let word: i64 = raw.parse().map_err(|_| "Invalid VST3 UID word")?;
        if !(-2147483648..=4294967295).contains(&word) {
            return Err("Invalid VST3 UID word".into());
        }
        result.push_str(&format!("{:08X}", word as u32));
    }
    Ok(Some(result))
}
pub(super) fn reference(reference: &Element) -> Result<(String, String, String), String> {
    let path = value(reference, &["Path"])?;
    let path = if path.is_empty() {
        value(reference, &["OriginalFilePath"])?
    } else {
        path
    };
    let mut relative = value(reference, &["RelativePath"])?.to_owned();
    if relative.is_empty() {
        if let Some(parts) = child(reference, "RelativePath")? {
            for part in parts.named("RelativePathElement") {
                if !relative.is_empty() {
                    relative.push('/');
                }
                relative.push_str(part.attr("Dir"));
            }
        }
    }
    if relative.is_empty() {
        if let Some(data) = child(reference, "Data")? {
            for part in data.named("RelativePathElement") {
                if !relative.is_empty() {
                    relative.push('/');
                }
                relative.push_str(part.attr("Dir"));
            }
        }
    }
    let name = value(reference, &["Name"])?;
    if !name.is_empty() && Path::new(&relative).file_name().is_none_or(|p| p != name) {
        if !relative.is_empty() {
            relative.push('/');
        }
        relative.push_str(name);
    }
    let pack = value(reference, &["PackId"])?.to_owned();
    if [path, &relative, &pack].into_iter().any(|s| !text_valid(s)) {
        return Err("Asset path or Pack identity exceeds bounds".into());
    }
    Ok((path.into(), relative, pack))
}
pub(super) fn asset(
    reference: &Element,
    id: String,
    source: &mut Source,
    options: &Options,
    snapshot: &crate::media_location::Snapshot,
    media: &mut Vec<Arc<Sample>>,
    pcm: &mut u64,
    cancel: &AtomicBool,
) -> Result<Option<usize>, String> {
    let (original, relative, pack) = self::reference(reference)?;
    let mut asset = Asset {
        id,
        original,
        relative,
        pack,
        source: None,
        availability: Availability::Missing,
        detail: "Source media is missing; source path and Pack identity retained".into(),
        audio_sha256: None,
    };
    let mut result = None;
    let mut paths = Vec::new();
    for (from, to) in &options.remaps {
        if let Some(tail) = asset
            .original
            .strip_prefix(from)
            .filter(|tail| tail.is_empty() || tail.starts_with('/'))
        {
            paths.push(to.join(tail.trim_start_matches('/')));
        }
    }
    if !asset.relative.is_empty() {
        paths.push(
            source
                .path
                .parent()
                .ok_or("Set has no parent directory")?
                .join(&asset.relative),
        );
    }
    if asset.original.starts_with('/') {
        paths.push(asset.original.clone().into());
    }
    if !asset.pack.is_empty() && !asset.relative.is_empty() {
        let relative=Path::new(&asset.relative);
        if relative.is_absolute() || relative.components().any(|c|matches!(c,std::path::Component::ParentDir|std::path::Component::RootDir)) {return Err("Pack asset escapes its declared installation".into());}
        for pack in &options.libraries {if pack.validate()?==asset.pack {paths.insert(0,pack.root(cancel)?.join(relative));}}
    }
    paths.dedup();
    for path in paths {
        active(cancel)?;
        let location = match snapshot.identify(&path) {
            Ok(location) => location,
            Err(error) => {
                match std::fs::metadata(&path) {
                    Err(io) if io.kind() == std::io::ErrorKind::NotFound => {
                        if asset.source.is_none() {
                            for parent in path.ancestors().skip(1).take(64) {
                                if let Ok(base) = snapshot.identify(parent) {
                                    if let Ok(child) = base.child(&path) {
                                        asset.source = Some(child.source);
                                    }
                                    break;
                                }
                            }
                        }
                    }
                    _ => {
                        asset.availability = Availability::Inaccessible;
                        asset.detail = error.to_string();
                    }
                }
                continue;
            }
        };
        asset.source = Some(location.source.clone());
        if let Some(index) = media
            .iter()
            .position(|s| s.path == location.path.to_string_lossy())
        {
            result = Some(index);
            asset.audio_sha256 = Some(
                crate::project_dependencies::audio_hash(&media[index], cancel)?
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect(),
            );
            asset.availability = Availability::Embedded;
            asset.detail = "Verified source audio is embedded in the native project".into();
            break;
        }
        let file = match OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NONBLOCK | libc::O_CLOEXEC)
            .open(&location.path)
        {
            Ok(file) => file,
            Err(error) => {
                asset.availability = Availability::Inaccessible;
                asset.detail = error.to_string();
                continue;
            }
        };
        let meta = file.metadata().map_err(|e| e.to_string())?;
        if !meta.is_file() {
            asset.availability = Availability::Unsupported;
            asset.detail = "Source is not a regular audio file".into();
            continue;
        }
        let fingerprint = FileFingerprint::from_metadata(&meta);
        let check = file.try_clone().map_err(|e| e.to_string())?;
        let decoded = match engine::decode::decode_sampler_file(
            &location.path,
            file,
            MAX_PCM.saturating_sub(*pcm),
            || cancel.load(Ordering::Acquire),
        ) {
            Ok(decoded) => decoded,
            Err(error) => {
                active(cancel)?;
                asset.availability = Availability::Unsupported;
                asset.detail = format!("Audio decode refused: {error}");
                continue;
            }
        };
        location
            .verify_file(&check, fingerprint)
            .map_err(|e| e.to_string())?;
        *pcm = pcm
            .checked_add(decoded.sample.data.len() as u64 * 4)
            .ok_or("Imported audio storage overflow")?;
        if *pcm > MAX_PCM {
            return Err(
                "Imported audio exceeds 512 MiB; migrate fewer assets or use bounded renders"
                    .into(),
            );
        }
        result = Some(media.len());
        asset.audio_sha256 = Some(
            crate::project_dependencies::audio_hash(&decoded.sample, cancel)?
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect(),
        );
        asset.availability = Availability::Embedded;
        asset.detail = "Verified source audio is embedded in the native project".into();
        media.push(Arc::new(decoded.sample));
        break;
    }
    if !matches!(asset.availability, Availability::Embedded) {
        source.difference(&asset.id, "Media", &asset.detail)?;
    }
    source.assets.push(asset);
    Ok(result)
}

/// Resolve retained device samples without substituting an engine.
/// Takes the source device tree, explicit libraries and native media budget; embeds verified audio while preserving original device paths and the unresolved engine decision.
pub(super) fn resolve_inventory(track:&Element, id:i64, source:&mut Source, options:&Options, snapshot:&crate::media_location::Snapshot, media:&mut Vec<Arc<Sample>>, pcm:&mut u64, cancel:&AtomicBool)->Result<(),String>{
    fn visit(node:&Element, id:i64, path:&str, source:&mut Source, options:&Options, snapshot:&crate::media_location::Snapshot, media:&mut Vec<Arc<Sample>>, pcm:&mut u64, cancel:&AtomicBool)->Result<(),String>{
        active(cancel)?;
        for (index,item) in node.children.iter().enumerate(){
            let identity=if item.attr("Id").is_empty(){index.to_string()}else{item.attr("Id").into()};let next=format!("{path}/{}[{identity}]",item.name);
            if item.name=="FileRef" {let asset_id=format!("track:{id}/{next}");if let Some(index)=source.assets.iter().position(|a|a.id==asset_id){source.assets.remove(index);asset(item,asset_id,source,options,snapshot,media,pcm,cancel)?;}}
            visit(item,id,&next,source,options,snapshot,media,pcm,cancel)?;
        }Ok(())
    }
    if let Some(chain)=at(track,&["DeviceChain","DeviceChain"])? {visit(chain,id,"DeviceChain",source,options,snapshot,media,pcm,cancel)?;}Ok(())
}
