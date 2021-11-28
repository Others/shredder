use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use crate::collector::alloc::GcAllocation;
use crate::collector::ref_cnt::GcRefCount;
use crate::concurrency::lockout::{Lockout, LockoutProvider};
use crate::Scan;

/// Represents a piece of data tracked by the collector
#[derive(Debug)]
pub struct GcData {
    /// lockout to prevent scanning the underlying data while it may be changing
    pub(crate) lockout: Lockout,
    /// have we started deallocating this piece of data yet?
    pub(crate) deallocated: AtomicBool,
    // reference count
    pub(crate) ref_cnt: GcRefCount,
    /// a wrapper to manage (ie deallocate) the underlying allocation
    pub(crate) underlying_allocation: GcAllocation,
}

impl LockoutProvider for Arc<GcData> {
    fn provide(&self) -> &Lockout {
        &self.lockout
    }
}

impl GcData {
    pub(crate) fn scan_ptr(&self) -> *const dyn Scan {
        self.underlying_allocation.scan_ptr
    }
}
