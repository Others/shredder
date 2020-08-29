use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use parking_lot::Condvar;
use parking_lot::Mutex;

const EXCLUSIVE_SIGNPOST: u64 = !0;

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

    pub fn get_warrant<P: LockoutProvider>(provider: P) -> Warrant<P> {
        let lockout = provider.provide();

        let starting_count = lockout.count.load(Ordering::SeqCst);

        // Fast path, where the count is not SIGNPOSTED
        if starting_count != EXCLUSIVE_SIGNPOST {
            let prev_value = lockout.count.compare_and_swap(
                starting_count,
                starting_count + 1,
                Ordering::SeqCst,
            );
            if prev_value == starting_count {
                return Warrant { provider };
            }
        }

        // Slow path, where we need to wait on a potential signposted val
        let mut guard = lockout.lockout_mutex.lock();
        loop {
            let value = lockout.count.load(Ordering::SeqCst);

            if value == EXCLUSIVE_SIGNPOST {
                lockout.lockout_condvar.wait(&mut guard);
            } else {
                let prev_value = lockout
                    .count
                    .compare_and_swap(value, value + 1, Ordering::SeqCst);
                if prev_value == value {
                    // Dropping the guard early is fine, the warrant has already been taken
                    drop(guard);

                    return Warrant { provider };
                }
            }
        }
    }

    pub fn get_exclusive_warrant<P: LockoutProvider>(provider: P) -> Option<ExclusiveWarrant<P>> {
        let lockout = provider.provide();

        let prev_value = lockout
            .count
            .compare_and_swap(0, EXCLUSIVE_SIGNPOST, Ordering::SeqCst);

        if prev_value == 0 {
            Some(ExclusiveWarrant { provider })
        } else {
            None
        }
    }
}

#[derive(Debug)]
pub struct Warrant<P: LockoutProvider> {
    provider: P,
}

impl<P: LockoutProvider> Drop for Warrant<P> {
    fn drop(&mut self) {
        loop {
            let lockout = self.provider.provide();

            let count = lockout.count.load(Ordering::SeqCst);
            assert!(count > 0 && count != EXCLUSIVE_SIGNPOST);
            let prev_value = lockout
                .count
                .compare_and_swap(count, count - 1, Ordering::SeqCst);
            if prev_value == count {
                return;
            }
        }
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
        let prev_count = lockout
            .count
            .compare_and_swap(EXCLUSIVE_SIGNPOST, 0, Ordering::SeqCst);
        assert_eq!(prev_count, EXCLUSIVE_SIGNPOST);
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
        let _warrant = Lockout::get_warrant(lockout.clone());
        let exclusive_warrant_option = Lockout::get_exclusive_warrant(lockout);
        assert!(exclusive_warrant_option.is_none());
    }

    #[test]
    fn exclusive_warrant_works_by_itself() {
        let lockout = Arc::new(Lockout::new());
        let exclusive_warrant_option = Lockout::get_exclusive_warrant(lockout);
        assert!(exclusive_warrant_option.is_some());
    }

    #[test]
    fn multiple_warrants() {
        let lockout = Arc::new(Lockout::new());
        let _warrant_1 = Lockout::get_warrant(lockout.clone());
        let _warrant_2 = Lockout::get_warrant(lockout);
    }
}
