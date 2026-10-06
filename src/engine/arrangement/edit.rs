use super::super::{
    midi_edit::{Ack, Outcome},
    undo::Checkpoint,
    RtEngine,
};
use super::*;
use std::sync::atomic::AtomicBool;

/// Own a reviewed timeline replacement and its acknowledgment.
/// Preparation validates native save limits and builds the entire schedule outside audio; admission requires the unchanged, stopped project.
#[derive(Clone, Debug)]
pub(crate) struct Request {
    pub(in crate::engine) replacement: Option<Box<Playback>>,
    original_bytes: usize,
    pub(in crate::engine) reserved: Option<Arc<Plan>>,
    pub(in crate::engine) ack: Ack,
    checkpoint: Checkpoint,
}
impl Request {
    /// Prepare a complete song edit from one coherent capture.
    /// Takes captured state, replacement metadata and cancellation; returns a prepared Undo transaction or refuses stale/unsaveable content.
    pub(crate) fn prepare(
        mut captured: project::Captured,
        model: Model,
        cancel: &AtomicBool,
    ) -> Result<(Box<Self>, Ack), String> {
        let layout = captured
            .state
            .session
            .as_ref()
            .ok_or("Arrangement requires retained track identities")?;
        let original_bytes = captured
            .state
            .arrangement
            .as_ref()
            .map(|m| Plan::prepare(m.clone(), &captured.media, layout, cancel).map(|p| p.bytes()))
            .transpose()?
            .unwrap_or(0);
        captured.state.version = project::STATE_VERSION;
        captured.state.arrangement = Some(Arc::new(model));
        captured.state.compact_media(&mut captured.media)?;
        let plan = Plan::prepare(
            captured.state.arrangement.as_ref().unwrap().clone(),
            &captured.media,
            captured.state.session.as_ref().unwrap(),
            cancel,
        )?;
        let limits = crate::project_file::Limits::default();
        if captured.media.len() > limits.max_media
            || captured
                .media
                .iter()
                .map(|s| s.data.len().saturating_mul(4))
                .sum::<usize>() as u64
                > limits.max_pcm_bytes
        {
            return Err("Arrangement sources exceed native media storage".into());
        }
        crate::project_file::validate_metadata(
            &crate::project_file::Bundle {
                state: captured.state.clone(),
                media: captured.media.clone(),
            },
            &limits,
            cancel,
        )
        .map_err(|e| e.to_string())?;
        let ack = Ack::new();
        let request = Box::new(Self {
            replacement: Some(Playback::new(Some(plan.clone()), captured.state.beat)),
            original_bytes,
            reserved: Some(plan),
            ack: ack.clone(),
            checkpoint: captured.checkpoint,
        });
        Ok((request, ack))
    }
    pub(in crate::engine) fn current(&self, rt: &RtEngine) -> bool {
        self.ack.state() == Outcome::Pending
            && self.checkpoint == rt.undo.checkpoint()
            && !rt.playing
            && !rt.recording
            && rt.count_in.is_none()
            && !rt.decks.iter().any(|d| d.playing || d.touching)
    }
    pub(in crate::engine) fn bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + self.original_bytes
            + self.reserved.as_ref().map_or(0, |p| p.bytes())
            + self.replacement.as_ref().map_or(0, |p| p.storage_bytes())
    }
}
