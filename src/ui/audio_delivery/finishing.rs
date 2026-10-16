use super::{accessibility, keyboard_authoring, number, number_is_valid, text};
use crate::audio_delivery::{finishing::Config, measurement::Policy, wav, Format};
use eframe::egui;

/// Validate broadcast metadata through the existing pure container writer.
/// Args: authored metadata and actual export format. Returns the existing validator's result without opening files.
fn validate_metadata(metadata: &wav::Metadata, format: Format) -> Result<(), String> {
    let container = match format {
        Format::Bwf24 => wav::Container::Bwf,
        Format::Rf64Float32 | Format::Rf64Pcm24 => wav::Container::Rf64,
        _ => return metadata.validate(),
    };
    wav::Settings {
        container,
        metadata: metadata.clone(),
        ..Default::default()
    }
    .header(2, 48000, format.encoding(), 0)
    .map(|_| ())
}

pub(super) fn controls(ui: &mut egui::Ui, config: &mut Config, format: Format) {
    let validation = keyboard_authoring::property(ui, "finishing-invalid-draft");
    let mut invalid = false;
    let response = ui.collapsing("Delivery metadata & final-file QC", |ui| {
        for (salt, name, value) in [
            ("title", "Title", &mut config.metadata.title),
            ("artist", "Artist", &mut config.metadata.artist),
            ("isrc", "ISRC", &mut config.metadata.isrc),
            ("description", "Description", &mut config.metadata.description),
            ("originator", "BWF originator", &mut config.metadata.originator),
            ("originator-reference", "BWF originator reference", &mut config.metadata.originator_reference),
            ("date", "BWF date YYYY-MM-DD", &mut config.metadata.date),
            ("time", "BWF time HH:MM:SS", &mut config.metadata.time),
        ] {
            ui.horizontal(|ui| {
                let response = text(ui, salt, name, value, false);
                let mut checked = wav::Metadata::default();
                *match salt {
                    "title" => &mut checked.title,
                    "artist" => &mut checked.artist,
                    "isrc" => &mut checked.isrc,
                    "description" => &mut checked.description,
                    "originator" => &mut checked.originator,
                    "originator-reference" => &mut checked.originator_reference,
                    "date" => &mut checked.date,
                    "time" => &mut checked.time,
                    _ => unreachable!(),
                } = value.clone();
                if let Err(error) = validate_metadata(&checked, format) {
                    keyboard_authoring::error(ui, &response, &error);
                }
            });
        }
        invalid |= !number_is_valid(&number(ui, "origin-samples", "BWF source-origin sample reference (samples)", &mut config.metadata.time_reference, None, None));
        ui.label("The selected range start is added to this 64-bit origin. Every aligned pass uses the same origin.");
        ui.horizontal(|ui| {
            ui.label("Final-file target policy");
            for (policy, name) in [(Policy::Report, "Report measurements"), (Policy::Warn, "Warn on target miss"), (Policy::Block, "Block publication on target miss")] {
                let response = keyboard_authoring::native(ui, keyboard_authoring::property(ui, ("qc-policy", policy as u8)), |ui| ui.selectable_value(&mut config.targets.policy, policy, name));
                accessibility::focus(ui, &response);
            }
        });
        let mut target = config.targets.integrated_lufs.is_some();
        let response = keyboard_authoring::native(ui, keyboard_authoring::property(ui, "check-integrated"), |ui| ui.checkbox(&mut target, "Check integrated loudness target"));
        accessibility::focus(ui, &response);
        if response.changed() { config.targets.integrated_lufs = target.then_some(-23.); }
        if let Some(value) = &mut config.targets.integrated_lufs {
            ui.horizontal(|ui| {
                invalid |= !number_is_valid(&number(ui, "integrated-lufs", "Integrated target (LUFS)", value, Some(-70.0..=0.0), Some(0.1)));
                invalid |= !number_is_valid(&number(ui, "loudness-tolerance", "Tolerance (LU)", &mut config.targets.loudness_tolerance, Some(0.1..=10.0), Some(0.1)));
            });
        }
        invalid |= !number_is_valid(&number(ui, "true-peak", "Maximum true peak (dBTP)", &mut config.targets.max_true_peak, Some(-30.0..=6.0), Some(0.1)));
        let mut lra = config.targets.max_lra.is_some();
        let response = keyboard_authoring::native(ui, keyboard_authoring::property(ui, "check-lra"), |ui| ui.checkbox(&mut lra, "Check maximum loudness range"));
        accessibility::focus(ui, &response);
        if response.changed() { config.targets.max_lra = lra.then_some(20.); }
        if let Some(value) = &mut config.targets.max_lra {
            invalid |= !number_is_valid(&number(ui, "maximum-lra", "Maximum LRA (LU)", value, Some(0.0..=70.0), Some(0.1)));
        }
        ui.label("LUFS, LRA, sample peak and true peak come from the final encoded file, including MP3. Targets never change gain. True-peak readings use the included meter; other meters can differ at abrupt file edges. Silence and unknown speaker layouts cannot qualify a loudness target. FFmpeg and ffprobe are required; the native meter is included.");
    });
    accessibility::focus(ui, &response.header_response);
    if response.body_returned.is_some() {
        ui.data_mut(|data| data.insert_temp(validation, invalid));
    } else if ui.data(|data| data.get_temp::<bool>(validation).unwrap_or(false))
        || validate_metadata(&config.metadata, format).is_err()
        || config.targets.validate().is_err()
    {
        keyboard_authoring::error(
            ui,
            &response.header_response,
            "Open metadata and final-file QC to resolve the invalid field",
        );
    }
}
