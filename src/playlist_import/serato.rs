use super::*;

fn text(bytes: &[u8]) -> Result<String, String> {
    if bytes.len() % 2 != 0 || bytes.len() > 8192 {
        return Err("Invalid bounded UTF-16BE Serato field".into());
    }
    let words: Vec<_> = bytes
        .chunks_exact(2)
        .map(|b| u16::from_be_bytes([b[0], b[1]]))
        .collect();
    let value = String::from_utf16(&words).map_err(|_| "Invalid Serato UTF-16BE")?;
    label(&value)
}
fn records<'a>(
    mut bytes: &'a [u8],
    check: &impl Fn() -> bool,
) -> Result<Vec<(&'a [u8], &'a [u8])>, String> {
    let mut records = Vec::new();
    while !bytes.is_empty() {
        active(check)?;
        if bytes.len() < 8 || records.len() >= 100_000 {
            return Err("Truncated or oversized Serato record list".into());
        }
        let size = u32::from_be_bytes(bytes[4..8].try_into().unwrap()) as usize;
        if size == 0 || size > bytes.len() - 8 || !bytes[..4].iter().all(u8::is_ascii_alphanumeric)
        {
            return Err("Invalid Serato record header or length".into());
        }
        records.push((&bytes[..4], &bytes[8..8 + size]));
        bytes = &bytes[8 + size..];
    }
    Ok(records)
}

/// Decode legacy Serato crate membership without reading or writing its database.
/// Takes crate bytes, filename and cancellation predicate; returns ordered root-relative references or refuses an unknown version.
pub(super) fn parse(
    bytes: &[u8],
    path: &Path,
    check: &impl Fn() -> bool,
) -> Result<Vec<RawPlaylist>, String> {
    let all = records(bytes, check)?;
    if all.first().map(|r| r.0) != Some(b"vrsn".as_slice())
        || all.iter().filter(|r| r.0 == b"vrsn").count() != 1
    {
        return Err("Serato crate needs one leading version record".into());
    }
    let version = text(all[0].1)?;
    if version != "1.0/Serato ScratchLive Crate" {
        return Err(format!(
            "Unsupported Serato crate version {version}; export M3U or supported legacy crates"
        ));
    }
    let mut entries = Vec::new();
    for (tag, value) in &all[1..] {
        if *tag != b"otrk" {
            continue;
        }
        if entries.len() >= MAX_REFERENCES {
            return Err("Serato crate exceeds 4096 references".into());
        }
        let fields = records(value, check)?;
        let paths: Vec<_> = fields.iter().filter(|r| r.0 == b"ptrk").collect();
        if paths.len() != 1 {
            return Err("Serato track needs exactly one path".into());
        }
        let reference = text(paths[0].1)?;
        if reference.is_empty()
            || reference.starts_with('/')
            || reference.contains('\\')
            || Path::new(&reference)
                .components()
                .any(|c| matches!(c, std::path::Component::ParentDir))
        {
            return Err("Serato crate path must stay relative to its source volume root".into());
        }
        entries.push(RawEntry {
            reference,
            title: String::new(),
            artist: String::new(),
            blocked: None,
            details: Default::default(),
        });
    }
    let title = path
        .file_stem()
        .and_then(|n| n.to_str())
        .ok_or("Serato crate filename must be UTF-8")?;
    let mut components: Vec<_> = title.split("%%").map(name).collect::<Result<_, _>>()?;
    let title = components.pop().ok_or("Serato crate has no name")?;
    if components.len() >= crate::library::crates::MAX_DEPTH {
        return Err("Serato crate hierarchy exceeds 32 levels".into());
    }
    let mut result = Vec::new();
    let mut parent = None;
    for index in 0..components.len() {
        let key = format!(
            "serato:folder:{}",
            serde_json::to_string(&components[..=index]).map_err(|e| e.to_string())?
        );
        result.push(RawPlaylist {
            name: components[index].clone(),
            folders: components[..index].to_vec(),
            entries: Vec::new(),
            note: "Serato organizational folder".into(),
            key: key.clone(),
            parent: parent.clone(),
            folder: true,
        });
        parent = Some(key);
    }
    result.push(RawPlaylist { name:title, folders:components, key:"serato:crate".into(), parent, folder:false, entries, note:format!("Legacy Serato {version}; membership and folders retained. Crate files contain no cues, loops or beatgrid; embedded audio tags remain available.") });
    Ok(result)
}
