use super::*;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::{Read, Write};
use std::path::Path;

const MAGIC: &[u8; 8] = b"OMATJNL1";
const HEADER: usize = 8 + 8 + 8 + 32;
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Body {
    pub schema: u32,
    pub checkpoint: bool,
    pub metadata: RecordMeta,
    pub media: Vec<String>,
    pub state: Value,
}
#[derive(Debug)]
pub(super) struct Record {
    pub sequence: u64,
    pub digest: [u8; 32],
    pub previous: [u8; 32],
    pub body: Body,
}
#[derive(Debug)]
pub(super) struct ReadResult {
    pub records: Vec<Record>,
    pub warning: Option<String>,
}
struct Bounded(Vec<u8>);
impl Write for Bounded {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if self
            .0
            .len()
            .checked_add(bytes.len())
            .is_none_or(|len| len > RECORD_LIMIT)
        {
            return Err(std::io::Error::other(
                "recovery record metadata exceeds 8 MiB",
            ));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
pub(super) fn json<T: Serialize>(value: &T) -> Result<Vec<u8>, Error> {
    let mut output = Bounded(Vec::new());
    serde_json::to_writer(&mut output, value).map_err(|e| Error::invalid(e.to_string()))?;
    Ok(output.0)
}
pub(super) fn frame(
    sequence: u64,
    previous: [u8; 32],
    body: &Body,
) -> Result<(Vec<u8>, [u8; 32]), Error> {
    let json = json(body)?;
    let mut bytes = Vec::with_capacity(HEADER + json.len() + 32);
    bytes.extend_from_slice(MAGIC);
    bytes.extend_from_slice(&sequence.to_le_bytes());
    bytes.extend_from_slice(&(json.len() as u64).to_le_bytes());
    bytes.extend_from_slice(&previous);
    bytes.extend_from_slice(&json);
    let digest: [u8; 32] = Sha256::digest(&bytes).into();
    bytes.extend_from_slice(&digest);
    Ok((bytes, digest))
}
pub(super) fn read(path: &Path, cancel: &AtomicBool) -> Result<ReadResult, Error> {
    let mut file = files::open(path, false, false)?;
    let len = file
        .metadata()
        .map_err(|e| Error::io("inspect journal", e))?
        .len();
    if len > SEGMENT_LIMIT {
        return Err(Error::invalid(
            "journal segment exceeds 64 MiB replay bound",
        ));
    }
    let mut result = ReadResult {
        records: Vec::new(),
        warning: None,
    };
    while result.records.len() < RECORDS_LIMIT as usize {
        check(cancel)?;
        match read_one(&mut file, cancel) {
            Ok(None) => return Ok(result),
            Ok(Some(record)) => {
                if let Some(previous) = result.records.last() {
                    if record.body.checkpoint
                        || Some(record.sequence) != previous.sequence.checked_add(1)
                        || record.previous != previous.digest
                        || record.body.metadata.epoch != previous.body.metadata.epoch
                    {
                        result.warning = Some(
                            "Journal ordering error; stopped at prior valid edit state".into(),
                        );
                        break;
                    }
                } else if !record.body.checkpoint
                    || path
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .and_then(|s| s.parse::<u64>().ok())
                        != Some(record.sequence)
                {
                    result.warning = Some("Journal does not start with a checkpoint".into());
                    break;
                }
                result.records.push(record);
            }
            Err(Error::Cancelled) => return Err(Error::Cancelled),
            Err(error) => {
                result.warning = Some(format!("{error}; retained prior valid edit state"));
                break;
            }
        }
    }
    if result.records.len() == RECORDS_LIMIT as usize {
        let mut tail = [0];
        if file
            .read(&mut tail)
            .map_err(|e| Error::io("check journal tail", e))?
            != 0
        {
            result.warning =
                Some("Journal exceeds 256-record replay bound; later data ignored".into());
        }
    }
    Ok(result)
}
// Verify each bounded frame before exposing its state to the replay scanner.
fn read_one(file: &mut File, cancel: &AtomicBool) -> Result<Option<Record>, Error> {
    let mut header = [0; HEADER];
    let first = file
        .read(&mut header[..1])
        .map_err(|e| Error::io("read journal", e))?;
    if first == 0 {
        return Ok(None);
    }
    file.read_exact(&mut header[1..])
        .map_err(|e| Error::io("read journal header", e))?;
    if &header[..8] != MAGIC {
        return Err(Error::invalid("unknown journal version or malformed frame"));
    }
    let sequence = u64::from_le_bytes(header[8..16].try_into().unwrap());
    let size = u64::from_le_bytes(header[16..24].try_into().unwrap());
    if size > RECORD_LIMIT as u64 || sequence == 0 {
        return Err(Error::invalid("invalid journal frame length or sequence"));
    }
    let mut body = vec![0; size as usize];
    for chunk in body.chunks_mut(IO_CHUNK) {
        check(cancel)?;
        file.read_exact(chunk)
            .map_err(|e| Error::io("read journal payload", e))?;
    }
    let mut expected = [0; 32];
    file.read_exact(&mut expected)
        .map_err(|e| Error::io("read journal digest", e))?;
    let mut hash = Sha256::new();
    hash.update(header);
    hash.update(&body);
    let actual: [u8; 32] = hash.finalize().into();
    if expected != actual {
        return Err(Error::invalid("journal SHA256 mismatch"));
    }
    let body: Body =
        serde_json::from_slice(&body).map_err(|e| Error::invalid(format!("journal JSON: {e}")))?;
    body.metadata.validate()?;
    if body.schema != 1
        || body.media.len() > crate::project_file::DEFAULT_MEDIA_LIMIT
        || body.media.iter().any(|id| !files::valid_hash(id))
    {
        return Err(Error::invalid(
            "unsupported journal schema or malformed media identity",
        ));
    }
    Ok(Some(Record {
        sequence,
        digest: actual,
        previous: header[24..56].try_into().unwrap(),
        body,
    }))
}
