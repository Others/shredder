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
mod scan;
mod smart_ptr;

use collector::COLLECTOR;

pub use collector::GcInternalHandle;
pub use scan::Scan;
pub use smart_ptr::{Gc, GcGuard};

#[must_use]
pub fn number_of_tracked_allocations() -> usize {
    COLLECTOR.tracked_data_count()
}

#[must_use]
pub fn number_of_active_handles() -> usize {
    COLLECTOR.handle_count()
}

// TODO: Consider creating a mechanism for configuration "priority"
pub fn set_gc_trigger_percent(percent: f32) {
    if percent < -0.0 || percent.is_nan() {
        panic!(
            "The trigger percentage cannot be less than zero or NaN! (percent = {})",
            percent
        )
    }
    COLLECTOR.set_gc_trigger_percent(percent)
}

#[allow(clippy::must_use_candidate)]
pub fn try_to_collect() -> bool {
    COLLECTOR.collect()
}

// TODO: Add Gc cleanup+shutdown function

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
        fn scan(&self, out: &mut Vec<GcInternalHandle>) {
            self.edges.scan(out);
        }
    }

    #[test]
    fn alloc_u32_gc() {
        let guard = TEST_MUTEX.lock();
        assert_eq!(number_of_tracked_allocations(), 0);

        let val = 7;

        let gc_ptr = Gc::new(val);
        assert_eq!(*gc_ptr.get(), val);
        drop(gc_ptr);

        assert!(try_to_collect());
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

        assert!(try_to_collect());
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

        assert!(try_to_collect());
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

        assert!(try_to_collect());
        assert_eq!(number_of_tracked_allocations(), 0);
        drop(guard);
    }
}
