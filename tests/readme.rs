use std::cell::RefCell;

use shredder::{
    number_of_active_handles, number_of_tracked_allocations, run_with_gc_cleanup, Gc, Scan,
};

#[derive(Scan)]
struct Node {
    data: String,
    directed_edges: Vec<Gc<RefCell<Node>>>,
}

#[test]
fn _main() {
    // Using `run_with_gc_cleanup` is good practice, since it helps ensure destructors are run
    run_with_gc_cleanup(|| {
        let a = Gc::new(RefCell::new(Node {
            data: "A".to_string(),
            directed_edges: Vec::new(),
        }));

        let b = Gc::new(RefCell::new(Node {
            data: "B".to_string(),
            directed_edges: Vec::new(),
        }));

        // Usually would need `get` for `Gc` data, but `RefCell` is a special case
        a.borrow_mut().directed_edges.push(b.clone());
        b.borrow_mut().directed_edges.push(a.clone());
        // We now have cyclical data!
    });
    // Everything was cleaned up!
    assert_eq!(number_of_tracked_allocations(), 0);
    assert_eq!(number_of_active_handles(), 0);
}
