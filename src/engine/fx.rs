//! Per-track / per-scene FX chain. Slots are stackable; order is the chain.

use crate::engine::dsp::{Delay, Reverb, Svf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FxId {
    Comp,
    Spread,
    Balance,
    Reverb,
    Chorus,
    Delay,
    Gate,
    Arp,
    Dist,
    Filter,
    Eq3,
    Eq5,
    Eq8,
}

impl FxId {
    pub fn all() -> &'static [FxId] {
        &[
            FxId::Comp,
            FxId::Spread,
            FxId::Balance,
            FxId::Reverb,
            FxId::Chorus,
            FxId::Delay,
            FxId::Gate,
            FxId::Arp,
            FxId::Dist,
            FxId::Filter,
            FxId::Eq3,
            FxId::Eq5,
            FxId::Eq8,
        ]
    }
    pub fn name(self) -> &'static str {
        match self {
            FxId::Comp => "comp",
            FxId::Spread => "spread",
            FxId::Balance => "balance",
            FxId::Reverb => "reverb",
            FxId::Chorus => "chorus",
            FxId::Delay => "delay",
            FxId::Gate => "gate",
            FxId::Arp => "arp",
            FxId::Dist => "drive",
            FxId::Filter => "filter",
            FxId::Eq3 => "eq3",
            FxId::Eq5 => "eq5",
            FxId::Eq8 => "eq8",
        }
    }
}

#[derive(Clone, Debug)]
pub struct FxSlot {
    pub id: FxId,
    pub on: bool,
    pub mix: f32,
    pub p: [f32; 4],
}

impl FxSlot {
    pub fn new(id: FxId) -> Self {
        let p = match id {
            FxId::Comp => [0.4, 0.35, 0.2, 0.0],
            FxId::Spread => [0.5, 0.0, 0.0, 0.0],
            FxId::Balance => [0.5, 0.0, 0.0, 0.0],
            FxId::Reverb => [0.35, 0.5, 0.0, 0.0],
            FxId::Chorus => [0.4, 0.3, 0.0, 0.0],
            FxId::Delay => [0.4, 0.35, 0.0, 0.0],
            FxId::Gate => [0.15, 0.4, 0.0, 0.0],
            FxId::Arp => [0.25, 0.5, 0.0, 0.0],
            FxId::Dist => [0.25, 0.0, 0.0, 0.0],
            FxId::Filter => [0.5, 0.3, 0.0, 0.0],
            FxId::Eq3 | FxId::Eq5 | FxId::Eq8 => [0.5, 0.5, 0.5, 0.5],
        };
        Self {
            id,
            on: true,
            mix: 0.5,
            p,
        }
    }
}

#[derive(Clone, Debug)]
pub struct FxChain {
    pub slots: Vec<FxSlot>,
    delay: Delay,
    reverb: Reverb,
    chorus: Delay,
    chorus_ph: f32,
    env: f32,
    svf: Svf,
    eq_lp: [crate::engine::dsp::OnePole; 8],
    haas: Delay,
}

impl FxChain {
    pub fn new(sr: f32) -> Self {
        let mut chorus = Delay::new((sr * 0.05) as usize);
        chorus.time_samples = sr * 0.012;
        chorus.fb = 0.0;
        chorus.mix = 1.0;
        Self {
            slots: Vec::new(),
            delay: Delay::new((sr * 2.0) as usize),
            reverb: Reverb::new(),
            chorus,
            chorus_ph: 0.0,
            env: 0.0,
            svf: Svf::default(),
            eq_lp: [
                crate::engine::dsp::OnePole::lpf(sr, 80.0),
                crate::engine::dsp::OnePole::lpf(sr, 160.0),
                crate::engine::dsp::OnePole::lpf(sr, 320.0),
                crate::engine::dsp::OnePole::lpf(sr, 640.0),
                crate::engine::dsp::OnePole::lpf(sr, 1280.0),
                crate::engine::dsp::OnePole::lpf(sr, 2560.0),
                crate::engine::dsp::OnePole::lpf(sr, 5120.0),
                crate::engine::dsp::OnePole::lpf(sr, 9000.0),
            ],
            haas: Delay::new((sr * 0.02) as usize),
        }
    }

    pub fn tick(&mut self, x: f32, sr: f32) -> f32 {
        let mut y = x;
        let slots = self.slots.clone();
        for s in &slots {
            if !s.on {
                continue;
            }
            let wet = match s.id {
                FxId::Comp => {
                    self.env = self.env * 0.995 + y.abs() * 0.005;
                    let thr = 0.05 + s.p[0] * 0.4;
                    let gr = if self.env > thr {
                        thr / self.env.max(1e-6)
                    } else {
                        1.0
                    };
                    y * (1.0 - s.p[1] + s.p[1] * gr)
                }
                FxId::Spread | FxId::Balance | FxId::Arp => y,
                FxId::Reverb => {
                    self.reverb.mix = s.mix;
                    self.reverb.tick(y)
                }
                FxId::Chorus => {
                    self.chorus_ph = (self.chorus_ph + 0.7 / sr) % 1.0;
                    self.chorus.time_samples = sr * (0.008 + 0.006 * (self.chorus_ph * std::f32::consts::TAU).sin());
                    self.chorus.mix = s.mix;
                    self.chorus.tick(y)
                }
                FxId::Delay => {
                    self.delay.fb = s.p[1];
                    self.delay.mix = s.mix;
                    self.delay.tick(y)
                }
                FxId::Gate => {
                    self.env = self.env * 0.98 + y.abs() * 0.02;
                    if self.env < s.p[0] {
                        y * 0.05
                    } else {
                        y
                    }
                }
                FxId::Dist => (y * (1.0 + s.p[0] * 8.0)).tanh(),
                FxId::Filter => {
                    let morph = s.p[0];
                    let cut = 120.0 + s.p[1] * 8000.0;
                    self.svf.process(y, cut, 0.35, sr, morph)
                }
                FxId::Eq3 => eq_n(&mut self.eq_lp, y, s.p, 3),
                FxId::Eq5 => eq_n(&mut self.eq_lp, y, s.p, 5),
                FxId::Eq8 => eq_n(&mut self.eq_lp, y, s.p, 8),
            };
            y = y * (1.0 - s.mix) + wet * s.mix;
        }
        y
    }

    pub fn tick_stereo(&mut self, x: f32, sr: f32) -> (f32, f32) {
        let y = self.tick(x, sr);
        let w = self.width();
        self.haas.time_samples = (w - 0.5).abs() * sr * 0.012;
        self.haas.mix = 1.0;
        self.haas.fb = 0.0;
        let delayed = self.haas.tick(y);
        let (mut l, mut r) = if w >= 0.5 {
            (y, delayed)
        } else {
            let m = (0.5 - w) * 2.0;
            (y * (1.0 - m) + delayed * m, y * (1.0 - m) + delayed * m)
        };
        let bal = self.balance();
        let pan = (bal * 2.0 - 1.0).clamp(-1.0, 1.0);
        l *= (1.0 - pan.max(0.0)).sqrt();
        r *= (1.0 + pan.min(0.0)).sqrt();
        (l, r)
    }

    pub fn width(&self) -> f32 {
        self.slots
            .iter()
            .find(|s| s.id == FxId::Spread && s.on)
            .map(|s| s.p[0])
            .unwrap_or(0.5)
    }
    pub fn balance(&self) -> f32 {
        self.slots
            .iter()
            .find(|s| s.id == FxId::Balance && s.on)
            .map(|s| s.p[0])
            .unwrap_or(0.5)
    }
}

fn eq_n(lp: &mut [crate::engine::dsp::OnePole; 8], x: f32, p: [f32; 4], n: usize) -> f32 {
    let n = n.clamp(3, 8);
    let mut prev = 0.0f32;
    let mut acc = 0.0;
    for i in 0..n {
        let c = lp[i.min(7)].tick(x);
        let band = if i + 1 == n { x - prev } else { c - prev };
        prev = c;
        let t = i as f32 / (n - 1) as f32;
        let g = if t < 0.5 {
            p[0] + (p[1] - p[0]) * (t * 2.0)
        } else {
            p[1] + (p[2] - p[1]) * ((t - 0.5) * 2.0)
        };
        acc += band * (0.25 + g * 1.5);
    }
    acc
}
