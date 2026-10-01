//! Opt-in, hardware-free production-renderer measurements. Objective diagnostics
//! are not perceptual scores. File/decode/analysis work is outside timed blocks.
use super::*;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, OpenOptions},
    io::{BufWriter, Write},
    os::unix::fs::DirBuilderExt,
    path::{Path, PathBuf},
    time::Instant,
};

const RATIOS: [f32; 7] = [0.5, 0.84, 0.92, 1.0, 1.08, 1.16, 1.5];
const RATES: [u32; 3] = [44_100, 48_000, 96_000];
const BLOCKS: [usize; 3] = [64, 128, 512];
const OUTPUT_LIMIT: u64 = 2 * 1024 * 1024 * 1024;
const ROOT: &str = "/home/michael/Projects/omatainer-work";
struct Corpus {
    id: &'static str,
    kind: &'static str,
    audio: Arc<Sample>,
    original_sha: Option<String>,
}
#[derive(Clone, Copy)]
struct Cost {
    cpu: Option<u64>,
    wall: u64,
    frames: usize,
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn pcm_digest(data: &[f32]) -> String {
    let mut hash = Sha256::new();
    for sample in data {
        hash.update(sample.to_le_bytes());
    }
    format!("{:x}", hash.finalize())
}
fn sample(id: &str, sr: u32, data: Vec<f32>) -> Arc<Sample> {
    Arc::new(Sample {
        name: id.into(),
        sr,
        ch: 2,
        data,
        peaks: vec![].into(),
        bpm: 120.0,
        path: String::new(),
    })
}
fn corpus() -> Vec<Corpus> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/keylock");
    let provenance: Value =
        serde_json::from_slice(&fs::read(root.join("sources.json")).unwrap()).unwrap();
    let mut corpus = Vec::new();
    for (id, file) in [
        ("vocal_f1", "f1_arpeggios_c_slow_forte_i.wav"),
        ("vocal_m1", "m1_arpeggios_c_slow_forte_i.wav"),
    ] {
        let path = root.join(file);
        let sha = digest(&fs::read(&path).unwrap());
        let record = provenance["files"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["file"] == file)
            .unwrap();
        assert_eq!(
            sha,
            record["sha256"].as_str().unwrap(),
            "recorded corpus changed"
        );
        let audio = Arc::new(decode::decode_audio(&path).unwrap().sample);
        corpus.push(Corpus {
            id,
            kind: "recorded VocalSet singing vowel; CC-BY-4.0",
            audio,
            original_sha: Some(sha),
        });
    }
    let sr = 48_000;
    let mut bass = vec![0.0; sr as usize * 6 * 2];
    for (i, frame) in bass.chunks_exact_mut(2).enumerate() {
        let t = i as f64 / sr as f64;
        let f = if t < 3.0 { 55.0 } else { 93.75 };
        let phase = std::f64::consts::TAU * f * t;
        for (channel, out) in frame.iter_mut().enumerate() {
            let p = phase + channel as f64 * 0.35;
            *out = ((p.sin() + if t < 3.0 { 0.2 * (2.0 * p).sin() } else { 0.0 })
                * 0.35
                * if channel == 0 { 1.0 } else { 0.75 }) as f32;
        }
    }
    corpus.push(Corpus {
        id: "bass",
        kind: "original analytic stereo bass:55Hz+harmonic, then93.75Hz stress",
        audio: sample("bass", sr, bass),
        original_sha: None,
    });
    let mut transients = vec![0.0; sr as usize * 6 * 2];
    for pulse in 1..12 {
        for channel in 0..2 {
            let start = pulse * sr as usize / 2 + channel * 72; // R delayed 1.5ms in source time.
            for n in 0..240 {
                let envelope = (1.0 - n as f64 / 240.0).powi(2);
                transients[(start + n) * 2 + channel] +=
                    (0.65 * envelope * if n % 2 == 0 { 1.0 } else { -1.0 }) as f32;
            }
        }
    }
    corpus.push(Corpus {
        id: "transients",
        kind: "original5ms alternating impulses every0.5s; right delayed1.5ms",
        audio: sample("transients", sr, transients),
        original_sha: None,
    });
    let (drums, harmony) = demo_stems(sr, 124.0);
    let mix: Vec<_> = drums
        .data
        .iter()
        .zip(&harmony.data)
        .map(|(a, b)| 0.35 * (a + b))
        .collect();
    corpus.push(Corpus {
        id: "full_mix",
        kind: "complete original procedural Omatainer16-beat drums+harmony; fixed0.35sum",
        audio: sample("full_mix", sr, mix),
        original_sha: None,
    });
    corpus
}
fn renderer(sr: u32) -> (CommandPort, RtEngine) {
    let (port, rx) = CommandPort::channel(256);
    let rt = RtEngine::new(sr as f32, rx, Arc::new(Mutex::new(Snapshot::default())));
    (port, rt)
}
fn prepare(rt: &mut RtEngine, audio: Arc<Sample>, ratio: f32, locked: bool) {
    // Fixture preparation is off the measured thread interval and bypasses undo
    // admission; the production load/reset handler and render path are unchanged.
    rt.decks[0].last_output = [0.0; 2];
    rt.apply_plain(Command::DeckAudio { deck: 0, audio });
    rt.apply_plain(Command::DeckUnload { deck: 1 });
    let d = &mut rt.decks[0];
    d.gain = 1.0;
    d.pitch_range = 2;
    d.pitch = ratio - 0.5;
    d.rate = ratio;
    d.target_rate = ratio;
    d.sync = false;
    d.playing = true;
    d.touching = false;
    d.keylock = locked;
    // Fixed initial rate avoids measuring a different control ramp at each ratio.
    // Load/seek reset still follows the production transition and DSP path.
    d.transition_to(0.0, rt.sr, DeckTransition::Jump);
    rt.playing = false;
    rt.master = 1.0;
    rt.xfader = 0.0;
    rt.fx_wet = [0.0; 3];
}
fn write_new(path: &Path, bytes: &[u8]) {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .unwrap();
    file.write_all(bytes).unwrap();
    file.sync_all().unwrap();
}
fn wav(path: &Path, sr: u32, data: &[f32]) {
    assert!(data.len().is_multiple_of(2));
    let bytes = u32::try_from(data.len() * 4).unwrap();
    let mut file = BufWriter::new(
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .unwrap(),
    );
    file.write_all(b"RIFF").unwrap();
    file.write_all(&(36 + bytes).to_le_bytes()).unwrap();
    file.write_all(b"WAVEfmt ").unwrap();
    file.write_all(&16u32.to_le_bytes()).unwrap();
    file.write_all(&3u16.to_le_bytes()).unwrap();
    file.write_all(&2u16.to_le_bytes()).unwrap();
    file.write_all(&sr.to_le_bytes()).unwrap();
    file.write_all(&(sr * 8).to_le_bytes()).unwrap();
    file.write_all(&8u16.to_le_bytes()).unwrap();
    file.write_all(&32u16.to_le_bytes()).unwrap();
    file.write_all(b"data").unwrap();
    file.write_all(&bytes.to_le_bytes()).unwrap();
    for sample in data {
        file.write_all(&sample.to_le_bytes()).unwrap();
    }
    file.flush().unwrap();
    file.get_ref().sync_all().unwrap();
}
fn distribution(values: impl Iterator<Item = u64>) -> Value {
    let mut values: Vec<_> = values.collect();
    values.sort_unstable();
    if values.is_empty() {
        return Value::Null;
    }
    let quantile = |p: f64| values[((values.len() - 1) as f64 * p).round() as usize];
    json!({"count":values.len(),"min":values[0],"mean":values.iter().map(|v| *v as f64).sum::<f64>() / values.len() as f64,
        "p50":quantile(0.5),"p95":quantile(0.95),"p99":quantile(0.99),"max":values[values.len()-1]})
}
fn costs(root: &Path, id: &str, sr: u32, values: &[Cost], counts: test_alloc::Counts) -> Value {
    let name = format!("{id}.timing.csv");
    let mut text = String::from("block,frames,thread_cpu_ns,wall_ns,deadline_ns\n");
    use std::fmt::Write as _;
    for (i, v) in values.iter().enumerate() {
        writeln!(
            text,
            "{i},{},{},{},{}",
            v.frames,
            v.cpu.map_or_else(String::new, |v| v.to_string()),
            v.wall,
            v.frames as u64 * 1_000_000_000 / sr as u64
        )
        .unwrap();
    }
    write_new(&root.join(&name), text.as_bytes());
    json!({"raw_csv":name,"sha256":digest(text.as_bytes()),"thread_cpu_ns":distribution(values.iter().filter_map(|c| c.cpu)),
        "wall_ns":distribution(values.iter().map(|c| c.wall)),
        "wall_deadline_exceedances":values.iter().filter(|c| c.wall > c.frames as u64 * 1_000_000_000 / sr as u64).count(),
        "rust_allocations":counts.allocations,"rust_frees":counts.frees,"rust_allocated_bytes":counts.bytes,
        "scope":"per-block current-thread CPU and elapsed wall; offline execution, not backend XRUN evidence"})
}
fn measure_blocks(
    mut render: impl FnMut(&mut [f32]),
    data: &mut [f32],
    block: usize,
) -> (Vec<Cost>, test_alloc::Counts) {
    let mut result = Vec::with_capacity((data.len() / 2).div_ceil(block));
    let mut total = test_alloc::Counts::default();
    for chunk in data.chunks_mut(block * 2) {
        let cpu = audio_metrics::thread_cpu_ns();
        let start = Instant::now();
        let count = test_alloc::measure(|| render(chunk));
        let wall = start.elapsed().as_nanos().min(u64::MAX as u128) as u64;
        let elapsed = cpu
            .zip(audio_metrics::thread_cpu_ns())
            .map(|(a, b)| b.saturating_sub(a));
        total.allocations += count.allocations;
        total.frees += count.frees;
        total.bytes += count.bytes;
        result.push(Cost {
            cpu: elapsed,
            wall,
            frames: chunk.len() / 2,
        });
    }
    (result, total)
}
fn levels(data: &[f32]) -> Value {
    let channel = |ch: usize| {
        let values: Vec<_> = data.iter().skip(ch).step_by(2).map(|v| *v as f64).collect();
        let finite = values.iter().filter(|v| v.is_finite()).count();
        let peak = values.iter().fold(0.0f64, |a, v| a.max(v.abs()));
        let rms = (values.iter().map(|v| v * v).sum::<f64>() / values.len().max(1) as f64).sqrt();
        let step = values
            .windows(2)
            .fold(0.0f64, |a, v| a.max((v[1] - v[0]).abs()));
        json!({"finite_samples":finite,"samples":values.len(),"rms":rms,"peak":peak,"max_adjacent_step":step,
            "first_above_1e_5_frame":values.iter().position(|v| v.abs()>1e-5)})
    };
    json!([channel(0), channel(1)])
}
fn projection(data: &[f64], sr: f64, hz: f64) -> f64 {
    let omega = std::f64::consts::TAU * hz / sr;
    let coefficient = 2.0 * omega.cos();
    let (mut q1, mut q2) = (0.0, 0.0);
    for value in data {
        let q = value + coefficient * q1 - q2;
        q2 = q1;
        q1 = q;
    }
    (q1 * q1 + q2 * q2 - coefficient * q1 * q2).max(0.0)
}
fn bass_metrics(data: &[f32], sr: u32, ratio: f32, locked: bool) -> Value {
    let mut sections = Vec::new();
    for (source_start, source_end, fundamental, harmonic) in
        [(0.75, 2.25, 55.0, true), (3.75, 5.25, 93.75, false)]
    {
        let a = (source_start / ratio as f64 * sr as f64) as usize;
        let b = ((source_end / ratio as f64 * sr as f64) as usize).min(data.len() / 2);
        let step = (sr / 4000).max(1) as usize;
        let measurement_sr = sr as f64 / step as f64;
        let expected = fundamental * if locked { 1.0 } else { ratio as f64 };
        let mut channels = Vec::new();
        for ch in 0..2 {
            let signal: Vec<_> = (a..b)
                .step_by(step)
                .map(|i| data[2 * i + ch] as f64)
                .collect();
            let windowed: Vec<_> = signal
                .iter()
                .enumerate()
                .map(|(i, value)| {
                    value
                        * (0.5
                            - 0.5
                                * (std::f64::consts::TAU * i as f64 / signal.len().max(1) as f64)
                                    .cos())
                })
                .collect();
            let coarse = (0..=320)
                .map(|i| 20.0 + i as f64 * 0.5)
                .max_by(|a, b| {
                    projection(&windowed, measurement_sr, *a).total_cmp(&projection(
                        &windowed,
                        measurement_sr,
                        *b,
                    ))
                })
                .unwrap();
            let measured = (-10..=10)
                .map(|i| coarse + i as f64 * 0.05)
                .max_by(|a, b| {
                    projection(&windowed, measurement_sr, *a).total_cmp(&projection(
                        &windowed,
                        measurement_sr,
                        *b,
                    ))
                })
                .unwrap();
            let basis: Vec<_> = signal
                .iter()
                .enumerate()
                .map(|(i, _)| {
                    let p = std::f64::consts::TAU * expected * i as f64 / measurement_sr;
                    [
                        p.sin(),
                        p.cos(),
                        if harmonic { (2.0 * p).sin() } else { 0.0 },
                        if harmonic { (2.0 * p).cos() } else { 0.0 },
                    ]
                })
                .collect();
            let n = signal.len().max(1) as f64;
            let coefficients: [f64; 4] = std::array::from_fn(|k| {
                2.0 * signal
                    .iter()
                    .zip(&basis)
                    .map(|(v, b)| v * b[k])
                    .sum::<f64>()
                    / n
            });
            let residual = (signal
                .iter()
                .zip(&basis)
                .map(|(v, b)| {
                    (v - b.iter().zip(coefficients).map(|(b, c)| b * c).sum::<f64>()).powi(2)
                })
                .sum::<f64>()
                / n)
                .sqrt();
            let rms = (signal.iter().map(|v| v * v).sum::<f64>() / n).sqrt();
            let measurable = rms >= 1e-6;
            channels.push(json!({"spectral_peak_hz":measurable.then_some(measured),"peak_measurable_above_rms_1e_6":measurable,"peak_search_step_hz":0.05,"expected_hz":expected,
                "pitch_error_cents":measurable.then(||1200.0*(measured/expected).log2()),"fundamental_amplitude":coefficients[0].hypot(coefficients[1]),
                "off_expected_harmonic_fit_rms":residual,"rms":rms}));
        }
        sections.push(json!({"source_window_seconds":[source_start,source_end],"source_fundamental_hz":fundamental,"channels":channels}));
    }
    json!({"kind":"objective sinusoid diagnostics; residual includes amplitude modulation/sidebands/pitch error and fit leakage, not a perceptual score", "sections":sections})
}
fn transient_metrics(data: &[f32], sr: u32, ratio: f32) -> Value {
    let mut pulses = Vec::new();
    for pulse in 1..11 {
        let source = pulse as f64 * 0.5;
        let center = source / ratio as f64;
        let a = ((center - 0.16).max(0.0) * sr as f64) as usize;
        let b = (((center + 0.16) * sr as f64) as usize).min(data.len() / 2);
        let mut channels = Vec::new();
        for ch in 0..2 {
            let mut energy = 0.0;
            let mut moment = 0.0;
            let mut peak = 0.0f64;
            for i in a..b {
                let v = (data[2 * i + ch] as f64).powi(2);
                energy += v;
                moment += i as f64 * v;
                peak = peak.max(v);
            }
            let onset =
                (a..b).find(|i| (data[2 * i + ch] as f64).powi(2) > peak * 0.01 && peak > 1e-12);
            let mut cumulative = 0.0;
            let (mut lo, mut hi) = (None, None);
            for i in a..b {
                cumulative += (data[2 * i + ch] as f64).powi(2);
                if energy > 1e-12 {
                    if lo.is_none() && cumulative >= energy * 0.025 {
                        lo = Some(i);
                    }
                    if hi.is_none() && cumulative >= energy * 0.975 {
                        hi = Some(i);
                    }
                }
            }
            let expected = source + ch as f64 * 0.0015;
            channels.push(json!({"energy":energy,"source_onset_displacement_ms":onset.map(|i|(i as f64/sr as f64*ratio as f64-expected)*1000.0),
                "source_energy_centroid_seconds":(energy>1e-12).then(||moment/energy/sr as f64*ratio as f64),
                "output_95percent_energy_width_ms":lo.zip(hi).map(|(a,b)|(b-a) as f64/sr as f64*1000.0)}));
        }
        pulses.push(json!({"source_onset_seconds":source,"expected_right_delay_source_ms":1.5,"channels":channels}));
    }
    json!({"kind":"objective transient displacement/envelope diagnostics, not device latency", "pulses":pulses})
}
fn difference(a: &[f32], b: &[f32]) -> Value {
    assert_eq!(a.len(), b.len());
    let peak = a
        .iter()
        .zip(b)
        .fold(0.0f64, |p, (a, b)| p.max((*a as f64 - *b as f64).abs()));
    let rms = (a
        .iter()
        .zip(b)
        .map(|(a, b)| (*a as f64 - *b as f64).powi(2))
        .sum::<f64>()
        / a.len().max(1) as f64)
        .sqrt();
    json!({"peak_absolute_difference":peak,"rms_difference":rms})
}
fn source_reference(audio: &Sample, frames: usize, sr: u32, ratio: f32) -> Vec<f32> {
    (0..frames)
        .flat_map(|i| {
            let (l, r) = audio.at((i + 1) as f64 * audio.sr as f64 / sr as f64 * ratio as f64);
            [l, r]
        })
        .collect()
}
fn alignment(output: &[f32], reference: &[f32], sr: u32) -> Value {
    // Offline correlation on both channels, bounded to a central2s window.
    // Positive lag means measured content occurs later than the reference.
    let step = (sr / 6000).max(1) as usize;
    let start = (sr as usize / 4).min(output.len() / 8);
    let end = (start + sr as usize * 2).min((output.len() / 2).saturating_sub(sr as usize / 40));
    let limit = sr as isize / 50;
    let mut best = (f64::NEG_INFINITY, 0isize);
    for lag in (-limit..=limit).step_by(step) {
        let (mut xy, mut xx, mut yy) = (0.0, 0.0, 0.0);
        for frame in (start..end).step_by(step) {
            let shifted = frame as isize + lag;
            if shifted < 0 || shifted as usize >= output.len() / 2 {
                continue;
            }
            for ch in 0..2 {
                let x = reference[2 * frame + ch] as f64;
                let y = output[2 * shifted as usize + ch] as f64;
                xy += x * y;
                xx += x * x;
                yy += y * y;
            }
        }
        let score = if xx * yy > 1e-20 {
            xy / (xx * yy).sqrt()
        } else {
            0.0
        };
        if score > best.0 {
            best = (score, lag);
        }
    }
    json!({"lag_output_frames":best.1,"lag_ms":best.1 as f64/sr as f64*1000.0,"normalized_correlation":best.0,
        "search_limit_ms":20,"resolution_frames":step,"scope":"central content alignment, not device/output-buffer latency; periodic sources may have ambiguous peaks"})
}
fn transition_cases(root: &Path, source: &Arc<Sample>) -> Vec<Value> {
    let mut records = Vec::new();
    for sr in RATES {
        for block in BLOCKS {
            let (_port, mut rt) = renderer(sr);
            prepare(&mut rt, source.clone(), 0.84, true);
            let events = [
                (sr as usize / 2, "keylock_off"),
                (sr as usize, "keylock_on"),
                (sr as usize * 3 / 2, "seek61percent"),
                (sr as usize * 2, "scratch_reverse_contact"),
                (sr as usize * 2 + sr as usize / 20, "scratch_release"),
                (sr as usize * 5 / 2, "pitch_to1.16"),
                (sr as usize * 3, "seek20percent"),
                (sr as usize * 7 / 2, "pause"),
                (sr as usize * 4, "resume"),
            ];
            let mut output = vec![0.0f32; sr as usize * 5 * 2];
            let mut frame_index = 0usize;
            let (timings, heap) = measure_blocks(
                |chunk| {
                    for frame in chunk.chunks_exact_mut(2) {
                        if let Some((_, name)) = events.iter().find(|(at, _)| *at == frame_index) {
                            match *name {
                                "keylock_off" | "keylock_on" => {
                                    rt.apply(Command::DeckKeylock { deck: 0 })
                                }
                                "seek61percent" => rt.apply(Command::DeckSeek {
                                    deck: 0,
                                    frac: 0.61,
                                }),
                                "scratch_reverse_contact" => {
                                    rt.apply(Command::DeckTouch { deck: 0, on: true });
                                    rt.apply(Command::DeckJog {
                                        deck: 0,
                                        delta: -0.2,
                                    });
                                }
                                "scratch_release" => {
                                    rt.apply(Command::DeckTouch { deck: 0, on: false })
                                }
                                "pitch_to1.16" => rt.apply(Command::DeckPitch {
                                    deck: 0,
                                    value: 0.66,
                                }),
                                "seek20percent" => {
                                    rt.apply(Command::DeckSeek { deck: 0, frac: 0.2 })
                                }
                                "pause" | "resume" => rt.apply(Command::DeckPlay { deck: 0 }),
                                _ => unreachable!(),
                            }
                        }
                        let (l, r) = rt.render_deck(0);
                        frame.copy_from_slice(&[l, r]);
                        frame_index += 1;
                    }
                },
                &mut output,
                block,
            );
            let id = format!("transitions_{sr}_{block}");
            let file = format!("{id}.wav");
            if block == 128 {
                wav(&root.join(&file), sr, &output);
            }
            records.push(json!({"id":id,"output_sr":sr,"block_frames":block,"source":"full_mix","initial_ratio":0.84,
            "wav":(block==128).then_some(file),"pcm_sha256":pcm_digest(&output),"measurement":costs(root,&id,sr,&timings,heap),
            "events":events.iter().map(|(at,name)|json!({"output_frame":at,"action":name,
                "window_plus_minus10ms":levels(&output[at.saturating_sub(sr as usize/100)*2..(at+sr as usize/100).min(output.len()/2)*2])})).collect::<Vec<_>>(),
            "scope":"production renderer and actual seek/toggle/touch/jog commands; adjacent-step values are diagnostics, not click audibility scores"}));
        }
    }
    records
}
fn callback_cases(root: &Path, source: &Arc<Sample>) -> Vec<Value> {
    let mut cases = Vec::new();
    for sr in RATES {
        for block in BLOCKS {
            for ratio in RATIOS {
                for locked in [false, true] {
                    let (_port, mut rt) = renderer(sr);
                    prepare(&mut rt, source.clone(), ratio, locked);
                    // Keep source well away from EOF throughout preparation and measurement.
                    rt.decks[0].loop_on = true;
                    rt.decks[0].loop_start = 0.0;
                    rt.decks[0].loop_len = source.frames() as f64;
                    let mut callback = audio::OutputCallback::new(rt, 2);
                    let mut warm = vec![0.0f32; block * 2];
                    // Let ordinary worker startup settle outside the measured window.
                    std::thread::sleep(std::time::Duration::from_millis(20));
                    for _ in 0..16 {
                        callback.render(&mut warm);
                    }
                    let mut output = vec![0.0f32; sr as usize * 2];
                    let (timings, heap) =
                        measure_blocks(|chunk| callback.render(chunk), &mut output, block);
                    let id = format!(
                        "callback_{sr}_{block}_{ratio:.2}_{}",
                        if locked { "locked" } else { "unlocked" }
                    );
                    cases.push(json!({"id":id,"ratio":ratio,"output_sr":sr,"block_frames":block,"locked":locked,
            "measurement":costs(root,&id,sr,&timings,heap),"levels":levels(&output),
            "path":"actual OutputCallback<f32>, stereo, one deck, neutral master; warm allocations excluded and disclosed"}));
                }
            }
        }
    }
    cases
}

#[test]
#[ignore = "explicit bounded offline quality export; requires fresh OMATAINER_KEYLOCK_QUALITY_OUT under omatainer-work"]
fn export_keylock_quality() {
    assert!(
        !cfg!(debug_assertions),
        "build the test executable with cargo test --release --no-run"
    );
    let root = PathBuf::from(
        std::env::var_os("OMATAINER_KEYLOCK_QUALITY_OUT")
            .expect("set explicit new evidence directory"),
    );
    assert!(
        root.is_absolute() && root.starts_with(ROOT) && !root.exists(),
        "evidence must be a fresh directory under {ROOT}"
    );
    let parent = root.parent().unwrap().canonicalize().unwrap();
    assert!(parent.starts_with(Path::new(ROOT).canonicalize().unwrap()));
    fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
    write_new(&root.join("INCOMPLETE"),b"No success is implied until report.json exists. All metrics are objective diagnostics; human scores absent.\n");
    let corpus = corpus();
    let estimate: u64 = corpus
        .iter()
        .map(|c| {
            RATIOS
                .iter()
                .map(|r| {
                    RATES
                        .iter()
                        .map(|sr| {
                            ((c.audio.frames() as f64 / c.audio.sr as f64 / *r as f64 * *sr as f64)
                                .ceil() as u64)
                                * 8
                                * 2
                        })
                        .sum::<u64>()
                })
                .sum::<u64>()
        })
        .sum();
    let timing_bound: u64 = corpus
        .iter()
        .map(|c| {
            RATIOS
                .iter()
                .map(|r| {
                    RATES
                        .iter()
                        .map(|sr| {
                            BLOCKS
                                .iter()
                                .map(|block| {
                                    ((c.audio.frames() as f64 / c.audio.sr as f64 / *r as f64
                                        * *sr as f64)
                                        .ceil() as u64)
                                        .div_ceil(*block as u64)
                                        * 100
                                        * 2
                                })
                                .sum::<u64>()
                        })
                        .sum::<u64>()
                })
                .sum::<u64>()
        })
        .sum();
    assert!(
        estimate + timing_bound + 64 * 1024 * 1024 < OUTPUT_LIMIT,
        "bounded WAV/timing export exceeds2GiB limit"
    );
    let sources:Vec<_>=corpus.iter().map(|c|json!({"id":c.id,"kind":c.kind,"source_sr":c.audio.sr,"channels":c.audio.ch,"frames":c.audio.frames(),"pcm_sha256":pcm_digest(&c.audio.data),"original_sha256":c.original_sha})).collect();
    let workload = json!({"schema":1,"ratios":RATIOS,"output_rates":RATES,"blocks":BLOCKS,"sources":sources,
        "canonical_block":128,"setup":"off-timing production apply_plain load/reset, empty history, zero transition origin; no admission benchmarking","initial_rate":"fixed target; no fader settling","load_envelope":"production2ms load transition retained",
        "source_lengths":"whole source / ratio; no loop for quality WAVs","callback":"one second after20ms worker settling and16warm blocks; source loop enabled",
        "diagnostics":"v1; source-time bass windows, transient train, central reference correlation, transition sequence; no human rating"});
    let workload_sha = digest(&serde_json::to_vec(&workload).unwrap());
    let mut records = Vec::new();
    for sr in RATES {
        let (_port, mut rt) = renderer(sr);
        for source in &corpus {
            eprintln!("Rendering {} at {sr} Hz", source.id);
            for ratio in RATIOS {
                for locked in [false, true] {
                    let mut canonical: Option<Vec<f32>> = None;
                    for block in [128, 64, 512] {
                        prepare(&mut rt, source.audio.clone(), ratio, locked);
                        let frames =
                            (source.audio.frames() as f64 / source.audio.sr as f64 / ratio as f64
                                * sr as f64)
                                .ceil() as usize;
                        let mut output = vec![0.0f32; frames * 2];
                        let (timings, heap) = measure_blocks(
                            |chunk| {
                                for frame in chunk.chunks_exact_mut(2) {
                                    let (l, r) = rt.render_deck(0);
                                    frame.copy_from_slice(&[l, r]);
                                }
                            },
                            &mut output,
                            block,
                        );
                        let id = format!(
                            "{}_{sr}_{block}_{ratio:.2}_{}",
                            source.id,
                            if locked { "locked" } else { "unlocked" }
                        );
                        let file = (block == 128).then(|| format!("{id}.wav"));
                        if let Some(file) = &file {
                            wav(&root.join(file), sr, &output);
                        }
                        let mut record = json!({"id":id,"source":source.id,"output_sr":sr,"block_frames":block,"ratio":ratio,"locked":locked,
                    "frames":frames,"wav":file,"wav_sha256":file.as_ref().map(|name|digest(&fs::read(root.join(name)).unwrap())),
                    "pcm_sha256":pcm_digest(&output),"levels":levels(&output),"measurement":costs(&root,&id,sr,&timings,heap),
                    "path":"production render_deck; excludes session tracks/master/OutputCallback conversion"});
                        if let Some(reference) = &canonical {
                            record["block_invariance"] = difference(&output, reference);
                        }
                        if block == 128 {
                            if !locked || ratio == 1.0 {
                                let reference = source_reference(&source.audio, frames, sr, ratio);
                                let skip = sr as usize / 20 * 2;
                                record["direct_source_reference_after50ms"] =
                                    difference(&output[skip..], &reference[skip..]);
                                if ratio == 1.0 {
                                    record["unity_content_alignment"] =
                                        alignment(&output, &reference, sr);
                                }
                            }
                            if source.id == "bass" {
                                record["bass"] = bass_metrics(&output, sr, ratio, locked);
                            }
                            if source.id == "transients" {
                                record["transients"] = transient_metrics(&output, sr, ratio);
                            }
                            canonical = Some(output);
                        }
                        records.push(record);
                    }
                }
            }
        }
    }
    let callbacks = callback_cases(
        &root,
        &corpus.iter().find(|c| c.id == "full_mix").unwrap().audio,
    );
    let transitions = transition_cases(
        &root,
        &corpus.iter().find(|c| c.id == "full_mix").unwrap().audio,
    );
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/keylock");
    for file in ["README.md", "sources.json", "VocalSet-CC-BY-4.0.txt"] {
        write_new(&root.join(file), &fs::read(fixtures.join(file)).unwrap());
    }
    let git = |args: &[&str]| {
        std::process::Command::new("git")
            .args(args)
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .output()
            .unwrap()
            .stdout
    };
    let report = json!({"schema":1,"workload":workload,"workload_sha256":workload_sha,"git_commit":String::from_utf8_lossy(&git(&["rev-parse","HEAD"])).trim(),
        "build":{"debug_assertions":cfg!(debug_assertions),"rustc":String::from_utf8_lossy(&std::process::Command::new("rustc").arg("--version").output().unwrap().stdout).trim(),"executable_sha256":digest(&fs::read(std::env::current_exe().unwrap()).unwrap())},
        "engine_worktree_diff_sha256":digest(&git(&["diff","--binary","HEAD","--","src/engine"])),"records":records,"callbacks":callbacks,"transitions":transitions,
        "human_listening_scores":Value::Null,"device_latency":Value::Null,"backend_xruns":Value::Null,
        "limitations":["Offline controlled corpus, not physical device qualification","No automatic metric establishes perceptual transparency","CPU is current-thread CPU; wall includes scheduling","Rust allocator instrumentation does not measure foreign allocation","No effect or plugin load stress; final release gate is separate"]});
    let report = serde_json::to_vec_pretty(&report).unwrap();
    write_new(&root.join("report.json"), &report);
    fs::remove_file(root.join("INCOMPLETE")).unwrap();
    eprintln!(
        "Key-lock objective report: {} (workload {})",
        root.display(),
        workload_sha
    );
}

#[test]
fn objective_helpers_distinguish_pitch_energy_and_transient_stereo_delay() {
    let sr = 4000;
    let data: Vec<_> = (0..sr * 6)
        .flat_map(|i| {
            let t = i as f64 / sr as f64;
            let f = if t < 3.0 { 55.0 } else { 93.75 };
            let v = (std::f64::consts::TAU * f * t).sin() as f32 * 0.35;
            [v, v * 0.75]
        })
        .collect();
    let measured = bass_metrics(&data, sr, 1.0, true);
    for section in measured["sections"].as_array().unwrap() {
        let ch = &section["channels"][0];
        assert!(
            (ch["spectral_peak_hz"].as_f64().unwrap() - ch["expected_hz"].as_f64().unwrap()).abs()
                <= 0.051
        );
        assert!((ch["fundamental_amplitude"].as_f64().unwrap() - 0.35).abs() < 0.002);
    }
    assert_eq!(difference(&data, &data)["peak_absolute_difference"], 0.0);
    let silent = bass_metrics(&vec![0.0; data.len()], sr, 1.0, true);
    assert!(silent["sections"][0]["channels"][0]["spectral_peak_hz"].is_null());
    let mut pulses = vec![0.0; sr as usize * 6 * 2];
    for pulse in 1..11 {
        for channel in 0..2 {
            pulses[(pulse * sr as usize / 2 + channel * 6) * 2 + channel] = 0.5;
        }
    }
    let transients = transient_metrics(&pulses, sr, 1.0);
    for pulse in transients["pulses"].as_array().unwrap() {
        for channel in pulse["channels"].as_array().unwrap() {
            assert!(
                channel["source_onset_displacement_ms"]
                    .as_f64()
                    .unwrap()
                    .abs()
                    < 1e-8
            );
            assert_eq!(channel["output_95percent_energy_width_ms"], 0.0);
        }
        let left = pulse["channels"][0]["source_energy_centroid_seconds"]
            .as_f64()
            .unwrap();
        let right = pulse["channels"][1]["source_energy_centroid_seconds"]
            .as_f64()
            .unwrap();
        assert!(((right - left) * 1000.0 - 1.5).abs() < 1e-8);
    }
}

#[test]
fn production_setup_repeats_exactly_across_block_sizes_without_render_heap() {
    let audio = sample(
        "setup-regression",
        48_000,
        (0..48_000)
            .flat_map(|i| {
                let value = (std::f32::consts::TAU * 93.75 * i as f32 / 48_000.0).sin() * 0.35;
                [value, value * 0.75]
            })
            .collect(),
    );
    for sr in RATES {
        let (_port, mut rt) = renderer(sr);
        for ratio in RATIOS {
            for locked in [false, true] {
                let mut reference = None;
                for block in [128, 64, 512] {
                    prepare(&mut rt, audio.clone(), ratio, locked);
                    assert!((rt.decks[0].pitch_rate() - ratio).abs() < 1e-7);
                    let mut output = vec![0.0; 2048 * 2];
                    let (_, heap) = measure_blocks(
                        |chunk| {
                            for frame in chunk.chunks_exact_mut(2) {
                                let (l, r) = rt.render_deck(0);
                                frame.copy_from_slice(&[l, r]);
                            }
                        },
                        &mut output,
                        block,
                    );
                    assert_eq!(heap, test_alloc::Counts::default(), "{sr}/{ratio}/{locked}");
                    assert!(output.iter().all(|sample| sample.is_finite()));
                    if let Some(reference) = &reference {
                        assert_eq!(&output, reference, "{sr}/{ratio}/{locked}/{block}");
                    } else {
                        reference = Some(output);
                    }
                }
            }
        }
    }
}
