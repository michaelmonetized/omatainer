//! Ordered, bounded requests for the GUI. The dispatch worker advances a cursor
//! over the GUI's published filtered view before capturing subsequent loads.
//! The raw MIDI callback and audio renderer never enter this producer bridge.
use super::media_source::Selection;
use super::{SubmissionError, SubmissionOutcome, DECKS};
use crossbeam_channel::{bounded, Receiver as QueueReceiver, Sender, TrySendError};
use parking_lot::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

pub const CAPACITY: usize = 16;
pub const PER_FRAME: usize = 8;

/// Implementations resolve immutable source identity from the published view.
/// A retired/unresolved view returns None; it must never substitute another row.
pub trait SelectionView: Send + Sync {
    fn len(&self) -> Option<usize>;
    fn selection(&self, index: usize) -> Option<Arc<Selection>>;
}
/// Resolve one named crate from the published discovery list.
/// Methods take a row index; return the exact saved identity or None for a retired view.
pub(crate) trait CrateView: Send + Sync {
    fn len(&self) -> Option<usize>;
    fn id(&self, index: usize) -> Option<String>;
}
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Stats {
    pub pending: usize,
    pub accepted: u64,
    pub dispatched: u64,
    pub rejected: u64,
}
pub struct LoadRequest {
    pub deck: u8,
    pub selection: Arc<Selection>,
}
pub struct BrowseRequest {
    pub index: usize,
    pub panel: u8,
    pub epoch: u64,
    pub selection: Arc<Selection>,
}
pub struct CrateBrowseRequest {
    pub index: usize,
    pub epoch: u64,
    pub id: String,
}
pub enum Request {
    TrackStart { deck: u8, selection: Arc<Selection> },
    Panel(u8),
    Crate(CrateBrowseRequest),
    CrateReturn(u64),
    Browse(BrowseRequest),
    Load(LoadRequest),
    Prepare(Vec<Arc<Selection>>),
}
#[derive(Default)]
struct Navigation {
    view: Option<Arc<dyn SelectionView>>,
    cursor: usize,
    epoch: u64,
    crates: Option<Arc<dyn CrateView>>,
    crate_cursor: usize,
    crate_epoch: u64,
    crate_return: Option<u64>,
    panel: u8,
    prepared: Option<Arc<dyn SelectionView>>,
    file_view: Option<Arc<dyn SelectionView>>,
    file_cursor: usize,
    prepared_cursor: usize,
    loaded: [Option<(Arc<Selection>, f64)>; DECKS],
}
#[derive(Default)]
struct Shared {
    attached: AtomicBool,
    navigation: Mutex<Navigation>,
    epoch: AtomicU64,
    crate_epoch: AtomicU64,
    accepted: AtomicU64,
    dispatched: AtomicU64,
    rejected: AtomicU64,
}
pub(super) struct Mailbox {
    shared: Arc<Shared>,
    sender: Sender<Request>,
    receiver: Mutex<Option<QueueReceiver<Request>>>,
}
impl Default for Mailbox {
    fn default() -> Self {
        let (sender, receiver) = bounded(CAPACITY);
        Self {
            shared: Arc::new(Shared::default()),
            sender,
            receiver: Mutex::new(Some(receiver)),
        }
    }
}
impl Mailbox {
    pub fn receiver(&self) -> Option<Receiver> {
        let receiver = self.receiver.lock().take()?;
        self.shared.attached.store(true, Ordering::Release);
        Some(Receiver {
            shared: self.shared.clone(),
            receiver,
        })
    }
    fn submit(
        &self,
        make: impl FnOnce(&mut Navigation) -> Result<Request, SubmissionError>,
    ) -> Result<SubmissionOutcome, SubmissionError> {
        let result = if !self.shared.attached.load(Ordering::Acquire) {
            Err(SubmissionError::UiUnavailable)
        } else {
            // Only GUI/control producers and the MIDI dispatch worker use this
            // lock. It orders cursor movement and captured source admission.
            let mut navigation = self.shared.navigation.lock();
            let previous = navigation.cursor;
            let previous_crate = navigation.crate_cursor;
            let previous_panel = navigation.panel;
            let previous_view = navigation.view.clone();
            let previous_file = navigation.file_cursor;
            let previous_prepared = navigation.prepared_cursor;
            match make(&mut navigation).and_then(|request| {
                self.sender.try_send(request).map_err(|error| match error {
                    TrySendError::Full(_) => SubmissionError::UiFull,
                    TrySendError::Disconnected(_) => SubmissionError::UiUnavailable,
                })
            }) {
                Ok(()) => Ok(()),
                Err(error) => {
                    navigation.cursor = previous;
                    navigation.crate_cursor = previous_crate;
                    navigation.panel = previous_panel;
                    navigation.view = previous_view;
                    navigation.file_cursor = previous_file;
                    navigation.prepared_cursor = previous_prepared;
                    Err(error)
                }
            }
        };
        match result {
            Ok(()) => {
                self.shared.accepted.fetch_add(1, Ordering::Relaxed);
                Ok(SubmissionOutcome::Accepted)
            }
            Err(error) => {
                self.shared.rejected.fetch_add(1, Ordering::Relaxed);
                Err(error)
            }
        }
    }
    pub fn load(&self, deck: u8) -> Result<SubmissionOutcome, SubmissionError> {
        self.submit(|navigation| {
            if deck as usize >= DECKS {
                return Err(SubmissionError::InvalidTarget);
            }
            let selection = navigation
                .view
                .as_ref()
                .and_then(|view| view.selection(navigation.cursor))
                .ok_or(SubmissionError::UncapturedSelection)?;
            Ok(Request::Load(LoadRequest { deck, selection }))
        })
    }
    /// Capture track navigation relative to the media actually loaded on a deck.
    /// Takes deck and direction; restarts after three seconds or captures an adjacent source in the current view.
    pub fn track(&self, deck: u8, forward: bool) -> Result<SubmissionOutcome, SubmissionError> {
        self.submit(|navigation| {
            let (loaded, seconds) = navigation.loaded.get(usize::from(deck)).ok_or(SubmissionError::InvalidTarget)?.as_ref().ok_or(SubmissionError::UncapturedSelection)?;
            if !forward && *seconds > 3.0 { return Ok(Request::TrackStart { deck, selection: loaded.clone() }); }
            let view = navigation.view.as_ref().ok_or(SubmissionError::UncapturedSelection)?;
            let len = view.len().ok_or(SubmissionError::UncapturedSelection)?;
            let index = (0..len).find(|&index| view.selection(index).is_some_and(|selection| selection.source == loaded.source && selection.fingerprint == loaded.fingerprint)).ok_or(SubmissionError::UncapturedSelection)?;
            let next = if forward { index.checked_add(1) } else { index.checked_sub(1) }.filter(|index| *index < len).ok_or(SubmissionError::UncapturedSelection)?;
            let selection = view.selection(next).ok_or(SubmissionError::UncapturedSelection)?;
            Ok(Request::Load(LoadRequest { deck, selection }))
        })
    }
    /// Capture one explicit local file for the ordinary GUI loader.
    /// Takes its deck and absolute path; returns bounded admission with the current file identity.
    pub fn load_file(&self, deck: u8, path: std::path::PathBuf) -> Result<SubmissionOutcome, SubmissionError> {
        self.submit(|_| {
            if deck as usize >= DECKS || !path.is_absolute() || path.as_os_str().len() > 3072 {
                return Err(SubmissionError::InvalidTarget);
            }
            let fingerprint = super::media_source::FileFingerprint::read(&path)
                .ok_or(SubmissionError::UncapturedSelection)?;
            let title = path.file_stem().ok_or(SubmissionError::InvalidTarget)?.to_string_lossy().into_owned();
            let selection = Arc::new(Selection { source: super::media_source::LibSource::File(path), title, fingerprint: Some(fingerprint) });
            Ok(Request::Load(LoadRequest { deck, selection }))
        })
    }
    /// Capture selected or filtered upcoming tracks before navigation changes.
    /// Takes the whole-view flag; returns bounded admission without touching the renderer.
    pub fn prepare(&self, all: bool) -> Result<SubmissionOutcome, SubmissionError> {
        self.submit(|navigation| {
            let view = navigation.view.as_ref().ok_or(SubmissionError::UncapturedSelection)?;
            let len = if all { view.len().ok_or(SubmissionError::UncapturedSelection)? } else { 1 };
            if len == 0 || len > 4096 { return Err(SubmissionError::InvalidTarget); }
            let mut selections = Vec::with_capacity(len);
            let mut bytes = 0usize;
            for index in 0..len {
                let selected = view.selection(if all { index } else { navigation.cursor }).ok_or(SubmissionError::UncapturedSelection)?;
                bytes = bytes.saturating_add(selected.bytes());
                if bytes > 2 * 1024 * 1024 { return Err(SubmissionError::PayloadFull); }
                selections.push(selected);
            }
            Ok(Request::Prepare(selections))
        })
    }
    pub fn browse(&self, steps: f32) -> Result<SubmissionOutcome, SubmissionError> {
        self.submit(|navigation| {
            // Browse is a signed count, never an absolute knob position.
            if !steps.is_finite() || steps.fract() != 0.0 || steps.abs() > i32::MAX as f32 {
                return Err(SubmissionError::InvalidTarget);
            }
            let view = navigation
                .view
                .as_ref()
                .ok_or(SubmissionError::UncapturedSelection)?;
            let len = view
                .len()
                .filter(|len| *len > 0)
                .ok_or(SubmissionError::UncapturedSelection)?;
            let cursor = (navigation.cursor as i64)
                .saturating_add(steps as i64)
                .clamp(0, len.saturating_sub(1) as i64) as usize;
            let selection = view
                .selection(cursor)
                .ok_or(SubmissionError::UncapturedSelection)?;
            navigation.cursor = cursor;
            Ok(Request::Browse(BrowseRequest {
                index: cursor,
                panel: navigation.panel,
                epoch: navigation.epoch,
                selection,
            }))
        })
    }
    /// Switch the hardware browser before capturing later knob/load events.
    /// Takes Files (0), Crates (1), or Prepare (2); queues the matching native panel without substituting a later selection.
    pub fn panel(&self, panel: u8) -> Result<SubmissionOutcome, SubmissionError> {
        if panel > 2 { return Err(SubmissionError::InvalidTarget); }
        self.submit(|navigation| {
            if navigation.panel == 2 { navigation.prepared_cursor = navigation.cursor; } else { navigation.file_cursor = navigation.cursor; }
            navigation.panel = panel;
            navigation.view = if panel == 2 { navigation.prepared.clone() } else { navigation.file_view.clone() };
            navigation.cursor = if panel == 2 { navigation.prepared_cursor } else { navigation.file_cursor };
            Ok(Request::Panel(panel))
        })
    }
    /// Browse an exact crate list on a control producer.
    /// Takes signed integral steps; captures a stable crate identity and restores the cursor if the bounded mailbox is full.
    pub fn browse_crates(&self, steps: f32) -> Result<SubmissionOutcome, SubmissionError> {
        self.submit(|navigation| {
            if !steps.is_finite() || steps.fract() != 0.0 || steps.abs() > i32::MAX as f32 { return Err(SubmissionError::InvalidTarget); }
            let view = navigation.crates.as_ref().ok_or(SubmissionError::UncapturedSelection)?;
            let len = view.len().filter(|len| *len > 0 && *len <= 4096).ok_or(SubmissionError::UncapturedSelection)?;
            let previous = if navigation.crate_cursor < len { navigation.crate_cursor as i64 } else if steps >= 0.0 { -1 } else { len as i64 };
            let cursor = previous.saturating_add(steps as i64).clamp(0,len.saturating_sub(1) as i64) as usize;
            let id = view.id(cursor).filter(|id|id.len() == 32 && id.bytes().all(|b|b.is_ascii_digit() || (b'a'..=b'f').contains(&b))).ok_or(SubmissionError::UncapturedSelection)?;
            navigation.crate_cursor = cursor;
            Ok(Request::Crate(CrateBrowseRequest { index:cursor,epoch:navigation.crate_epoch,id }))
        })
    }
    /// Request the currently captured discovery return point.
    /// Takes no arguments; refuses a missing bookmark instead of inventing navigation.
    pub fn return_crate(&self) -> Result<SubmissionOutcome, SubmissionError> {
        self.submit(|navigation|navigation.crate_return.map(Request::CrateReturn).ok_or(SubmissionError::UncapturedSelection))
    }
    pub fn reject_uncaptured(&self) {
        self.shared.rejected.fetch_add(1, Ordering::Relaxed);
    }
    pub fn stats(&self) -> Stats {
        Stats {
            pending: self.sender.len(),
            accepted: self.shared.accepted.load(Ordering::Relaxed),
            dispatched: self.shared.dispatched.load(Ordering::Relaxed),
            rejected: self.shared.rejected.load(Ordering::Relaxed),
        }
    }
}
pub struct Receiver {
    shared: Arc<Shared>,
    receiver: QueueReceiver<Request>,
}
impl Receiver {
    /// Publish the identity and elapsed source time confirmed by the GUI's load receipt.
    /// Takes deck, optional source and seconds; updates bounded navigation state outside the renderer.
    pub(crate) fn publish_deck(&self, deck: usize, selection: Option<&Selection>, seconds: f64) {
        let mut navigation = self.shared.navigation.lock();
        let Some(slot) = navigation.loaded.get_mut(deck) else { return; };
        match selection {
            Some(selection) => {
                if let Some((current, elapsed)) = slot.as_mut().filter(|(current, _)| current.source == selection.source && current.fingerprint == selection.fingerprint) { let _ = current; *elapsed = seconds; }
                else { *slot = Some((Arc::new(selection.clone()), seconds)); }
            }
            None => *slot = None,
        }
    }
    pub fn publish_view(&self, view: Arc<dyn SelectionView>, cursor: usize) {
        let mut navigation = self.shared.navigation.lock();
        navigation.file_view = Some(view.clone());
        navigation.file_cursor = cursor;
        if navigation.panel == 2 { return; }
        navigation.view = Some(view);
        navigation.cursor = cursor;
        navigation.epoch = navigation.epoch.wrapping_add(1);
        self.shared.epoch.store(navigation.epoch, Ordering::Release);
    }
    /// Publish a versioned Prepare queue independently of the main browser.
    /// Takes the immutable queue and selected cursor; refreshes active selection only when Prepare owns browsing.
    pub(crate) fn publish_prepare(&self, view: Arc<dyn SelectionView>, cursor: usize) {
        let mut navigation = self.shared.navigation.lock();
        let changed = !navigation.prepared.as_ref().is_some_and(|old| Arc::ptr_eq(old, &view));
        navigation.prepared = Some(view.clone()); navigation.prepared_cursor = cursor;
        if navigation.panel == 2 {
            navigation.view = Some(view); navigation.cursor = cursor;
            if changed { navigation.epoch = navigation.epoch.wrapping_add(1); self.shared.epoch.store(navigation.epoch, Ordering::Release); }
        }
    }
    /// Publish one exact crate list and its acknowledged selection.
    /// Takes an immutable view and cursor; changes the epoch so pending events cannot retarget later results.
    pub(crate) fn publish_crates(&self, view: Arc<dyn CrateView>, cursor: usize) {
        let mut navigation = self.shared.navigation.lock();
        navigation.crates = Some(view);
        navigation.crate_cursor = cursor;
        navigation.crate_epoch = navigation.crate_epoch.wrapping_add(1);
        self.shared.crate_epoch.store(navigation.crate_epoch,Ordering::Release);
    }
    /// Publish the current return-point identity without resetting ordered browsing.
    /// Takes an optional token; clears controller return availability when the bookmark is gone.
    pub(crate) fn publish_crate_return(&self, token: Option<u64>) { self.shared.navigation.lock().crate_return = token; }
    pub(crate) fn crate_epoch(&self) -> u64 { self.shared.crate_epoch.load(Ordering::Acquire) }
    #[cfg(test)]
    pub(crate) fn with_navigation_held_for_test(&self, action: impl FnOnce()) {
        let _guard = self.shared.navigation.lock();
        action();
    }
    pub fn epoch(&self) -> u64 {
        self.shared.epoch.load(Ordering::Acquire)
    }
    pub fn take_requests(&self) -> [Option<Request>; PER_FRAME] {
        std::array::from_fn(|_| {
            self.receiver.try_recv().ok().inspect(|_| {
                self.shared.dispatched.fetch_add(1, Ordering::Relaxed);
            })
        })
    }
    #[cfg(test)]
    pub fn publish_selection(&self, selection: Option<Arc<Selection>>) {
        struct Single(Option<Arc<Selection>>);
        impl SelectionView for Single {
            fn len(&self) -> Option<usize> {
                Some(usize::from(self.0.is_some()))
            }
            fn selection(&self, index: usize) -> Option<Arc<Selection>> {
                (index == 0).then(|| self.0.clone()).flatten()
            }
        }
        self.publish_view(Arc::new(Single(selection)), 0);
    }
}
impl Drop for Receiver {
    fn drop(&mut self) {
        self.shared.attached.store(false, Ordering::Release);
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    struct NamedView(Vec<Arc<Selection>>);
    impl SelectionView for NamedView {
        fn len(&self) -> Option<usize> { Some(self.0.len()) }
        fn selection(&self, index: usize) -> Option<Arc<Selection>> { self.0.get(index).cloned() }
    }
    fn named(title: &str) -> Arc<Selection> { Arc::new(Selection { source: crate::engine::media_source::LibSource::File(title.into()), title: title.into(), fingerprint: None }) }
    #[test]
    fn ns7_controls_browser_prepare_load_and_track_navigation_capture_exact_sources() {
        let mailbox = Mailbox::default(); let receiver = mailbox.receiver().unwrap();
        let files: Arc<dyn SelectionView> = Arc::new(NamedView(vec![named("One"), named("Two"), named("Three")]));
        receiver.publish_view(files, 0);
        receiver.publish_prepare(Arc::new(NamedView(vec![named("Prepared one"), named("Prepared two")])), 0);
        mailbox.panel(2).unwrap(); mailbox.browse(1.0).unwrap(); mailbox.load(1).unwrap();
        mailbox.panel(0).unwrap(); mailbox.load(0).unwrap();
        let requests = receiver.take_requests();
        assert!(matches!(&requests[2], Some(Request::Load(request)) if request.deck == 1 && request.selection.title == "Prepared two"));
        assert!(matches!(&requests[4], Some(Request::Load(request)) if request.deck == 0 && request.selection.title == "One"));
        receiver.publish_deck(0, Some(&named("Two")), 1.0);
        mailbox.track(0, true).unwrap(); mailbox.track(0, false).unwrap();
        receiver.publish_deck(0, Some(&named("Two")), 10.0); mailbox.track(0, false).unwrap();
        let requests = receiver.take_requests();
        assert!(matches!(&requests[0], Some(Request::Load(request)) if request.selection.title == "Three"));
        assert!(matches!(&requests[1], Some(Request::Load(request)) if request.selection.title == "One"));
        assert!(matches!(&requests[2], Some(Request::TrackStart { deck: 0, selection }) if selection.title == "Two"));
        for _ in 0..CAPACITY { mailbox.panel(2).unwrap(); }
        assert_eq!(mailbox.panel(0), Err(SubmissionError::UiFull));
        for _ in 0..CAPACITY { receiver.take_requests(); }
        mailbox.load(0).unwrap(); assert!(matches!(&receiver.take_requests()[0], Some(Request::Load(request)) if request.selection.title == "Prepared two"));
        assert_eq!(mailbox.track(2, true), Err(SubmissionError::InvalidTarget));
    }

    #[test]
    fn raw_renderer_request_rejects_without_allocating_or_touching_selection_arcs() {
        let (engine, mut renderer) = crate::engine::Engine::headless_for_test(48000, 48);
        engine
            .ui_requests
            .publish_selection(Some(Arc::new(Selection {
                source: crate::engine::media_source::LibSource::File("private.wav".into()),
                title: "private".into(), fingerprint: None, })));
        let counts = crate::engine::test_alloc::measure(|| {
            renderer.apply(crate::engine::Command::DeckLoadSelected { deck: 0 });
            renderer.apply(crate::engine::Command::Browse(1.0));
        });
        assert_eq!(counts.allocations, 0);
        assert_eq!(counts.frees, 0);
        assert_eq!(
            engine.cmd.stats().last_error,
            Some(SubmissionError::UncapturedSelection)
        );
        assert_eq!(engine.cmd.ui_request_stats().pending, 0);
    }
}

#[cfg(test)]
mod crate_tests {
    use super::*;
    struct Crates(usize);
    impl CrateView for Crates {
        fn len(&self) -> Option<usize> { Some(self.0) }
        fn id(&self,index:usize) -> Option<String> { (index<self.0).then(||format!("{index:032x}")) }
    }
    #[test]
    fn crate_bursts_capture_identity_rollback_full_mailbox_and_fence_filter_changes() {
        let mailbox = Mailbox::default();
        let receiver = mailbox.receiver().unwrap();
        receiver.publish_crates(Arc::new(Crates(4096)),0);
        for _ in 0..CAPACITY { assert_eq!(mailbox.browse_crates(1.0),Ok(SubmissionOutcome::Accepted)); }
        assert_eq!(mailbox.browse_crates(1.0),Err(SubmissionError::UiFull));
        for (index,request) in receiver.take_requests().into_iter().enumerate() {
            let Some(Request::Crate(request)) = request else { panic!("missing crate"); };
            assert_eq!(request.id,format!("{:032x}",index+1));
            assert_eq!(request.epoch,receiver.crate_epoch());
        }
        assert_eq!(mailbox.browse_crates(1.0),Ok(SubmissionOutcome::Accepted));
        let queued = receiver.take_requests().into_iter().chain(receiver.take_requests());
        let Some(Request::Crate(last)) = queued.flatten().last() else { panic!("missing appended crate"); };
        assert_eq!(last.index,CAPACITY+1,"failed admission must not advance the dispatch cursor");
        mailbox.browse_crates(1.0).unwrap();
        receiver.publish_crates(Arc::new(Crates(8)),0);
        let Some(Request::Crate(stale)) = receiver.take_requests().into_iter().flatten().next() else { panic!("missing stale crate"); };
        assert_ne!(stale.epoch,receiver.crate_epoch());
        for invalid in [f32::NAN,f32::INFINITY,0.5] { assert_eq!(mailbox.browse_crates(invalid),Err(SubmissionError::InvalidTarget)); }
        assert_eq!(mailbox.return_crate(),Err(SubmissionError::UncapturedSelection));
        receiver.publish_crate_return(Some(77));
        mailbox.return_crate().unwrap();
        assert!(matches!(receiver.take_requests()[0],Some(Request::CrateReturn(77))));
        receiver.publish_crate_return(None);
        assert_eq!(mailbox.return_crate(),Err(SubmissionError::UncapturedSelection));
    }
    #[test]
    fn renderer_crate_requests_reject_without_heap_work() {
        let (engine,mut rt) = crate::engine::Engine::headless_for_test(48000,32);
        let counts = crate::engine::test_alloc::measure(|| {
            rt.apply(crate::engine::Command::BrowseCrates(1.0));
            rt.apply(crate::engine::Command::CrateReturn);
        });
        assert_eq!(counts,crate::engine::test_alloc::Counts::default());
        assert_eq!(engine.cmd.stats().last_error,Some(SubmissionError::UncapturedSelection));
        assert_eq!(engine.cmd.ui_request_stats().pending,0);
    }
}
