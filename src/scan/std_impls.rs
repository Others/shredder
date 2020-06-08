use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::hash::BuildHasher;
use std::ops::Deref;
use std::sync::{Arc, Mutex, RwLock, TryLockError};
use std::time::{Duration, Instant};

use crate::{GcSafe, Scan, Scanner};

macro_rules! impl_empty_scan_for_send_type {
    ( $t:ty ) => {
        unsafe impl GcSafe for $t where $t: Send {}
        unsafe impl Scan for $t {
            #[inline(always)]
            fn scan(&self, _: &mut Scanner<'_>) {}
        }
    };
}

// For collections that own their elements, Collection<T>: Scan iff T: Scan
// Safety: GcSafe is a structural property for normally Send collections
unsafe impl<T: Scan> Scan for Vec<T> {
    #[inline]
    fn scan(&self, scanner: &mut Scanner<'_>) {
        for e in self {
            scanner.scan(e)
        }
    }
}
unsafe impl<T: GcSafe> GcSafe for Vec<T> {}

unsafe impl<T: Scan, S: BuildHasher> Scan for HashSet<T, S> {
    #[inline]
    fn scan(&self, scanner: &mut Scanner<'_>) {
        for e in self {
            scanner.scan(e)
        }
    }
}
// FIXME: Would a bad build hasher cause problems?
unsafe impl<T: GcSafe, S: BuildHasher> GcSafe for HashSet<T, S> {}

unsafe impl<K: Scan, V: Scan, S: BuildHasher> Scan for HashMap<K, V, S> {
    #[inline]
    fn scan(&self, scanner: &mut Scanner<'_>) {
        for (k, v) in self {
            scanner.scan(k);
            scanner.scan(v);
        }
    }
}
// FIXME: Would a bad build hasher cause problems?
unsafe impl<K: GcSafe, V: GcSafe, S: BuildHasher> GcSafe for HashMap<K, V, S> {}

unsafe impl<T: Scan> Scan for RefCell<T> {
    #[inline]
    fn scan(&self, scanner: &mut Scanner<'_>) {
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
    #[inline]
    fn scan(&self, scanner: &mut Scanner<'_>) {
        if let Some(v) = self {
            scanner.scan(v);
        }
    }
}
unsafe impl<T: GcSafe> GcSafe for Option<T> {}

unsafe impl<T: Scan> Scan for Mutex<T> {
    #[inline]
    fn scan(&self, scanner: &mut Scanner<'_>) {
        match self.try_lock() {
            Ok(data) => {
                let raw: &T = data.deref();
                scanner.scan(raw);
            }
            Err(TryLockError::WouldBlock) => {
                error!("A Mutex was in use when it was scanned -- something is buggy here! (no memory unsafety yet, so proceeding...)");
            }
            Err(TryLockError::Poisoned(_)) => {
                // TODO(issue): https://github.com/Others/shredder/issues/6
            }
        }
    }
}
unsafe impl<T: GcSafe> GcSafe for Mutex<T> {}

unsafe impl<T: Scan> Scan for RwLock<T> {
    #[inline]
    fn scan(&self, scanner: &mut Scanner<'_>) {
        match self.try_read() {
            Ok(data) => {
                let raw: &T = data.deref();
                scanner.scan(raw);
            }
            Err(TryLockError::WouldBlock) => {
                error!("A RwLock was in use when it was scanned -- something is buggy here! (no memory unsafety yet, so proceeding...)");
            }
            Err(TryLockError::Poisoned(_)) => {
                // TODO(issue): https://github.com/Others/shredder/issues/6
            }
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
unsafe impl<T: GcSafe> GcSafe for Arc<T> where Arc<T>: Send {}

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
        fn scan(&self, scanner: &mut Scanner<'_>) {
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
