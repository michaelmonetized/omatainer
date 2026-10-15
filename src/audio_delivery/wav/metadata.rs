use super::{headers, Container};
use std::{
    fs::OpenOptions,
    io::{Read, Seek, SeekFrom, Write},
    os::unix::fs::OpenOptionsExt,
    path::Path,
};

/// Attach the final measured report without moving audio frames.
/// Args: authored native container and JSON report. Returns after rewriting only the fixed metadata slot and syncing the file.
pub(crate) fn embed(path: &Path, report: &serde_json::Value) -> Result<(), String> {
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
        .map_err(|e| e.to_string())?;
    let mut first = [0; 12];
    file.read_exact(&mut first).map_err(|e| e.to_string())?;
    let big = &first[..4] == b"FORM" && &first[8..] == b"AIFF";
    if !big
        && !([b"RIFF", b"RF64"].contains(&first[..4].try_into().unwrap()) && &first[8..] == b"WAVE")
    {
        return Err("Embedded report requires authored WAVE/RF64/AIFF".into());
    }
    let container = if big {
        Container::Aiff
    } else {
        Container::Wave
    };
    let bytes = headers::report_bytes(report, container)?;
    let mut offset = 12_u64;
    while offset < 65536 {
        file.seek(SeekFrom::Start(offset))
            .map_err(|e| e.to_string())?;
        let mut head = [0; 8];
        file.read_exact(&mut head).map_err(|e| e.to_string())?;
        let size = if big {
            u32::from_be_bytes(head[4..].try_into().unwrap())
        } else {
            u32::from_le_bytes(head[4..].try_into().unwrap())
        };
        if &head[..4] == if big { b"ANNO" } else { b"iXML" } {
            if size as usize != headers::REPORT_BYTES {
                return Err("Authored report slot has changed".into());
            }
            file.write_all(&bytes)
                .and_then(|_| file.sync_all())
                .map_err(|e| e.to_string())?;
            return Ok(());
        }
        if &head[..4] == b"data" || &head[..4] == b"SSND" {
            break;
        }
        offset = offset
            .checked_add(8 + u64::from(size) + u64::from(size & 1))
            .ok_or("Metadata offset overflow")?;
    }
    Err("Native file has no authored embedded report slot".into())
}
