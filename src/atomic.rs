use std::marker::PhantomData;
use std::mem;
use std::sync::atomic::{AtomicPtr, Ordering};
use std::sync::Arc;

use crate::collector::{GcData, InternalGcRef, COLLECTOR};
use crate::{Gc, Scan};

/// An atomic `Gc<T>`, useful for concurrent algorithms
///
/// This has more overhead than an `AtomicPtr`, but cleanly handles memory management. It also is
/// similar to `Gc<T>` in that it can be cloned, and therefore easily shared.
///
/// A good analogy would be to the excellent `arc-swap` crate. However, we can be more performant,
/// as relying on the collector lets us avoid some synchronization.
///
/// `AtomicGc` should be fairly fast, but you may not assume it does not block. In fact in the
/// presence of an active garbage collection operation, all operations will block. Otherwise
/// it shouldn't block.
#[derive(Clone, Debug)]
pub struct AtomicGc<T: Scan> {
    // It is only safe to read the data here if a collection is not happening
    atomic_ptr: Arc<AtomicPtr<GcData>>,
    backing_handle: InternalGcRef,
    _mark: PhantomData<Gc<T>>,
}

impl<T: Scan> AtomicGc<T> {
    /// Create a new `AtomicGc`
    ///
    /// The created `AtomicGc` will point to the same data as `data`
    #[must_use]
    pub fn new(data: &Gc<T>) -> Self {
        // Ensure we don't create an atomic out of dead data...
        data.assert_live();

        // `data` is guaranteed to be pointing to the data we're about to contain, so we don't need to
        // worry about data getting cleaned up (and therefore we don't need to block the collector)

        // Carefully craft a ptr to store atomically
        let data_arc = data.internal_handle_ref().data();
        let data_ptr = Arc::as_ptr(data_arc);

        let atomic_ptr = Arc::new(AtomicPtr::new(data_ptr as _));

        Self {
            atomic_ptr: atomic_ptr.clone(),
            backing_handle: COLLECTOR.new_handle_for_atomic(atomic_ptr),
            _mark: PhantomData,
        }
    }

    pub(crate) fn internal_handle(&self) -> InternalGcRef {
        self.backing_handle.clone()
    }

    /// `load` the data from this `AtomicGc<T>`, getting back a `Gc<T>`
    ///
    /// The ordering/atomicity guarantees are identical to `AtomicPtr::load`
    ///
    /// # Safety
    /// Must not be called from a Gc<T> destructor! Also you may not load from an atomic storing
    /// data that has been deallocated. (Only possible if you send a `AtomicGc` out of the
    /// background thread somehow.)
    #[must_use]
    pub unsafe fn load(&self, ordering: Ordering) -> Gc<T> {
        let ptr;
        let internal_handle;
        {
            let _collection_blocker = COLLECTOR.get_collection_blocker_spinlock();

            // Safe to manipulate this ptr only because we have the `_collection_blocker`
            let gc_data_ptr = self.atomic_ptr.load(ordering);
            let gc_data_temp = Arc::from_raw(gc_data_ptr);

            // Create a new `Arc` pointing to the same data, but don't invalidate the existing `Arc`
            // (which is effectively "behind" the pointer)
            let new_gc_data_ref = gc_data_temp.clone();
            mem::forget(gc_data_temp);

            ptr = new_gc_data_ref.scan_ptr().cast();
            internal_handle = COLLECTOR.handle_from_data(new_gc_data_ref);
        }

        Gc::new_raw(internal_handle, ptr)
    }

    /// `store` new data into this `AtomicGc`
    ///
    /// The ordering/atomicity guarantees are identical to `AtomicPtr::store`
    ///
    /// # Safety
    /// Must not be called from a Gc<T> destructor! Also you may not load from an atomic storing
    /// data that has been deallocated. (Only possible if you send a `AtomicGc` out of the
    /// background thread somehow.)
    pub unsafe fn store(&self, v: &Gc<T>, ordering: Ordering) {
        // Ensure we're not storing dead data...
        v.assert_live();

        let data = v.internal_handle_ref().data();
        let raw_data_ptr = Arc::as_ptr(data);

        {
            let _collection_blocker = COLLECTOR.get_collection_blocker_spinlock();

            // Safe to manipulate this ptr only because we have the `_collection_blocker`
            self.atomic_ptr.store(raw_data_ptr as _, ordering);
        }
    }

    /// `swap` what data is stored in this `AtomicGc`, getting a `Gc` to the old data back
    ///
    /// The ordering/atomicity guarantees are identical to `AtomicPtr::swap`
    ///
    /// # Safety
    /// Must not be called from a Gc<T> destructor! Also you may not load from an atomic storing
    /// data that has been deallocated. (Only possible if you send a `AtomicGc` out of the
    /// background thread somehow.)
    #[must_use]
    pub unsafe fn swap(&self, v: &Gc<T>, ordering: Ordering) -> Gc<T> {
        // Ensure we're not storing dead data...
        v.assert_live();

        let data = v.internal_handle_ref().data();
        let raw_data_ptr = Arc::as_ptr(data);

        let ptr;
        let internal_handle;
        {
            let _collection_blocker = COLLECTOR.get_collection_blocker_spinlock();
            let old_data_ptr = self.atomic_ptr.swap(raw_data_ptr as _, ordering);

            // Safe since we know the collector is blocked
            let old_data_arc = Arc::from_raw(old_data_ptr);
            let gc_data = old_data_arc.clone();
            mem::forget(old_data_arc);

            ptr = gc_data.scan_ptr().cast();
            internal_handle = COLLECTOR.handle_from_data(gc_data);
        }

        Gc::new_raw(internal_handle, ptr)
    }

    /// Do a CAS operation. If this `AtomicGc` points to the same data as `current` then after this
    /// operation it will point to the same data as `new`. (And this happens atomically.)
    ///
    /// Data is compared for pointer equality. NOT `Eq` equality. (A swap will only happen if
    /// `current` and this `AtomicGc` point to the same underlying allocation.)
    ///
    /// The ordering/atomicity guarantees are identical to `AtomicPtr::compare_and_swap`
    ///
    /// # Returns
    /// Returns `true` if the swap happened and this `AtomicGc` now points to `new`
    /// Returns `false` if the swap failed / this `AtomicGc` was not pointing to `current`
    ///
    /// # Safety
    /// Must not be called from a Gc<T> destructor! Also you may not load from an atomic storing
    /// data that has been deallocated. (Only possible if you send a `AtomicGc` out of the
    /// background thread somehow.)
    #[allow(clippy::must_use_candidate)]
    pub unsafe fn compare_and_swap(
        &self,
        current: &Gc<T>,
        new: &Gc<T>,
        ordering: Ordering,
    ) -> bool {
        // Ensure we're not storing dead data...
        new.assert_live();

        // Turn guess data into a raw ptr
        let guess_data = current.internal_handle_ref().data();
        let guess_data_raw = Arc::as_ptr(guess_data) as _;

        // Turn new data into a raw ptr
        let new_data = new.internal_handle_ref().data();
        let new_data_raw = Arc::as_ptr(new_data) as _;

        let compare_res;
        {
            let _collection_blocker = COLLECTOR.get_collection_blocker_spinlock();
            // Only safe since we have the `collection_blocker`
            compare_res = self
                .atomic_ptr
                .compare_and_swap(guess_data_raw, new_data_raw, ordering);
        }

        compare_res == guess_data_raw
    }

    /// Do a CAE operation. If this `AtomicGc` points to the same data as `current` then after this
    /// operation it will point to the same data as `new`. (And this happens atomically.)
    ///
    /// Data is compared for pointer equality. NOT `Eq` equality. (A swap will only happen if
    /// `current` and this `AtomicGc` point to the same underlying allocation.)
    ///
    /// The ordering/atomicity guarantees are identical to `AtomicPtr::compare_exchange`, refer to
    /// that documentation for documentation about `success` and `failure` orderings.
    ///
    /// # Returns
    /// Returns `true` if the swap happened and this `AtomicGc` now points to `new`
    /// Returns `false` if the swap failed / this `AtomicGc` was not pointing to `current`
    ///
    /// # Safety
    /// Must not be called from a Gc<T> destructor! Also you may not load from an atomic storing
    /// data that has been deallocated. (Only possible if you send a `AtomicGc` out of the
    /// background thread somehow.)
    #[allow(clippy::must_use_candidate)]
    pub unsafe fn compare_exchange(
        &self,
        current: &Gc<T>,
        new: &Gc<T>,
        success: Ordering,
        failure: Ordering,
    ) -> bool {
        // Ensure we're not storing dead data...
        new.assert_live();

        let guess_data = current.internal_handle_ref().data();
        let guess_data_raw = Arc::as_ptr(guess_data) as _;

        let new_data = new.internal_handle_ref().data();
        let new_data_raw = Arc::as_ptr(new_data) as _;

        let compare_res;
        {
            let _collection_blocker = COLLECTOR.get_collection_blocker_spinlock();
            compare_res =
                self.atomic_ptr
                    .compare_exchange(guess_data_raw, new_data_raw, success, failure);
        }

        compare_res.is_ok()
    }

    // TODO: Compare and swap/compare and exchange that return the current value
}

impl<T: Scan> Drop for AtomicGc<T> {
    fn drop(&mut self) {
        // Manually cleanup the backing handle...
        self.backing_handle.invalidate();
    }
}
