use crate::marker::{GcDeref, GcDrop, GcSafe};
use crate::{Finalize, Scan, Scanner};
use std::prelude::v1::*;

use std::cell::{Cell, RefCell};
use std::sync::{Arc, Mutex, RwLock, TryLockError};

// ARC
unsafe impl<T: ?Sized> GcDeref for Arc<T> where T: GcDeref + Send {}
unsafe impl<T: ?Sized> GcDrop for Arc<T> where T: GcDrop {}
unsafe impl<T: ?Sized> GcSafe for Arc<T> where T: GcSafe {}

// CELL
// unsafe impl<T> !GcDeref for Cell<T> where T: GcDeref {}
unsafe impl<T: ?Sized> GcDrop for Cell<T> where T: GcDrop {}
unsafe impl<T: ?Sized> GcSafe for Cell<T> where T: GcSafe {}

unsafe impl<T: Copy + GcSafe + ?Sized> Scan for Cell<T> {
    #[inline(always)]
    fn scan(&self, _: &mut Scanner<'_>) {
        // A `Copy` type cannot contain a `Gc` so we can make this empty
        // TODO: Document this so we can update this method if this changes
    }
}

unsafe impl<T: Finalize + ?Sized> Finalize for Cell<T> {
    unsafe fn finalize(&mut self) {
        self.get_mut().finalize();
    }
}

// MUTEX
// unsafe impl<T> !GcDeref for Mutex<T> where T: GcDeref {}
unsafe impl<T: ?Sized> GcDrop for Mutex<T> where T: GcDrop {}
unsafe impl<T: ?Sized> GcSafe for Mutex<T> where T: GcSafe {}

unsafe impl<T: Scan + ?Sized> Scan for Mutex<T> {
    #[inline]
    fn scan(&self, scanner: &mut Scanner<'_>) {
        match self.try_lock() {
            Ok(data) => {
                let raw: &T = &*data;
                scanner.scan(raw);
            }
            Err(TryLockError::WouldBlock) => {
                error!("A Mutex was in use when it was scanned -- something is buggy here! (no memory unsafety yet, so proceeding...)");
            }
            Err(TryLockError::Poisoned(poison_error)) => {
                let inner_guard = poison_error.into_inner();
                let raw: &T = &*inner_guard;
                scanner.scan(raw);
            }
        }
    }
}

unsafe impl<T: Finalize + ?Sized> Finalize for Mutex<T> {
    unsafe fn finalize(&mut self) {
        let v = self.get_mut();
        match v {
            Ok(v) => v.finalize(),
            Err(e) => e.into_inner().finalize(),
        }
    }
}

// OPTION
unsafe impl<T> GcDeref for Option<T> where T: GcDeref {}
unsafe impl<T> GcDrop for Option<T> where T: GcDrop {}
unsafe impl<T> GcSafe for Option<T> where T: GcSafe {}

unsafe impl<T: Scan> Scan for Option<T> {
    fn scan(&self, scanner: &mut Scanner<'_>) {
        if let Some(v) = self {
            v.scan(scanner);
        }
    }
}

unsafe impl<T: Finalize> Finalize for Option<T> {
    unsafe fn finalize(&mut self) {
        if let Some(v) = self {
            v.finalize();
        }
    }
}

// REFCELL
// unsafe impl<T> !GcDeref for Cell<T> where T: GcDeref {}
unsafe impl<T: ?Sized> GcDrop for RefCell<T> where T: GcDrop {}
unsafe impl<T: ?Sized> GcSafe for RefCell<T> where T: GcSafe {}

unsafe impl<T: Scan + ?Sized> Scan for RefCell<T> {
    #[inline]
    fn scan(&self, scanner: &mut Scanner<'_>) {
        // It's an error if this fails
        if let Ok(reference) = self.try_borrow() {
            let raw: &T = &*reference;
            scanner.scan(raw);
        } else {
            error!("A RefCell was in use when it was scanned -- something is buggy here! (no memory unsafety yet, so proceeding...)")
        }
    }
}

unsafe impl<T: Finalize + ?Sized> Finalize for RefCell<T> {
    unsafe fn finalize(&mut self) {
        self.get_mut().finalize();
    }
}

// RESULT
unsafe impl<T, E> GcDeref for Result<T, E>
where
    T: GcDeref,
    E: GcDeref,
{
}
unsafe impl<T, E> GcDrop for Result<T, E>
where
    T: GcDrop,
    E: GcDrop,
{
}
unsafe impl<T, E> GcSafe for Result<T, E>
where
    T: GcSafe,
    E: GcSafe,
{
}

unsafe impl<T: Scan, E: Scan> Scan for Result<T, E> {
    fn scan(&self, scanner: &mut Scanner<'_>) {
        match self {
            Ok(v) => v.scan(scanner),
            Err(e) => e.scan(scanner),
        }
    }
}

unsafe impl<T: Finalize, E: Finalize> Finalize for Result<T, E> {
    unsafe fn finalize(&mut self) {
        match self {
            Ok(v) => v.finalize(),
            Err(e) => e.finalize(),
        }
    }
}

// RWLOCK
// unsafe impl<T> !GcDeref for Mutex<T> where T: GcDeref {}
unsafe impl<T: ?Sized> GcDrop for RwLock<T> where T: GcDrop {}
unsafe impl<T: ?Sized> GcSafe for RwLock<T> where T: GcSafe {}

unsafe impl<T: Scan + ?Sized> Scan for RwLock<T> {
    #[inline]
    fn scan(&self, scanner: &mut Scanner<'_>) {
        match self.try_read() {
            Ok(data) => {
                let raw: &T = &*data;
                scanner.scan(raw);
            }
            Err(TryLockError::WouldBlock) => {
                error!("A RwLock was in use when it was scanned -- something is buggy here! (no memory unsafety yet, so proceeding...)");
            }
            Err(TryLockError::Poisoned(poison_error)) => {
                let inner_guard = poison_error.into_inner();
                let raw: &T = &*inner_guard;
                scanner.scan(raw);
            }
        }
    }
}

unsafe impl<T: Finalize + ?Sized> Finalize for RwLock<T> {
    unsafe fn finalize(&mut self) {
        let v = self.get_mut();
        match v {
            Ok(v) => v.finalize(),
            Err(e) => e.into_inner().finalize(),
        }
    }
}
