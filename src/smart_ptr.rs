use std::ops::Deref;

use crate::collector::{GcInternalHandle, COLLECTOR};
use crate::Scan;

#[derive(Debug)]
pub struct Gc<T: Scan> {
    backing_handle: GcInternalHandle,
    direct_ptr: *const T,
}

impl<T: Scan> Gc<T> {
    pub fn new(v: T) -> Self
    where
        T: 'static,
    {
        let (handle, ptr) = COLLECTOR.lock().track_data(v);
        Self {
            backing_handle: handle,
            direct_ptr: ptr,
        }
    }

    #[must_use]
    pub fn get(&self) -> GcGuard<T> {
        COLLECTOR.lock().inc_held_references();
        GcGuard { gc_ptr: self }
    }

    pub(crate) fn internal_handle(&self) -> GcInternalHandle {
        self.backing_handle
    }
}

impl<T: Scan> Clone for Gc<T> {
    fn clone(&self) -> Self {
        let new_handle = COLLECTOR.lock().clone_handle(self.backing_handle);

        Self {
            backing_handle: new_handle,
            direct_ptr: self.direct_ptr,
        }
    }
}

// TODO: Validate these impls
unsafe impl<T: Scan> Send for Gc<T> where T: Send {}
unsafe impl<T: Scan> Sync for Gc<T> where T: Sync {}

impl<T: Scan> Drop for Gc<T> {
    fn drop(&mut self) {
        COLLECTOR.lock().drop_handle(self.backing_handle);
    }
}

pub struct GcGuard<'a, T: Scan> {
    gc_ptr: &'a Gc<T>,
}

impl<'a, T: Scan> Drop for GcGuard<'a, T> {
    fn drop(&mut self) {
        COLLECTOR.lock().dec_held_references();
    }
}

impl<'a, T: Scan> Deref for GcGuard<'a, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        unsafe { &*self.gc_ptr.direct_ptr }
    }
}
