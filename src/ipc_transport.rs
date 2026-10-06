//! Finite framing and I/O budgets shared by the server and CLI.
use std::io::{self, BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};

pub const REQUEST_BYTES: usize = 4096;
pub const RESPONSE_BYTES: usize = 8192;
pub const STATUS_MIDI_NAME_BYTES:usize=256;
pub const STATUS_DECK_TITLE_BYTES:usize=1024;
pub const CLIENTS: usize = 8;

#[derive(Clone, Copy)]
pub struct Limits {
    pub idle: Duration,
    pub read: Duration,
    pub write: Duration,
    pub snapshot: Duration,
    pub requests: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            idle: Duration::from_millis(500),
            read: Duration::from_secs(2),
            write: Duration::from_millis(200),
            snapshot: Duration::from_millis(50),
            requests: 32,
        }
    }
}

#[derive(Debug)]
pub enum ReadFailure {
    TooLarge,
    Idle,
    Deadline,
    Io(io::Error),
}
impl ReadFailure {
    pub fn code(&self) -> &'static str {
        match self {
            Self::TooLarge => "request_too_large",
            Self::Idle => "idle_timeout",
            Self::Deadline => "read_deadline",
            Self::Io(_) => "read_failed",
        }
    }
}
impl std::fmt::Display for ReadFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::TooLarge => "IPC line exceeds the byte limit",
            Self::Idle => "IPC connection exceeded the idle read deadline",
            Self::Deadline => "IPC line exceeded the total read deadline",
            Self::Io(_) => "IPC connection read failed",
        })
    }
}
impl std::error::Error for ReadFailure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

// Payload bytes exclude the newline. Storage never grows, even when a peer
// sends an arbitrarily large prefix without a delimiter. A final EOF prefix is
// returned for the protocol parser to validate, retaining prior wire behavior.
pub fn read_line<const N: usize>(
    reader: &mut BufReader<UnixStream>,
    line: &mut [u8; N],
    idle: Duration,
    total: Duration,
) -> Result<Option<usize>, ReadFailure> {
    let deadline = Instant::now() + total;
    let mut used = 0;
    loop {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .filter(|duration| !duration.is_zero())
            .ok_or(ReadFailure::Deadline)?;
        reader
            .get_ref()
            .set_read_timeout(Some(idle.min(remaining)))
            .map_err(ReadFailure::Io)?;
        let buffer = match reader.fill_buf() {
            Ok(buffer) => buffer,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) =>
            {
                return Err(if Instant::now() >= deadline {
                    ReadFailure::Deadline
                } else {
                    ReadFailure::Idle
                });
            }
            Err(error) => return Err(ReadFailure::Io(error)),
        };
        if buffer.is_empty() {
            return Ok((used > 0).then_some(used));
        }
        let newline = buffer.iter().position(|byte| *byte == b'\n');
        let take = newline.unwrap_or(buffer.len());
        if take > N - used {
            return Err(ReadFailure::TooLarge);
        }
        line[used..used + take].copy_from_slice(&buffer[..take]);
        used += take;
        reader.consume(take + usize::from(newline.is_some()));
        if newline.is_some() {
            return Ok(Some(used));
        }
    }
}

pub fn write_all(stream: &mut UnixStream, bytes: &[u8], timeout: Duration) -> io::Result<()> {
    let deadline = Instant::now() + timeout;
    let mut remaining = bytes;
    while !remaining.is_empty() {
        let budget = deadline
            .checked_duration_since(Instant::now())
            .filter(|duration| !duration.is_zero())
            .ok_or_else(|| io::Error::new(io::ErrorKind::TimedOut, "IPC write deadline"))?;
        stream.set_write_timeout(Some(budget))?;
        match stream.write(remaining) {
            Ok(0) => {
                return Err(io::Error::new(
                    io::ErrorKind::WriteZero,
                    "IPC peer stopped accepting data",
                ))
            }
            Ok(written) => remaining = &remaining[written..],
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

pub fn text(value: &str) -> &str {
    short_text(value, 256)
}

pub fn short_text(value: &str, limit: usize) -> &str {
    let mut end = value.len().min(limit);
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    &value[..end]
}

/// Bound a text prefix by its JSON wire size.
/// Takes UTF-8 text and an escaped-byte limit; returns a whole-character prefix excluding the two enclosing quotes.
pub fn short_json_text(value: &str, limit: usize) -> &str {
    let mut bytes = 0;
    let mut end = 0;
    for (offset, character) in value.char_indices() {
        let cost = match character { '"' | '\\' | '\n' | '\r' | '\t' | '\u{8}' | '\u{c}' => 2, '\0'..='\u{1f}' => 6, _ => character.len_utf8() };
        if bytes + cost > limit { break; }
        bytes += cost; end = offset + character.len_utf8();
    }
    &value[..end]
}

pub fn reply(
    stream: &mut UnixStream,
    value: &serde_json::Value,
    timeout: Duration,
) -> io::Result<()> {
    let mut encoded = value.to_string();
    // Fields entering a response are independently capped. Keep this final
    // guard so a future response extension cannot silently remove the limit.
    if encoded.len() > RESPONSE_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "IPC response exceeds byte limit",
        ));
    }
    encoded.push('\n');
    write_all(stream, encoded.as_bytes(), timeout)
}

pub fn reject(
    stream: &mut UnixStream,
    id: serde_json::Value,
    code: &str,
    error: &str,
    timeout: Duration,
) -> io::Result<()> {
    reply(
        stream,
        &serde_json::json!({
            "ok": false, "id": id, "error_code": code,
            "accepted": false, "command_status": "rejected", "error": text(error),
        }),
        timeout,
    )
}
