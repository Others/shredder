use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::hash::BuildHasher;
use std::ops::Deref;
use std::sync::{Arc, Mutex, RwLock, TryLockError};
use std::time::{Duration, Instant};

use crate::{EmptyScan, GcSafe, Scan, Scanner};

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

unsafe impl<T: Copy> Scan for Cell<T>
where
    T: GcSafe,
{
    #[inline(always)]
    fn scan(&self, _: &mut Scanner<'_>) {
        // A `Copy` type cannot contain a `Gc` so we can make this empty
        // TODO: Document this so we can update this method if this changes
    }
}
unsafe impl<T: GcSafe> GcSafe for Cell<T> {}

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
            Err(TryLockError::Poisoned(poison_error)) => {
                let inner_guard = poison_error.into_inner();
                let raw: &T = inner_guard.deref();
                scanner.scan(raw);
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
            Err(TryLockError::Poisoned(poison_error)) => {
                let inner_guard = poison_error.into_inner();
                let raw: &T = inner_guard.deref();
                scanner.scan(raw);
            }
        }
    }
}
unsafe impl<T: GcSafe> GcSafe for RwLock<T> {}

// Primitives do not hold any Gc<T>s
impl EmptyScan for isize {}
impl EmptyScan for usize {}

impl EmptyScan for i8 {}
impl EmptyScan for u8 {}

impl EmptyScan for i16 {}
impl EmptyScan for u16 {}

impl EmptyScan for i32 {}
impl EmptyScan for u32 {}

impl EmptyScan for i64 {}
impl EmptyScan for u64 {}

impl EmptyScan for i128 {}
impl EmptyScan for u128 {}

// It's nice if other send types from std also get the scan treatment
// These are value types that have no internal content needing a scan
impl EmptyScan for String {}

impl EmptyScan for Duration {}
impl EmptyScan for Instant {}

// impl you need missing? Check the link!
// TODO(issue): https://github.com/Others/shredder/issues/5

// Some other types are GcSafe, but not `Scan`
unsafe impl<T: GcSafe> GcSafe for Arc<T> where Arc<T>: Send {}

// TODO(issue): https://github.com/Others/shredder/issues/4
#[cfg(test)]
mod test {
    use std::cell::Cell;
    use std::panic::catch_unwind;
    use std::sync::{Mutex, RwLock};

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
    fn cell_scans() {
        let cell2: Cell<Option<u32>> = Cell::new(None);
        let mut count = 0;
        let mut scanner = Scanner::new(|_| {
            count += 1;
        });
        scanner.scan(&cell2);
        drop(scanner);
        assert_eq!(count, 0);
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

    #[test]
    fn unpoisoned_mutex_scans() {
        let m = Mutex::new(MockGc {
            handle: get_mock_handle(),
        });

        let mut count = 0;
        let mut scanner = Scanner::new(|_| {
            count += 1;
        });
        scanner.scan(&m);

        drop(scanner);
        assert_eq!(count, 1);
    }

    #[test]
    fn poisoned_mutex_scans() {
        let m = Mutex::new(MockGc {
            handle: get_mock_handle(),
        });

        let catch_res = catch_unwind(|| {
            let _guard = m.lock().unwrap();
            panic!("test panic!");
        });
        assert!(catch_res.is_err());

        let mut count = 0;
        let mut scanner = Scanner::new(|_| {
            count += 1;
        });
        scanner.scan(&m);

        drop(scanner);
        assert_eq!(count, 1);
    }

    #[test]
    fn unpoisoned_rwlock_scans() {
        let m = RwLock::new(MockGc {
            handle: get_mock_handle(),
        });

        let mut count = 0;
        let mut scanner = Scanner::new(|_| {
            count += 1;
        });
        scanner.scan(&m);

        drop(scanner);
        assert_eq!(count, 1);
    }

    #[test]
    fn poisoned_rwlock_scans() {
        let m = RwLock::new(MockGc {
            handle: get_mock_handle(),
        });

        let catch_res = catch_unwind(|| {
            let _guard = m.read().unwrap();
            panic!("test panic!");
        });
        assert!(catch_res.is_err());

        let mut count = 0;
        let mut scanner = Scanner::new(|_| {
            count += 1;
        });
        scanner.scan(&m);

        drop(scanner);
        assert_eq!(count, 1);
    }
}
