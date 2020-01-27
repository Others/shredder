//! # Shredder
//!
//! `shredder` is a library providing a garbage collected smart pointer: `Gc`
//! This is useful for times where you want an shared access to some data, but the structure
//! of the data has unpredictable cycles in it. (So Arc would not be appropriate.)
//!
//! `shredder` has the following features
//! - fairly ergonomic: no need to manually manage roots, just a regular smart pointer
//! - destructors: no need for finalization, your destructors are seamlessly run
//! - ready for fearless concurrency: works in multi-threaded contexts
//! - safe: detects error conditions on the fly, and protects you from common mistakes
//! - limited stop-the world: no regular processing on data can be interrupted
//! - concurrent collection: collection and destruction happens in the background
//!
//!
//! `shredder` has the following limitations
//! - non-sync ergonomics: `Send` objects need a guard object
//! - multiple collectors: multiple collectors do not co-operate
//! - can't handle `Rc`/`Arc`: requires all `Gc` objects have straightforward ownership semantics
//! - no derive for `Scan`: this would make implementing `Scan` easier (WIP)
//! - non static data: `Gc` cannot handle non 'static data (fix WIP)

// We love docs here
#![deny(missing_docs)]
// Clippy configuration:
// I'd like the most pedantic warning level
#![warn(
    clippy::cargo,
    clippy::needless_borrow,
    clippy::pedantic,
    clippy::redundant_clone
)]
// But I don't care about these ones
#![allow(
    clippy::cast_precision_loss,     // There is no way to avoid this precision loss
    clippy::module_name_repetitions, // Sometimes clear naming calls for repetition
    clippy::multiple_crate_versions  // There is no way to easily fix this without modifying our dependencies
)]

#[macro_use]
extern crate log;

mod collector;
mod lockout;
mod scan;
mod smart_ptr;

use collector::COLLECTOR;

pub use scan::{GcSafe, Scan, Scanner};
pub use smart_ptr::{Gc, GcGuard};

/// Returns how many underlying allocations are currently allocated
#[must_use]
pub fn number_of_tracked_allocations() -> usize {
    COLLECTOR.tracked_data_count()
}

/// Returns how many `Gc`s are currently in use
#[must_use]
pub fn number_of_active_handles() -> usize {
    COLLECTOR.handle_count()
}

// TODO: Consider creating a mechanism for configuration "priority"

/// `shredders` collection automatically triggers when:
/// ```text
///     allocations > allocations_after_last_collection * (1 + gc_trigger_percent)
/// ```
/// The default value of `gc_trigger_percent` is 0.75, but `set_gc_trigger_percent` lets you configure
/// it yourself. Only values 0 or greater are allowed
pub fn set_gc_trigger_percent(percent: f32) {
    if percent < -0.0 || percent.is_nan() {
        panic!(
            "The trigger percentage cannot be less than zero or NaN! (percent = {})",
            percent
        )
    }
    COLLECTOR.set_gc_trigger_percent(percent)
}

/// `collect` allows you to manually run a collection, ignoring the heuristic that governs normal
/// garbage collector operations. This can be an extremely slow operation, since the algorithm is
/// designed to be run in the background, while this method runs it on the thread that calls the
/// method. Additionally, you may end up blocking waiting to collect, since `shredder` doesn't allow
/// two collections at once
#[allow(clippy::must_use_candidate)]
pub fn collect() {
    COLLECTOR.collect();
}

// TODO: Add many more tests
// TODO: Run tests under valgrind
#[cfg(test)]
mod test {
    use std::cell::RefCell;
    use std::mem::drop;
    use std::ops::Deref;

    use once_cell::sync::Lazy;
    use parking_lot::Mutex;

    use super::*;

    static TEST_MUTEX: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

    #[derive(Debug)]
    struct DirectedGraphNode {
        label: String,
        edges: Vec<Gc<RefCell<DirectedGraphNode>>>,
    }

    unsafe impl Scan for DirectedGraphNode {
        fn scan(&self, scanner: &mut Scanner) {
            scanner.scan(&self.edges);
        }
    }
    unsafe impl GcSafe for DirectedGraphNode {}

    #[test]
    fn alloc_u32_gc() {
        let guard = TEST_MUTEX.lock();
        assert_eq!(number_of_tracked_allocations(), 0);

        let val = 7;

        let gc_ptr = Gc::new(val);
        assert_eq!(*gc_ptr.get(), val);
        drop(gc_ptr);

        collect();
        assert_eq!(number_of_tracked_allocations(), 0);
        drop(guard);
    }

    #[test]
    fn alloc_directed_graph_node_gc() {
        let guard = TEST_MUTEX.lock();
        assert_eq!(number_of_tracked_allocations(), 0);

        let node = DirectedGraphNode {
            label: "A".to_string(),
            edges: Vec::new(),
        };

        let gc_ptr = Gc::new(node);
        assert_eq!(gc_ptr.get().label, "A");
        drop(gc_ptr);

        collect();
        assert_eq!(number_of_tracked_allocations(), 0);
        drop(guard);
    }

    #[test]
    fn clone_directed_graph_node_gc() {
        let guard = TEST_MUTEX.lock();
        assert_eq!(number_of_tracked_allocations(), 0);

        let node = DirectedGraphNode {
            label: "A".to_string(),
            edges: Vec::new(),
        };

        let gc_ptr_one = Gc::new(node);
        let gc_ptr_two = gc_ptr_one.clone();
        assert_eq!(number_of_tracked_allocations(), 1);
        assert_eq!(number_of_active_handles(), 2);

        assert_eq!(gc_ptr_one.get().label, "A");
        assert_eq!(gc_ptr_two.get().label, "A");
        drop(gc_ptr_one);
        drop(gc_ptr_two);

        collect();
        assert_eq!(number_of_tracked_allocations(), 0);
        drop(guard);
    }

    #[test]
    fn clone_directed_graph_chain_gc() {
        let guard = TEST_MUTEX.lock();
        assert_eq!(number_of_tracked_allocations(), 0);

        let node = DirectedGraphNode {
            label: "A".to_string(),
            edges: Vec::new(),
        };

        let gc_ptr_one = Gc::new(RefCell::new(node));
        let gc_ptr_two = gc_ptr_one.clone();
        assert_eq!(number_of_tracked_allocations(), 1);
        assert_eq!(number_of_active_handles(), 2);

        assert_eq!(gc_ptr_one.get().borrow().label, "A");
        assert_eq!(gc_ptr_two.get().borrow().label, "A");

        gc_ptr_two
            .get()
            .deref()
            .borrow_mut()
            .edges
            .push(gc_ptr_one.clone());

        drop(gc_ptr_one);
        drop(gc_ptr_two);

        collect();
        assert_eq!(number_of_tracked_allocations(), 0);
        drop(guard);
    }
}
