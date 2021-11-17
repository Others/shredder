#[cfg(feature = "nightly-features")]
use std::{marker::Unsize, ops::CoerceUnsized};

use std::any::{Any, TypeId};
use std::cmp::Ordering;
use std::fmt::{Debug, Display, Formatter};
use std::hash::{Hash, Hasher};
use std::ops::Deref;
use std::{fmt, ptr};

use crate::collector::{InternalGcRef, COLLECTOR};
use crate::marker::{GcDeref, GcDrop, GcSafe};
use crate::{Finalize, Scan, Scanner, ToScan};

/// A `Gc`, but with the ability to `Deref` to its contents!
///
/// This comes with the requirement that your data implement `GcDeref`, which can be limiting. See
/// `GcDeref` documentation for details.
pub struct DerefGc<T: Scan + GcDeref + ?Sized> {
    backing_handle: InternalGcRef,
    direct_ptr: *const T,
}

impl<T: Scan + GcDeref + ?Sized> DerefGc<T> {
    /// Create a new `DerefGc` containing the given data.
    /// `T: GcDrop` in order to create a `Gc<T>` with this method.
    /// If your `T` is not `GcDrop`, consider `new_with_finalizer`.
    ///
    /// When this data is garbage collected, its `drop` implementation will be run.
    ///
    /// It is possible for this data not to be collected before the program terminates, or for
    /// the program to terminate before the background thread runs its destructor. So be careful
    /// when relying on this guarantee.
    pub fn new(v: T) -> Self
    where
        T: Sized + GcDrop,
    {
        let (handle, ptr) = COLLECTOR.track_with_drop(v);
        Self {
            backing_handle: handle,
            direct_ptr: ptr,
        }
    }

    /// Create a new `DerefGc` containing the given data. (But specifying not to run its destructor.)
    /// This is useful because `T: GcDrop` is no longer necessary!
    ///
    /// When this data is garbage collected, its `drop` implementation will NOT be run.
    /// Be careful using this method! It can lead to memory leaks!
    pub fn new_no_drop(v: T) -> Self
    where
        T: Sized,
    {
        let (handle, ptr) = COLLECTOR.track_with_no_drop(v);
        Self {
            backing_handle: handle,
            direct_ptr: ptr,
        }
    }

    /// Create a new `DerefGc` containing the given data. (But specifying to call `finalize` on it
    /// instead of running its destructor.)
    /// This is useful because `T: GcDrop` is no longer necessary!
    ///
    /// As long as `finalize` does what you think it does, this is probably what you want for
    /// non-`'static`/non-`GcDrop` data!
    ///
    /// It is possible for this data not to be collected before the program terminates, or for
    /// the program to terminate before the background thread runs `finalize`. So be careful!
    pub fn new_with_finalizer(v: T) -> Self
    where
        T: Sized + Finalize,
    {
        let (handle, ptr) = COLLECTOR.track_with_finalization(v);
        Self {
            backing_handle: handle,
            direct_ptr: ptr,
        }
    }

    /// Create a new `DerefGc` using the given `Box<T>`.
    ///
    /// This function does not allocate anything - rather, it uses the `Box<T>` and releases its
    /// memory appropriately. This is useful since it removes the requirement for types to be
    /// sized.
    pub fn from_box(v: Box<T>) -> Self
    where
        T: ToScan + GcDrop,
    {
        let (handle, ptr) = COLLECTOR.track_boxed_value(v);
        Self {
            backing_handle: handle,
            direct_ptr: ptr,
        }
    }

    /// `ptr_eq` lets you compare two `DerefGc`s for pointer equality.
    ///
    /// This has the same semantics as `ptr::eq` or `Arc::ptr_eq`.
    #[must_use]
    pub fn ptr_eq(&self, o: &Self) -> bool {
        ptr::eq(self.direct_ptr, o.direct_ptr)
    }
}

impl<T: Scan + GcDeref + ?Sized> DerefGc<T> {
    /// Attempt to `downcast` this `DerefGc<T>` to a `DerefGc<S>`
    ///
    /// For implementation reasons this returns a new `DerefGc<T>` on success
    /// On failure (if there was not an `S` in the `DerefGc<T>`) then `None` is returned
    #[must_use]
    pub fn downcast<S>(&self) -> Option<DerefGc<S>>
    where
        T: Any + 'static,
        S: Scan + GcDeref + Any + 'static,
    {
        let ptr: &T = self.deref();

        if ptr.type_id() == TypeId::of::<S>() {
            let new_handle = COLLECTOR.clone_handle(&self.backing_handle);

            Some(DerefGc {
                backing_handle: new_handle,
                direct_ptr: self.direct_ptr.cast(),
            })
        } else {
            None
        }
    }
}

unsafe impl<T: Scan + GcDeref + ?Sized> GcSafe for DerefGc<T> {}
// unsafe impl<T: Scan + GcDeref + ?Sized> !GcDrop for DerefGc<T> {}
unsafe impl<T: Scan + GcDeref + Send + Sync + ?Sized> GcDeref for DerefGc<T> {}

// Same bounds as Arc<T>
unsafe impl<T: Scan + GcDeref + ?Sized> Sync for DerefGc<T> where T: Sync + Send {}
unsafe impl<T: Scan + GcDeref + ?Sized> Send for DerefGc<T> where T: Sync + Send {}
// Since we can clone DerefGc<T>, being able to send a DerefGc<T> implies possible sharing between threads
// (Thus for DerefGc<T> to be send, T must be Send and Sync)

// This is a fundamental implementation, since it's how GcInternalHandles make it into the Scanner
// Safety: The implementation is built around this, so it's by definition safe
unsafe impl<T: Scan + GcDeref + ?Sized> Scan for DerefGc<T> {
    #[allow(clippy::inline_always)]
    #[inline(always)]
    fn scan(&self, scanner: &mut Scanner<'_>) {
        scanner.add_internal_handle(self.backing_handle.clone());
    }
}

impl<T: Scan + GcDeref + ?Sized> Clone for DerefGc<T> {
    fn clone(&self) -> Self {
        let new_handle = COLLECTOR.clone_handle(&self.backing_handle);

        Self {
            backing_handle: new_handle,
            direct_ptr: self.direct_ptr,
        }
    }
}

impl<T: Scan + GcDeref + ?Sized> Deref for DerefGc<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        unsafe { &*self.direct_ptr }
    }
}

impl<T: Scan + GcDeref + ?Sized> Drop for DerefGc<T> {
    fn drop(&mut self) {
        self.backing_handle.invalidate()
    }
}

unsafe impl<T: Scan + GcDeref + ?Sized> Finalize for DerefGc<T> {
    unsafe fn finalize(&mut self) {
        self.backing_handle.invalidate();
    }
}

#[cfg(feature = "nightly-features")]
impl<T, U> CoerceUnsized<DerefGc<U>> for DerefGc<T>
where
    T: Scan + GcDeref + ?Sized + Unsize<U>,
    U: Scan + GcDeref + ?Sized,
{
}

impl<T: Scan + GcDeref + ?Sized> Debug for DerefGc<T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("DerefGc")
            .field("backing_handle", &"<SNIP>")
            .field("direct_ptr", &self.direct_ptr)
            .finish()
    }
}

impl<T: Scan + GcDeref + ?Sized> Default for DerefGc<T>
where
    T: Default + GcDrop,
{
    #[must_use]
    fn default() -> Self {
        let v = T::default();
        Self::new(v)
    }
}

impl<T: Scan + GcDeref + ?Sized> Display for DerefGc<T>
where
    T: Display,
{
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        self.deref().fmt(f)
    }
}

impl<T: Scan + GcDeref + ?Sized> fmt::Pointer for DerefGc<T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        fmt::Pointer::fmt(&self.direct_ptr, f)
    }
}

impl<T: Scan + GcDeref + ?Sized> Eq for DerefGc<T> where T: Eq {}

impl<T: Scan + GcDeref + ?Sized> Hash for DerefGc<T>
where
    T: Hash,
{
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.deref().hash(state)
    }
}

impl<T: Scan + GcDeref + ?Sized> Ord for DerefGc<T>
where
    T: Ord,
{
    #[must_use]
    fn cmp(&self, other: &Self) -> Ordering {
        (self.deref()).cmp(other.deref())
    }
}

#[allow(clippy::partialeq_ne_impl)]
impl<T: Scan + GcDeref + ?Sized> PartialEq for DerefGc<T>
where
    T: PartialEq,
{
    #[must_use]
    fn eq(&self, other: &Self) -> bool {
        (self.deref()).eq(other.deref())
    }

    #[must_use]
    fn ne(&self, other: &Self) -> bool {
        (self.deref()).ne(other.deref())
    }
}

impl<T: Scan + GcDeref + ?Sized> PartialOrd for DerefGc<T>
where
    T: PartialOrd,
{
    #[must_use]
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        (self.deref()).partial_cmp(other.deref())
    }

    #[must_use]
    fn lt(&self, other: &Self) -> bool {
        (self.deref()).lt(other.deref())
    }

    #[must_use]
    fn le(&self, other: &Self) -> bool {
        (self.deref()).le(other.deref())
    }

    #[must_use]
    fn gt(&self, other: &Self) -> bool {
        (self.deref()).gt(other.deref())
    }

    #[must_use]
    fn ge(&self, other: &Self) -> bool {
        (self.deref()).ge(other.deref())
    }
}

#[cfg(test)]
mod test {
    use crate::DerefGc;

    #[test]
    #[allow(clippy::eq_op)]
    fn test_eq() {
        let a = DerefGc::new(1);
        let b = DerefGc::new(1);
        assert_eq!(a, a);
        assert_eq!(b, b);
        assert_eq!(a, b);
        assert_eq!(b, a);

        assert!(a.ptr_eq(&a));
        assert!(b.ptr_eq(&b));
        assert!(!a.ptr_eq(&b));
        assert!(!b.ptr_eq(&a));
        assert!(a.ptr_eq(&a.clone()));
    }
}
