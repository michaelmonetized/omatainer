use super::*;
use crate::engine::{test_alloc, Command, Engine};
use std::time::Duration;

fn follower(policy: LossPolicy) -> (Arc<Shared>, Runtime) {
    let shared = Arc::new(Shared::default());
    let mut rt = Runtime::new(shared.clone());
    rt.configure(
        Config {
            source: Some(71),
            loss: policy,
            ..Default::default()
        },
        0.0,
        false,
        0,
    );
    (shared, rt)
}
fn send(shared: &Shared, source: u64, at: u64, message: Message) {
    shared.input(
        source,
        shared.generation(),
        0,
        shared.origin + Duration::from_nanos(at),
        message,
        false,
    );
}
fn tick(shared: &Shared, at: u64) {
    send(shared, 71, at, Message::Tick { packet_ticks: 1 });
}

#[test]
fn timestamped_jitter_and_missing_pulses_follow_an_independent_audio_sample_oracle_without_heap() {
    let mut cases = 0;
    let mut max_phase = 0.0_f64;
    for rate in [44100_u32, 48000, 96000, 192000] {
        for bpm in [40_u32, 73, 120, 137, 180, 240] {
            for amplitude in [100_000_i64, 1_000_000] {
                let (shared, mut rt) = follower(LossPolicy::Freewheel);
                let base = 1_000_000_000_u64;
                rt.begin_at(rate, base, true);
                send(&shared, 71, base, Message::Start);
                let mut next = 0_u64;
                let mut beat = 0.0;
                let mut running = false;
                let mut maximum = 0.0_f64;
                let mut allocations = test_alloc::Counts::default();
                for frame in 0..rate as usize * 3 {
                    let now = base + frame as u64 * 1_000_000_000 / u64::from(rate);
                    let nominal = base + next * 60_000_000_000 / (u64::from(bpm) * 24);
                    let jitter = if next == 0 {
                        0
                    } else if next % 2 == 0 {
                        amplitude
                    } else {
                        -amplitude
                    };
                    let arrival = nominal.saturating_add_signed(jitter);
                    if now >= arrival {
                        if next != 17 {
                            tick(&shared, arrival);
                        }
                        next += 1;
                    }
                    let measured = test_alloc::measure(|| {
                        let update = rt.frame(frame, beat, running, 0);
                        if let Some(position) = update.start {
                            beat = position;
                            running = true;
                        }
                        if update.stop {
                            running = false;
                        }
                        if running {
                            beat += update.step.unwrap_or(120.0 / 60.0 / f64::from(rate));
                        }
                    });
                    allocations.allocations += measured.allocations;
                    allocations.frees += measured.frees;
                    if frame > rate as usize {
                        let expected = (frame + 1) as f64 * f64::from(bpm) / 60.0 / f64::from(rate);
                        maximum = maximum.max((beat - expected).abs());
                    }
                }
                assert_eq!(allocations, test_alloc::Counts::default());
                let status = rt.status();
                assert!(status.locked, "{rate}/{bpm}: {status:?}");
                assert!(
                    (status.estimated_bpm.unwrap() - f64::from(bpm)).abs() < 0.1,
                    "{rate}/{bpm}: {status:?}"
                );
                assert!(maximum < 0.07, "{rate}/{bpm}: phase {maximum}, {status:?}");
                assert_eq!(status.inferred_missing_ticks, 1, "{rate}/{bpm}: {status:?}");
                max_phase = max_phase.max(maximum);
                cases += 1;
            }
        }
    }
    println!("CLOCK_INPUT_ORACLE {{\"cases\":{cases},\"maximum_phase_beats\":{max_phase},\"jitter_amplitudes_ns\":[100000,1000000],\"physical_devices_opened\":false}}");
}

#[test]
fn start_continue_position_stop_wait_for_the_next_selected_clock_and_ignore_other_sources() {
    let (shared, mut rt) = follower(LossPolicy::Freewheel);
    rt.begin_at(48000, 1_000_000_000, true);
    send(&shared, 72, 1_000_000_000, Message::Start);
    tick(&shared, 1_000_000_000);
    let update = rt.frame(0, 9.0, false, 0);
    assert!(update.start.is_none());
    assert_eq!(rt.status().ignored, 1);
    send(&shared, 71, 1_000_000_001, Message::Start);
    let update = rt.frame(1, 9.0, false, 0);
    assert!(update.start.is_none());
    assert!(rt.status().awaiting_tick);
    tick(&shared, 1_020_833_333);
    assert_eq!(rt.frame(1000, 9.0, false, 0).start, Some(0.0));
    send(&shared, 71, 1_021_000_000, Message::Stop);
    let update = rt.frame(1010, 2.5, true, 0);
    assert!(update.stop);
    assert!(update.start.is_none());
    send(&shared, 71, 1_022_000_000, Message::Position(23));
    send(&shared, 71, 1_022_000_000, Message::Continue);
    assert!(rt.frame(1100, 2.5, false, 0).start.is_none());
    tick(&shared, 1_041_666_666);
    assert_eq!(rt.frame(2000, 2.5, false, 0).start, Some(5.75));
    send(&shared, 71, 1_042_000_000, Message::Position(16383));
    assert!(rt.frame(2020, 5.75, true, 0).start.is_none());
    tick(&shared, 1_062_500_000);
    assert_eq!(rt.frame(3000, 5.75, true, 0).start, Some(4095.75));
}

#[test]
fn queue_pressure_selected_stop_and_loss_fence_old_start_without_callback_heap_work() {
    for policy in [LossPolicy::Freewheel, LossPolicy::Stop] {
        let (shared, mut rt) = follower(policy);
        rt.begin_at(48000, 1_000_000_000, true);
        let old = shared.generation();
        for _ in 0..EVENTS {
            send(&shared, 71, 1_000_000_000, Message::Start);
        }
        assert_eq!(
            test_alloc::measure(|| send(&shared, 71, 1_000_000_000, Message::Stop)),
            Default::default()
        );
        let update = rt.frame(0, 7.0, true, 0);
        assert!(update.stop);
        assert!(update.start.is_none());
        assert_eq!(rt.status().lost, Some(Loss::Overflow));
        assert_ne!(old, shared.generation());
        shared.input(
            71,
            old,
            0,
            shared.origin + Duration::from_secs(1),
            Message::Start,
            false,
        );
        assert!(rt.frame(1, 7.0, false, 0).start.is_none());
        send(&shared, 71, 1_000_041_666, Message::Continue);
        tick(&shared, 1_020_833_333);
        assert_eq!(rt.frame(1000, 7.0, false, 0).start, Some(7.0));
    }
}

#[test]
fn silence_freewheel_stop_retirement_feedback_and_selection_are_generation_scoped() {
    for policy in [LossPolicy::Freewheel, LossPolicy::Stop] {
        let (shared, mut rt) = follower(policy);
        rt.begin_at(48000, 1_000_000_000, true);
        send(&shared, 71, 1_000_000_000, Message::Start);
        for pulse in 0..25 {
            tick(&shared, 1_000_000_000 + pulse * 20_833_333);
            rt.frame((pulse * 1000) as usize, pulse as f64 / 24.0, pulse > 0, 0);
        }
        let bpm = rt.status().estimated_bpm.unwrap();
        let update = rt.frame(49000, 1.0, true, 0);
        assert_eq!(update.stop, policy == LossPolicy::Stop);
        assert_eq!(rt.status().lost, Some(Loss::Silence));
        assert_eq!(rt.status().estimated_bpm, Some(bpm));
        tick(&shared, 2_041_666_666);
        rt.frame(50000, 1.0, policy == LossPolicy::Freewheel, 0);
        assert_eq!(rt.status().reacquisitions, 1);
        assert!(!rt.status().locked);
        shared.retire(72, Loss::Retired);
        assert!(rt.frame(50001, 1.0, true, 0).stop == false);
        shared.retire(71, Loss::Retired);
        rt.frame(50002, 1.0, true, 0);
        assert_eq!(rt.status().lost, Some(Loss::Retired));
        shared.input(
            71,
            shared.generation(),
            0,
            shared.origin + Duration::from_secs(3),
            Message::Tick { packet_ticks: 1 },
            true,
        );
        rt.frame(96000, 1.0, true, 0);
        assert_eq!(rt.status().lost, Some(Loss::Feedback));
        rt.configure(
            Config {
                source: Some(72),
                ..Default::default()
            },
            1.0,
            false,
            0,
        );
        send(&shared, 71, 3_000_000_000, Message::Start);
        tick(&shared, 3_000_000_000);
        assert!(rt.frame(96000, 1.0, false, 0).start.is_none());
        assert_eq!(rt.status().accepted_ticks, 0);
    }
}

#[test]
fn batched_clocks_change_tempo_without_confusing_sustained_slower_clock_with_dropped_pulses() {
    let (shared, mut rt) = follower(LossPolicy::Freewheel);
    let base = 1_000_000_000_u64;
    rt.begin_at(48000, base, true);
    tick(&shared, base);
    rt.frame(0, 0.0, true, 0);
    for packet in 1..=12 {
        let at = base + packet * 41_666_666;
        for _ in 0..2 {
            send(&shared, 71, at, Message::Tick { packet_ticks: 2 });
        }
        rt.frame((packet * 2000) as usize, packet as f64 / 12.0, true, 0);
    }
    assert_eq!(rt.status().accepted_ticks, 25);
    assert_eq!(rt.status().batched_ticks, 24);
    assert!((rt.status().estimated_bpm.unwrap() - 120.0).abs() < 0.001);
    for pulse in 1..=48 {
        let at = base + 500_000_000 + pulse * 41_666_666;
        tick(&shared, at);
        rt.frame(
            (24000 + pulse * 2000) as usize,
            1.0 + pulse as f64 / 24.0,
            true,
            0,
        );
    }
    assert!(
        (rt.status().estimated_bpm.unwrap() - 60.0).abs() < 0.05,
        "{:?}",
        rt.status()
    );
    assert_eq!(rt.status().inferred_missing_ticks, 0);
}

#[test]
fn exact_system_common_parser_preserves_realtime_interleaving_and_refuses_orphan_truncated_position(
) {
    use crate::engine::midi::routing::packet::{frames, Frame, Packet};
    assert_eq!(
        frames(&[0xf2, 23, 0xf8, 1, 0xfa]).collect::<Vec<_>>(),
        vec![
            Frame::Realtime(0xf8),
            Frame::SongPosition(151),
            Frame::Realtime(0xfa)
        ]
    );
    assert_eq!(
        frames(&[0xf2, 127, 127, 23, 1]).collect::<Vec<_>>(),
        vec![Frame::SongPosition(16383)]
    );
    assert!(frames(&[0xf2, 23]).all(|f| matches!(f, Frame::Malformed)));
    assert!(frames(&[23, 1]).next().is_none());
    assert!(Packet::new(&[0xf2, 23, 1]).is_none());
}

#[test]
fn actual_private_worker_preserves_input_time_position_selection_and_manual_takeover() {
    let (engine, mut rt) = Engine::headless_for_test(48000, 128);
    let map = crate::engine::midi::MidiMap {
        name: "Clock fixture".into(),
        matchers: vec![],
        bindings: vec![],
        unmapped_notes: crate::engine::midi::UnmappedNotes::Live,
    };
    let mut input =
        engine
            .midi
            .open_for_test(&engine.cmd, 71, map.clone(), "Fixture clock", "clock:1");
    let mut other = engine
        .midi
        .open_for_test(&engine.cmd, 72, map, "Unselected clock", "clock:2");
    engine
        .cmd
        .send(Command::ClockFollow(Config {
            source: Some(71),
            ..Default::default()
        }))
        .unwrap();
    rt.process(&mut []);
    let origin = rt.clock_input.shared.origin;
    other.push_at(&[0xfa, 0xf8], origin + Duration::from_secs(1));
    input.push_at(&[0xf2, 23, 0xf8, 0, 0xfb], origin + Duration::from_secs(1));
    rt.clock_input.begin_at(48000, 1_000_000_000, true);
    rt.process(&mut []);
    assert!(!rt.playing);
    assert!(rt.clock_input.status().awaiting_tick);
    assert_eq!(rt.clock_input.status().accepted_ticks, 1);
    input.push_at(&[0xf8], origin + Duration::from_nanos(1_020_833_333));
    rt.clock_input.begin_at(48000, 1_020_833_333, true);
    rt.process(&mut []);
    assert!(rt.playing);
    assert_eq!(rt.precise_midi_beat(), 5.75);
    assert_eq!(rt.clock_input.status().ignored, 2);
    input.push_at(&[0xfc], origin + Duration::from_nanos(1_041_666_666));
    rt.clock_input.begin_at(48000, 1_041_666_666, true);
    rt.process(&mut []);
    assert!(!rt.playing);
    assert_eq!(rt.precise_midi_beat(), 5.75);
    rt.apply(Command::SetBpm(137.0));
    assert!(!rt.clock_input.enabled());
    input.push_at(&[0xfa, 0xf8], origin + Duration::from_nanos(1_062_500_000));
    rt.clock_input.begin_at(48000, 1_062_500_000, true);
    rt.process(&mut []);
    assert!(rt.playing);
    assert_eq!(rt.bpm, 137.0);
}

#[test]
fn supported_tempo_boundaries_and_invalid_configuration_cannot_escape_the_project_domain() {
    assert!(!Config {
        source: Some(0),
        ..Default::default()
    }
    .valid());
    assert!(!Config {
        timeout_ms: 249,
        ..Default::default()
    }
    .valid());
    assert!(!Config {
        timeout_ms: 2001,
        ..Default::default()
    }
    .valid());
    for bpm in [30_u64, 40, 240, 300] {
        let (shared, mut rt) = follower(LossPolicy::Freewheel);
        rt.begin_at(48000, 1_000_000_000, true);
        for pulse in 0..=24 {
            let at = 1_000_000_000 + pulse * 60_000_000_000 / (bpm * 24);
            tick(&shared, at);
            rt.frame(
                ((at - 1_000_000_000) * 48000 / 1_000_000_000 + 1) as usize,
                0.0,
                false,
                0,
            );
        }
        if [40, 240].contains(&bpm) {
            assert!(rt.status().locked, "{bpm}: {:?}", rt.status());
            assert!((rt.status().estimated_bpm.unwrap() - bpm as f64).abs() < 0.001);
        } else {
            assert!(!rt.status().locked);
            assert!(rt.status().unsupported_bpm.is_some());
        }
        assert!(rt
            .status()
            .estimated_bpm
            .is_none_or(|bpm| (40.0..=240.0).contains(&bpm)));
    }
}

#[test]
fn unsupported_tempo_after_a_slower_change_never_publishes_an_invalid_project_bpm() {
    let (shared, mut rt) = follower(LossPolicy::Freewheel);
    let base = 1_000_000_000_u64;
    rt.begin_at(48000, base, true);
    for pulse in 0..=24 {
        let at = base + pulse * 60_000_000_000 / (120 * 24);
        tick(&shared, at);
        rt.frame(
            ((at - base) * 48000 / 1_000_000_000 + 1) as usize,
            0.0,
            false,
            0,
        );
    }
    for pulse in 1..=40 {
        let at = base + 500_000_000 + pulse * 60_000_000_000 / (30 * 24);
        tick(&shared, at);
        rt.frame(
            ((at - base) * 48000 / 1_000_000_000 + 1) as usize,
            0.0,
            false,
            0,
        );
        assert!(
            (40.0..=240.0).contains(&rt.status().estimated_bpm.unwrap()),
            "{:?}",
            rt.status()
        );
    }
    assert!(rt.status().unsupported_bpm.is_some());
    assert!(!rt.status().locked);
}

#[test]
fn external_stop_continue_chases_overlapping_clip_notes_and_preserves_live_owners_and_launch_identity(
) {
    use crate::engine::{
        dsp::{InputKey, VoiceOwner},
        MidiNote,
    };
    let (_engine, mut rt) = Engine::headless_for_test(48000, 128);
    rt.quant = 0.0;
    let notes = vec![
        MidiNote { variation: None,
            id: crate::engine::midi_edit::NoteId::new(),
            muted: false,
            pitch: 60,
            start: 0.0,
            len: 3.0,
            vel: 100,
            channel: 0,
            release_vel: 64,
            source_timing: None,
        },
        MidiNote { variation: None,
            id: crate::engine::midi_edit::NoteId::new(),
            muted: false,
            pitch: 60,
            start: 0.01,
            len: 1.0,
            vel: 90,
            channel: 0,
            release_vel: 64,
            source_timing: None,
        },
    ];
    let ids = notes.iter().map(|n| n.id).collect::<Vec<_>>();
    rt.apply(Command::SetNotes {
        track: 2,
        scene: 0,
        notes,
    });
    rt.apply(Command::LaunchClip { track: 2, scene: 0 });
    rt.selected_track = 1;
    rt.apply(Command::LiveNoteOn {
        source: 72,
        ch: 0,
        note: 65,
        vel: 100,
    });
    rt.configure_clock_input(Config {
        source: Some(71),
        ..Default::default()
    });
    let shared = rt.clock_input.shared.clone();
    let base = 1_000_000_000_u64;
    send(&shared, 71, base, Message::Start);
    tick(&shared, base);
    rt.clock_input.begin_at(48000, base, true);
    rt.process(&mut [0.0; 2048]);
    let launch = rt.tracks[2].playing.unwrap();
    let held_beat = rt.precise_midi_beat();
    assert!(rt.tracks[2]
        .poly
        .voices
        .iter()
        .any(|v| v.owner == VoiceOwner::Clip && matches!(v.env.stage, 1..=3)));
    send(&shared, 71, base + 21_333_333, Message::Stop);
    rt.clock_input.begin_at(48000, base + 21_333_333, true);
    assert_eq!(
        test_alloc::measure(|| rt.process(&mut [])),
        Default::default()
    );
    assert!(!rt.playing);
    assert_eq!(rt.precise_midi_beat(), held_beat);
    assert!(rt.tracks[2].playing.is_none());
    let saved = rt.tracks[2].project_resume.unwrap();
    assert_eq!(saved.scene, launch.scene);
    assert_eq!(saved.start_beat, launch.start_beat);
    assert_eq!(saved.midi_start_beat, launch.midi_start_beat);
    assert!(rt.tracks[2]
        .poly
        .voices
        .iter()
        .all(|v| v.owner != VoiceOwner::Clip || !matches!(v.env.stage, 1..=3)));
    assert!(rt.tracks[1].poly.voices.iter().any(|v| v.input
        == Some(InputKey::Midi {
            source: 72,
            ch: 0,
            note: 65
        })
        && matches!(v.env.stage, 1..=3)));
    send(&shared, 71, base + 22_000_000, Message::Continue);
    tick(&shared, base + 41_666_666);
    rt.clock_input.begin_at(48000, base + 41_666_666, true);
    assert_eq!(
        test_alloc::measure(|| rt.process(&mut [0.0; 2])),
        Default::default()
    );
    assert!(rt.playing);
    assert!(rt.precise_midi_beat() > held_beat);
    assert!(rt.tracks[2].project_resume.is_none());
    assert!(rt.tracks[2]
        .poly
        .voices
        .iter()
        .any(|v| v.owner == VoiceOwner::Clip && matches!(v.env.stage, 1..=3)));
    assert_eq!(
        rt.tracks[2].clips[0]
            .notes
            .iter()
            .map(|n| n.id)
            .collect::<Vec<_>>(),
        ids
    );
    assert!(rt.tracks[1].poly.voices.iter().any(|v| v.input
        == Some(InputKey::Midi {
            source: 72,
            ch: 0,
            note: 65
        })
        && matches!(v.env.stage, 1..=3)));
}

#[test]
fn imported_conductor_is_preserved_external_samples_override_it_and_deliberate_play_restores_it() {
    use crate::engine::midi_data::Conductor;
    use crate::midi_file::{Meta, MetaValue};
    let (_engine, mut rt) = Engine::headless_for_test(48000, 128);
    let map = Conductor::from_meta(
        480,
        vec![
            Meta {
                tick: 0,
                order: 0,
                value: MetaValue::Tempo(600000),
            },
            Meta {
                tick: 1920,
                order: 1,
                value: MetaValue::Tempo(750000),
            },
        ]
        .into_iter(),
    )
    .unwrap();
    rt.conductor = Some(map.clone());
    rt.configure_clock_input(Config {
        source: Some(71),
        ..Default::default()
    });
    let shared = rt.clock_input.shared.clone();
    let base = 1_000_000_000_u64;
    send(&shared, 71, base, Message::Start);
    for pulse in 0..25 {
        tick(&shared, base + pulse * 20_833_333);
    }
    rt.clock_input.begin_at(48000, base, true);
    let mut output = vec![0.0; 48000];
    assert_eq!(
        test_alloc::measure(|| rt.process(&mut output)),
        Default::default()
    );
    assert!(output.iter().all(|x| x.is_finite()));
    assert_eq!(rt.timeline_seconds(), 0.5);
    assert!((rt.bpm - 120.0).abs() < 0.01);
    assert!(Arc::ptr_eq(rt.conductor.as_ref().unwrap(), &map));
    assert!((rt.note_recording.clock - rt.precise_midi_beat()).abs() < 1e-7);
    rt.apply(Command::Play);
    assert!(!rt.clock_input.enabled());
    rt.process(&mut [0.0; 2]);
    assert!((rt.bpm - 100.0).abs() < 0.001);
    assert!(Arc::ptr_eq(rt.conductor.as_ref().unwrap(), &map));
}

#[test]
fn actual_output_callback_keeps_an_unsynced_deck_pcm_exact_while_external_song_clock_changes() {
    use crate::engine::audio::OutputCallback;
    let (_engine, mut followed) = Engine::headless_for_test(48000, 128);
    let (_reference, mut internal) = Engine::headless_for_test(48000, 128);
    let audio = followed.decks[1].audio.clone().unwrap();
    for rt in [&mut followed, &mut internal] {
        for track in &mut rt.tracks {
            track.gain = 0.0;
        }
        rt.metronome = false;
        rt.fx_wet = [0.0; 3];
        rt.apply(Command::DeckAudio {
            deck: 1,
            audio: audio.clone(),
        });
        rt.apply(Command::DeckPlay { deck: 1 });
        rt.master = 0.4;
    }
    followed.configure_clock_input(Config {
        source: Some(71),
        ..Default::default()
    });
    let shared = followed.clock_input.shared.clone();
    let start = Instant::now() + Duration::from_secs(30);
    shared.input(71, shared.generation(), 0, start, Message::Start, false);
    for pulse in 0..64 {
        shared.input(
            71,
            shared.generation(),
            0,
            start + Duration::from_nanos(pulse * 60_000_000_000 / (180 * 24)),
            Message::Tick { packet_ticks: 1 },
            false,
        );
    }
    let mut followed = OutputCallback::new(followed, 2);
    let mut internal = OutputCallback::new(internal, 2);
    let mut actual = [0.0_f32; 1024];
    let mut expected = [0.0_f32; 1024];
    let mut energy = 0.0_f64;
    for block in 0..32 {
        let target = start + Duration::from_nanos(block * 512 * 1_000_000_000 / 48000);
        let latency = target.checked_duration_since(Instant::now()).unwrap();
        assert_eq!(
            test_alloc::measure(|| followed.render_timed(&mut actual, Some(latency))),
            Default::default()
        );
        internal.render(&mut expected);
        assert_eq!(actual, expected, "independent deck block {block}");
        energy += actual.iter().map(|&x| f64::from(x * x)).sum::<f64>();
    }
    assert!(energy > 0.01);
    assert!(followed.renderer_for_test().clock_input.status().locked);
    assert!((followed.renderer_for_test().bpm - 180.0).abs() < 0.01);
    println!("CLOCK_INPUT_CALLBACK {{\"other_deck_compared_frames\":16384,\"other_deck_max_error\":0,\"callback_allocations\":0,\"callback_frees\":0,\"energy\":{energy},\"physical_devices_opened\":false}}");
}

#[test]
fn safety_and_prepared_project_swap_return_internal_and_discard_source_owned_pending_transport() {
    use crate::engine::{performance::Safety, project::Prepared};
    for swap in [false, true] {
        let (_engine, mut rt) = Engine::headless_for_test(48000, 128);
        let mut project = Prepared::empty(48000).unwrap();
        rt.configure_clock_input(Config {
            source: Some(71),
            ..Default::default()
        });
        let shared = rt.clock_input.shared.clone();
        let old = shared.generation();
        send(&shared, 71, 1_000_000_000, Message::Start);
        if swap {
            assert_eq!(
                test_alloc::measure(|| project.swap_into(&mut rt)),
                Default::default()
            );
        } else {
            rt.apply(Command::SafetyStop(Safety::Stop));
        }
        assert!(!rt.clock_input.enabled());
        assert_ne!(shared.generation(), old);
        shared.input(
            71,
            old,
            rt.performance.input_epoch(),
            shared.origin + Duration::from_secs(1),
            Message::Start,
            false,
        );
        rt.clock_input.begin_at(48000, 1_000_000_000, true);
        rt.process(&mut [0.0; 2]);
        assert!(!rt.playing);
        assert_eq!(rt.clock_input.status().accepted_ticks, 0);
    }
}

#[test]
fn automatic_song_loop_wrap_preserves_external_phase_across_imported_tempo_anchors() {
    use crate::engine::{
        clip_launch::Grid,
        midi_data::Conductor,
        song_navigation::{
            metadata::{Loop, Model, Saved},
            Action,
        },
    };
    use crate::midi_file::{Meta, MetaValue};
    let mut maximum = 0.0_f64;
    for rate in [44100_u32, 48000, 96000, 192000] {
        let (_engine, mut rt) = Engine::headless_for_test(rate, 128);
        rt.bpm = 120.0;
        let map = Conductor::from_meta(
            480,
            vec![
                Meta {
                    tick: 0,
                    order: 0,
                    value: MetaValue::Tempo(750000),
                },
                Meta {
                    tick: 720,
                    order: 1,
                    value: MetaValue::Tempo(250000),
                },
            ]
            .into_iter(),
        )
        .unwrap();
        rt.conductor = Some(map.clone());
        rt.navigation.saved = Some(Saved {
            model: Arc::new(Model {
                loop_region: Some(Loop {
                    start: 1.0,
                    end: 2.0,
                }),
                ..Default::default()
            }),
            looping: true,
            next_id: 1,
        });
        rt.configure_clock_input(Config {
            source: Some(71),
            ..Default::default()
        });
        let shared = rt.clock_input.shared.clone();
        let base = 1_000_000_000_u64;
        send(&shared, 71, base, Message::Start);
        for pulse in 0..96 {
            tick(&shared, base + pulse * 60_000_000_000 / (120 * 24));
        }
        let frames = rate as usize * 7 / 4;
        let mut done = 0;
        let mut output = [0.0; 256];
        while done < frames {
            let block = (frames - done).min(128);
            rt.clock_input.begin_at(
                rate,
                base + done as u64 * 1_000_000_000 / u64::from(rate),
                true,
            );
            assert_eq!(
                test_alloc::measure(|| rt.process(&mut output[..block * 2])),
                Default::default()
            );
            done += block;
            assert!(rt.clock_input.enabled());
            assert!(output[..block * 2].iter().all(|x| x.is_finite()));
        }
        let error = (rt.precise_midi_beat() - 1.5).abs();
        maximum = maximum.max(error);
        assert!(
            error < 0.001,
            "rate {rate}: {error}, {:?}",
            rt.clock_input.status()
        );
        assert!(rt.clock_input.status().locked);
        assert!(rt.clock_input.status().lost.is_none());
        assert!(rt.clock_input.status().phase_error_beats.abs() < 0.001);
        assert!(Arc::ptr_eq(rt.conductor.as_ref().unwrap(), &map));
        rt.apply(Command::SongNavigation(Action::Beat {
            beat: 4.0,
            grid: Grid::Immediate,
        }));
        assert!(!rt.clock_input.enabled());
        assert!((rt.precise_midi_beat() - 4.0).abs() < 1e-9);
    }
    println!("CLOCK_INPUT_LOOP {{\"sample_rates\":4,\"maximum_position_error_beats\":{maximum},\"automatic_wrap_retains_clock\":true,\"manual_locator_returns_internal\":true,\"physical_devices_opened\":false}}");
}

#[test]
fn producer_refuses_invalid_selection_and_external_selection_retires_count_in_without_restarting() {
    let (engine, mut rt) = Engine::headless_for_test(48000, 128);
    for config in [
        Config {
            source: Some(0),
            ..Default::default()
        },
        Config {
            timeout_ms: 249,
            ..Default::default()
        },
        Config {
            timeout_ms: 2001,
            ..Default::default()
        },
    ] {
        assert!(matches!(
            engine.cmd.send(Command::ClockFollow(config)),
            Err(crate::engine::SubmissionError::InvalidTarget)
        ));
    }
    assert!(rt.cmd_rx.is_empty());
    use crate::engine::midi_data::{Conductor, Meter, Tempo, TimingSettings};
    rt.conductor = Some(
        Conductor::native(
            480,
            vec![Tempo::new(0, 120.0, false).unwrap()],
            vec![Meter {
                tick: 0,
                numerator: 4,
                denominator_power: 2,
                clocks: 24,
                thirty_seconds: 8,
            }],
            TimingSettings {
                count_in: 1,
                ..Default::default()
            },
        )
        .unwrap(),
    );
    rt.apply(Command::Play);
    assert!(rt.count_in.is_some());
    engine
        .cmd
        .send(Command::ClockFollow(Config {
            source: Some(71),
            ..Default::default()
        }))
        .unwrap();
    assert_eq!(
        test_alloc::measure(|| rt.process(&mut [])),
        Default::default()
    );
    assert!(rt.clock_input.enabled());
    assert!(rt.count_in.is_none());
    assert!(rt.playing);
    assert_eq!(rt.precise_midi_beat(), 0.0);
}

#[test]
fn fresh_start_after_a_long_stopped_gap_waits_for_clock_and_preserves_a_held_position() {
    for policy in [LossPolicy::Freewheel, LossPolicy::Stop] {
        let (shared, mut rt) = follower(policy);
        let base = 1_000_000_000_u64;
        rt.begin_at(48000, base, true);
        send(&shared, 71, base, Message::Start);
        tick(&shared, base);
        assert_eq!(rt.frame(0, 0.0, false, 0).start, Some(0.0));
        tick(&shared, base + 20_833_333);
        rt.frame(1000, 0.04, true, 0);
        send(&shared, 71, base + 30_000_000, Message::Stop);
        assert!(rt.frame(1440, 0.1, true, 0).stop);
        let later = base + 30_000_000_000;
        send(&shared, 71, later, Message::Start);
        rt.begin_at(48000, later, true);
        let update = rt.frame(0, 0.1, false, 0);
        assert!(update.start.is_none());
        assert!(rt.status().awaiting_tick);
        tick(&shared, later + 10_000_000);
        assert_eq!(rt.frame(480, 0.1, false, 0).start, Some(0.0));
        assert_eq!(rt.status().reacquisitions, 1);
        send(&shared, 71, later + 20_000_000, Message::Stop);
        assert!(rt.frame(960, 0.5, true, 0).stop);
        send(&shared, 71, later + 30_000_000, Message::Position(35));
        rt.frame(1440, 0.5, false, 0);
        rt.frame(30000, 0.5, false, 0);
        assert_eq!(rt.status().lost, Some(Loss::Silence));
        send(&shared, 71, later + 1_000_000_000, Message::Continue);
        tick(&shared, later + 1_000_000_000);
        assert_eq!(rt.frame(48000, 0.5, false, 0).start, Some(8.75));
    }
}

#[test]
fn extreme_supported_tempo_changes_accept_every_pulse_and_converge_without_reverse_steps() {
    let mut cases = 0;
    for (from, to) in [(40_u64, 240_u64), (240, 40), (73, 180), (180, 73)] {
        let (shared, mut rt) = follower(LossPolicy::Freewheel);
        let base = 1_000_000_000_u64;
        rt.begin_at(48000, base, true);
        for pulse in 0..=24 {
            let at = base + pulse * 60_000_000_000 / (from * 24);
            tick(&shared, at);
            rt.frame(
                ((at - base) * 48000 / 1_000_000_000 + 1) as usize,
                pulse as f64 / 24.0,
                true,
                0,
            );
        }
        let changed = base + 60_000_000_000 / from;
        for pulse in 1..=64 {
            let at = changed + pulse * 60_000_000_000 / (to * 24);
            tick(&shared, at);
            let update = rt.frame(
                ((at - base) * 48000 / 1_000_000_000 + 1) as usize,
                1.0 + pulse as f64 / 24.0,
                true,
                0,
            );
            assert!(update.step.unwrap() > 0.0);
            assert!(update.start.is_none());
        }
        assert_eq!(
            rt.status().accepted_ticks,
            89,
            "{from}->{to}: {:?}",
            rt.status()
        );
        assert!(rt.status().lost.is_none());
        assert_eq!(rt.status().inferred_missing_ticks, 0);
        assert!(
            (rt.status().estimated_bpm.unwrap() - to as f64).abs() < 0.05,
            "{from}->{to}: {:?}",
            rt.status()
        );
        cases += 1;
    }
    println!("CLOCK_INPUT_TEMPO {{\"cases\":{cases},\"accepted_pulses_per_case\":89,\"physical_devices_opened\":false}}");
}

#[test]
fn reacquisition_uses_new_intervals_immediately_instead_of_retired_estimator_slots() {
    for (from, to) in [(120_u64, 240_u64), (240, 40), (40, 73)] {
        let (shared, mut rt) = follower(LossPolicy::Freewheel);
        let base = 1_000_000_000_u64;
        rt.begin_at(48000, base, true);
        for pulse in 0..=9 {
            let at = base + pulse * 60_000_000_000 / (from * 24);
            tick(&shared, at);
            rt.frame(
                ((at - base) * 48000 / 1_000_000_000 + 1) as usize,
                0.0,
                true,
                0,
            );
        }
        let prior = rt.status().estimated_bpm.unwrap();
        rt.frame(120000, 0.0, true, 0);
        assert_eq!(rt.status().lost, Some(Loss::Silence));
        let later = base + 3_000_000_000;
        tick(&shared, later);
        rt.frame(144001, 0.0, true, 0);
        let next = later + 60_000_000_000 / (to * 24);
        tick(&shared, next);
        rt.frame(
            ((next - base) * 48000 / 1_000_000_000 + 1) as usize,
            0.0,
            true,
            0,
        );
        let expected = 1.0 / (0.75 / prior + 0.25 / to as f64);
        assert!(
            (rt.status().estimated_bpm.unwrap() - expected).abs() < 0.001,
            "{from}->{to}: {:?}",
            rt.status()
        );
        assert_eq!(rt.status().reacquisitions, 1);
        assert_eq!(rt.status().inferred_missing_ticks, 0);
    }
}
