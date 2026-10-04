use super::*;
use crate::ui::grid_editor::tests::Gui;
use egui::accesskit::{Action, ActionData};

#[test]
fn native_review_cancel_apply_undo_and_active_refusal_keep_the_fader_separate() {
    let mut gui = Gui::new(48_000);
    let original_fader = gui.rt.decks[0].gain;
    let original_pcm = gui.rt.decks[0].audio.clone().unwrap();
    gui.app.open_track_gain(0);
    gui.frame(vec![]);
    gui.frame(vec![]);
    gui.click("Manual source gain");
    gui.action(
        "Manual source trim (dB)",
        Action::SetValue,
        Some(ActionData::NumericValue(-6.0)),
    );
    assert_eq!(gui.app.track_gain.as_ref().unwrap().manual_db, -6.0);
    gui.click("Cancel source gain review");
    assert!(gui.app.track_gain.is_none());
    assert_eq!(gui.rt.decks[0].source_gain.policy(), Policy::Off);
    gui.app.open_track_gain(0);
    gui.frame(vec![]);
    gui.frame(vec![]);
    gui.click("Auto source gain");
    let expected = Resolved::prepare(
        gui.app.track_gain.as_ref().unwrap().policy(),
        gui.app.track_gain.as_ref().unwrap().receipt.source_level(),
    )
    .unwrap();
    gui.click("Apply source gain");
    gui.frame(vec![]);
    assert!(gui.app.track_gain.as_ref().unwrap().pending.is_none());
    assert_eq!(gui.rt.decks[0].source_gain, expected);
    assert_eq!(gui.rt.decks[0].gain, original_fader);
    assert!(Arc::ptr_eq(
        &gui.rt.decks[0].audio.clone().unwrap(),
        &original_pcm
    ));
    assert_eq!(
        gui.app
            .track_gain
            .as_ref()
            .unwrap()
            .receipt
            .preparation()
            .unwrap()
            .1
            .source_gain,
        expected.policy()
    );
    gui.rt.apply(Command::Undo);
    assert_eq!(gui.rt.decks[0].source_gain.policy(), Policy::Off);
    gui.rt.apply(Command::Redo);
    assert_eq!(gui.rt.decks[0].source_gain, expected);
    let receipt = gui.app.track_gain.as_ref().unwrap().receipt.clone();
    gui.rt.decks[0].playing = true;
    let ack = GridEditAck::new();
    let next = Resolved::prepare(Policy::Manual { db: -12.0 }, receipt.source_level()).unwrap();
    let counts = crate::engine::test_alloc::measure(|| {
        gui.rt.apply(Command::DeckSourceGain {
            deck: 0,
            gain: next,
            receipt,
            ack: ack.clone(),
        })
    });
    assert_eq!(counts, crate::engine::test_alloc::Counts::default());
    assert_eq!(ack.state(), GridEditState::Rejected);
    assert_eq!(gui.rt.decks[0].source_gain, expected);
    gui.rt.apply(Command::Undo);
    assert_eq!(gui.rt.decks[0].source_gain, expected);
}

#[test]
fn changed_receipt_closes_native_review_and_rejects_queued_gain() {
    let mut gui = Gui::new(48_000);
    gui.app.open_track_gain(0);
    let receipt = gui.app.track_gain.as_ref().unwrap().receipt.clone();
    let gain = Resolved::prepare(Policy::Manual { db: -6.0 }, receipt.source_level()).unwrap();
    gui.rt.apply(Command::DeckUnload { deck: 0 });
    let ack = GridEditAck::new();
    gui.rt.apply(Command::DeckSourceGain {
        deck: 0,
        gain,
        receipt,
        ack: ack.clone(),
    });
    assert_eq!(ack.state(), GridEditState::Rejected);
    assert_eq!(gui.rt.decks[0].source_gain, Resolved::default());
    gui.rt.publish_for_test();
    gui.frame(vec![]);
    gui.frame(vec![]);
    assert!(gui.app.track_gain.is_none());
}

#[test]
fn native_file_review_saves_and_recalls_policy_through_the_catalog_owner() {
    for policy in [
        Policy::Manual { db: -6.0 },
        Policy::Auto {
            target_dbfs: -20.0,
            peak_dbfs: -4.0,
        },
    ] {
        let files = crate::engine::media_analysis::tests::Files::new();
        let source_file = files.0.join("Gain Native.flac");
        std::fs::write(
            &source_file,
            include_bytes!("../../../tests/fixtures/audio/tone.flac"),
        )
        .unwrap();
        let source = LibSource::File(source_file.clone());
        let fingerprint = FileFingerprint::read(&source_file).unwrap();
        let catalog_file = files.0.join("gain-catalog.json");
        let mut gui = Gui::new(48_000);
        gui.app.loader = Some(
            crate::engine::media_load::Loader::start_with_performance(
                gui.app.engine.cmd.performance().clone(),
            )
            .unwrap(),
        );
        gui.app.start_library_store(catalog_file.clone());
        settle(&mut gui, |gui| !gui.app.library_metadata.active());
        gui.app.load_file(0, source_file.clone(), "Gain Native");
        settle(&mut gui, |gui| {
            gui.app.loads[0]
                .as_ref()
                .and_then(|load| load.receipt.as_ref())
                .is_some_and(|receipt| {
                    receipt.state() == State::Current
                        && gui.app.snap.decks[0].receipt_key == receipt.snapshot_key()
                })
        });
        let fader = gui.rt.decks[0].gain;
        gui.app.open_track_gain(0);
        assert!(gui.app.track_gain.is_some(), "{}", gui.app.status);
        gui.frame(vec![]);
        gui.frame(vec![]);
        match policy {
            Policy::Manual { db } => {
                gui.click("Manual source gain");
                gui.action(
                    "Manual source trim (dB)",
                    Action::SetValue,
                    Some(ActionData::NumericValue(f64::from(db))),
                );
            }
            Policy::Auto {
                target_dbfs,
                peak_dbfs,
            } => {
                gui.click("Auto source gain");
                gui.action(
                    "Target RMS (dBFS)",
                    Action::SetValue,
                    Some(ActionData::NumericValue(f64::from(target_dbfs))),
                );
                gui.action(
                    "Source sample-peak limit (dBFS)",
                    Action::SetValue,
                    Some(ActionData::NumericValue(f64::from(peak_dbfs))),
                );
            }
            Policy::Off => unreachable!(),
        }
        assert_eq!(gui.app.track_gain.as_ref().unwrap().policy(), policy);
        gui.click("Apply source gain");
        settle(&mut gui, |_| {
            crate::library::read(&catalog_file)
                .ok()
                .is_some_and(|catalog| {
                    catalog
                        .version(&source, Some(fingerprint))
                        .is_some_and(|version| version.preparation.source_gain == policy)
                })
        });
        let expected = gui.rt.decks[0].source_gain;
        assert_eq!(expected.policy(), policy);
        assert_eq!(gui.rt.decks[0].gain, fader);
        gui.click("Cancel source gain review");
        gui.app.send(Command::DeckUnload { deck: 0 });
        gui.frame(vec![]);
        gui.frame(vec![]);
        gui.rt.set_sample_rate(96_000).unwrap();
        gui.app.load_file(0, source_file, "Gain Native");
        settle(&mut gui, |gui| {
            gui.rt.decks[0].source_gain.policy() == policy
                && gui.app.snap.decks[0].source_gain == policy
        });
        assert_eq!(gui.rt.decks[0].source_gain, expected);
        assert_eq!(gui.rt.decks[0].gain, fader);
        gui.app.library_analysis.open = true;
        gui.app.lib_filter = "Gain Native".into();
        gui.app.refresh_library_view();
        gui.frame(vec![]);
        gui.click("Analyze BPM");
        gui.click("Analyze duration");
        gui.click("Analyze waveform");
        gui.click("Force selected fields");
        gui.click("Analyze selected row");
        settle(&mut gui, |gui| !gui.app.library_analysis.busy());
        assert_eq!(gui.rt.decks[0].source_gain, expected);
        let saved = crate::library::read(&catalog_file).unwrap();
        let version = saved.version(&source, Some(fingerprint)).unwrap();
        assert_eq!(version.preparation.source_gain, policy);
        assert!(version.analysis.as_ref().unwrap().level.is_some());
    }
}

fn settle(gui: &mut Gui, predicate: impl Fn(&Gui) -> bool) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    loop {
        gui.frame(vec![]);
        if predicate(gui) {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "Gain workflow did not settle: {}",
            gui.app.status
        );
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
}
