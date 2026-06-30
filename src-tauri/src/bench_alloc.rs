//! A counting global allocator used **only** by the uploader performance benchmark
//! (`cargo test --features bench-alloc … --ignored`) to report the peak heap
//! high-water mark of the split/encode pipeline.
//!
//! Zero production impact: the [`#[global_allocator]`](crate) override in `lib.rs`
//! is gated behind the `bench-alloc` feature, so normal builds (the app, release,
//! plain `cargo test`) use the system allocator and never touch these counters.
//! When the feature is off the module still compiles, but [`peak`]/[`current`]
//! simply return 0 (nothing ever updates them).

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

static CURRENT: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

/// A pass-through allocator that tracks live bytes and a high-water mark. Delegates
/// every operation to the system allocator; the only added cost is two relaxed
/// atomics per alloc/dealloc, which is why it lives behind a feature flag.
pub struct TrackingAlloc;

// SAFETY: every method forwards to `System` (a sound `GlobalAlloc`) with the same
// pointer/layout it was given; the atomic bookkeeping never affects the returned
// pointer. `realloc` falls back to the default `GlobalAlloc::realloc`, which is
// implemented in terms of `alloc`/`copy`/`dealloc` and so is accounted here too.
unsafe impl GlobalAlloc for TrackingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = System.alloc(layout);
        if !ptr.is_null() {
            let now = CURRENT.fetch_add(layout.size(), Ordering::Relaxed) + layout.size();
            PEAK.fetch_max(now, Ordering::Relaxed);
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        System.dealloc(ptr, layout);
        CURRENT.fetch_sub(layout.size(), Ordering::Relaxed);
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let ptr = System.alloc_zeroed(layout);
        if !ptr.is_null() {
            let now = CURRENT.fetch_add(layout.size(), Ordering::Relaxed) + layout.size();
            PEAK.fetch_max(now, Ordering::Relaxed);
        }
        ptr
    }
}

/// Reset the high-water mark down to the current live byte count. Call this
/// immediately before a measured region so [`peak`] reflects only that region.
pub fn reset_peak() {
    PEAK.store(CURRENT.load(Ordering::Relaxed), Ordering::Relaxed);
}

/// The peak live heap bytes observed since the last [`reset_peak`] (0 when the
/// `bench-alloc` feature is off — nothing updates the counter).
pub fn peak() -> usize {
    PEAK.load(Ordering::Relaxed)
}

/// The current live heap bytes (0 when the `bench-alloc` feature is off).
pub fn current() -> usize {
    CURRENT.load(Ordering::Relaxed)
}
