use super::*;
fn fixture() -> (
    Endpoint,
    rtrb::Consumer<Box<Packet>>,
    rtrb::Producer<Box<Packet>>,
) {
    let class:Class=serde_json::from_value(serde_json::json!({"info":{"path":"/Fixture.vst3","name":"Fixture","vendor":"Omatainer tests","version":"1","category":"Fx","uid":"00000000000000000000000000000001","audio_inputs":1,"audio_outputs":1,"has_midi_input":true,"has_midi_output":false,"has_gui":false},"layout":{"inputs":[{"channel_count":2,"active":true}],"outputs":[{"channel_count":2,"active":true}]},"parameters":[],"latency":0,"tail":0})).unwrap();
    let shared = Arc::new(Shared {
        cancel: AtomicBool::new(false),
        fault: AtomicBool::new(false),
        error: Mutex::new(None),
        latency: AtomicU32::new(0),
        missed: AtomicU64::new(0),
        submitted: AtomicU64::new(0),
        editor: AtomicU8::new(0),
        editor_open: AtomicBool::new(false),
        editor_busy: AtomicBool::new(false),
        values: vec![],
        editor_error: Mutex::new(None),
    });
    let (requests, rx) = rtrb::RingBuffer::new(POOL);
    let (tx, responses) = rtrb::RingBuffer::new(POOL);
    let (jobs, _) = crossbeam_channel::bounded(8);
    let control = Control {
        shared,
        jobs,
        class: Arc::new(class),
    };
    let mut free = Vec::with_capacity(POOL);
    for _ in 0..POOL {
        free.push(Box::new(Packet::new()));
    }
    (
        Endpoint {
            latency: BLOCK as u32 * BRIDGE_BLOCKS as u32,
            prepared: false,
            output_delay: None,
            available: false,
            requests,
            responses,
            free,
            ready: std::array::from_fn(|_| None),
            current: None,
            output: [[0.; BLOCK]; MAX_CHANNELS],
            sequence: 0,
            offset: 0,
            pending_midi: [Midi::default(); 512],
            pending_midi_count: 0,
            held_notes: [[false;128];16],
            pending_parameters: [Parameter::default(); 512],
            pending_parameter_count: 0,
            control,
        },
        rx,
        tx,
    )
}
#[test]
fn fixed_bridge_delay_and_first_sample_events_are_allocation_free() {
    let (mut endpoint, mut requests, mut responses) = fixture();
    let counts = crate::engine::test_alloc::measure(|| {
        for sample in 0..BLOCK * 12 {
            if sample == 0 {
                assert!(endpoint.midi([0x90, 60, 127]));
                assert!(endpoint.parameter(7, 0.75));
            }
            let mut input = [0.; MAX_CHANNELS];
            input[0] = if sample == 0 { 1. } else { 0. };
            let output = endpoint.tick(input, Context::default());
            assert_eq!(
                output[0],
                if sample == BRIDGE_BLOCKS as usize * BLOCK {
                    1.
                } else {
                    0.
                }
            );
            if let Ok(mut packet) = requests.pop() {
                if packet.sequence == 0 {
                    assert_eq!(packet.midi_count, 1);
                    assert_eq!(packet.midi[0].offset, 0);
                    assert_eq!(packet.parameter_count, 1);
                    assert_eq!(packet.parameters[0].offset, 0);
                    assert_eq!(packet.parameters[0].id, 7);
                }
                packet.output = packet.input;
                assert!(responses.push(packet).is_ok());
            }
        }
    });
    assert_eq!(counts, crate::engine::test_alloc::Counts::default());
    assert_eq!(endpoint.control.missed_blocks(), 0);
    assert_eq!(
        endpoint.control.latency(),
        BLOCK as u32 * BRIDGE_BLOCKS as u32
    );
}
#[test]
fn late_workers_and_event_bounds_never_grow_callback_storage() {
    let (mut endpoint, _, _) = fixture();
    let counts = crate::engine::test_alloc::measure(|| {
        for _ in 0..512 {
            assert!(endpoint.midi([0x90, 60, 100]));
            assert!(endpoint.parameter(1, 0.5));
        }
        assert!(!endpoint.midi([0x90, 61, 100]));
        assert!(!endpoint.parameter(2, 0.5));
        assert!(!endpoint.parameter(1, f64::NAN));
        for _ in 0..BLOCK * 64 {
            assert_eq!(
                endpoint.tick([0.1; MAX_CHANNELS], Context::default()),
                [0.; MAX_CHANNELS]
            );
        }
    });
    assert_eq!(counts, crate::engine::test_alloc::Counts::default());
    assert!(endpoint.control.missed_blocks() > 0);
    assert!(endpoint.free.capacity() <= POOL);
}
#[test]
#[ignore = "requires explicitly selected freshly built native worker and private SDK fixture; no audio device"]
fn native_processor_bridge_preserves_delay_state_and_samples() {
    let fixture = PathBuf::from(std::env::var_os("OMATAINER_VST3_FIXTURES").unwrap())
        .join("vst3sdk-build/VST3/Release/again-sample-accurate.vst3");
    let exe = executable().unwrap();
    let mut probe = process::Process::start(&exe).unwrap();
    let binary = identify(&fixture).unwrap();
    let class = match probe
        .exchange(
            &Request::Probe {
                binary: binary.clone(),
            },
            &AtomicBool::new(false),
            Duration::from_secs(10),
        )
        .unwrap()
    {
        Response::Classes { classes } => classes[0].clone(),
        _ => panic!("wrong probe response"),
    };
    let parameter = class
        .parameters
        .iter()
        .find(|p| p.name == "Gain")
        .unwrap()
        .id;
    let saved = Saved {
        schema: 1,
        binary,
        class_id: class.info.uid,
        plugin_version: class.info.version,
        state_codec: "vst3-host-0.9-state".into(),
        state: vec![],
    };
    let mut endpoint = Endpoint::start(saved, 48000, &AtomicBool::new(false)).unwrap();
    let mut peak = 0.;
    for block in 0..12 {
        let started = Instant::now();
        if block == 0 {
            assert!(endpoint.parameter(parameter, 1.));
        }
        for offset in 0..BLOCK {
            let mut input = [0.; MAX_CHANNELS];
            input[0] = if block == 0 && offset == 0 { 0.25 } else { 0. };
            input[1] = input[0];
            let out = endpoint.tick(
                input,
                Context {
                    sample_position: (block * BLOCK) as i64,
                    ..Default::default()
                },
            );
            if block == BRIDGE_BLOCKS as usize && offset == 0 {
                assert_eq!(out[0], 0.25);
            }
            peak = f32::max(peak, out[0]);
        }
        let wait = Duration::from_secs_f64(BLOCK as f64 / 48000.);
        if let Some(wait) = wait.checked_sub(started.elapsed()) {
            std::thread::sleep(wait);
        }
    }
    assert_eq!(peak, 0.25);
    assert_eq!(endpoint.control.missed_blocks(), 0);
    assert!(endpoint.control.error().is_none());
    let saved = endpoint.control.snapshot(&AtomicBool::new(false)).unwrap();
    assert!(!saved.state.is_empty());
    assert!(endpoint.control.class.info.name.contains("Gain"));
}
