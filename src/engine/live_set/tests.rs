use super::*;

fn constant(value: f32) -> Prepared {
    let mut prepared = Prepared::empty(48_000).unwrap();
    prepared.rt.master = 1.0;
    prepared.rt.xfader = 0.0;
    prepared.rt.playing = true;
    prepared.rt.decks[0].audio = Some(Arc::new(dsp::Sample { spectrum: None, name: "resident".into(), sr: 48_000, ch: 2,
        data: vec![value; 480_000], peaks: Arc::new(vec![]), bpm: 120.0, path: String::new() }));
    prepared.rt.decks[0].playing = true;
    prepared
}

fn queue(live: &mut RtEngine, prepared: Prepared) -> (Handle, Arc<Control>) {
    let handle = live.project.live_sets();
    let control = handle.stage(handle.reserve().unwrap(), prepared, live.session.namespace, Arc::new(AtomicBool::new(false))).unwrap();
    let counts = test_alloc::measure(|| live.process_interleaved(&mut [0.0; 512], 4));
    assert_eq!(counts, test_alloc::Counts::default());
    assert!(control.ready());
    (handle, control)
}

#[test]
fn separate_cue_preserves_outgoing_master_and_cancellation_retires_off_callback() {
    let mut live = constant(0.2).rt;
    let mut reference = constant(0.2).rt;
    let namespace = live.session.namespace;
    let (handle, control) = queue(&mut live, constant(-0.3));
    reference.process(&mut [0.0; 256]);
    control.preview.store(true, Ordering::Release);
    let mut heard = 0.0;
    for _ in 0..32 {
        let mut output = [0.0; 512];
        let mut expected = [0.0; 256];
        reference.process(&mut expected);
        let counts = test_alloc::measure(|| live.process_interleaved(&mut output, 4));
        assert_eq!(counts, test_alloc::Counts::default());
        for (frame, expected) in output.chunks_exact(4).zip(expected.chunks_exact(2)) {
            assert_eq!(&frame[..2], expected);
            heard += frame[2].abs() + frame[3].abs();
        }
    }
    assert!(heard > 100.0);
    control.preview.store(false, Ordering::Release);
    live.process_interleaved(&mut [0.0; 16_384], 4);
    reference.process(&mut [0.0; 8192]);
    let cue_position = live.live_set.as_ref().unwrap().prepared.rt.decks[0].pos;
    live.process_interleaved(&mut [0.0; 512], 4);
    reference.process(&mut [0.0; 256]);
    assert_eq!(live.live_set.as_ref().unwrap().prepared.rt.decks[0].pos, cue_position);
    assert_eq!(live.session.namespace, namespace);
    assert_eq!(live.decks[0].pos, reference.decks[0].pos);
    assert!(handle.reserve().is_err());
    control.cancel.store(true, Ordering::Release);
    let counts = test_alloc::measure(|| live.process_interleaved(&mut [0.0; 512], 4));
    assert_eq!(counts, test_alloc::Counts::default());
    assert!(handle.busy());
    assert!(matches!(handle.retire(), Some(Err(Error::Cancelled))));
    assert!(!handle.busy());
    assert!(live.decks[0].playing);
}

#[test]
fn transition_matches_both_complete_renderers_sample_by_sample_and_keeps_tails() {
    for channels in [1, 2, 4] {
        let mut live = constant(0.2).rt;
        let mut outgoing = constant(0.2).rt;
        live.fx_wet[0] = 0.4;
        outgoing.fx_wet[0] = 0.4;
        let mut incoming = constant(-0.3);
        incoming.rt.fx_wet[1] = 0.2;
        let incoming_namespace = incoming.rt.session.namespace;
        let mut reference = constant(-0.3).rt;
        reference.fx_wet[1] = 0.2;
        let (handle, control) = queue(&mut live, incoming);
        outgoing.process(&mut [0.0; 256]);
        control.transition(&live.project, live.project.revision(), 0.02).unwrap();
        let mut elapsed = 0u64;
        let mut energy = 0.0;
        for _ in 0..9 {
            let mut output = vec![0.0; 128 * channels];
            let mut old = [0.0; 256];
            let mut next = [0.0; 256];
            outgoing.process(&mut old);
            reference.process(&mut next);
            let counts = test_alloc::measure(|| live.process_interleaved(&mut output, channels));
            assert_eq!(counts, test_alloc::Counts::default());
            for ((actual, old), next) in output.chunks_exact(channels).zip(old.chunks_exact(2)).zip(next.chunks_exact(2)) {
                let weight = (elapsed as f64 / 959.0).min(1.0) as f32;
                let expected = if channels == 1 { vec![(next[0] + next[1]) * 0.5 * weight + (old[0] + old[1]) * 0.5 * (1.0-weight)] }
                    else { vec![next[0] * weight + old[0] * (1.0-weight), next[1] * weight + old[1] * (1.0-weight)] };
                for (actual, expected) in actual.iter().zip(expected) { assert!((actual - expected).abs() < 1e-6, "frame {elapsed}: {actual} != {expected}"); energy += actual.abs(); }
                elapsed += 1;
            }
        }
        assert!(energy > 10.0);
        assert_eq!(live.session.namespace, incoming_namespace);
        assert!(live.decks[0].playing);
        assert!(handle.applied().is_some());
        assert!(matches!(handle.retire(), Some(Ok(()))));
        assert!(!handle.busy());
    }
}

#[test]
fn stale_recording_rate_and_safety_refusals_preserve_the_current_set() {
    for condition in 0..5 {
        let mut live = constant(0.2).rt;
        let namespace = live.session.namespace;
        let (handle, control) = queue(&mut live, constant(-0.3));
        match condition {
            0 => { control.transition(&live.project, live.project.revision(), 0.01).unwrap(); live.project.edited(); }
            1 => { control.transition(&live.project, live.project.revision(), 0.01).unwrap(); live.recording = true; }
            2 => { live.set_sample_rate(44_100).unwrap(); }
            3 => { live.performance.request_safety(performance::Safety::Silence); }
            _ => { control.cancel.store(true, Ordering::Release); }
        }
        let counts = test_alloc::measure(|| live.process(&mut [0.0; 256]));
        assert_eq!(counts, test_alloc::Counts::default());
        assert_eq!(live.session.namespace, namespace);
        assert!(handle.applied().is_none());
        assert!(handle.retire().unwrap().is_err());
    }
}

#[test]
fn cue_requires_four_channels_and_cancel_after_commit_does_not_undo_transition() {
    let mut live = constant(0.2).rt;
    let incoming = constant(-0.3);
    let namespace = incoming.rt.session.namespace;
    let (handle, control) = queue(&mut live, incoming);
    control.preview.store(true, Ordering::Release);
    live.process(&mut [0.0; 256]);
    assert!(!control.cue_available.load(Ordering::Acquire));
    assert_eq!(live.live_set.as_ref().unwrap().prepared.rt.decks[0].pos, 0.0);
    control.transition(&live.project, live.project.revision(), 0.01).unwrap();
    live.process(&mut [0.0; 256]);
    control.cancel.store(true, Ordering::Release);
    for _ in 0..4 { live.process(&mut [0.0; 256]); }
    assert_eq!(live.session.namespace, namespace);
    assert!(handle.applied().is_some());
    assert!(matches!(handle.retire(), Some(Ok(()))));
}

#[test]
fn outgoing_effect_tail_is_retained_and_mixed_waveform_timing_stays_unknown() {
    let mut live = constant(0.2).rt;
    let mut reference = constant(0.2).rt;
    live.fx_wet[0] = 0.6;
    reference.fx_wet[0] = 0.6;
    for _ in 0..192 {
        live.process(&mut [0.0; 256]);
        reference.process(&mut [0.0; 256]);
    }
    live.decks[0].playing = false;
    reference.decks[0].playing = false;
    let (handle, control) = queue(&mut live, constant(0.0));
    reference.process(&mut [0.0; 256]);
    let timing = live.audible.handle();
    control.transition(&live.project, live.project.revision(), 0.02).unwrap();
    let mut elapsed = 0u64;
    let mut tail_energy = 0.0;
    for block in 0..8 {
        let now = 1_000_000_000 + block * 128 * 1_000_000_000 / 48_000;
        live.audible.begin(48_000, Some(now));
        let mut output = [0.0; 256];
        let mut tail = [0.0; 256];
        reference.process(&mut tail);
        let counts = test_alloc::measure(|| live.process(&mut output));
        assert_eq!(counts, test_alloc::Counts::default());
        live.audible.finish();
        assert!(timing.positions_at(now).is_none());
        for (actual, old) in output.chunks_exact(2).zip(tail.chunks_exact(2)) {
            let gain = 1.0 - (elapsed as f64 / 959.0).min(1.0) as f32;
            for (actual, old) in actual.iter().zip(old) { assert!((actual - old * gain).abs() < 1e-6); tail_energy += actual.abs(); }
            elapsed += 1;
        }
    }
    assert!(tail_energy > 10.0, "an actual delayed outgoing tail must remain audible");
    assert!(handle.applied().is_some());
    assert!(matches!(handle.retire(), Some(Ok(()))));
}

#[test]
fn loading_reservation_is_exclusive_and_safety_silences_a_committed_tail() {
    let mut live = constant(0.2).rt;
    let handle = live.project.live_sets();
    let reservation = handle.reserve().unwrap();
    assert!(handle.busy());
    assert!(handle.reserve().is_err());
    drop(reservation);
    assert!(!handle.busy());
    let (handle, control) = queue(&mut live, constant(-0.3));
    control.transition(&live.project, live.project.revision(), 2.0).unwrap();
    live.process(&mut [0.0; 256]);
    assert!(handle.applied().is_some());
    live.performance.request_safety(performance::Safety::Silence);
    let mut output = [9.0; 512];
    let counts = test_alloc::measure(|| live.process_interleaved(&mut output, 4));
    assert_eq!(counts, test_alloc::Counts::default());
    assert_eq!(output, [0.0; 512]);
    assert!(matches!(handle.retire(), Some(Err(Error::Protected(performance::Error::Recovery)))));
}

fn routed(value: f32, main: [u16; 2], extra: [u16; 2]) -> Prepared {
    use audio::routing::model::*;
    let mut prepared = constant(value);
    let mut model = Model::default();
    model.ports[0].channels = main.to_vec();
    model.next_id = 3;
    model.ports.push(Port { id: 2, alias: "Independent deck output".into(), direction: Direction::Output, channels: extra.to_vec() });
    model.connections.push(Connection { source: Source { group: Group::Deck(0), tap: Tap::PostFx }, destination: Group::Output(2),
        map: vec![ChannelMap { source: 0, destination: 0, gain: 0.35 }, ChannelMap { source: 1, destination: 1, gain: 0.65 }] });
    prepared.rt.routing = Some(Box::new(audio::routing::prepared::Prepared::new(Arc::new(model), &prepared.rt.session).unwrap()));
    prepared
}

#[test]
fn routed_fades_match_all_physical_outputs_and_cue_cannot_replace_a_program_alias() {
    for (channels, main, extra, cue) in [(6, [0, 1], [4, 5], true), (6, [4, 5], [2, 3], false), (64, [60, 61], [62, 63], true)] {
        let mut live = routed(0.2, main, extra).rt;
        let mut outgoing = routed(0.2, main, extra).rt;
        let incoming = routed(-0.3, main, extra);
        let namespace = incoming.rt.session.namespace;
        let mut next = routed(-0.3, main, extra).rt;
        let handle = live.project.live_sets();
        let control = handle.stage(handle.reserve().unwrap(), incoming, live.session.namespace, Arc::new(AtomicBool::new(false))).unwrap();
        let mut output = vec![0.0; channels * 128];
        live.process_interleaved(&mut output, channels);
        outgoing.process_interleaved(&mut vec![0.0; channels * 128], channels);
        assert!(control.ready());
        assert_eq!(control.cue_available.load(Ordering::Acquire), cue);
        control.preview.store(true, Ordering::Release);
        for _ in 0..16 {
            let mut old = vec![0.0; channels * 128];
            outgoing.process_interleaved(&mut old, channels);
            if cue { next.process_interleaved(&mut vec![0.0; channels * 128], channels); }
            assert_eq!(test_alloc::measure(|| live.process_interleaved(&mut output, channels)), test_alloc::Counts::default());
            for (actual, expected) in output.chunks_exact(channels).zip(old.chunks_exact(channels)) {
                for channel in 0..channels { if !cue || ![2, 3].contains(&channel) { assert_eq!(actual[channel], expected[channel]); } }
                if cue { assert!(actual[2].is_finite() && actual[3].is_finite()); }
            }
        }
        control.transition(&live.project, live.project.revision(), 0.02).unwrap();
        let mut elapsed = 0;
        for _ in 0..9 {
            let mut old = vec![0.0; channels * 128];
            let mut expected_next = vec![0.0; channels * 128];
            outgoing.process_interleaved(&mut old, channels);
            next.process_interleaved(&mut expected_next, channels);
            assert_eq!(test_alloc::measure(|| live.process_interleaved(&mut output, channels)), test_alloc::Counts::default());
            for ((actual, old), next) in output.chunks_exact(channels).zip(old.chunks_exact(channels)).zip(expected_next.chunks_exact(channels)) {
                let gain = (elapsed as f64 / 959.0).min(1.0) as f32;
                for channel in 0..channels { assert!((actual[channel] - (old[channel] * (1.0 - gain) + next[channel] * gain)).abs() < 1e-6, "routed channel {channel}, frame {elapsed}"); }
                elapsed += 1;
            }
        }
        assert_eq!(live.session.namespace, namespace);
        assert!(handle.applied().is_some());
        assert!(matches!(handle.retire(), Some(Ok(()))));
    }
}

#[test]
fn physical_input_preload_refusal_and_an_active_input_preserve_the_current_routes() {
    let with_input = || {
        use audio::routing::model::*;
        let mut prepared = constant(-0.3);
        let mut model = Model::default();
        model.next_id = 3;
        model.ports.push(Port { id: 2, alias: "Live physical input".into(), direction: Direction::Input, channels: vec![0, 1] });
        prepared.rt.routing = Some(Box::new(audio::routing::prepared::Prepared::new(Arc::new(model), &prepared.rt.session).unwrap()));
        prepared
    };
    let mut current = constant(0.2).rt;
    let owner = current.project.live_sets();
    let namespace = current.session.namespace;
    assert!(owner.stage(owner.reserve().unwrap(), with_input(), namespace, Arc::new(AtomicBool::new(false))).err().unwrap().contains("physical input"));
    assert!(!owner.busy());
    current = with_input().rt;
    let namespace = current.session.namespace;
    let owner = current.project.live_sets();
    let _control = owner.stage(owner.reserve().unwrap(), constant(0.2), namespace, Arc::new(AtomicBool::new(false))).unwrap();
    assert_eq!(test_alloc::measure(|| current.process(&mut [0.0; 256])), test_alloc::Counts::default());
    assert_eq!(current.session.namespace, namespace);
    assert!(owner.applied().is_none());
    assert!(matches!(owner.retire(), Some(Err(Error::Invalid(reason))) if reason.contains("physical input")));
}

fn delayed(value: f32) -> Prepared {
    use audio::routing::model::*;
    let mut prepared = constant(value);
    let model = Model { latency: Some(LatencyConfiguration { reports: vec![LatencyReport {
        group: Group::Deck(0), external_micros: 0, processing_micros: 20_000,
    }], ..Default::default() }), ..Default::default() };
    prepared.rt.routing = Some(Box::new(audio::routing::prepared::Prepared::new(Arc::new(model), &prepared.rt.session).unwrap()));
    prepared
}

#[test]
fn short_fade_refuses_unfilled_history_without_claiming_performance_and_cue_then_allows_it() {
    let mut live = constant(0.2).rt;
    let namespace = live.session.namespace;
    let incoming = delayed(-0.3);
    assert_eq!(priming(&incoming.rt), 960);
    let next_namespace = incoming.rt.session.namespace;
    let (handle, control) = queue(&mut live, incoming);
    let performance = live.performance.status();
    assert_eq!(control.priming_seconds(48_000), 0.02);
    assert!(control.transition(&live.project, live.project.revision(), 0.01).unwrap_err().contains("processing history"));
    assert!(control.ready());
    assert_eq!(live.performance.status(), performance);
    assert_eq!(live.session.namespace, namespace);
    assert!(handle.applied().is_none());
    control.preview.store(true, Ordering::Release);
    for _ in 0..12 {
        assert_eq!(test_alloc::measure(|| live.process_interleaved(&mut [0.0; 512], 4)), test_alloc::Counts::default());
    }
    assert_eq!(control.priming_seconds(48_000), 0.0);
    assert_eq!(live.session.namespace, namespace);
    control.transition(&live.project, live.project.revision(), 0.01).unwrap();
    for _ in 0..4 {
        assert_eq!(test_alloc::measure(|| live.process_interleaved(&mut [0.0; 512], 4)), test_alloc::Counts::default());
    }
    assert_eq!(live.session.namespace, next_namespace);
    assert!(handle.applied().is_some());
    assert!(matches!(handle.retire(), Some(Ok(()))));
    assert!(!live.performance.status().changing);
}

#[test]
fn delay_reset_after_fade_review_refuses_at_commit_and_keeps_the_current_graph() {
    let mut live = constant(0.2).rt;
    let namespace = live.session.namespace;
    let (handle, control) = queue(&mut live, delayed(-0.3));
    control.preview.store(true, Ordering::Release);
    for _ in 0..12 { live.process_interleaved(&mut [0.0; 512], 4); }
    assert_eq!(control.priming_seconds(48_000), 0.0);
    control.transition(&live.project, live.project.revision(), 0.01).unwrap();
    assert_eq!(test_alloc::measure(|| live.live_set.as_mut().unwrap().prepared.rt.routing.as_mut().unwrap().reset_latency()), test_alloc::Counts::default());
    assert_eq!(test_alloc::measure(|| live.process_interleaved(&mut [0.0; 512], 4)), test_alloc::Counts::default());
    assert_eq!(live.session.namespace, namespace);
    assert!(handle.applied().is_none());
    assert!(matches!(handle.retire(), Some(Err(Error::Invalid(reason))) if reason.contains("processing history")));
    assert!(!live.performance.status().changing);
}

#[test]
fn unavailable_connected_outputs_refuse_before_ready_and_again_after_width_changes() {
    for shrink_after_ready in [false, true] {
        let mut live = constant(0.2).rt;
        let namespace = live.session.namespace;
        let incoming = routed(-0.3, [0, 1], [4, 5]);
        let routes = incoming.rt.routing.as_ref().unwrap().model.clone();
        let handle = live.project.live_sets();
        let control = handle.stage(handle.reserve().unwrap(), incoming, namespace, Arc::new(AtomicBool::new(false))).unwrap();
        if shrink_after_ready {
            live.process_interleaved(&mut [0.0; 768], 6);
            assert!(control.ready());
            control.transition(&live.project, live.project.revision(), 0.01).unwrap();
        }
        assert_eq!(test_alloc::measure(|| live.process_interleaved(&mut [0.0; 512], 4)), test_alloc::Counts::default());
        assert_eq!(live.session.namespace, namespace);
        assert!(handle.applied().is_none());
        let stage = handle.0.returned.try_recv().unwrap();
        assert_eq!(*stage.prepared.rt.routing.as_ref().unwrap().model, *routes);
        assert!(handle.0.retired.try_send(stage).is_ok());
        assert!(matches!(handle.retire(), Some(Err(Error::Invalid(reason))) if reason.contains("unavailable next-set outputs")));
        assert!(!live.performance.status().changing);
    }
}

#[test]
fn unused_and_zero_gain_unavailable_aliases_do_not_block_a_valid_transition() {
    let mut live = constant(0.2).rt;
    let mut incoming = routed(-0.3, [0, 1], [4, 5]);
    let mut model = (*incoming.rt.routing.as_ref().unwrap().model).clone();
    for map in &mut model.connections.last_mut().unwrap().map { map.gain = 0.0; }
    model.next_id = 4;
    model.ports.push(audio::routing::model::Port { id: 3, alias: "Offline unused alias".into(), direction: Direction::Output, channels: vec![62, 63] });
    incoming.rt.routing = Some(Box::new(audio::routing::prepared::Prepared::new(Arc::new(model), &incoming.rt.session).unwrap()));
    let namespace = incoming.rt.session.namespace;
    let (handle, control) = queue(&mut live, incoming);
    control.transition(&live.project, live.project.revision(), 0.01).unwrap();
    for _ in 0..4 {
        assert_eq!(test_alloc::measure(|| live.process_interleaved(&mut [0.0; 512], 4)), test_alloc::Counts::default());
    }
    assert_eq!(live.session.namespace, namespace);
    assert!(handle.applied().is_some());
    assert!(matches!(handle.retire(), Some(Ok(()))));
}

#[test]
fn refusal_waits_one_callback_for_a_claimed_request_and_retires_its_permit_off_callback() {
    let mut live = constant(0.2).rt;
    let namespace = live.session.namespace;
    let handle = live.project.live_sets();
    let control = handle.stage(handle.reserve().unwrap(), routed(-0.3, [0, 1], [4, 5]), namespace,
        Arc::new(AtomicBool::new(false))).unwrap();
    live.process_interleaved(&mut [0.0; 768], 6);
    assert!(control.ready());
    let permit = live.project.performance().project_change().unwrap();
    control.phase.compare_exchange(READY, REQUESTED, Ordering::AcqRel, Ordering::Acquire).unwrap();
    assert_eq!(test_alloc::measure(|| live.process_interleaved(&mut [0.0; 512], 4)), test_alloc::Counts::default());
    assert_eq!(live.session.namespace, namespace);
    assert!(handle.retire().is_none());
    assert!(control.request.try_send(Transition { revision: live.project.revision(), frames: 480, _permit: permit }).is_ok());
    assert_eq!(test_alloc::measure(|| live.process_interleaved(&mut [0.0; 512], 4)), test_alloc::Counts::default());
    assert_eq!(live.session.namespace, namespace);
    assert!(handle.applied().is_none());
    assert!(matches!(handle.retire(), Some(Err(Error::Invalid(reason))) if reason.contains("unavailable next-set outputs")));
    assert!(!live.performance.status().changing);
}
