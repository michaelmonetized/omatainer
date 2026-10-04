use super::*;
use crate::{
    engine::{beatgrid::Grid, preparation::Loop},
    sampler_bank::SourceRef,
    track_analysis::{Fields, WaveformRef},
};

// Pure catalog fixture: identities are modeled values. Worker tests independently
// prove hashes against real opened/decoded media; no source I/O happens here.
fn fixture() -> (Catalog, SourceRef, Version) {
    let source = LibSource::File(PathBuf::from("/virtual/analysis-source.wav"));
    let fingerprint: FileFingerprint = serde_json::from_value(serde_json::json!({
        "device": 7, "inode": 91, "length": 4096,
        "modified": [123, 456], "changed": [123, 789]
    }))
    .unwrap();
    let version = Version {
        fingerprint: Some(fingerprint),
        metadata: Metadata {
            title: "Prepared title".into(),
            artist: "Artist".into(),
            bpm: Bpm::new(119.0, Origin::Heuristic),
            key: "F#m".into(),
            duration: Some(30.0),
            last_play: Some(SystemTime::UNIX_EPOCH),
        },
        preparation: Preparation {
            cue: 1.25,
            grid: Some(Grid::new(-0.125, 127.5).unwrap()),
            hotcues: [Some(1.25), None, None, None, None, None, None, Some(11.5)],
            loop_region: Some(Loop {
                start: 1.25,
                length: 2.0,
                enabled: true,
            }),
            ..Default::default()
        },
        content_hash: None,
        analysis: None,
        tags: None,
        audio_identity: None,
    };
    let reference = SourceRef {
        track: TrackId("1".repeat(32)),
        source: source.clone(),
        fingerprint,
        content_hash: Some([7; 32]),
    };
    let mut catalog = Catalog::default();
    catalog.tracks.push(Track {
        annotations: Default::default(),
        locks: Default::default(),
        id: reference.track.clone(),
        source,
        current: 0,
        versions: vec![version.clone()],
        previous_locations: vec![],
    });
    catalog.validate().unwrap();
    (catalog, reference, version)
}
fn patch(reference: SourceRef, bpm: Option<f32>, fields: Fields) -> Patch {
    Patch {
        reference,
        fields,
        level: fields.level.then(|| crate::track_gain::Analysis::new(crate::track_gain::measure_channels(&[0.25; 32], 1, || false).unwrap())),
        key: fields.key.then(crate::musical_key::Analysis::unknown),
        at_unix_ms: 123456,
        bpm,
        duration: 12.5,
        waveform: fields.waveform.then_some(WaveformRef {
            sha256: [8; 32],
            bytes: 16000,
            frames: 600_000,
            sample_rate: 48_000,
            channels: 2,
            bins: 2048,
        }),
    }
}

fn recorded_analysis_survives_stale_automatic_metadata_upserts(bpm: Option<f32>) {
    let (mut catalog, reference, original) = fixture();
    catalog
        .apply_analysis(&patch(reference.clone(), bpm, Fields::ALL))
        .unwrap();
    let measured = catalog
        .version(&reference.source, Some(reference.fingerprint))
        .unwrap()
        .clone();
    let expected_bpm = bpm.map_or(Bpm::UNKNOWN, |value| Bpm::new(value, Origin::Heuristic));
    for stale_bpm in [original.metadata.bpm, Bpm::hint(96.0), Bpm::UNKNOWN] {
        let mut stale = original.metadata.clone();
        stale.bpm = stale_bpm;
        stale.duration = Some(90.0);
        let actual = catalog
            .upsert(reference.source.clone(), Some(reference.fingerprint), stale)
            .unwrap();
        assert_eq!(
            actual.metadata.bpm, expected_bpm,
            "older automatic value replaced measured {bpm:?}"
        );
        assert_eq!(actual.metadata.duration, Some(12.5));
        assert_eq!(actual.analysis, measured.analysis);
        assert_eq!(actual.preparation, original.preparation);
        assert_eq!(actual.metadata.title, original.metadata.title);
        assert_eq!(actual.metadata.artist, original.metadata.artist);
        assert_eq!(actual.metadata.key, original.metadata.key);
        assert_eq!(actual.metadata.last_play, original.metadata.last_play);
    }
    catalog.validate().unwrap();
}

#[test]
fn recorded_known_bpm_survives_stale_automatic_metadata_upserts() {
    recorded_analysis_survives_stale_automatic_metadata_upserts(Some(143.5));
}

#[test]
fn recorded_unknown_bpm_survives_stale_automatic_metadata_upserts() {
    recorded_analysis_survives_stale_automatic_metadata_upserts(None);
}

#[test]
fn user_bpm_correction_before_or_after_analysis_remains_authoritative() {
    for before in [false, true] {
        let (mut catalog, reference, original) = fixture();
        let mut user = original.metadata.clone();
        user.bpm = Bpm::new(156.25, Origin::User);
        if before {
            catalog
                .upsert(
                    reference.source.clone(),
                    Some(reference.fingerprint),
                    user.clone(),
                )
                .unwrap();
        }
        catalog
            .apply_analysis(&patch(reference.clone(), Some(143.5), Fields::ALL))
            .unwrap();
        if !before {
            catalog
                .upsert(reference.source.clone(), Some(reference.fingerprint), user)
                .unwrap();
        }
        // Reanalysis can replace the measured estimate with Unknown while the
        // explicitly corrected display value and preparation stay authoritative.
        catalog
            .apply_analysis(&patch(
                reference.clone(),
                None,
                Fields {
                    bpm: true,
                    duration: false,
                    waveform: false, level: false, key: false },
            ))
            .unwrap();
        let actual = catalog
            .upsert(
                reference.source.clone(),
                Some(reference.fingerprint),
                original.metadata.clone(),
            )
            .unwrap();
        assert_eq!(actual.metadata.bpm, Bpm::new(156.25, Origin::User));
        assert_eq!(
            actual
                .analysis
                .as_ref()
                .unwrap()
                .bpm
                .as_ref()
                .unwrap()
                .value,
            None
        );
        assert_eq!(actual.metadata.duration, Some(12.5));
        assert_eq!(actual.preparation, original.preparation);
    }
}

#[test]
fn selective_reanalysis_does_not_write_unrequested_fields_or_other_versions() {
    for fields in [
        Fields {
            bpm: true,
            duration: false,
            waveform: false, level: false, key: false },
        Fields {
            bpm: false,
            duration: true,
            waveform: false, level: false, key: false },
        Fields {
            bpm: false,
            duration: false,
            waveform: true, level: false, key: false },
    ] {
        let (mut catalog, reference, original) = fixture();
        catalog
            .apply_analysis(&patch(reference.clone(), Some(143.5), fields))
            .unwrap();
        let mut stale = original.metadata.clone();
        // The ordinary stale-row path retains original values for fields not
        // requested by the analysis and must not override requested fields.
        stale.duration = original.metadata.duration;
        let actual = catalog
            .upsert(reference.source.clone(), Some(reference.fingerprint), stale)
            .unwrap();
        assert_eq!(
            actual.metadata.bpm,
            if fields.bpm {
                Bpm::new(143.5, Origin::Heuristic)
            } else {
                original.metadata.bpm
            }
        );
        assert_eq!(
            actual.metadata.duration,
            if fields.duration {
                Some(12.5)
            } else {
                original.metadata.duration
            }
        );
        let measured = actual.analysis.as_ref().unwrap();
        assert_eq!(measured.bpm.is_some(), fields.bpm);
        assert_eq!(measured.duration.is_some(), fields.duration);
        assert_eq!(measured.waveform.is_some(), fields.waveform);
        assert_eq!(actual.preparation, original.preparation);
        let replacement: FileFingerprint = serde_json::from_value(serde_json::json!({
            "device": 7, "inode": 92, "length": 8192,
            "modified": [456, 0], "changed": [456, 1]
        }))
        .unwrap();
        let new = catalog
            .upsert(
                reference.source.clone(),
                Some(replacement),
                original.metadata.clone(),
            )
            .unwrap();
        assert!(new.analysis.is_none());
        assert_eq!(new.metadata, original.metadata);
        assert_eq!(new.preparation, Preparation::default());
        assert!(catalog
            .version(&reference.source, Some(reference.fingerprint))
            .unwrap()
            .analysis
            .is_some());
    }
}

#[test]
fn late_analysis_cannot_replace_newly_locked_fields_and_still_publishes_allowed_waveforms() {
    use crate::library::protection::{Target,Patch as LockPatch};
    let (mut catalog,reference,original)=fixture();let target=Target::capture(&catalog.tracks[0]);catalog.protect(&[target],LockPatch {bpm:Some(true),metadata:Some(true),grid:Some(true)}).unwrap();
    let forbidden=patch(reference.clone(),Some(150.0),Fields {bpm:true,duration:true,waveform:false, level: false, key: false });
    assert!(catalog.apply_analysis(&forbidden).unwrap_err().contains("protected"));assert_eq!(catalog.tracks[0].versions[0],original);
    catalog.apply_analysis(&patch(reference,Some(150.0),Fields::ALL)).unwrap();let version=&catalog.tracks[0].versions[0];
    assert_eq!(version.metadata,original.metadata);assert_eq!(version.preparation,original.preparation);let record=version.analysis.as_ref().unwrap();assert!(record.bpm.is_none());assert!(record.duration.is_none());assert!(record.waveform.is_some());
}

#[test]
fn musical_key_analysis_preserves_manual_embedded_and_inferred_provenance() {
    use crate::musical_key::{Analysis,Key};
    let (mut catalog,reference,original)=fixture();
    let fields=Fields {bpm:false,duration:false,waveform:false,level:false,key:true};
    let mut measured=patch(reference.clone(),None,fields);
    measured.key=Some(Analysis {key:Some(Key {tonic:0,minor:false}),score:0.9,margin:0.2,frames:32});
    catalog.apply_analysis(&measured).unwrap();
    let version=catalog.version(&reference.source,Some(reference.fingerprint)).unwrap();
    assert_eq!(version.metadata,original.metadata);
    let (cell,detail)=crate::musical_key::display(Some(version),"",false);
    assert_eq!(cell,"C · 8B");assert!(detail.contains("Analyzed musical key"));
    let (locked,detail)=crate::musical_key::display(Some(version),"",true);
    assert_eq!(locked,"F#m · 11A");assert!(detail.contains("Locked saved key"));
    let review=crate::library::tags::Review {id:reference.track.clone(),source:reference.source.clone(),fingerprint:reference.fingerprint};
    catalog.apply_tag_sidecar(&review,&crate::library::tags::Patch {key:Some("F#m".into()),..Default::default()}).unwrap();
    catalog.apply_analysis(&measured).unwrap();
    let version=catalog.version(&reference.source,Some(reference.fingerprint)).unwrap();
    assert_eq!(version.metadata.key,"F#m");
    let (cell,detail)=crate::musical_key::display(Some(version),"",false);
    assert_eq!(cell,"F#m · 11A");assert!(detail.contains("user sidecar"));assert!(detail.contains("C · 8B"));
    let mut embedded=version.clone();embedded.metadata.key="Gm".into();
    let tags=embedded.tags.as_mut().unwrap();tags.overrides.key=None;
    tags.observed.key=Some(crate::media_tags::Field {value:"Gm".into(),source:crate::media_tags::TagSource::Vorbis});
    let (cell,detail)=crate::musical_key::display(Some(&embedded),"",false);
    assert_eq!(cell,"Gm · 6A");assert!(detail.contains("Vorbis comments"));assert!(detail.contains("C · 8B"));
    catalog.apply_tag_sidecar(&review,&crate::library::tags::Patch {key:Some(String::new()),..Default::default()}).unwrap();
    catalog.apply_analysis(&measured).unwrap();
    let version=catalog.version(&reference.source,Some(reference.fingerprint)).unwrap();
    assert_eq!(crate::musical_key::display(Some(version),"",false).0,"—");
    assert_eq!(version.analysis.as_ref().unwrap().key.as_ref().unwrap().value,measured.key.unwrap());
}
