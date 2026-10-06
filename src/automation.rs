//! Version-1 automation reuses native command admission and prepared session edits.
use crate::engine::{
    self,
    midi_edit::{Ack, Outcome},
    remote, session, Command, CommandPort, Snapshot,
};
use crate::ipc_transport::{self, Limits};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::os::unix::net::UnixStream;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::Duration;

pub(crate) const VERSION: u32 = 1;
pub(crate) mod osc;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub(crate) struct Key(pub [u64; 2]);
impl TryFrom<String> for Key {
    type Error = &'static str;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        if value.len() != 32 || !value.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err("namespace must contain 32 hexadecimal characters");
        }
        let key = [
            u64::from_str_radix(&value[..16], 16).unwrap(),
            u64::from_str_radix(&value[16..], 16).unwrap(),
        ];
        if key == [0, 0] {
            return Err("namespace cannot be zero");
        }
        Ok(Self(key))
    }
}
impl From<Key> for String {
    fn from(key: Key) -> Self {
        format!("{:016x}{:016x}", key.0[0], key.0[1])
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub(crate) struct ObjectId(pub u64);
impl TryFrom<String> for ObjectId {
    type Error = &'static str;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        if value.len() != 16 || !value.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err("id must contain 16 hexadecimal characters");
        }
        let id = u64::from_str_radix(&value, 16).unwrap();
        if id == 0 {
            return Err("object id cannot be zero");
        }
        Ok(Self(id))
    }
}
impl From<ObjectId> for String {
    fn from(id: ObjectId) -> Self {
        format!("{:016x}", id.0)
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub(crate) struct Count(pub u64);
impl TryFrom<String> for Count {
    type Error = &'static str;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        let count = value
            .parse::<u64>()
            .map_err(|_| "counter must be a decimal string")?;
        if count.to_string() != value {
            return Err("counter must be a canonical decimal string");
        }
        Ok(Self(count))
    }
}
impl From<Count> for String {
    fn from(count: Count) -> Self {
        count.0.to_string()
    }
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Axis {
    #[default]
    Track,
    Scene,
}
impl Axis {
    /// Select the native identity table.
    /// Takes an API axis; returns its matching native session axis.
    fn native(self) -> session::Axis {
        match self {
            Self::Track => session::Axis::Track,
            Self::Scene => session::Axis::Scene,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Target {
    pub namespace: Key,
    pub axis: Axis,
    pub id: ObjectId,
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Expected {
    pub namespace: Key,
    pub generation: Count,
    pub revision: Count,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Page {
    #[serde(default)]
    pub axis: Axis,
    #[serde(default)]
    pub offset: usize,
    #[serde(default = "page_limit")]
    pub limit: usize,
}
/// Set the default state page size.
/// Takes no arguments; returns eight bounded object summaries.
fn page_limit() -> usize {
    8
}
impl Default for Page {
    fn default() -> Self {
        Self {
            axis: Axis::Track,
            offset: 0,
            limit: page_limit(),
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Action {
    Play {},
    Stop {},
    LaunchScene { target: Target },
    TrackGain { target: Target, value: f32 },
    TrackPan { target: Target, value: f32 },
    Crossfader { value: f32 },
    CrossfaderContour { value: f32 },
    MasterGain { value: f32 },
    DeckControl { deck: u8, control: engine::deck_controls::Control },
    Monitor { control: engine::monitor::Control },
}
#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Edit {
    Rename { name: String },
    Color { color: Option<[u8; 3]> },
    Move { position: usize },
}
#[derive(Debug, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Request {
    NowPlaying {},
    Surfaces {},
    Discover {},
    State {
        #[serde(default)]
        page: Page,
    },
    Subscribe {
        #[serde(default)]
        page: Page,
    },
    Command {
        namespace: Key,
        action: Action,
    },
    Schedule {
        namespace: Key,
        beat: f64,
        action: Action,
    },
    Edit {
        expected: Expected,
        target: Target,
        action: Edit,
    },
    Job {
        id: String,
    },
    Cancel {
        id: String,
    },
    SafeStop {},
    EmergencySilence {
        confirm: bool,
    },
}

#[derive(Debug)]
pub(crate) struct Error {
    code: &'static str,
    message: String,
}
impl Error {
    /// Describe a stable API failure.
    /// Takes a code and explanation; returns a bounded error without request payloads.
    fn new(code: &'static str, message: impl ToString) -> Self {
        Self {
            code,
            message: ipc_transport::short_text(&message.to_string(), 256).to_owned(),
        }
    }
}

/// Publish the supported protocol and its bounds.
/// Takes no arguments; returns typed command fields, access rules and completion semantics.
fn discovery() -> Value {
    json!({
        "version": VERSION,
        "requests": {
            "now_playing": {"access":"read-only; explicit profile enablement and redaction; latest digital-output contributors; no paths"},
            "discover": {}, "state": {"page": "Page?"}, "surfaces": {}, "subscribe": {"page": "Page?"},
            "command": {"namespace": "Namespace", "action": "Action"},
            "schedule": {"namespace": "Namespace", "beat": "finite absolute quarter-note beat", "action": "Action"},
            "edit": {"expected": "Expected", "target": "Target", "action": "Edit"},
            "job": {"id": "opaque job string"}, "cancel": {"id": "opaque job string"},
            "safe_stop": {}, "emergency_silence": {"confirm": "true"}
        },
        "actions": {"play": {}, "stop": {}, "launch_scene": {"target":"scene Target"},
            "track_gain":{"target":"track Target","value":"number 0..1.5"},
            "track_pan":{"target":"track Target","value":"number 0..1; center 0.5"},
            "crossfader":{"value":"number 0..1"},"crossfader_contour":{"value":"number 0..1, fade to cut"},"master_gain":{"value":"number 0..1.5"},
            "monitor":{"control":"{op: source, value: pfl|deck_mix}; volume/blend/mix: 0..1; master/split: bool; pfl: {deck: 0|1, enabled: bool}; tone: 0|1 (one second at -40 dBFS, stopped available pair); cancel_tone"},"deck_control":{"deck":"0|1","control":"controller control: quantize (enabled: bool, division: 0..5 = 1/8, 1/4, 1/2, 1, 2, 4 beats), hold, keylock, pitch_range, strip, loop_mode, loop_button, loop_toggle, loop_select, reloop, loop_scale, loop_shift, loop_bounds (media_key, start_seconds, end_seconds), loop_move (media_key, signed beats), loop_length (media_key, 0.125..64 beats), beat_jump (forward: bool), beat_jump_size (index: 0..9), beat_jump_scale (up: bool), tap, start_time, stop_time, track_start, slip, pad_mode, parameter, hot_loop, auto_loop_pad, manual_pad, sync_off","quantized_onset":"applied acknowledges the accepted controller gesture; state.decks[].controls.pending reports any deferred musical onset and disappears on dispatch or cancellation"}},
        "edits":{"rename":{"name":"UTF-8 string, at most 1024 bytes"},
            "color":{"color":"null or three integer bytes"},"move":{"position":"zero-based display position"}},
        "types":{"Namespace":"32 hex characters; never a JSON number", "ObjectId":"16 nonzero hex characters",
            "Counter":"canonical unsigned decimal string", "Target":{"namespace":"Namespace","axis":"track|scene","id":"ObjectId"},
            "Expected":{"namespace":"Namespace","generation":"Counter","revision":"Counter"},
            "Page":{"axis":"track|scene; default track","offset":"integer 0..512; default 0","limit":"integer 1..16; default 8"}},
        "errors":["invalid_json","invalid_id","invalid_operation","unsupported_version","invalid_target","conflict",
            "snapshot_unavailable","submission_rejected","job_capacity","job_expired","cancel_conflict","invalid_schedule","unsupported_transport","permission_denied","not_applied","invalid_osc","request_too_large"],
        "limits":{"request_bytes":ipc_transport::REQUEST_BYTES,"response_bytes":ipc_transport::RESPONSE_BYTES,
            "connections":ipc_transport::CLIENTS,"scheduled_actions":remote::SCHEDULE_CAPACITY,"retained_jobs":128,
            "subscription_interval_ms":250,"object_name_bytes":32,"schedule_ahead_beats":16384},
        "access":{"unix":"socket mode 0600 and effective-UID peer check","osc":"optional 127.0.0.1 UDP with per-enable random token",
            "osc_message":"/omatainer/v1 ,sb: token string, UTF-8 API JSON blob","osc_reply":"/omatainer/v1/reply ,b: JSON blob",
            "osc_subscribe":false,"osc_bundles":false},
        "completion":"command, schedule and edit return a job. Poll pending/applied/rejected/cancelled; accepted is not applied. Up to 128 results survive client reconnect, not app restart; pending jobs are never evicted.",
        "schedule":"requires a playing transport; actions dispatch on musical sample boundaries. Stops, safety changes or project replacement invalidate pending actions. Scene playback retains native launch quantization.",
        "transaction":"one validated rename/color/move is an atomic undoable edit. Expected namespace, generation and project revision are checked again by audio; concurrent stale edits reject."
    })
}

/// Read a bounded coherent state page.
/// Takes the snapshot, commands, page and deadlines; returns stable IDs and a conflict receipt.
fn state(
    snapshot: &Arc<Mutex<Snapshot>>,
    commands: &CommandPort,
    page: Page,
    limits: Limits,
) -> Result<Value, Error> {
    if !(1..=16).contains(&page.limit) || page.offset > session::MAX_SCENES {
        return Err(Error::new(
            "invalid_operation",
            "Page limit must be 1..16 and offset 0..512",
        ));
    }
    let s = snapshot.try_lock_for(limits.snapshot).ok_or_else(|| {
        Error::new(
            "snapshot_unavailable",
            "Snapshot is temporarily unavailable",
        )
    })?;
    let layout = s.session.as_ref().ok_or_else(|| {
        Error::new(
            "snapshot_unavailable",
            "Session identity is not published yet",
        )
    })?;
    let slots: Vec<usize> = match page.axis {
        Axis::Track => layout.track_order.iter().map(|s| usize::from(*s)).collect(),
        Axis::Scene => layout.scene_order.iter().map(|s| usize::from(*s)).collect(),
    };
    let items = match page.axis {
        Axis::Track => &layout.tracks,
        Axis::Scene => &layout.scenes,
    };
    let objects: Vec<_> = slots.iter().skip(page.offset).take(page.limit).map(|slot| {
        let item = &items[*slot]; let name = ipc_transport::short_json_text(&item.name,32); json!({"target":Target {namespace:Key(layout.namespace),axis:page.axis,id:ObjectId(item.id.0)},
            "name":name,"name_truncated":name.len()<item.name.len(),"color":item.color})
    }).collect();
    Ok(
        json!({"expected":Expected {namespace:Key(layout.namespace),generation:Count(layout.generation),revision:Count(s.project_revision)},
        "playing":s.playing,"recording":s.recording,"beat":s.beat,"bpm":s.bpm,"master":s.master,"crossfader":s.xfader,"crossfader_contour":s.xfader_curve,"monitor":s.monitor,
        "decks":s.decks.iter().take(2).map(|deck| json!({"title":ipc_transport::short_text(&deck.title,32),"playing":deck.playing,"position_seconds":deck.pos/f64::from(deck.source_sample_rate.max(1)),"duration":deck.duration,"keylock":deck.keylock,"keylock_mode":deck.keylock_mode,"pitch_range":deck.pitch_range,"controls":deck.controls,"loop_on":deck.loop_on,"hotcues":deck.hotcues,
            "media_key":Count(deck.media_key),"source_sample_rate":deck.source_sample_rate,
            "loop_region":(deck.loop_len>=64.0).then(||json!({"start_seconds":deck.loop_start/f64::from(deck.source_sample_rate.max(1)),"end_seconds":(deck.loop_start+deck.loop_len)/f64::from(deck.source_sample_rate.max(1)),"enabled":deck.loop_on}))})).collect::<Vec<_>>(),
        "transport_epoch":Count(s.transport_epoch),"performance":commands.performance().status(),
        "page":page,"total":slots.len(),"objects":objects,
        "next_offset":(page.offset.saturating_add(page.limit)<slots.len()).then_some(page.offset+page.limit)}),
    )
}

/// Resolve an API target without using display order.
/// Takes the native layout, target and required axis; returns its original slot and identity.
fn target(
    layout: &session::Layout,
    target: Target,
    axis: session::Axis,
) -> Result<(usize, session::Reference), Error> {
    if target.namespace.0 != layout.namespace || target.axis.native() != axis {
        return Err(Error::new(
            "invalid_target",
            "Target namespace or axis does not match",
        ));
    }
    let id = session::Id(target.id.0);
    let slot = layout
        .resolve(axis, id)
        .ok_or_else(|| Error::new("invalid_target", "Target was deleted or does not exist"))?;
    Ok((
        slot,
        session::Reference {
            namespace: layout.namespace,
            id,
        },
    ))
}

impl Action {
    /// Prepare the existing native control.
    /// Takes the reviewed layout; returns a fixed action or a value/identity error.
    fn prepare(self, layout: &session::Layout) -> Result<remote::Action, Error> {
        let check = |value: f32, max: f32| {
            if value.is_finite() && (0.0..=max).contains(&value) {
                Ok(value)
            } else {
                Err(Error::new(
                    "invalid_operation",
                    format!("Value must be finite and within 0..{max}"),
                ))
            }
        };
        Ok(match self {
            Self::Play {} => remote::Action::Play,
            Self::Stop {} => remote::Action::Stop,
            Self::LaunchScene { target: t } => {
                let (slot, target) = target(layout, t, session::Axis::Scene)?;
                remote::Action::Scene { slot, target }
            }
            Self::TrackGain { target: t, value } => {
                let (slot, target) = target(layout, t, session::Axis::Track)?;
                remote::Action::Gain {
                    slot,
                    target,
                    value: check(value, 1.5)?,
                }
            }
            Self::TrackPan { target: t, value } => {
                let (slot, target) = target(layout, t, session::Axis::Track)?;
                remote::Action::Pan {
                    slot,
                    target,
                    value: check(value, 1.0)?,
                }
            }
            Self::Crossfader { value } => remote::Action::Crossfader(check(value, 1.0)?),
            Self::CrossfaderContour { value } => remote::Action::CrossfaderContour(check(value, 1.0)?),
            Self::MasterGain { value } => remote::Action::Master(check(value, 1.5)?),
            Self::DeckControl { deck, control } => {
                if usize::from(deck) >= engine::DECKS || !control.valid() { return Err(Error::new("invalid_operation", "Invalid deck controller target or value")); }
                remote::Action::DeckControl { deck, control }
            }
            Self::Monitor { control } => {
                if !control.valid() { return Err(Error::new("invalid_operation", "Invalid headphone control or value")); }
                remote::Action::Monitor(control)
            }
        })
    }
}

/// Submit one typed API operation.
/// Takes the request, command port, snapshot and deadlines; returns a state/result or stable failure.
fn dispatch(
    request: Request,
    commands: &CommandPort,
    snapshot: &Arc<Mutex<Snapshot>>,
    limits: Limits,
) -> Result<Value, Error> {
    match request {
        Request::NowPlaying {} => Ok(commands.now_playing().read()),
        Request::Discover {} => Ok(discovery()),
        Request::Surfaces {} => snapshot.try_lock_for(limits.snapshot).map(|snapshot| json!(snapshot.surfaces)).ok_or_else(|| Error::new("snapshot_unavailable", "Controller state is temporarily unavailable")),
        Request::State { page } | Request::Subscribe { page } => {
            state(snapshot, commands, page, limits)
        }
        Request::Job { id } | Request::Cancel { id } => {
            unreachable!("job requests are handled before dispatch: {id}")
        }
        Request::SafeStop {} => {
            commands
                .send(Command::SafetyStop(engine::performance::Safety::Stop))
                .map_err(|e| Error::new("submission_rejected", e))?;
            Ok(json!({"accepted":true,"safety":"stop"}))
        }
        Request::EmergencySilence { confirm } => {
            if !confirm {
                return Err(Error::new(
                    "invalid_operation",
                    "Emergency silence requires confirm: true",
                ));
            }
            commands
                .send(Command::SafetyStop(engine::performance::Safety::Silence))
                .map_err(|e| Error::new("submission_rejected", e))?;
            Ok(json!({"accepted":true,"safety":"silence"}))
        }
        Request::Command { namespace, action } => {
            submit_action(commands, snapshot, limits, namespace, None, action)
        }
        Request::Schedule {
            namespace,
            beat,
            action,
        } => submit_action(commands, snapshot, limits, namespace, Some(beat), action),
        Request::Edit {
            expected,
            target: t,
            action,
        } => {
            let s = snapshot.try_lock_for(limits.snapshot).ok_or_else(|| {
                Error::new(
                    "snapshot_unavailable",
                    "Snapshot is temporarily unavailable",
                )
            })?;
            let layout = s.session.as_ref().ok_or_else(|| {
                Error::new("snapshot_unavailable", "Session identity unavailable")
            })?;
            if expected.namespace.0 != layout.namespace
                || expected.generation.0 != layout.generation
                || expected.revision.0 != s.project_revision
            {
                return Err(Error::new(
                    "conflict",
                    "Reviewed project state changed; read state again",
                ));
            }
            target(layout, t, t.axis.native())?;
            let axis = t.axis.native();
            let id = session::Id(t.id.0);
            let action = match action {
                Edit::Rename { name } => {
                    if name.len() > 1024 {
                        return Err(Error::new("invalid_operation", "Name exceeds 1024 bytes"));
                    }
                    session::Action::Rename { axis, id, name }
                }
                Edit::Color { color } => session::Action::Color { axis, id, color },
                Edit::Move { position } => session::Action::Move { axis, id, position },
            };
            let (request, ack) = session::Request::metadata(layout, s.sampler_epoch, action)
                .map_err(|e| Error::new("invalid_operation", e))?;
            let request = request.at_revision(expected.revision.0, s.sample_rate);
            drop(s);
            submit(commands, Command::SessionEdit(request), ack)
        }
    }
}

/// Prepare an immediate or musical-time control off audio.
/// Takes the port, snapshot, limits, session key, optional beat and action; returns a reconnectable job.
fn submit_action(
    commands: &CommandPort,
    snapshot: &Arc<Mutex<Snapshot>>,
    limits: Limits,
    namespace: Key,
    at: Option<f64>,
    action: Action,
) -> Result<Value, Error> {
    if at.is_some() && matches!(action, Action::DeckControl { .. } | Action::Monitor { .. }) { return Err(Error::new("invalid_schedule", "Deck and monitor gestures require immediate commands")); }
    let s = snapshot.try_lock_for(limits.snapshot).ok_or_else(|| {
        Error::new(
            "snapshot_unavailable",
            "Snapshot is temporarily unavailable",
        )
    })?;
    let layout = s
        .session
        .as_ref()
        .ok_or_else(|| Error::new("snapshot_unavailable", "Session identity unavailable"))?;
    if namespace.0 != layout.namespace {
        return Err(Error::new("conflict", "Project namespace changed"));
    }
    if at.is_some_and(|beat| {
        !beat.is_finite() || !s.playing || beat <= s.beat || beat > s.beat + 16384.0
    }) {
        return Err(Error::new(
            "invalid_schedule",
            "Schedule requires playback and a future beat within 16384 quarter notes",
        ));
    }
    let action = action.prepare(layout)?;
    let ack = Ack::new();
    let request = remote::Request {
        namespace: layout.namespace,
        transport_epoch: s.transport_epoch,
        safety_epoch: commands.performance().safety_epoch(),
        at,
        action,
        ack: ack.clone(),
    };
    drop(s);
    submit(commands, Command::Remote(request), ack)
}

/// Retain and submit a prepared operation.
/// Takes the native command and receipt; returns a job ID only after bounded admission succeeds.
fn submit(commands: &CommandPort, command: Command, ack: Ack) -> Result<Value, Error> {
    let id = commands
        .remote_jobs()
        .insert(ack.clone())
        .map_err(|e| Error::new("job_capacity", e))?;
    if let Err(error) = commands.send(command) {
        ack.cancel();
        return Err(Error::new("submission_rejected", error));
    }
    Ok(json!({"accepted":true,"job":id,"status":"pending"}))
}

/// Inspect or cancel a retained operation.
/// Takes the port, job ID and cancellation intent; returns its honest current completion state.
fn job(commands: &CommandPort, id: String, cancel: bool) -> Result<Value, Error> {
    let ack = commands.remote_jobs().get(&id).ok_or_else(|| {
        Error::new(
            "job_expired",
            "Job is not retained; pending jobs are never evicted",
        )
    })?;
    if cancel && !ack.cancel() {
        return Err(Error::new(
            "cancel_conflict",
            "Job already completed or audio has claimed its commit",
        ));
    }
    let outcome = ack.state();
    let status = match outcome {
        Outcome::Pending => "pending",
        Outcome::Applied => "applied",
        Outcome::Rejected => "rejected",
        Outcome::Cancelled => "cancelled",
    };
    Ok(
        json!({"job":id,"status":status,"error_code":(outcome==Outcome::Rejected).then_some("not_applied")}),
    )
}

/// Parse and execute one versioned request.
/// Takes an API envelope and native services; returns a bounded reply and optional subscription page.
pub(crate) fn reply(
    value: &Value,
    commands: &CommandPort,
    snapshot: &Arc<Mutex<Snapshot>>,
    limits: Limits,
    stream: bool,
) -> (Value, Option<Page>) {
    let mut id = Value::Null;
    let mut page = None;
    let result = (|| {
        id = crate::ipc_request_id(value).map_err(|e| Error::new("invalid_id", e))?;
        if value["op"] != "api" {
            return Err(Error::new(
                "invalid_operation",
                "OSC accepts only versioned API envelopes",
            ));
        }
        let version = value["version"].as_u64().ok_or_else(|| {
            Error::new(
                "invalid_operation",
                "API version must be an unsigned integer",
            )
        })?;
        if version != u64::from(VERSION) {
            return Err(Error::new(
                "unsupported_version",
                "Only automation version 1 is supported",
            ));
        }
        let crate::ipc_schema::Operation::Api { request, .. } =
            crate::ipc_schema::Operation::parse(value)
                .map_err(|e| Error::new("invalid_operation", e))?
        else {
            unreachable!("validated API envelope")
        };
        match request {
            Request::Job { id } => job(commands, id, false),
            Request::Cancel { id } => job(commands, id, true),
            Request::Subscribe { page: p } => {
                if !stream {
                    return Err(Error::new(
                        "unsupported_transport",
                        "Subscriptions require a Unix stream",
                    ));
                }
                let state = state(snapshot, commands, p, limits)?;
                page = Some(p);
                Ok(state)
            }
            request => dispatch(request, commands, snapshot, limits),
        }
    })();
    (
        match result {
            Ok(result) => json!({"ok":true,"id":id,"version":VERSION,"result":result}),
            Err(error) => {
                json!({"ok":false,"id":id,"version":VERSION,"error_code":error.code,"error":error.message})
            }
        },
        page,
    )
}

/// Serve one API request or a bounded read-only subscription.
/// Takes the socket, envelope and native services; returns whether this connection became a subscription.
pub(crate) fn serve(
    writer: &mut UnixStream,
    value: &Value,
    commands: &CommandPort,
    snapshot: &Arc<Mutex<Snapshot>>,
    limits: Limits,
    stopped: Option<&AtomicBool>,
) -> anyhow::Result<bool> {
    let (response, page) = reply(value, commands, snapshot, limits, true);
    ipc_transport::reply(writer, &response, limits.write)?;
    let Some(page) = page else {
        return Ok(false);
    };
    while !stopped.is_some_and(|stop| stop.load(Ordering::Acquire)) {
        std::thread::sleep(Duration::from_millis(250));
        let result = state(snapshot, commands, page, limits);
        let response = match result {
            Ok(state) => {
                json!({"ok":true,"id":response["id"],"version":VERSION,"event":"state","result":state})
            }
            Err(error) => {
                json!({"ok":false,"id":response["id"],"version":VERSION,"error_code":error.code,"error":error.message})
            }
        };
        ipc_transport::reply(writer, &response, limits.write)?;
    }
    Ok(true)
}

#[cfg(test)]
mod tests;

/// Stream native version-1 state to a CLI consumer.
/// Takes the verified socket path, subscribe envelope and output; returns a transport error on disconnect.
pub(crate) fn follow(
    path: &std::path::Path,
    request: &Value,
    output: &mut impl std::io::Write,
) -> anyhow::Result<()> {
    use std::io::BufReader;
    let request = crate::ipc_request::encode(request)?;
    let mut stream = crate::instance::connect(path)?;
    ipc_transport::write_all(
        &mut stream,
        request.line.as_bytes(),
        Duration::from_millis(800),
    )?;
    let mut reader = BufReader::new(stream);
    let mut line = [0; ipc_transport::RESPONSE_BYTES];
    loop {
        let size = ipc_transport::read_line(
            &mut reader,
            &mut line,
            Duration::from_secs(2),
            Duration::from_secs(2),
        )?
        .ok_or_else(|| anyhow::anyhow!("API subscription disconnected"))?;
        let frame: Value = serde_json::from_slice(&line[..size])?;
        anyhow::ensure!(
            frame["version"] == VERSION && frame["id"] == request.id,
            "API subscription correlation or version mismatch"
        );
        anyhow::ensure!(
            frame["ok"] == true
                || (frame["ok"] == false && frame["error_code"] == "snapshot_unavailable"),
            "API subscription rejected [{}]: {}",
            frame["error_code"],
            frame["error"]
        );
        output.write_all(&line[..size])?;
        output.write_all(b"\n")?;
        output.flush()?;
    }
}
