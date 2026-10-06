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
