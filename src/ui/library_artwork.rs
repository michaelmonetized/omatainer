//! A bounded worker reads embedded artwork for visible source versions only.
use super::*;
use crate::engine::performance::{Handle, WorkPermit};
use crate::media_location::{Location, Snapshot};
use lofty::{file::TaggedFileExt, picture::PictureType};
use std::{
    fs::OpenOptions,
    io::Cursor,
    os::unix::fs::OpenOptionsExt,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
};

const CACHE_LIMIT: usize = 96;
const PENDING_LIMIT: usize = 8;
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct Key {
    source: LibSource,
    fingerprint: FileFingerprint,
}
struct Job {
    key: Key,
    id: u64,
    cancel: Arc<AtomicBool>,
    work: WorkPermit,
}
struct ResultRow {
    key: Key,
    id: u64,
    cancelled: bool,
    result: Result<Option<egui::ColorImage>, String>,
}
struct Worker {
    request: mpsc::SyncSender<Job>,
    ready: mpsc::Receiver<ResultRow>,
}
struct Entry {
    id: u64,
    seen: u64,
    cancel: Arc<AtomicBool>,
    state: State,
}
enum State {
    Loading,
    Ready(egui::TextureHandle),
    Empty,
    Failed(String),
}
#[derive(Default)]
pub(super) struct Artwork {
    worker: Option<Worker>,
    rows: HashMap<Key, Entry>,
    frame: u64,
    next: u64,
    waiting: bool,
    pub status: String,
}
struct Cancellation<'a>(&'a AtomicBool, &'a WorkPermit);
impl crate::media_tags::Cancellation for Cancellation<'_> {
    fn cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire) || self.1.cancelled()
    }
}
/// Decode supported embedded image bytes within explicit resource limits.
/// Takes PNG or JPEG bytes; returns a thumbnail no larger than 96 by 96 pixels.
fn thumbnail(bytes: &[u8]) -> Result<egui::ColorImage, String> {
    if bytes.len() > 4 * 1024 * 1024 {
        return Err("Artwork exceeds 4 MiB".into());
    }
    let format = image::guess_format(bytes).map_err(|e| e.to_string())?;
    if !matches!(format, image::ImageFormat::Png | image::ImageFormat::Jpeg) {
        return Err("Artwork preview supports embedded PNG and JPEG".into());
    }
    let (width, height) = image::ImageReader::with_format(Cursor::new(bytes), format)
        .into_dimensions()
        .map_err(|e| e.to_string())?;
    if width == 0
        || height == 0
        || width > 4096
        || height > 4096
        || u64::from(width) * u64::from(height) > 8_388_608
    {
        return Err("Artwork exceeds 4096 pixels per side or 8 million pixels".into());
    }
    let mut reader = image::ImageReader::with_format(Cursor::new(bytes), format);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(4096);
    limits.max_image_height = Some(4096);
    limits.max_alloc = Some(64 * 1024 * 1024);
    reader.limits(limits);
    let decoded = reader.decode().map_err(|e| e.to_string())?;
    let image = if width > 96 || height > 96 {
        decoded.thumbnail(96, 96)
    } else {
        decoded
    };
    let rgba = image.to_rgba8();
    Ok(egui::ColorImage::from_rgba_unmultiplied(
        [rgba.width() as usize, rgba.height() as usize],
        rgba.as_raw(),
    ))
}
/// Read only the captured source version's embedded cover.
/// Takes a cancellable worker job; returns validated thumbnail pixels or a truthful absence/error.
fn read(job: &Job) -> Result<Option<egui::ColorImage>, String> {
    let cancel = Cancellation(&job.cancel, &job.work);
    let check = || {
        if crate::media_tags::Cancellation::cancelled(&cancel) {
            Err("Artwork request cancelled".to_string())
        } else {
            Ok(())
        }
    };
    check()?;
    let location = Location::resolve(&job.key.source).map_err(|e| e.to_string())?;
    let before = Snapshot::discover().map_err(|e| e.to_string())?;
    location.recheck_with(&before).map_err(|e| e.to_string())?;
    let access = before.access(&location.path).map_err(|e| e.to_string())?;
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(&location.path)
        .map_err(|e| e.to_string())?;
    if FileFingerprint::from_metadata(&file.metadata().map_err(|e| e.to_string())?)
        != job.key.fingerprint
    {
        return Err("Artwork source changed before reading".into());
    }
    let tagged = crate::media_tags::read_tagged(&mut file, &cancel, true)?;
    check()?;
    let picture = tagged
        .tags()
        .iter()
        .flat_map(|tag| tag.pictures())
        .find(|picture| picture.pic_type() == PictureType::CoverFront)
        .or_else(|| tagged.tags().iter().flat_map(|tag| tag.pictures()).next());
    let image = picture
        .map(|picture| thumbnail(picture.data()))
        .transpose()?;
    check()?;
    let after = Snapshot::discover().map_err(|e| e.to_string())?;
    access
        .check(&after, &location.path)
        .map_err(|e| e.to_string())?;
    location.recheck_with(&after).map_err(|e| e.to_string())?;
    if FileFingerprint::from_metadata(&file.metadata().map_err(|e| e.to_string())?)
        != job.key.fingerprint
        || after
            .inspect(&location)
            .map(|meta| FileFingerprint::from_metadata(&meta))
            .ok()
            != Some(job.key.fingerprint)
    {
        return Err("Artwork source changed while reading".into());
    }
    Ok(image)
}
impl Worker {
    /// Start one bounded embedded-artwork reader.
    /// Takes no arguments; returns request and result mailboxes owned by the library browser.
    fn start() -> Result<Self, String> {
        let (request, input) = mpsc::sync_channel::<Job>(PENDING_LIMIT);
        let (output, ready) = mpsc::sync_channel(PENDING_LIMIT);
        std::thread::Builder::new()
            .name("omatainer-artwork".into())
            .spawn(move || {
                while let Ok(job) = input.recv() {
                    let result = read(&job).map_err(|error| error.chars().take(256).collect());
                    if job.cancel.load(Ordering::Acquire) {
                        continue;
                    }
                    if output
                        .send(ResultRow {
                            key: job.key,
                            id: job.id,
                            cancelled: job.work.cancelled(),
                            result,
                        })
                        .is_err()
                    {
                        break;
                    }
                }
            })
            .map_err(|e| e.to_string())?;
        Ok(Self { request, ready })
    }
}
impl Artwork {
    /// Poll bounded thumbnail results before drawing visible rows.
    /// Takes the GUI context; uploads at most eight validated images and advances viewport ownership.
    pub fn begin_frame(&mut self, ctx: &egui::Context) {
        self.frame = self.frame.checked_add(1).expect("artwork frame exhausted");
        self.waiting = false;
        if let Some(worker) = &self.worker {
            for _ in 0..PENDING_LIMIT {
                let Ok(row) = worker.ready.try_recv() else {
                    break;
                };
                let Some(entry) = self
                    .rows
                    .get_mut(&row.key)
                    .filter(|entry| entry.id == row.id && !entry.cancel.load(Ordering::Acquire))
                else {
                    continue;
                };
                if row.cancelled {
                    self.rows.remove(&row.key);
                    continue;
                }
                entry.state = match row.result {
                    Ok(Some(image)) => State::Ready(ctx.load_texture(
                        format!("library-artwork-{}", row.id),
                        image,
                        egui::TextureOptions::LINEAR,
                    )),
                    Ok(None) => State::Empty,
                    Err(error) => State::Failed(error),
                };
            }
        }
    }
    /// Resolve a visible artwork preview without doing filesystem work.
    /// Takes source identity, performance owner and read permission; returns a texture and concise status.
    pub fn visible(
        &mut self,
        item: &LibItem,
        performance: &Handle,
        allowed: bool,
    ) -> (Option<(egui::TextureId, Vec2)>, String) {
        let Some(fingerprint) = item.fingerprint else {
            return (None, "Artwork needs a scanned source version".into());
        };
        if !matches!(
            item.source,
            LibSource::File(_) | LibSource::Removable { .. }
        ) {
            return (None, "No embedded artwork for this source".into());
        }
        let key = Key {
            source: item.source.clone(),
            fingerprint,
        };
        if let Some(entry) = self.rows.get_mut(&key) {
            entry.seen = self.frame;
            return match &entry.state {
                State::Ready(texture) => (
                    Some((texture.id(), texture.size_vec2())),
                    "Embedded artwork".into(),
                ),
                State::Empty => (None, "No embedded artwork".into()),
                State::Loading => (None, "Reading embedded artwork…".into()),
                State::Failed(error) => (None, format!("Artwork: {error}")),
            };
        }
        if !allowed || performance.protected() {
            return (None, "Artwork reads deferred by protection".into());
        }
        if self
            .rows
            .values()
            .filter(|entry| matches!(entry.state, State::Loading))
            .count()
            >= PENDING_LIMIT
        {
            self.waiting = true;
            return (None, "Artwork is waiting for the reader".into());
        }
        if self.worker.is_none() {
            match Worker::start() {
                Ok(worker) => self.worker = Some(worker),
                Err(error) => {
                    self.status = error;
                    return (None, self.status.clone());
                }
            }
        }
        let work = match performance.optional_work() {
            Ok(work) => work,
            Err(error) => return (None, error.to_string()),
        };
        self.next = self.next.checked_add(1).expect("artwork request exhausted");
        let cancel = Arc::new(AtomicBool::new(false));
        if self
            .worker
            .as_ref()
            .unwrap()
            .request
            .try_send(Job {
                key: key.clone(),
                id: self.next,
                cancel: cancel.clone(),
                work,
            })
            .is_err()
        {
            self.waiting = true;
            return (None, "Artwork is waiting for the reader".into());
        }
        if self.rows.len() == CACHE_LIMIT {
            let oldest = self
                .rows
                .iter()
                .min_by_key(|(_, entry)| entry.seen)
                .map(|(key, _)| key.clone())
                .unwrap();
            self.rows
                .remove(&oldest)
                .unwrap()
                .cancel
                .store(true, Ordering::Release);
        }
        self.rows.insert(
            key,
            Entry {
                id: self.next,
                seen: self.frame,
                cancel,
                state: State::Loading,
            },
        );
        (None, "Reading embedded artwork…".into())
    }
    /// Retry artwork after a user changes read access or source availability.
    /// Takes this bounded cache; cancels pending reads and releases old thumbnails before visible rows request fresh work.
    pub fn retry(&mut self) {
        for entry in self.rows.values() {
            entry.cancel.store(true, Ordering::Release);
        }
        self.rows.clear();
        self.status.clear();
    }
    /// Cancel invisible pending work while retaining a bounded thumbnail cache.
    /// Takes the current viewport ownership; requests another frame only while visible reads remain.
    pub fn end_frame(&mut self, ctx: &egui::Context) {
        self.rows.retain(|_, entry| {
            let keep = !matches!(entry.state, State::Loading) || entry.seen == self.frame;
            if !keep {
                entry.cancel.store(true, Ordering::Release);
            }
            keep
        });
        if self.waiting
            || self
                .rows
                .values()
                .any(|entry| matches!(entry.state, State::Loading))
        {
            ctx.request_repaint_after(std::time::Duration::from_millis(30));
        }
    }
}
impl Drop for Artwork {
    fn drop(&mut self) {
        for entry in self.rows.values() {
            entry.cancel.store(true, Ordering::Release);
        }
    }
}
#[cfg(test)]
mod tests;
