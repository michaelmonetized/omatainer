use super::*;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

#[derive(Clone, Debug)]
pub(crate) struct Request {
    pub audio: Arc<Sample>,
    pub cancel: Arc<AtomicBool>,
    pub state: Arc<AtomicU8>,
    pub transport_epoch: u64,
    pub safety_epoch: u64,
}
impl Request {
    /// Charge the actual owned preview storage.
    /// Takes this request; returns bytes counted by admission and off-thread retirement.
    pub fn bytes(&self) -> usize {
        self.audio.data.capacity() * 4
            + self.audio.name.capacity()
            + self.audio.path.capacity()
            + self.audio.peaks.capacity() * std::mem::size_of::<[f32; 3]>()
            + std::mem::size_of::<Self>()
    }
    /// Settle a request rejected before application.
    /// Takes this request; preserves any renderer-owned applied or ended state.
    pub fn reject(&self) {
        let _ = self
            .state
            .compare_exchange(0, 2, Ordering::AcqRel, Ordering::Acquire);
    }
}
pub(crate) struct Active {
    request: Request,
    position: f64,
}

impl RtEngine {
    /// Install one unchanged-pitch preview outside project and deck state.
    /// Takes prepared PCM and current epochs; accepts or retires it without callback heap release.
    pub(super) fn apply_provider_preview(&mut self, request: Request) {
        let previous = self
            .provider_preview
            .as_ref()
            .map_or(0, |active| active.request.bytes());
        let valid = !request.cancel.load(Ordering::Acquire)
            && request.transport_epoch == self.transport_epoch
            && request.safety_epoch == self.performance.safety_epoch()
            && (8000..=384000).contains(&request.audio.sr)
            && matches!(request.audio.ch, 1 | 2)
            && request.audio.frames() >= 2
            && request.audio.data.len() % request.audio.ch as usize == 0
            && request.audio.path.is_empty()
            && request.bytes() <= 128 * 1024 * 1024
            && self
                .undo
                .can_retire_device(previous.saturating_add(request.bytes()));
        if valid {
            if let Some(active) = self.provider_preview.take() {
                active.request.state.store(3, Ordering::Release);
                self.undo
                    .retire_command(Command::ProviderPreview(active.request));
            }
            request.state.store(1, Ordering::Release);
            self.provider_preview = Some(Active {
                request,
                position: 0.0,
            });
        } else {
            request.reject();
            self.undo.retire_command(Command::ProviderPreview(request));
        }
    }

    /// Retire stopped previews only at a block boundary with worker capacity.
    /// Takes current renderer state; releases ownership to the existing retirement worker.
    pub(super) fn maintain_provider_preview(&mut self) {
        if let Some(active) = &self.provider_preview {
            if active.request.cancel.load(Ordering::Acquire)
                || active.request.transport_epoch != self.transport_epoch
                || active.request.safety_epoch != self.performance.safety_epoch()
            {
                active.request.state.store(3, Ordering::Release);
            }
            if active.request.state.load(Ordering::Acquire) == 3
                && self.undo.can_retire_device(active.request.bytes())
            {
                let active = self.provider_preview.take().unwrap();
                self.undo
                    .retire_command(Command::ProviderPreview(active.request));
            }
        }
    }

    /// Render one original-pitch preview sample without deck effects or export capture.
    /// Takes this renderer; returns stereo preview audio and keeps its owner until block retirement.
    pub(super) fn tick_provider_preview(&mut self) -> [f32; 2] {
        let Some(active) = &mut self.provider_preview else {
            return [0.0; 2];
        };
        if active.request.transport_epoch != self.transport_epoch
            || active.request.safety_epoch != self.performance.safety_epoch()
        {
            active.request.state.store(3, Ordering::Release);
        }
        if active.request.cancel.load(Ordering::Acquire)
            || active.request.state.load(Ordering::Acquire) != 1
        {
            return [0.0; 2];
        }
        if active.position >= active.request.audio.frames() as f64 - 1.0 {
            active.request.state.store(3, Ordering::Release);
            return [0.0; 2];
        }
        let (l, r) = active.request.audio.at(active.position);
        active.position += f64::from(active.request.audio.sr) / f64::from(self.sr);
        [l * 0.25, r * 0.25]
    }
}

#[cfg(test)]
mod tests;
