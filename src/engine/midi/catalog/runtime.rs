//! Private instance pins and bounded off-callback controller acquisition.
use super::{identity::Device, storage, *};
use crate::engine::{performance, Snapshot};
use arc_swap::ArcSwap;
use crossbeam_channel::{bounded, Sender};
use parking_lot::Mutex;
use std::{collections::BTreeMap, sync::Arc, time::Duration};
const URL: &str = "https://raw.githubusercontent.com/michaelmonetized/omatainer/stack/app-completion/profiles/catalog.json";
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Anchor {
    key: String,
    name: String,
    port: String,
    profile: String,
    version: String,
    hash: String,
    generation: u64,
}
#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Saved {
    schema: u32,
    floor: u64,
    anchors: Vec<Anchor>,
}
#[derive(Clone)]
pub(crate) struct Connected {
    pub id: String,
    pub name: String,
    pub device: Device,
    pub output: Option<String>,
    pub profile: Option<Profile>,
    pub profile_hash: String,
    pub generation: u64,
    pub active_bindings_sha256: String,
    pub local_preset_override: bool,
    pub endpoint_name: String,
    pub endpoint_id: String,
    pub reason: String,
    pub input_open: bool,
    pub output_open: bool,
    pub initialization_sent: bool,
    pub output_failures: u64,
    pub input_packets: u64,
    pub inquiry: Option<String>,
}
#[derive(Clone, Default)]
pub(crate) struct View {
    pub ready: bool,
    pub busy: bool,
    pub generation: u64,
    pub cached_generation: Option<u64>,
    pub previous_generation: Option<u64>,
    pub devices: Vec<Connected>,
    pub message: String,
    pub guide: Option<Check>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Check {
    pub schema: u32,
    pub instance: String,
    pub input_port_id: String,
    pub usb_connection: String,
    pub connection_ended: bool,
    pub control: String,
    pub app_version: String,
    pub app_binary_sha256: String,
    pub source_revision: String,
    pub platform: String,
    pub driver: String,
    pub profile_id: String,
    pub profile_version: String,
    pub profile_data_sha256: String,
    pub profile_file_sha256: Option<String>,
    pub active_bindings_sha256: String,
    pub local_preset_override: bool,
    pub usb_release: u16,
    pub firmware_reply: Option<String>,
    pub packets: Vec<String>,
    pub input_received: bool,
    pub worker_processed: u64,
    pub application_observation: Option<String>,
    pub physical_observation: Option<String>,
    pub capturing: bool,
}
struct State {
    view: View,
    saved: Saved,
    bundled: Vec<Profile>,
    accepted: Option<storage::Acquired>,
    available: Option<storage::Acquired>,
    previous: Option<storage::Acquired>,
    generations: BTreeMap<u64, storage::Acquired>,
    apply: Option<bool>,
    guide_revision: u64,
    guide_revisions: BTreeMap<String, u64>,
    capture_states: BTreeMap<String, Arc<std::sync::atomic::AtomicU64>>,
    binary: String,
    disk_failed: bool,
}
struct Shared {
    data: Mutex<State>,
    published: ArcSwap<View>,
    directory: PathBuf,
    cancel: Mutex<Option<Arc<AtomicBool>>>,
    alive: AtomicBool,
}
enum Job {
    Acquire(performance::WorkPermit),
    Save(Check, performance::WorkPermit),
}
pub(crate) struct Registry {
    shared: Arc<Shared>,
    requests: Sender<Job>,
    worker: Mutex<Option<std::thread::JoinHandle<()>>>,
}
fn binary_hash() -> Result<String, String> {
    use std::io::Read;
    let mut file = std::fs::File::open(std::env::current_exe().map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    let mut digest = Sha256::new();
    let mut buffer = [0u8; 65536];
    loop {
        let n = file.read(&mut buffer).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        digest.update(&buffer[..n]);
    }
    Ok(format!("{:x}", digest.finalize()))
}
fn profile_hash(profile: &Profile) -> String {
    hash(&serde_json::to_vec(profile).unwrap_or_default())
}
fn cache_at(
    directory: &Path,
    signed_name: &str,
    minimum: u64,
) -> Result<storage::Acquired, String> {
    let signed = storage::regular(&directory.join(signed_name), MAX_CATALOG)?;
    let catalog = Catalog::decode(&signed, minimum)?;
    storage::Acquired::cached(
        &directory.join(format!("generation-{}", catalog.generation)),
        minimum,
    )
}
fn stopped(snapshot: &Snapshot) -> bool {
    !snapshot.playing && !snapshot.recording && snapshot.decks.iter().all(|d| !d.playing)
}
impl Shared {
    fn publish(&self, state: &State) {
        self.published.store(Arc::new(state.view.clone()));
    }
    fn save(&self, state: &State) -> Result<(), String> {
        storage::atomic(
            &self.directory.join("instances.json"),
            &serde_json::to_vec_pretty(&state.saved).map_err(|e| e.to_string())?,
        )
    }
}
impl Registry {
    /// Start private cache recovery and one bounded acquisition owner.
    /// Takes compiled factory data and the private cache directory; returns an asynchronous registry without doing network or filesystem work in MIDI/audio callbacks.
    pub fn start(directory: PathBuf) -> Result<Arc<Self>, String> {
        let shared = Arc::new(Shared {
            data: Mutex::new(State {
                view: View::default(),
                saved: Saved {
                    schema: SCHEMA,
                    ..Saved::default()
                },
                bundled: bundled()?,
                accepted: None,
                available: None,
                previous: None,
                generations: BTreeMap::new(),
                apply: None,
                guide_revision: 0,
                guide_revisions: BTreeMap::new(),
                capture_states: BTreeMap::new(),
                binary: String::new(),
                disk_failed: false,
            }),
            published: ArcSwap::from_pointee(View::default()),
            directory,
            cancel: Mutex::new(None),
            alive: AtomicBool::new(true),
        });
        let (requests, receiver) = bounded::<Job>(1);
        let worker_shared = shared.clone();
        let worker=std::thread::Builder::new().name("omatainer-controller-catalog".into()).spawn(move|| {
            {
                let mut state=worker_shared.data.lock();
                let path=worker_shared.directory.join("instances.json");
                if path.exists() {
                    let saved = storage::regular(&path,256*1024).and_then(|bytes|serde_json::from_slice::<Saved>(&bytes).map_err(|e|e.to_string())).and_then(|saved| {
                        if saved.schema!=SCHEMA || saved.anchors.len()>256 || saved.anchors.iter().any(|a|!visible(&a.key,1024)||!visible(&a.name,1024)||!visible(&a.port,1024)||!visible(&a.profile,80)||version(&a.version).is_err()||hex(&a.hash,32).is_err()) || saved.anchors.iter().map(|a|&a.key).collect::<BTreeSet<_>>().len()!=saved.anchors.len() {Err("Invalid private controller instance pins; review cache before updates".into())}else{Ok(saved)}
                    });
                    match saved {Ok(saved)=>state.saved=saved,Err(e)=>{state.disk_failed=true;state.view.message=e;}}
                }
                if !state.disk_failed && worker_shared.directory.join("latest.json").exists() {
                    match cache_at(&worker_shared.directory,"latest.json",state.saved.floor) {
                        Ok(cache)=> {state.saved.floor=state.saved.floor.max(cache.catalog.generation);state.available=Some(cache);},
                        Err(e)=>state.view.message=format!("Cached update refused; retained pinned/bundled mappings: {e}"),
                    }
                }
                if worker_shared.directory.join("last-good.json").exists() { state.previous=cache_at(&worker_shared.directory,"last-good.json",0).ok(); }
                let mut generations=BTreeSet::new();
                for anchor in &state.saved.anchors {if anchor.generation!=0{generations.insert(anchor.generation);}}
                for generation in generations {if let Ok(cache)=storage::Acquired::cached(&worker_shared.directory.join(format!("generation-{generation}")),0){state.generations.insert(generation,cache);}}
                if state.generations.len()==1 {state.accepted=state.generations.values().next().cloned();}
                state.binary=binary_hash().unwrap_or_else(|_|"unavailable".into());
                if let Some(cache)=state.available.clone(){state.generations.insert(cache.catalog.generation,cache);}
                state.view.ready=true;
                state.view.cached_generation=state.available.as_ref().map(|c|c.catalog.generation);
                state.view.previous_generation=Some(state.previous.as_ref().map_or(0,|c|c.catalog.generation));
                state.view.generation=state.accepted.as_ref().map_or(0,|c|c.catalog.generation);
                worker_shared.publish(&state);
            }
            while worker_shared.alive.load(Ordering::Acquire) {
                let job=match receiver.recv_timeout(Duration::from_millis(200)) {Ok(p)=>p,Err(crossbeam_channel::RecvTimeoutError::Timeout)=>continue,Err(_)=>break};
                let permit=match job{Job::Acquire(p)=>p,Job::Save(check,p)=>{
                    let result=(||{let _commit=p.commit().map_err(|e|e.to_string())?;let file=format!("check-{}-{}.json",std::process::id(),super::super::next_source_id());storage::atomic(&worker_shared.directory.join("qualification").join(&file),&serde_json::to_vec_pretty(&check).map_err(|e|e.to_string())?)?;Ok::<_,String>(file)})();
                    let mut s=worker_shared.data.lock();s.view.message=match result{Ok(file)=>format!("Controller evidence saved: {file}"),Err(e)=>format!("Controller evidence save refused: {e}")};s.view.busy=false;*worker_shared.cancel.lock()=None;worker_shared.publish(&s);continue;
                }};
                let minimum=worker_shared.data.lock().saved.floor;
                let result=if permit.cancelled(){Err("Profile acquisition cancelled by performance protection".into())}else{storage::Acquired::download(URL,&worker_shared.directory,minimum,&permit.cancel())};
                let mut state=worker_shared.data.lock();
                match result {
                    Ok(cache) if !permit.cancelled()=> {
                        state.saved.floor=state.saved.floor.max(cache.catalog.generation);
                        state.generations.insert(cache.catalog.generation,cache.clone());
                        state.available=Some(cache);
                        state.previous=cache_at(&worker_shared.directory,"last-good.json",0).ok();
                        state.view.cached_generation=state.available.as_ref().map(|c|c.catalog.generation);
                        state.view.previous_generation=Some(state.previous.as_ref().map_or(0,|c|c.catalog.generation));
                        state.view.message="Authenticated profiles cached. Apply explicitly while transport and decks are stopped.".into();
                        if let Err(e)=worker_shared.save(&state){state.disk_failed=true;state.view.message=format!("Profile cache saved, instance pins unavailable: {e}");}
                    },
                    Ok(_)=>state.view.message="Acquisition cancelled; active instance pins preserved".into(),
                    Err(e)=>state.view.message=format!("Profile acquisition refused; working mappings preserved: {e}"),
                }
                state.view.busy=false;*worker_shared.cancel.lock()=None;worker_shared.publish(&state);
            }
        }).map_err(|e|e.to_string())?;
        Ok(Arc::new(Self {
            shared,
            requests,
            worker: Mutex::new(Some(worker)),
        }))
    }
    /// Publish compact connection evidence for native status and diagnostics.
    /// Takes no arguments; returns bounded aggregate input/output facts and a packet-free check summary, preserving the 8 KiB IPC budget. Full identities and evidence remain in the panel and private receipts.
    pub fn receipt(&self) -> serde_json::Value {
        let v = self.view();
        let check=v.guide.as_ref().map(|g|serde_json::json!({"control":g.control,"input_port_id":g.input_port_id,"input_received":g.input_received,"worker_processed":g.worker_processed,"captured_packets":g.packets.len(),"capturing":g.capturing,"connection_ended":g.connection_ended}));
        serde_json::json!({"schema":SCHEMA,"ready":v.ready,"busy":v.busy,"cached_generation":v.cached_generation,"previous_generation":v.previous_generation,"physical_inputs":v.devices.len(),"profiled_inputs":v.devices.iter().filter(|d|d.profile.is_some()).count(),"input_open":v.devices.iter().filter(|d|d.input_open).count(),"output_open":v.devices.iter().filter(|d|d.output_open).count(),"initialization_sent":v.devices.iter().filter(|d|d.initialization_sent).count(),"output_failures":v.devices.iter().map(|d|d.output_failures).fold(0u64,u64::saturating_add),"control_check":check,"physical_qualification":"pending"})
    }
    pub fn loaded(&self) -> bool {
        self.view().ready
    }
    pub fn view(&self) -> Arc<View> {
        self.shared.published.load_full()
    }
    pub fn acquire(&self, performance: &performance::Handle) -> Result<(), String> {
        let mut state = self.shared.data.lock();
        if !state.view.ready || state.view.busy || state.disk_failed {
            return Err("Controller cache is not ready for acquisition".into());
        }
        let permit = performance.optional_work().map_err(|e| e.to_string())?;
        *self.shared.cancel.lock() = Some(permit.cancel());
        self.requests
            .try_send(Job::Acquire(permit))
            .map_err(|_| "Controller acquisition is already queued".to_string())?;
        state.view.busy = true;
        state.view.message = "Checking authenticated profile data".into();
        self.shared.publish(&state);
        Ok(())
    }
    pub fn cancel(&self) {
        if let Some(flag) = self.shared.cancel.lock().as_ref() {
            flag.store(true, Ordering::Release);
        }
    }
    /// Queue a mapping change for the connection manager's exclusive transaction.
    /// Takes an explicit update/rollback choice and stopped snapshot; returns pending intent only, leaving active maps untouched until OS reconnection applies it.
    pub fn request_apply(&self, rollback: bool, snapshot: &Snapshot) -> Result<(), String> {
        let mut state = self.shared.data.lock();
        if !stopped(snapshot) || snapshot.performance.protected || snapshot.performance.recovery {
            return Err(
                "Stop transport and decks, then use Studio mode to apply controller profiles"
                    .into(),
            );
        }
        if state.disk_failed
            || !state.view.ready
            || state.view.busy
            || state.apply.is_some()
            || state.view.guide.as_ref().is_some_and(|g| g.capturing)
        {
            return Err("Finish controller work before changing profiles".into());
        }
        if !rollback && state.available.is_none() {
            return Err("No complete authenticated generation is available".into());
        }
        state.apply = Some(rollback);
        state.view.message =
            "Controller profile change queued at a stopped connection boundary".into();
        self.shared.publish(&state);
        Ok(())
    }
    pub fn abandon_apply(&self) {
        self.shared.data.lock().apply = None;
    }
    /// Resolve complete physical input/output inventories inside the connection transaction.
    /// Takes observed IDs and optional USB attributes; returns exact profile choices, refusing duplicate serials, conflicting profiles and ambiguous output roles.
    pub fn refresh(
        &self,
        inputs: Vec<(String, String, Option<Device>)>,
        outputs: Vec<(String, Option<Device>)>,
        snapshot: &Snapshot,
    ) {
        let mut state = self.shared.data.lock();
        if !state.view.ready {
            return;
        }
        if let Some(rollback) = state.apply.take() {
            if !stopped(snapshot) {
                state.view.message =
                    "Profile change refused because playback started; existing maps retained"
                        .into();
            } else {
                let candidate = if rollback {
                    state.previous.clone()
                } else {
                    state.available.clone()
                };
                let (profiles, generation) = candidate.as_ref().map_or_else(
                    || (state.bundled.clone(), 0),
                    |c| (c.profiles.clone(), c.catalog.generation),
                );
                let old = state.saved.anchors.clone();
                for anchor in &mut state.saved.anchors {
                    if let Some(p) = profiles.iter().find(|p| p.id == anchor.profile) {
                        anchor.version = p.version.clone();
                        anchor.hash = profile_hash(p);
                        anchor.generation = generation;
                    }
                }
                if let Err(e) = self.shared.save(&state) {
                    state.saved.anchors = old;
                    state.view.message =
                        format!("Profile change refused; instance pins could not be saved: {e}");
                } else {
                    if let Some(c) = &candidate {
                        state.generations.insert(c.catalog.generation, c.clone());
                    }
                    state.accepted = candidate;
                    state.view.generation = generation;
                    state.view.message="Explicit controller profile generation applied; learned and local preset overrides retained".into();
                }
            }
        }

        let prior = state.view.devices.clone();
        let profiles = state
            .available
            .as_ref()
            .or(state.accepted.as_ref())
            .map_or_else(|| state.bundled.clone(), |c| c.profiles.clone());
        let mut devices = Vec::new();
        let mut changed = false;
        for (id, name, device) in &inputs {
            let Some(device) = device else {
                continue;
            };
            let key = device.key();
            let duplicate = inputs
                .iter()
                .filter(|(_, _, d)| d.as_ref().is_some_and(|d| d.key() == key))
                .count()
                != 1;
            let candidates: Vec<_> = profiles.iter().filter(|p| p.matches(device)).collect();
            let matching_outputs: Vec<_> = outputs
                .iter()
                .filter(|(_, d)| d.as_ref().is_some_and(|d| d == device))
                .collect();
            let mut reason = if duplicate {
                "Duplicate physical identity; select and qualify ports explicitly".into()
            } else if candidates.len() > 1 {
                "Conflicting controller profiles; retain generic MIDI until reviewed".into()
            } else if candidates.is_empty() {
                "No compatible USB/protocol/port profile; generic MIDI only".into()
            } else if matching_outputs.len() != 1 {
                "Output role absent or ambiguous; input mapping available, feedback disabled".into()
            } else {
                "Exact USB model and uniquely paired port; physical function checks pending".into()
            };
            let anchor = state.saved.anchors.iter().find(|a| a.key == key).cloned();
            let mut profile = if duplicate {
                None
            } else if let Some(a) = &anchor {
                let pool = if a.generation == 0 {
                    Some(state.bundled.clone())
                } else {
                    state
                        .generations
                        .get(&a.generation)
                        .map(|c| c.profiles.clone())
                };
                let p = pool.and_then(|p| {
                    p.into_iter().find(|p| {
                        p.id == a.profile
                            && p.version == a.version
                            && profile_hash(p) == a.hash
                            && p.matches(device)
                    })
                });
                if p.is_none() {
                    reason="Pinned controller profile unavailable or changed; generic input, feedback disabled".into();
                }
                p
            } else {
                if candidates.len() == 1 {
                    Some(candidates[0].clone())
                } else {
                    None
                }
            };
            if profile.is_some() && anchor.is_none() && state.saved.anchors.len() < 256 {
                let p = profile.as_ref().unwrap();
                let generation = state
                    .available
                    .as_ref()
                    .or(state.accepted.as_ref())
                    .map_or(0, |c| c.catalog.generation);
                state.saved.anchors.push(Anchor {
                    key: key.clone(),
                    name: name.clone(),
                    port: id.clone(),
                    profile: p.id.clone(),
                    version: p.version.clone(),
                    hash: profile_hash(p),
                    generation,
                });
                changed = true;
            }
            if duplicate {
                profile = None;
            }
            let anchor = state.saved.anchors.iter().find(|a| a.key == key);
            let mut entry = Connected {
                id: id.clone(),
                name: name.clone(),
                device: device.clone(),
                output: (!duplicate && matching_outputs.len() == 1 && profile.is_some())
                    .then(|| matching_outputs[0].0.clone()),
                profile_hash: profile.as_ref().map_or(String::new(), profile_hash),
                generation: anchor.map_or(0, |a| a.generation),
                active_bindings_sha256: String::new(),
                local_preset_override: false,
                endpoint_name: anchor.map_or_else(|| name.clone(), |a| a.name.clone()),
                endpoint_id: anchor.map_or_else(|| id.clone(), |a| a.port.clone()),
                profile,
                reason,
                input_open: false,
                output_open: false,
                initialization_sent: false,
                output_failures: 0,
                input_packets: 0,
                inquiry: None,
            };
            if let Some(old) = prior.iter().find(|p| {
                p.id == *id && p.device == *device && p.profile_hash == entry.profile_hash
            }) {
                entry.active_bindings_sha256 = old.active_bindings_sha256.clone();
                entry.local_preset_override = old.local_preset_override;
                entry.input_open = old.input_open;
                entry.output_open = old.output_open;
                entry.initialization_sent = old.initialization_sent;
                entry.output_failures = old.output_failures;
                entry.input_packets = old.input_packets;
                entry.inquiry = old.inquiry.clone();
            }
            devices.push(entry);
        }
        state.view.devices = devices;
        Self::end_missing_check(&mut state);
        if changed {
            if let Err(e) = self.shared.save(&state) {
                state.view.message = format!("Controller pins are temporary: {e}");
            }
        }
        self.shared.publish(&state);
    }
    fn end_missing_check(state: &mut State) {
        if state.view.guide.as_ref().is_some_and(|g| {
            !state.view.devices.iter().any(|d| {
                d.id == g.input_port_id
                    && d.device.key() == g.instance
                    && d.device.connection == g.usb_connection
            })
        }) {
            if let Some(g) = &mut state.view.guide {
                g.capturing = false;
                g.connection_ended = true;
                let id = g.input_port_id.clone();
                if let Some(capture) = state.capture_states.get(&id) {
                    capture
                        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |epoch| {
                            Some(epoch.wrapping_add(2) & !1)
                        })
                        .ok();
                }
            }
        }
        let present: std::collections::BTreeSet<_> =
            state.view.devices.iter().map(|d| d.id.clone()).collect();
        state.capture_states.retain(|id, _| present.contains(id));
        state.guide_revisions.retain(|id, _| present.contains(id));
    }
    /// Retire lost native ports during protection without acquiring profiles or opening new inputs.
    /// Takes bounded exact port IDs and USB connection incarnations; keeps surviving callbacks, stops the removed source through its owner and ends obsolete checks.
    pub fn retire_absent(&self, present: &[(String, String)]) {
        let mut state = self.shared.data.lock();
        state.view.devices.retain(|d| {
            present
                .iter()
                .any(|(id, c)| *id == d.id && *c == d.device.connection)
        });
        Self::end_missing_check(&mut state);
        self.shared.publish(&state);
    }
    pub fn mapping(&self, id: &str, local_mpd: Option<&MidiMap>) -> MidiMap {
        let mut state = self.shared.data.lock();
        let Some(device) = state.view.devices.iter_mut().find(|d| d.id == id) else {
            return super::super::class_compliant();
        };
        let Some(profile) = &device.profile else {
            return super::super::class_compliant();
        };
        let override_map = if profile.driver == Driver::Mpd232 {
            local_mpd
        } else {
            None
        };
        let map = override_map.cloned().unwrap_or_else(|| profile.map());
        device.local_preset_override = override_map.is_some();
        device.active_bindings_sha256 =
            hash(&serde_json::to_vec(&map.bindings).unwrap_or_default());
        self.shared.publish(&state);
        map
    }
    pub fn endpoint(
        &self,
        id: &str,
        name: &str,
        policy: &super::super::policy::InputPolicy,
    ) -> (String, String, String) {
        let view = self.view();
        match view
            .devices
            .iter()
            .find(|d| d.id == id && d.profile.is_some())
        {
            Some(d) => (
                d.endpoint_name.clone(),
                d.endpoint_id.clone(),
                if policy.allows(name) {
                    name.into()
                } else {
                    d.endpoint_name.clone()
                },
            ),
            None => (name.into(), id.into(), name.into()),
        }
    }
    pub fn input_open(&self, id: &str, open: bool) {
        let mut s = self.shared.data.lock();
        if let Some(d) = s.view.devices.iter_mut().find(|d| d.id == id) {
            d.input_open = open;
            if !open {
                d.output_open = false;
            }
        }
        self.shared.publish(&s);
    }
    pub fn output(&self, id: &str) -> Option<Connected> {
        self.view()
            .devices
            .iter()
            .find(|d| d.input_open && d.output.as_deref() == Some(id) && d.profile.is_some())
            .cloned()
    }
    pub fn output_result(&self, id: &str, open: bool, initialization: bool, failed: bool) {
        let mut s = self.shared.data.lock();
        if let Some(d) = s
            .view
            .devices
            .iter_mut()
            .find(|d| d.output.as_deref() == Some(id))
        {
            d.output_open = open;
            d.initialization_sent |= initialization;
            d.output_failures += u64::from(failed);
        }
        self.shared.publish(&s);
    }
    pub fn begin_check(&self, id: &str, control: &str, snapshot: &Snapshot) -> Result<(), String> {
        if !visible(control, 128)
            || !stopped(snapshot)
            || snapshot.performance.protected
            || snapshot.performance.recovery
        {
            return Err(
                "Name the control and stop transport/decks in Studio before an input check".into(),
            );
        }
        let mut s = self.shared.data.lock();
        let d = s
            .view
            .devices
            .iter()
            .find(|d| d.id == id && d.input_open)
            .cloned()
            .ok_or("Open an unambiguous supported input first")?;
        if s.view.guide.as_ref().is_some_and(|g| g.capturing) {
            return Err("Finish the current input capture first".into());
        }
        let p = d.profile.as_ref();
        s.view.guide = Some(Check {
            schema: SCHEMA,
            instance: d.device.key(),
            input_port_id: id.into(),
            usb_connection: d.device.connection.clone(),
            connection_ended: false,
            control: control.into(),
            app_version: env!("CARGO_PKG_VERSION").into(),
            app_binary_sha256: s.binary.clone(),
            source_revision: option_env!("OMATAINER_SOURCE_REVISION")
                .unwrap_or("unrecorded; identify binary in external build receipt")
                .into(),
            platform: format!("{}-{}", std::env::consts::ARCH, std::env::consts::OS),
            driver: p.map_or(Driver::Generic, |p| p.driver).capability().into(),
            profile_id: p.map_or("unassigned", |p| p.id.as_str()).into(),
            profile_version: p.map_or("0.0.0", |p| p.version.as_str()).into(),
            profile_data_sha256: d.profile_hash,
            profile_file_sha256: s
                .generations
                .get(&d.generation)
                .and_then(|c| {
                    c.catalog
                        .profiles
                        .iter()
                        .find(|e| p.is_some_and(|p| p.id == e.id && p.version == e.version))
                })
                .map(|e| e.sha256.clone()),
            active_bindings_sha256: d.active_bindings_sha256,
            local_preset_override: d.local_preset_override,
            usb_release: d.device.release,
            firmware_reply: d.inquiry,
            packets: Vec::new(),
            input_received: false,
            worker_processed: 0,
            application_observation: None,
            physical_observation: None,
            capturing: true,
        });
        s.guide_revision = s.guide_revision.wrapping_add(1).max(1);
        let revision = s.guide_revision;
        s.guide_revisions.insert(id.into(), revision);
        let capture = s
            .capture_states
            .entry(id.into())
            .or_insert_with(|| Arc::new(std::sync::atomic::AtomicU64::new(0)));
        capture.store((revision << 1) | 1, Ordering::Release);
        self.shared.publish(&s);
        Ok(())
    }
    pub fn finish_check(&self) {
        let mut s = self.shared.data.lock();
        if let Some(g) = s.view.guide.as_mut().filter(|g| g.capturing) {
            g.capturing = false;
            let id = g.input_port_id.clone();
            s.guide_revision = s.guide_revision.wrapping_add(1).max(1);
            let revision = s.guide_revision;
            s.guide_revisions.insert(id.clone(), revision);
            if let Some(capture) = s.capture_states.get(&id) {
                capture.store(revision << 1, Ordering::Release);
            }
            self.shared.publish(&s);
        }
    }
    pub fn finish_when_unsafe(&self, snapshot: &Snapshot, panel_open: bool) {
        if !panel_open
            || !stopped(snapshot)
            || snapshot.performance.protected
            || snapshot.performance.recovery
        {
            self.finish_check();
        }
    }
    pub fn capture_state(&self, id: &str) -> Arc<std::sync::atomic::AtomicU64> {
        let mut s = self.shared.data.lock();
        s.capture_states
            .entry(id.into())
            .or_insert_with(|| Arc::new(std::sync::atomic::AtomicU64::new(0)))
            .clone()
    }
    pub fn guide_revision(&self, id: &str) -> u64 {
        self.shared
            .data
            .lock()
            .guide_revisions
            .get(id)
            .copied()
            .unwrap_or(0)
    }
    /// Record a bounded packet after callback handoff, before any musical action.
    /// Takes exact physical input ID and bytes; returns whether the explicitly armed input check consumes it. Sent output never establishes received input or a physical observation.
    pub fn observe(&self, id: &str, bytes: &[u8]) -> bool {
        let mut s = self.shared.data.lock();
        let mut key = None;
        if let Some(d) = s.view.devices.iter_mut().find(|d| d.id == id) {
            d.input_packets = d.input_packets.saturating_add(1);
            key = Some(d.device.key());
            if bytes.len() >= 6
                && bytes[0] == 0xf0
                && bytes[1] == 0x7e
                && bytes[3] == 6
                && bytes[4] == 2
                && bytes.last() == Some(&0xf7)
            {
                d.inquiry = Some(
                    bytes
                        .iter()
                        .map(|b| format!("{b:02x}"))
                        .collect::<Vec<_>>()
                        .join(" "),
                );
            }
        }
        let consume = if let Some(g) =
            s.view.guide.as_mut().filter(|g| {
                g.capturing && g.input_port_id == id && Some(&g.instance) == key.as_ref()
            }) {
            g.input_received = true;
            if g.packets.len() < 64 {
                g.packets.push(
                    bytes
                        .iter()
                        .take(256)
                        .map(|b| format!("{b:02x}"))
                        .collect::<Vec<_>>()
                        .join(" "),
                );
            }
            g.worker_processed = g.worker_processed.saturating_add(1);
            g.capturing
        } else {
            false
        };
        if consume {
            self.shared.publish(&s);
        }
        consume
    }
    pub fn observations(
        &self,
        application: Option<&str>,
        physical: Option<&str>,
    ) -> Result<(), String> {
        let mut s = self.shared.data.lock();
        let g = s.view.guide.as_mut().ok_or("Run an input check first")?;
        if g.connection_ended {
            return Err("Controller disconnected during this check; start a fresh input capture before recording observations".into());
        }
        if g.capturing {
            return Err(
                "Finish capture and exercise the normal control before recording observations"
                    .into(),
            );
        }
        for value in [application, physical].into_iter().flatten() {
            if !visible(value, 512) {
                return Err("Use a bounded, explicit observation".into());
            }
        }
        if let Some(v) = application {
            g.application_observation = Some(v.into());
        }
        if let Some(v) = physical {
            g.physical_observation = Some(v.into());
        }
        self.shared.publish(&s);
        Ok(())
    }
    pub fn save_check(&self, performance: &performance::Handle) -> Result<(), String> {
        let mut s = self.shared.data.lock();
        let g = s.view.guide.as_ref().ok_or("No controller check to save")?;
        if g.capturing || s.view.busy {
            return Err("Finish capture and pending work first".into());
        }
        let check = g.clone();
        let permit = performance.optional_work().map_err(|e| e.to_string())?;
        *self.shared.cancel.lock() = Some(permit.cancel());
        self.requests
            .try_send(Job::Save(check, permit))
            .map_err(|_| "Controller evidence owner is busy")?;
        s.view.busy = true;
        s.view.message = "Saving controller observations".into();
        self.shared.publish(&s);
        Ok(())
    }
}
impl Drop for Registry {
    fn drop(&mut self) {
        self.shared.alive.store(false, Ordering::Release);
        self.cancel();
        if let Some(worker) = self.worker.lock().take() {
            if worker.is_finished() {
                let _ = worker.join();
            }
        }
    }
}
