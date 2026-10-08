use super::*;
use crate::{
    engine::{
        beatgrid::Grid,
        cue_metadata::{Name, Style},
        preparation::Preparation,
        saved_loops::Slot,
    },
    interchange_xml::Element,
    library::{annotations::Annotations, imports::Details},
};

fn number(node: &Element, key: &str) -> Result<f64, String> {
    let value = node
        .attr(key)
        .parse::<f64>()
        .map_err(|_| format!("Invalid {} {key}", node.name))?;
    if !value.is_finite() || !(0.0..=1.0e10).contains(&value) {
        return Err(format!("Out-of-range {} {key}", node.name));
    }
    Ok(value)
}
fn count(node: &Element, key: &str, actual: usize) -> Result<(), String> {
    if !node.attr(key).is_empty() && node.attr(key).parse::<usize>().ok() != Some(actual) {
        return Err(format!("{} {key} differs from actual entries", node.name));
    }
    Ok(())
}
fn optional_bpm(value: &str) -> Result<Option<f32>, String> {
    if value.is_empty() || value == "0" || value == "0.000000" {
        return Ok(None);
    }
    let bpm = value.parse::<f32>().map_err(|_| "Invalid imported BPM")?;
    if !bpm.is_finite() || !(20.0..=400.0).contains(&bpm) {
        return Err("Imported BPM must be 20–400".into());
    }
    Ok(Some(bpm))
}
fn color(value: &str) -> Result<Option<[u8; 3]>, String> {
    if value.is_empty() {
        return Ok(None);
    }
    let value = value.trim_start_matches("0x").trim_start_matches('#');
    let number = u32::from_str_radix(value, 16).map_err(|_| "Invalid imported RGB color")?;
    if number > 0xffffff {
        return Err("Imported color exceeds RGB".into());
    }
    Ok(Some([
        (number >> 16) as u8,
        (number >> 8) as u8,
        number as u8,
    ]))
}
fn cue_style(node: &Element, key: &str, details: &mut Details) -> Result<Style, String> {
    let value = node.attr(key);
    let name = match Name::new(value) {
        Ok(n) => n,
        Err(_) => {
            details.warnings.push(format!(
                "Cue label exceeds native 64-byte limit: {}",
                label(value)?
            ));
            Name::default()
        }
    };
    let color = if node.attr("Red").is_empty() {
        None
    } else {
        let component = |key| {
            node.attr(key)
                .parse::<u8>()
                .map_err(|_| "Invalid cue RGB component")
        };
        Some([component("Red")?, component("Green")?, component("Blue")?])
    };
    Ok(Style { name, color })
}
fn unsupported_attributes(
    node: &Element,
    supported: &[&str],
    details: &mut Details,
) -> Result<(), String> {
    for (key, value) in &node.attributes {
        if value.is_empty() || supported.contains(&key.as_str()) {
            continue;
        }
        let prefix = format!("Source-only {} {key}: ", node.name);
        let mut end = (4000 - prefix.len()).min(value.len());
        while !value.is_char_boundary(end) {
            end -= 1;
        }
        details.warnings.push(format!(
            "{prefix}{}{}",
            &value[..end],
            if end < value.len() {
                " (truncated in report; source unchanged)"
            } else {
                ""
            }
        ));
    }
    if details.warnings.len() > 64 {
        return Err("Imported metadata exceeds 64 source-only notices; export a smaller track metadata selection".into());
    }
    Ok(())
}
fn marker(
    preparation: &mut Preparation,
    details: &mut Details,
    slot: i32,
    start: f64,
    end: Option<f64>,
    style: Style,
) -> Result<(), String> {
    if slot < -1 {
        return Err("Imported cue index is invalid".into());
    }
    if slot >= 8 {
        details.warnings.push(format!(
            "Cue {slot} at {start}s exceeds eight native hot cues"
        ));
        return Ok(());
    }
    if let Some(end) = end {
        if end <= start {
            return Err("Imported loop end must follow its start".into());
        }
        let index = if slot >= 0 {
            Some(slot as usize)
        } else {
            preparation
                .saved_loops
                .slots
                .iter()
                .position(Option::is_none)
        };
        if let Some(index) = index {
            if preparation.saved_loops.slots[index].is_some() {
                return Err("Repeated imported loop slot".into());
            }
            preparation.saved_loops.slots[index] = Some(Slot {
                start,
                length: end - start,
                style,
            });
            if preparation.loop_region.is_none() {
                preparation.loop_region = Some(crate::engine::preparation::Loop {
                    start,
                    length: end - start,
                    enabled: false,
                });
            }
            if slot >= 0 {
                preparation.saved_loops.cue_loops[index] = Some(index as u8 + 1);
            }
        } else {
            details.warnings.push(format!(
                "Loop {start}–{end}s exceeds eight saved loop slots"
            ));
        }
    }
    if slot >= 0 {
        let index = slot as usize;
        if preparation.hotcues[index].is_some() {
            return Err("Repeated imported hot cue slot".into());
        }
        preparation.hotcues[index] = Some(start);
        preparation.hotcue_styles[index] = style;
    } else if end.is_none() {
        details.warnings.push(format!(
            "Memory cue retained in source report: {start}s ({})",
            style.name.as_str()
        ));
    }
    Ok(())
}

fn rekordbox_details(node: &Element) -> Result<Details, String> {
    let mut details = Details {
        bpm: optional_bpm(node.attr("AverageBpm"))?,
        key: label(node.attr("Tonality"))?,
        ..Default::default()
    };
    let rating = if node.attr("Rating").is_empty() {
        0
    } else {
        node.attr("Rating")
            .parse::<u16>()
            .map_err(|_| "Invalid rekordbox rating")?
    };
    if rating > 255 || rating % 51 != 0 {
        return Err("Rekordbox rating must be 0, 51, 102, 153, 204 or 255".into());
    }
    details.annotations = Some(Annotations {
        rating: (rating / 51) as u8,
        color: color(node.attr("Colour"))?,
        group: label(node.attr("Grouping"))?,
        notes: label(node.attr("Comments"))?,
        tags: if node.attr("Genre").is_empty() {
            vec![]
        } else {
            vec![label(node.attr("Genre"))?]
        },
    });
    unsupported_attributes(
        node,
        &[
            "TrackID",
            "Name",
            "Artist",
            "Location",
            "AverageBpm",
            "Tonality",
            "Rating",
            "Genre",
            "Colour",
            "Grouping",
            "Comments",
        ],
        &mut details,
    )?;
    let mut preparation = Preparation::default();
    let mut tempos = Vec::new();
    for tempo in node.named("TEMPO") {
        let bpm = optional_bpm(tempo.attr("Bpm"))?.ok_or("Beatgrid tempo is missing")? as f64;
        let beat = tempo
            .attr("Battito")
            .parse::<u8>()
            .map_err(|_| "Invalid rekordbox bar beat")?;
        if !(1..=4).contains(&beat) || tempo.attr("Metro") != "4/4" {
            details.warnings.push("Non-4/4 rekordbox meter is retained in the source; native phrase grid requires 4/4".into());
            tempos.clear();
            break;
        }
        tempos.push((number(tempo, "Inizio")?, bpm, beat));
        if tempos.len() > 63 {
            return Err("Rekordbox grid exceeds 63 tempo changes".into());
        }
    }
    if let Some(&(first, bpm, bar_beat)) = tempos.first() {
        let mut grid =
            Grid::new(first - f64::from(bar_beat - 1) * 60.0 / bpm, bpm).map_err(str::to_owned)?;
        let mut prior = (first, bpm, f64::from(bar_beat - 1));
        for &(seconds, bpm, _) in &tempos[1..] {
            if seconds <= prior.0 {
                return Err("Rekordbox tempo changes must be ordered".into());
            }
            let beat = prior.2 + (seconds - prior.0) * prior.1 / 60.0;
            grid = grid.with_anchor(beat, seconds).map_err(str::to_owned)?;
            prior = (seconds, bpm, beat);
        }
        if tempos.len() > 1 {
            grid = grid
                .with_anchor(prior.2 + 1.0, prior.0 + 60.0 / prior.1)
                .map_err(str::to_owned)?;
        }
        preparation.grid = Some(grid);
    }
    for cue in node.named("POSITION_MARK") {
        let start = number(cue, "Start")?;
        let slot = cue
            .attr("Num")
            .parse::<i32>()
            .map_err(|_| "Invalid rekordbox cue index")?;
        let style = cue_style(cue, "Name", &mut details)?;
        match cue.attr("Type") {
            "0" => marker(&mut preparation, &mut details, slot, start, None, style)?,

            "4" => marker(
                &mut preparation,
                &mut details,
                slot,
                start,
                Some(number(cue, "End")?),
                style,
            )?,
            kind => details.warnings.push(format!(
                "Unsupported rekordbox marker type {kind} at {start}s"
            )),
        }
    }
    for child in &node.children {
        if !matches!(child.name.as_str(), "TEMPO" | "POSITION_MARK") {
            unsupported_attributes(child, &[], &mut details)?;
        }
    }
    if preparation != Preparation::default() {
        details.preparation = Some(Arc::new(preparation));
    }
    details.validate()?;
    Ok(details)
}

fn entry(node: &Element, reference: String, details: Details) -> Result<RawEntry, String> {
    Ok(RawEntry {
        reference,
        title: label(if node.name == "TRACK" {
            node.attr("Name")
        } else {
            node.attr("TITLE")
        })?,
        artist: label(if node.name == "TRACK" {
            node.attr("Artist")
        } else {
            node.attr("ARTIST")
        })?,
        blocked: None,
        details: Arc::new(details),
    })
}
fn absent(reference: &str) -> RawEntry {
    RawEntry {
        reference: reference.into(),
        title: String::new(),
        artist: String::new(),
        blocked: Some("Playlist identity is absent from its source collection"),
        details: Default::default(),
    }
}

fn tree(
    node: &Element,
    folders: &[String],
    key: &str,
    root: bool,
    tracks: &HashMap<String, RawEntry>,
    traktor: bool,
    result: &mut Vec<RawPlaylist>,
    check: &impl Fn() -> bool,
) -> Result<(), String> {
    active(check)?;
    if result.len() >= MAX_PLAYLISTS || folders.len() >= crate::library::crates::MAX_DEPTH {
        return Err("Playlist tree exceeds 128 nodes or 32 levels".into());
    }
    let folder = if traktor {
        match node.attr("TYPE") {
            "FOLDER" => true,
            "PLAYLIST" => false,
            _ => return Err("Unsupported Traktor playlist node".into()),
        }
    } else {
        match node.attr("Type") {
            "0" => true,
            "1" => false,
            _ => return Err("Unsupported rekordbox playlist node".into()),
        }
    };
    let title = if traktor {
        node.attr("NAME")
    } else {
        node.attr("Name")
    };
    let mut path = folders.to_vec();
    if !root {
        result.push(RawPlaylist {name:name(title)?,entries:Vec::new(),folders:path.clone(),key:key.into(),parent:key.rsplit_once('/').filter(|(parent,_)|*parent!="root").map(|(parent,_)|parent.to_owned()),folder,note:if traktor {"Traktor NML 19; ordered membership and folders. Saved native track preparation wins conflicts."}else{"Rekordbox XML 1.0.0; ordered membership and folders. Saved native track preparation wins conflicts."}.into()});
    }
    if folder {
        if !root {
            path.push(name(title)?);
        }
        let children = if traktor { node.one("SUBNODES")? } else { node };
        count(
            children,
            if traktor { "COUNT" } else { "Count" },
            children.named("NODE").count(),
        )?;
        for (index, child) in children.named("NODE").enumerate() {
            tree(
                child,
                &path,
                &format!("{key}/{index}"),
                false,
                tracks,
                traktor,
                result,
                check,
            )?;
        }
    } else {
        if root {
            return Err("Playlist root must be a folder".into());
        }
        let content = if traktor { node.one("PLAYLIST")? } else { node };
        if traktor && content.attr("TYPE") != "LIST" {
            return Err("Unsupported Traktor smart/dynamic playlist; export a static list".into());
        }
        let kind = if traktor { "ENTRY" } else { "TRACK" };
        count(
            content,
            if traktor { "ENTRIES" } else { "Entries" },
            content.named(kind).count(),
        )?;
        let mut entries = Vec::new();
        for item in content.named(kind) {
            active(check)?;
            let reference = if traktor {
                let key = item.one("PRIMARYKEY")?;
                if key.attr("TYPE") != "TRACK" {
                    return Err("Unsupported Traktor playlist key type".into());
                }
                key.attr("KEY")
            } else {
                item.attr("Key")
            };
            let lookup = if traktor {
                reference.to_owned()
            } else {
                match node.attr("KeyType") {
                    "0" => format!("id:{reference}"),
                    "1" => format!("path:{reference}"),
                    _ => return Err("Unsupported rekordbox playlist KeyType".into()),
                }
            };
            if entries.len() >= MAX_REFERENCES {
                return Err("Playlist exceeds 4096 references".into());
            }
            entries.push(
                tracks
                    .get(&lookup)
                    .cloned()
                    .unwrap_or_else(|| absent(reference)),
            );
        }
        result.last_mut().unwrap().entries = entries;
    }
    Ok(())
}

fn traktor_details(node: &Element) -> Result<Details, String> {
    let tempo = node.named("TEMPO").next();
    let info = node.named("INFO").next();
    let mut details = Details {
        bpm: tempo
            .map(|n| optional_bpm(n.attr("BPM")))
            .transpose()?
            .flatten(),
        ..Default::default()
    };
    if let Some(info) = info {
        details.key = label(info.attr("KEY"))?;
        details.annotations = Some(Annotations {
            notes: label(info.attr("COMMENT"))?,
            tags: if info.attr("GENRE").is_empty() {
                vec![]
            } else {
                vec![label(info.attr("GENRE"))?]
            },
            ..Default::default()
        });
        if !info.attr("RANKING").is_empty() {
            details.warnings.push(format!(
                "Traktor ranking retained in source: {}",
                info.attr("RANKING")
            ));
        }
    }
    if let Some(key) = node.named("MUSICAL_KEY").next() {
        details.warnings.push(format!(
            "Traktor numeric key retained in source: {}",
            key.attr("VALUE")
        ));
    }
    unsupported_attributes(node, &["TITLE", "ARTIST"], &mut details)?;
    if let Some(info) = info {
        unsupported_attributes(info, &["KEY", "COMMENT", "GENRE", "RANKING"], &mut details)?;
    }
    for child in &node.children {
        if !matches!(
            child.name.as_str(),
            "LOCATION" | "INFO" | "TEMPO" | "MUSICAL_KEY" | "CUE_V2"
        ) {
            unsupported_attributes(child, &[], &mut details)?;
        }
    }
    let mut preparation = Preparation::default();
    for cue in node.named("CUE_V2") {
        let start = number(cue, "START")? / 1000.0;
        let slot = cue
            .attr("HOTCUE")
            .parse::<i32>()
            .map_err(|_| "Invalid Traktor hot cue index")?;
        let style = cue_style(cue, "NAME", &mut details)?;
        match cue.attr("TYPE") {
            "0" => marker(&mut preparation, &mut details, slot, start, None, style)?,
            "3" => preparation.cue = start,
            "4" => {
                if preparation.grid.is_some() {
                    details.warnings.push(format!("Additional Traktor grid marker at {start}s is not converted without a tempo map"));
                } else if let Some(bpm) = details.bpm {
                    preparation.grid = Some(Grid::new(start, bpm as f64).map_err(str::to_owned)?);
                }
            }
            "5" => marker(
                &mut preparation,
                &mut details,
                slot,
                start,
                Some(start + number(cue, "LEN")? / 1000.0),
                style,
            )?,
            kind => details
                .warnings
                .push(format!("Unsupported Traktor cue type {kind} at {start}s")),
        }
    }
    if preparation != Preparation::default() {
        details.preparation = Some(Arc::new(preparation));
    }
    details.validate()?;
    Ok(details)
}

/// Decode documented rekordbox XML or Traktor NML 19 exports.
/// Takes a bounded parsed tree and cancellation predicate; returns source-version diagnostics and ordered playlists with reviewed musical metadata.
pub(super) fn parse(
    root: &Element,
    check: &impl Fn() -> bool,
) -> Result<(String, Vec<RawPlaylist>), String> {
    let traktor = match root.name.as_str() {
        "DJ_PLAYLISTS" if root.attr("Version") == "1.0.0" => false,
        "NML" if root.attr("VERSION") == "19" => true,
        "DJ_PLAYLISTS" | "NML" => {
            return Err(format!(
                "Unsupported {} version; export rekordbox XML 1.0.0 or Traktor NML 19",
                root.name
            ))
        }
        _ => {
            return Err("Unknown DJ XML export; use documented rekordbox XML or Traktor NML".into())
        }
    };
    let collection = root.one("COLLECTION")?;
    let kind = if traktor { "ENTRY" } else { "TRACK" };
    count(
        collection,
        if traktor { "ENTRIES" } else { "Entries" },
        collection.named(kind).count(),
    )?;
    let mut tracks = HashMap::new();
    let mut metadata_bytes = 0usize;
    for node in collection.named(kind) {
        active(check)?;
        if tracks.len() > 100_000 {
            return Err("DJ collection exceeds 100000 records".into());
        }
        if traktor {
            let location = node.one("LOCATION")?;
            let directory = location.attr("DIR");
            let file = location.attr("FILE");
            let volume = location.attr("VOLUME");
            if !directory.starts_with("/:")
                || !directory.ends_with("/:")
                || file.is_empty()
                || file.contains('/')
                || file.contains('\\')
            {
                return Err("Invalid Traktor encoded directory or basename".into());
            }
            let key = format!("{volume}{directory}{file}");
            let reference = if volume.is_empty() {
                format!("{}{file}", directory.replace("/:", "/"))
            } else {
                format!("{volume}{}{file}", directory.replace("/:", "/"))
            };
            let mut raw = entry(node, reference, traktor_details(node)?)?;
            if !volume.is_empty() && !volume.ends_with(':') {
                raw.blocked = Some("Traktor volume name needs an explicit path prefix mapping");
            }
            metadata_bytes += retained(&raw);
            if metadata_bytes > 32 * 1024 * 1024 {
                return Err(
                    "DJ collection metadata exceeds 32 MiB; export a smaller selection".into(),
                );
            }
            if tracks.insert(key, raw).is_some() {
                return Err("Duplicate Traktor source identity".into());
            }
        } else {
            let id = node.attr("TrackID");
            let location = node.attr("Location");
            if id.parse::<u64>().is_err() || location.is_empty() {
                return Err("Rekordbox track needs an integer TrackID and Location".into());
            }
            let raw = entry(node, location.into(), rekordbox_details(node)?)?;
            metadata_bytes += 2 * retained(&raw);
            if metadata_bytes > 32 * 1024 * 1024 {
                return Err(
                    "DJ collection metadata exceeds 32 MiB; export a smaller selection".into(),
                );
            }
            if tracks.insert(format!("id:{id}"), raw.clone()).is_some()
                || tracks.insert(format!("path:{location}"), raw).is_some()
            {
                return Err("Duplicate rekordbox track identity/location".into());
            }
        }
    }
    let root_node = root.one("PLAYLISTS")?.one("NODE")?;
    let mut result = Vec::new();
    tree(
        root_node,
        &[],
        "root",
        true,
        &tracks,
        traktor,
        &mut result,
        check,
    )?;
    Ok((
        if traktor {
            "Traktor NML 19"
        } else {
            "rekordbox XML 1.0.0"
        }
        .into(),
        result,
    ))
}

fn retained(entry: &RawEntry) -> usize {
    let details = &entry.details;
    std::mem::size_of::<RawEntry>()
        + std::mem::size_of::<Details>()
        + entry.reference.capacity()
        + entry.title.capacity()
        + entry.artist.capacity()
        + details.key.capacity()
        + details
            .preparation
            .as_ref()
            .map_or(0, |_| std::mem::size_of::<Preparation>())
        + details.annotations.as_ref().map_or(0, |a| {
            a.group.capacity()
                + a.notes.capacity()
                + a.tags.capacity() * std::mem::size_of::<String>()
                + a.tags.iter().map(String::capacity).sum::<usize>()
        })
        + details.warnings.capacity() * std::mem::size_of::<String>()
        + details.warnings.iter().map(String::capacity).sum::<usize>()
        + 128
}
