use super::*;
pub(super) fn parse(bytes: &[u8], title: &str, check: &impl Fn()->bool) -> Result<Vec<RawPlaylist>,String> {
    let text=std::str::from_utf8(bytes).map_err(|_|"M3U import requires UTF-8; re-export legacy encoded playlists as M3U8")?.trim_start_matches('\u{feff}');
    let mut entries=Vec::new();let mut hint=String::new();
    for line in text.lines() {
        active(check)?;let line=line.trim();
        if line.len()>8192 || line.contains('\0') {return Err("Playlist line exceeds 8192 bytes or contains NUL".into());}
        if let Some(value)=line.strip_prefix("#EXTINF:") {hint=value.split_once(',').map(|(_,s)|s.to_owned()).unwrap_or_default();continue;}
        if line.is_empty() || line.starts_with('#') {continue;}
        if line.len()>4096 || line.chars().any(char::is_control) {return Err("Playlist path exceeds 4096 bytes or contains a control character".into());}
        if entries.len()>=MAX_REFERENCES {return Err("Playlist exceeds 4096 references".into());}
        let (artist,title)=hint.split_once(" - ").map(|(a,t)|(a.to_owned(),t.to_owned())).unwrap_or_else(||(String::new(),hint.clone()));
        entries.push(RawEntry{reference:line.into(),title:label(&title)?,artist:label(&artist)?,blocked:None});hint.clear();
    }
    Ok(vec![RawPlaylist{name:name(title)?,entries,note:"Static UTF-8 M3U references; duplicates keep their first position".into()}])
}
