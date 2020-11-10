use std::cell::{BorrowError, BorrowMutError, RefCell};
use std::fmt::{self, Debug, Formatter};
use std::ops::{Deref, DerefMut};
use std::prelude::v1::*;
use std::sync;

#[cfg(feature = "std")]
use std::sync::TryLockError;

use crate::{GcGuard, Scan};

// This is special casing for Gc<RefCell<T>>
rental! {
    mod gc_refcell_internals {
        use crate::{Scan, GcGuard};
        use std::cell::{Ref, RefCell, RefMut};

        /// Self referential wrapper around `Ref` for ergonomics
        #[rental(deref_suffix)]
        pub struct GcRefInt<'a, T: Scan + 'static> {
            gc_guard: GcGuard<'a, RefCell<T>>,
            cell_ref: Ref<'gc_guard, T>
        }

        /// Self referential wrapper around `RefMut` for ergonomics
        #[rental(deref_mut_suffix)]
        pub struct GcRefMutInt<'a, T: Scan + 'static> {
            gc_guard: GcGuard<'a, RefCell<T>>,
            cell_ref: RefMut<'gc_guard, T>
        }
    }
}

/// This is like a `Ref`, but taken directly from a `Gc`
pub struct GcRef<'a, T: Scan + 'static> {
    internal_ref: gc_refcell_internals::GcRefInt<'a, T>,
}

impl<T: Scan + 'static + Debug> Debug for GcRef<'_, T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("GcRef").field("ref", self.deref()).finish()
    }
}

impl<'a, T: Scan + 'static> GcRef<'a, T> {
    pub(crate) fn borrow(g: GcGuard<'a, RefCell<T>>) -> Self {
        let internal_ref = gc_refcell_internals::GcRefInt::new(g, RefCell::borrow);
        Self { internal_ref }
    }

    pub(crate) fn try_borrow(g: GcGuard<'a, RefCell<T>>) -> Result<Self, BorrowError> {
        let internal_ref =
            gc_refcell_internals::GcRefInt::try_new(g, RefCell::try_borrow).map_err(|e| e.0)?;

        Ok(Self { internal_ref })
    }
}

impl<'a, T: Scan + 'static> Deref for GcRef<'a, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.internal_ref.deref()
    }
}

/// This is like a `RefMut`, but taken directly from a `Gc`
pub struct GcRefMut<'a, T: Scan + 'static> {
    internal_ref: gc_refcell_internals::GcRefMutInt<'a, T>,
}

impl<'a, T: Scan + 'static> GcRefMut<'a, T> {
    pub(crate) fn borrow_mut(g: GcGuard<'a, RefCell<T>>) -> Self {
        let internal_ref = gc_refcell_internals::GcRefMutInt::new(g, RefCell::borrow_mut);
        Self { internal_ref }
    }

    pub(crate) fn try_borrow_mut(g: GcGuard<'a, RefCell<T>>) -> Result<Self, BorrowMutError> {
        let internal_ref = gc_refcell_internals::GcRefMutInt::try_new(g, RefCell::try_borrow_mut)
            .map_err(|e| e.0)?;

        Ok(Self { internal_ref })
    }
}

impl<T: Scan + 'static + Debug> Debug for GcRefMut<'_, T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("GcRefMut")
            .field("ref", self.deref())
            .finish()
    }
}

impl<'a, T: Scan + 'static> Deref for GcRefMut<'a, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.internal_ref.deref()
    }
}

impl<'a, T: Scan + 'static> DerefMut for GcRefMut<'a, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.internal_ref.deref_mut()
    }
}

/// An error representing that the `Mutex` or `RwLock` you tried to lock was poisoned
///
/// It contains a locked guard which you can recover with `into_inner`
#[derive(Debug)]
pub struct GcPoisonError<T> {
    pub(crate) guard: T,
}

impl<T> GcPoisonError<T> {
    /// Recover the guard from inside this error
    pub fn into_inner(self) -> T {
        self.guard
    }
}

/// An error representing that there was some reason you couldn't lock with `try_lock`
#[derive(Debug)]
pub enum GcTryLockError<T> {
    /// The lock was poisoned, so here is a `GcPoisonError`
    Poisoned(GcPoisonError<T>),
    /// The operation would block
    WouldBlock,
}

// This is special casing for Gc<Mutex<T>>
// TODO: Rename `cell_ref`
rental! {
    mod gc_mutex_internals {
        use std::sync::{Mutex, MutexGuard};

        use crate::{Scan, GcGuard};

        /// Self referential wrapper around `MutexGuard` for ergonomics
        #[rental(deref_mut_suffix)]
        pub struct GcMutexGuardInt<'a, T: Scan + 'static> {
            gc_guard: GcGuard<'a, Mutex<T>>,
            cell_ref: MutexGuard<'gc_guard, T>
        }
    }
}

/// This is like a `MutexGuard`, but taken directly from a `Gc`
pub struct GcMutexGuard<'a, T: Scan + 'static> {
    internal_guard: gc_mutex_internals::GcMutexGuardInt<'a, T>,
}

impl<'a, T: Scan + 'static> GcMutexGuard<'a, T> {
    pub(crate) fn lock(g: GcGuard<'a, sync::Mutex<T>>) -> Result<Self, GcPoisonError<Self>> {
        let mut was_poisoned = false;
        let internal_guard = gc_mutex_internals::GcMutexGuardInt::new(g, |g| {
            #[cfg(feature = "std")]
            match g.lock() {
                Ok(v) => v,
                Err(e) => {
                    was_poisoned = true;
                    e.into_inner()
                }
            }

            #[cfg(not(feature = "std"))]
            g.lock()
        });

        let guard = Self { internal_guard };

        if was_poisoned {
            Err(GcPoisonError { guard })
        } else {
            Ok(guard)
        }
    }

    pub(crate) fn try_lock(g: GcGuard<'a, sync::Mutex<T>>) -> Result<Self, GcTryLockError<Self>> {
        let mut was_poisoned = false;
        let internal_guard = gc_mutex_internals::GcMutexGuardInt::try_new(g, |g| {
            #[cfg(feature = "std")]
            match g.try_lock() {
                Ok(g) => Ok(g),
                Err(TryLockError::Poisoned(e)) => {
                    was_poisoned = true;
                    Ok(e.into_inner())
                }
                Err(TryLockError::WouldBlock) => {
                    Err(GcTryLockError::<GcMutexGuard<'_, T>>::WouldBlock)
                }
            }

            #[cfg(not(feature = "std"))]
            match g.try_lock() {
                Some(g) => Ok(g),
                None => Err(GcTryLockError::<GcMutexGuard<'_, T>>::WouldBlock),
            }
        })
        .map_err(|e| e.0)?;

        let guard = GcMutexGuard { internal_guard };

        if was_poisoned {
            Err(GcTryLockError::Poisoned(GcPoisonError { guard }))
        } else {
            Ok(guard)
        }
    }
}

impl<T: Scan + 'static> Deref for GcMutexGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.internal_guard.deref()
    }
}

impl<T: Scan + 'static> DerefMut for GcMutexGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.internal_guard.deref_mut()
    }
}

impl<T: Scan + 'static + Debug> Debug for GcMutexGuard<'_, T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("GcMutexGuard")
            .field("guarding", self.deref())
            .finish()
    }
}

rental! {
    mod gc_rwlock_internals {
        use std::sync::{RwLock, MutexGuard, RwLockReadGuard, RwLockWriteGuard};

        use crate::{Scan, GcGuard};

        /// Self referential wrapper around `RwLockReadGuard` for ergonomics
        #[rental(deref_suffix)]
        pub struct GcRwLockReadGuardInternal<'a, T: Scan + 'static> {
            gc_guard: GcGuard<'a, RwLock<T>>,
            cell_ref: RwLockReadGuard<'gc_guard, T>
        }

        /// Self referential wrapper around `RwLockReadGuard` for ergonomics
        #[rental(deref_mut_suffix)]
        pub struct GcRwLockWriteGuardInternal<'a, T: Scan + 'static> {
            gc_guard: GcGuard<'a, RwLock<T>>,
            cell_ref: RwLockWriteGuard<'gc_guard, T>
        }
    }
}

/// A wrapper around a `RwLockReadGuard` taken directly from a `Gc`
pub struct GcRwLockReadGuard<'a, T: Scan + 'static> {
    internal_guard: gc_rwlock_internals::GcRwLockReadGuardInternal<'a, T>,
}

impl<'a, T: Scan + 'static> GcRwLockReadGuard<'a, T> {
    pub(crate) fn read(g: GcGuard<'a, sync::RwLock<T>>) -> Result<Self, GcPoisonError<Self>> {
        let mut was_poisoned = false;
        let internal_guard = gc_rwlock_internals::GcRwLockReadGuardInternal::new(g, |g| {
            #[cfg(feature = "std")]
            match g.read() {
                Ok(v) => v,
                Err(e) => {
                    was_poisoned = true;
                    e.into_inner()
                }
            }

            #[cfg(not(feature = "std"))]
            g.read()
        });

        let guard = Self { internal_guard };

        if was_poisoned {
            Err(GcPoisonError { guard })
        } else {
            Ok(guard)
        }
    }

    pub(crate) fn try_read(g: GcGuard<'a, sync::RwLock<T>>) -> Result<Self, GcTryLockError<Self>> {
        let mut was_poisoned = false;

        let internal_guard = gc_rwlock_internals::GcRwLockReadGuardInternal::try_new(g, |g| {
            #[cfg(feature = "std")]
            match g.try_read() {
                Ok(g) => Ok(g),
                Err(TryLockError::Poisoned(e)) => {
                    was_poisoned = true;
                    Ok(e.into_inner())
                }
                Err(TryLockError::WouldBlock) => {
                    Err(GcTryLockError::<GcRwLockReadGuard<'_, T>>::WouldBlock)
                }
            }

            #[cfg(not(feature = "std"))]
            match g.try_read() {
                Some(g) => Ok(g),
                None => Err(GcTryLockError::<GcRwLockReadGuard<'_, T>>::WouldBlock),
            }
        })
        .map_err(|e| e.0)?;

        let guard = Self { internal_guard };

        if was_poisoned {
            Err(GcTryLockError::Poisoned(GcPoisonError { guard }))
        } else {
            Ok(guard)
        }
    }
}

impl<T: Scan + 'static + Debug> Debug for GcRwLockReadGuard<'_, T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("GcRwLockReadGuard")
            .field("guarding", self.deref())
            .finish()
    }
}

impl<'a, T: Scan + 'static> Deref for GcRwLockReadGuard<'a, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.internal_guard.deref()
    }
}

/// A wrapper around a `RwLockWriteGuard` taken directly from a `Gc`
pub struct GcRwLockWriteGuard<'a, T: Scan + 'static> {
    internal_guard: gc_rwlock_internals::GcRwLockWriteGuardInternal<'a, T>,
}

impl<'a, T: Scan + 'static> GcRwLockWriteGuard<'a, T> {
    pub(crate) fn write(g: GcGuard<'a, sync::RwLock<T>>) -> Result<Self, GcPoisonError<Self>> {
        let mut was_poisoned = false;
        let internal_guard = gc_rwlock_internals::GcRwLockWriteGuardInternal::new(g, |g| {
            #[cfg(feature = "std")]
            match g.write() {
                Ok(v) => v,
                Err(e) => {
                    was_poisoned = true;
                    e.into_inner()
                }
            }

            #[cfg(not(feature = "std"))]
            g.write()
        });

        let guard = Self { internal_guard };

        if was_poisoned {
            Err(GcPoisonError { guard })
        } else {
            Ok(guard)
        }
    }

    pub(crate) fn try_write(g: GcGuard<'a, sync::RwLock<T>>) -> Result<Self, GcTryLockError<Self>> {
        let mut was_poisoned = false;
        let internal_guard = gc_rwlock_internals::GcRwLockWriteGuardInternal::try_new(g, |g| {
            #[cfg(feature = "std")]
            match g.try_write() {
                Ok(g) => Ok(g),
                Err(TryLockError::Poisoned(e)) => {
                    was_poisoned = true;
                    Ok(e.into_inner())
                }
                Err(TryLockError::WouldBlock) => {
                    Err(GcTryLockError::<GcRwLockWriteGuard<'_, T>>::WouldBlock)
                }
            }

            #[cfg(not(feature = "std"))]
            match g.try_write() {
                Some(g) => Ok(g),
                None => Err(GcTryLockError::<GcRwLockWriteGuard<'_, T>>::WouldBlock),
            }
        })
        .map_err(|e| e.0)?;

        let guard = GcRwLockWriteGuard { internal_guard };

        if was_poisoned {
            Err(GcTryLockError::Poisoned(GcPoisonError { guard }))
        } else {
            Ok(guard)
        }
    }
}

impl<T: Scan + 'static + Debug> Debug for GcRwLockWriteGuard<'_, T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("GcRwLockWriteGuard")
            .field("guarding", self.deref())
            .finish()
    }
}

impl<'a, T: Scan + 'static> Deref for GcRwLockWriteGuard<'a, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.internal_guard.deref()
    }
}

impl<'a, T: Scan + 'static> DerefMut for GcRwLockWriteGuard<'a, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.internal_guard.deref_mut()
    }
}
