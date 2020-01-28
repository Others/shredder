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

        a.get().borrow_mut().directed_edges.push(b.clone());
        b.get().borrow_mut().directed_edges.push(a.clone());
    }

    collect();
    assert_eq!(number_of_tracked_allocations(), 0);
}
