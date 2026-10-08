use super::*;
use std::{
    io::{Read, Write},
    os::unix::{io::FromRawFd, net::UnixStream},
    sync::{Arc, Mutex},
};
use vst3_host::{BusDirection, MediaType, BusAudioBuffers, MidiChannel, MidiEvent, Plugin, PluginWindow, Vst3Host};

struct Instance {
    host: Vst3Host,
    plugin: Arc<Mutex<Plugin>>,
    saved: Saved,
    buffers: BusAudioBuffers,
    editor: Option<PluginWindow>,
    parameters: Vec<u32>,
}
fn class(plugin: &Plugin) -> Result<Class, String> {
    let result = Class {
        info: plugin.info().clone(),
        layout: plugin.audio_bus_layout().map_err(|e| e.to_string())?,
        parameters: plugin.get_parameters().map_err(|e| e.to_string())?,
        latency: plugin.latency_samples(),
        tail: plugin.tail_samples(),
    };
    result.validate()?;
    Ok(result)
}
fn host(rate: u32) -> Result<Vst3Host, String> {
    if !(8000..=384000).contains(&rate) {
        return Err("Unsupported plugin sample rate".into());
    }
    Vst3Host::builder()
        .sample_rate(rate as f64)
        .block_size(BLOCK)
        .build()
        .map_err(|e| e.to_string())
}
fn unchanged(binary: &BinaryIdentity) -> Result<(), String> {
    if &identify(&binary.bundle)? != binary {
        return Err("Plugin bundle changed; rescan and review the new installation".into());
    }
    Ok(())
}
fn handle(request: Request, instance: &mut Option<Instance>) -> Result<Response, String> {
    match request {
        Request::Paths { roots } => {
            let catalog = scanner::Catalog {
                roots,
                ..Default::default()
            };
            catalog.validate()?;
            Ok(Response::Paths {
                paths: scanner::candidates(
                    &catalog.roots,
                    &std::sync::atomic::AtomicBool::new(false),
                )?,
            })
        }
        Request::Inspect { path } => Ok(Response::Identity {
            binary: identify(&path)?,
        }),
        Request::Probe { binary } => {
            unchanged(&binary)?;
            let detailed =
                vst3_host::get_detailed_plugin_info(&binary.bundle).map_err(|e| e.to_string())?;
            let classes: Vec<_> = detailed
                .classes
                .into_iter()
                .filter(|c| c.category == "Audio Module Class")
                .collect();
            if classes.is_empty() || classes.len() > 64 {
                return Err(
                    "Plugin factory has no supported processor classes or exceeds 64 classes"
                        .into(),
                );
            }
            let mut host = host(48000)?;
            let mut reports = Vec::new();
            for c in classes {
                let p = host
                    .load_plugin_class(&binary.bundle, &c.class_id)
                    .map_err(|e| e.to_string())?;
                reports.push(class(&p)?);
                drop(p);
            }
            unchanged(&binary)?;
            Ok(Response::Classes { classes: reports })
        }
        Request::Load { saved, rate } => {
            saved.validate()?;
            unchanged(&saved.binary)?;
            let mut host = host(rate)?;
            let mut plugin = host
                .load_plugin_class(&saved.binary.bundle, &saved.class_id)
                .map_err(|e| e.to_string())?;
            if !plugin.info().uid.eq_ignore_ascii_case(&saved.class_id) {
                return Err(
                    "Plugin class identity was replaced; review compatibility before relinking"
                        .into(),
                );
            }
            if plugin.info().version != saved.plugin_version {
                return Err(
                    "Plugin version changed; review state compatibility before relinking".into(),
                );
            }
            if !saved.state.is_empty() {
                plugin.load_state(&saved.state).map_err(|e| e.to_string())?;
            }
            let layout = plugin.audio_bus_layout().map_err(|e|e.to_string())?;
            layout_valid(&layout)?;
            for (direction,buses) in [(BusDirection::Input,&layout.inputs),(BusDirection::Output,&layout.outputs)] {
                for (index,bus) in buses.iter().enumerate() { if !bus.active { plugin.set_bus_active(MediaType::Audio,direction,index as i32,true).map_err(|e|format!("Declared plugin bus could not be enabled: {e}"))?; } }
            }
            if plugin.info().has_midi_input { plugin.set_bus_active(MediaType::Event,BusDirection::Input,0,true).map_err(|e|e.to_string())?; }
            let report = class(&plugin)?;
            let buffers = plugin
                .create_bus_audio_buffers(BLOCK)
                .map_err(|e| e.to_string())?;
            plugin.start_processing().map_err(|e| e.to_string())?;
            *instance = Some(Instance {
                parameters: report.writable_parameters().map(|p|p.id).collect(),
                host,
                plugin: Arc::new(Mutex::new(plugin)),
                saved,
                buffers,
                editor: None,
            });
            Ok(Response::Loaded { class: report })
        }
        Request::Process { frame } => {
            frame.validate()?;
            let instance = instance.as_mut().ok_or("No plugin is loaded")?;
            let mut p = instance.plugin.lock().map_err(|e| e.to_string())?;
            let layout = p.audio_bus_layout().map_err(|e| e.to_string())?;
            layout_valid(&layout)?;
            if frame.inputs.len() != layout.inputs.len()
                || frame
                    .inputs
                    .iter()
                    .zip(&layout.inputs)
                    .any(|(b, l)| b.len() != l.channel_count)
            {
                return Err("Plugin input bus layout changed; rebuild the graph explicitly".into());
            }
            instance.buffers =
                BusAudioBuffers::new(&layout, frame.frames, instance.host.config().sample_rate);
            for (target, source) in instance.buffers.inputs.iter_mut().zip(frame.inputs) {
                target.channels = source;
            }
            p.set_tempo(frame.bpm).map_err(|e| e.to_string())?;
            p.set_time_signature(frame.signature[0], frame.signature[1])
                .map_err(|e| e.to_string())?;
            p.set_playing(frame.playing).map_err(|e| e.to_string())?;
            p.seek_transport(frame.sample_position, frame.beat)
                .map_err(|e| e.to_string())?;
            for (id, value, offset) in frame.parameters {
                p.set_parameter_at(id, value, offset)
                    .map_err(|e| e.to_string())?;
            }
            for (offset, b) in frame.midi {
                let channel = MidiChannel::from_index(b[0] & 15).ok_or("Invalid MIDI channel")?;
                let event = match b[0] & 0xf0 {
                    0x80 => MidiEvent::NoteOff {
                        channel,
                        note: b[1],
                        velocity: b[2],
                    },
                    0x90 if b[2] == 0 => MidiEvent::NoteOff {
                        channel,
                        note: b[1],
                        velocity: 0,
                    },
                    0x90 => MidiEvent::NoteOn {
                        channel,
                        note: b[1],
                        velocity: b[2],
                    },
                    0xa0 => MidiEvent::PolyAftertouch {
                        channel,
                        note: b[1],
                        pressure: b[2],
                    },
                    0xc0 => MidiEvent::ProgramChange {
                        channel,
                        program: b[1],
                    },
                    0xd0 => MidiEvent::ChannelAftertouch {
                        channel,
                        pressure: b[1],
                    },
                    0xb0 => MidiEvent::ControlChange {
                        channel,
                        controller: b[1],
                        value: b[2],
                    },
                    0xe0 => MidiEvent::PitchBend {
                        channel,
                        value: b[1] as u16 | ((b[2] as u16) << 7),
                    },
                    _ => return Err("MIDI event type is unsupported by this input path".into()),
                };
                p.send_midi_event_at(event, offset)
                    .map_err(|e| e.to_string())?;
            }
            p.process_bus_audio(&mut instance.buffers)
                .map_err(|e| e.to_string())?;
            let restart = p.take_restart_flags().bits() as u32;
            let outputs = instance
                .buffers
                .outputs
                .iter()
                .map(|b| b.channels.clone())
                .collect::<Vec<_>>();
            if outputs.iter().flatten().flatten().any(|v| !v.is_finite()) {
                return Err("Plugin returned non-finite audio; processor stopped".into());
            }
            let response = Response::Audio {
                outputs,
                latency: p.latency_samples(),
                tail: p.tail_samples(),
                restart,
                editor_open: instance.editor.as_ref().is_some_and(|window|!window.closed_by_user()),
                parameters: instance.parameters.iter().map(|id|p.get_parameter(*id).map(|value|(*id,value))).collect::<Result<Vec<_>,_>>().map_err(|e|e.to_string())?,
                midi: p.take_output_midi(),
            };
            drop(p);
            Ok(response)
        }
        Request::State => {
            let instance = instance.as_mut().ok_or("No plugin is loaded")?;
            let p = instance.plugin.lock().map_err(|e| e.to_string())?;
            let mut saved = instance.saved.clone();
            saved.state = p.save_state().map_err(|e| e.to_string())?;
            saved.validate()?;
            let parameters = instance.parameters.iter().map(|id|p.get_parameter(*id).map(|value|(*id,value))).collect::<Result<Vec<_>,_>>().map_err(|e|e.to_string())?;
            Ok(Response::State { saved, parameters })
        }
        Request::Editor { open } => {
            let instance = instance.as_mut().ok_or("No plugin is loaded")?;
            if open && instance.editor.is_none() {
                let mut window = PluginWindow::new(instance.plugin.clone());
                window.open().map_err(|e| e.to_string())?;
                instance.editor = Some(window);
            } else if !open {
                if let Some(mut e) = instance.editor.take() {
                    e.close();
                }
            }
            Ok(Response::Ok)
        }
        Request::Quit => {
            *instance = None;
            Ok(Response::Ok)
        }
    }
}
/// Serve the private plugin wire protocol.
/// Takes inherited socket descriptor three; returns on disconnect or invalid framing. Only this child loads native plugin code.
pub(crate) fn run() -> Result<(), String> {
    let mut socket = unsafe { UnixStream::from_raw_fd(3) };
    let mut reader = socket.try_clone().map_err(|e| e.to_string())?;
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    std::thread::Builder::new()
        .name("plugin-wire".into())
        .spawn(move || loop {
            let mut header = [0u8; 4];
            if reader.read_exact(&mut header).is_err() {
                break;
            }
            let size = u32::from_le_bytes(header) as usize;
            if size > MAX_MESSAGE {
                break;
            }
            let mut data = vec![0; size];
            if reader.read_exact(&mut data).is_err() {
                break;
            }
            let Ok(request) = serde_json::from_slice::<Request>(&data) else {
                break;
            };
            if tx.send(request).is_err() {
                break;
            }
        })
        .map_err(|e| e.to_string())?;
    let mut instance = None;
    loop {
        match rx.recv_timeout(std::time::Duration::from_millis(10)) {
            Ok(request) => {
                let quit = matches!(request, Request::Quit);
                let response = handle(request, &mut instance)
                    .unwrap_or_else(|message| Response::Error { message });
                let data = serde_json::to_vec(&response).map_err(|e| e.to_string())?;
                if data.len() > MAX_MESSAGE {
                    return Err("Plugin response exceeds its limit".into());
                }
                socket
                    .write_all(&(data.len() as u32).to_le_bytes())
                    .and_then(|_| socket.write_all(&data))
                    .map_err(|e| e.to_string())?;
                if quit {
                    return Ok(());
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return Ok(()),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
        }
        if let Some(instance) = instance.as_mut() {
            instance.plugin.lock().map_err(|e| e.to_string())?.service_run_loop();
            if let Some(editor) = &mut instance.editor {
                editor
                    .service_platform_events()
                    .map_err(|e| e.to_string())?;
                if editor.closed_by_user() {
                    instance.editor = None;
                }
            }
        }
    }
}
