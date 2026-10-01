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
    };
    let reference = SourceRef {
        track: TrackId("1".repeat(32)),
        source: source.clone(),
        fingerprint,
        content_hash: Some([7; 32]),
    };
    let mut catalog = Catalog::default();
    catalog.tracks.push(Track {
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
                    waveform: false,
                },
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
            waveform: false,
        },
        Fields {
            bpm: false,
            duration: true,
            waveform: false,
        },
        Fields {
            bpm: false,
            duration: false,
            waveform: true,
        },
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
