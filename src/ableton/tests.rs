use super::*;
use std::io::Write;
struct Fixture {
    root: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        engine::midi_edit::initialize().unwrap();
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!(
            "target/validation/ableton-{}-{}",
            std::process::id(),
            engine::midi_edit::NoteId::new().words()[1]
        ));
        std::fs::create_dir_all(&root).unwrap();
        Self { root }
    }
    fn set(&self, text: &str) -> PathBuf {
        let path = self.root.join("User 東京.als");
        std::fs::write(&path, text).unwrap();
        path
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
pub(crate) fn document(version: u32) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><Ableton MajorVersion="5" MinorVersion="{version}.0_1" Creator="Original format-contract fixture"><LiveSet><Tracks><MidiTrack Id="4"><Name><UserName Value="Keys &amp; 東京"/></Name><TrackGroupId Value="8"/><DeviceChain><MainSequencer><ClipSlotList><ClipSlot Id="3"><ClipSlot><Value>{}</Value></ClipSlot></ClipSlot></ClipSlotList><ClipTimeable><ArrangerAutomation><Events>{}</Events></ArrangerAutomation></ClipTimeable></MainSequencer><DeviceChain><Devices><InstrumentGroupDevice Id="7"><On><Manual Value="true"/></On><Branches><InstrumentBranch Id="2"><DeviceChain><Devices><Operator Id="1"><On><Manual Value="false"/></On><State><Buffer>0102 &amp; &#65;</Buffer></State></Operator></Devices></DeviceChain></InstrumentBranch></Branches></InstrumentGroupDevice></Devices></DeviceChain></DeviceChain></MidiTrack><GroupTrack Id="8"><Name><EffectiveName Value="Group"/></Name></GroupTrack><ReturnTrack Id="9"><Name><EffectiveName Value="Return"/></Name></ReturnTrack></Tracks><MasterTrack><DeviceChain><Mixer><Tempo><Manual Value="128"/></Tempo><TimeSignature><Manual Value="201"/></TimeSignature></Mixer></DeviceChain></MasterTrack><Scenes><Scene Id="3"><Name Value="Scene Ω"/></Scene></Scenes><Transport><LoopStart Value="4"/><LoopLength Value="8"/><LoopOn Value="true"/></Transport><Locators><Locators><Locator Id="1"><Name Value="Intro"/><Time Value="4"/></Locator></Locators></Locators></LiveSet></Ableton>"#,
        clip(2),
        clip(3)
    )
}
fn clip(id: u32) -> String {
    format!(
        r#"<MidiClip Id="{id}"><Name Value="Melody"/><CurrentStart Value="4"/><CurrentEnd Value="8"/><Loop><LoopStart Value="0"/><LoopEnd Value="4"/><LoopOn Value="true"/></Loop><Notes><KeyTracks><KeyTrack Id="1"><MidiKey Value="60"/><Notes><MidiNoteEvent Time="0.25" Duration="0.5" Velocity="93" OffVelocity="9" IsEnabled="true" NoteId="1"/></Notes></KeyTrack></KeyTracks></Notes></MidiClip>"#
    )
}
#[test]
fn live_10_and_11_structure_timing_unicode_devices_and_native_round_trip() {
    for version in [10, 11] {
        let fixture = Fixture::new();
        let text = document(version);
        let path = fixture.set(&text);
        let imported = load(&path, &Default::default(), &AtomicBool::new(false)).unwrap();
        assert_eq!(imported.state.bpm, 128.);
        assert_eq!(imported.state.tracks.len(), 3);
        assert_eq!(imported.state.tracks[0].name, "Keys & 東京");
        let note = &imported.state.tracks[0].clips[0].notes[0];
        assert_eq!(
            (note.pitch, note.start, note.len, note.vel, note.release_vel),
            (60, 0.25, 0.5, 93, 9)
        );
        assert_eq!(
            imported.state.arrangement.as_ref().unwrap().instances[0].start,
            4.
        );
        assert_eq!(
            imported
                .state
                .navigation
                .as_ref()
                .unwrap()
                .model
                .loop_region
                .unwrap()
                .end,
            12.
        );
        let source = &imported.state.migration.as_ref().unwrap().sources[0];
        assert_eq!(source.xml, text);
        assert_eq!(source.devices.len(), 2);
        assert!(!source.devices[1].enabled);
        assert_eq!(source.tracks[0].parent, 8);
        assert_eq!(source.tracks[2].role, "ReturnTrack");
        assert!(imported.state.tracks[0].synth.offline.is_some());
        let native = fixture.root.join("import.omatainer");
        let bundle = crate::project_file::Bundle {
            state: imported.state,
            media: imported.media,
        };
        crate::project_file::save(
            &native,
            &bundle,
            crate::project_file::Overwrite::Never,
            &Default::default(),
            &AtomicBool::new(false),
        )
        .unwrap();
        let reopened = crate::project_file::load::<project::State>(
            &native,
            &Default::default(),
            &AtomicBool::new(false),
        )
        .unwrap();
        reopened.state.validate(&reopened.media).unwrap();
        assert_eq!(
            reopened.state.migration.as_ref().unwrap().sources[0].xml,
            text
        );
        let prepared =
            project::Prepared::from_state(reopened.state, reopened.media, 48000).unwrap();
        assert_eq!(
            prepared.into_offline().migration.as_ref().unwrap().sources[0]
                .devices
                .len(),
            2
        );
    }
}
#[test]
fn gzip_crc_members_decompression_and_xml_boundaries_are_enforced() {
    let fixture = Fixture::new();
    let text = document(11);
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(text.as_bytes()).unwrap();
    let bytes = encoder.finish().unwrap();
    let path = fixture.root.join("gzip.als");
    std::fs::write(&path, &bytes).unwrap();
    assert!(load(&path, &Default::default(), &AtomicBool::new(false)).is_ok());
    for bytes in [
        bytes[..bytes.len() - 3].to_vec(),
        [bytes.as_slice(), b"junk"].concat(),
        [bytes.as_slice(), bytes.as_slice()].concat(),
    ] {
        std::fs::write(&path, bytes).unwrap();
        assert!(load(&path, &Default::default(), &AtomicBool::new(false)).is_err());
    }
    for xml in [
        text.replace("11.0_1", "12.0_1"),
        text.replace(
            "<LiveSet>",
            "<!DOCTYPE LiveSet SYSTEM 'file:///etc/passwd'><LiveSet>",
        ),
        text.replace("Duration=\"0.5\"", "Duration=\"NaN\""),
        text[..text.len() - 8].into(),
        text.replace("Id=\"8\"", "Id=\"4\""),
        text.replace("<Scene Id=\"3\">", "<Scene Id=\"5\">"),
    ] {
        let path = fixture.set(&xml);
        assert!(load(&path, &Default::default(), &AtomicBool::new(false)).is_err());
    }
    assert!(load(
        &fixture.set(&text),
        &Default::default(),
        &AtomicBool::new(true)
    )
    .is_err());
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    encoder.write_all(&vec![b' '; MAX_XML + 1]).unwrap();
    assert!(expand(&encoder.finish().unwrap(), &AtomicBool::new(false))
        .unwrap_err()
        .contains("32 MiB"));
}
#[test]
#[ignore = "requires an explicitly supplied private saved Live Set; never commits factory or private content"]
fn private_saved_live_set_is_read_without_modification() {
    let path = PathBuf::from(std::env::var_os("OMATAINER_ALS_FIXTURE").unwrap());
    let before = std::fs::read(&path).unwrap();
    let imported = load(&path, &Default::default(), &AtomicBool::new(false)).unwrap();
    imported.state.validate(&imported.media).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), before);
    println!(
        "format {}, tracks {}, scenes {}, differences {}",
        imported.state.migration.as_ref().unwrap().sources[0].format,
        imported.state.tracks.len(),
        imported.state.scene_fx.len(),
        imported.state.migration.as_ref().unwrap().sources[0]
            .differences
            .len()
    );
}

pub(crate) fn capture(engine: &engine::Engine, rt: &mut engine::RtEngine) -> project::Captured {
    let handle = engine.project.clone();
    let task = std::thread::spawn(move || handle.capture(&AtomicBool::new(false)));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !task.is_finished() {
        rt.process(&mut [0.; 256]);
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    task.join().unwrap().unwrap()
}
#[test]
fn imported_source_archive_rebinds_on_native_merge_and_survives_atomic_undo_redo_capture() {
    let fixture = Fixture::new();
    let source = load(
        &fixture.set(&document(11)),
        &Default::default(),
        &AtomicBool::new(false),
    )
    .unwrap();
    let (engine, mut rt) = engine::Engine::headless_for_test(48000, 256);
    let old_tracks = rt.tracks.len();
    let old_scenes = rt.session.scenes.len();
    let selection = session::ImportSelection {
        tracks: source
            .state
            .session
            .as_ref()
            .unwrap()
            .tracks
            .iter()
            .map(|t| t.id)
            .collect(),
        scenes: source
            .state
            .session
            .as_ref()
            .unwrap()
            .scenes
            .iter()
            .map(|s| s.id)
            .collect(),
        clips: true,
        devices: true,
        keep_timing: true,
    };
    let (request, ack) = session::Request::import(
        capture(&engine, &mut rt),
        &source.state,
        &source.media,
        &selection,
        48000,
    )
    .unwrap();
    engine.send(engine::Command::session_edit(request)).unwrap();
    assert_eq!(
        engine::test_alloc::measure(|| rt.process(&mut [])),
        Default::default()
    );
    assert_eq!(ack.state(), engine::midi_edit::Outcome::Applied);
    assert_eq!(rt.tracks.len(), old_tracks + 3);
    assert_eq!(rt.session.scenes.len(), old_scenes + 1);
    let retained = &rt.migration.as_ref().unwrap().sources[0];
    for track in &retained.tracks {
        assert_eq!(track.native.namespace, rt.session.namespace);
        assert!(rt
            .session
            .resolve(session::Axis::Track, track.native.id)
            .is_some());
    }
    assert_eq!(retained.devices.len(), 2);
    engine.send(engine::Command::Undo).unwrap();
    assert_eq!(
        engine::test_alloc::measure(|| rt.process(&mut [])),
        Default::default()
    );
    assert!(rt.migration.is_none());
    assert_eq!(rt.tracks.len(), old_tracks);
    engine.send(engine::Command::Redo).unwrap();
    assert_eq!(
        engine::test_alloc::measure(|| rt.process(&mut [])),
        Default::default()
    );
    assert!(rt.migration.is_some());
    let captured = capture(&engine, &mut rt);
    captured.state.validate(&captured.media).unwrap();
    assert_eq!(
        captured.state.migration.as_ref().unwrap().sources[0].xml,
        document(11)
    );
}
#[test]
fn unicode_media_remap_and_missing_source_refs_remain_distinct_after_native_reopen() {
    let fixture = Fixture::new();
    let sample = fixture.root.join("音.wav");
    let mut wave = Vec::new();
    let samples = 48000u32;
    wave.extend(b"RIFF");
    wave.extend((36 + samples * 2).to_le_bytes());
    wave.extend(b"WAVEfmt ");
    wave.extend(16u32.to_le_bytes());
    wave.extend(1u16.to_le_bytes());
    wave.extend(1u16.to_le_bytes());
    wave.extend(48000u32.to_le_bytes());
    wave.extend(96000u32.to_le_bytes());
    wave.extend(2u16.to_le_bytes());
    wave.extend(16u16.to_le_bytes());
    wave.extend(b"data");
    wave.extend((samples * 2).to_le_bytes());
    for _ in 0..samples {
        wave.extend(8192i16.to_le_bytes());
    }
    std::fs::write(&sample, wave).unwrap();
    let audio = r#"<AudioClip Id="5"><Name Value="音"/><Loop><LoopStart Value="0.25"/><LoopEnd Value="0.75"/><LoopOn Value="false"/></Loop><SampleRef><FileRef><Path Value="/old/mac/音.wav"/></FileRef></SampleRef></AudioClip>"#;
    let text = document(11).replacen(&clip(2), audio, 1);
    let path = fixture.set(&text);
    let options = Options {
        remaps: vec![("/old/mac".into(), fixture.root.clone())],
        ..Default::default()
    };
    let imported = load(&path, &options, &AtomicBool::new(false)).unwrap();
    let clip = &imported.state.tracks[0].clips[0];
    assert_eq!(clip.kind, engine::ClipKind::Audio);
    let region = clip.audio_region.unwrap();
    assert_eq!((region.start, region.end), (12000, 36000));
    assert!(!clip.properties.disabled);
    assert_eq!(imported.media[clip.audio.unwrap()].frames(), 48000);
    let asset = &imported.state.migration.as_ref().unwrap().sources[0].assets[0];
    assert!(matches!(asset.availability, Availability::Embedded));
    assert_eq!(asset.audio_sha256.as_ref().unwrap().len(), 64);
    std::fs::remove_file(&sample).unwrap();
    let missing = load(&path, &options, &AtomicBool::new(false)).unwrap();
    assert!(missing.state.tracks[0].clips[0].audio.is_none());
    let asset = &missing.state.migration.as_ref().unwrap().sources[0].assets[0];
    assert!(matches!(asset.availability, Availability::Missing));
    assert_eq!(asset.original, "/old/mac/音.wav");
    assert!(asset.audio_sha256.is_none());
    let raw = serde_json::to_vec(&missing.state).unwrap();
    let reopened: project::State = serde_json::from_slice(&raw).unwrap();
    reopened.validate(&missing.media).unwrap();
    assert_eq!(
        reopened.migration.as_ref().unwrap().sources[0].assets[0].original,
        "/old/mac/音.wav"
    );
}

#[test]
fn target_bound_tempo_meter_groups_and_pre_fader_returns_preserve_native_routing() {
    use crate::engine::audio::routing::model::{Group, Tap};
    let fixture = Fixture::new();
    let master = r#"<MasterTrack><AutomationEnvelopes><Envelopes><AutomationEnvelope Id="1"><EnvelopeTarget><PointeeId Value="810"/></EnvelopeTarget><Automation><Events><FloatEvent Time="-63072000" Value="120"/><FloatEvent Time="0" Value="120"/><FloatEvent Time="4" Value="180"/><FloatEvent Time="8" Value="90"/></Events></Automation></AutomationEnvelope><AutomationEnvelope Id="2"><EnvelopeTarget><PointeeId Value="811"/></EnvelopeTarget><Automation><Events><EnumEvent Time="-63072000" Value="201"/><EnumEvent Time="4" Value="303"/></Events></Automation></AutomationEnvelope></Envelopes></AutomationEnvelopes><DeviceChain><Mixer><Tempo><Manual Value="140"/><AutomationTarget Id="810"/></Tempo><TimeSignature><Manual Value="201"/><AutomationTarget Id="811"/></TimeSignature></Mixer></DeviceChain></MasterTrack>"#;
    let original = document(11);
    let start = original.find("<MasterTrack>").unwrap();
    let end = original.find("</MasterTrack>").unwrap() + "</MasterTrack>".len();
    let text=format!("{}{}{}",&original[..start],master,&original[end..]).replace("</LiveSet>",r#"<SendsPre><SendPreBool Id="0" Value="true"/></SendsPre></LiveSet>"#).replace("<DeviceChain><MainSequencer>",r#"<DeviceChain><Mixer><Volume><Manual Value="0.25"/></Volume><Sends><TrackSendHolder Id="0"><Send><Manual Value="0.5"/></Send><Active Value="true"/></TrackSendHolder></Sends></Mixer><AudioOutputRouting><Target Value="AudioOut/GroupTrack"/></AudioOutputRouting><MainSequencer>"#);
    let imported = load(
        &fixture.set(&text),
        &Default::default(),
        &AtomicBool::new(false),
    )
    .unwrap();
    let map = imported.state.conductor.as_ref().unwrap();
    assert!((map.seconds_at(4.) - 60. / 15. * (180f64 / 120.).ln()).abs() < 2e-6);
    assert!(
        (map.seconds_at(8.) - map.seconds_at(4.) - 60. / -22.5 * (90f64 / 180.).ln()).abs() < 2e-6
    );
    assert!(imported.state.migration.as_ref().unwrap().sources[0]
        .differences
        .iter()
        .any(|item| item.feature == "Tempo precision"));
    assert_eq!(
        (
            map.meters[1].numerator,
            map.meters[1].denominator_power,
            map.meters[1].tick
        ),
        (7, 3, 4 * 32767)
    );
    assert_eq!(imported.state.bpm, 120.);
    let layout = imported.state.session.as_ref().unwrap();
    let graph = imported.state.routing.as_ref().unwrap();
    let t = Group::Track(layout.tracks[0].id);
    let parent = Group::Track(layout.tracks[1].id);
    let return_track = Group::Track(layout.tracks[2].id);
    assert!(graph
        .connections
        .iter()
        .any(|r| r.source.group == t && r.destination == parent && r.source.tap == Tap::PostMixer));
    assert!(graph.connections.iter().any(|r| r.source.group == t
        && r.destination == return_track
        && r.source.tap == Tap::PostFx
        && r.map[0].gain == 0.5));
    assert_eq!(graph.tracks_without_default_send.len(), 3);
    assert_eq!(
        imported.state.tracks[1].input_monitor,
        Some(engine::input_monitor::Mode::In)
    );
    let (engine, mut rt) = engine::Engine::headless_for_test(48000, 256);
    let initial = rt.tracks.len();
    let selection = session::ImportSelection {
        tracks: layout.tracks.iter().map(|t| t.id).collect(),
        scenes: layout.scenes.iter().map(|s| s.id).collect(),
        clips: true,
        devices: true,
        keep_timing: true,
    };
    let (request, ack) = session::Request::import(
        capture(&engine, &mut rt),
        &imported.state,
        &imported.media,
        &selection,
        48000,
    )
    .unwrap();
    engine.send(engine::Command::session_edit(request)).unwrap();
    assert_eq!(
        engine::test_alloc::measure(|| rt.process(&mut [])),
        Default::default()
    );
    assert_eq!(ack.state(), engine::midi_edit::Outcome::Applied);
    assert_eq!(
        rt.tracks[initial + 1].input_monitor,
        Some(engine::input_monitor::Mode::In)
    );
    let retained = rt.routing.as_ref().unwrap().model.clone();
    let new = rt.session.tracks[initial].id;
    assert!(retained
        .connections
        .iter()
        .any(|r| r.source.group == Group::Track(new)
            && r.destination == Group::Track(rt.session.tracks[initial + 1].id)));
    engine.send(engine::Command::Undo).unwrap();
    assert_eq!(
        engine::test_alloc::measure(|| rt.process(&mut [])),
        Default::default()
    );
    assert!(rt.routing.is_none());
    engine.send(engine::Command::Redo).unwrap();
    assert_eq!(
        engine::test_alloc::measure(|| rt.process(&mut [])),
        Default::default()
    );
    assert_eq!(rt.routing.as_ref().unwrap().model, retained);
    let bad = text.replace("Value=\"303\"", "Value=\"495\"");
    assert!(load(
        &fixture.set(&bad),
        &Default::default(),
        &AtomicBool::new(false)
    )
    .is_err());
    let bad = text.replace("Time=\"8\" Value=\"90\"", "Time=\"4\" Value=\"90\"");
    assert!(load(
        &fixture.set(&bad),
        &Default::default(),
        &AtomicBool::new(false)
    )
    .is_err());
}
