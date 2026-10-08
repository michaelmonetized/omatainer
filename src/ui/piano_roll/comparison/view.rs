use super::*;
#[derive(Clone, Copy)]
pub(in crate::ui::piano_roll) enum Action {
    Add((u8, u16)),
    Focus(usize),
    Remove(usize),
    Select,
    Preview,
    Restore,
    Keep,
    Cancel,
}
fn owned(draft: &Draft) -> String {
    format!(
        "Track {} scene {} · {}",
        draft.baseline.track + 1,
        draft.baseline.scene + 1,
        draft.name
    )
}
fn checkbox(ui: &mut Ui, label: &str, value: &mut bool) {
    let response = ui.checkbox(value, label);
    accessibility::button(ui, &response, label, Some(*value));
    help::annotate(ui, &response, HelpControl::MidiNotes);
}
fn detail(ui: &mut Ui, draft: &Draft, offset: f64, tuning: f32, role: &str) {
    ui.label(format!(
        "{} · {} · {} notes, {} selected",
        owned(draft),
        role,
        draft.notes.len(),
        draft.selected.len()
    ));
    ui.label(format!("Saved key: {}", scale::description(draft.resolved_context())));
    ui.label(format!(
        "Source {:.6}–{:.6} beats · loop {:.6}–{:.6} · shared start {:.6} · A4 {:.3} Hz",
        draft.region.start,
        draft.region.end,
        draft.region.loop_start,
        draft.region.loop_end,
        offset,
        tuning
    ));
}
/// Show explicit ownership, permission and shared selection controls.
/// Takes the comparison, captured focus and current session limits; returns one intentional action without changing committed clips.
pub(in crate::ui::piano_roll) fn show(
    ui: &mut Ui,
    comparison: &mut Comparison,
    focus: &Draft,
    selected: (u8, u16),
    tracks: usize,
    scenes: usize,
) -> Option<Action> {
    let mut action = None;
    egui::CollapsingHeader::new("MIDI clip comparison").id_salt("midi-comparison").show(ui,|ui| {
        accessibility::scope(ui,"MIDI clip comparison",|ui| {
            detail(ui,focus,comparison.offset,comparison.tuning_hz,"Focused; ordinary editor controls change this clip");
            ui.add_enabled_ui(comparison.editing(focus),|ui| {
                ui.horizontal_wrapped(|ui| {
                    let mut track=f64::from(comparison.add_track)+1.0;
                    if number(ui,"Compare track",&mut track,1.0,tracks.max(1) as f64) {comparison.add_track=(track.round()-1.0) as u8;}
                    let mut scene=f64::from(comparison.add_scene)+1.0;
                    if number(ui,"Compare scene",&mut scene,1.0,scenes.max(1) as f64) {comparison.add_scene=(scene.round()-1.0) as u16;}
                    if button(ui,"Add comparison clip").clicked(){action=Some(Action::Add((comparison.add_track,comparison.add_scene)));}
                    if button(ui,"Add selected session clip").clicked(){action=Some(Action::Add(selected));}
                });
                number(ui,"Focused shared offset",&mut comparison.offset,-262144.0,262144.0);
                checkbox(ui,"Enable edits across explicitly enabled clips",&mut comparison.multi);
                for(index,peer)in comparison.clips.iter_mut().enumerate() {
                    let owner=(peer.draft.baseline.track,peer.draft.baseline.scene);
                    ui.push_id(owner,|ui| {
                        detail(ui,&peer.draft,peer.offset,peer.tuning_hz,
                            if comparison.multi&&peer.editable {"Enabled for shared tool actions"}else{"Protected ghost; focus to edit here"});
                        ui.horizontal_wrapped(|ui| {
                            if button(ui,&format!("Focus track {} scene {}",owner.0+1,owner.1+1)).clicked(){action=Some(Action::Focus(index));}
                            if button(ui,&format!("Remove comparison track {} scene {}",owner.0+1,owner.1+1)).clicked(){action=Some(Action::Remove(index));}
                            number(ui,&format!("Shared offset track {} scene {}",owner.0+1,owner.1+1),&mut peer.offset,-262144.0,262144.0);
                            ui.add_enabled_ui(comparison.multi,|ui|checkbox(ui,&format!("Enable track {} scene {} for shared edits",owner.0+1,owner.1+1),&mut peer.editable));
                        });
                    });
                }
                ui.separator();
                ui.label("Selection uses shared quarter-note beats. Protected ghost selections and notes stay unchanged.");
                let criteria=&mut comparison.criteria;
                ui.horizontal_wrapped(|ui| {
                    for(index,label)in ["Selection pitch minimum","Selection pitch maximum"].into_iter().enumerate() {
                        let mut next=f64::from(criteria.pitch[index]);if number(ui,label,&mut next,0.0,127.0){criteria.pitch[index]=next.round() as u8;}
                    }
                    number(ui,"Selection shared time minimum",&mut criteria.time[0],-524288.0,524288.0);
                    number(ui,"Selection shared time maximum",&mut criteria.time[1],-524288.0,524288.0);
                });
                ui.horizontal_wrapped(|ui| {
                    for(index,label)in ["Selection velocity minimum","Selection velocity maximum"].into_iter().enumerate() {
                        let mut next=f64::from(criteria.velocity[index]);if number(ui,label,&mut next,1.0,127.0){criteria.velocity[index]=next.round() as u8;}
                    }
                    checkbox(ui,"Include notes overlapping the shared time range",&mut criteria.overlap);
                    checkbox(ui,"Invert pitch, time and velocity match",&mut criteria.invert);
                    if button(ui,"Select enabled clips by criteria").clicked(){action=Some(Action::Select);}
                });
            });
            let pending=comparison.busy();
            ui.horizontal_wrapped(|ui| {
                let preview=ui.add_enabled(!pending&&!comparison.clips.is_empty()&&focus.tools.editing()&&comparison.clips.iter().all(|c|c.draft.tools.editing()),egui::Button::new("Preview shared tool on enabled clips"));
                accessibility::button(ui,&preview,"Preview shared tool on enabled clips",None);
                if preview.clicked(){action=Some(Action::Preview);}
                for(label,value)in [("Restore all clip previews",Action::Restore),("Keep all clip previews",Action::Keep)] {
                    let response=ui.add_enabled(!pending&&comparison.group.previewed(),egui::Button::new(label));
                    accessibility::button(ui,&response,label,None);if response.clicked(){action=Some(value);}
                }
                let cancel=ui.add_enabled(pending,egui::Button::new("Cancel pending comparison work"));
                accessibility::button(ui,&cancel,"Cancel pending comparison work",None);if cancel.clicked(){action=Some(Action::Cancel);}
            });
            for((track,scene),summary)in &comparison.group.summaries {
                ui.label(format!("Track {} scene {} · {} selected notes · phrase {:.6}–{:.6} beats → {:.6}–{:.6} · overlap {} → {} · maximum rounding {:.9} beats",
                    track+1,scene+1,summary.original.notes,summary.original.first,summary.original.end,summary.transformed.first,
                    summary.transformed.end,summary.original.maximum_overlap,summary.transformed.maximum_overlap,summary.maximum_rounding_beats));
            }
            ui.label("Shared previews use the settings in MIDI transformations. Each clip retains its own source timing and expression. Apply MIDI edit commits every changed captured clip as one Undo step, including changes made before switching focus.");
        });
    });
    action
}
