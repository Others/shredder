use std::fmt::{self, Debug, Formatter};
use std::ops::{Deref, DerefMut};
use std::sync::{self, TryLockError};

use crate::{Gc, Scan};

/// An error representing that the `Mutex` or `RWLock` you tried to lock was poisoned
///
/// It contains a locked guard which you can recover with `into_inner`
#[derive(Debug)]
pub struct GcPoisonError<T> {
    guard: T,
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
rental! {
    mod gc_mutex_internals {
        use crate::{Scan, GcGuard};
        use std::sync::{Mutex, MutexGuard};

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

impl<T: Scan + 'static> Gc<sync::Mutex<T>> {
    /// Call the underlying `lock` method on the inner `Mutex`
    ///
    /// This is just a nice method so you don't have to `get` manually
    ///
    /// # Errors
    /// Returns a `GcPoisonError` if the underlying `.lock` method returns an error.
    /// You may use `into_inner` in order to recover the guard from that error.
    pub fn lock(&self) -> Result<GcMutexGuard<'_, T>, GcPoisonError<GcMutexGuard<'_, T>>> {
        let g = self.get();
        let mut was_poisoned = false;
        let internal_guard = gc_mutex_internals::GcMutexGuardInt::new(g, |g| match g.lock() {
            Ok(v) => v,
            Err(e) => {
                was_poisoned = true;
                e.into_inner()
            }
        });

        let guard = GcMutexGuard { internal_guard };

        if was_poisoned {
            Err(GcPoisonError { guard })
        } else {
            Ok(guard)
        }
    }

    /// Call the underlying `try_lock` method on the inner `Mutex`
    ///
    /// This is just a nice method so you don't have to `get` manually
    ///
    /// # Errors
    /// Returns a `GcTryLockError` if the underlying `.try_lock` method returns an error
    pub fn try_lock(&self) -> Result<GcMutexGuard<'_, T>, GcTryLockError<GcMutexGuard<'_, T>>> {
        let g = self.get();
        let mut was_poisoned = false;
        let internal_guard =
            gc_mutex_internals::GcMutexGuardInt::try_new(g, |g| match g.try_lock() {
                Ok(g) => Ok(g),
                Err(TryLockError::Poisoned(e)) => {
                    was_poisoned = true;
                    Ok(e.into_inner())
                }
                Err(TryLockError::WouldBlock) => {
                    Err(GcTryLockError::<GcMutexGuard<'_, T>>::WouldBlock)
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
