use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicU64, Ordering};
use std::sync::Arc;

use crate::collector::alloc::GcAllocation;
use crate::concurrency::lockout::{Lockout, LockoutProvider};
use crate::Scan;

/// Represents a piece of data tracked by the collector
#[derive(Debug)]
pub struct GcData {
    /// lockout to prevent scanning the underlying data while it may be changing
    pub(crate) lockout: Lockout,
    /// have we started deallocating this piece of data yet?
    pub(crate) deallocated: AtomicBool,
    // During what collection was this last marked?
    //     0 if this is a new piece of data
    pub(crate) last_marked: AtomicU64,
    /// a wrapper to manage (ie deallocate) the underlying allocation
    pub(crate) underlying_allocation: GcAllocation,
}

impl LockoutProvider for Arc<GcData> {
    fn provide(&self) -> &Lockout {
        &self.lockout
    }
}

impl GcData {
    pub fn scan_ptr(&self) -> *const dyn Scan {
        self.underlying_allocation.scan_ptr
    }
}

/// There is one `GcHandle` per `Gc<T>`. We need this metadata for collection
#[derive(Debug)]
pub struct GcHandle {
    /// what data is backing this handle
    pub(crate) underlying_data: UnderlyingData,
    // During what collection was this last found in a piece of GcData?
    //     0 if this is a new piece of data
    pub(crate) last_non_rooted: AtomicU64,
}

#[derive(Debug)]
pub enum UnderlyingData {
    Fixed(Arc<GcData>),
    DynamicForAtomic(Arc<AtomicPtr<GcData>>),
}

impl UnderlyingData {
    // Safe only if called when the data is known to be live
    #[inline]
    pub unsafe fn with_data<F: FnOnce(&GcData)>(&self, f: F) {
        match self {
            Self::Fixed(data) => f(&*data),
            Self::DynamicForAtomic(ptr) => {
                let arc_ptr = ptr.load(Ordering::Relaxed);
                f(&*arc_ptr)
            }
        }
    }
}
