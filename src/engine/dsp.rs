//! Real-time-safe DSP primitives used by the mixer, decks, and instruments.

#[derive(Clone, Copy, Debug)]
pub struct OnePole {
    pub a: f32,
    pub z: f32,
}

impl OnePole {
    pub fn lpf(sr: f32, hz: f32) -> Self {
        let a = (-2.0 * std::f32::consts::PI * hz / sr).exp();
        Self { a, z: 0.0 }
    }
    pub fn tick(&mut self, x: f32) -> f32 {
        self.z = x + self.a * (self.z - x);
        self.z
    }
}

/// State-variable filter. `morph` 0 = LP, 0.5 = BP-ish, 1 = HP (Serato channel filter).
#[derive(Clone, Copy, Debug, Default)]
pub struct Svf {
    pub ic1eq: f32,
    pub ic2eq: f32,
}

impl Svf {
    pub fn process(&mut self, x: f32, cutoff: f32, res: f32, sr: f32, morph: f32) -> f32 {
        let f = (std::f32::consts::PI * cutoff / sr).clamp(0.0001, 0.45).tan();
        let g = f;
        let k = 2.0 - res.clamp(0.0, 0.95) * 1.8;
        let a1 = 1.0 / (1.0 + g * (g + k));
        let a2 = g * a1;
        let a3 = g * a2;
        let v3 = x - self.ic2eq;
        let v1 = a1 * self.ic1eq + a2 * v3;
        let v2 = self.ic2eq + a2 * self.ic1eq + a3 * v3;
        self.ic1eq = 2.0 * v1 - self.ic1eq;
        self.ic2eq = 2.0 * v2 - self.ic2eq;
        let lp = v2;
        let hp = x - k * v1 - lp;
        let bp = v1;
        if morph < 0.5 {
            let t = morph * 2.0;
            lp * (1.0 - t) + bp * t
        } else {
            let t = (morph - 0.5) * 2.0;
            bp * (1.0 - t) + hp * t
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ThreeBand {
    pub low: OnePole,
    pub high: OnePole,
    pub low_g: f32,
    pub mid_g: f32,
    pub high_g: f32,
}

impl ThreeBand {
    pub fn new(sr: f32) -> Self {
        Self {
            low: OnePole::lpf(sr, 250.0),
            high: OnePole::lpf(sr, 3200.0),
            low_g: 1.0,
            mid_g: 1.0,
            high_g: 1.0,
        }
    }
    pub fn tick(&mut self, x: f32) -> f32 {
        let l = self.low.tick(x);
        let h_lp = self.high.tick(x);
        let h = x - h_lp;
        let m = h_lp - l;
        l * self.low_g + m * self.mid_g + h * self.high_g
    }
}

#[derive(Clone, Debug)]
pub struct Delay {
    buf: Vec<f32>,
    w: usize,
    pub time_samples: f32,
    pub fb: f32,
    pub mix: f32,
}

impl Delay {
    pub fn new(max: usize) -> Self {
        Self {
            buf: vec![0.0; max.max(64)],
            w: 0,
            time_samples: 12000.0,
            fb: 0.35,
            mix: 0.0,
        }
    }
    pub fn tick(&mut self, x: f32) -> f32 {
        let n = self.buf.len() as f32;
        let t = self.time_samples.clamp(1.0, n - 2.0);
        let r = (self.w as f32 - t + n) % n;
        let i = r as usize;
        let f = r.fract();
        let a = self.buf[i];
        let b = self.buf[(i + 1) % self.buf.len()];
        let y = a + (b - a) * f;
        self.buf[self.w] = x + y * self.fb;
        self.w = (self.w + 1) % self.buf.len();
        x * (1.0 - self.mix) + y * self.mix
    }
}

#[derive(Clone, Debug)]
pub struct Reverb {
    delays: [Delay; 4],
    pub mix: f32,
}

impl Reverb {
    pub fn new() -> Self {
        Self::at_sample_rate(48_000.0)
    }

    /// Preserve the original 48 kHz comb durations at the output sample rate.
    /// Arrival times are rounded to the closest audio frame (at most half a
    /// frame of timing error); each instance owns one channel's history.
    pub fn at_sample_rate(sr: f32) -> Self {
        let scale = sr / 48_000.0;
        let capacities = [3011.0, 4057.0, 5059.0, 2333.0];
        let times = [1307.0, 1637.0, 1999.0, 887.0];
        let delays = std::array::from_fn(|i| {
            let mut delay = Delay::new((capacities[i] * scale).round() as usize);
            delay.time_samples = (times[i] * scale).round().max(1.0);
            delay.fb = 0.72;
            delay.mix = 1.0;
            delay
        });
        Self { delays, mix: 0.0 }
    }
    pub fn tick(&mut self, x: f32) -> f32 {
        let mut y = 0.0;
        for (i, d) in self.delays.iter_mut().enumerate() {
            let s = if i % 2 == 0 { x } else { x * -1.0 };
            y += d.tick(s);
        }
        x * (1.0 - self.mix) + (y * 0.25) * self.mix
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Env {
    pub level: f32,
    pub stage: u8, // 0 idle 1 att 2 dec 3 sus 4 rel
    pub att: f32,
    pub dec: f32,
    pub sus: f32,
    pub rel: f32,
}

impl Env {
    pub fn adsr(sr: f32, a: f32, d: f32, s: f32, r: f32) -> Self {
        Self {
            level: 0.0,
            stage: 0,
            att: (1.0 / (a * sr).max(1.0)),
            dec: (1.0 / (d * sr).max(1.0)),
            sus: s,
            rel: (1.0 / (r * sr).max(1.0)),
        }
    }
    pub fn on(&mut self) {
        self.stage = 1;
    }
    pub fn off(&mut self) {
        self.stage = 4;
    }
    pub fn tick(&mut self) -> f32 {
        match self.stage {
            1 => {
                self.level += self.att;
                if self.level >= 1.0 {
                    self.level = 1.0;
                    self.stage = 2;
                }
            }
            2 => {
                self.level -= self.dec * (1.0 - self.sus);
                if self.level <= self.sus {
                    self.level = self.sus;
                    self.stage = 3;
                }
            }
            3 => {}
            4 => {
                self.level -= self.rel;
                if self.level <= 0.0 {
                    self.level = 0.0;
                    self.stage = 0;
                }
            }
            _ => self.level = 0.0,
        }
        self.level.max(0.0)
    }
    pub fn active(&self) -> bool {
        self.stage != 0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VoiceOwner {
    Live,
    Clip,
}

/// The physical gate that owns a live voice. Pitch can change while a gate is
/// held; this key remains stable until its matching release arrives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputKey {
    Midi { source: u64, ch: u8, note: u8 },
    Pad(u8),
}

#[derive(Clone, Copy, Debug)]
pub struct Voice {
    pub note: u8,
    pub owner: VoiceOwner,
    pub input: Option<InputKey>,
    pub vel: f32,
    pub phase: f32,
    pub phase2: f32,
    pub env: Env,
    pub cutoff: f32,
    pub kind: u8, // 0 analog bass, 1 keys, 2 pad
}

impl Voice {
    pub fn new(sr: f32, kind: u8) -> Self {
        let env = match kind {
            0 => Env::adsr(sr, 0.005, 0.18, 0.35, 0.12),
            1 => Env::adsr(sr, 0.008, 0.22, 0.45, 0.28),
            _ => Env::adsr(sr, 0.04, 0.4, 0.7, 0.8),
        };
        Self {
            note: 0,
            owner: VoiceOwner::Live,
            input: None,
            vel: 0.0,
            phase: 0.0,
            phase2: 0.0,
            env,
            cutoff: 1200.0,
            kind,
        }
    }
    pub fn trig(&mut self, note: u8, vel: f32) {
        self.note = note;
        self.vel = vel;
        self.env.on();
    }
    pub fn tick(&mut self, sr: f32, cutoff: f32, res_svf: &mut Svf) -> f32 {
        if !self.env.active() {
            return 0.0;
        }
        let hz = 440.0 * 2f32.powf((self.note as f32 - 69.0) / 12.0);
        let inc = hz / sr;
        self.phase = (self.phase + inc) % 1.0;
        self.phase2 = (self.phase2 + inc * 0.997) % 1.0;
        let saw = self.phase * 2.0 - 1.0;
        let sq = if self.phase < 0.5 { 0.7 } else { -0.7 };
        let sine = (self.phase * std::f32::consts::TAU).sin();
        let osc = match self.kind {
            0 => saw * 0.7 + sq * 0.3,
            1 => saw * 0.35 + sine * 0.65,
            _ => sine * 0.6 + (self.phase2 * 2.0 - 1.0) * 0.4,
        };
        let e = self.env.tick();
        let cf = (cutoff + e * 1800.0).clamp(80.0, sr * 0.42);
        let y = res_svf.process(osc * e * self.vel, cf, 0.35, sr, 0.0);
        y * 0.35
    }
}

#[derive(Clone, Debug)]
pub struct Poly {
    pub voices: Vec<Voice>,
    pub filters: Vec<Svf>,
    pub kind: u8,
    pub cutoff: f32,
    #[cfg(test)]
    pub note_on_events: u64,
}

impl Poly {
    pub fn new(sr: f32, kind: u8, n: usize) -> Self {
        Self {
            voices: (0..n).map(|_| Voice::new(sr, kind)).collect(),
            filters: vec![Svf::default(); n],
            kind,
            cutoff: if kind == 0 { 700.0 } else { 1800.0 },
            #[cfg(test)]
            note_on_events: 0,
        }
    }
    pub fn note_on(&mut self, note: u8, vel: f32) {
        self.note_on_owned(note, vel, VoiceOwner::Live, None);
    }
    pub fn note_on_clip(&mut self, note: u8, vel: f32) {
        self.note_on_owned(note, vel, VoiceOwner::Clip, None);
    }
    pub fn note_on_input(&mut self, note: u8, vel: f32, input: InputKey) {
        self.note_on_owned(note, vel, VoiceOwner::Live, Some(input));
    }
    fn note_on_owned(&mut self, note: u8, vel: f32, owner: VoiceOwner, input: Option<InputKey>) {
        #[cfg(test)]
        { self.note_on_events += 1; }
        // Prefer a gate's existing voice over an earlier free slot. A repeat
        // note-on from that gate retriggers it rather than leaving duplicates.
        let available = self
            .voices
            .iter()
            .position(|v| v.owner == owner && v.input == input
                && (input.is_some() || v.note == note))
            .or_else(|| self.voices.iter().position(|v| !v.env.active()));
        if let Some(i) = available {
            let v = &mut self.voices[i];
            v.owner = owner;
            v.input = input;
            v.trig(note, vel);
            return;
        }
        let i = self
            .voices
            .iter()
            .enumerate()
            .min_by(|a, b| a.1.env.level.partial_cmp(&b.1.env.level).unwrap())
            .map(|(i, _)| i)
            .unwrap_or(0);
        self.voices[i].owner = owner;
        self.voices[i].input = input;
        self.voices[i].trig(note, vel);
    }
    pub fn note_off(&mut self, note: u8) {
        self.note_off_owned(note, VoiceOwner::Live);
    }
    pub fn note_off_clip(&mut self, note: u8) {
        self.note_off_owned(note, VoiceOwner::Clip);
    }
    fn note_off_owned(&mut self, note: u8, owner: VoiceOwner) {
        for v in &mut self.voices {
            if v.note == note && v.owner == owner && v.input.is_none() && matches!(v.env.stage, 1..=3) {
                v.env.off();
            }
        }
    }
    pub fn note_off_input(&mut self, input: InputKey) {
        for v in &mut self.voices {
            if v.input == Some(input) && matches!(v.env.stage, 1..=3) {
                v.env.off();
            }
        }
    }
    pub fn transpose_input(&mut self, input: InputKey, semitones: i8) {
        for v in &mut self.voices {
            if v.input == Some(input) && v.env.active() {
                v.note = (v.note as i16 + semitones as i16).clamp(0, 127) as u8;
            }
        }
    }
    pub fn release_clip(&mut self) {
        for v in &mut self.voices {
            if v.owner == VoiceOwner::Clip && matches!(v.env.stage, 1..=3) {
                v.env.off();
            }
        }
    }
    pub fn tick(&mut self, sr: f32) -> f32 {
        let mut s = 0.0;
        let cut = self.cutoff;
        for (v, f) in self.voices.iter_mut().zip(self.filters.iter_mut()) {
            s += v.tick(sr, cut, f);
        }
        s
    }
}

#[derive(Clone, Debug)]
pub struct Sample {
    pub name: String,
    pub sr: u32,
    pub ch: u16,
    pub data: Vec<f32>,
    pub peaks: Vec<[f32; 3]>,
    pub bpm: f32,
    pub path: String,
}

impl Sample {
    pub fn frames(&self) -> usize {
        if self.ch == 0 {
            0
        } else {
            self.data.len() / self.ch as usize
        }
    }
    pub fn at(&self, pos: f64) -> (f32, f32) {
        if self.data.is_empty() {
            return (0.0, 0.0);
        }
        let frames = self.frames() as f64;
        if pos < 0.0 || pos >= frames - 1.0 {
            return (0.0, 0.0);
        }
        let i = pos.floor() as usize;
        let f = (pos - i as f64) as f32;
        let ch = self.ch as usize;
        if ch >= 2 {
            let l0 = self.data[i * ch];
            let r0 = self.data[i * ch + 1];
            let l1 = self.data[(i + 1) * ch];
            let r1 = self.data[(i + 1) * ch + 1];
            (l0 + (l1 - l0) * f, r0 + (r1 - r0) * f)
        } else {
            let a = self.data[i];
            let b = self.data[i + 1];
            let y = a + (b - a) * f;
            (y, y)
        }
    }
}

pub fn db(g: f32) -> f32 {
    if g <= 0.0001 {
        0.0
    } else {
        10f32.powf(g / 20.0)
    }
}

pub fn xfader_gains(x: f32, curve: f32) -> (f32, f32) {
    // x 0 = full A, 1 = full B. curve 0 = linear, 1 = fast cut (Serato).
    let x = x.clamp(0.0, 1.0);
    let pow = 1.0 + curve * 2.5;
    let a = (1.0 - x).powf(pow);
    let b = x.powf(pow);
    (a, b)
}

pub fn cubic(p: f32) -> f32 {
    p * p * (3.0 - 2.0 * p)
}

pub fn limiter(x: f32) -> f32 {
    x.tanh()
}

pub fn midi_to_hz(n: u8) -> f32 {
    440.0 * 2f32.powf((n as f32 - 69.0) / 12.0)
}

pub fn resample_mono(src: &[f32], ratio: f32) -> Vec<f32> {
    if src.is_empty() {
        return Vec::new();
    }
    let ratio = ratio.clamp(0.25, 4.0);
    let n = ((src.len() as f32) / ratio).max(1.0) as usize;
    let mut o = vec![0.0f32; n];
    for (i, s) in o.iter_mut().enumerate() {
        let x = i as f32 * ratio;
        let j = x as usize;
        let f = x.fract();
        let a = src[j.min(src.len() - 1)];
        let b = src[(j + 1).min(src.len() - 1)];
        *s = a + (b - a) * f;
    }
    o
}

/// Render a one-shot drum hit into a mono buffer at `sr`.
pub fn synth_drum(kind: u8, sr: u32) -> Vec<f32> {
    let n = (sr as f32 * 0.55) as usize;
    let mut o = vec![0.0; n];
    match kind {
        0 => {
            // kick
            let mut ph = 0.0;
            for (i, s) in o.iter_mut().enumerate() {
                let t = i as f32 / sr as f32;
                let env = (-t * 8.5).exp();
                let hz = 148.0 * (-t * 18.0).exp() + 38.0;
                ph += hz / sr as f32;
                let click = if t < 0.003 { (1.0 - t / 0.003) * 0.4 } else { 0.0 };
                *s = (ph * std::f32::consts::TAU).sin() * env + click;
            }
        }
        1 => {
            // snare
            let mut seed = 1u32;
            let mut ph = 0.0;
            for (i, s) in o.iter_mut().enumerate() {
                seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                let nse = (seed as f32 / u32::MAX as f32) * 2.0 - 1.0;
                let t = i as f32 / sr as f32;
                let env = (-t * 14.0).exp();
                ph += 190.0 / sr as f32;
                *s = (ph * std::f32::consts::TAU).sin() * env * 0.35 + nse * env * 0.7;
            }
        }
        2 => {
            // closed hat
            let mut seed = 7u32;
            let mut lp = 0.0;
            for (i, s) in o.iter_mut().enumerate() {
                seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                let nse = (seed as f32 / u32::MAX as f32) * 2.0 - 1.0;
                let t = i as f32 / sr as f32;
                let env = (-t * 55.0).exp();
                lp = lp + 0.35 * (nse - lp);
                *s = (nse - lp) * env * 0.55;
            }
            o.truncate((sr as f32 * 0.09) as usize);
        }
        3 => {
            // clap
            let mut seed = 3u32;
            for (i, s) in o.iter_mut().enumerate() {
                seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                let nse = (seed as f32 / u32::MAX as f32) * 2.0 - 1.0;
                let t = i as f32 / sr as f32;
                let burst = [0.0, 0.012, 0.024, 0.04];
                let mut e: f32 = 0.0;
                for b in burst {
                    if t >= b {
                        e = e.max((-((t - b) * 70.0)).exp());
                    }
                }
                *s = nse * e * 0.7;
            }
            o.truncate((sr as f32 * 0.25) as usize);
        }
        4 => {
            // open hat
            let mut seed = 11u32;
            let mut lp = 0.0;
            for (i, s) in o.iter_mut().enumerate() {
                seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                let nse = (seed as f32 / u32::MAX as f32) * 2.0 - 1.0;
                let t = i as f32 / sr as f32;
                let env = (-t * 8.0).exp();
                lp = lp + 0.25 * (nse - lp);
                *s = (nse - lp) * env * 0.4;
            }
        }
        _ => {
            // tom
            let mut ph = 0.0;
            for (i, s) in o.iter_mut().enumerate() {
                let t = i as f32 / sr as f32;
                let env = (-t * 9.0).exp();
                let hz = 220.0 * (-t * 6.0).exp() + 80.0;
                ph += hz / sr as f32;
                *s = (ph * std::f32::consts::TAU).sin() * env * 0.8;
            }
        }
    }
    // fade tail
    let fade = (sr as f32 * 0.01) as usize;
    let n = o.len();
    for i in 0..fade.min(n) {
        let g = i as f32 / fade as f32;
        o[n - 1 - i] *= g;
    }
    o
}

pub fn peaks_3band(data: &[f32], ch: u16, buckets: usize) -> Vec<[f32; 3]> {
    if data.is_empty() || ch == 0 {
        return vec![[0.0; 3]; buckets.max(1)];
    }
    let frames = data.len() / ch as usize;
    let buckets = buckets.max(1);
    let mut out = vec![[0.0f32; 3]; buckets];
    let mut lp = 0.0f32;
    let mut bp = 0.0f32;
    for b in 0..buckets {
        let a = b * frames / buckets;
        let z = ((b + 1) * frames / buckets).min(frames);
        let mut l = 0.0;
        let mut m = 0.0;
        let mut h = 0.0;
        let mut c = 0.0;
        for i in a..z {
            let s = if ch >= 2 {
                0.5 * (data[i * ch as usize] + data[i * ch as usize + 1])
            } else {
                data[i]
            };
            lp += 0.08 * (s - lp);
            bp += 0.22 * (s - lp - bp);
            let hi = s - lp - bp;
            l += lp.abs();
            m += bp.abs();
            h += hi.abs();
            c += 1.0;
        }
        if c > 0.0 {
            out[b] = [(l / c).min(1.0), (m / c).min(1.0), (h / c).min(1.0)];
        }
    }
    out
}

pub fn detect_bpm(data: &[f32], ch: u16, sr: u32) -> f32 {
    if data.is_empty() || ch == 0 {
        return 120.0;
    }
    let hop = 512usize;
    let frames = data.len() / ch as usize;
    let mut env = Vec::with_capacity(frames / hop + 1);
    for i in (0..frames).step_by(hop) {
        let mut e = 0.0;
        let end = (i + hop).min(frames);
        for k in i..end {
            let s = if ch >= 2 {
                0.5 * (data[k * ch as usize] + data[k * ch as usize + 1])
            } else {
                data[k]
            };
            e += s * s;
        }
        env.push((e / hop as f32).sqrt());
    }
    if env.len() < 8 {
        return 120.0;
    }
    let mut flux = vec![0.0; env.len()];
    for i in 1..env.len() {
        flux[i] = (env[i] - env[i - 1]).max(0.0);
    }
    let hop_t = hop as f32 / sr as f32;
    let mut best_bpm = 120.0;
    let mut best = 0.0f32;
    let mut bpm = 70.0f32;
    while bpm <= 180.0 {
        let period = 60.0 / bpm / hop_t;
        if period < 2.0 {
            bpm += 0.5;
            continue;
        }
        let mut acc = 0.0;
        let mut n = 0.0;
        let mut t = period;
        while t < flux.len() as f32 - 1.0 {
            let i = t as usize;
            acc += flux[i];
            n += 1.0;
            t += period;
        }
        if n > 0.0 {
            let score = acc / n;
            if score > best {
                best = score;
                best_bpm = bpm;
            }
        }
        bpm += 0.5;
    }
    if best_bpm < 85.0 {
        best_bpm *= 2.0;
    }
    if best_bpm > 170.0 {
        best_bpm *= 0.5;
    }
    best_bpm
}

pub fn decode_audio(path: &std::path::Path) -> anyhow::Result<Sample> {
    use std::fs::File;
    use symphonia::core::audio::SampleBuffer;
    use symphonia::core::codecs::DecoderOptions;
    use symphonia::core::formats::FormatOptions;
    use symphonia::core::io::MediaSourceStream;
    use symphonia::core::meta::MetadataOptions;
    use symphonia::core::probe::Hint;

    let file = File::open(path)?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    let probed = symphonia::default::get_probe().format(
        &hint,
        mss,
        &FormatOptions::default(),
        &MetadataOptions::default(),
    )?;
    let mut format = probed.format;
    let track = format
        .default_track()
        .ok_or_else(|| anyhow::anyhow!("no audio track"))?;
    let id = track.id;
    let mut decoder =
        symphonia::default::get_codecs().make(&track.codec_params, &DecoderOptions::default())?;
    let sr = track.codec_params.sample_rate.unwrap_or(48000);
    let mut ch = track.codec_params.channels.map(|c| c.count()).unwrap_or(2) as u16;
    let mut data = Vec::new();
    let mut spec = None;
    loop {
        let pkt = match format.next_packet() {
            Ok(p) => p,
            Err(symphonia::core::errors::Error::IoError(e))
                if e.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
                break;
            }
            Err(symphonia::core::errors::Error::ResetRequired) => break,
            Err(_) => break,
        };
        if pkt.track_id() != id {
            continue;
        }
        match decoder.decode(&pkt) {
            Ok(buf) => {
                if spec.is_none() {
                    spec = Some(*buf.spec());
                    ch = buf.spec().channels.count() as u16;
                }
                let mut sb = SampleBuffer::<f32>::new(buf.capacity() as u64, *buf.spec());
                sb.copy_interleaved_ref(buf);
                data.extend_from_slice(sb.samples());
            }
            Err(_) => continue,
        }
    }
    if data.is_empty() {
        anyhow::bail!("empty decode");
    }
    let peaks = peaks_3band(&data, ch, 2048);
    let bpm = detect_bpm(&data, ch, sr);
    Ok(Sample {
        name: path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("track")
            .to_string(),
        sr,
        ch,
        data,
        peaks,
        bpm,
        path: path.to_string_lossy().into(),
    })
}
