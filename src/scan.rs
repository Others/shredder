use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::hash::BuildHasher;
use std::ops::Deref;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::collector::GcInternalHandle;
use crate::Gc;

// TODO: Add non-'static data as an option
//  Enhance Scan with distinction between options
//  Add flag, so we don't run destructors for non-'static data

// TODO: Add a Scan auto-derive

// TODO: Create a Safe way for non-Send data to be used

/// `Scan` is the trait capturing the ability of data to be scanned for references to other Gc data.
///
/// It is unsafe since a bad `scan` implementation can cause memory unsafety. However, importantly,
/// this can only happen if the `scan` scans data that the concrete object does not own. In other
/// words, missing connected data can only cause leaks, not memory unsafety.
///
/// Note: It's important that `scan` only scans data that is truly owned. `Rc`/`Arc` cannot have
/// sensible `scan` implementations, since each individual smart pointer doesn't own the underlying
/// data.
///
/// Importantly, any empty `scan` implementation is safe (assuming the `GcSafe` impl is correct)
pub unsafe trait Scan: GcSafe {
    /// `scan` should use the scanner to scan all of its directly owned data
    fn scan(&self, scanner: &mut Scanner);
}

/// `GcSafe` is a marker trait that indicates that the data can be managed in the background by the
/// garbage collector. Data that is `GcSafe` satisfies the following requirements:
/// 1) It's okay for any thread to call `scan`, as long as it has exclusive access to the data
/// 2) Any thread can drop the data safely
/// Importantly if a type is Send, then it is always `GcSafe`
///
/// NOTE: `GcSafe` cannot simply be `Send`, since `Gc` must be `GcSafe` but sometimes is not `Send`
pub unsafe trait GcSafe {}

/// Scanner is a struct used to manage the scanning of data, sort of analogous to `Hasher`
/// Usually you will only care about this while implementing `Scan`
#[derive(Debug, Default)]
pub struct Scanner {
    found: Vec<GcInternalHandle>,
}

impl Scanner {
    /// Create a new Scanner, with nothing found yet
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Scan a piece of data, tracking any `Gc`s found
    pub fn scan<T: Scan>(&mut self, from: &T) {
        from.scan(self);
    }

    fn add_internal_handle<T: Scan>(&mut self, gc: &Gc<T>) {
        self.found.push(gc.internal_handle())
    }

    pub(crate) fn extract_found_handles(self) -> Vec<GcInternalHandle> {
        self.found
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

// For collections that own their elements, Collection<T>: Scan iff T: Scan
// Safety: GcSafe is a structural property for normally Send collections
unsafe impl<T: Scan> Scan for Vec<T> {
    fn scan(&self, scanner: &mut Scanner) {
        for e in self {
            scanner.scan(e)
        }
    }
}
unsafe impl<T: Scan> GcSafe for Vec<T> {}

unsafe impl<T: Scan, S: BuildHasher> Scan for HashSet<T, S> {
    fn scan(&self, scanner: &mut Scanner) {
        for e in self {
            scanner.scan(e)
        }
    }
}
unsafe impl<T: Scan, S: BuildHasher> GcSafe for HashSet<T, S> {}

unsafe impl<K: Scan, V: Scan, S: BuildHasher> Scan for HashMap<K, V, S> {
    fn scan(&self, scanner: &mut Scanner) {
        for (k, v) in self {
            scanner.scan(k);
            scanner.scan(v);
        }
    }
}
unsafe impl<K: Scan, V: Scan, S: BuildHasher> GcSafe for HashMap<K, V, S> {}

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
unsafe impl<T: Scan> GcSafe for RefCell<T> {}

unsafe impl<T: Scan> Scan for Mutex<T> {
    fn scan(&self, scanner: &mut Scanner) {
        // TODO: Consider the treatment of poisoned mutexes

        // It's okay if we can't scan for now -- if the mutex is locked everything below is still in use
        if let Ok(data) = self.try_lock() {
            let raw: &T = data.deref();
            scanner.scan(raw);
        }
    }
}
unsafe impl<T: Scan> GcSafe for Mutex<T> {}

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
impl_empty_scan_for_send_type!(Duration);
impl_empty_scan_for_send_type!(Instant);
// TODO: Add more Scan impls here

// All code bellow here used for testing only
// TODO: Add tests for scan impls
#[cfg(test)]
mod test {
    use crate::collector::GcInternalHandle;
    use crate::{GcSafe, Scan, Scanner};

    struct MockGc {
        handle: GcInternalHandle,
    }
    unsafe impl GcSafe for MockGc {}
    unsafe impl Scan for MockGc {
        fn scan(&self, scanner: &mut Scanner) {
            scanner.found.push(self.handle.clone())
        }
    }

    #[test]
    fn vec_scans_correctly() {
        let mut v = Vec::new();
        v.push(MockGc {
            handle: GcInternalHandle::new(0),
        });

        let mut scanner = Scanner::new();
        scanner.scan(&v);
        assert_eq!(scanner.found.len(), 1);
    }
}
