use super::*;
use crate::engine::midi_data::{Conductor, Meter, Tempo, TimingSettings, MAX_CONDUCTOR_POINTS};
use std::collections::BTreeMap;

const PPQN: u16 = 32767;

/// Decode Live's saved main-track time signature.
/// Takes the packed enum value; returns a numerator and denominator power, refusing unknown encodings.
pub(super) fn meter(raw: f64) -> Result<(u8, u8), String> {
    if !(0.0..=494.).contains(&raw) || raw.fract() != 0. {
        return Err(
            "Unknown Live time-signature enum; source retained without guessing a meter".into(),
        );
    }
    let value = raw as u32;
    Ok(((value % 99 + 1) as u8, (value / 99) as u8))
}

fn tick(beat: f64, source: &mut Source) -> Result<u64, String> {
    if !(0.0..=262144.).contains(&beat) {
        return Err("Live timing event exceeds the native song range".into());
    }
    let tick = (beat * f64::from(PPQN)).round() as u64;
    if (tick as f64 / f64::from(PPQN) - beat).abs() > 1e-10 {
        source.difference("master","Timing precision","Fractional timing events round to 32767 ticks per quarter note; original coordinates remain in the source archive")?;
    }
    Ok(tick)
}

/// Convert the authoritative tempo and meter targets.
/// Takes a saved Set and native draft; returns a validated conductor, retaining unsupported master envelopes in the review.
pub(super) fn convert(
    set: &Element,
    state: &mut project::State,
    source: &mut Source,
) -> Result<(), String> {
    let master = match (child(set, "MasterTrack")?, child(set, "MainTrack")?) {
        (Some(master), None) => master,
        (None, Some(main)) if source.format.starts_with("12.") => main,
        _ => return Err("Live Set needs exactly one supported main-track record".into()),
    };
    let bpm = number(master, &["DeviceChain", "Mixer", "Tempo", "Manual"], 120.)?;
    let initial = meter(number(
        master,
        &["DeviceChain", "Mixer", "TimeSignature", "Manual"],
        201.,
    )?)?;
    state.master = number(master, &["DeviceChain", "Mixer", "Volume", "Manual"], 1.)? as f32;
    let mut tempos = BTreeMap::from([(0, Tempo::new(0, bpm, false)?)]);
    let make_meter = |tick, u: (u8, u8)| Meter {
        tick,
        numerator: u.0,
        denominator_power: u.1,
        clocks: 24,
        thirty_seconds: 8,
    };
    let mut meters = BTreeMap::from([(0, make_meter(0, initial))]);
    let target = |name| -> Result<&str, String> {
        Ok(
            at(master, &["DeviceChain", "Mixer", name, "AutomationTarget"])?
                .map_or("", |n| n.attr("Id")),
        )
    };
    let tempo_target = target("Tempo")?;
    let meter_target = target("TimeSignature")?;
    let mut seen = BTreeSet::new();
    if let Some(envelopes) = at(master, &["AutomationEnvelopes", "Envelopes"])? {
        for envelope in &envelopes.children {
            if envelope.name != "AutomationEnvelope" {
                return Err("Unknown master envelope record".into());
            }
            let pointee = value(envelope, &["EnvelopeTarget", "PointeeId"])?;
            if pointee.is_empty() || !seen.insert(pointee) {
                return Err("Master automation needs unique target identities".into());
            }
            let tempo = !tempo_target.is_empty() && pointee == tempo_target;
            let signature = !meter_target.is_empty() && pointee == meter_target;
            if !tempo && !signature {
                source.difference(format!("master/target:{pointee}"),"Master automation","Original target and envelope retained; native processing target requires explicit review")?;
                continue;
            }
            let Some(events) = at(envelope, &["Automation", "Events"])? else {
                continue;
            };
            if events.children.len() > MAX_CONDUCTOR_POINTS {
                return Err("Live timing envelope exceeds 4096 events".into());
            }
            let mut previous = f64::NEG_INFINITY;
            let mut precision = false;
            let mut curves = false;
            for event in &events.children {
                if event.name != if tempo { "FloatEvent" } else { "EnumEvent" } {
                    return Err("Live timing envelope has an unexpected event type".into());
                }
                let coordinate = |name: &str| -> Result<f64, String> {
                    let value: f64 = event
                        .attr(name)
                        .parse()
                        .map_err(|_| "Timing event lacks a numeric coordinate")?;
                    if !value.is_finite() {
                        return Err("Nonfinite Live timing event".into());
                    }
                    Ok(value)
                };
                let time = coordinate("Time")?;
                let value = coordinate("Value")?;
                if time <= previous || time < -63072000. {
                    return Err(
                        "Live timing envelope is unordered or outside the source pre-roll range"
                            .into(),
                    );
                }
                previous = time;
                let tick = tick(time.max(0.), source)?;
                if tempo {
                    let point = Tempo::new(tick, value, false)?;
                    precision |= (60000000. / f64::from(point.micros) - value).abs() > 1e-8;
                    curves |= [
                        "CurveControl1X",
                        "CurveControl1Y",
                        "CurveControl2X",
                        "CurveControl2Y",
                    ]
                    .iter()
                    .any(|key| !event.attr(key).is_empty());
                    tempos.insert(tick, point);
                } else {
                    meters.insert(tick, make_meter(tick, meter(value)?));
                }
            }
            if tempo {
                let mut points: Vec<_> = tempos.values_mut().collect();
                let last = points.len().saturating_sub(1);
                for point in points.iter_mut().take(last) {
                    point.ramp = true;
                }
                if precision {
                    source.difference("master","Tempo precision","Native conductor stores integer microseconds per quarter note; source BPM values remain archived for comparison")?;
                }
                if curves {
                    source.difference("master","Tempo curves","Bezier handles retained; native tempo interpolates linearly between the original breakpoints. Use a render if the exact source curve is required")?;
                }
            }
        }
    }
    let conductor = Conductor::native(
        PPQN,
        tempos.into_values().collect(),
        meters.into_values().collect(),
        TimingSettings::default(),
    )?;
    state.bpm = (60000000. / f64::from(conductor.tempos[0].micros)) as f32;
    state.conductor = Some(conductor);
    Ok(())
}
