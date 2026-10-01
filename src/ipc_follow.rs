//! Read-only, rate-limited status subscriptions. Neither side queues updates.
use crate::engine::audio_metrics::AudioMetrics;
use crate::engine::{CommandPort, CommandStats, MidiClockInput, Snapshot, SubmissionStats};
use crate::ipc_transport::{self, Limits};
use anyhow::Context;
use parking_lot::Mutex;
use serde::Serialize;
use serde_json::{json, Value};
use std::io::{self, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

pub(crate) const INTERVAL: Duration = Duration::from_millis(250);
const CLIENT_IO: Duration = Duration::from_secs(2);
const MAX_BACKOFF: Duration = Duration::from_secs(4);

#[derive(Clone, Copy, PartialEq, Serialize)]
struct MasterFx {
    types: [crate::engine::FxKind; 3],
    wet: [f32; 3],
}

#[derive(Clone, Copy, PartialEq, Serialize)]
struct SmallStatus {
    playing: bool,
    recording: bool,
    bpm: f32,
    bar: u32,
    beat: f32,
    xfader: f32,
    master_fx: MasterFx,
    midi_clock: MidiClockInput,
    audio: AudioMetrics,
    commands: CommandStats,
    submissions: SubmissionStats,
    #[serde(rename = "deckAPlaying")]
    deck_a_playing: bool,
    #[serde(rename = "deckBPlaying")]
    deck_b_playing: bool,
}
impl SmallStatus {
    fn capture(snapshot: &Snapshot, commands: &CommandPort) -> Self {
        Self {
            playing: snapshot.playing,
            recording: snapshot.recording,
            bpm: snapshot.bpm,
            bar: snapshot.bar,
            beat: snapshot.beat_in_bar,
            xfader: snapshot.xfader,
            master_fx: MasterFx {
                types: snapshot.fx_kind,
                wet: snapshot.fx_wet,
            },
            midi_clock: snapshot.midi_clock,
            audio: commands.audio_metrics(),
            commands: snapshot.commands,
            submissions: commands.stats(),
            deck_a_playing: snapshot.decks.first().is_some_and(|d| d.playing),
            deck_b_playing: snapshot.decks.get(1).is_some_and(|d| d.playing),
        }
    }
}

#[derive(Default, Serialize)]
struct Metadata {
    midi: Vec<String>,
    #[serde(rename = "deckA")]
    deck_a: String,
    #[serde(rename = "deckB")]
    deck_b: String,
    state_truncated: bool,
}
impl Metadata {
    fn refresh(&mut self, s: &Snapshot) -> bool {
        let a = s
            .decks
            .first()
            .map(|d| ipc_transport::text(&d.title))
            .unwrap_or("");
        let b = s
            .decks
            .get(1)
            .map(|d| ipc_transport::text(&d.title))
            .unwrap_or("");
        let truncated = s.midi.len() > 8
            || s.midi.iter().take(8).any(|s| s.len() > 64)
            || s.decks.iter().take(2).any(|d| d.title.len() > 256);
        let changed = self.deck_a != a
            || self.deck_b != b
            || self.state_truncated != truncated
            || self.midi.len() != s.midi.len().min(8)
            || self
                .midi
                .iter()
                .zip(&s.midi)
                .any(|(a, b)| a != ipc_transport::short_text(b, 64));
        if changed {
            self.deck_a.clear();
            self.deck_a.push_str(a);
            self.deck_b.clear();
            self.deck_b.push_str(b);
            self.midi.clear();
            self.midi.extend(
                s.midi
                    .iter()
                    .take(8)
                    .map(|s| ipc_transport::short_text(s, 64).to_owned()),
            );
            self.state_truncated = truncated;
        }
        changed
    }
}

#[derive(Serialize)]
struct Frame<'a> {
    ok: bool,
    id: &'a Value,
    accepted: Option<bool>,
    command_status: Option<&'static str>,
    event: &'static str,
    state_available: bool,
    follow: bool,
    #[serde(flatten)]
    status: &'a SmallStatus,
    #[serde(flatten)]
    metadata: &'a Metadata,
}

/// Only bounded display metadata and scalar telemetry are copied. Changes to
/// tracks, clip notes, waveform arrays, or unrelated snapshot fields do no work.
#[derive(Default)]
pub(super) struct Cache {
    small: Option<SmallStatus>,
    metadata: Metadata,
    encoded: String,
    #[cfg(test)]
    pub serializations: usize,
}
impl Cache {
    pub(super) fn update(
        &mut self,
        s: &Snapshot,
        commands: &CommandPort,
        id: &Value,
    ) -> io::Result<&str> {
        let small = SmallStatus::capture(s, commands);
        let metadata_changed = self.metadata.refresh(s);
        if self.small != Some(small) || metadata_changed {
            let encoded = serde_json::to_string(&Frame {
                ok: true,
                id,
                accepted: None,
                command_status: None,
                event: "state",
                state_available: true,
                follow: true,
                status: &small,
                metadata: &self.metadata,
            })?;
            self.encoded = framed(encoded)?;
            self.small = Some(small);
            #[cfg(test)]
            {
                self.serializations += 1;
            }
        }
        Ok(&self.encoded)
    }
}
fn framed(mut encoded: String) -> io::Result<String> {
    if encoded.len() > ipc_transport::RESPONSE_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "IPC response exceeds byte limit",
        ));
    }
    encoded.push('\n');
    Ok(encoded)
}

pub(super) fn serve(
    mut stream: UnixStream,
    commands: CommandPort,
    snapshot: Arc<Mutex<Snapshot>>,
    id: Value,
    limits: Limits,
) -> anyhow::Result<()> {
    let mut cache = Cache::default();
    // Contention is a temporary status error, never a command receipt. Keep the
    // subscription instead of creating a new thread for the next status try.
    let unavailable = framed(
        json!({
            "ok": false, "id": id, "accepted": null, "command_status": null,
            "follow": true, "event": "state", "state_available": false,
            "error_code": "snapshot_unavailable", "error": "snapshot temporarily unavailable",
        })
        .to_string(),
    )?;
    loop {
        let frame = if let Some(s) = snapshot.try_lock_for(limits.snapshot) {
            cache.update(&s, &commands, &id)?
        } else {
            &unavailable
        };
        ipc_transport::write_all(&mut stream, frame.as_bytes(), limits.write)?;
        // A slow writer cannot accumulate a queue or catch-up burst. Server
        // shutdown closes its duplicate socket; this wait adds at most 250ms.
        std::thread::sleep(INTERVAL);
    }
}

struct Subscription {
    reader: BufReader<UnixStream>,
    id: Value,
    line: [u8; ipc_transport::RESPONSE_BYTES],
}
impl Subscription {
    fn connect(path: &Path) -> anyhow::Result<Self> {
        let mut stream = crate::instance::connect(path).context("could not connect to status subscription")?;
        let request = crate::ipc_request::encode(&crate::ipc_request::ReadOperation::Follow)?;
        ipc_transport::write_all(&mut stream, request.line.as_bytes(), CLIENT_IO)?;
        let id = request.id;
        Ok(Self {
            reader: BufReader::with_capacity(ipc_transport::RESPONSE_BYTES, stream),
            id,
            line: [0; ipc_transport::RESPONSE_BYTES],
        })
    }
    fn next(&mut self) -> anyhow::Result<&str> {
        let size =
            ipc_transport::read_line(&mut self.reader, &mut self.line, CLIENT_IO, CLIENT_IO)?
                .context("status subscription disconnected")?;
        let line = std::str::from_utf8(&self.line[..size]).context(ProtocolFailure("malformed follow response"))?;
        let response: Value = serde_json::from_str(line).context(ProtocolFailure("malformed follow response"))?;
        anyhow::ensure!(
            response.get("id") == Some(&self.id),
            ProtocolFailure("follow response request id mismatch")
        );
        anyhow::ensure!(
            response.get("ok").is_some_and(Value::is_boolean)
                && response.get("follow") == Some(&Value::Bool(true))
                && response.get("event") == Some(&json!("state"))
                && response.get("accepted") == Some(&Value::Null)
                && response.get("command_status") == Some(&Value::Null),
            ProtocolFailure("invalid status subscription response")
        );
        Ok(line)
    }
}

#[derive(Debug)]
struct ProtocolFailure(&'static str);
impl std::fmt::Display for ProtocolFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str(self.0) }
}
impl std::error::Error for ProtocolFailure {}

fn failure_code(error: &anyhow::Error) -> &'static str {
    if error.downcast_ref::<ProtocolFailure>().is_some()
        || matches!(error.downcast_ref::<ipc_transport::ReadFailure>(), Some(ipc_transport::ReadFailure::TooLarge)) {
        "protocol_error"
    } else if error.downcast_ref::<io::Error>().is_some_and(|error|
        matches!(error.kind(), io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused)) {
        "not_running"
    } else {
        "transport_error"
    }
}

fn output(out: &mut impl Write, line: &str) -> io::Result<()> {
    writeln!(out, "{line}")?;
    out.flush()
}

/// No helper threads or unbounded pending work. Kernel process teardown closes
/// the subscription on SIGINT/SIGTERM; a closed stdout pipe exits successfully.
pub(super) fn run(path: &Path, out: &mut impl Write) -> anyhow::Result<()> {
    let mut backoff = INTERVAL;
    loop {
        let error = match Subscription::connect(path) {
            Ok(mut subscription) => loop {
                match subscription.next() {
                    Ok(line) => {
                        if let Err(error) = output(out, line) {
                            return if error.kind() == io::ErrorKind::BrokenPipe {
                                Ok(())
                            } else {
                                Err(error.into())
                            };
                        }
                        backoff = INTERVAL;
                    }
                    Err(error) => break error,
                }
            },
            Err(error) => error,
        };
        // Bounded diagnostics preserve the existing shell follower error shape.
        let line = json!({"ok":false, "event":"state", "accepted":null,
            "command_status":null, "state_available":false, "playing":false,
            "error_code":failure_code(&error),
            "error":ipc_transport::text(&format!("{error:#}"))})
        .to_string();
        if let Err(error) = output(out, &line) {
            return if error.kind() == io::ErrorKind::BrokenPipe {
                Ok(())
            } else {
                Err(error.into())
            };
        }
        std::thread::sleep(backoff);
        backoff = (backoff * 2).min(MAX_BACKOFF);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::BufRead;

    #[test]
    fn subscriber_rejects_receipts_wrong_ids_malformed_and_oversized_frames() {
        for case in 0..8 {
            let (client, mut peer) = UnixStream::pair().unwrap();
            let payload = match case {
                0 => "not-json\n".into(),
                1 => json!({"id":"wrong","ok":true,"follow":true,"event":"state","accepted":null,"command_status":null}).to_string() + "\n",
                2 => json!({"id":"test","ok":true,"follow":true,"event":"state","accepted":true,"command_status":"accepted"}).to_string() + "\n",
                3 => json!({"id":"test","ok":true}).to_string() + "\n",
                4 => "x".repeat(ipc_transport::RESPONSE_BYTES + 1),
                5 => String::new(),
                6 => json!({"id":"test","ok":true,"follow":true,"event":"state","accepted":null,"command_status":null}).to_string() + "\n",
                _ => json!({"id":"test","ok":false,"follow":true,"event":"state","accepted":null,"command_status":null,"error_code":"snapshot_unavailable"}).to_string() + "\n",
            };
            let writer = std::thread::spawn(move || {
                let _ = peer.write_all(payload.as_bytes());
            });
            let mut subscription = Subscription {
                reader: BufReader::new(client),
                id: json!("test"),
                line: [0; ipc_transport::RESPONSE_BYTES],
            };
            assert_eq!(subscription.next().is_ok(), case >= 6, "case={case}");
            writer.join().unwrap();
        }
    }

    #[test]
    fn follower_closed_output_exits_and_drops_subscription_immediately() {
        struct Closed;
        impl Write for Closed {
            fn write(&mut self, _: &[u8]) -> io::Result<usize> {
                Err(io::ErrorKind::BrokenPipe.into())
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let path =
            std::env::temp_dir().join(format!("omatainer-follow-exit-{}.sock", std::process::id()));
        let listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
        let server = std::thread::spawn(move || {
            let (peer, _) = listener.accept().unwrap();
            peer.set_read_timeout(Some(CLIENT_IO)).unwrap();
            let mut reader = BufReader::new(peer);
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            let request: Value = serde_json::from_str(&line).unwrap();
            writeln!(reader.get_mut(), "{}", json!({"ok":true,"id":request["id"],"follow":true,"event":"state","accepted":null,"command_status":null})).unwrap();
            line.clear();
            assert_eq!(reader.read_line(&mut line).unwrap(), 0);
        });
        assert!(run(&path, &mut Closed).is_ok());
        server.join().unwrap();
        std::fs::remove_file(path).unwrap();
    }
}
