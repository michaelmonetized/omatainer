//! Count only the current test thread; parallel tests cannot pollute a sample.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Counts {
    pub allocations: usize,
    pub frees: usize,
    pub bytes: usize,
}

thread_local! {
    static COUNTS: Cell<Option<Counts>> = const { Cell::new(None) };
}

struct CountingAllocator;

fn record(bytes: usize, free: bool) {
    let _ = COUNTS.try_with(|slot| {
        if let Some(mut counts) = slot.get() {
            counts.allocations += usize::from(!free);
            counts.frees += usize::from(free);
            counts.bytes += bytes;
            slot.set(Some(counts));
        }
    });
}

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        record(layout.size(), false);
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        record(layout.size(), false);
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        record(size, false);
        record(0, true);
        unsafe { System.realloc(ptr, layout, size) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        record(0, true);
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

pub(super) fn measure(f: impl FnOnce()) -> Counts {
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            COUNTS.with(|slot| slot.set(None));
        }
    }
    COUNTS.with(|slot| {
        assert!(slot.get().is_none(), "allocation measurements cannot nest");
        slot.set(Some(Counts::default()));
    });
    let _reset = Reset;
    f();
    COUNTS.with(|slot| slot.get().unwrap())
}
