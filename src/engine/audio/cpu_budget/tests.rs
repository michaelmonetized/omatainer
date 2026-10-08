use super::*;
use std::{
    process::Command,
    time::{Duration, Instant},
};

#[test]
fn owned_callback_registration_is_bounded_reusable_and_allocation_free() {
    let mut retained_guard = None;
    let counts = crate::engine::test_alloc::measure(|| {
        let mut guard = Guard::new();
        assert!(!guard.exceeded());
        for _ in 0..4096 {
            assert!(!guard.exceeded());
        }
        retained_guard = Some(guard);
    });
    assert_eq!(counts, crate::engine::test_alloc::Counts::default());
    let mut guard = retained_guard.unwrap();
    let slot = guard.slot.unwrap();
    CALLBACKS[slot].exceeded.store(true, Ordering::Release);
    assert!(guard.exceeded());
    assert!(guard.cpu_exhausted());
    drop(guard);
    assert_eq!(CALLBACKS[slot].tid.load(Ordering::Acquire), 0);
    let mut retained = Vec::new();
    for _ in 0..SLOTS {
        let mut guard = Guard::new();
        if !guard.exceeded() {
            retained.push(guard);
        }
    }
    let mut refused = Guard::new();
    assert!(refused.exceeded());
    assert!(refused.slot.is_none());
    assert!(!refused.cpu_exhausted());
    drop(retained);
    let mut reused = Guard::new();
    assert!(!reused.exceeded());
}

static FORWARDED: AtomicUsize = AtomicUsize::new(0);
pub(super) static SIGNAL_TID: AtomicI32 = AtomicI32::new(0);
extern "C" fn previous(_: i32, _: *mut libc::siginfo_t, _: *mut libc::c_void) {
    FORWARDED.fetch_add(1, Ordering::Relaxed);
}

#[test]
#[ignore = "Requires Linux rtkit and OMATAINER_CPU_BUDGET_DIR under /home; opens no audio or MIDI device"]
fn native_kernel_cpu_limit_demotes_only_owned_audio_threads() {
    use std::os::unix::process::ExitStatusExt;
    if let Ok(scenario) = std::env::var("OMATAINER_CPU_BUDGET_CHILD") {
        unsafe {
            libc::prctl(libc::PR_SET_DUMPABLE, 0);
        }
        if scenario == "previous" {
            let mut action: libc::sigaction = unsafe { std::mem::zeroed() };
            action.sa_sigaction = previous as *const () as usize;
            action.sa_flags = libc::SA_SIGINFO;
            assert_eq!(
                unsafe { libc::sigaction(libc::SIGXCPU, &action, std::ptr::null_mut()) },
                0
            );
        }
        install().unwrap();
        let mut guard = (scenario != "foreign").then(Guard::new);
        if let Some(guard) = &mut guard {
            assert!(!guard.exceeded());
        }
        if !matches!(scenario.as_str(), "budget" | "budget-blocked" | "foreign") {
            assert_eq!(unsafe { libc::raise(libc::SIGXCPU) }, 0);
            assert_eq!(scenario, "previous", "default SIGXCPU returned");
            assert_eq!(FORWARDED.load(Ordering::Relaxed), 1);
            assert!(!guard.as_mut().unwrap().exceeded());
            return;
        }
        if scenario == "budget-blocked" {
            let mut mask: libc::sigset_t = unsafe { std::mem::zeroed() };
            unsafe {
                libc::sigemptyset(&mut mask);
                libc::sigaddset(&mut mask, libc::SIGXCPU);
            }
            assert_eq!(
                unsafe { libc::pthread_sigmask(libc::SIG_BLOCK, &mask, std::ptr::null_mut()) },
                0
            );
        }
        let limit = libc::rlimit {
            rlim_cur: 5804,
            rlim_max: 200000,
        };
        assert_eq!(unsafe { libc::setrlimit(libc::RLIMIT_RTTIME, &limit) }, 0);
        let tid = unsafe { libc::syscall(libc::SYS_gettid) };
        let promoted = Command::new("busctl")
            .args([
                "--system",
                "call",
                "org.freedesktop.RealtimeKit1",
                "/org/freedesktop/RealtimeKit1",
                "org.freedesktop.RealtimeKit1",
                "MakeThreadRealtimeWithPID",
                "ttu",
                &std::process::id().to_string(),
                &tid.to_string(),
                "10",
            ])
            .status()
            .unwrap();
        assert!(promoted.success(), "rtkit promotion refused");
        let initial_flags = unsafe { libc::sched_getscheduler(0) };
        let initial_policy = initial_flags & !libc::SCHED_RESET_ON_FORK;
        assert!(matches!(initial_policy, libc::SCHED_FIFO | libc::SCHED_RR));
        let cpu_before = crate::engine::audio_metrics::thread_cpu_ns().unwrap();
        let deadline = Instant::now() + Duration::from_millis(500);
        while !guard.as_mut().is_some_and(|guard| guard.exceeded()) {
            assert!(Instant::now() < deadline, "kernel budget was not enforced");
            std::hint::black_box(std::hint::black_box(1.2345_f64).sin());
        }
        let cpu_ns = crate::engine::audio_metrics::thread_cpu_ns().unwrap() - cpu_before;
        let recovered_flags = unsafe { libc::sched_getscheduler(0) };
        let recovered_policy = recovered_flags & !libc::SCHED_RESET_ON_FORK;
        assert_eq!(recovered_policy, libc::SCHED_OTHER);
        assert_eq!(
            recovered_flags & libc::SCHED_RESET_ON_FORK,
            initial_flags & libc::SCHED_RESET_ON_FORK
        );
        let mut raised: libc::rlimit = unsafe { std::mem::zeroed() };
        assert_eq!(
            unsafe { libc::getrlimit(libc::RLIMIT_RTTIME, &mut raised) },
            0
        );
        assert_eq!(raised.rlim_cur, 1_005_804);
        assert_eq!(raised.rlim_max, 200_000);
        assert!(cpu_ns > 0);
        let signal_tid = SIGNAL_TID.load(Ordering::Acquire);
        assert_ne!(signal_tid, 0);
        if scenario == "budget-blocked" {
            assert_ne!(signal_tid as i64, tid as i64);
        }
        let directory =
            std::path::PathBuf::from(std::env::var_os("OMATAINER_CPU_BUDGET_DIR").unwrap());
        assert!(directory.starts_with("/home"));
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(
            directory.join(format!("kernel-{scenario}.json")),
            serde_json::to_vec_pretty(&serde_json::json!({
                "pid":std::process::id(),"tid":tid,"signal_tid":signal_tid,"signal_blocked_on_owned_thread":scenario=="budget-blocked","soft_limit_us":5804,"hard_limit_us":200000,
                "cpu_ns_after_promotion_return":cpu_ns,"kernel_raised_soft_limit_us":raised.rlim_cur,"initial_policy":initial_policy,"recovered_policy":recovered_policy,"reset_on_fork_preserved":true,
                "guard_exceeded":guard.as_mut().unwrap().exceeded(),"physical_devices_opened":false,
            }))
            .unwrap(),
        )
        .unwrap();
        return;
    }
    let directory = std::path::PathBuf::from(
        std::env::var_os("OMATAINER_CPU_BUDGET_DIR").expect("private qualification directory"),
    );
    assert!(directory.starts_with("/home"));
    std::fs::create_dir_all(&directory).unwrap();
    let selector = "engine::audio::cpu_budget::tests::native_kernel_cpu_limit_demotes_only_owned_audio_threads";
    for scenario in ["budget", "budget-blocked", "previous", "default", "foreign"] {
        let log = std::fs::File::create(directory.join(format!("{scenario}.log"))).unwrap();
        let status = Command::new("timeout")
            .args(["--signal=TERM", "--kill-after=2", "15"])
            .arg(std::env::current_exe().unwrap())
            .args([
                selector,
                "--exact",
                "--ignored",
                "--nocapture",
                "--test-threads=1",
            ])
            .env("OMATAINER_CPU_BUDGET_CHILD", scenario)
            .stdout(log.try_clone().unwrap())
            .stderr(log)
            .status()
            .unwrap();
        if matches!(scenario, "default" | "foreign") {
            assert_eq!(status.signal(), Some(libc::SIGXCPU));
        } else {
            assert!(status.success(), "{scenario}: {status}");
        }
    }
}

struct BudgetBackend {
    native: super::super::Native,
    inject: bool,
}
impl super::super::owner::Backend for BudgetBackend {
    type Stream = super::super::NativeStream;
    fn select(
        &mut self,
        settings: &crate::preferences::Audio,
    ) -> Result<super::super::config::Plan, String> {
        self.native.select(settings)
    }
    fn open(
        &mut self,
        plan: &super::super::config::Plan,
        mut callback: super::super::OutputCallback,
        fault: std::sync::Arc<AtomicBool>,
        identity: Option<&str>,
    ) -> Result<Self::Stream, String> {
        if std::mem::replace(&mut self.inject, false) {
            callback.cpu_stall_once = Some((8, Duration::from_millis(30)));
        }
        self.native.open(plan, callback, fault, identity)
    }
    fn identity(&mut self, plan: &super::super::config::Plan) -> Option<String> {
        self.native.identity(plan)
    }
    fn reconnect(
        &mut self,
        target: &super::super::recovery::Target,
    ) -> Result<super::super::config::Plan, String> {
        self.native.reconnect(target)
    }
    fn play(&mut self, stream: &Self::Stream) -> Result<(), String> {
        self.native.play(stream)
    }
    fn maintain(&mut self, stream: &Self::Stream) -> Result<Option<u32>, String> {
        self.native.maintain(stream)
    }
    fn calibrate(
        &mut self,
        request: &super::super::calibration::Request,
        cancel: &AtomicBool,
        stopped: &std::sync::Arc<AtomicBool>,
    ) -> Result<super::super::calibration::Measurement, String> {
        self.native.calibrate(request, cancel, stopped)
    }
}

#[test]
#[ignore = "Requires exclusive NS7 master XLR outputs wired to Peavey USB return and OMATAINER_LOOPBACK_DIR under /home; emits quiet -48 dBFS probes"]
fn native_ns7_master_to_peavey_usb_return_channels() {
    use super::super::{calibration, config};
    let directory = std::path::PathBuf::from(std::env::var_os("OMATAINER_LOOPBACK_DIR").unwrap());
    assert!(directory.starts_with("/home"));
    std::fs::create_dir_all(&directory).unwrap();
    let level_db = std::env::var("OMATAINER_LOOPBACK_LEVEL_DBFS")
        .map(|value| value.parse::<f32>().expect("numeric probe level"))
        .unwrap_or(-48.0);
    assert!(level_db.is_finite() && (-60.0..=-24.0).contains(&level_db));
    let output_channels = std::env::var("OMATAINER_LOOPBACK_OUTPUT_CHANNELS")
        .map(|value| value.parse::<u16>().expect("numeric channel count"))
        .unwrap_or(2);
    assert!(matches!(output_channels, 2 | 4));
    let output = config::Plan {
        backend: "ALSA".into(),
        graph: Default::default(),
        device: "hw:CARD=NS7,DEV=0".into(),
        channels: 4,
        rate: 44100,
        format: cpal::SampleFormat::I32,
        buffer: Some(512),
        warning: None,
    };
    let input = config::Plan {
        backend: "ALSA".into(),
        graph: Default::default(),
        device: "hw:CARD=CODEC,DEV=0".into(),
        channels: 2,
        rate: 44100,
        format: cpal::SampleFormat::I16,
        buffer: Some(512),
        warning: None,
    };
    let mut pairs = Vec::new();
    let mut found = vec![false; output_channels as usize];
    for output_channel in 0..output_channels {
        for input_channel in 0..2 {
            let request = calibration::Request {
                profile: "NS7 XLR -> Peavey 8 channel 6 -> USB Audio CODEC".into(),
                input: input.clone(),
                output: output.clone(),
                input_channel,
                output_channel,
                level_db,
            };
            let result = calibration::native::run(
                &request,
                &AtomicBool::new(false),
                std::sync::Arc::new(AtomicBool::new(false)),
            );
            let row = match result {
                Ok(measured) => {
                    found[output_channel as usize] = true;
                    serde_json::json!({"output_channel":output_channel,"input_channel":input_channel,"status":"matched","host_return_ns":measured.host_return_ns,"minimum_ns":measured.minimum_ns,"maximum_ns":measured.maximum_ns,"callback_resolution_ns":measured.callback_resolution_ns,"correlations":measured.correlations,"input_frames":measured.input_frames})
                }
                Err(error) => {
                    serde_json::json!({"output_channel":output_channel,"input_channel":input_channel,"status":"refused","error":error})
                }
            };
            pairs.push(row);
            std::fs::write(directory.join("ns7-peavey-pairs.json"),serde_json::to_vec_pretty(&serde_json::json!({"nominal_rate":44100,"probe_level_dbfs":level_db,"output_device":output.device,"input_device":input.device,"owner_reported_wiring":"NS7 XLR outputs to Peavey 8 channel 6; mixer USB return to m1pro16","pairs":pairs,"output_detected_on_wired_master_return":found,"headphone_jack_captured":false,"scope":"Three distinct probe matches per successful output/input pair. Common host callback-entry timing includes converter, mixer and callback batching; this is not isolated analog converter latency or an independent-channel claim. Outputs 1/2 are the expected master pair; optional outputs 3/4 are a channel-assignment diagnostic."})).unwrap()).unwrap();
        }
    }
    assert!(found[..2].iter().all(|v|*v),"Each expected NS7 master output must deliver all three probes to at least one Peavey USB input: {pairs:?}");
}

#[test]
#[ignore = "Requires exclusive powered original NS7 and OMATAINER_CPU_BUDGET_DIR under /home; physical output is silent"]
fn native_ns7_cpu_limit_retains_recording_and_reconnects_only_explicitly() {
    use super::super::owner::{finish_shutdown, start_with, Phase};
    use crate::engine::{Command, Engine, SamplerInstrument, SynthInstrument};
    let directory = std::path::PathBuf::from(std::env::var_os("OMATAINER_CPU_BUDGET_DIR").unwrap());
    assert!(directory.starts_with("/home"));
    std::fs::create_dir_all(&directory).unwrap();
    let (engine, mut rt) = Engine::headless_for_test(44100, 256);
    for command in [
        Command::Master(0.0),
        Command::SamplerInst(SamplerInstrument::Synth(SynthInstrument::Keys)),
        Command::ComposeArm { track: 4, scene: 3 },
        Command::Play,
        Command::SamplerPad { pad: 0, on: true },
    ] {
        engine.cmd.send(command).unwrap();
    }
    rt.process(&mut [0.0; 2048]);
    for command in [Command::SamplerPad { pad: 0, on: false }, Command::Stop] {
        engine.cmd.send(command).unwrap();
    }
    rt.process(&mut [0.0; 2048]);
    let notes = rt.tracks[4].clips[3].notes.clone();
    assert_eq!(notes.len(), 1);
    assert!(notes[0].len > 0.0);
    let audio = start_with(
        rt,
        crate::preferences::Audio {
            backend: Some("ALSA".into()),
            device: Some("hw:CARD=NS7,DEV=0".into()),
            sample_rate: Some(44100),
            channels: Some(4),
            format: Some(crate::preferences::AudioFormat::I32),
            buffer_frames: Some(512),
            ..Default::default()
        },
        || BudgetBackend {
            native: super::super::Native,
            inject: true,
        },
    )
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while audio.handle.status().phase != Phase::Offline {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(2));
    }
    let offline = audio.handle.status();
    assert!(offline.message.contains("exceeded its real-time CPU limit"));
    assert!(offline.recovery.as_ref().unwrap().identity.is_some());
    assert_eq!(engine.cmd.audio_metrics().cpu_budget_exhaustions, 1);
    assert!(engine.cmd.send(Command::Play).is_err());
    let captured = engine.project.capture(&AtomicBool::new(false)).unwrap();
    assert_eq!(captured.state.tracks[4].clips[3].notes, notes);
    assert_eq!(captured.state.master, 0.0);
    let path = directory.join("ns7-retained.omat");
    crate::project_file::save(
        &path,
        &crate::project_file::Bundle {
            state: captured.state,
            media: captured.media,
        },
        crate::project_file::Overwrite::Never,
        &crate::project_file::Limits::default(),
        &AtomicBool::new(false),
    )
    .unwrap();
    let reopened = crate::project_file::load::<crate::engine::project::State>(
        &path,
        &crate::project_file::Limits::default(),
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(reopened.state.tracks[4].clips[3].notes, notes);
    let handle = &audio.handle;
    let restored = handle
        .reconnect_permitted(
            std::sync::Arc::new(AtomicBool::new(false)),
            handle.performance_permit().unwrap(),
            offline.generation,
        )
        .unwrap();
    assert_eq!(restored.phase, Phase::Running);
    assert!(engine.cmd.send(Command::Play).is_err());
    engine
        .cmd
        .performance()
        .acknowledge_inputs_released()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while engine.cmd.performance().status().recovery {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(2));
    }
    let report = serde_json::json!({"device":restored.active.as_ref().unwrap().plan.device,"master":0.0,"injected_callback_cpu_ms":30,"audio":engine.cmd.audio_metrics(),"recovery_message":offline.message,"recorded_notes":notes.len(),"reopened_notes":reopened.state.tracks[4].clips[3].notes.len(),"reconnect_required":true,"input_acknowledgment_required":true,"playing_after_reconnect":engine.snapshot().playing,"analog_listening":false});
    std::fs::write(
        directory.join("ns7-cpu-recovery.json"),
        serde_json::to_vec_pretty(&report).unwrap(),
    )
    .unwrap();
    drop(audio);
    finish_shutdown();
}
