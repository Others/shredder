mod collections;
mod value_types;
mod wrap_types;

// TODO(issue): https://github.com/Others/shredder/issues/4
#[cfg(test)]
mod test {
    use std::cell::Cell;
    use std::panic::catch_unwind;
    use std::sync::{Mutex, RwLock};

    use crate::collector::{get_mock_handle, InternalGcRef};
    use crate::marker::GcSafe;
    use crate::{Scan, Scanner};

    struct MockGc {
        handle: InternalGcRef,
    }
    unsafe impl GcSafe for MockGc {}
    unsafe impl Scan for MockGc {
        fn scan(&self, scanner: &mut Scanner<'_>) {
            (scanner.scan_callback)(&self.handle);
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
        let v = vec![MockGc {
            handle: get_mock_handle(),
        }];

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
