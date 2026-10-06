use super::test_support::Fixture;
use super::*;
use crate::engine::{
    midi::{Action, Binding, MidiMap, MsgKind, RelativeEncoding, RelativeSpec, UnmappedNotes},
    SubmissionError, SubmissionOutcome,
};
use std::sync::atomic::{AtomicUsize, Ordering};

struct Files(PathBuf);
impl Files {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "omatainer-browse-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn wave(&self, name: &str) -> PathBuf {
        let path = self.0.join(format!("{name}.wav"));
        let frames = 480u32;
        let mut bytes = b"RIFF".to_vec();
        bytes.extend((36 + frames * 2).to_le_bytes());
        bytes.extend(b"WAVEfmt ");
        bytes.extend(16u32.to_le_bytes());
        bytes.extend(1u16.to_le_bytes());
        bytes.extend(1u16.to_le_bytes());
        bytes.extend(48000u32.to_le_bytes());
        bytes.extend(96000u32.to_le_bytes());
        bytes.extend(2u16.to_le_bytes());
        bytes.extend(16u16.to_le_bytes());
        bytes.extend(b"data");
        bytes.extend((frames * 2).to_le_bytes());
        for _ in 0..frames {
            bytes.extend(1000i16.to_le_bytes());
        }
        std::fs::write(&path, bytes).unwrap();
        path
    }
}
impl Drop for Files {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn map() -> MidiMap {
    MidiMap {
        name: "Synthetic browse fixture (not a hardware profile)".into(),
        matchers: vec![],
        unmapped_notes: UnmappedNotes::Ignore,
        bindings: vec![
            Binding {
                kind: MsgKind::CcRel,
                ch: 0,
                data: 17,
                action: Action::Browse,
                deck: 0,
                extra: 0,
                relative: Some(RelativeSpec {
                    encoding: RelativeEncoding::OffsetBinary,
                    scale: 1.0,
                }),
             controls: None,
             pair_order: None,
            },
            Binding {
                kind: MsgKind::Note,
                ch: 0,
                data: 2,
                action: Action::DeckLoad,
                deck: 0,
                extra: 0,
                relative: None,
                controls: None,
                pair_order: None,
            },
        ],
    }
}
fn burst(f: &Fixture, bytes: &[u8]) {
    f.app
        .engine
        .midi
        .receive_map_for_test(&f.app.engine.cmd, 72, map(), "synthetic browse", bytes);
}
fn frame(f: &mut Fixture, ctx: &egui::Context, time: f64) -> egui::FullOutput {
    ctx.run(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1440.0, 900.0))),
            time: Some(time),
            ..Default::default()
        },
        |ctx| f.app.update_frame(ctx),
    )
}
fn fixture(count: usize) -> (Files, Fixture, egui::Context, Vec<PathBuf>) {
    let files = Files::new();
    let mut f = Fixture::new(256);
    let ctx = egui::Context::default();
    let mut paths = Vec::new();
    let mut items = Vec::new();
    for index in 0..count {
        let title = format!("visible {index:03}");
        let path = files.wave(&title);
        paths.push(path.clone());
        items.push(LibItem {
            title,
            artist: String::new(),
            bpm: Bpm::hint(120.0),
            fingerprint: FileFingerprint::read(&path),
            key: String::new(),
            length: None,
            last_play: None,
            source: LibSource::File(path),
        });
        items.push(LibItem {
            title: format!("hidden {index:03}"),
            source: LibSource::File(files.0.join(format!("hidden-{index}.wav"))),
            ..items.last().unwrap().clone()
        });
    }
    f.app.library = Arc::new(items);
    f.app.lib_filter = "visible".into();
    frame(&mut f, &ctx, 0.0);
    (files, f, ctx, paths)
}
fn selected(f: &mut Fixture) -> LibSource {
    f.app.selected_library_item().unwrap().source.clone()
}
fn finish_real_decode(f: &mut Fixture, path: &PathBuf) {
    if matches!(f.app.loads[0].as_ref().unwrap().phase, Phase::Loading) {
        f.poll_loads();
    }
    let command = f.rt.cmd_rx.try_recv().unwrap();
    assert!(
        matches!(&command,Command::DeckLoadRequested {media:Media::Decoded{audio,..},..} if audio.path==path.to_string_lossy())
    );
    f.rt.apply(command);
    f.app.poll_load_receipts();
    assert_eq!(
        f.rt.decks[0].audio.as_ref().unwrap().path,
        path.to_string_lossy()
    );
}

#[test]
fn one_midi_packet_browse_then_load_captures_new_visible_row_before_any_gui_frame() {
    let (_files, mut f, ctx, paths) = fixture(8);
    f.app.loader = Some(Loader::start().unwrap());
    burst(&f, &[0xb0, 17, 65, 0x90, 2, 127]);
    assert_eq!(f.app.lib_sel, 0);
    assert_eq!(f.app.engine.cmd.ui_request_stats().pending, 2);
    assert!(f.rt.cmd_rx.is_empty());
    frame(&mut f, &ctx, 1.0);
    assert_eq!(selected(&mut f), LibSource::File(paths[1].clone()));
    finish_real_decode(&mut f, &paths[1]);
}

#[test]
fn signed_relative_steps_clamp_within_filtered_rows_and_reveal_selection() {
    let (_files, mut f, ctx, paths) = fixture(80);
    burst(&f, &[0xb0, 17, 127]);
    frame(&mut f, &ctx, 1.0);
    assert_eq!(selected(&mut f), LibSource::File(paths[63].clone()));
    assert!(f.app.library_view.offset > 0.0);
    let top = f.app.lib_sel as f32 * f.app.library_view.stride;
    assert!(
        top + f.app.library_view.stride
            <= f.app.library_view.offset + f.app.library_view.height + 0.1
    );
    burst(&f, &[0xb0, 17, 127]);
    frame(&mut f, &ctx, 2.0);
    assert_eq!(selected(&mut f), LibSource::File(paths[79].clone()));
    burst(&f, &[0xb0, 17, 0, 0xb0, 17, 0]);
    frame(&mut f, &ctx, 3.0);
    assert_eq!(selected(&mut f), LibSource::File(paths[0].clone()));
    let accepted = f.app.engine.cmd.ui_request_stats().accepted;
    burst(&f, &[0xb0, 17, 64]);
    frame(&mut f, &ctx, 4.0);
    assert_eq!(
        f.app.engine.cmd.ui_request_stats().accepted,
        accepted,
        "neutral wheel report queued a browse"
    );
}

#[test]
fn partial_frame_drain_does_not_reset_worker_cursor_and_full_queue_does_not_advance_it() {
    use crate::engine::ui_requests::{CAPACITY, PER_FRAME};
    let (_files, mut f, ctx, paths) = fixture(40);
    f.app.loader = Some(Loader::start().unwrap());
    for _ in 0..CAPACITY {
        assert_eq!(
            f.app.engine.send(Command::Browse(1.0)),
            Ok(SubmissionOutcome::Accepted)
        );
    }
    assert_eq!(
        f.app.engine.send(Command::Browse(1.0)),
        Err(SubmissionError::UiFull)
    );
    frame(&mut f, &ctx, 1.0);
    assert_eq!(f.app.lib_sel, PER_FRAME);
    assert_eq!(
        f.app.engine.cmd.ui_request_stats().pending,
        CAPACITY - PER_FRAME
    );
    assert_eq!(
        f.app.engine.send(Command::DeckLoadSelected { deck: 0 }),
        Ok(SubmissionOutcome::Accepted)
    );
    frame(&mut f, &ctx, 2.0);
    assert_eq!(f.app.lib_sel, CAPACITY);
    frame(&mut f, &ctx, 3.0);
    finish_real_decode(&mut f, &paths[CAPACITY]);
    assert_eq!(selected(&mut f), LibSource::File(paths[CAPACITY].clone()));
}

#[test]
fn later_manual_filter_supersedes_browse_without_retargeting_already_captured_load() {
    let (_files, mut f, ctx, paths) = fixture(8);
    f.app.loader = Some(Loader::start().unwrap());
    burst(&f, &[0xb0, 17, 65, 0x90, 2, 127]);
    f.app.lib_filter = "visible 004".into();
    f.app.publish_library_selection();
    frame(&mut f, &ctx, 1.0);
    assert_eq!(selected(&mut f), LibSource::File(paths[4].clone()));
    finish_real_decode(&mut f, &paths[1]);
    assert_eq!(selected(&mut f), LibSource::File(paths[4].clone()));
}

#[test]
fn empty_retired_and_invalid_requests_reject_without_audio_commands_or_selection_substitution() {
    let (_files, mut f, _ctx, _paths) = fixture(4);
    for steps in [f32::NAN, f32::INFINITY, 0.5, f32::MAX] {
        assert_eq!(
            f.app.engine.send(Command::Browse(steps)),
            Err(SubmissionError::InvalidTarget)
        );
    }
    f.app.lib_filter = "nothing".into();
    f.app.publish_library_selection();
    for command in [Command::Browse(1.0), Command::DeckLoadSelected { deck: 0 }] {
        assert_eq!(
            f.app.engine.send(command),
            Err(SubmissionError::UncapturedSelection)
        );
    }
    f.app.lib_filter = "visible".into();
    f.app.publish_library_selection();
    f.app.library = Arc::new(builtin_crate_items());
    f.app.refresh_library_view(); // old weak adapter is now unresolved
    assert_eq!(
        f.app.engine.send(Command::Browse(1.0)),
        Err(SubmissionError::UncapturedSelection)
    );
    assert!(f.rt.cmd_rx.is_empty());
    assert_eq!(f.app.engine.cmd.ui_request_stats().pending, 0);
    let mut unsupported = map();
    unsupported.bindings[0].kind = MsgKind::Cc;
    unsupported.bindings[0].relative = None;
    assert!(unsupported
        .validate()
        .unwrap_err()
        .to_string()
        .contains("Browse requires"));
    let mut fractional = map();
    fractional.bindings[0].relative.as_mut().unwrap().scale = 0.35;
    assert!(fractional.validate().is_err());
}
