//! Immutable, bounded source lanes and the session's musical conductor.
use crate::midi_file::{Message, Meta, MetaValue};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

pub(crate) const MAX_LANE_BYTES: usize = 16 * 1024 * 1024;
pub(crate) const MAX_CONDUCTOR_POINTS: usize = 4096;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Lanes {
    pub ppqn: u16,
    pub end_tick: u64,
    pub messages: Vec<Message>,
    pub meta: Vec<Meta>,
    #[serde(skip)]
    cached_bytes: usize,
}
impl PartialEq for Lanes {
    fn eq(&self, other: &Self) -> bool {
        self.ppqn == other.ppqn
            && self.end_tick == other.end_tick
            && self.messages == other.messages
            && self.meta == other.meta
    }
}
impl Eq for Lanes {}
impl Lanes {
    pub fn new(
        ppqn: u16,
        end_tick: u64,
        messages: Vec<Message>,
        meta: Vec<Meta>,
    ) -> Result<Arc<Self>, String> {
        Self::new_with_cancel(ppqn, end_tick, messages, meta, &mut || false)
    }
    pub fn new_with_cancel(
        ppqn: u16,
        end_tick: u64,
        messages: Vec<Message>,
        meta: Vec<Meta>,
        cancel: &mut impl FnMut() -> bool,
    ) -> Result<Arc<Self>, String> {
        if cancel() {
            return Err("MIDI operation cancelled".into());
        }
        Self {
            ppqn,
            end_tick,
            messages,
            meta,
            cached_bytes: 0,
        }
        .prepare_with_cancel(cancel)
    }
    pub fn prepare(&self) -> Result<Arc<Self>, String> {
        self.prepare_with_cancel(&mut || false)
    }
    fn prepare_with_cancel(&self, cancel: &mut impl FnMut() -> bool) -> Result<Arc<Self>, String> {
        if cancel() {
            return Err("MIDI operation cancelled".into());
        }
        let mut result = self.clone();
        result.cached_bytes = 0;
        result.messages.shrink_to_fit();
        result.messages.sort_unstable_by_key(|m|(m.tick,m.order));
        result.meta.shrink_to_fit();
        for m in &mut result.meta {
            if let MetaValue::Text { bytes, .. } = &mut m.value {
                bytes.shrink_to_fit();
            }
        }
        result.validate_with_cancel(cancel)?;
        result.cached_bytes = result.bytes();
        Ok(Arc::new(result))
    }
    pub fn bytes(&self) -> usize {
        if self.cached_bytes != 0 {
            return self.cached_bytes;
        }
        std::mem::size_of::<Self>()
            + self.messages.capacity() * std::mem::size_of::<Message>()
            + self.meta.capacity() * std::mem::size_of::<Meta>()
            + self
                .meta
                .iter()
                .map(|m| match &m.value {
                    MetaValue::Text { bytes, .. } => bytes.capacity(),
                    _ => 0,
                })
                .sum::<usize>()
    }
    pub fn validate(&self) -> Result<(), String> {
        self.validate_with_cancel(&mut || false)
    }
    fn validate_with_cancel(&self, cancel: &mut impl FnMut() -> bool) -> Result<(), String> {
        if self.ppqn == 0
            || self.ppqn > 32767
            || self.end_tick > u64::from(self.ppqn) * 262144
            || self.messages.len() + self.meta.len() > crate::midi_file::MAX_EVENTS
            || self.bytes() > MAX_LANE_BYTES
        {
            return Err("MIDI source lanes exceed PPQN, time, event or 16 MiB limits".into());
        }
        // The same writer validates all supported statuses/meta values and
        // ordering. This runs on preparation workers, never in the callback.
        crate::midi_file::encode_with_cancel(
            &crate::midi_file::File {
                format: crate::midi_file::Format::Single,
                ppqn: self.ppqn,
                tracks: vec![crate::midi_file::Track {
                    end_tick: self.end_tick,
                    notes: vec![],
                    messages: self.messages.clone(),
                    meta: self.meta.clone(),
                }],
                warnings: vec![],
            },
            false,
            cancel,
        )
        .map(|_| ())
        .map_err(|e| e.to_string())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Tempo {
    pub tick: u64,
    pub micros: u32,
    #[serde(skip)]
    seconds: f64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Meter {
    pub tick: u64,
    pub numerator: u8,
    pub denominator_power: u8,
    pub clocks: u8,
    pub thirty_seconds: u8,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Conductor {
    pub ppqn: u16,
    pub tempos: Vec<Tempo>,
    pub meters: Vec<Meter>,
}
impl PartialEq for Conductor {
    fn eq(&self, other: &Self) -> bool {
        self.ppqn == other.ppqn
            && self.meters == other.meters
            && self
                .tempos
                .iter()
                .map(|p| (p.tick, p.micros))
                .eq(other.tempos.iter().map(|p| (p.tick, p.micros)))
    }
}
impl Conductor {
    pub fn from_meta(ppqn: u16, meta: impl Iterator<Item = Meta>) -> Result<Arc<Self>, String> {
        let mut tempos = vec![Tempo {
            tick: 0,
            micros: 500000,
            seconds: 0.0,
        }];
        let mut meters = vec![Meter {
            tick: 0,
            numerator: 4,
            denominator_power: 2,
            clocks: 24,
            thirty_seconds: 8,
        }];
        let mut events: Vec<_> = meta.collect();
        events.sort_by_key(|m| (m.tick, m.order));
        let mut tempo_ticks = std::collections::BTreeMap::new();
        let mut meter_ticks = std::collections::BTreeMap::new();
        for m in events {
            match m.value {
                MetaValue::Tempo(micros) => {
                    if tempo_ticks
                        .insert(m.tick, micros)
                        .is_some_and(|v| v != micros)
                    {
                        return Err("Conflicting tempos at one tick: choose an authoritative file track or keep the session tempo".into());
                    }
                }
                MetaValue::Meter {
                    numerator,
                    denominator_power,
                    clocks,
                    thirty_seconds,
                } => {
                    let value = Meter {
                        tick: m.tick,
                        numerator,
                        denominator_power,
                        clocks,
                        thirty_seconds,
                    };
                    if meter_ticks
                        .insert(m.tick, value)
                        .is_some_and(|v| v != value)
                    {
                        return Err("Conflicting meters at one tick: choose an authoritative file track or keep the session conductor".into());
                    }
                }
                _ => {}
            }
        }
        for (tick, micros) in tempo_ticks {
            if tick == 0 {
                tempos.clear();
            }
            tempos.push(Tempo {
                tick,
                micros,
                seconds: 0.0,
            });
        }
        for (tick, meter) in meter_ticks {
            if tick == 0 {
                meters.clear();
            }
            meters.push(meter);
        }
        Self {
            ppqn,
            tempos,
            meters,
        }
        .prepare()
    }
    pub fn bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + self.tempos.capacity() * std::mem::size_of::<Tempo>()
            + self.meters.capacity() * std::mem::size_of::<Meter>()
    }
    pub fn validate(&self) -> Result<(), String> {
        let max = u64::from(self.ppqn) * 262144;
        if self.ppqn == 0
            || self.ppqn > 32767
            || self.tempos.is_empty()
            || self.meters.is_empty()
            || self.tempos[0].tick != 0
            || self.meters[0].tick != 0
            || self.tempos.len() > MAX_CONDUCTOR_POINTS
            || self.meters.len() > MAX_CONDUCTOR_POINTS
            || self
                .tempos
                .iter()
                .any(|p| p.tick > max || !(250000..=1500000).contains(&p.micros))
            || self
                .meters
                .iter()
                .any(|p| p.tick > max || p.numerator == 0 || p.denominator_power > 7)
            || self.tempos.windows(2).any(|w| w[0].tick >= w[1].tick)
            || self.meters.windows(2).any(|w| w[0].tick >= w[1].tick)
        {
            return Err("Session conductor requires PPQN timing, ordered points, tempos 40–240 BPM and meter denominators through 128; keep session tempo to retain other source values for export".into());
        }
        Ok(())
    }
    /// Rebuild derived seconds on the worker after project deserialization.
    pub fn prepare(&self) -> Result<Arc<Self>, String> {
        self.validate()?;
        let mut result = self.clone();
        result.tempos[0].seconds = 0.0;
        for i in 1..result.tempos.len() {
            let previous = &result.tempos[i - 1];
            result.tempos[i].seconds = previous.seconds
                + (result.tempos[i].tick - previous.tick) as f64 / f64::from(self.ppqn)
                    * f64::from(previous.micros)
                    / 1000000.0;
        }
        result.tempos.shrink_to_fit();
        result.meters.shrink_to_fit();
        Ok(Arc::new(result))
    }
    pub fn micros_at(&self, beat: f64) -> u32 {
        let tick = beat * f64::from(self.ppqn);
        let i = self
            .tempos
            .partition_point(|p| p.tick as f64 <= tick)
            .saturating_sub(1);
        self.tempos[i].micros
    }
    pub fn seconds_at(&self, beat: f64) -> f64 {
        let tick = beat * f64::from(self.ppqn);
        let i = self
            .tempos
            .partition_point(|p| p.tick as f64 <= tick)
            .saturating_sub(1);
        let p = &self.tempos[i];
        p.seconds + (beat - p.tick as f64 / f64::from(self.ppqn)) * f64::from(p.micros) / 1000000.0
    }
    pub fn beat_at_seconds(&self, seconds: f64) -> f64 {
        let i = self
            .tempos
            .partition_point(|p| p.seconds <= seconds)
            .saturating_sub(1);
        let p = &self.tempos[i];
        p.tick as f64 / f64::from(self.ppqn)
            + (seconds - p.seconds) * 1000000.0 / f64::from(p.micros)
    }
    pub fn click_between(&self, start: f64, end: f64) -> Option<bool> {
        use super::super::midi_schedule::BEAT_EPSILON;
        if end <= start {
            return None;
        }
        let index = self
            .meters
            .partition_point(|m| m.tick as f64 / f64::from(self.ppqn) <= start)
            .saturating_sub(1);
        let meter = self.meters[index];
        let origin = meter.tick as f64 / f64::from(self.ppqn);
        let unit = 4.0 / f64::from(1u32 << meter.denominator_power);
        let number = ((start - origin - BEAT_EPSILON) / unit).ceil();
        let boundary = origin + number * unit;
        let next = self
            .meters
            .get(index + 1)
            .map(|m| m.tick as f64 / f64::from(self.ppqn));
        if next.is_some_and(|change| change <= boundary && change < end - BEAT_EPSILON) {
            return Some(true);
        }
        if boundary < end - BEAT_EPSILON {
            Some(number.rem_euclid(f64::from(meter.numerator)) == 0.0)
        } else if next.is_some_and(|change| change < end - BEAT_EPSILON) {
            Some(true)
        } else {
            None
        }
    }
    pub fn position(&self, beat: f64) -> (u32, f32, Meter) {
        let mut bar = 1u32;
        let mut start = 0.0;
        let mut meter = self.meters[0];
        for next in &self.meters[1..] {
            let boundary = next.tick as f64 / f64::from(self.ppqn);
            if boundary > beat {
                break;
            }
            let length =
                f64::from(meter.numerator) * 4.0 / f64::from(1u32 << meter.denominator_power);
            bar = bar.saturating_add(((boundary - start) / length).ceil() as u32);
            start = boundary;
            meter = *next;
        }
        let length = f64::from(meter.numerator) * 4.0 / f64::from(1u32 << meter.denominator_power);
        (
            bar.saturating_add(((beat - start) / length).floor() as u32),
            ((beat - start).rem_euclid(length) * f64::from(1u32 << meter.denominator_power) / 4.0)
                as f32,
            meter,
        )
    }
    pub fn meta(&self) -> Vec<Meta> {
        let tempos = self.tempos.iter().enumerate().map(|(i, p)| Meta {
            tick: p.tick,
            order: i as u32,
            value: MetaValue::Tempo(p.micros),
        });
        let meters = self.meters.iter().enumerate().map(|(i, p)| Meta {
            tick: p.tick,
            order: (self.tempos.len() + i) as u32,
            value: MetaValue::Meter {
                numerator: p.numerator,
                denominator_power: p.denominator_power,
                clocks: p.clocks,
                thirty_seconds: p.thirty_seconds,
            },
        });
        tempos.chain(meters).collect()
    }
}
