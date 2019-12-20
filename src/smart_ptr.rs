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
        self.backing_handle.clone()
    }
}

impl<T: Scan> Clone for Gc<T> {
    #[must_use]
    fn clone(&self) -> Self {
        let new_handle = COLLECTOR.lock().clone_handle(&self.backing_handle);

        Self {
            backing_handle: new_handle,
            direct_ptr: self.direct_ptr,
        }
    }
}

// Gc<T> only gives you access to &T, so it can be Sync if T is Sync
unsafe impl<T: Scan> Sync for Gc<T> where T: Sync {}
// Since we can clone Gc<T>, being able to send a Gc<T> implies possible sharing between threads
// (Thus for Gc<T> to be send, T must be Send and Sync)
unsafe impl<T: Scan> Send for Gc<T> where T: Sync + Send {}

impl<T: Scan> Drop for Gc<T> {
    fn drop(&mut self) {
        COLLECTOR.lock().drop_handle(&self.backing_handle);
    }
}

#[derive(Debug)]
pub struct GcGuard<'a, T: Scan> {
    gc_ptr: &'a Gc<T>,
}

// TODO: Consider Send/Sync implementations for GcGuard

impl<'a, T: Scan> Drop for GcGuard<'a, T> {
    fn drop(&mut self) {
        COLLECTOR.lock().dec_held_references();
    }
}

impl<'a, T: Scan> Deref for GcGuard<'a, T> {
    type Target = T;

    #[must_use]
    fn deref(&self) -> &Self::Target {
        unsafe { &*self.gc_ptr.direct_ptr }
    }
}

// TODO: Consider Display implementations for Gc / GcGuard
