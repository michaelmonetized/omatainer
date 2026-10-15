use crate::audio_delivery::{finishing::Config, measurement::Policy};
use eframe::egui;

pub(super) fn controls(ui: &mut egui::Ui, config: &mut Config) {
    ui.collapsing("Delivery metadata & final-file QC", |ui| {
        for (label,value) in [("Title",&mut config.metadata.title),("Artist",&mut config.metadata.artist),("ISRC",&mut config.metadata.isrc),("Description",&mut config.metadata.description),("BWF originator",&mut config.metadata.originator),("BWF originator reference",&mut config.metadata.originator_reference),("BWF date YYYY-MM-DD",&mut config.metadata.date),("BWF time HH:MM:SS",&mut config.metadata.time)] {
            ui.horizontal(|ui|{let label=ui.label(label);ui.text_edit_singleline(value).labelled_by(label.id);});
        }
        ui.horizontal(|ui|{let label=ui.label("BWF source-origin sample reference");ui.add(egui::DragValue::new(&mut config.metadata.time_reference)).labelled_by(label.id);});
        ui.label("The selected range start is added to this 64-bit origin. Every aligned pass uses the same origin.");
        ui.horizontal(|ui|{ui.label("Final-file target policy");for (policy,label) in [(Policy::Report,"Report measurements"),(Policy::Warn,"Warn on target miss"),(Policy::Block,"Block publication on target miss")]{ui.selectable_value(&mut config.targets.policy,policy,label);}});
        let mut target=config.targets.integrated_lufs.is_some();if ui.checkbox(&mut target,"Check integrated loudness target").changed(){config.targets.integrated_lufs=target.then_some(-23.);}
        if let Some(value)=&mut config.targets.integrated_lufs {ui.horizontal(|ui|{let label=ui.label("Integrated target LUFS");ui.add(egui::DragValue::new(value).range(-70.0..=0.0).speed(0.1)).labelled_by(label.id);let label=ui.label("Tolerance LU");ui.add(egui::DragValue::new(&mut config.targets.loudness_tolerance).range(0.1..=10.0).speed(0.1)).labelled_by(label.id);});}
        ui.horizontal(|ui|{let label=ui.label("Maximum true peak dBTP");ui.add(egui::DragValue::new(&mut config.targets.max_true_peak).range(-30.0..=6.0).speed(0.1)).labelled_by(label.id);});
        let mut lra=config.targets.max_lra.is_some();if ui.checkbox(&mut lra,"Check maximum loudness range").changed(){config.targets.max_lra=lra.then_some(20.);}
        if let Some(value)=&mut config.targets.max_lra {ui.horizontal(|ui|{let label=ui.label("Maximum LRA LU");ui.add(egui::DragValue::new(value).range(0.0..=70.0).speed(0.1)).labelled_by(label.id);});}
        ui.label("LUFS, LRA, sample peak and true peak come from the final encoded file, including MP3. Targets never change gain. Silence and unknown speaker layouts cannot qualify a loudness target. FFmpeg and ffprobe are required; the native meter is included.");
    });
}
