//! Optional loopback OSC messages carry the same typed API, never audio-thread IO.
use super::*;
use std::io::Read;
use std::net::UdpSocket;
use std::sync::mpsc::{self, SyncSender};
use std::thread::JoinHandle;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Config {
    pub enabled: bool,
    pub port: u16,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            enabled: false,
            port: 9000,
        }
    }
}
impl Config {
    /// Validate the optional loopback listener.
    /// Takes this configuration; returns an error for privileged nonzero ports.
    pub(crate) fn validate(self) -> Result<(), String> {
        if self.port != 0 && self.port < 1024 {
            Err("OSC port must be zero for automatic selection or 1024..65535".into())
        } else {
            Ok(())
        }
    }
}

#[derive(Clone, Default)]
pub(crate) struct Status {
    pub requested: Config,
    pub applied: Option<Config>,
    pub port: Option<u16>,
    pub token: Option<String>,
    pub error: Option<String>,
    pub pending: bool,
}
struct Worker {
    requests: SyncSender<Config>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            thread.thread().unpark();
            let _ = thread.join();
        }
    }
}

pub(crate) struct Manager {
    commands: CommandPort,
    snapshot: Arc<Mutex<Snapshot>>,
    status: Arc<Mutex<Status>>,
    requested: Option<Config>,
    worker: Option<Worker>,
}
impl Manager {
    /// Create a disabled adapter without opening a socket or starting a thread.
    /// Takes native services; returns a manager ready for an explicitly saved configuration.
    pub(crate) fn new(commands: CommandPort, snapshot: Arc<Mutex<Snapshot>>) -> Self {
        Self {
            commands,
            snapshot,
            status: Arc::new(Mutex::new(Status::default())),
            requested: None,
            worker: None,
        }
    }

    /// Read the actual listener state.
    /// Takes the manager; returns applied settings and whether the latest request is still pending.
    pub(crate) fn status(&self) -> Status {
        let mut status = self.status.lock().clone();
        status.pending = self
            .requested
            .is_some_and(|config| config != status.requested);
        status
    }

    /// Apply a saved listener intent off the interface and audio threads.
    /// Takes the configuration; returns admission failure while preserving the running listener.
    pub(crate) fn configure(&mut self, config: Config) -> Result<(), String> {
        config.validate()?;
        if self.requested == Some(config) && self.status.lock().error.is_none() {
            return Ok(());
        }
        if self.worker.is_none() && !config.enabled {
            *self.status.lock() = Status {
                requested: config,
                applied: Some(config),
                ..Default::default()
            };
            self.requested = Some(config);
            return Ok(());
        }
        if self.worker.is_none() {
            self.worker = Some(start(
                self.commands.clone(),
                self.snapshot.clone(),
                self.status.clone(),
            )?);
        }
        self.worker
            .as_ref()
            .unwrap()
            .requests
            .try_send(config)
            .map_err(|_| "OSC configuration is busy; retry saved settings".to_string())?;
        self.requested = Some(config);
        Ok(())
    }
}

/// Generate a fresh loopback access token.
/// Takes no arguments; returns 128 random bits as hexadecimal or fails closed.
fn token() -> std::io::Result<String> {
    let mut bytes = [0u8; 16];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

/// Start the optional adapter owner.
/// Takes native services and status; returns one bounded control worker owning all network IO.
fn start(
    commands: CommandPort,
    snapshot: Arc<Mutex<Snapshot>>,
    status: Arc<Mutex<Status>>,
) -> Result<Worker, String> {
    let (requests, incoming) = mpsc::sync_channel::<Config>(1);
    let stop = Arc::new(AtomicBool::new(false));
    let stopped = stop.clone();
    let thread = std::thread::Builder::new()
        .name("omatainer-osc".into())
        .spawn(move || {
        let mut socket: Option<UdpSocket> = None;
        let mut access = String::new();
        let mut packet = [0u8; ipc_transport::REQUEST_BYTES + 1];
        while !stopped.load(Ordering::Acquire) {
            if let Ok(config) = incoming.try_recv() {
                if !config.enabled {
                    socket = None;
                    access.clear();
                    *status.lock() = Status {
                        requested: config,
                        applied: Some(config),
                        ..Default::default()
                    };
                } else if status.lock().applied == Some(config) && socket.is_some() {
                    let mut state = status.lock();
                    state.requested = config;
                    state.error = None;
                } else {
                    let prepared = (|| -> std::io::Result<(UdpSocket, String)> {
                        let socket = UdpSocket::bind((std::net::Ipv4Addr::LOCALHOST, config.port))?;
                        socket.set_nonblocking(true)?;
                        Ok((socket, token()?))
                    })();
                    match prepared {
                        Ok((next, next_access)) => {
                            let port = next.local_addr().ok().map(|address| address.port());
                            socket = Some(next);
                            access = next_access;
                            *status.lock() = Status {
                                requested: config,
                                applied: Some(config),
                                port,
                                token: Some(access.clone()),
                                ..Default::default()
                            };
                        }
                        Err(error) => {
                            let mut state = status.lock();
                            state.requested = config;
                            state.error = Some(format!("OSC listener was not changed: {error}"));
                        }
                    }
                }
            }
            if let Some(socket) = &socket {
                match socket.recv_from(&mut packet) {
                    Ok((size, peer)) => {
                        let response=(|| {
                                if !peer.ip().is_loopback() {return Err("permission_denied");}
                                if size>ipc_transport::REQUEST_BYTES {return Err("request_too_large");}
                                let (supplied,payload)=decode(&packet[..size]).map_err(|_|"invalid_osc")?;
                                if !same_token(supplied,&access) {return Err("permission_denied");}
                                let value=serde_json::from_slice::<Value>(payload).map_err(|_|"invalid_json")?;
                                Ok(super::reply(&value,&commands,&snapshot,Limits::default(),false).0)
                            })().unwrap_or_else(|code|json!({"ok":false,"id":null,"version":VERSION,"error_code":code,"error":"OSC request rejected"}));
                        let payload = serde_json::to_vec(&response).unwrap();
                        if payload.len() <= ipc_transport::RESPONSE_BYTES {
                            let _ = socket.send_to(&encode_reply(&payload), peer);
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                    Err(error) => status.lock().error = Some(format!("OSC receive failed: {error}")),
                }
            }
            std::thread::park_timeout(Duration::from_millis(5));
        }
        })
        .map_err(|e| e.to_string())?;
    Ok(Worker {
        requests,
        stop,
        thread: Some(thread),
    })
}

/// Compare fixed-length access tokens without an early differing-byte exit.
/// Takes supplied and active tokens; returns true only for the complete current token.
fn same_token(supplied: &str, active: &str) -> bool {
    supplied.len() == 32
        && active.len() == 32
        && supplied
            .bytes()
            .zip(active.bytes())
            .fold(0, |difference, (a, b)| difference | (a ^ b))
            == 0
}

/// Read one padded OSC string.
/// Takes packet bytes and a cursor; returns ASCII text after checking null terminators and padding.
fn string<'a>(packet: &'a [u8], cursor: &mut usize) -> Result<&'a str, &'static str> {
    let tail = packet.get(*cursor..).ok_or("short OSC string")?;
    let size = tail
        .iter()
        .position(|b| *b == 0)
        .ok_or("unterminated OSC string")?;
    let padded = (size + 4) & !3;
    if tail
        .get(size..padded)
        .is_none_or(|padding| padding.iter().any(|b| *b != 0))
    {
        return Err("invalid OSC padding");
    }
    let text = std::str::from_utf8(&tail[..size]).map_err(|_| "invalid OSC text")?;
    if !text.is_ascii() {
        return Err("OSC strings must be ASCII; JSON uses a UTF-8 blob");
    }
    *cursor += padded;
    Ok(text)
}

/// Read the supported OSC message envelope.
/// Takes one bounded datagram; returns its access token and exact UTF-8 JSON blob.
fn decode(packet: &[u8]) -> Result<(&str, &[u8]), &'static str> {
    if packet.len() % 4 != 0 {
        return Err("OSC packet is not four-byte aligned");
    }
    let mut cursor = 0;
    if string(packet, &mut cursor)? != "/omatainer/v1" || string(packet, &mut cursor)? != ",sb" {
        return Err("Unsupported OSC address or types");
    }
    let token = string(packet, &mut cursor)?;
    let size = u32::from_be_bytes(
        packet
            .get(cursor..cursor + 4)
            .ok_or("short OSC blob")?
            .try_into()
            .unwrap(),
    ) as usize;
    cursor += 4;
    let end = cursor.checked_add(size).ok_or("oversized OSC blob")?;
    let padded = end.checked_add(3).ok_or("oversized OSC blob")? & !3;
    let payload = packet.get(cursor..end).ok_or("short OSC blob")?;
    if padded != packet.len()
        || packet
            .get(end..padded)
            .is_none_or(|padding| padding.iter().any(|b| *b != 0))
    {
        return Err("invalid OSC blob padding or extra arguments");
    }
    Ok((token, payload))
}

/// Append a padded OSC string.
/// Takes an output buffer and ASCII text; appends its null terminator and alignment bytes.
fn push_string(packet: &mut Vec<u8>, text: &str) {
    packet.extend_from_slice(text.as_bytes());
    packet.push(0);
    while packet.len() % 4 != 0 {
        packet.push(0);
    }
}
/// Encode the supported OSC reply.
/// Takes UTF-8 JSON; returns a single /omatainer/v1/reply message with one blob argument.
fn encode_reply(payload: &[u8]) -> Vec<u8> {
    let mut packet = Vec::new();
    push_string(&mut packet, "/omatainer/v1/reply");
    push_string(&mut packet, ",b");
    packet.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    packet.extend_from_slice(payload);
    while packet.len() % 4 != 0 {
        packet.push(0);
    }
    packet
}

#[cfg(test)]
mod tests;
