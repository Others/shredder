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
/// `AtomicGc` should be fairly fast, but you may not assume it does not block. In fact in the
/// presence of an active garbage collection operation, all operations will block.
#[derive(Clone, Debug)]
pub struct AtomicGc<T: Scan> {
    atomic_ptr: Arc<AtomicPtr<GcData>>,
    backing_handle: InternalGcRef,
    _mark: PhantomData<Gc<T>>,
}

impl<T: Scan> AtomicGc<T> {
    pub fn new(v: &Gc<T>) -> Self {
        let _collection_blocker = COLLECTOR.get_atomic_guard_spinlock_inclusive();

        let data = v.internal_handle_ref().data();
        let data_ptr = Arc::into_raw(data);
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

    #[must_use]
    pub fn load(&self, ordering: Ordering) -> Gc<T> {
        let ptr;
        let internal_handle;
        {
            let _collection_blocker = COLLECTOR.get_atomic_guard_spinlock_inclusive();
            let gc_data_ref = self.atomic_ptr.load(ordering);

            // Safe since we know the collector is blocked
            let gc_data_raw = unsafe { Arc::from_raw(gc_data_ref) };
            let new_gc_data_ref = gc_data_raw.clone();
            mem::forget(gc_data_raw);

            ptr = new_gc_data_ref.scan_ptr().cast();
            internal_handle = COLLECTOR.handle_from_data(new_gc_data_ref);
        }

        Gc::new_raw(internal_handle, ptr)
    }

    pub fn store(&self, v: &Gc<T>, ordering: Ordering) {
        let data = v.internal_handle_ref().data();
        let raw_data_ptr = Arc::into_raw(data);

        {
            let _collection_blocker = COLLECTOR.get_atomic_guard_spinlock_inclusive();
            self.atomic_ptr.store(raw_data_ptr as _, ordering);
        }
    }

    #[must_use]
    pub fn swap(&self, v: &Gc<T>, ordering: Ordering) -> Gc<T> {
        let data = v.internal_handle_ref().data();
        let raw_data_ptr = Arc::into_raw(data);

        let ptr;
        let internal_handle;
        {
            let _collection_blocker = COLLECTOR.get_atomic_guard_spinlock_inclusive();
            let old_data_ptr = self.atomic_ptr.swap(raw_data_ptr as _, ordering);

            // Safe since we know the collector is blocked
            let gc_data = unsafe { Arc::from_raw(old_data_ptr) };

            ptr = gc_data.scan_ptr().cast();
            internal_handle = COLLECTOR.handle_from_data(gc_data);
        }

        Gc::new_raw(internal_handle, ptr)
    }

    #[allow(clippy::must_use_candidate)]
    pub fn compare_and_swap(&self, current: &Gc<T>, new: &Gc<T>, ordering: Ordering) -> bool {
        let guess_data = current.internal_handle_ref().data();
        let guess_data_raw = Arc::as_ptr(&guess_data) as _;

        let new_data = new.internal_handle_ref().data();
        let new_data_raw = Arc::into_raw(new_data) as _;

        let compare_res;
        {
            let _collection_blocker = COLLECTOR.get_atomic_guard_spinlock_inclusive();
            compare_res = self
                .atomic_ptr
                .compare_and_swap(guess_data_raw, new_data_raw, ordering);
        }

        if compare_res == guess_data_raw {
            // The swap succeeded, and we need to take back ownership of the old data arc
            unsafe {
                Arc::from_raw(compare_res as *const GcData);
            }
            true
        } else {
            // The swap failed, and we need to take back ownership of the new data arc
            unsafe {
                Arc::from_raw(new_data_raw as *const GcData);
            }
            false
        }
    }

    #[allow(clippy::must_use_candidate)]
    pub fn compare_exchange(
        &self,
        current: &Gc<T>,
        new: &Gc<T>,
        success: Ordering,
        failure: Ordering,
    ) -> bool {
        let guess_data = current.internal_handle_ref().data();
        let guess_data_raw = Arc::as_ptr(&guess_data) as _;

        let new_data = new.internal_handle_ref().data();
        let new_data_raw = Arc::into_raw(new_data) as _;

        let compare_res;
        {
            let _collection_blocker = COLLECTOR.get_atomic_guard_spinlock_inclusive();
            compare_res =
                self.atomic_ptr
                    .compare_exchange(guess_data_raw, new_data_raw, success, failure);
        }

        if let Ok(old_ptr) = compare_res {
            // The swap succeeded, and we need to take back ownership of the old data arc
            unsafe {
                Arc::from_raw(old_ptr as *const GcData);
            }
            true
        } else {
            // The swap failed, and we need to take back ownership of the new data arc
            unsafe {
                Arc::from_raw(new_data_raw as *const GcData);
            }
            false
        }
    }
}

impl<T: Scan> Drop for AtomicGc<T> {
    fn drop(&mut self) {
        self.backing_handle.invalidate();
    }
}
