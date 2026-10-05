use super::*;
use crate::engine::{beatgrid::Grid, dsp::Sample, test_alloc, Command, Engine};

#[test]
fn captured_clicks_and_output_positions_agree_across_source_rates_and_a_tempo_change() {
    for (source_rate, output_rate) in [(44100, 48000), (48000, 44100), (96000, 48000)] {
        let (engine, mut rt) = Engine::headless_for_test(output_rate, 128);
        let source_clicks = [0.5, 1.0, 1.0 + 2.0 / 3.0, 1.0 + 4.0 / 3.0];
        let mut pcm = vec![0.0; source_rate * 4];
        for seconds in source_clicks {
            let center = (seconds * source_rate as f64).round() as usize;
            let radius = source_rate / 1000;
            for (index, sample) in pcm
                .iter_mut()
                .enumerate()
                .skip(center - radius)
                .take(radius * 2 + 1)
            {
                *sample = (1.0 - index.abs_diff(center) as f32 / radius as f32) * 0.3;
            }
        }
        let audio = Arc::new(Sample {
            name: "Variable click clock".into(),
            sr: source_rate as u32,
            ch: 1,
            data: pcm,
            peaks: Arc::new(vec![]),
            bpm: 120.0,
            path: String::new(),
        });
        rt.apply(Command::DeckAudio {
            deck: 0,
            audio: audio.clone(),
        });
        rt.decks[1].audio = None;
        rt.decks[0].grid = Some(
            Grid::new(0.0, 120.0)
                .unwrap()
                .with_anchor(2.0, 1.0)
                .unwrap()
                .with_anchor(4.0, 1.0 + 4.0 / 3.0)
                .unwrap(),
        );
        rt.decks[0].sync = true;
        rt.decks[0].sync_bpm = 120.0;
        rt.decks[0].playing = true;
        rt.xfader = 0.0;
        let initial_fade_frames = rt.decks[0].transition_remaining as usize;
        assert!(initial_fade_frames > 0);
        let mut callback = OutputCallback::new(rt, 2);
        let handle = engine.audible.clone();
        let count = output_rate as usize * 21 / 10;
        let mut captured = vec![0.0; count];
        let mut source_positions = vec![0.0; count];
        let origin = 1_020_000_000u64;
        let mut output = [0.0f32; 256];
        let counts = test_alloc::measure(|| {
            for start in (0..count).step_by(128) {
                let frames = (count - start).min(128);
                let anchor = origin + start as u64 * 1_000_000_000 / u64::from(output_rate);
                callback.render_at(
                    &mut output[..frames * 2],
                    Some(std::time::Duration::from_millis(20)),
                    Some(anchor),
                );
                for offset in 0..frames {
                    let time =
                        anchor + (offset as u64 * 1_000_000_000).div_ceil(u64::from(output_rate));
                    let position = handle.positions_at(time).unwrap()[0];
                    assert_eq!(position.output_frame, (start + offset) as u64);
                    if start + offset + 1 < initial_fade_frames {
                        assert_eq!(position.media_key, 0);
                    } else {
                        assert_ne!(position.media_key, 0);
                    }
                    captured[start + offset] =
                        output[offset * 2].abs().max(output[offset * 2 + 1].abs());
                    source_positions[start + offset] = position.source_frame;
                }
            }
        });
        assert_eq!(counts, test_alloc::Counts::default());
        for (click, seconds) in source_clicks.iter().enumerate() {
            let expected = ((click + 1) as f64 * 0.5 * output_rate as f64).round() as usize;
            let radius = output_rate as usize / 200;
            let peak = (expected - radius..=expected + radius)
                .max_by(|a, b| captured[*a].total_cmp(&captured[*b]))
                .unwrap();
            assert!(captured[peak] > 0.01, "missing captured click {click}");
            assert!(
                peak.abs_diff(expected) <= 4,
                "captured click {click}: {peak}/{expected}"
            );
            assert!(
                (source_positions[peak] / source_rate as f64 - seconds).abs() < 0.0002,
                "visual source position disagrees with captured click {click}"
            );
        }
        assert!(Arc::ptr_eq(
            callback.rt.decks[0].audio.as_ref().unwrap(),
            &audio
        ));
    }
}

#[test]
fn queued_positions_survive_actual_reverse_loop_seek_and_oversized_callbacks() {
    let (engine, mut rt) = Engine::headless_for_test(48000, 128);
    rt.decks[0].playing = true;
    rt.decks[0].pos = 50000.0;
    rt.decks[0].loop_on = true;
    rt.decks[0].loop_start = 48000.0;
    rt.decks[0].loop_len = 2400.0;
    let mut callback = OutputCallback::new(rt, 2);
    let handle = engine.audible.clone();
    let mut output = vec![0.0f32; 40000 * 2];
    assert_eq!(
        test_alloc::measure(|| callback.render_at(&mut output, None, Some(1_000_000_000))),
        test_alloc::Counts::default()
    );
    let before = handle.positions_at(1_100_000_000).unwrap()[0];
    assert!((48000.0..50400.0).contains(&before.source_frame));
    callback.rt.apply(Command::DeckTouch { deck: 0, on: true });
    callback.rt.apply(Command::DeckJog {
        deck: 0,
        delta: -1.0,
    });
    let initial = callback.rt.decks[0].pos;
    let anchor = 1_000_000_000 + 40000u64 * 1_000_000_000 / 48000;
    callback.render_at(&mut output[..256], None, Some(anchor));
    let first = handle.positions_at(anchor).unwrap()[0].source_frame;
    let last = handle
        .positions_at(anchor + (127u64 * 1_000_000_000).div_ceil(48000))
        .unwrap()[0]
        .source_frame;
    assert!(
        first < initial && last < first,
        "scratch must reverse the captured trajectory"
    );
    assert_eq!(
        handle.positions_at(1_100_000_000).unwrap()[0].source_frame,
        before.source_frame
    );
    callback.rt.apply(Command::DeckSeek { deck: 0, frac: 0.5 });
    let anchor = anchor + 128u64 * 1_000_000_000 / 48000;
    callback.render_at(&mut output[..256], None, Some(anchor));
    assert_ne!(handle.positions_at(anchor).unwrap()[0].source_frame, last);
    callback.rt.audible.restart();
    assert!(handle.positions_at(anchor).is_none());
}
