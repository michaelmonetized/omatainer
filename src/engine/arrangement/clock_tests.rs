use super::*;
use crate::engine::test_alloc;
use std::sync::atomic::AtomicBool;

#[test]
fn printed_ramp_uses_source_seconds_without_allocating_and_retains_its_clock() {
    crate::engine::midi_edit::initialize().unwrap();
    let rate = 48000;
    let audio = Arc::new(Sample {
        name: "Original numbered render".into(),
        sr: rate,
        ch: 1,
        data: (0..rate * 3)
            .map(|frame| frame as f32 / rate as f32 * 0.01)
            .collect(),
        peaks: Default::default(),
        spectrum: None,
        bpm: 120.,
        path: String::new(),
    });
    let mut clip = project::SavedClip::empty();
    clip.kind = ClipKind::Audio;
    clip.audio = Some(0);
    clip.audio_region = Some(audio_clip::Region::full(&audio, 120.).unwrap());
    clip.bars = 1.5;
    let layout = session::Layout::fresh(["Printed ramp".into()], 1);
    let conductor = midi_data::Conductor::native(
        32767,
        vec![
            midi_data::Tempo::new(0, 120., true).unwrap(),
            midi_data::Tempo::new(4 * 32767, 180., false).unwrap(),
        ],
        vec![midi_data::Meter {
            tick: 0,
            numerator: 4,
            denominator_power: 2,
            clocks: 24,
            thirty_seconds: 8,
        }],
        Default::default(),
    )
    .unwrap();
    let model = Model {
        enabled: true,
        next_id: 3,
        sources: vec![Source {
            id: 1,
            clip,
            audio_clock: Some(AudioClock {
                exact_bpm: None,
                origin: 0.,
                conductor,
            }),
        }],
        instances: vec![Instance {
            id: 2,
            source: 1,
            track: layout.reference(session::Axis::Track, 0).unwrap(),
            start: 0.,
            offset: 0.,
            duration: 4.,
            repeating: false,
            gain: 1.,
            fades: None,
            fade_link: 0,
            crossfade: None,
        }],
    };
    let raw = serde_json::to_vec(&model).unwrap();
    let reopened: Model = serde_json::from_slice(&raw).unwrap();
    let plan = Plan::prepare(
        Arc::new(reopened),
        &[audio.clone()],
        &layout,
        &AtomicBool::new(false),
    )
    .unwrap();
    let mut playback = Playback::new(Some(plan), 0.);
    for beat in [0., 0.25, 1., 2., 3., 3.999] {
        let mut output = ([0.; 2], false);
        assert_eq!(
            test_alloc::measure(|| output = playback.sample(0, beat)),
            Default::default()
        );
        let expected = (60. / 15. * (1. + beat * 15. / 120.).ln() * 0.01) as f32;
        assert!(output.1);
        assert!(
            (output.0[0] - expected).abs() < 1e-7,
            "beat {beat}: {} vs {expected}",
            output.0[0]
        );
        assert_eq!(output.0[0], output.0[1]);
    }
    let mut bad = model.clone();
    bad.instances[0].repeating = true;
    assert!(bad.validate(&[audio.clone()], &layout).is_err());
    let mut bad = model;
    bad.sources[0].audio_clock.as_mut().unwrap().origin = f64::NAN;
    assert!(bad.validate(&[audio], &layout).is_err());
}

#[test]
fn recorded_constant_tempo_retains_sample_positions_after_a_long_timeline_offset() {
    let rate = 48000;
    let bpm = f64::from(103.9606_f32);
    let origin = 100000.0;
    let audio = Arc::new(Sample {
        name: "Original recording clock test".into(), sr: rate, ch: 1,
        data: (0..rate * 3).map(|frame| frame as f32 / rate as f32 * 0.01).collect(),
        peaks: Default::default(), spectrum: None, bpm: bpm as f32, path: String::new(),
    });
    let mut clip = project::SavedClip::empty();
    clip.kind = ClipKind::Audio;
    clip.audio = Some(0);
    clip.audio_region = Some(audio_clip::Region::full(&audio, bpm as f32).unwrap());
    clip.bars = (3.0 * bpm / 60.0 / 4.0) as f32;
    let layout = session::Layout::fresh(["Recorded source".into()], 1);
    let conductor = midi_data::Conductor {
        ppqn: 960, tempos: vec![midi_data::Tempo::new(0, bpm, false).unwrap()],
        meters: vec![midi_data::Meter { tick: 0, numerator: 4, denominator_power: 2, clocks: 24, thirty_seconds: 8 }], native: None,
    }.prepare().unwrap();
    let model = Model {
        enabled: true, next_id: 3,
        sources: vec![Source { id: 1, clip, audio_clock: Some(AudioClock { origin, conductor, exact_bpm: Some(bpm) }) }],
        instances: vec![Instance { id: 2, source: 1, track: layout.reference(session::Axis::Track, 0).unwrap(), start: origin, offset: 0.0, duration: 3.0 * bpm / 60.0, repeating: false, gain: 1.0, fades: None, fade_link: 0, crossfade: None }],
    };
    let reopened: Model = serde_json::from_slice(&serde_json::to_vec(&model).unwrap()).unwrap();
    let plan = Plan::prepare(Arc::new(reopened), &[audio.clone()], &layout, &AtomicBool::new(false)).unwrap();
    let mut playback = Playback::new(Some(plan), origin);
    for frame in [0, 1, 1000, 48000, 100000, 143000] {
        let beat = origin + f64::from(frame) / f64::from(rate) * bpm / 60.0;
        let mut output = ([0.0; 2], false);
        assert_eq!(test_alloc::measure(|| output = playback.sample(0, beat)), Default::default());
        assert!(output.1);
        let expected = audio.data[frame as usize];
        assert!((output.0[0] - expected).abs() < 1e-7, "frame {frame}: {} vs {expected}", output.0[0]);
        assert_eq!(output.0[0], output.0[1]);
    }
    let mut invalid = model;
    invalid.sources[0].audio_clock.as_mut().unwrap().exact_bpm = Some(f64::NAN);
    assert!(invalid.validate(&[audio], &layout).is_err());
}
