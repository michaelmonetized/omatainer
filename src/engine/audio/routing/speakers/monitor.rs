use super::BassManagement;
use crate::engine::audio::routing::model::{Direction, Model, MAX_PHYSICAL_CHANNELS};

#[derive(Debug)]
pub(crate) struct Runtime {
    channels: [usize; 3],
    blend: f32,
    low: [f32; 2],
    level: f32,
}
impl Runtime {
    /// Prepare the explicitly selected software monitoring split.
    /// Takes a validated 2.1 alias, cutoff, level and actual sample rate; returns fixed filter history without opening a device.
    pub fn new(config: BassManagement, model: &Model, rate: u32) -> Self {
        let channels = &model
            .port(config.output, Direction::Output)
            .unwrap()
            .channels;
        Self {
            channels: std::array::from_fn(|i| usize::from(channels[i])),
            blend: 1. - (-std::f32::consts::TAU * config.cutoff / rate.max(1) as f32).exp(),
            low: [0.; 2],
            level: config.level,
        }
    }
    /// Split front-channel bass only in the physical monitoring path.
    /// Takes the completed physical frame; replaces each front channel with its complementary high pass and adds their averaged low pass to the existing unity LFE.
    pub fn process(&mut self, frame: &mut [f32; MAX_PHYSICAL_CHANNELS]) {
        let mut bass = 0.;
        for channel in 0..2 {
            let index = self.channels[channel];
            let source = frame[index];
            self.low[channel] += self.blend * (source - self.low[channel]);
            frame[index] = source - self.low[channel];
            bass += self.low[channel] * 0.5;
        }
        frame[self.channels[2]] += bass * self.level;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stereo_lfe_monitor_split_matches_independent_recurrence_and_preserves_dedicated_lfe_without_heap(
    ) {
        for rate in [44100, 48000, 96000] {
            let mut model = Model::default();
            model.ports[0].channels = vec![3, 0, 5];
            let config = BassManagement {
                output: 1,
                cutoff: 80.,
                level: 0.75,
            };
            let mut monitor = Runtime::new(config, &model, rate);
            let a = 1. - (-std::f32::consts::TAU * 80. / rate as f32).exp();
            let mut low = [0_f32; 2];
            for i in 0..2048 {
                let input = if i == 8 {
                    [0.25, 0.125, 0.0625]
                } else {
                    [0.; 3]
                };
                let mut frame = [0.; MAX_PHYSICAL_CHANNELS];
                frame[3] = input[0];
                frame[0] = input[1];
                frame[5] = input[2];
                assert_eq!(
                    crate::engine::test_alloc::measure(|| monitor.process(&mut frame)),
                    Default::default()
                );
                for channel in 0..2 {
                    low[channel] = (1. - a) * low[channel] + a * input[channel];
                }
                for (physical, expected) in [
                    (3, input[0] - low[0]),
                    (0, input[1] - low[1]),
                    (5, input[2] + 0.75 * 0.5 * (low[0] + low[1])),
                ] {
                    assert!(
                        (frame[physical] - expected).abs() < 1e-7,
                        "{rate}/{i}/{physical}"
                    );
                }
                for physical in [1, 2, 4, 6, 63] {
                    assert_eq!(frame[physical], 0.);
                }
            }
        }
    }
}

#[cfg(test)]
mod integration_tests {
    use super::*;
    use crate::engine::{
        audio::routing::{model::*, prepared::Prepared, speakers::Layout},
        test_alloc, Command, Engine,
    };
    use std::sync::Arc;
    #[test]
    fn stereo_lfe_monitor_split_is_excluded_from_offline_program_and_raw_record_taps() {
        let (_engine, mut rt) = Engine::headless_for_test(48000, 256);
        rt.apply(Command::Stop);
        let mut model = Model::default();
        model.next_id = 3;
        model.ports[0].channels = vec![3, 0, 5];
        model.ports.push(Port {
            id: 2,
            alias: "Declared bed input".into(),
            direction: Direction::Input,
            channels: vec![0, 1, 2],
        });
        model.set_speaker_layout(Group::Output(1), Layout::StereoLfe);
        model.set_speaker_layout(Group::Input(2), Layout::StereoLfe);
        model.connections = vec![Connection {
            source: Source {
                group: Group::Input(2),
                tap: Tap::PostMixer,
            },
            destination: Group::Output(1),
            map: (0..3)
                .map(|c| ChannelMap {
                    source: c,
                    destination: c,
                    gain: 1.,
                })
                .collect(),
        }];
        model.monitor_bass = Some(BassManagement {
            output: 1,
            cutoff: 80.,
            level: 1.,
        });
        let mut live = Prepared::at_rate(Arc::new(model.clone()), &rt.session, 48000).unwrap();
        let mut offline = Prepared::at_rate(Arc::new(model), &rt.session, 48000).unwrap();
        offline.offline();
        let a = 1. - (-std::f32::consts::TAU * 80. / 48000.).exp();
        let mut low = [0_f32; 2];
        for i in 0..256 {
            let input = if i == 8 {
                [0.25, 0.125, 0.0625]
            } else {
                [0.; 3]
            };
            rt.routing_input_frame[..3].copy_from_slice(&input);
            let mut actual = [0.; MAX_PHYSICAL_CHANNELS];
            assert_eq!(
                test_alloc::measure(|| actual = live.render(&mut rt, false, 0., 6)),
                Default::default()
            );
            let raw = offline.render(&mut rt, false, 0., 6);
            for c in 0..2 {
                low[c] = (1. - a) * low[c] + a * input[c];
            }
            for c in 0..3 {
                let physical = [3, 0, 5][c];
                assert!((raw[physical] - input[c].tanh()).abs() < 1e-7);
                let expected = if c < 2 {
                    input[c] - low[c]
                } else {
                    input[c] + 0.5 * (low[0] + low[1])
                };
                assert!((actual[physical] - expected.tanh()).abs() < 1e-7);
            }
            let output = live
                .nodes
                .iter()
                .find(|n| n.group == Group::Output(1))
                .unwrap();
            assert_eq!(
                &output.input[..3],
                &input,
                "Pre-monitor record source must retain the declared bed"
            );
        }
    }
}
