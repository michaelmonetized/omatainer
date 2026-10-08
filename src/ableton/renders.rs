use super::*;
use crate::engine::{
    arrangement,
    audio::routing::model::{ChannelMap, Connection, Direction, Group, Source as TapSource, Tap},
    audio_clip,
};

/// Record an authorized aligned audio replacement.
/// Keeps original editable tracks and devices alongside the rendered track, source identity, timing and printed-processing decision.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Record {
    pub target: Option<session::Reference>,
    pub audio_track: session::Reference,
    pub muted_tracks: Vec<session::Reference>,
    pub source: LibSource,
    pub fingerprint: FileFingerprint,
    pub audio_sha256: String,
    pub start: f64,
    pub body_beats: f64,
    pub duration: f64,
    pub tail_seconds: f64,
    pub includes_returns_master: bool,
    pub original_mutes: Vec<bool>,
    pub original_connections: Vec<Connection>,
    pub original_default_sends: Vec<session::Id>,
    pub printed_routing_sha256: String,
    pub restorable: bool,
}
impl Record {
    /// Validate a saved render receipt.
    /// Takes retained provenance; refuses corrupt identities, timing, hashes and inconsistent printed-processing scope.
    pub(crate) fn validate(&self) -> Result<(), String> {
        crate::media_location::validate_root_source(&self.source).map_err(|e| e.to_string())?;
        let valid = |r: session::Reference| r.id.0 != 0 && r.namespace != [0, 0];
        if !valid(self.audio_track)
            || self.target.is_some_and(|r| !valid(r))
            || self.muted_tracks.is_empty()
            || self.muted_tracks.len() > session::MAX_TRACKS
            || self.muted_tracks.iter().any(|r| !valid(*r))
            || self
                .muted_tracks
                .iter()
                .collect::<std::collections::HashSet<_>>()
                .len()
                != self.muted_tracks.len()
            || self.audio_sha256.len() != 64
            || !self.audio_sha256.bytes().all(|b| b.is_ascii_hexdigit())
            || ![
                self.start,
                self.body_beats,
                self.duration,
                self.tail_seconds,
            ]
            .into_iter()
            .all(f64::is_finite)
            || !(0.0..=262144.).contains(&self.start)
            || self.body_beats <= 0.
            || self.duration < self.body_beats
            || self.start + self.duration > 262144.
            || !(0.0..=3600.).contains(&self.tail_seconds)
            || self.includes_returns_master && self.target.is_some()
            || self.original_mutes.len() != self.muted_tracks.len()
            || self.original_connections.len() > engine::audio::routing::model::MAX_CONNECTIONS
            || self
                .original_connections
                .iter()
                .map(|r| r.map.len())
                .sum::<usize>()
                > engine::audio::routing::model::MAX_MAPS
            || self.original_default_sends.len() > session::MAX_TRACKS
            || self.printed_routing_sha256.len() != 64
            || !self
                .printed_routing_sha256
                .bytes()
                .all(|b| b.is_ascii_hexdigit())
        {
            return Err("Invalid aligned render provenance".into());
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Options {
    pub target: Option<i64>,
    pub start: f64,
    pub body_beats: f64,
    pub includes_returns_master: bool,
}

/// Attach reviewed audio without replacing editable source clips.
/// Takes a native draft, authorized mono/stereo export, original track or complete-mix scope, start/body bounds and printed-processing decision; returns a fully validated draft with original routes silenced and tails preserved.
pub(crate) fn attach(
    mut draft: Imported,
    path: &Path,
    options: &Options,
    cancel: &AtomicBool,
) -> Result<Imported, String> {
    active(cancel)?;
    if !path.is_absolute()
        || path.to_str().is_none_or(|p| !text_valid(p))
        || !options.start.is_finite()
        || !options.body_beats.is_finite()
        || options.start < 0.
        || options.body_beats <= 0.
        || options.start + options.body_beats > 262144.
        || options.includes_returns_master && options.target.is_some()
    {
        return Err("Choose an absolute owned render, finite start/body length, and track exports without baked returns/master; use complete mix for baked master processing".into());
    }
    let origin = draft
        .state
        .migration
        .as_ref()
        .and_then(|m| m.sources.first())
        .ok_or("Imported source archive is absent")?;
    if !origin.renders.is_empty() {
        return Err("Restore the pre-render draft before choosing another render; an existing print must not play twice".into());
    }
    let layout = draft
        .state
        .session
        .as_ref()
        .ok_or("Native session identities unavailable")?;
    if layout.tracks.len() >= session::MAX_TRACKS {
        return Err("No room for an editable render track; import fewer tracks".into());
    }
    let target = options
        .target
        .map(|id| {
            origin
                .tracks
                .iter()
                .find(|t| t.source_id == id)
                .ok_or("Source track changed")
        })
        .transpose()?;
    if target.is_some_and(|t| !matches!(t.role.as_str(), "MidiTrack" | "AudioTrack")) {
        return Err("Group and return prints require a complete-mix render to avoid duplicate child/return audio".into());
    }
    let target_ref = target.map(|t| t.native);
    let original_tracks: Vec<_> = origin
        .tracks
        .iter()
        .filter(|t| target_ref.is_none_or(|r| r == t.native))
        .map(|t| t.native)
        .collect();
    let mut graph = draft
        .state
        .routing
        .as_deref()
        .cloned()
        .ok_or("Imported routing is absent")?;
    let original_connections = graph.connections.clone();
    let original_default_sends = graph.tracks_without_default_send.clone();
    let original_mutes = original_tracks
        .iter()
        .map(|r| draft.state.tracks[layout.resolve(session::Axis::Track, r.id).unwrap()].mute)
        .collect();
    let mut outgoing = Vec::new();
    if let Some(target) = target_ref {
        let ids: BTreeSet<_> = origin
            .devices
            .iter()
            .filter(|d| {
                origin
                    .tracks
                    .iter()
                    .any(|t| t.source_id == d.track && t.native == target)
            })
            .filter_map(|d| d.resolution.as_ref().map(|r| r.native_id))
            .collect();
        let owned = |g: Group| {
            g == Group::Track(target.id) || matches!(g,Group::Plugin(id) if ids.contains(&id))
        };
        for route in &graph.connections {
            if owned(route.source.group) && !owned(route.destination) {
                if route.source.tap != Tap::PostMixer || route.map.iter().any(|m| m.source > 1) {
                    return Err("A printed track cannot reproduce its pre-fader or additional-output sends; export a complete mix or resolve those routes explicitly".into());
                }
                outgoing.push(route.clone());
            }
        }
        graph
            .connections
            .retain(|r| !owned(r.source.group) || owned(r.destination));
        if outgoing.is_empty() {
            return Err("Source track has no qualified output route; review routing before attaching a track render".into());
        }
    } else {
        graph.connections.retain(|r| {
            !matches!(r.destination, Group::Main | Group::Output(_))
                || r.source.group == Group::Main
        });
    }
    let clock = draft
        .state
        .conductor
        .as_ref()
        .ok_or("Imported tempo map is absent")?
        .prepare()?;
    let snapshot = crate::media_location::Snapshot::discover().map_err(|e| e.to_string())?;
    let location = snapshot.identify(path).map_err(|e| e.to_string())?;
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK | libc::O_CLOEXEC)
        .open(&location.path)
        .map_err(|e| e.to_string())?;
    let meta = file.metadata().map_err(|e| e.to_string())?;
    if !meta.is_file() {
        return Err("Render must be a regular owned audio file".into());
    }
    let fingerprint = FileFingerprint::from_metadata(&meta);
    let check = file.try_clone().map_err(|e| e.to_string())?;
    let used = draft
        .media
        .iter()
        .try_fold(0u64, |n, s| n.checked_add(s.data.len() as u64 * 4))
        .ok_or("Render media storage overflow")?;
    let decoded = engine::decode::decode_sampler_file(
        &location.path,
        file,
        MAX_PCM.saturating_sub(used),
        || cancel.load(Ordering::Acquire),
    )
    .map_err(|e| e.to_string())?;
    location
        .verify_file(&check, fingerprint)
        .map_err(|e| e.to_string())?;
    let sample = Arc::new(decoded.sample);
    let seconds = sample.frames() as f64 / f64::from(sample.sr);
    let start_seconds = clock.seconds_at(options.start);
    let body_seconds = clock.seconds_at(options.start + options.body_beats) - start_seconds;
    if seconds + 0.5 / f64::from(sample.sr) < body_seconds {
        return Err("Render ends before the declared musical body; choose the correct export or shorten its reviewed body range".into());
    }
    let duration = clock.beat_at_seconds(start_seconds + seconds) - options.start;
    let tail_seconds = (seconds - body_seconds).max(0.);
    if options.start + duration > 262144. || tail_seconds > 3600. {
        return Err("Render timeline or tail exceeds its supported range".into());
    }
    let audio_sha256 = crate::project_dependencies::audio_hash(&sample, cancel)?
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let name = target.map_or("Live render · complete mix".into(), |t| {
        format!(
            "Live render · {}",
            draft.state.tracks[layout.resolve(session::Axis::Track, t.native.id).unwrap()].name
        )
    });
    let mut track = draft.state.tracks[0].clone();
    track.name = name.clone();
    track.kind = 0;
    track.gain = 1.;
    track.pan = 0.;
    track.mute = false;
    track.solo = false;
    track.armed = false;
    track.input_monitor = Some(engine::input_monitor::Mode::Off);
    track.fx.clear();
    track.eq = [1.; 3];
    track.synth.offline = None;
    track.clips = vec![project::SavedClip::empty(); layout.scenes.len()];
    let layout = draft.state.session.as_mut().unwrap();
    let slot = layout.tracks.len();
    let id = session::Id(layout.next_id);
    layout.next_id = layout
        .next_id
        .checked_add(1)
        .ok_or("Session identities exhausted")?;
    let mut item = layout.tracks[0].clone();
    item.id = id;
    item.active = true;
    item.name = name.clone();
    item.color = None;
    item.scene = Default::default();
    layout.tracks.push(item);
    layout.track_order.push(slot as u8);
    let audio_track = layout
        .reference(session::Axis::Track, slot)
        .ok_or("Cannot allocate render track identity")?;
    for original in &original_tracks {
        draft.state.tracks[layout
            .resolve(session::Axis::Track, original.id)
            .ok_or("Original render track is absent")?]
        .mute = true;
    }
    draft.state.tracks.push(track);
    let region = audio_clip::Region::full(&sample, draft.state.bpm).map_err(str::to_owned)?;
    let mut clip = project::SavedClip::empty();
    clip.kind = engine::ClipKind::Audio;
    clip.name = name;
    clip.audio = Some(draft.media.len());
    clip.audio_region = Some(region);
    clip.bars = (region
        .prepare(&sample)
        .map_err(str::to_owned)?
        .duration_beats
        / 4.) as f32;
    draft.media.push(sample);
    let song = Arc::make_mut(
        draft
            .state
            .arrangement
            .get_or_insert_with(|| Arc::new(Default::default())),
    );
    let source_id = song.identity()?;
    let instance_id = song.identity()?;
    song.sources.push(arrangement::Source {
        id: source_id,
        clip,
        audio_clock: Some(arrangement::AudioClock {
            origin: options.start,
            conductor: clock,
        }),
    });
    song.instances.push(arrangement::Instance {
        id: instance_id,
        source: source_id,
        track: audio_track,
        start: options.start,
        offset: 0.,
        duration,
        repeating: false,
        gain: 1.,
        fades: None,
        fade_link: 0,
        crossfade: None,
    });
    song.enabled = true;
    graph.tracks_without_default_send.push(id);
    if target_ref.is_some() {
        for route in &mut outgoing {
            route.source = TapSource {
                group: Group::Track(id),
                tap: Tap::PostMixer,
            };
        }
        graph.connections.extend(outgoing);
    } else {
        let destination = if options.includes_returns_master {
            let port = graph
                .ports
                .iter()
                .find(|p| p.direction == Direction::Output && p.channels.len() == 2)
                .ok_or("Complete printed master needs an explicit stereo physical output alias")?;
            Group::Output(port.id)
        } else {
            Group::Main
        };
        graph.connections.push(Connection {
            source: TapSource {
                group: Group::Track(id),
                tap: Tap::PostMixer,
            },
            destination,
            map: (0..2)
                .map(|n| ChannelMap {
                    source: n,
                    destination: n,
                    gain: 1.,
                })
                .collect(),
        });
    }
    graph.order(layout)?;
    let printed_routing_sha256 = routing_hash(&graph)?;
    draft.state.routing = Some(Arc::new(graph));
    let record = Record {
        target: target_ref,
        audio_track,
        muted_tracks: original_tracks,
        source: location.source,
        fingerprint,
        audio_sha256,
        start: options.start,
        body_beats: options.body_beats,
        duration: duration.max(options.body_beats),
        tail_seconds,
        includes_returns_master: options.includes_returns_master,
        original_connections,
        original_default_sends,
        original_mutes,
        printed_routing_sha256,
        restorable: true,
    };
    record.validate()?;
    let source = &mut Arc::make_mut(draft.state.migration.as_mut().unwrap()).sources[0];
    source.renders.push(record);
    source.difference("render","Aligned audio","Original MIDI, clips and devices stay editable and muted. Printed track effects/mixer are bypassed; baked return/master audio uses a direct stereo output. Source tempo ramps select original rendered sample positions, tails remain in the arrangement, and the native output safety limiter still applies")?;
    draft.state.version = project::STATE_VERSION;
    draft.state.validate(&draft.media)?;
    crate::project_file::validate_metadata(
        &crate::project_file::Bundle {
            state: &draft.state,
            media: draft.media.clone(),
        },
        &Default::default(),
        cancel,
    )
    .map_err(|e| e.to_string())?;
    active(cancel)?;
    Ok(draft)
}

fn routing_hash(graph: &engine::audio::routing::model::Model) -> Result<String, String> {
    serde_json::to_vec(&(&graph.connections, &graph.tracks_without_default_send))
        .map(|bytes| digest(&bytes))
        .map_err(|e| e.to_string())
}

/// Restore editable source routing from a saved render review.
/// Takes a draft whose printed routes and identities are unchanged; returns original track mutes/routing with the print removed, refusing later routing edits or merged-source ambiguity.
pub(crate) fn restore(mut draft: Imported, cancel: &AtomicBool) -> Result<Imported, String> {
    active(cancel)?;
    let record = draft
        .state
        .migration
        .as_ref()
        .and_then(|m| m.sources.first())
        .and_then(|s| s.renders.first())
        .ok_or("No saved print to restore")?
        .clone();
    record.validate()?;
    let layout = draft
        .state
        .session
        .as_mut()
        .ok_or("Native session identities absent")?;
    if !record.restorable || record.audio_track.namespace != layout.namespace {
        return Err("This print was merged into another project. Open the original saved migration or review its archived source before replacing routing".into());
    }
    let slot = layout
        .resolve(session::Axis::Track, record.audio_track.id)
        .ok_or("Printed track is absent")?;
    let graph = draft
        .state
        .routing
        .as_mut()
        .ok_or("Printed routes are absent")?;
    if routing_hash(graph)? != record.printed_routing_sha256 {
        return Err("Routing changed after the print; review those edits before restoring original output routes".into());
    }
    for (reference, mute) in record.muted_tracks.iter().zip(&record.original_mutes) {
        if reference.namespace != layout.namespace {
            return Err("Original source namespace changed".into());
        }
        let original = layout
            .resolve(session::Axis::Track, reference.id)
            .ok_or("An original track was removed")?;
        if !draft.state.tracks[original].mute {
            return Err(
                "An original track was unmuted after printing; review its output before restoring"
                    .into(),
            );
        }
        draft.state.tracks[original].mute = *mute;
    }
    let song = draft
        .state
        .arrangement
        .as_mut()
        .ok_or("Printed arrangement absent")?;
    let song = Arc::make_mut(song);
    let removed: BTreeSet<_> = song
        .instances
        .iter()
        .filter(|i| i.track == record.audio_track)
        .map(|i| i.source)
        .collect();
    if removed.is_empty()
        || song
            .instances
            .iter()
            .any(|i| removed.contains(&i.source) && i.track != record.audio_track)
    {
        return Err("Printed source was edited or reused on another track; review those placements before restoring".into());
    }
    song.instances.retain(|i| i.track != record.audio_track);
    song.sources.retain(|s| !removed.contains(&s.id));
    layout.tracks.remove(slot);
    layout.track_order.retain(|&i| usize::from(i) != slot);
    for index in &mut layout.track_order {
        if usize::from(*index) > slot {
            *index -= 1;
        }
    }
    draft.state.tracks.remove(slot);
    draft.state.selected_track = 0;
    draft.state.fx_view = -1;
    let graph = Arc::make_mut(graph);
    graph.connections = record.original_connections;
    graph.tracks_without_default_send = record.original_default_sends;
    graph.order(layout)?;
    let source = &mut Arc::make_mut(draft.state.migration.as_mut().unwrap()).sources[0];
    source.renders.clear();
    source.difference("render","Restored editable source","Saved original routes and mutes restored. The printed arrangement source was removed; original MIDI, device state and automation remain editable")?;
    draft.state.compact_media(&mut draft.media)?;
    crate::project_file::validate_metadata(
        &crate::project_file::Bundle {
            state: &draft.state,
            media: draft.media.clone(),
        },
        &Default::default(),
        cancel,
    )
    .map_err(|e| e.to_string())?;
    active(cancel)?;
    Ok(draft)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    pub(crate) fn wave(path: &Path) {
        let frames = 60000u32;
        let mut bytes = Vec::new();
        bytes.extend(b"RIFF");
        bytes.extend((36 + frames * 2).to_le_bytes());
        bytes.extend(b"WAVEfmt ");
        bytes.extend(16u32.to_le_bytes());
        bytes.extend(1u16.to_le_bytes());
        bytes.extend(1u16.to_le_bytes());
        bytes.extend(48000u32.to_le_bytes());
        bytes.extend(96000u32.to_le_bytes());
        bytes.extend(2u16.to_le_bytes());
        bytes.extend(16u16.to_le_bytes());
        bytes.extend(b"data");
        bytes.extend((frames * 2).to_le_bytes());
        for _ in 0..frames {
            bytes.extend(8192i16.to_le_bytes());
        }
        std::fs::write(path, bytes).unwrap();
    }
    #[test]
    fn aligned_print_preserves_midi_tail_and_state_without_double_master_gain() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!(
            "target/validation/ableton-render-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let source_path = root.join("User set.als");
        let xml = crate::ableton::tests::document(11);
        std::fs::write(&source_path, &xml).unwrap();
        let path = root.join("Authorized original.wav");
        wave(&path);
        let cancel = AtomicBool::new(false);
        let mut draft = load(&source_path, &Default::default(), &cancel).unwrap();
        draft.state.master = 0.3;
        let options = Options {
            target: None,
            start: 0.,
            body_beats: 2.,
            includes_returns_master: true,
        };
        let mut invalid = options.clone();
        invalid.body_beats = 3.;
        assert!(attach(draft.clone(), &path, &invalid, &cancel).is_err());
        invalid = options.clone();
        invalid.target = Some(4);
        assert!(attach(draft.clone(), &path, &invalid, &cancel).is_err());
        assert!(attach(draft.clone(), &path, &options, &AtomicBool::new(true)).is_err());
        let attached = attach(draft, &path, &options, &cancel).unwrap();
        let record = &attached.state.migration.as_ref().unwrap().sources[0].renders[0];
        assert!((record.tail_seconds - 0.3125).abs() < 1e-10);
        assert_eq!(record.muted_tracks.len(), 3);
        assert!(attached.state.tracks[..3].iter().all(|t| t.mute));
        assert_eq!(attached.state.tracks[0].clips[0].notes.len(), 1);
        assert_eq!(attached.state.master, 0.3);
        assert_eq!(
            attached.state.migration.as_ref().unwrap().sources[0].xml,
            xml
        );
        let saved = root.join("Rendered.omatainer");
        crate::project_file::save(
            &saved,
            &crate::project_file::Bundle {
                state: attached.state.clone(),
                media: attached.media.clone(),
            },
            crate::project_file::Overwrite::Never,
            &Default::default(),
            &cancel,
        )
        .unwrap();
        let reopened =
            crate::project_file::load::<project::State>(&saved, &Default::default(), &cancel)
                .unwrap();
        assert_eq!(
            reopened.state.migration.as_ref().unwrap().sources[0].renders[0].audio_sha256,
            record.audio_sha256
        );
        let mut legacy = serde_json::to_value(&reopened.state).unwrap();
        legacy["version"] = 30.into();
        assert!(serde_json::from_value::<project::State>(legacy).is_err());
        let mut rt = project::Prepared::from_state(reopened.state, reopened.media, 48000)
            .unwrap()
            .into_offline();
        rt.apply(engine::Command::Play);
        let mut audio = [0.; 512];
        for _ in 0..64 {
            rt.process(&mut audio);
        }
        let expected = 0.25f32.tanh();
        assert!(
            audio.iter().all(|v| (*v - expected).abs() < 1e-5),
            "Printed master gained or doubled: {} vs {expected}",
            audio[0]
        );
        assert_eq!(std::fs::read_to_string(&source_path).unwrap(), xml);
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(test)]
mod archive_tests {
    use super::*;
    #[test]
    fn saved_print_restores_original_routes_and_midi_with_live_source_and_export_offline() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!(
            "target/validation/ableton-archive-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let set = root.join("User set.als");
        std::fs::write(&set, crate::ableton::tests::document(11)).unwrap();
        let wave = root.join("User print.wav");
        tests::wave(&wave);
        let cancel = AtomicBool::new(false);
        let original = load(&set, &Default::default(), &cancel).unwrap();
        let original_routes = original.state.routing.clone();
        let original_media = original.media.len();
        let printed = attach(
            original,
            &wave,
            &Options {
                target: None,
                start: 0.,
                body_beats: 2.,
                includes_returns_master: true,
            },
            &cancel,
        )
        .unwrap();
        let native = root.join("Retained.omatainer");
        crate::project_file::save(
            &native,
            &crate::project_file::Bundle {
                state: crate::ui::project::Document {
                    engine: printed.state,
                    view: Default::default(),
                    mapping_schema: crate::ui::project::FACTORY_MAPPING_SCHEMA,
                },
                media: printed.media,
            },
            crate::project_file::Overwrite::Never,
            &Default::default(),
            &cancel,
        )
        .unwrap();
        std::fs::remove_file(&set).unwrap();
        std::fs::remove_file(&wave).unwrap();
        let archived = load_native(&native, &cancel).unwrap();
        verify_draft(&archived, &cancel).unwrap();
        assert!(verify_sources(&archived.state, &cancel).is_err());
        let mut edited = archived.clone();
        Arc::make_mut(edited.state.routing.as_mut().unwrap())
            .connections
            .last_mut()
            .unwrap()
            .map[0]
            .gain = 0.5;
        assert!(restore(edited, &cancel).is_err());
        let mut merged = archived.clone();
        Arc::make_mut(merged.state.migration.as_mut().unwrap()).sources[0].renders[0].restorable =
            false;
        assert!(restore(merged, &cancel).is_err());
        let restored = restore(archived.clone(), &cancel).unwrap();
        assert_eq!(restored.state.routing, original_routes);
        assert_eq!(restored.state.tracks.len(), 3);
        assert!(restored.state.tracks.iter().all(|t| !t.mute));
        assert_eq!(restored.state.tracks[0].clips[0].notes.len(), 1);
        assert_eq!(restored.media.len(), original_media);
        assert!(restored.state.migration.as_ref().unwrap().sources[0]
            .renders
            .is_empty());
        verify_draft(&restored, &cancel).unwrap();
        let mut bytes = std::fs::read(&native).unwrap();
        bytes.push(0);
        std::fs::write(&native, &bytes).unwrap();
        assert!(verify_draft(&archived, &cancel).is_err());
        assert!(load_native(&native, &cancel).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
}
