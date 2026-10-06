use crate::{engine::media_source::FileFingerprint, library::{Catalog, Metadata, TrackId, crates::{CrateId, Edit}}, media_location::{Location, Snapshot}, ui::bpm::Bpm};
use std::{collections::{HashMap, HashSet}, fs::OpenOptions, io::Read, os::unix::fs::OpenOptionsExt, path::{Path, PathBuf}, sync::Arc};
mod m3u;
mod plist;
#[cfg(test)] mod tests;

const MAX_BYTES: u64 = 32 * 1024 * 1024;
const MAX_REFERENCES: usize = 4096;
const MAX_PLAYLISTS: usize = 128;

#[derive(Clone, Debug)]
pub(crate) struct Input { pub path: PathBuf, pub mapping: Option<Mapping> }
#[derive(Clone, Debug)]
pub(crate) struct Mapping { pub from: String, pub to: PathBuf }
impl Input {
    /// Validate an import request. Takes the playlist path and optional explicit prefix replacement; returns an error without accessing files.
    pub fn validate(&self) -> Result<(), String> {
        let path = |p: &Path| p.is_absolute() && p.to_str().is_some_and(|s| s.len() <= 4096 && !s.chars().any(char::is_control));
        if !path(&self.path) { return Err("Choose an absolute UTF-8 playlist path within 4096 bytes".into()); }
        if self.mapping.as_ref().is_some_and(|m| m.from.is_empty() || m.from.len()>4096 || m.from.trim()!=m.from || m.from.chars().any(char::is_control) || !path(&m.to)) {return Err("Path mapping needs a nonempty source prefix and an absolute local directory".into());}
        Ok(())
    }
}
#[derive(Clone, Debug)]
struct RawEntry { reference: String, title: String, artist: String, blocked: Option<&'static str> }
#[derive(Clone, Debug)]
struct RawPlaylist { name: String, entries: Vec<RawEntry>, note: String }
#[derive(Clone, Debug)]
struct Media { location: Location, fingerprint: FileFingerprint, metadata: Metadata, existing: Option<TrackId> }
#[derive(Clone, Debug)]
pub(crate) struct Row { pub title: String, pub reference: String, pub status: String, media: Option<Arc<Media>> }
impl Row {
    pub fn ready(&self) -> bool { self.media.is_some() }
    pub fn resolved(&self)->Option<&Path> {self.media.as_ref().map(|m|m.location.path.as_path())}
}
#[derive(Clone, Debug)]
pub(crate) struct Playlist { pub name: String, pub rows: Vec<usize>, pub note: String }
#[derive(Clone, Debug)]
pub(crate) struct Review { path: PathBuf, fingerprint: FileFingerprint, pub playlists: Vec<Playlist>, pub rows: Vec<Row> }

fn active(check: &impl Fn() -> bool) -> Result<(), String> {if check() {Ok(())} else {Err("Playlist operation cancelled before publication".into())}}
fn name(value: &str) -> Result<String, String> {
    let value=value.trim();
    if value.is_empty() || value.len()>256 || value.chars().any(char::is_control) {Err("Playlist names must be nonempty, control-free and at most 256 UTF-8 bytes".into())} else {Ok(value.into())}
}
fn label(value: &str) -> Result<String, String> {if crate::media_tags::text_valid(value) {Ok(value.into())} else {Err("Playlist labels exceed 4096 bytes or contain control characters".into())}}
fn local_path(reference: &str, parent: &Path, mapping: Option<&Mapping>) -> Result<PathBuf, String> {
    if reference.is_empty() || reference.len()>4096 || reference.chars().any(char::is_control) {return Err("Invalid path reference".into());}
    let file_uri=reference.get(..5).is_some_and(|s|s.eq_ignore_ascii_case("file:"));
    let mut text=if file_uri {
        let uri=url::Url::parse(reference).map_err(|_|"Invalid file URI")?;
        if uri.query().is_some() || uri.fragment().is_some() {return Err("File URI has a query or fragment".into());}
        uri.to_file_path().map_err(|_|"Unmapped file URI authority")?.to_str().ok_or("File URI path is not UTF-8")?.to_owned()
    } else {reference.replace('\\',"/")};
    if text.as_bytes().get(0)==Some(&b'/') && text.as_bytes().get(2)==Some(&b':') {text.remove(0);}
    let foreign=text.as_bytes().get(1)==Some(&b':') || text.starts_with("//");
    if !foreign && !file_uri && url::Url::parse(reference).is_ok() {return Err("Provider-only or remote URI; local audio is required".into());}
    if let Some(mapping)=mapping {
        let prefix=mapping.from.replace('\\',"/").trim_end_matches('/').to_owned();
        if text==prefix || text.strip_prefix(&prefix).is_some_and(|tail|tail.starts_with('/')) {
            let tail=text.strip_prefix(&prefix).unwrap().trim_start_matches('/');
            if Path::new(tail).components().any(|c|matches!(c,std::path::Component::ParentDir)) {return Err("Mapped path escapes the reviewed prefix".into());}
            return Ok(mapping.to.join(tail));
        }
    }
    if foreign {return Err("Unmapped Windows drive or network path; add an explicit prefix mapping".into());}
    let path=PathBuf::from(text);Ok(if path.is_absolute(){path}else{parent.join(path)})
}
struct Cancel<'a>(&'a dyn Fn() -> bool);
impl crate::media_tags::Cancellation for Cancel<'_> {fn cancelled(&self)->bool {!(self.0)()}}

/// Review a local playlist on the catalog worker. Takes input, current catalog and cancellation check; returns bounded ordered references with explicit per-entry failures and no catalog changes.
pub(crate) fn review(input: &Input, catalog: &Catalog, check: impl Fn() -> bool) -> Result<Review, String> {
    input.validate()?;active(&check)?;
    if input.mapping.as_ref().is_some_and(|m|!m.to.is_dir()) {return Err("Path mapping destination must be an existing local directory".into());}
    let mut file=OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW|libc::O_NONBLOCK).open(&input.path).map_err(|e|e.to_string())?;
    let meta=file.metadata().map_err(|e|e.to_string())?;
    if !meta.is_file() || meta.len()>MAX_BYTES {return Err("Playlist must be a regular file within 32 MiB".into());}
    let fingerprint=FileFingerprint::from_metadata(&meta);
    let mut bytes=Vec::with_capacity(meta.len() as usize);let mut chunk=[0u8;65536];
    loop {active(&check)?;let count=file.read(&mut chunk).map_err(|e|e.to_string())?;if count==0 {break;}if bytes.len()+count>MAX_BYTES as usize {return Err("Playlist exceeds 32 MiB".into());}bytes.extend_from_slice(&chunk[..count]);}
    if FileFingerprint::from_metadata(&file.metadata().map_err(|e|e.to_string())?)!=fingerprint || FileFingerprint::read(&input.path)!=Some(fingerprint) {return Err("Playlist changed during review".into());}
    let extension=input.path.extension().and_then(|s|s.to_str()).unwrap_or("").to_ascii_lowercase();
    let raw=match extension.as_str() {
        "m3u"|"m3u8"=>m3u::parse(&bytes,input.path.file_stem().and_then(|s|s.to_str()).unwrap_or("Imported playlist"),&check)?,
        "xml"=>plist::parse(&bytes,&check)?,
        _=>return Err("Supported imports: UTF-8 M3U/M3U8 and Apple Music/iTunes XML".into()),
    };
    if raw.is_empty() || raw.len()>MAX_PLAYLISTS || raw.iter().map(|p|p.entries.len()).sum::<usize>()>MAX_REFERENCES {return Err("Review accepts 1–128 playlists and at most 4096 total references; export a smaller selection".into());}
    let snapshot=Snapshot::discover().map_err(|e|e.to_string())?;
    let mut cache=HashMap::<String,Result<Arc<Media>,String>>::new();let mut rows=Vec::new();let mut playlists=Vec::new();
    for playlist in raw {
        active(&check)?;let mut indices=Vec::new();let mut seen=HashSet::new();
        for entry in playlist.entries {
            active(&check)?;
            let result=if let Some(reason)=entry.blocked {Err(reason.into())} else if let Some(result)=cache.get(&entry.reference) {result.clone()} else {
                let result=(||{
                    let path=local_path(&entry.reference,input.path.parent().unwrap(),input.mapping.as_ref())?;
                    let extension=path.extension().and_then(|s|s.to_str()).unwrap_or("").to_ascii_lowercase();
                    if extension=="m4p" {return Err("Protected media; obtain an unprotected local file".into());}
                    if !matches!(extension.as_str(),"wav"|"mp3"|"flac"|"ogg"|"aiff"|"aif"|"m4a"|"aac") {return Err("Unsupported local audio format".into());}
                    let location=snapshot.identify(&path).map_err(|e|format!("Missing/unmapped local file: {e}"))?;
                    if location.path.to_str().is_none() || location.path.as_os_str().len()>4096 {return Err("Local path exceeds the UTF-8 catalog limit".into());}
                    let meta=snapshot.inspect(&location).map_err(|e|e.to_string())?;
                    if !meta.is_file() {return Err("Reference is not a regular local audio file".into());}
                    let fingerprint=FileFingerprint::from_metadata(&meta);
                    let existing=catalog.track_for_version(&location.source,Some(fingerprint)).or_else(||catalog.track(&location.source));
                    if existing.is_some_and(|track|track.versions[track.current].fingerprint!=Some(fingerprint)) {return Err("Catalog version differs; resolve changed media before importing this reference".into());}
                    let observed=crate::media_tags::inspect_cancellable(&location,fingerprint,&Cancel(&check)).map_err(|e|format!("Audio container could not be inspected: {e}"))?;
                    let filename=location.path.file_stem().and_then(|s|s.to_str()).unwrap_or("Track");
                    let fields=observed.fields;
                    let metadata=Metadata{title:label(fields.title.as_ref().map(|f|f.value.as_str()).unwrap_or(if entry.title.is_empty(){filename}else{&entry.title}))?,artist:label(fields.artist.as_ref().map(|f|f.value.as_str()).unwrap_or(&entry.artist))?,bpm:fields.bpm.as_ref().and_then(|f|f.value.parse::<f32>().ok()).map_or(Bpm::UNKNOWN,|n|Bpm::new(n,crate::ui::bpm::Origin::EmbeddedTag)),key:fields.key.map(|f|f.value).unwrap_or_default(),duration:None,last_play:None};
                    Ok(Arc::new(Media{location,fingerprint,metadata,existing:existing.map(|t|t.id.clone())}))
                })();cache.insert(entry.reference.clone(),result.clone());result
            };
            let (media,status)=match result {Ok(media)=>{let duplicate=!seen.insert(media.location.source.clone());let status=if duplicate {"Duplicate reference; first position retained"} else if media.existing.is_some(){"Ready: existing catalog track"}else{"Ready: new local track"};(Some(media),status.into())},Err(error)=>(None,error)};
            let title=media.as_ref().map(|m|m.metadata.title.clone()).unwrap_or(entry.title);
            indices.push(rows.len());rows.push(Row{title,reference:entry.reference,status,media});
        }
        playlists.push(Playlist{name:name(&playlist.name)?,rows:indices,note:playlist.note});
    }
    active(&check)?;Ok(Review{path:input.path.clone(),fingerprint,playlists,rows})
}

/// Apply an exact reviewed import to a private candidate. Takes the review, selected playlist indices, catalog and cancellation check; returns fresh crate IDs or refuses all changes. The sole catalog owner publishes the candidate afterward.
pub(crate) fn apply(review: &Review, selected: &[usize], catalog: &mut Catalog, check: impl Fn() -> bool) -> Result<Vec<CrateId>, String> {
    active(&check)?;
    if selected.is_empty() || selected.len()>MAX_PLAYLISTS || selected.iter().any(|&i|i>=review.playlists.len()) || selected.iter().copied().collect::<HashSet<_>>().len()!=selected.len() {return Err("Select distinct reviewed playlists".into());}
    if FileFingerprint::read(&review.path)!=Some(review.fingerprint) {return Err("Playlist changed after review; inspect it again".into());}
    let wanted:HashSet<_>=selected.iter().flat_map(|&i|review.playlists[i].rows.iter()).filter_map(|&i|review.rows[i].media.as_ref()).map(|m|m.location.source.clone()).collect();
    if wanted.is_empty() {return Err("Selected playlists contain no resolved local audio".into());}
    let mut tracks=HashMap::new();
    for media in review.rows.iter().filter_map(|r|r.media.as_ref()) {
        active(&check)?;if !wanted.contains(&media.location.source) || tracks.contains_key(&media.location.source) {continue;}
        let file=OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW|libc::O_NONBLOCK).open(&media.location.path).map_err(|e|e.to_string())?;
        media.location.verify_file(&file,media.fingerprint).map_err(|e|format!("Reviewed media changed: {e}"))?;
        let known=catalog.track_for_version(&media.location.source,Some(media.fingerprint)).or_else(||catalog.track(&media.location.source));
        match (media.existing.as_ref(),known) {
            (Some(expected),Some(track)) if &track.id==expected && track.versions[track.current].fingerprint==Some(media.fingerprint)=>{},
            (None,None)=>{catalog.upsert(media.location.source.clone(),Some(media.fingerprint),media.metadata.clone())?;},
            _=>return Err("Catalog source/version changed after review; inspect the playlist again".into()),
        }
        let track=catalog.track_for_version(&media.location.source,Some(media.fingerprint)).unwrap();tracks.insert(media.location.source.clone(),track.id.clone());
    }
    let mut created=Vec::new();
    for &i in selected {
        active(&check)?;let playlist=&review.playlists[i];let mut seen=HashSet::new();
        let members:Vec<_>=playlist.rows.iter().filter_map(|&r|review.rows[r].media.as_ref()).filter_map(|m|tracks.get(&m.location.source)).filter(|id|seen.insert((*id).clone())).cloned().collect();
        if members.is_empty() {continue;}
        let mut bytes=[0u8;16];std::fs::File::open("/dev/urandom").and_then(|mut f|f.read_exact(&mut bytes)).map_err(|e|e.to_string())?;
        let id=CrateId(bytes.iter().map(|b|format!("{b:02x}")).collect());
        catalog.edit_crates(catalog.crates.revision(),&Edit::Create{id:id.clone(),name:playlist.name.clone(),parent:None,before:None})?;
        catalog.edit_crates(catalog.crates.revision(),&Edit::AddMembers{id:id.clone(),members,before:None})?;created.push(id);
    }
    active(&check)?;Ok(created)
}
