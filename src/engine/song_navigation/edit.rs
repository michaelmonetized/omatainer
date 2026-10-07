use super::{
    super::{
        clip_launch::Grid,
        midi_edit::{Ack, Outcome},
        project, RtEngine,
    },
    metadata::{Model, Saved},
};
use std::sync::{atomic::AtomicBool, Arc};

#[derive(Clone, Copy, Debug)]
pub(crate) enum Action {
    Locator { id: u16, grid: Grid },
    Previous(Grid),
    Next(Grid),
    Beat { beat: f64, grid: Grid },
    Seconds(f64),
    ToggleLoop,
    Cancel,
}
impl Action {
    /// Validate navigation command data before queue admission.
    /// Takes one copied action; returns false for zero IDs, nonfinite values or unsupported native ranges.
    pub(crate) fn valid(self) -> bool {
        match self {
            Self::Locator { id, .. } => id != 0,
            Self::Beat { beat, .. } => super::metadata::position(beat),
            Self::Seconds(seconds) => seconds.is_finite() && (0.0..=86400.0).contains(&seconds),
            _ => true,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub(crate) enum Error {
    MissingLocator,
    MissingNeighbor,
    MissingLoop,
    PositionLimit,
}
impl Error {
    /// Explain a refused live destination.
    /// Takes one static error; returns user-visible text without allocating on audio.
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::MissingLocator => "Locator no longer exists.",
            Self::MissingNeighbor => "There is no named section in that direction.",
            Self::MissingLoop => "Draw loop braces before enabling song looping.",
            Self::PositionLimit => "The destination exceeds the supported song time range.",
        }
    }
}
#[derive(Clone, Copy, Debug)]
pub(crate) struct Pending {
    pub beat: f64,
    pub when: f64,
    pub loop_after: bool,
}
#[derive(Default)]
pub(crate) struct Runtime {
    pub saved: Option<Saved>,
    pub pending: Option<Pending>,
    pub error: Option<Error>,
}
impl Runtime {
    /// Retire pending song movement while retaining saved sections and braces.
    /// Takes this live runtime; clears the queued jump and its visible refusal.
    pub(crate) fn cancel(&mut self) {
        self.pending = None;
        self.error = None;
    }
}
fn same(a: Option<&Saved>, b: Option<&Saved>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => a.same(b),
        _ => false,
    }
}
#[derive(Clone, Debug)]
pub(crate) struct Request {
    pub(in crate::engine) expected: Option<Saved>,
    pub(in crate::engine) replacement: Option<Saved>,
    pub(in crate::engine) inverse: Option<Saved>,
    pub(in crate::engine) jump: Option<(f64, Grid)>,
    pub(in crate::engine) loop_after: bool,
    pub(in crate::engine) ack: Ack,
    epoch: u64,
    namespace: [u64; 2],
    retained_bytes: usize,
}
impl Request {
    /// Prepare a live, saveable locator edit on its worker.
    /// Takes coherent native state, the exact reviewed navigation, replacement metadata, loop flag, optional section jump and cancellation; returns an atomic Ack transaction or leaves the live song untouched.
    pub(crate) fn prepare(
        mut captured: project::Captured,
        expected: Option<Saved>,
        model: Model,
        looping: bool,
        jump: Option<(f64, Grid)>,
        cancel: &AtomicBool,
    ) -> Result<(Box<Self>, Ack), String> {
        if !same(expected.as_ref(), captured.state.navigation.as_ref()) {
            return Err("Song sections changed; refresh and review this edit again".into());
        }
        let namespace = captured
            .state
            .session
            .as_ref()
            .ok_or("Song identity is unavailable")?
            .namespace;
        let floor = expected.as_ref().map_or(1, |saved| saved.next_id);
        if model.next_id < floor
            || model.locators.iter().any(|locator| {
                u32::from(locator.id) < floor
                    && !expected
                        .as_ref()
                        .is_some_and(|saved| saved.model.destination(locator.id).is_some())
            })
        {
            return Err(
                "Locator IDs cannot be reused after deletion or Undo; refresh the section draft"
                    .into(),
            );
        }
        let next_id = model.next_id;
        let inverse = Some(expected.as_ref().map_or_else(
            || Saved {
                model: Arc::new(Model {
                    next_id,
                    ..Model::default()
                }),
                looping: false,
                next_id,
            },
            |saved| Saved {
                next_id,
                ..saved.clone()
            },
        ));
        let mut saved = Saved {
            model: Arc::new(model),
            looping,
            next_id,
        };
        saved.validate()?;
        let conductor = captured
            .state
            .conductor
            .as_ref()
            .map(|map| map.prepare())
            .transpose()?;
        let seconds = |beat| {
            conductor
                .as_ref()
                .map_or(beat * 60.0 / f64::from(captured.state.bpm), |map| {
                    map.seconds_at(beat)
                })
        };
        if saved
            .model
            .locators
            .iter()
            .any(|locator| seconds(locator.beat) > 86400.0)
            || saved
                .model
                .loop_region
                .is_some_and(|region| seconds(region.end) > 86400.0)
        {
            return Err("Song sections exceed the supported song time range".into());
        }
        if jump.is_some() {
            saved.looping = false;
        }
        if jump.is_some_and(|(beat, _)| !super::metadata::position(beat)) {
            return Err("Section jump is outside the supported song range".into());
        }
        let bytes = saved.model.bytes()
            + expected.as_ref().map_or(0, |saved| saved.model.bytes())
            + inverse.as_ref().map_or(0, |saved| saved.model.bytes())
            + std::mem::size_of::<Self>();
        captured.state.version = super::super::project::STATE_VERSION;
        captured.state.navigation = Some(saved.clone());
        crate::project_file::validate_metadata(
            &crate::project_file::Bundle {
                state: captured.state.clone(),
                media: captured.media.clone(),
            },
            &captured.state.import_metadata_limits(),
            cancel,
        )
        .map_err(|error| error.to_string())?;
        let ack = Ack::new();
        let request = Box::new(Self {
            expected,
            replacement: Some(saved),
            inverse,
            jump,
            loop_after: looping,
            ack: ack.clone(),
            epoch: captured.checkpoint.epoch,
            namespace,
            retained_bytes: bytes,
        });
        Ok((request, ack))
    }
    /// Recheck one exact reviewed locator edit without rebuilding metadata.
    /// Takes the audio-owned song; returns whether epoch, namespace, navigation identity and cancellation still match.
    pub(in crate::engine) fn current(&self, rt: &RtEngine) -> bool {
        self.ack.state() == Outcome::Pending
            && self.epoch == rt.undo.checkpoint().epoch
            && self.namespace == rt.session.namespace
            && same(self.expected.as_ref(), rt.navigation.saved.as_ref())
    }
    /// Reserve retained locator metadata for command and Undo admission.
    /// Takes this worker-prepared request; returns the conservative retained byte total.
    pub(in crate::engine) fn bytes(&self) -> usize {
        self.retained_bytes
    }
}
