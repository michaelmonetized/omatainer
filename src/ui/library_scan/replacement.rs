//! Replacement discovery shares the scanner, leaving the catalog writer free.
use super::*;
use crate::library::{relocation_search, Catalog, Relocate};

pub(super) struct Task {
    pub id: u64,
    pub catalog: Arc<Catalog>,
    pub target: Relocate,
    pub progress: Arc<relocation_search::Progress>,
}
pub(in crate::ui) struct SearchHandle {
    pub id: u64,
    pub progress: Arc<relocation_search::Progress>,
    cancel: Arc<AtomicBool>,
    work: Arc<WorkPermit>,
}
impl SearchHandle {
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Release);
    }
    pub fn cancelled(&self) -> bool {
        self.work.cancelled()
    }
}
impl Drop for SearchHandle {
    fn drop(&mut self) {
        self.cancel();
    }
}
impl LibraryScan {
    pub fn search_replacement(
        &mut self,
        catalog: Arc<Catalog>,
        target: Relocate,
        roots: Vec<PathBuf>,
    ) -> Result<SearchHandle, String> {
        if self.active() {
            return Err("Wait for the current library scan/search or cancel it first".into());
        }
        if roots.is_empty() || roots.len() > relocation_search::MAX_ROOTS {
            return Err("Choose 1–64 search folders".into());
        }
        let work = Arc::new(
            self.performance
                .optional_work()
                .map_err(|e| e.to_string())?,
        );
        if self.requests.is_none() {
            let performance = self.performance.clone();
            let watching = self.watch_enabled;
            let next = self.next_search;
            *self = Self::default();
            self.performance = performance;
            self.watch_enabled = watching;
            self.next_search = next;
        }
        let id = self
            .next_search
            .checked_add(1)
            .ok_or("Replacement search identifier exhausted; restart the application")?;
        let progress = Arc::new(relocation_search::Progress::default());
        let cancel = work.cancel();
        let request = Request {
            replacement: Some(Task {
                id,
                catalog,
                target,
                progress: progress.clone(),
            }),
            performance: self.performance.clone(),
            watch_enabled: self.watch_enabled,
            watch: None,
            work: work.clone(),
            roots,
            kind: Kind::Import,
            baseline: Arc::new(Vec::new()),
            cancel: cancel.clone(),
            progress: Arc::new(Progress::default()),
            options: Options::default(),
        };
        self.requests
            .as_ref()
            .ok_or("Library filesystem worker unavailable")?
            .try_send(request)
            .map_err(|_| "Library filesystem worker unavailable; retry after it finishes")?;
        self.next_search = id;
        self.cancel = cancel.clone();
        self.replacement = None;
        self.state = ScanState::Searching;
        Ok(SearchHandle {
            id,
            progress,
            cancel,
            work,
        })
    }
    pub fn replacement_result(
        &mut self,
        id: u64,
    ) -> Option<Result<Arc<relocation_search::Receipt>, String>> {
        if self
            .replacement
            .as_ref()
            .is_some_and(|(found, _)| *found == id)
        {
            self.replacement.take().map(|(_, result)| result)
        } else {
            None
        }
    }
}
