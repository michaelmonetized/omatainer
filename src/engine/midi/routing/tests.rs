use super::{packet::*, *};
fn endpoint(name: &str) -> Endpoint {
    Endpoint {
        name: name.into(),
        id: None,
    }
}
fn route() -> Route {
    Route {
        track: 2,
        inputs: vec![Input {
            port: endpoint("Keyboard"),
            channels: 1,
        }],
        output: Some(endpoint("Synth")),
        output_channel: Some(4),
        monitor: true,
        thru: true,
        filter: Filter::default(),
    }
}
#[test]
fn exact_ports_channels_types_and_feedback_are_validated() {
    let mut routing = Routing {
        enabled: true,
        routes: vec![route()],
    };
    assert!(routing.validate().is_ok());
    assert_eq!(routing.output_mask(), 4);
    assert!(routing.routes[0].inputs[0]
        .port
        .matches("Keyboard", "128:0"));
    assert!(!routing.routes[0].inputs[0]
        .port
        .matches("keyboard", "128:0"));
    routing.routes[0].inputs[0].port.id = Some("128:0".into());
    assert!(!routing.routes[0].inputs[0]
        .port
        .matches("Keyboard", "129:0"));
    routing.routes[0].output.as_mut().unwrap().id = Some("128:2".into());
    assert!(routing.validate().unwrap_err().contains("Feedback"));
    routing.routes[0].output.as_mut().unwrap().id = Some("129:2".into());
    assert!(routing.validate().is_ok());
    routing.routes.push(routing.routes[0].clone());
    assert!(routing.validate().is_err());
    routing.routes.pop();
    routing.routes[0].inputs[0].channels = 0;
    assert!(routing.validate().is_err());
    let mut filter = Filter::default();
    filter.cc = false;
    assert!(filter.accepts(&[0xb1, 0, 7]));
    assert!(!filter.accepts(&[0xb1, 1, 7]));
    assert!(!filter.accepts(&[0xf0, 0x7d, 1, 0xf7]));
    filter.sysex = true;
    assert!(filter.accepts(&[0xf0, 0x7d, 1, 0xf7]));
}
#[test]
fn full_message_widths_running_status_realtime_and_sysex_are_exact() {
    let bytes = [
        0x92, 60, 100, 61, 80, 0xf8, 0xc2, 9, 0xd2, 55, 0xe2, 0, 64, 0xf0, 0x7d, 1, 0xf8, 2, 0xf7,
    ];
    let out: Vec<_> = frames(&bytes).collect();
    let actual: Vec<_> = out
        .iter()
        .filter_map(|f| {
            if let Frame::Musical(p) = f {
                Some(p.bytes().to_vec())
            } else {
                None
            }
        })
        .collect();
    assert_eq!(
        actual,
        vec![
            vec![0x92, 60, 100],
            vec![0x92, 61, 80],
            vec![0xc2, 9],
            vec![0xd2, 55],
            vec![0xe2, 0, 64],
            vec![0xf0, 0x7d, 1, 2, 0xf7]
        ]
    );
    assert_eq!(
        out.iter()
            .filter(|f| matches!(f, Frame::Realtime(0xf8)))
            .count(),
        2
    );
    assert_eq!(
        Packet::new(&[0xc2, 9])
            .unwrap()
            .with_channel(Some(4))
            .bytes(),
        &[0xc4, 9]
    );
    assert_eq!(
        Packet::new(&[0xf0, 0x7d, 1, 0xf7])
            .unwrap()
            .with_channel(Some(4))
            .bytes(),
        &[0xf0, 0x7d, 1, 0xf7]
    );
    assert!(Packet::new(&[0x90, 60]).is_none());
    assert!(Packet::new(&[0xd0, 128]).is_none());
    assert!(Packet::new(&[0xf0, 0x7d, 0x90, 0xf7]).is_none());
    assert!(Packet::new(&[0xf0, 0, 0xf7]).is_none());
    assert!(Packet::new(&[0xf0, 0, 1, 2, 0xf7]).is_some());
    assert!(frames(&[0xf0, 0x7d, 1]).any(|f| matches!(f, Frame::Malformed)));
    assert!(frames(&[0x90, 60]).any(|f| matches!(f, Frame::Malformed)));
    assert!(frames(&[60, 100]).next().is_none());
    let mut max = vec![1; MAX_BYTES];
    max[0] = 0xf0;
    max[MAX_BYTES - 1] = 0xf7;
    assert!(Packet::new(&max).is_some());
    max.insert(1, 1);
    assert!(Packet::new(&max).is_none());
}
#[test]
fn profile_roundtrip_and_legacy_midi_presence_guard() {
    let mut prefs = crate::preferences::Preferences::defaults(std::path::Path::new("/tmp"));
    prefs.profiles.get_mut("Studio").unwrap().midi_routing = Routing {
        enabled: true,
        routes: vec![route()],
    };
    let bytes = serde_json::to_vec(&prefs).unwrap();
    let (decoded, migrated) = crate::preferences::storage::decode(&bytes).unwrap();
    assert_eq!(decoded, prefs);
    assert!(!migrated);
    for version in 2..=5 {
        let mut old = serde_json::to_value(&prefs).unwrap();
        old["version"] = version.into();
        for profile in old["profiles"].as_object_mut().unwrap().values_mut() {
            profile.as_object_mut().unwrap().remove("automation");
            profile.as_object_mut().unwrap().remove("workspaces");profile.as_object_mut().unwrap().remove("midi_learn");
            for field in ["locale", "contrast", "reduced_motion", "waveform_contrast", "level_contrast"] { profile["appearance"].as_object_mut().unwrap().remove(field); }
            profile["startup"].as_object_mut().unwrap().remove("session");
        }
        assert!(crate::preferences::storage::decode(&serde_json::to_vec(&old).unwrap()).is_err());
        for p in old["profiles"].as_object_mut().unwrap().values_mut() {
            p.as_object_mut().unwrap().remove("midi_routing");
            p["startup"].as_object_mut().unwrap().remove("session");
        }
        let (decoded, migrated) =
            crate::preferences::storage::decode(&serde_json::to_vec(&old).unwrap()).unwrap();
        assert!(migrated);
        assert_eq!(decoded.version, crate::preferences::VERSION);
        assert!(!decoded.current().unwrap().midi_routing.enabled);
    }
}
