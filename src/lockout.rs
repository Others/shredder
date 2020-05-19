use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use parking_lot::Condvar;
use parking_lot::Mutex;

const EXCLUSIVE_SIGNPOST: u64 = !0;

/// The Lockout mechanism is used internally. It's basically just a `RwLock` that doesn't support
/// blocking on reads, and gives out guards that use an Arc instead of a reference
#[derive(Debug)]
pub struct Lockout {
    count: AtomicU64,
    lockout_mutex: Mutex<()>,
    lockout_condvar: Condvar,
}

impl Lockout {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            count: AtomicU64::new(0),
            lockout_mutex: Mutex::new(()),
            lockout_condvar: Condvar::new(),
        })
    }

    pub fn get_warrant(self: &Arc<Self>) -> Warrant {
        let starting_count = self.count.load(Ordering::SeqCst);

        // Fast path, where the count is not SIGNPOSTED
        if starting_count != EXCLUSIVE_SIGNPOST {
            let prev_value =
                self.count
                    .compare_and_swap(starting_count, starting_count + 1, Ordering::SeqCst);
            if prev_value == starting_count {
                return Warrant {
                    lockout: self.clone(),
                };
            }
        }

        // Slow path, where we need to wait on a potential signposted val
        let mut guard = self.lockout_mutex.lock();
        loop {
            let value = self.count.load(Ordering::SeqCst);

            if value == EXCLUSIVE_SIGNPOST {
                self.lockout_condvar.wait(&mut guard);
            } else {
                let prev_value = self
                    .count
                    .compare_and_swap(value, value + 1, Ordering::SeqCst);
                if prev_value == value {
                    return Warrant {
                        lockout: self.clone(),
                    };
                }
            }
        }
    }

    pub fn get_exclusive_warrant(self: &Arc<Self>) -> Option<ExclusiveWarrant> {
        let prev_value = self
            .count
            .compare_and_swap(0, EXCLUSIVE_SIGNPOST, Ordering::SeqCst);

        if prev_value == 0 {
            Some(ExclusiveWarrant {
                lockout: self.clone(),
            })
        } else {
            None
        }
    }
}

#[derive(Debug)]
pub struct Warrant {
    lockout: Arc<Lockout>,
}

impl Drop for Warrant {
    fn drop(&mut self) {
        loop {
            let count = self.lockout.count.load(Ordering::SeqCst);
            assert!(count > 0 && count != EXCLUSIVE_SIGNPOST);
            let prev_value =
                self.lockout
                    .count
                    .compare_and_swap(count, count - 1, Ordering::SeqCst);
            if prev_value == count {
                return;
            }
        }
    }
}

#[derive(Debug)]
pub struct ExclusiveWarrant {
    lockout: Arc<Lockout>,
}

impl Drop for ExclusiveWarrant {
    fn drop(&mut self) {
        let _guard = self.lockout.lockout_mutex.lock();
        let prev_count =
            self.lockout
                .count
                .compare_and_swap(EXCLUSIVE_SIGNPOST, 0, Ordering::SeqCst);
        assert_eq!(prev_count, EXCLUSIVE_SIGNPOST);
        self.lockout.lockout_condvar.notify_all();
    }
}

// TODO(issue): https://github.com/Others/shredder/issues/10
