use std::marker::PhantomData;
use std::mem;
use std::ptr::drop_in_place;
use std::sync::atomic::{AtomicPtr, Ordering};
use std::sync::Arc;

use crate::collector::{GcData, InternalGcRef, RefCountPolicy, COLLECTOR};
use crate::marker::{GcDeref, GcSafe};
use crate::{Finalize, Gc, Scan, Scanner};

/// An atomic `Gc<T>`, useful for concurrent algorithms
///
/// This has more overhead than an `AtomicPtr`, but cleanly handles memory management. (Similar
/// to the excellent `arc-swap` crate or crossbeam's `Atomic`.)
///
/// `AtomicGc` should be fairly fast, but you may not assume it does not block. In fact in the
/// presence of an active garbage collection operation, all operations will block. Otherwise
/// it shouldn't block.
#[derive(Debug)]
pub struct AtomicGc<T: Scan> {
    // This is a pointer to the data that this "AtomicGc" is pointing to. This is taken from an `Arc`
    // and is only valid as long as that `Arc` is valid. However, we know that the collector must
    // hold arcs to the data, so as long as the data is live, this pointer is valid.
    //
    // Only in a `drop` or `finalize` call (in the background thread) will the data no longer be
    // live. But the contracts of `GcDrop` and `Finalize` require that no methods on `AtomicGc` are
    // called.
    //
    // Taken together, this means that this pointer is always valid when executing a method on
    // `AtomicGc`.
    atomic_ptr: AtomicPtr<GcData>,
    _mark: PhantomData<Gc<T>>,
}

impl<T: Scan> AtomicGc<T> {
    /// Create a new `AtomicGc`
    ///
    /// The created `AtomicGc` will point to the same data as `data`
    #[must_use]
    pub fn new(data: Gc<T>) -> Self {
        // Ensure we don't create an atomic out of dead data...
        data.assert_live();

        // `data` is guaranteed to be pointing to the data we're about to contain, so we don't need to
        // worry about data getting cleaned up (and therefore we don't need to block the collector)

        // Carefully craft a ptr to store atomically
        let data_arc = data.internal_handle_ref().data();
        let atomic_ptr = AtomicPtr::new(Arc::as_ptr(data_arc) as _);
        // Forget the initial data, we will absorb its reference counts
        data.drop_preserving_reference_counts();

        Self {
            atomic_ptr,
            _mark: PhantomData,
        }
    }

    // NOTE: Throughout the methods here, the `collection_blocker_spinlock` is used to protect
    // against concurrently changing the graph while the collector is running.
    //
    // TODO: Validate if we could make the collector work without this

    #[inline]
    unsafe fn arc_ptr_to_new_arc(v: *const GcData) -> Arc<GcData> {
        let temp = Arc::from_raw(v);
        let new = temp.clone();
        mem::forget(temp);
        new
    }

    /// `load` the data from this `AtomicGc<T>`, getting back a `Gc<T>`
    ///
    /// The ordering/atomicity guarantees are identical to `AtomicPtr::load`
    #[must_use]
    pub fn load(&self, ordering: Ordering) -> Gc<T> {
        // No need for collection blocker, as we're not modifying the graph
        // This is safe. See comment on the `atomic_ptr` field
        let gc_data_ptr = self.atomic_ptr.load(ordering);

        // Create a new `Arc` pointing to the same data, but don't invalidate the existing `Arc`
        // (which is actually stored in the collector metadata struct)
        let data = unsafe { Self::arc_ptr_to_new_arc(gc_data_ptr) };

        let ptr = data.scan_ptr().cast();
        let internal_handle = InternalGcRef::new(data, RefCountPolicy::FromExistingHandle);

        Gc::new_raw(internal_handle, ptr)
    }

    /// `store` new data into this `AtomicGc`
    ///
    /// The ordering/atomicity guarantees are identical to `AtomicPtr::store`
    pub fn store(&self, new: Gc<T>, ordering: Ordering) {
        // Ensure we're not storing dead data...
        new.assert_live();
        let raw_data_ptr = Arc::as_ptr(new.internal_handle_ref().data());

        {
            //  Need the collection blocker as we are mutating the graph
            let _collection_blocker = COLLECTOR.get_collection_blocker_spinlock();

            // We absorb the reference counts of the data we're storing
            // TODO: Is this actually more efficient that taking by reference and incrementing? Do we want to support both?
            new.drop_preserving_reference_counts();

            // Safe to change this ptr only because we have the `_collection_blocker`
            let old_data = self.atomic_ptr.swap(raw_data_ptr as _, ordering);
            let old_arc = unsafe { Arc::from_raw(old_data) };

            // The count of the data going out decreases
            COLLECTOR.decrement_reference_count(&old_arc);
            mem::forget(old_arc);
        }
    }

    /// `swap` new data with the data in this `AtomicGc`
    ///
    /// The ordering/atomicity guarantees are identical to `AtomicPtr::swap`
    pub fn swap(&self, new: Gc<T>, ordering: Ordering) -> Gc<T> {
        // Ensure we're not storing dead data...
        new.assert_live();

        let raw_data_ptr = Arc::as_ptr(new.internal_handle_ref().data());

        {
            //  Need the collection blocker as we are mutating the graph
            let _collection_blocker = COLLECTOR.get_collection_blocker_spinlock();

            let old_data_ptr = self.atomic_ptr.swap(raw_data_ptr as _, ordering);
            // We absorb the reference counts of the data we're storing
            new.drop_preserving_reference_counts();

            // Then we return out the old data
            let old_data = unsafe { Self::arc_ptr_to_new_arc(old_data_ptr) };
            let old_ptr = old_data.underlying_allocation.scan_ptr.cast();
            let internal_handle =
                InternalGcRef::new(old_data, RefCountPolicy::InheritExistingCounts);
            Gc::new_raw(internal_handle, old_ptr)
        }
    }

    /// Execute a `compare_exchange` operation
    ///
    /// The ordering/atomicity guarantees are identical to `AtomicPtr::compare_exchange`
    ///
    /// # Errors
    /// On success returns `Ok(previous_value)` (which is guaranteed to be the same as `current`)
    /// On failure returns an error containing the current value, and the `new` value passed in
    pub fn compare_exchange(
        &self,
        current: &Gc<T>,
        new: Gc<T>,
        success: Ordering,
        failure: Ordering,
    ) -> Result<Gc<T>, CompareExchangeError<T>> {
        // Ensure we're not storing dead data...
        new.assert_live();

        let guess_ptr = Arc::as_ptr(current.internal_handle_ref().data());
        let new_ptr = Arc::as_ptr(new.internal_handle_ref().data());

        {
            //  Need the collection blocker as we are mutating the graph
            let _collection_blocker = COLLECTOR.get_collection_blocker_spinlock();

            let exchange_res =
                self.atomic_ptr
                    .compare_exchange(guess_ptr as _, new_ptr as _, success, failure);

            match exchange_res {
                Ok(old) => {
                    // Get the old value
                    let old_data = unsafe { Self::arc_ptr_to_new_arc(old) };
                    let old_ptr = old_data.underlying_allocation.scan_ptr.cast();

                    // We absorb the reference counts of the data we're storing
                    new.drop_preserving_reference_counts();
                    // Our current reference counts aer being inhereted by the new data
                    let internal_handle =
                        InternalGcRef::new(old_data, RefCountPolicy::InheritExistingCounts);

                    Ok(Gc::new_raw(internal_handle, old_ptr))
                }
                Err(current) => {
                    let current = unsafe { Self::arc_ptr_to_new_arc(current) };
                    let current_ptr = current.underlying_allocation.scan_ptr.cast();

                    let internal_handle =
                        InternalGcRef::new(current, RefCountPolicy::FromExistingHandle);

                    let current = Gc::new_raw(internal_handle, current_ptr);

                    Err(CompareExchangeError { current, new })
                }
            }
        }
    }
}

/// If a `compare_exchange` operation fails, this error is returned
///
/// It contains the actual value that was in the `AtomicGc`, as well as the `new` value that was
/// passed in to the `compare_exchange` operation
pub struct CompareExchangeError<T: Scan> {
    /// The value that was in the `AtomicGc` at the time of the `compare_exchange` operation
    pub current: Gc<T>,
    /// The value that was in the `new` parameter when you called `compare_exchange`
    pub new: Gc<T>,
}

unsafe impl<T: Scan> Scan for AtomicGc<T> {
    fn scan(&self, scanner: &mut Scanner<'_>) {
        // This is safe for the same reasons as `AtomicPtr::load`
        let gc_data_ptr = self.atomic_ptr.load(Ordering::SeqCst);
        let gc_data = unsafe { Self::arc_ptr_to_new_arc(gc_data_ptr) };

        let internal_handle = InternalGcRef::new(gc_data, RefCountPolicy::TransientHandle);

        scanner.add_internal_handle(&internal_handle);
    }
}

unsafe impl<T: Scan> GcSafe for AtomicGc<T> {}
// unsafe impl<T: Scan> !GcDrop for AtomicGc<T> {}
// This is valid, as `AtomicGc` does its own sychronization with the collector
unsafe impl<T: Scan + Send + Sync> GcDeref for AtomicGc<T> {}

unsafe impl<T: Scan> Finalize for AtomicGc<T> {
    unsafe fn finalize(&mut self) {
        drop_in_place(self)
    }
}

impl<T: Scan> Drop for AtomicGc<T> {
    fn drop(&mut self) {
        // This is safe, since `Finalize` and `GcDrop` rules prevent reviving an `AtomicGc`
        // (and the background dropper always preserves the `Arc<GcData>` until all drop/finalize are run)
        let x = *self.atomic_ptr.get_mut();
        COLLECTOR.decrement_reference_count(unsafe { &*x });
    }
}
