use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use shredder::{Finalize, FinalizeFields};

struct FinalizeMark {
    finalized: Arc<AtomicBool>,
}

unsafe impl Finalize for FinalizeMark {
    unsafe fn finalize(&mut self) {
        self.finalized.store(true, Ordering::SeqCst)
    }
}

#[derive(Finalize, FinalizeFields)]
struct Test {
    m: FinalizeMark,
}

#[test]
fn finalize_derive_works() {
    let finalized = Arc::new(AtomicBool::new(false));
    let mut v = Test {
        m: FinalizeMark {
            finalized: finalized.clone(),
        },
    };

    unsafe {
        v.finalize();
    }

    assert!(finalized.load(Ordering::SeqCst))
}

#[test]
fn finalize_fields_derive_works() {
    let finalized = Arc::new(AtomicBool::new(false));
    let mut v = Test {
        m: FinalizeMark {
            finalized: finalized.clone(),
        },
    };

    unsafe {
        v.finalize_fields();
    }

    assert!(finalized.load(Ordering::SeqCst))
}
