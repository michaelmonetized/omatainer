use super::*;
struct StopAfterRead<'a> {
    cancel: &'a AtomicBool,
    reads: usize,
}
impl Read for StopAfterRead<'_> {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        self.reads += 1;
        out.fill(0x41);
        self.cancel.store(true, Ordering::Release);
        Ok(out.len())
    }
}
#[test]
fn digest_and_metadata_reads_observe_incremental_cancel_and_preserve_sha256() {
    let cancel = AtomicBool::new(false);
    let mut reader = StopAfterRead {
        cancel: &cancel,
        reads: 0,
    };
    assert!(digest(&mut reader, &cancel)
        .unwrap_err()
        .contains("cancelled"));
    assert_eq!(reader.reads, 1);
    let cancel = AtomicBool::new(false);
    let mut reader = StopAfterRead {
        cancel: &cancel,
        reads: 0,
    };
    assert!(read_bounded(&mut reader, 16384, &cancel)
        .unwrap_err()
        .contains("cancelled"));
    assert_eq!(reader.reads, 1);
    assert_eq!(
        digest(&mut std::io::Cursor::new(b"abc"), &AtomicBool::new(false)).unwrap(),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    assert!(read_bounded(
        &mut std::io::Cursor::new(b"abcd"),
        3,
        &AtomicBool::new(false)
    )
    .unwrap_err()
    .contains("byte limit"));
    assert!(sha256_cancelled(
        Path::new("/home/nonexistent-must-not-open"),
        &AtomicBool::new(true)
    )
    .unwrap_err()
    .contains("cancelled"));
}
