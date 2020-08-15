use std::borrow::Borrow;
use std::cell::{BorrowError, BorrowMutError, RefCell};
use std::cmp::Ordering;
use std::fmt::{self, Debug, Display, Formatter};
use std::hash::{Hash, Hasher};
use std::ops::Deref;
use std::sync;

use stable_deref_trait::StableDeref;

use crate::collector::{GcGuardWarrant, InternalGcRef, COLLECTOR};
use crate::wrappers::{
    GcMutexGuard, GcPoisonError, GcRef, GcRefMut, GcRwLockReadGuard, GcRwLockWriteGuard,
    GcTryLockError,
};
use crate::{Finalize, Scan};

/// A smart-pointer for data tracked by `shredder` garbage collector
pub struct Gc<T: Scan> {
    backing_handle: InternalGcRef,
    direct_ptr: *const T,
}

impl<T: Scan> Gc<T> {
    /// Create a new `Gc` containing the given data.
    /// `T: 'static` in order to create a `Gc<T>` with this method.
    /// If your `T` is not static, consider `new_with_finalizer`.
    ///
    /// When this data is garbage collected, its `drop` implementation will be run.
    ///
    /// It is possible for this data not to be collected before the program terminates, or for
    /// the program to terminate before the background thread runs its destructor. So be careful
    /// when relying on this guarantee.
    pub fn new(v: T) -> Self
    where
        T: 'static,
    {
        let (handle, ptr) = COLLECTOR.track_with_drop(v);
        Self {
            backing_handle: handle,
            direct_ptr: ptr,
        }
    }

    /// Create a new `Gc` containing the given data. (But specifying not to run its destructor.)
    /// This is useful because `T: 'static` is no longer necessary!
    ///
    /// When this data is garbage collected, its `drop` implementation will NOT be run.
    /// Be careful using this method! It can lead to memory leaks!
    pub fn new_no_drop(v: T) -> Self {
        let (handle, ptr) = COLLECTOR.track_with_no_drop(v);
        Self {
            backing_handle: handle,
            direct_ptr: ptr,
        }
    }

    /// Create a new `Gc` containing the given data. (But specifying to call `finalize` on it
    /// instead of running its destructor.)
    /// This is useful because `T: 'static` is no longer necessary!
    ///
    /// As long as `finalize` does what you think it does, this is probably what you want for
    /// non-'static data!
    ///
    /// It is possible for this data not to be collected before the program terminates, or for
    /// the program to terminate before the background thread runs `finalize`. So be careful!
    pub fn new_with_finalizer(v: T) -> Self
    where
        T: Finalize,
    {
        let (handle, ptr) = COLLECTOR.track_with_finalization(v);
        Self {
            backing_handle: handle,
            direct_ptr: ptr,
        }
    }

    pub(crate) fn new_raw(backing_handle: InternalGcRef, direct_ptr: *const T) -> Self {
        Self {
            backing_handle,
            direct_ptr,
        }
    }

    /// `get` lets you get a `GcGuard`, which will deref to the underlying data.
    ///
    /// `get` is used to get a `GcGuard`. This is usually what you want when accessing non-`Sync`
    /// data in a `Gc`. The API is very analogous to the `Mutex` API. It may block if the data is
    /// being scanned
    #[must_use]
    pub fn get(&self) -> GcGuard<'_, T> {
        let warrant = COLLECTOR.get_data_warrant(&self.backing_handle);
        GcGuard {
            gc_ptr: self,
            _warrant: warrant,
        }
    }

    pub(crate) fn internal_handle(&self) -> InternalGcRef {
        self.backing_handle.clone()
    }

    pub(crate) fn internal_handle_ref(&self) -> &InternalGcRef {
        &self.backing_handle
    }
}

impl<T: Scan> Clone for Gc<T> {
    #[must_use]
    fn clone(&self) -> Self {
        let new_handle = COLLECTOR.clone_handle(&self.backing_handle);

        Self {
            backing_handle: new_handle,
            direct_ptr: self.direct_ptr,
        }
    }
}

// Same bounds as Arc<T>
unsafe impl<T: Scan> Sync for Gc<T> where T: Sync + Send {}
unsafe impl<T: Scan> Send for Gc<T> where T: Sync + Send {}
// Since we can clone Gc<T>, being able to send a Gc<T> implies possible sharing between threads
// (Thus for Gc<T> to be send, T must be Send and Sync)

impl<T: Scan> Drop for Gc<T> {
    fn drop(&mut self) {
        self.backing_handle.invalidate();
    }
}

// TODO: Implement GRwLock along the same lines

// Lots of traits it's good for a smart ptr to implement:
impl<T: Scan> Debug for Gc<T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("Gc")
            .field("backing_handle", &"<SNIP>")
            .field("direct_ptr", &self.direct_ptr)
            .finish()
    }
}

impl<T: Scan> Default for Gc<T>
where
    T: Default + 'static,
{
    #[must_use]
    fn default() -> Self {
        let v = T::default();
        Self::new(v)
    }
}

impl<T: Scan> Display for Gc<T>
where
    T: Display,
{
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let a = self.get();
        a.fmt(f)
    }
}

impl<T: Scan> fmt::Pointer for Gc<T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        fmt::Pointer::fmt(&self.direct_ptr, f)
    }
}

impl<T: Scan> Eq for Gc<T> where T: Eq {}

impl<T: Scan> Hash for Gc<T>
where
    T: Hash,
{
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.get().hash(state)
    }
}

impl<T: Scan> Ord for Gc<T>
where
    T: Ord,
{
    #[must_use]
    fn cmp(&self, other: &Self) -> Ordering {
        let a = self.get();
        let b = other.get();

        a.cmp(b.deref())
    }
}

#[allow(clippy::partialeq_ne_impl)]
impl<T: Scan> PartialEq for Gc<T>
where
    T: PartialEq,
{
    #[must_use]
    fn eq(&self, other: &Self) -> bool {
        let a = self.get();
        let b = other.get();
        a.eq(&b)
    }

    #[must_use]
    fn ne(&self, other: &Self) -> bool {
        let a = self.get();
        let b = other.get();
        a.ne(&b)
    }
}

impl<T: Scan> PartialOrd for Gc<T>
where
    T: PartialOrd,
{
    #[must_use]
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        let a = self.get();
        let b = other.get();

        a.partial_cmp(&b)
    }

    #[must_use]
    fn lt(&self, other: &Self) -> bool {
        let a = self.get();
        let b = other.get();

        a.lt(&b)
    }

    #[must_use]
    fn le(&self, other: &Self) -> bool {
        let a = self.get();
        let b = other.get();

        a.le(&b)
    }

    #[must_use]
    fn gt(&self, other: &Self) -> bool {
        let a = self.get();
        let b = other.get();

        a.gt(&b)
    }

    #[must_use]
    fn ge(&self, other: &Self) -> bool {
        let a = self.get();
        let b = other.get();

        a.ge(&b)
    }
}

/// A guard object that lets you access the underlying data of a `Gc`.
/// It exists as data needs protection from being scanned while it's being concurrently modified.
pub struct GcGuard<'a, T: Scan> {
    gc_ptr: &'a Gc<T>,
    _warrant: GcGuardWarrant,
}

impl<'a, T: Scan> Deref for GcGuard<'a, T> {
    type Target = T;

    #[must_use]
    fn deref(&self) -> &Self::Target {
        unsafe { &*self.gc_ptr.direct_ptr }
    }
}

/// It is impossible for the value behind a `GcGuard` to move (since it's basically a `&T`)
unsafe impl<'a, T: Scan> StableDeref for GcGuard<'a, T> {}

impl<'a, T: Scan> AsRef<T> for GcGuard<'a, T> {
    #[must_use]
    fn as_ref(&self) -> &T {
        self.deref()
    }
}

impl<'a, T: Scan> Borrow<T> for GcGuard<'a, T> {
    #[must_use]
    fn borrow(&self) -> &T {
        self.deref()
    }
}

impl<'a, T: Scan + Debug> Debug for GcGuard<'a, T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("GcGuard")
            .field("v", self.deref())
            .field("warrant", &"<SNIP>")
            .finish()
    }
}

// Special casing goes here, mostly so rustdoc documents it in the right order
impl<T: Scan + 'static> Gc<RefCell<T>> {
    /// Call the underlying `borrow` method on the `RefCell`.
    ///
    /// This is just a nice method so you don't have to call `get` manually.
    #[must_use]
    pub fn borrow(&self) -> GcRef<'_, T> {
        let g = self.get();
        GcRef::borrow(g)
    }

    /// Call the underlying `try_borrow` method on the `RefCell`.
    ///
    /// This is just a nice method so you don't have to call `get` manually.
    ///
    /// # Errors
    /// Propagates a `BorrowError` if the underlying `RefCell` is already borrowed mutably
    pub fn try_borrow(&self) -> Result<GcRef<'_, T>, BorrowError> {
        let g = self.get();
        GcRef::try_borrow(g)
    }

    /// Call the underlying `borrow_mut` method on the `RefCell`.
    ///
    /// This is just a nice method so you don't have to call `get` manually.
    #[must_use]
    pub fn borrow_mut(&self) -> GcRefMut<'_, T> {
        let g = self.get();
        GcRefMut::borrow_mut(g)
    }

    /// Call the underlying `try_borrow_mut` method on the `RefCell`.
    ///
    /// This is just a nice method so you don't have to call `get` manually.
    /// # Errors
    /// Propagates a `BorrowError` if the underlying `RefCell` is already borrowed
    pub fn try_borrow_mut(&self) -> Result<GcRefMut<'_, T>, BorrowMutError> {
        let g = self.get();
        GcRefMut::try_borrow_mut(g)
    }
}

impl<T: Scan + 'static> Gc<sync::Mutex<T>> {
    /// Call the underlying `lock` method on the inner `Mutex`
    ///
    /// This is just a nice method so you don't have to `get` manually
    ///
    /// # Errors
    /// Returns a `GcPoisonError` if the underlying `.lock` method returns a poison error.
    /// You may use `into_inner` in order to recover the guard from that error.
    pub fn lock(&self) -> Result<GcMutexGuard<'_, T>, GcPoisonError<GcMutexGuard<'_, T>>> {
        let g = self.get();
        GcMutexGuard::lock(g)
    }

    /// Call the underlying `try_lock` method on the inner `Mutex`
    ///
    /// This is just a nice method so you don't have to `get` manually
    ///
    /// # Errors
    /// Returns a `GcTryLockError` if the underlying `.try_lock` method returns an error
    pub fn try_lock(&self) -> Result<GcMutexGuard<'_, T>, GcTryLockError<GcMutexGuard<'_, T>>> {
        let g = self.get();
        GcMutexGuard::try_lock(g)
    }
}

impl<T: Scan + 'static> Gc<sync::RwLock<T>> {
    /// Call the underlying `read` method on the inner `RwLock`
    ///
    /// This is just a nice method so you don't have to `get` manually
    ///
    /// # Errors
    /// Returns a `GcPoisonError` if the underlying `read` method returns a poison error.
    /// You may use `into_inner` in order to recover the guard from that error.
    pub fn read(
        &self,
    ) -> Result<GcRwLockReadGuard<'_, T>, GcPoisonError<GcRwLockReadGuard<'_, T>>> {
        let g = self.get();
        GcRwLockReadGuard::read(g)
    }

    /// Call the underlying `write` method on the inner `RwLock`
    ///
    /// This is just a nice method so you don't have to `get` manually
    ///
    /// # Errors
    /// Returns a `GcPoisonError` if the underlying `write` method returns a poison error.
    /// You may use `into_inner` in order to recover the guard from that error.
    pub fn write(
        &self,
    ) -> Result<GcRwLockWriteGuard<'_, T>, GcPoisonError<GcRwLockWriteGuard<'_, T>>> {
        let g = self.get();
        GcRwLockWriteGuard::write(g)
    }

    /// Call the underlying `try_read` method on the inner `RwLock`
    ///
    /// This is just a nice method so you don't have to `get` manually
    ///
    /// # Errors
    /// Returns a `GcTryLockError` if the underlying `try_read` method returns an error
    pub fn try_read(
        &self,
    ) -> Result<GcRwLockReadGuard<'_, T>, GcTryLockError<GcRwLockReadGuard<'_, T>>> {
        let g = self.get();
        GcRwLockReadGuard::try_read(g)
    }

    /// Call the underlying `try_write` method on the inner `RwLock`
    ///
    /// This is just a nice method so you don't have to `get` manually
    ///
    /// # Errors
    /// Returns a `GcTryLockError` if the underlying `try_write` method returns an error
    pub fn try_write(
        &self,
    ) -> Result<GcRwLockWriteGuard<'_, T>, GcTryLockError<GcRwLockWriteGuard<'_, T>>> {
        let g = self.get();
        GcRwLockWriteGuard::try_write(g)
    }
}
