mod r;
mod std_impls;

use std::hash::{Hash, Hasher};
use std::ops::{Deref, DerefMut};

use crate::collector::InternalGcRef;
use crate::Gc;

pub use r::{RMut, R};

/// A trait capturing the ability of data to be scanned for references to data in a `Gc`.
///
/// This is unsafe, since a bad `scan` implementation can cause memory unsafety in two ways:
/// 1) If `scan` scans data that this object does not own
/// 2) If `scan` does anything other than `scan` data with a non-`'static` lifetime
///
/// The importance of (1) is so that the collector does not collect data that is in use. The
/// importance of (2) is so that this data can still be scanned even after its lifetime has
/// technically expired.
///
/// Regarding (1): Note that it's okay to miss data that you own. Missing connected data can only
/// cause memory leaks--not memory unsafety.
/// Regarding (2): In particular, `scan` should not call anything but `Scan` on `R` and `RMut`. Even
/// implicitly using the `deref` implementations on these structs is incorrect.
///
/// Importantly, any empty `scan` implementation is safe (assuming the `GcSafe` impl is correct)
///
/// NB: It's important that `scan` only scans data that is truly owned. `Rc`/`Arc` cannot have
/// sensible `scan` implementations, since each individual smart pointer doesn't own the underlying
/// data.
///
/// # Examples
/// In practice you probably want to use the derive macro:
/// ```
/// use shredder::Scan;
///
/// #[derive(Scan)]
/// struct Example {
///     v: u32
/// }
/// ```
///
/// This also comes with a `#[shredder(skip)]` attribute, for when some data implements `GcSafe` but
/// not `Scan`
/// ```
/// use std::sync::Arc;
/// use shredder::Scan;
///
/// #[derive(Scan)]
/// struct Example {
///     #[shredder(skip)]
///     v: Arc<u32>
/// }
/// ```
///
/// This can work for any `Send` data using `GcSafeWrapper`
/// ```
/// use std::sync::Arc;
/// use shredder::{Scan, GcSafeWrapper};
///
/// struct SendDataButNotScan {
///     i: u32
/// }
///
/// #[derive(Scan)]
/// struct Example {
///     #[shredder(skip)]
///     v: GcSafeWrapper<SendDataButNotScan>
/// }
/// ```
///
/// In emergencies, you can break out `#[shredder(unsafe_skip)]`, but this is potentially unsafe
/// ```
/// use std::sync::Arc;
/// use shredder::{Scan, GcSafeWrapper};
///
/// struct NotEvenSendData {
///     data: *mut u32
/// }
///
/// #[derive(Scan)]
/// struct Example {
///     #[shredder(unsafe_skip)]
///     v: NotEvenSendData
/// }
/// ```
pub unsafe trait Scan: GcSafe {
    /// `scan` should use the scanner to scan all of its directly owned data
    fn scan(&self, scanner: &mut Scanner);
}

/// `GcSafe` is a marker trait that indicates that the data can be managed in the background by the
/// garbage collector. Data that is `GcSafe` satisfies the following requirements:
/// 1) It's okay for any thread to call `scan`, as long as it has exclusive access to the data
/// 2) If this data is `'static`, any thread can drop the data safely
/// Requirement (1) can be relaxed if you can ensure that the type does not implement `Scan`
/// (A negative impl can be used to ensure this constraint.)
///
/// Importantly if a type is Send, then it is always `GcSafe`
///
/// NOTE: `GcSafe` cannot simply be `Send`, since `Gc` must be `GcSafe` but sometimes is not `Send`
pub unsafe trait GcSafe {}

/// Scanner is a struct used to manage the scanning of data, sort of analogous to `Hasher`
/// Usually you will only care about this while implementing `Scan`
pub struct Scanner<'a> {
    scan_callback: Box<dyn FnMut(InternalGcRef) + 'a>,
}

#[allow(clippy::unused_self)]
impl<'a> Scanner<'a> {
    #[must_use]
    pub(crate) fn new<F: FnMut(InternalGcRef) + 'a>(callback: F) -> Self {
        Self {
            scan_callback: Box::new(callback),
        }
    }

    /// Scan a piece of data, tracking any `Gc`s found
    pub fn scan<T: Scan>(&mut self, from: &T) {
        from.scan(self);
    }

    /// This function is used internally to fail the `Scan` derive if a field is not `GcSafe`
    /// It's a little bit of a cludge, but that's okay for now
    #[doc(hidden)]
    pub fn check_gc_safe<T: GcSafe>(&self, _: &T) {}

    fn add_internal_handle<T: Scan>(&mut self, gc: &Gc<T>) {
        (self.scan_callback)(gc.internal_handle());
    }
}

// This is a fundamental implementation, since it's how GcInternalHandles make it into the Scanner
// Safety: The implementation is built around this, so it's by definition safe
unsafe impl<T: Scan> Scan for Gc<T> {
    #[allow(clippy::inline_always)]
    #[inline(always)]
    fn scan(&self, scanner: &mut Scanner) {
        scanner.add_internal_handle(self)
    }
}
unsafe impl<T: Scan> GcSafe for Gc<T> {}

// FIXME: This macro can be removed once we have overlapping marker traits
//        (https://github.com/rust-lang/rust/issues/29864)
/// A `Send` type can be safely marked as `GcSafe`, and this macro eases that implementation
#[macro_export]
macro_rules! mark_send_type_gc_safe {
    ( $t:ty ) => {
        unsafe impl GcSafe for $t where $t: Send {}
    };
}

/// `GcSafeWrapper` wraps a `Send` datatype to make it `GcSafe`
/// See the documentation of `Send` to see where this would be useful
pub struct GcSafeWrapper<T: Send> {
    /// The wrapped value
    pub v: T,
}

unsafe impl<T: Send> GcSafe for GcSafeWrapper<T> {}

impl<T: Send> GcSafeWrapper<T> {
    /// Create a new `GcSafeWrapper` storing `v`
    pub fn new(v: T) -> Self {
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
