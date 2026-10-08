use super::*;

/// Show the keys retained by currently playing clips.
/// Takes the native status area and coherent snapshot; reports agreement, explicit disagreement or incomplete context without changing any pitch or key.
pub(super) fn show(ui: &mut Ui, snapshot: &crate::engine::Snapshot) {
    let active=snapshot.active_scale;
    let text=if active.conflicted {
        Some("Playing clips use different keys. Each keeps its own key and pitches.".to_owned())
    } else if let Some(context)=active.context {
        let tonic=["C","C♯","D","D♯","E","F","F♯","G","G♯","A","A♯","B"][usize::from(context.tonic)];
        Some(format!("{} {}{}",tonic,context.scale.name(),if active.unspecified{" · some playing clips have no saved key"}else{""}))
    } else {None};
    if let Some(text)=text {let response=ui.label(&text);accessibility::status(ui,&response,&text);}
}
