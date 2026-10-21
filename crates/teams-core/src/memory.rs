//! Counts the Rust heap so Settings › Debug can tell it apart from the Swift side's.
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};

struct Counting;

static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);
static ALLOCATIONS: AtomicUsize = AtomicUsize::new(0);

#[global_allocator]
static ALLOCATOR: Counting = Counting;

fn grew(size: usize) {
    let live = LIVE.fetch_add(size, Relaxed) + size;
    PEAK.fetch_max(live, Relaxed);
}

// SAFETY: every call is forwarded to `System` unchanged; only the counters are extra.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() {
            grew(layout.size());
            ALLOCATIONS.fetch_add(1, Relaxed);
        }
        ptr
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { System.alloc_zeroed(layout) };
        if !ptr.is_null() {
            grew(layout.size());
            ALLOCATIONS.fetch_add(1, Relaxed);
        }
        ptr
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) };
        LIVE.fetch_sub(layout.size(), Relaxed);
        ALLOCATIONS.fetch_sub(1, Relaxed);
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let new = unsafe { System.realloc(ptr, layout, new_size) };
        if !new.is_null() {
            if new_size >= layout.size() {
                grew(new_size - layout.size());
            } else {
                LIVE.fetch_sub(layout.size() - new_size, Relaxed);
            }
        }
        new
    }
}

/// Bytes the Rust core has allocated and not freed yet.
#[derive(Debug, Clone, uniffi::Record)]
pub struct RustMemory {
    pub live_bytes: u64,
    pub peak_bytes: u64,
    pub allocations: u64,
}

#[uniffi::export]
pub fn rust_memory() -> RustMemory {
    RustMemory {
        live_bytes: LIVE.load(Relaxed) as u64,
        peak_bytes: PEAK.load(Relaxed) as u64,
        allocations: ALLOCATIONS.load(Relaxed) as u64,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_live_bytes() {
        let before = rust_memory().peak_bytes;
        let big = vec![0u8; 64 << 20];
        assert!(rust_memory().live_bytes >= big.len() as u64);
        assert!(rust_memory().peak_bytes >= before.max(big.len() as u64));
        drop(big);
    }
}
