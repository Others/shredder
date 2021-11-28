use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use parking_lot::Condvar;
use parking_lot::Mutex;

const UNSAFE_EXCLUSIVE_SIGNPOST: u64 = !0;
const EXCLUSIVE_SIGNPOST: u64 = UNSAFE_EXCLUSIVE_SIGNPOST - 1;

/// The Lockout mechanism is used internally. It's basically just a `RwLock` that doesn't support
/// blocking on reads. It also has a `LockoutProvider` interface that eases sharing the guards
/// in a non-trivial way
#[derive(Debug)]
pub struct Lockout {
    count: AtomicU64,
    lockout_mutex: Mutex<()>,
    lockout_condvar: Condvar,
}

impl Lockout {
    pub fn new() -> Self {
        Self {
            count: AtomicU64::new(0),
            lockout_mutex: Mutex::new(()),
            lockout_condvar: Condvar::new(),
        }
    }

    pub fn take_warrant<P: LockoutProvider>(provider: P) -> Warrant<P> {
        let lockout = provider.provide();

        let starting_count = lockout.count.load(Ordering::SeqCst);

        // Fast path, where the count is not SIGNPOSTED
        if starting_count < EXCLUSIVE_SIGNPOST {
            let swap_result = lockout.count.compare_exchange(
                starting_count,
                starting_count + 1,
                Ordering::SeqCst,
                Ordering::SeqCst,
            );
            if swap_result.is_ok() {
                return Warrant { provider };
            }
        }

        // Slow path, where we need to wait on a potential signposted val
        let mut guard = lockout.lockout_mutex.lock();
        loop {
            let value = lockout.count.load(Ordering::SeqCst);

            if value >= EXCLUSIVE_SIGNPOST {
                lockout.lockout_condvar.wait(&mut guard);
            } else {
                let swap_result = lockout.count.compare_exchange(
                    value,
                    value + 1,
                    Ordering::SeqCst,
                    Ordering::SeqCst,
                );
                if swap_result.is_ok() {
                    // Dropping the guard early is fine, the warrant has already been taken
                    drop(guard);

                    return Warrant { provider };
                }
            }
        }
    }

    pub fn try_take_exclusive_warrant<P: LockoutProvider>(
        provider: P,
    ) -> Option<ExclusiveWarrant<P>> {
        let lockout = provider.provide();

        let swap_result = lockout.count.compare_exchange(
            0,
            EXCLUSIVE_SIGNPOST,
            Ordering::SeqCst,
            Ordering::SeqCst,
        );

        match swap_result {
            Ok(_) => Some(ExclusiveWarrant { provider }),
            Err(_) => None,
        }
    }

    // Unsafe: only safe if paired with `try_release_exclusive_access_unsafe`
    pub unsafe fn try_take_exclusive_access_unsafe<P: LockoutProvider>(provider: &P) -> bool {
        let lockout = provider.provide();

        let swap_result = lockout.count.compare_exchange(
            0,
            UNSAFE_EXCLUSIVE_SIGNPOST,
            Ordering::SeqCst,
            Ordering::SeqCst,
        );

        swap_result.is_ok()
    }

    // Unsafe: you must guarantee that this is either paired with your `try_take_exclusive_access_unsafe`
    // call. Otherwise, you may be releasing someone else's exclusive access
    //
    // in shredder only the collector uses this method for this reason
    pub unsafe fn try_release_exclusive_access_unsafe<P: LockoutProvider>(provider: &P) {
        let lockout = provider.provide();

        let _guard = lockout.lockout_mutex.lock();

        // It's okay if this fails, since we only are trying to relase if it is taken
        let _ = lockout.count.compare_exchange(
            UNSAFE_EXCLUSIVE_SIGNPOST,
            0,
            Ordering::SeqCst,
            Ordering::SeqCst,
        );

        lockout.lockout_condvar.notify_all();
    }

    pub fn unsafe_exclusive_access_taken<P: LockoutProvider>(provider: &P) -> bool {
        let lockout = provider.provide();
        lockout.count.load(Ordering::SeqCst) == UNSAFE_EXCLUSIVE_SIGNPOST
    }
}

#[derive(Debug)]
pub struct Warrant<P: LockoutProvider> {
    provider: P,
}

impl<P: LockoutProvider> Drop for Warrant<P> {
    fn drop(&mut self) {
        let lockout = self.provider.provide();
        // Safe to assume we can subtract, because the warrant promises we incremented once
        lockout.count.fetch_sub(1, Ordering::SeqCst);
    }
}

#[derive(Debug)]
pub struct ExclusiveWarrant<P: LockoutProvider> {
    provider: P,
}

impl<P: LockoutProvider> Drop for ExclusiveWarrant<P> {
    fn drop(&mut self) {
        let lockout = self.provider.provide();

        let _guard = lockout.lockout_mutex.lock();

        let res = lockout.count.compare_exchange(
            EXCLUSIVE_SIGNPOST,
            0,
            Ordering::SeqCst,
            Ordering::SeqCst,
        );

        debug_assert!(res.is_ok());

        lockout.lockout_condvar.notify_all();
    }
}

pub trait LockoutProvider {
    fn provide(&self) -> &Lockout;
}

impl LockoutProvider for Arc<Lockout> {
    fn provide(&self) -> &Lockout {
        &*self
    }
}

// TODO(issue): https://github.com/Others/shredder/issues/10
#[cfg(test)]
mod test {
    use std::sync::Arc;

    use super::Lockout;

    #[test]
    fn warrant_prevents_exclusive_warrant() {
        let lockout = Arc::new(Lockout::new());
        let _warrant = Lockout::take_warrant(lockout.clone());
        let exclusive_warrant_option = Lockout::try_take_exclusive_warrant(lockout);
        assert!(exclusive_warrant_option.is_none());
    }

    #[test]
    fn exclusive_warrant_works_by_itself() {
        let lockout = Arc::new(Lockout::new());
        let exclusive_warrant_option = Lockout::try_take_exclusive_warrant(lockout);
        assert!(exclusive_warrant_option.is_some());
    }

    #[test]
    fn multiple_warrants() {
        let lockout = Arc::new(Lockout::new());
        let _warrant_1 = Lockout::take_warrant(lockout.clone());
        let _warrant_2 = Lockout::take_warrant(lockout);
    }
}
