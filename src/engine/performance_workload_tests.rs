//! Opt-in production callback workloads. This is CPU/render evidence, not an
//! audio-device, driver scheduling, converter latency or XRUN measurement.
use super::*;
use audio::OutputCallback;
use serde_json::{json, Value};
const RATE: u32 = 48_000;
const BLOCKS: usize = 8192;
const WARMUP: usize = 128;

fn prepared(role: &str) -> (Engine, RtEngine) {
    prepared_at(role, RATE)
}

pub(super) fn prepared_at(role: &str, rate: u32) -> (Engine, RtEngine) {
    let (engine, mut rt) = Engine::headless_for_test(rate, 256);
    rt.quant = 0.0;
    rt.master = 0.4;
    if role != "live_dj" {
        for track in 0..TRACKS {
            rt.tracks[track].kind = 1 + (track % 3) as u8;
            rt.tracks[track].gain = 0.18;
            rt.tracks[track].clips[0].kind = ClipKind::Midi;
            rt.tracks[track].clips[0].bars = 32.0;
            rt.tracks[track].fx.slots = [fx::FxId::Eq3, fx::FxId::Comp, fx::FxId::Reverb]
                .into_iter()
                .map(|id| fx::FxSlot::new(id, rate as f32))
                .collect();
            rt.apply(Command::SetNotes {
                track: track as u8,
                scene: 0,
                notes: (0..1024)
                    .map(|note| MidiNote { variation: None,
                        channel:0,release_vel:64,source_timing:None, id: crate::engine::midi_edit::NoteId::new(), muted: false,
                        pitch: 36 + ((note + track * 5) % 48) as u8,
                        start: note as f32 * 0.125,
                        len: 0.45,
                        vel: 45 + (note % 55) as u8,
                    })
                    .collect(),
            });
        }
        rt.apply(Command::LaunchScene { scene: 0 });
    }
    if matches!(role, "composer" | "hybrid") {
        rt.apply(Command::ComposeArm { track: 1, scene: 0 });
        rt.apply(Command::Record);
    }
    if matches!(role, "live_dj" | "hybrid") {
        for deck in 0..2 {
            rt.apply(Command::DeckLoop { deck, beats: 16.0 });
            if !rt.decks[deck as usize].playing {
                rt.apply(Command::DeckPlay { deck });
            }
        }
    }
    (engine, rt)
}

fn workload(role: &str, frames: usize) -> Value {
    let (engine, rt) = prepared(role);
    let bpm = rt.bpm as f64;
    let mut callback = OutputCallback::new(rt, 2);
    let mut output = vec![0.0f32; frames * 2];
    for _ in 0..WARMUP {
        callback.render(&mut output);
    }
    let mut wall = Vec::with_capacity(BLOCKS);
    let mut cpu = Vec::with_capacity(BLOCKS);
    let mut allocations = 0;
    let mut frees = 0;
    let mut energy = 0.0f64;
    let mut peak = 0.0f32;
    let mut hash = 0xcbf29ce484222325u64;
    let mut rejected = 0usize;
    for block in 0..BLOCKS {
        // Musical control changes are outside the callback allocation sample.
        // The complete suite adds App/MIDI/IPC entry paths separately.
        let mut submit = |command| {
            if engine.send(command).is_err() {
                rejected += 1;
            }
        };
        controls(role, block, false, &mut submit);
        let measured = test_alloc::measure(|| callback.render(&mut output));
        allocations += measured.allocations;
        frees += measured.frees;
        let measurement = engine
            .cmd
            .audio_metrics()
            .last_callback
            .expect("production callback telemetry");
        wall.push(measurement.elapsed_ns);
        cpu.push(
            measurement
                .render_cpu_ns
                .expect("Linux callback thread CPU clock"),
        );
        for sample in &output {
            assert!(sample.is_finite(), "{role}: nonfinite output");
            energy += (*sample as f64).powi(2);
            peak = peak.max(sample.abs());
            let quantized = (sample.clamp(-2.0, 2.0) * 100_000.0).round() as i32;
            for byte in quantized.to_le_bytes() {
                hash = (hash ^ byte as u64).wrapping_mul(0x100000001b3);
            }
        }
        // Permit the real snapshot/history recyclers to run between blocks;
        // this is outside the measured production callback, never an RT yield.
        std::thread::yield_now();
    }
    let rt = callback.renderer_for_test();
    let recording = matches!(role, "composer" | "hybrid");
    let expected_added = if recording { BLOCKS / 16 } else { 0 };
    let original_notes_intact = role == "live_dj"
        || rt.tracks.iter().enumerate().all(|(track, t)| {
            let notes = &t.clips[0].notes;
            notes.len() == 1024 + if track == 1 { expected_added } else { 0 }
                && notes.iter().take(1024).enumerate().all(|(n, note)| {
                    note.pitch == 36 + ((n + track * 5) % 48) as u8
                        && note.start == n as f32 * 0.125
                        && note.len == 0.45
                        && note.vel == 45 + (n % 55) as u8
                })
        });
    let exact_recorded_notes = !recording || {
        let notes = &rt.tracks[1].clips[0].notes;
        notes.len() == 1024 + expected_added
            && notes.iter().skip(1024).enumerate().all(|(n, note)| {
                let start = (WARMUP + n * 16) as f64 * frames as f64 / RATE as f64 * bpm / 60.0;
                let duration = 8.0 * frames as f64 / RATE as f64 * bpm / 60.0;
                note.pitch == 48 + (n % 24) as u8
                    && note.vel == 87
                    && (note.start as f64 - start.rem_euclid(128.0)).abs() < 0.0001
                    && (note.len as f64 - duration).abs() < 0.000001
            })
            && !rt.has_held_project_notes()
    };
    json!({"metrics":{"allocations":allocations,"frees":frees,
        "rejected_commands":rejected,"rms":(energy/(BLOCKS*output.len()) as f64).sqrt(),"peak":peak},
        "samples":{"callback_wall_ns":wall,"render_cpu_ns":cpu},
        "checks":{"finite_output":true,"nonzero_output":energy>0.0,
            "original_notes_intact":original_notes_intact,"exact_recorded_notes":exact_recorded_notes,
            "all_commands_applied":engine.cmd.len()==0 && engine.undo.view().failures==0},
        "observations":{"quantized_audio_hash":format!("{hash:016x}")}})
}

/// Same producer event order for the original show gate and the locked-deck
/// extension. Fixed decks retain their declared ratio and aligned search hops.
pub(super) fn controls(
    role: &str,
    block: usize,
    fixed_decks: bool,
    submit: &mut impl FnMut(Command),
) {
    if role != "live_dj" && block % 8 == 0 {
        for track in 0..TRACKS {
            let phase = ((block / 8 + track * 11) % 128) as f32 / 127.0;
            submit(Command::TrackGain {
                track: track as u8,
                value: 0.12 + phase * 0.12,
            });
        }
    }
    if matches!(role, "composer" | "hybrid") {
        let note = 48 + ((block / 16) % 24) as u8;
        if block % 16 == 0 {
            submit(Command::LiveNoteOn {
                source: 901,
                ch: 0,
                note,
                vel: 87,
            });
        }
        if block % 16 == 8 {
            submit(Command::LiveNoteOff {
                source: 901,
                ch: 0,
                note,
            });
        }
    }
    if matches!(role, "live_dj" | "hybrid") && block % 8 == 0 {
        let phase = ((block / 8) % 128) as f32 / 127.0;
        submit(Command::Xfader(phase));
        if !fixed_decks {
            submit(Command::DeckPitch {
                deck: 1,
                value: 0.46 + phase * 0.08,
            });
            submit(Command::DeckJog {
                deck: 0,
                delta: if block % 32 == 0 { -0.002 } else { 0.001 },
            });
        }
    }
}

pub(crate) fn callbacks() -> Vec<Value> {
    [
        ("producer", 256),
        ("composer", 256),
        ("live_dj", 128),
        ("hybrid", 256),
    ]
    .into_iter()
    .map(|(role, frames)| {
        let synth = role != "live_dj";
        json!({"id":format!("callback_{role}"),
                "conditions":{"sample_rate":RATE,"frames":frames,"blocks":BLOCKS,
                    "warmup_blocks":WARMUP,"channels":2,"tracks":if synth{8}else{0},
                    "notes_per_track":if synth{1024}else{0},"fx_per_track":if synth{3}else{0},
                    "parameter_sweep_tracks":if synth{8}else{0},"parameter_interval_blocks":8,
                    "live_note_recording":matches!(role,"composer"|"hybrid")},
                "measurements":(0..3).map(|_|workload(role,frames)).collect::<Vec<_>>()})
    })
    .collect()
}

#[test]
fn note_bursts_reuse_release_tails_before_stealing_current_holds() {
    use dsp::{InputKey, Poly};
    use instrument::SynthInstrument;
    let mut poly = Poly::new(48_000.0, SynthInstrument::Keys, 8);
    for pitch in 36..44 {
        poly.note_on_clip(pitch, 0.8);
    }
    for _ in 0..64 {
        poly.tick(48_000.0);
    }
    poly.release_clip();
    let inputs = std::array::from_fn::<_, 4, _>(|index| InputKey::Midi {
        source: 100 + index as u64 / 2,
        ch: 0,
        note: 60 + (index % 2) as u8 * 12,
    });
    let measured = test_alloc::measure(|| {
        for (index, input) in inputs.iter().enumerate() {
            poly.note_on_input(60 + index as u8, 0.8, *input);
        }
        for pitch in 48..51 {
            poly.note_on_clip(pitch, 0.7);
        }
    });
    assert_eq!((measured.allocations, measured.frees), (0, 0));
    for input in inputs {
        assert_eq!(
            poly.voices
                .iter()
                .filter(|voice| voice.input == Some(input) && matches!(voice.env.stage, 1..=3))
                .count(),
            1
        );
        poly.note_off_input(input);
    }
    assert_eq!(
        poly.voices
            .iter()
            .filter(|voice| voice.input.is_some() && matches!(voice.env.stage, 1..=3))
            .count(),
        0
    );
    assert_eq!(poly.voices.iter().filter(|voice| voice.owner==dsp::VoiceOwner::Clip && matches!(voice.env.stage,1..=3)).count(),3);
}
