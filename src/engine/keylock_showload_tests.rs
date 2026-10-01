//! Separate show-load qualification; does not change the matched quality corpus
//! or the original supported-workloads-v1 audio/state goldens.
use super::*;
use serde_json::{json, Value};
use std::{
    fs,
    io::Write,
    os::unix::fs::DirBuilderExt,
    path::{Path, PathBuf},
};

const RATES: [u32; 3] = [44_100, 48_000, 96_000];
const FRAMES: [usize; 2] = [128, 256];
const RATIOS: [f32; 3] = [0.5, 0.84, 1.5];
const BLOCKS: usize = 2048;
const WARMUP: usize = 128;
const REPEATS: usize = 3;
const POLICY: &str = include_str!("../../benchmarks/keylock-show-policy.json");

fn workload(rate: u32, frames: usize, ratio: f32, blocks: usize) -> Value {
    workload_observed(rate, frames, ratio, blocks, false)
}
pub(super) fn workload_observed(rate: u32, frames: usize, ratio: f32, blocks: usize, history: bool) -> Value {
    let (engine, mut rt) = performance_workload_tests::prepared_at("hybrid", rate);
    // Identical non-silent media and origins align both decks' expensive hops.
    // This original procedural source is a timing stressor, not listening proof.
    let (drums, harmony) = demo_stems(48_000, 124.0);
    let audio = Arc::new(Sample {
        name: "Original procedural simultaneous-search stress".into(),
        sr: 48_000,
        ch: 2,
        data: drums
            .data
            .iter()
            .zip(&harmony.data)
            .map(|(a, b)| 0.35 * (a + b) + 0.0001)
            .collect(),
        peaks: Vec::new().into(),
        bpm: 124.0,
        path: String::new(),
    });
    for deck in 0..2 {
        rt.apply(Command::DeckAudio {
            deck,
            audio: audio.clone(),
        });
        rt.apply(Command::DeckKeylock { deck });
        rt.apply(Command::DeckPlay { deck });
        let d = &mut rt.decks[deck as usize];
        d.pitch_range = 2;
        d.pitch = ratio - 0.5;
        d.rate = ratio;
        d.target_rate = ratio;
        d.sync = false;
        d.loop_on = true;
        d.loop_start = 0.0;
        d.loop_len = audio.frames() as f64;
        d.transition_to(0.0, rate as f32, DeckTransition::Jump);
    }
    let bpm = rt.bpm as f64;
    let mut callback = audio::OutputCallback::new(rt, 2);
    let mut output = vec![0.0f32; frames * 2];
    for _ in 0..WARMUP {
        callback.render(&mut output);
    }
    let history_handle = history.then(|| engine.performance_history.clone().unwrap());
    let history_receiver = history_handle.as_ref().map(|handle| handle.take_observations().unwrap());
    let history_start = history_handle.as_ref().map(|handle| handle.submit(history_measurement::control::Action::Start(1)).unwrap());
    let mut history_frames = [0_u64; 2];
    let before = callback
        .renderer_for_test()
        .decks
        .each_ref()
        .map(|d| d.keylock_dsp.analysis_count());
    let mut previous_searches = callback
        .renderer_for_test()
        .decks
        .each_ref()
        .map(|d| d.keylock_dsp.full_search_count());
    let before_searches = previous_searches;
    let mut coincident_search_callbacks = 0;
    let mut wall = Vec::with_capacity(blocks);
    let mut render_cpu = Vec::with_capacity(blocks);
    let mut full_cpu = Vec::with_capacity(blocks);
    let (mut allocations, mut frees, mut rejected) = (0, 0, 0);
    let (mut energy, mut peak) = (0.0_f64, 0.0_f32);
    let mut finite = true;
    let mut synchronized = true;
    let mut locked = true;
    let mut hash = 0xcbf29ce484222325u64;
    for block in 0..blocks {
        performance_workload_tests::controls("hybrid", block, true, &mut |command| {
            rejected += usize::from(engine.send(command).is_err());
        });
        let cpu_start = audio_metrics::thread_cpu_ns().expect("Linux thread CPU clock");
        let counts = test_alloc::measure(|| callback.render(&mut output));
        full_cpu.push(
            audio_metrics::thread_cpu_ns()
                .unwrap()
                .saturating_sub(cpu_start),
        );
        allocations += counts.allocations;
        frees += counts.frees;
        let measurement = engine
            .cmd
            .audio_metrics()
            .last_callback
            .expect("actual callback telemetry");
        wall.push(measurement.elapsed_ns);
        render_cpu.push(measurement.render_cpu_ns.expect("render CPU telemetry"));
        let decks = &callback.renderer_for_test().decks;
        synchronized &= decks[0].keylock_dsp.phase == decks[1].keylock_dsp.phase
            && decks[0].keylock_dsp.analysis_count() == decks[1].keylock_dsp.analysis_count();
        let searches = decks.each_ref().map(|d| d.keylock_dsp.full_search_count());
        let deltas = [
            searches[0] - previous_searches[0],
            searches[1] - previous_searches[1],
        ];
        synchronized &= deltas[0] == deltas[1];
        coincident_search_callbacks += usize::from(deltas[0] > 0 && deltas[0] == deltas[1]);
        previous_searches = searches;
        locked &= decks
            .iter()
            .all(|d| d.keylock_mode() == keylock::Mode::Locked && (d.rate - ratio).abs() < 1e-7);
        for sample in &output {
            finite &= sample.is_finite();
            energy += (*sample as f64).powi(2);
            peak = peak.max(sample.abs());
            let quantized = (sample.clamp(-2.0, 2.0) * 100_000.0).round() as i32;
            for byte in quantized.to_le_bytes() {
                hash = (hash ^ byte as u64).wrapping_mul(0x100000001b3);
            }
        }
        if block == 0 {
            if let Some(handle) = &history_handle {
                assert_eq!(handle.poll(history_start.unwrap()).unwrap().unwrap().outcome, history_measurement::control::Outcome::Started);
            }
        }
        if let Some(receiver) = &history_receiver {
            for observation in receiver.try_iter() {
                if observation.classification == history_measurement::Classification::Active {
                    history_frames[observation.episode.deck as usize] += u64::from(observation.frames);
                }
            }
        }
        std::thread::yield_now(); // Outside callback, same retirement scheduling as #95.
    }
    if history {
        let handle = history_handle.as_ref().unwrap();
        let end = handle.submit(history_measurement::control::Action::End(1)).unwrap();
        callback.renderer_mut_for_test().process(&mut []);
        let ack = handle.poll(end).unwrap().unwrap();
        assert_eq!(ack.outcome, history_measurement::control::Outcome::Ended);
        assert!(!ack.incomplete && ack.dropped == 0);
        for observation in history_receiver.as_ref().unwrap().try_iter() {
            if observation.classification == history_measurement::Classification::Active {
                history_frames[observation.episode.deck as usize] += u64::from(observation.frames);
            }
        }
        assert!(history_frames.into_iter().all(|frames| frames > 0));
    }
    let rt = callback.renderer_for_test();
    let hops = rt.decks.each_ref().map(|d| d.keylock_dsp.analysis_count());
    let hops = [hops[0] - before[0], hops[1] - before[1]];
    let searches = [
        previous_searches[0] - before_searches[0],
        previous_searches[1] - before_searches[1],
    ];
    let last_parameters = (blocks - 1) / 8;
    let parameters_applied = rt.xfader == (last_parameters % 128) as f32 / 127.0
        && rt.tracks.iter().enumerate().all(|(track, t)| {
            let phase = ((last_parameters + track * 11) % 128) as f32 / 127.0;
            t.gain == 0.12 + phase * 0.12
                && t.fx.slots.len() == 3
                && t.fx.slots.iter().map(|slot| slot.id()).eq([
                    fx::FxId::Eq3,
                    fx::FxId::Comp,
                    fx::FxId::Reverb,
                ])
        });
    let expected_added = blocks / 16;
    let originals = rt.tracks.iter().enumerate().all(|(track, t)| {
        let notes = &t.clips[0].notes;
        notes.len() == 1024 + if track == 1 { expected_added } else { 0 }
            && notes.iter().take(1024).enumerate().all(|(n, note)| {
                note.pitch == 36 + ((n + track * 5) % 48) as u8
                    && note.start == n as f32 * 0.125
                    && note.len == 0.45
                    && note.vel == 45 + (n % 55) as u8
            })
    });
    let recorded = rt.tracks[1].clips[0].notes.len() == 1024 + expected_added
        && rt.tracks[1].clips[0]
            .notes
            .iter()
            .skip(1024)
            .enumerate()
            .all(|(n, note)| {
                let start = (WARMUP + n * 16) as f64 * frames as f64 / rate as f64 * bpm / 60.0;
                let duration = 8.0 * frames as f64 / rate as f64 * bpm / 60.0;
                note.pitch == 48 + (n % 24) as u8
                    && note.vel == 87
                    && (note.start as f64 - start.rem_euclid(128.0)).abs() < 0.0001
                    && (note.len as f64 - duration).abs() < 0.000001
            })
        && !rt.has_held_project_notes();
    rt.undo.publish();
    let deadline = frames as u64 * 1_000_000_000 / rate as u64;
    json!({"samples":{"callback_wall_ns":wall,"render_cpu_ns":render_cpu,"full_callback_thread_cpu_ns":full_cpu},
        "metrics":{"allocations":allocations,"frees":frees,"rejected_commands":rejected,
            "rms":(energy/(blocks*output.len()) as f64).sqrt(),"peak":peak,
            "wall_deadline_exceedances":wall.iter().filter(|v|**v>deadline).count(),
            "render_cpu_deadline_exceedances":render_cpu.iter().filter(|v|**v>deadline).count(),
            "full_cpu_deadline_exceedances":full_cpu.iter().filter(|v|**v>deadline).count()},
        "checks":{"finite_output":finite,"nonzero_output":energy>0.0,"original_notes_intact":originals,
            "exact_recorded_notes":recorded,"all_commands_applied":parameters_applied&&engine.cmd.len()==0&&engine.undo.view().failures==0,
            "both_locked_at_fixed_ratio":locked,"coincident_search_hops":synchronized&&searches[0]>0&&searches==hops&&coincident_search_callbacks>0},
        "observations":{"searches_per_deck":searches,"analysis_hops_per_deck":hops,
            "coincident_search_callbacks":coincident_search_callbacks,"quantized_audio_hash":format!("{hash:016x}")}})
}

#[test]
fn showload_fixture_uses_two_locked_decks_and_preserves_exact_recording() {
    for (rate, frames, ratio, blocks) in [
        (44_100, 128, 0.5, 32),
        (48_000, 128, 0.84, BLOCKS),
        (96_000, 256, 1.5, 32),
    ] {
        let measured = workload(rate, frames, ratio, blocks);
        for (name, value) in measured["checks"].as_object().unwrap() {
            assert_eq!(value, true, "{rate}/{frames}/{ratio}: {name}: {measured}");
        }
        for name in ["allocations", "frees", "rejected_commands"] {
            assert_eq!(measured["metrics"][name], 0);
        }
    }
}

#[test]
#[ignore = "explicit source-bound, bounded two-deck show-load timing; use check-keylock-show-load.py"]
fn export_keylock_showload() {
    assert!(!cfg!(debug_assertions), "use a release test executable");
    let base = std::env::var_os("OMATAINER_KEYLOCK_EVIDENCE_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("target/keylock-quality"));
    assert!(base.is_absolute());
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&base)
        .unwrap();
    let out = PathBuf::from(
        std::env::var_os("OMATAINER_KEYLOCK_SHOW_OUT").expect("explicit output directory"),
    );
    assert!(out.is_absolute() && !out.exists());
    assert!(out
        .parent()
        .unwrap()
        .canonicalize()
        .unwrap()
        .starts_with(base.canonicalize().unwrap()));
    fs::DirBuilder::new().mode(0o700).create(&out).unwrap();
    let mut workloads = Vec::new();
    for rate in RATES {
        for frames in FRAMES {
            for ratio in RATIOS {
                eprintln!("Two-deck keylock show: {rate}Hz/{frames}frames/{ratio:.2}");
                workloads.push(json!({"id":format!("show_{rate}_{frames}_{ratio:.2}"),
            "conditions":{"sample_rate":rate,"frames":frames,"ratio":ratio,"blocks":BLOCKS,"warmup_blocks":WARMUP,
                "tracks":8,"notes_per_track":1024,"fx_per_track":3,"recorded_notes":BLOCKS/16,
                "locked_decks":2,"parameter_interval_blocks":8,"source_sr":48000},
            "measurements":(0..REPEATS).map(|_|workload(rate,frames,ratio,BLOCKS)).collect::<Vec<_>>()}));
            }
        }
    }
    let report = json!({"schema":1,"suite":"keylock-showload-v1","policy":serde_json::from_str::<Value>(POLICY).unwrap(),
        "embedded_manifest":serde_json::from_str::<Value>(crate::licenses::MANIFEST).unwrap(),"workloads":workloads});
    let bytes = serde_json::to_vec(&report).unwrap();
    assert!(bytes.len() < 32 * 1024 * 1024);
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(out.join("raw.json"))
        .unwrap();
    file.write_all(&bytes).unwrap();
    file.sync_all().unwrap();
}
