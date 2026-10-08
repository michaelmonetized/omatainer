//! Producer receipts keep queued explicit controls attached to the same objects.
use super::{Axis, Reference, Registry};
use crate::engine::{Command, RtEngine};

#[derive(Clone, Debug)]
pub(crate) struct Scoped {
    pub track: Option<(usize, Reference)>,
    pub scene: Option<(usize, Reference)>,
    pub command: Box<Command>,
}
impl Scoped {
    pub(crate) fn qualify(command: Command, registry: &Registry) -> Result<Command, Command> {
        if !registry.known() {
            return Ok(command);
        }
        Self::qualify_using(command, &|axis, slot| registry.reference(axis, slot))
    }
    pub(crate) fn qualify_layout(
        command: Command,
        layout: &super::Layout,
    ) -> Result<Command, Command> {
        Self::qualify_using(command, &|axis, slot| layout.reference(axis, slot))
    }
    fn qualify_using(
        command: Command,
        reference: &dyn Fn(Axis, usize) -> Option<Reference>,
    ) -> Result<Command, Command> {
        let (track, scene) = match &command {
            Command::Gesture { .. } => {
                return match command {
                    Command::Gesture { id, command } => Self::qualify_using(*command, reference)
                        .map(|inner| Command::Gesture {
                            id,
                            command: Box::new(inner),
                        }),
                    _ => unreachable!(),
                }
            }
            Command::MidiAdjust(adjust) => {
                let Some(track) = adjust.track() else { return Ok(command); };
                (Some(track), None)
            }
            Command::TrackGain { track, .. }
            | Command::ClipCancel { track }
            | Command::TrackPan { track, .. }
            | Command::Mute { track }
            | Command::Solo { track }
            | Command::Arm { track }
            | Command::TrackArm { track, .. }
            | Command::TrackMonitor { track, .. }
            | Command::TrackPfl { track, .. }
            | Command::OpenFxTrack(track) => (Some(*track as usize), None),
            Command::LaunchClip { track, scene }
            | Command::ClipPress(super::super::clip_launch::Press { target: super::super::clip_launch::Target::Slot { track, scene, .. }, .. })
            | Command::FireClip { track, scene, .. }
            | Command::ClipGain { track, scene, .. }
            | Command::SetNotes { track, scene, .. } => {
                (Some(*track as usize), Some(*scene as usize))
            }
            Command::Select { track, scene } | Command::ComposeArm { track, scene } => {
                (Some(*track), Some(*scene))
            }
            Command::LaunchScene { scene }
            | Command::RestartScene { scene }
            | Command::AddScene { scene }
            | Command::ToggleScene { scene }
            | Command::OpenFxScene(scene) => (None, Some(*scene as usize)),
            _ => return Ok(command),
        };
        let track = match track {
            Some(slot) => match reference(Axis::Track, slot) {
                Some(id) => Some((slot, id)),
                None => return Err(command),
            },
            None => None,
        };
        let scene = match scene {
            Some(slot) => match reference(Axis::Scene, slot) {
                Some(id) => Some((slot, id)),
                None => return Err(command),
            },
            None => None,
        };
        if track
            .zip(scene)
            .is_some_and(|((_, t), (_, s))| t.namespace != s.namespace)
        {
            return Err(command);
        }
        Ok(Command::SessionControl(Self {
            track,
            scene,
            command: Box::new(command),
        }))
    }
    pub(crate) fn current(&self, rt: &RtEngine) -> bool {
        self.track
            .is_none_or(|(slot, target)| rt.session.resolves(Axis::Track, slot, target))
            && self
                .scene
                .is_none_or(|(slot, target)| rt.session.resolves(Axis::Scene, slot, target))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{project, test_alloc, Engine};
    #[test]
    fn queued_controls_keep_target_after_reorder_and_reject_another_project_without_heap_activity()
    {
        let (engine, mut rt) = Engine::headless_for_test(48000, 256);
        engine
            .send(Command::TrackGain {
                track: 2,
                value: 0.17,
            })
            .unwrap();
        let id = rt.session.tracks[2].id;
        let (request, _) = crate::engine::session::Request::metadata(
            &rt.session,
            rt.undo.checkpoint().epoch,
            crate::engine::session::Action::Move {
                axis: Axis::Track,
                id,
                position: 0,
            },
        )
        .unwrap();
        rt.apply(Command::session_edit(request));
        assert_eq!(
            test_alloc::measure(|| rt.process(&mut [])),
            test_alloc::Counts::default()
        );
        assert_eq!(rt.tracks[2].gain, 0.17);
        assert_eq!(rt.session.track_order[0], 2);
        engine
            .send(Command::TrackGain {
                track: 2,
                value: 0.33,
            })
            .unwrap();
        let mut another = project::maximum_for_test();
        another.swap_into(&mut rt);
        let gain = rt.tracks[2].gain;
        assert_eq!(
            test_alloc::measure(|| rt.process(&mut [])),
            test_alloc::Counts::default()
        );
        assert_eq!(rt.tracks[2].gain, gain);
        assert!(rt.undo.checkpoint().untracked == 0);
        assert!(engine
            .send(Command::TrackGain {
                track: 200,
                value: 0.2
            })
            .is_err());
    }
    #[test]
    fn every_track_stop_and_global_stop_remain_reserved_at_the_128_track_limit() {
        let (engine, mut rt) = Engine::headless_for_test(48000, 256);
        let mut prepared = project::maximum_for_test();
        prepared.swap_into(&mut rt);
        while engine
            .send(Command::TrackGain {
                track: 127,
                value: 0.37,
            })
            .is_ok()
        {}
        let ordinary = engine.cmd.len();
        assert_eq!(ordinary, 111);
        for track in 0..128 {
            engine.send(Command::StopTrack { track }).unwrap();
        }
        engine.send(Command::Stop).unwrap();
        for pad in 0..16 {
            engine.send(Command::SamplerSlotStop { pad }).unwrap();
        }
        assert_eq!(engine.cmd.len(), 256);
        for _ in 0..8 {
            assert_eq!(
                test_alloc::measure(|| rt.process(&mut [])),
                test_alloc::Counts::default()
            );
        }
        assert_eq!(engine.cmd.len(), 0);
        assert!(!rt.playing);
        engine.send(Command::LaunchScene { scene: 511 }).unwrap();
        rt.process(&mut []);
        assert_eq!(rt.selected_scene, 511);
    }
}
