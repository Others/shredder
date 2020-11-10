use std::prelude::v1::*;
use std::sync::atomic::{AtomicU64, Ordering};

#[cfg(feature = "threads")]
use std::thread::yield_now;

const SENTINEL_VALUE: u64 = 1 << 60;

pub struct AtomicProtectingSpinlock {
    /// tracks who is holding the spinlock
    /// if zero, no guards exists
    /// if between zero and SENTINEL_VALUE, there are only inclusive guards
    /// if between SENTINEL_VALUE and u64::max, there is only a single exclusive guard
    tracker: AtomicU64,
}

impl AtomicProtectingSpinlock {
    pub fn new() -> Self {
        Self {
            tracker: AtomicU64::new(0),
        }
    }

    pub fn lock_exclusive(&self) -> APSExclusiveGuard<'_> {
        // Standard spinlock stuff
        loop {
            // Load what the current situation is
            let current_value = self.tracker.load(Ordering::Relaxed);

            // We can only take an exclusive lock if the tracker zero (see self.tracker docs)
            if current_value == 0 {
                // Compare and swap to put the sentinel value in place
                let prev = self
                    .tracker
                    .compare_and_swap(0, SENTINEL_VALUE, Ordering::Acquire);

                // If we succeeded then we can return the guard, which will clean up after itself
                if prev == 0 {
                    return APSExclusiveGuard { parent: self };
                }
            }

            // Try to be kind to our scheduler, even as we employ an anti-pattern
            //
            // Without threading support, we'll just have to busy-wait.
            // Should we let the user supply a 'yield' function of their own?
            #[cfg(feature = "threads")]
            yield_now()
        }
    }

    pub fn lock_inclusive(&self) -> Option<APSInclusiveGuard<'_>> {
        // Greedily increment without checking if it's going to work
        let old_value = self.tracker.fetch_add(1, Ordering::Acquire);
        // If the old value is below the SENTINEL_VALUE, then we're free and clear
        // (We assume SENTINEL_VALUE is so big, we'll never reach it by incrementing by 1)
        if old_value < SENTINEL_VALUE {
            Some(APSInclusiveGuard { parent: self })
        } else {
            None
        }
    }
}

impl Default for AtomicProtectingSpinlock {
    fn default() -> Self {
        Self::new()
    }
}

pub struct APSExclusiveGuard<'a> {
    parent: &'a AtomicProtectingSpinlock,
}

impl<'a> Drop for APSExclusiveGuard<'a> {
    fn drop(&mut self) {
        // Reset by sending us back to zero
        self.parent.tracker.store(0, Ordering::Release);
    }
}

pub struct APSInclusiveGuard<'a> {
    parent: &'a AtomicProtectingSpinlock,
}

impl<'a> Drop for APSInclusiveGuard<'a> {
    fn drop(&mut self) {
        // Reset by subtracting 1
        self.parent.tracker.fetch_sub(1, Ordering::Release);
    }
}
