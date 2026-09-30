//! Three fixed cells, owned exclusively by writer, reader and exchange slot.
//! Index exchanges transfer ownership; no thread waits, retries or allocates.
use super::Event;
use std::cell::UnsafeCell;
use std::sync::atomic::{AtomicUsize, Ordering::*};
use std::sync::Arc;

struct Cells {
    events: [UnsafeCell<Event>; 3],
    // Low bit = unread value. Remaining bits = exchange-cell index.
    exchange: AtomicUsize,
}

// SAFETY: Only the unique Writer and Reader access cells. Each accesses its
// private index exclusively. AcqRel swaps/CAS exchange a private index for the
// shared one before accessing it; neither side accesses the exchange cell.
// Clearing the unread bit changes no ownership. Handles cannot be cloned.
unsafe impl Sync for Cells {}

pub(super) struct Writer {
    cells: Arc<Cells>,
    index: usize,
    latest: Event,
}
pub(super) struct Reader {
    cells: Arc<Cells>,
    index: usize,
}

pub(super) fn channel() -> (Writer, Reader) {
    let empty = Event::new(0, 0, &[]);
    let cells = Arc::new(Cells {
        events: std::array::from_fn(|_| UnsafeCell::new(empty)),
        exchange: AtomicUsize::new(2 << 1),
    });
    (
        Writer {
            cells: cells.clone(),
            index: 0,
            latest: empty,
        },
        Reader { cells, index: 1 },
    )
}

impl Writer {
    pub(super) fn same_pending_key(&self, event: &Event) -> bool {
        self.cells.exchange.load(Acquire) & 1 != 0
            && self.latest.epoch == event.epoch
            && self.latest.bytes[..2] == event.bytes[..2]
    }
    /// Returns true only if an unread value was actually replaced.
    pub(super) fn publish(&mut self, event: Event) -> bool {
        // SAFETY: this index is writer-owned until the following swap.
        unsafe {
            self.cells.events[self.index].get().write(event);
        }
        self.latest = event;
        let previous = self.cells.exchange.swap((self.index << 1) | 1, AcqRel);
        self.index = previous >> 1;
        previous & 1 != 0
    }
    /// Claims the latest pending value for FIFO flush, unless the reader has
    /// already claimed it. Read from the writer's copy, never the shared cell.
    pub(super) fn take(&mut self) -> Option<Event> {
        let observed = self.cells.exchange.load(Acquire);
        if observed & 1 == 0 {
            return None;
        }
        self.cells
            .exchange
            .compare_exchange(observed, observed & !1, AcqRel, Acquire)
            .ok()
            .map(|_| self.latest)
    }
}

impl Reader {
    pub(super) fn take(&mut self) -> Option<Event> {
        let observed = self.cells.exchange.load(Acquire);
        if observed & 1 == 0 {
            return None;
        }
        let previous = self
            .cells
            .exchange
            .compare_exchange(observed, self.index << 1, AcqRel, Acquire)
            .ok()?;
        self.index = previous >> 1;
        // SAFETY: successful CAS transferred this cell to the reader; writer
        // cannot acquire it until a future Reader::take gives it back.
        Some(unsafe { *self.cells.events[self.index].get() })
    }
}
