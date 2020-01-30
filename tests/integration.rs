extern crate shredder;

use std::cell::RefCell;
use std::mem::drop;
use std::ops::Deref;

use once_cell::sync::Lazy;
use parking_lot::Mutex;

use shredder::*;

static TEST_MUTEX: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

#[derive(Debug, Scan)]
struct DirectedGraphNode {
    label: String,
    edges: Vec<Gc<RefCell<DirectedGraphNode>>>,
}

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
