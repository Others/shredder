use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::hash::{BuildHasher, Hash, Hasher};
use std::ops::{Deref, DerefMut};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use crate::collector::InternalGcRef;
use crate::Gc;

/// `Scan` is the trait capturing the ability of data to be scanned for references to other Gc data.
///
/// It is unsafe since a bad `scan` implementation can cause memory unsafety. However, importantly,
/// this can only happen if the `scan` scans data that the concrete object does not own. In other
/// words, missing connected data can only cause leaks, not memory unsafety.
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
/// 2) Any thread can drop the data safely
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
    pub fn new<F: FnMut(InternalGcRef) + 'a>(callback: F) -> Self {
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
    fn scan(&self, scanner: &mut Scanner) {
        scanner.add_internal_handle(self)
    }
}
unsafe impl<T: Scan> GcSafe for Gc<T> {}

// References can never own Gc<_> so they must have no-op scan implementations
// Safety: References have no destructors, so they are okay to be GcSafe (since the `scan` impl is empty)
unsafe impl<T> Scan for &T {
    fn scan(&self, _: &mut Scanner) {}
}
unsafe impl<T> GcSafe for &T {}

unsafe impl<T> Scan for &mut T {
    fn scan(&self, _: &mut Scanner) {}
}
unsafe impl<T> GcSafe for &mut T {}

// FIXME: This macro can be removed once we have overlapping marker traits
//        (https://github.com/rust-lang/rust/issues/29864)
/// A `Send` type can be safely marked as `GcSafe`, and this macro easies
#[macro_export]
macro_rules! mark_send_type_gc_safe {
    ( $t:ty ) => {
        unsafe impl GcSafe for $t where $t: Send {}
    };
}

macro_rules! impl_empty_scan_for_send_type {
    ( $t:ty ) => {
        unsafe impl GcSafe for $t where $t: Send {}
        unsafe impl Scan for $t {
            fn scan(&self, _: &mut Scanner) {}
        }
    };
}

/// `GcSafeWrapper` wraps a `Send` datatype to make it `GcSafe`
/// See the documentation of `Send` to see where this would be useful
pub struct GcSafeWrapper<T: Send> {
    /// The wrapped value
    pub v: T,
}
unsafe impl<T: Send> GcSafe for GcSafeWrapper<T> where GcSafeWrapper<T>: Send {}

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

// For collections that own their elements, Collection<T>: Scan iff T: Scan
// Safety: GcSafe is a structural property for normally Send collections
unsafe impl<T: Scan> Scan for Vec<T> {
    fn scan(&self, scanner: &mut Scanner) {
        for e in self {
            scanner.scan(e)
        }
    }
}
unsafe impl<T: GcSafe> GcSafe for Vec<T> {}

unsafe impl<T: Scan, S: BuildHasher> Scan for HashSet<T, S> {
    fn scan(&self, scanner: &mut Scanner) {
        for e in self {
            scanner.scan(e)
        }
    }
}
// FIXME: Would a bad build hasher cause problems?
unsafe impl<T: GcSafe, S: BuildHasher> GcSafe for HashSet<T, S> {}

unsafe impl<K: Scan, V: Scan, S: BuildHasher> Scan for HashMap<K, V, S> {
    fn scan(&self, scanner: &mut Scanner) {
        for (k, v) in self {
            scanner.scan(k);
            scanner.scan(v);
        }
    }
}
// FIXME: Would a bad build hasher cause problems?
unsafe impl<K: GcSafe, V: GcSafe, S: BuildHasher> GcSafe for HashMap<K, V, S> {}

unsafe impl<T: Scan> Scan for RefCell<T> {
    fn scan(&self, scanner: &mut Scanner) {
        // It's an error if this fails
        if let Ok(reference) = self.try_borrow() {
            let raw: &T = reference.deref();
            scanner.scan(raw);
        } else {
            error!("A RefCell was in use when it was scanned -- something is buggy here! (no memory unsafety yet, so proceeding...)")
        }
    }
}
unsafe impl<T: GcSafe> GcSafe for RefCell<T> {}

unsafe impl<T: Scan> Scan for Option<T> {
    fn scan(&self, scanner: &mut Scanner) {
        if let Some(v) = self {
            scanner.scan(v);
        }
    }
}
unsafe impl<T: GcSafe> GcSafe for Option<T> {}

unsafe impl<T: Scan> Scan for Mutex<T> {
    fn scan(&self, scanner: &mut Scanner) {
        // TODO(issue): https://github.com/Others/shredder/issues/6

        // It's okay if we can't scan for now -- if the mutex is locked everything below is still in use
        if let Ok(data) = self.try_lock() {
            let raw: &T = data.deref();
            scanner.scan(raw);
        }
    }
}
unsafe impl<T: GcSafe> GcSafe for Mutex<T> {}

unsafe impl<T: Scan> Scan for RwLock<T> {
    fn scan(&self, scanner: &mut Scanner) {
        // TODO(issue): https://github.com/Others/shredder/issues/6

        // It's okay if we can't scan for now -- if the mutex is locked everything below is still in use
        if let Ok(data) = self.try_read() {
            let raw: &T = data.deref();
            scanner.scan(raw);
        }
    }
}
unsafe impl<T: GcSafe> GcSafe for RwLock<T> {}

// Primitives do not hold any Gc<T>s
impl_empty_scan_for_send_type!(isize);
impl_empty_scan_for_send_type!(usize);

impl_empty_scan_for_send_type!(i8);
impl_empty_scan_for_send_type!(u8);

impl_empty_scan_for_send_type!(i16);
impl_empty_scan_for_send_type!(u16);

impl_empty_scan_for_send_type!(i32);
impl_empty_scan_for_send_type!(u32);

impl_empty_scan_for_send_type!(i64);
impl_empty_scan_for_send_type!(u64);

impl_empty_scan_for_send_type!(i128);
impl_empty_scan_for_send_type!(u128);

// It's nice if other send types from std also get the scan treatment
// These are value types that have no internal content needing a scan
impl_empty_scan_for_send_type!(String);

impl_empty_scan_for_send_type!(Duration);
impl_empty_scan_for_send_type!(Instant);

// impl you need missing? Check the link!
// TODO(issue): https://github.com/Others/shredder/issues/5

// Some other types are GcSafe, but not `Scan`
unsafe impl<T: GcSafe> GcSafe for Arc<T> {}

// TODO(issue): https://github.com/Others/shredder/issues/4
#[cfg(test)]
mod test {
    use crate::collector::{get_mock_handle, InternalGcRef};
    use crate::{GcSafe, Scan, Scanner};

    struct MockGc {
        handle: InternalGcRef,
    }
    unsafe impl GcSafe for MockGc {}
    unsafe impl Scan for MockGc {
        fn scan(&self, scanner: &mut Scanner) {
            (scanner.scan_callback)(self.handle.clone());
        }
    }

    #[test]
    fn vec_scans_correctly() {
        let mut v = Vec::new();
        v.push(MockGc {
            handle: get_mock_handle(),
        });

        let mut count = 0;
        let mut scanner = Scanner::new(|_| {
            count += 1;
        });
        scanner.scan(&v);
        drop(scanner);
        assert_eq!(count, 1);
    }
}
