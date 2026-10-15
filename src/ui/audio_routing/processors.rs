use super::*;

impl Draft {
    /// Append an effect to one retained shared return.
    /// Takes a catalog processor, project namespace and bus identity; commits a validated draft chain atomically or leaves the draft unchanged.
    pub(super) fn attach_return(
        &mut self,
        mut plugin: crate::engine::audio::routing::plugins::Instance,
        namespace: [u64; 2],
        bus: u64,
    ) -> Result<(), String> {
        if namespace != self.namespace || !self.model.buses.iter().any(|b| b.id == bus) {
            return Err("Selected return changed before processor inspection".into());
        }
        if plugin.instrument
            || plugin.inputs.is_empty()
            || self.model.plugins.len() >= crate::engine::audio::routing::plugins::MAX_PLUGINS
            || self.model.next_id == u64::MAX
        {
            return Err(
                "Return effects require an audio input and available processor capacity".into(),
            );
        }
        let mut model = self.model.clone();
        model.order(&self.layout)?;
        let mut source = Group::Bus(bus);
        loop {
            let outgoing = model
                .links()
                .filter(|c| {
                    c.source.group == source
                        && c.source.tap
                            == if matches!(source, Group::Bus(_)) {
                                Tap::PreFx
                            } else {
                                Tap::PostFx
                            }
                        && c.map.iter().any(|m| m.gain != 0.)
                })
                .collect::<Vec<_>>();
            let owned = outgoing
                .iter()
                .filter_map(|c| {
                    model
                        .plugins
                        .iter()
                        .find(|p| c.destination == Group::Plugin(p.id) && p.return_bus == Some(bus))
                        .map(|p| p.id)
                })
                .collect::<std::collections::BTreeSet<_>>();
            if owned.is_empty() {
                break;
            }
            if owned.len() != 1
                || outgoing
                    .iter()
                    .any(|c| c.destination != Group::Plugin(*owned.first().unwrap()))
            {
                return Err("Return has parallel device paths; review its explicit routes before appending an effect".into());
            }
            source = Group::Plugin(*owned.first().unwrap());
        }
        let width = if let Group::Plugin(id)=source {
            usize::from(model.plugins.iter().find(|p|p.id==id).unwrap().outputs[0])
        } else {
            model.width(source, &self.layout).ok_or("Return processor source was removed")?
        };
        let input = usize::from(plugin.inputs[0]);
        if width > 2 && width != input {
            return Err("Wide return processors require matching primary input width".into());
        }
        let id = model.next_id;
        plugin.id = id;
        plugin.name = format!("{} {id}", plugin.name);
        plugin.midi_track = None;
        plugin.scene_track = None;
        plugin.return_bus = Some(bus);
        for route in model
            .connections
            .iter_mut()
            .chain(model.sends.iter_mut().map(|s| &mut s.route))
            .filter(|c| c.source.group == source && c.source.tap == Tap::PostMixer)
        {
            if route
                .map
                .iter()
                .any(|m| usize::from(m.source) >= width || m.source >= plugin.outputs[0])
            {
                return Err(
                    "Return processor primary outputs cannot preserve the existing channel routes; review auxiliary routes separately".into(),
                );
            }
            route.source.group = Group::Plugin(id);
        }
        let map = if input == 1 && width == 2 {
            vec![
                ChannelMap {
                    source: 0,
                    destination: 0,
                    gain: 0.5,
                },
                ChannelMap {
                    source: 1,
                    destination: 0,
                    gain: 0.5,
                },
            ]
        } else {
            (0..input.min(width.max(2)))
                .map(|c| ChannelMap {
                    source: if width == 1 { 0 } else { c as u8 },
                    destination: c as u8,
                    gain: 1.,
                })
                .collect()
        };
        model.connections.push(Connection {
            source: Source {
                group: source,
                tap: if matches!(source, Group::Bus(_)) {
                    Tap::PreFx
                } else {
                    Tap::PostFx
                },
            },
            destination: Group::Plugin(id),
            map,
        });
        model.plugins.push(plugin);
        model.next_id += 1;
        model.version = 3;
        model.order(&self.layout)?;
        self.model = model;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::audio::routing::plugins::Instance;

    fn effect() -> Instance {
        Instance {
            id: 0,
            name: "Return effect".into(),
            saved: crate::plugin_host::Saved {
                schema: 1,
                binary: crate::plugin_host::BinaryIdentity {
                    bundle: "/home/michael/missing/Contract.vst3".into(),
                    binary: "/home/michael/missing/Contract.so".into(),
                    sha256: "0".repeat(64),
                    arch: "aarch64".into(),
                },
                class_id: "0".repeat(32),
                plugin_version: "1.0".into(),
                state_codec: "vst3-host-0.9-state".into(),
                state: vec![],
            },
            inputs: vec![2, 1],
            outputs: vec![2, 1],
            midi_track: None,
            scene_track: None,
            return_bus: None,
            instrument: false,
            bypass: true,
            latency: 768,
            parameters: vec![],
            automation: vec![],
            unavailable: None,
        }
    }

    #[test]
    fn named_return_native_ownership_preserves_shared_chain_and_refuses_stale_or_lossy_edits() {
        let layout = Layout::legacy((0..8).map(|n| format!("Track {n}")), 8);
        let mut model = Model::default();
        model.next_id = 3;
        model.buses.push(Bus {
            id: 2,
            alias: "Shared return".into(),
            channels: 2,
            gain: 1.,
            mute: false,
            mixer: Default::default(),
        });
        model.connections.push(Connection {
            source: Source {
                group: Group::Bus(2),
                tap: Tap::PostMixer,
            },
            destination: Group::Main,
            map: vec![
                ChannelMap {
                    source: 0,
                    destination: 0,
                    gain: 0.5,
                },
                ChannelMap {
                    source: 1,
                    destination: 1,
                    gain: 0.25,
                },
            ],
        });
        let mut draft = Draft {
            namespace: layout.namespace,
            generation: layout.generation,
            revision: 0,
            rate: 48000,
            layout,
            enabled: true,
            model,
            original: None,
            controls: vec![],
        };
        let namespace = draft.namespace;
        draft.attach_return(effect(), namespace, 2).unwrap();
        draft.attach_return(effect(), namespace, 2).unwrap();
        assert_eq!(
            draft
                .model
                .plugins
                .iter()
                .map(|p| p.return_bus)
                .collect::<Vec<_>>(),
            [Some(2), Some(2)]
        );
        assert!(draft
            .model
            .plugins
            .iter()
            .all(|p| p.midi_track.is_none() && p.scene_track.is_none()));
        assert!(draft
            .model
            .connections
            .iter()
            .any(|r| r.source.group == Group::Bus(2) && r.destination == Group::Plugin(3)));
        assert!(draft
            .model
            .connections
            .iter()
            .any(|r| r.source.group == Group::Plugin(3) && r.destination == Group::Plugin(4)));
        let interstage=draft.model.connections.iter().find(|r|r.source.group==Group::Plugin(3)&&r.destination==Group::Plugin(4)).unwrap();
        assert_eq!(interstage.map.iter().map(|m|(m.source,m.destination)).collect::<Vec<_>>(),[(0,0),(1,1)]);
        assert!(draft.model.plugins.iter().all(|p|p.inputs==[2,1]&&p.outputs==[2,1]));
        let terminal = draft
            .model
            .connections
            .iter()
            .find(|r| r.source.group == Group::Plugin(4) && r.destination == Group::Main)
            .unwrap();
        assert_eq!(
            terminal.map.iter().map(|m| m.gain).collect::<Vec<_>>(),
            [0.5, 0.25]
        );
        assert_eq!(
            draft.model,
            serde_json::from_slice::<Model>(&serde_json::to_vec(&draft.model).unwrap()).unwrap()
        );
        let mut state=crate::engine::project::State::blank();
        state.session=Some(draft.layout.clone());state.routing=Some(std::sync::Arc::new(draft.model.clone()));
        let capture=crate::engine::project::Captured {state,media:vec![],revision:0,checkpoint:Default::default(),playback_receipts:[None,None]};
        let selection=crate::engine::audio::routing::selection::Selection {namespace,targets:vec![crate::engine::audio::routing::selection::Target::Return(2)],policy:Default::default()};
        let selected=selection.plan(&capture,Some(1)).unwrap();
        let observer=selected.connections.iter().find(|r|r.source.group==Group::Plugin(4)&&r.destination==Group::Output(1)).unwrap();
        assert_eq!(observer.map.iter().map(|m|(m.source,m.destination)).collect::<Vec<_>>(),[(0,0),(1,1)]);
        let before = draft.model.clone();
        assert!(draft.attach_return(effect(), [0; 2], 2).is_err());
        assert!(draft.attach_return(effect(), namespace, 99).is_err());
        let mut mono = effect();
        mono.outputs = vec![1];
        assert!(draft
            .attach_return(mono, namespace, 2)
            .unwrap_err()
            .contains("outputs"));
        assert_eq!(before, draft.model);
        draft.model.connections.push(Connection {source:Source {group:Group::Plugin(4),tap:Tap::PostMixer},destination:Group::Main,map:vec![ChannelMap {source:2,destination:0,gain:0.125}]});
        let auxiliary=draft.model.clone();
        assert!(draft.attach_return(effect(),namespace,2).unwrap_err().contains("auxiliary"));
        assert_eq!(draft.model,auxiliary);
        draft.model=before.clone();
        let mut invalid = before.clone();
        invalid.buses.clear();
        assert!(invalid
            .order(&draft.layout)
            .unwrap_err()
            .contains("ownership"));
        let mut wrong = before;
        wrong.plugins[0].midi_track = Some(draft.layout.tracks[0].id);
        assert!(wrong.order(&draft.layout).is_err());
    }
}
