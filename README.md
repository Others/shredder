shredder
========
`shredder` is a library providing a garbage collected smart pointer: `Gc`.
This is useful for times where you want shared access to some data, but the structure
of the data has unpredictable cycles in it. (So Arc would not be appropriate.)

`shredder` has the following features:
- fairly ergonomic: no need to manually manage roots, just a regular smart pointer
- destructors: no need for finalization, your destructors are seamlessly run
- ready for fearless concurrency: works in multi-threaded contexts
- safe: detects error conditions on the fly, and protects you from undefined behavior
- limited stop-the world: regular processing will rarely be interrupted
- concurrent collection: collection happens in the background
- concurrent destruction: destructors are run in the background

`shredder` has the following limitations:
- guarded access: `Gc` requires acquiring a guard 
- multiple collectors: multiple collectors do not co-operate
- can't handle `Rc`/`Arc`: requires all `Gc` objects have straightforward ownership semantics
- non static data: `Gc` cannot handle non 'static data (fix WIP)
- no no-std support: The collector requires threading and other `std` features (fix WIP)
- non-optimal performance: The collector needs to be optimized and parallelized further (fix WIP)

Getting Started
---------------
Here is an easy example, showing how `Gc` works:
```rust
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
    });
    // Everything was cleaned up!
    assert_eq!(number_of_tracked_allocations(), 0);
    assert_eq!(number_of_active_handles(), 0);
}
```

If you're playing with this and run into a problem, go ahead and make a Github issue. Eventually there will be a FAQ.

Help Wanted!
------------
If you're interested in helping with `shredder`, feel free to reach out.
I'm @Others on the Rust discord. Or just look for an issue marked `help wanted
` or `good first issue`
