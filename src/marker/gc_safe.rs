use std::hash::{Hash, Hasher};
use std::ops::{Deref, DerefMut};

/// A marker trait that marks that data can be scanned in the background by the garbage collector.
///
/// Specifically, the requirement for implementing this trait is:
///  1) It's okay for any thread to call `scan` on this data, as long as it has exclusive access to
///     the data.
///  2) It is safe for `scan` to be called on this data, even if the lifetime of this data has
///     ended.
///
/// Importantly if a type is `Send + 'static`, then it is always `GcSafe`
///
/// NOTE: `GcSafe` cannot simply be `Send + 'static`, since `Gc` is always `GcSafe` but sometimes is
/// not `Send` or `'static`
pub unsafe trait GcSafe {}

/// `GcSafeWrapper` wraps a `Send + 'static` datatype to make it `GcSafe`
///
/// Usually you'd just want to implement `GcSafe` directly, but this is useful situationally if
/// you need to use someone else's type which does not implement `GcSafe`.
///
/// TODO: Remove this once overlapping marker traits are stabilized
pub struct GcSafeWrapper<T> {
    /// The wrapped value
    pub v: T,
}

unsafe impl<T> GcSafe for GcSafeWrapper<T> {}

impl<T> GcSafeWrapper<T> {
    /// Create a new `GcSafeWrapper` storing `v`, assuming `v: Send + 'static`
    ///
    /// Safe since all `Send + 'static` data is `GcSafe` automatically
    pub fn new(v: T) -> Self
    where
        T: Send + 'static,
    {
        Self { v }
    }

    /// Create a new `GcSafeWrapper` storing `v`, ignoring whether it's `Send + 'static`
    ///
    /// # Safety
    /// Only safe to use if you can guarantee the value passed in fulfills the requirements to
    /// implement `GcSafe`.
    pub unsafe fn new_unsafe(v: T) -> Self {
        Self { v }
    }

    /// `take` the value out of this wrapper
    pub fn take(self) -> T {
        self.v
    }
}

impl<T: Send + Clone> Clone for GcSafeWrapper<T> {
    fn clone(&self) -> Self {
        Self { v: self.v.clone() }
    }
}

impl<T: Send + Copy> Copy for GcSafeWrapper<T> {}

impl<T: Send + Default> Default for GcSafeWrapper<T> {
    fn default() -> Self {
        Self { v: T::default() }
    }
}

impl<T: Send> Deref for GcSafeWrapper<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.v
    }
}

impl<T: Send> DerefMut for GcSafeWrapper<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.v
    }
}

impl<T: Send + Hash> Hash for GcSafeWrapper<T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.v.hash(state);
    }
}

#[allow(clippy::partialeq_ne_impl)]
impl<T: Send + PartialEq> PartialEq for GcSafeWrapper<T> {
    fn eq(&self, other: &Self) -> bool {
        self.v.eq(other)
    }

    fn ne(&self, other: &Self) -> bool {
        self.v.ne(other)
    }
}
impl<T: Send + Eq> Eq for GcSafeWrapper<T> {}
