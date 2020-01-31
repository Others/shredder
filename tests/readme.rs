use std::cell::RefCell;

use shredder::{collect, number_of_tracked_allocations, Gc, Scan};

#[derive(Scan)]
struct Node {
    data: String,
    directed_edges: Vec<Gc<RefCell<Node>>>,
}

#[test]
fn _main() {
    {
        let a = Gc::new(RefCell::new(Node {
            data: "A".to_string(),
            directed_edges: Vec::new(),
        }));

        let b = Gc::new(RefCell::new(Node {
            data: "B".to_string(),
            directed_edges: Vec::new(),
        }));

        // Usually would need `get` for non-`Sync` data, but `RefCell` is a special case
        a.borrow_mut().directed_edges.push(b.clone());
        b.borrow_mut().directed_edges.push(a.clone());
    }

    // Running `collect` like this, at the end of main (after everything is dropped) is good practice
    // It helps ensure destructors are run
    collect();
    assert_eq!(number_of_tracked_allocations(), 0);
}
