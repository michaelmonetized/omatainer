use super::*;
use crate::engine::DeckSnap;

fn lines(snapshot: &DeckSnap) -> Vec<([Pos2; 2], egui::Stroke)> {
    let context = egui::Context::default();
    let theme = Theme::default();
    let count = Arc::strong_count(&snapshot.peaks);
    let output = context.run(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(240.0, 480.0))),
            ..Default::default()
        },
        |context| {
            egui::CentralPanel::default().show(context, |ui| {
                vertical_wave(ui, &theme, snapshot, theme.accent, 120.0, 400.0, |_| {});
            });
        },
    );
    assert_eq!(Arc::strong_count(&snapshot.peaks), count);
    output
        .shapes
        .into_iter()
        .filter_map(|shape| match shape.shape {
            egui::Shape::LineSegment { points, stroke } => Some((points, stroke)),
            _ => None,
        })
        .collect()
}

#[test]
fn shared_waveform_paints_the_same_geometry_as_previously_copied_peaks() {
    let peaks: Vec<_> = (0..8192)
        .map(|i| {
            let phase = i as f32 / 8192.0;
            [
                phase,
                (phase * 31.0).sin().abs(),
                (phase * 13.0).cos().abs(),
            ]
        })
        .collect();
    let shared = Arc::new(peaks);
    for position in [0.0, 0.25, 0.5, 0.999] {
        let snapshot = DeckSnap {
            frames: 48_000.0 * 60.0,
            duration: 60.0,
            pos: position * 48_000.0 * 60.0,
            peaks: shared.clone(),
            ..DeckSnap::default()
        };
        // Reproduce the old worker's owned Vec recopy as an output oracle.
        let copied = DeckSnap {
            peaks: Arc::new((*shared).clone()),
            ..snapshot.clone()
        };
        assert!(!Arc::ptr_eq(&snapshot.peaks, &copied.peaks));
        let painted = lines(&snapshot);
        assert!(painted.len() > 100, "waveform was not painted");
        assert_eq!(painted, lines(&copied));
    }
}
