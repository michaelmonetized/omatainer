use super::*;
use crate::engine::{midi_edit::Ack, project::Captured, RtEngine};

#[derive(Clone, Copy, Debug)]
pub(crate) enum Parameter {
    Gain(f32),
    Mute(bool),
    Tone(bool),
    Eq { band: u8, db: f32 },
    Duck(bool),
    Threshold(f32),
    Reduction(f32),
    Attack(f32),
    Release(f32),
    Mode(Override),
}
#[derive(Clone, Copy, Debug)]
pub(crate) struct Control {
    pub namespace: [u64; 2],
    pub role: u8,
    pub input: Option<u64>,
    pub parameter: Parameter,
}
impl Control {
    /// Resolve a live scalar gesture against its reviewed source.
    /// Takes the current renderer; returns a validated configuration or refuses a changed project/source.
    pub(crate) fn resolve(&self, rt: &RtEngine) -> Option<Configuration> {
        if rt.session.namespace != self.namespace || self.role >= 2 {
            return None;
        }
        let mut cfg = rt.mic_aux.configuration()?;
        let c = &mut cfg.channels[usize::from(self.role)];
        if c.input != self.input {
            return None;
        }
        match self.parameter {
            Parameter::Gain(v) => c.gain = v,
            Parameter::Mute(v) => c.mute = v,
            Parameter::Tone(v) => c.tone = v,
            Parameter::Eq { band, db } => *c.eq_db.get_mut(usize::from(band))? = db,
            Parameter::Duck(v) => cfg.duck.enabled = v,
            Parameter::Threshold(v) => cfg.duck.threshold = v,
            Parameter::Reduction(v) => cfg.duck.reduction_db = v,
            Parameter::Attack(v) => cfg.duck.attack_ms = v,
            Parameter::Release(v) => cfg.duck.release_ms = v,
            Parameter::Mode(v) => cfg.duck.mode = v,
        }
        let finite = |v: f32, a: f32, b: f32| v.is_finite() && (a..=b).contains(&v);
        let valid = match self.parameter {
            Parameter::Gain(v) => finite(v, 0.0, 4.0),
            Parameter::Eq { band, db } => band < 3 && finite(db, -18.0, 18.0),
            Parameter::Threshold(v) => finite(v, 0.00001, 1.0),
            Parameter::Reduction(v) => finite(v, 0.0, 40.0),
            Parameter::Attack(v) => finite(v, 1.0, 2000.0),
            Parameter::Release(v) => finite(v, 1.0, 10000.0),
            _ => true,
        };
        if !valid {
            return None;
        }
        Some(cfg)
    }
}
#[derive(Clone, Debug)]
pub(crate) struct Request {
    namespace: [u64; 2],
    generation: u64,
    revision: u64,
    rate: u32,
    pub configuration: Option<Configuration>,
    pub ack: Ack,
}
impl Request {
    /// Prepare a reviewed mic/aux topology change.
    /// Takes a coherent capture, rate and desired roles; returns a bounded request and application receipt without opening a device.
    pub(crate) fn new(
        captured: &Captured,
        rate: u32,
        configuration: Option<Configuration>,
    ) -> Result<(Box<Self>, Ack), String> {
        let layout = captured
            .state
            .session
            .as_ref()
            .ok_or("Session identity unavailable")?;
        if let Some(cfg) = configuration {
            cfg.validate(captured.state.routing.as_deref())
                .map_err(str::to_owned)?;
        }
        let ack = Ack::new();
        Ok((
            Box::new(Self {
                namespace: layout.namespace,
                generation: layout.generation,
                revision: captured.revision,
                rate,
                configuration,
                ack: ack.clone(),
            }),
            ack,
        ))
    }
    /// Validate an unapplied source-selection receipt.
    /// Takes the renderer; refuses changed aliases, project identity, rate, active recording or playing/touched decks.
    pub(crate) fn current(&self, rt: &RtEngine) -> bool {
        self.ack.state() == crate::engine::midi_edit::Outcome::Pending
            && self.namespace == rt.session.namespace
            && self.generation == rt.session.generation
            && self.revision == rt.project.revision()
            && self.rate == rt.sr as u32
            && !rt.playing
            && !rt.recording
            && !rt.routing_pipe.recorder.busy()
            && !rt.decks.iter().any(|d| d.playing || d.touching)
            && self.configuration.is_none_or(|cfg| {
                cfg.validate(rt.routing.as_ref().map(|r| r.model.as_ref()))
                    .is_ok()
            })
    }
}
