//! A bounded field preview from the catalog owner's inspected source version.
use super::*;

pub(super) struct Row {
    reference: SourceRef,
    title: String,
    bpm: Bpm,
    duration: Option<f64>,
    locks: crate::library::protection::Locks,
    lines: [String; 3],
    record: Option<crate::track_analysis::Record>,
}
impl Row {
    /// Describe fields a captured analysis request can refresh.
    /// Takes the captured title, exact source proof, inspected values and selected fields; returns bounded text without decoding or saving.
    pub fn new(
        title: &str,
        reference: &SourceRef,
        cached: &library_metadata::AnalysisCached,
        fields: Fields,
    ) -> Self {
        let metadata = &cached.metadata;
        let locks = cached.locks;
        let state = |selected: bool, needed: bool, locked: bool| {
            if !selected {
                "keep: unselected"
            } else if locked {
                "keep: locked"
            } else if !needed {
                "keep: verified cache"
            } else {
                "refresh analysis"
            }
        };
        let bpm = state(fields.bpm, cached.needed.bpm, locks.bpm);
        let duration = state(fields.duration, cached.needed.duration, locks.metadata);
        let waveform = state(fields.waveform, cached.needed.waveform, false);
        let old_duration = metadata
            .duration
            .map_or_else(|| "unknown".into(), |value| format!("{value:.3} seconds"));
        let old_waveform = cached
            .record
            .as_ref()
            .and_then(|record| record.waveform.as_ref())
            .map_or_else(
                || "none".into(),
                |value| format!("{} bins", value.value.bins),
            );
        Self {
            reference: reference.clone(),
            title: title.chars().take(512).collect(),
            bpm: metadata.bpm,
            duration: metadata.duration,
            locks,
            record: cached.record.clone(),
            lines: [
                format!("BPM: {} → {bpm}", metadata.bpm.cell()),
                format!("Duration: {old_duration} → {duration}"),
                format!("Waveform: {old_waveform} → {waveform}"),
            ],
        }
    }
    /// Display the old values and permitted replacements.
    /// Takes the UI and current catalog; reports stale or locked versions while leaving their saved values intact.
    pub fn show(&self, ui: &mut egui::Ui, catalog: &crate::library::Catalog) {
        ui.push_id((&self.reference.track, self.reference.fingerprint), |ui| {
            ui.add(egui::Label::new(&self.title).truncate());
            let current =
                catalog.track_for_version(&self.reference.source, Some(self.reference.fingerprint));
            let unchanged = current.is_some_and(|track| {
                track.id == self.reference.track
                    && track.locks == self.locks
                    && track.versions.iter().any(|version| {
                        version.fingerprint == Some(self.reference.fingerprint)
                            && version.metadata.bpm == self.bpm
                            && version.metadata.duration == self.duration
                            && version.analysis == self.record
                    })
            });
            if !unchanged {
                ui.colored_label(
                    ui.visuals().warn_fg_color,
                    "Preview is stale; preview this version again before analysis.",
                );
            }
            for line in &self.lines {
                ui.small(line);
            }
        });
    }
}
