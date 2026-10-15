use super::*;
use crate::engine::audio::routing::{
    model::Group,
    speakers::{Layout, OutputContract},
};

/// Review the saved output and recording aliases.
/// Takes one immutable capture; returns native encoded contracts without guessing speaker identity for wide aliases.
pub(super) fn inspect(
    capture: &crate::engine::project::Captured,
) -> Result<(Vec<OutputSource>, Vec<RecordSource>), String> {
    let Some(model) = &capture.state.routing else {
        return Ok((
            Vec::new(),
            vec![RecordSource {
                id: live::STANDARD_OUTPUT,
                name: "Standard master output".into(),
                channels: Some(vec![0, 1]),
                width: 2,
                contract: Some(OutputContract::stereo(vec![0, 1], 2)?),
            }],
        ));
    };
    let mut outputs = Vec::new();
    let mut records = Vec::new();
    for port in model.ports.iter().filter(|p| {
        matches!(p.direction, Direction::Output | Direction::Record)
            && (1..=26).contains(&p.channels.len())
            && model.monitor_output != Some(p.id)
    }) {
        let output = port.direction == Direction::Output;
        let group = if output {
            Group::Output(port.id)
        } else {
            Group::Record(port.id)
        };
        let width = port.channels.len() as u16;
        let contract = model
            .speaker_layout(group)
            .map(|_| model.channel_contract(group, width))
            .transpose()?;
        if output {
            if let Some(contract) = &contract {
                outputs.push(OutputSource {
                    id: port.id,
                    name: port.alias.clone(),
                    contract: contract.clone(),
                });
            }
        }
        records.push(RecordSource {
            id: port.id,
            name: format!(
                "{} · {}",
                port.alias,
                if output {
                    "final output"
                } else {
                    "raw routed source"
                }
            ),
            channels: output.then(|| port.channels.clone()),
            width,
            contract,
        });
    }
    Ok((outputs, records))
}

/// Choose a reviewed program alias.
/// Takes the native controls, export request and immutable plan; selecting another alias adopts its exact native encoded width.
pub(super) fn output(ui: &mut egui::Ui, request: &mut Export, plan: &Plan) {
    if !plan.routed {
        request.output_alias = None;
        ui.label(tr!("Standard stereo master output"));
        return;
    }
    let label = ui.label("Program output");
    egui::ComboBox::from_id_salt("audio-export-alias")
        .selected_text(
            plan.outputs
                .iter()
                .find(|s| Some(s.id) == request.output_alias)
                .map_or("Choose program output", |s| s.name.as_str()),
        )
        .show_ui(ui, |ui| {
            for source in &plan.outputs {
                if ui
                    .selectable_value(&mut request.output_alias, Some(source.id), &source.name)
                    .changed()
                {
                    request.options.channels = source.contract.channel_count;
                }
            }
        })
        .response
        .labelled_by(label.id);
    ui.label("Wide program outputs require a saved speaker declaration in Audio routing.");
}

/// Choose an encoded width supported by the reviewed alias.
/// Takes the request and plan; preserves explicit mono/stereo folds and offers the actual saved native bed without creating another layout model.
pub(super) fn width(ui: &mut egui::Ui, request: &mut Export, plan: &Plan) {
    ui.selectable_value(&mut request.options.channels, 1, tr!("Mono"));
    ui.selectable_value(&mut request.options.channels, 2, tr!("Stereo"));
    if let Some(source) = plan
        .outputs
        .iter()
        .find(|s| Some(s.id) == request.output_alias)
        .filter(|s| s.contract.channel_count > 2)
    {
        ui.selectable_value(
            &mut request.options.channels,
            source.contract.channel_count,
            Layout::ALL
                .iter()
                .find(|layout| layout.speakers() == source.contract.speakers.as_slice())
                .map_or("Saved speaker order", |layout| layout.label()),
        );
    }
}
