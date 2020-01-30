shredder
========
`shredder` is a library providing a garbage collected smart pointer: `Gc`.
This is useful for times where you want shared access to some data, but the structure
of the data has unpredictable cycles in it. (So Arc would not be appropriate.)

`shredder` has the following features:
- fairly ergonomic: no need to manually manage roots, just a regular smart pointer
- destructors: no need for finalization, your destructors are seamlessly run
- ready for fearless concurrency: works in multi-threaded contexts
- safe: detects error conditions on the fly, and protects you from common mistakes
- limited stop-the world: regular processing will rarely be interrupted
- concurrent collection: collection and destruction happens in the background

`shredder` has the following limitations:
- non-sync ergonomics: `Send` objects need a guard object
- multiple collectors: multiple collectors do not co-operate
- can't handle `Rc`/`Arc`: requires all `Gc` objects have straightforward ownership semantics
- non static data: `Gc` cannot handle non 'static data (fix WIP)
- `Gc<RefCell<_>>` is a bit awkward to work with, and this is a common case: need to `get` then `borrow` (fix WIP)

Getting Started
---------------
Here is an easy example, showing how `Gc` works:
```rust
use std::cell::RefCell;

use shredder::{collect, number_of_tracked_allocations, Gc, Scan};

#[derive(Scan)]
struct Node {
    data: String,
    directed_edges: Vec<Gc<RefCell<Node>>>,
}

fn main() {
    {
        let a = Gc::new(RefCell::new(Node {
            data: "A".to_string(),
            directed_edges: Vec::new(),
        }));

        let b = Gc::new(RefCell::new(Node {
            data: "B".to_string(),
            directed_edges: Vec::new(),
        }));
        // Needs `get`, since `RefCell` is not `Sync`
        // If we used `Mutex`, we would not need `get`
        a.get().borrow_mut().directed_edges.push(b.clone());
        b.get().borrow_mut().directed_edges.push(a.clone());
    }

    // Running `collect` like this, at the end of main (after everything is dropped) is good practice
    // It helps ensure destructors are run
    collect();
    assert_eq!(number_of_tracked_allocations(), 0);
}
```

If you're playing with this and run into a problem, go ahead and make a Github issue. Eventually there will be a FAQ.

Help Wanted!
------------
If you're interested in helping with `shredder`, feel free to reach out.
I'm @Others on the Rust discord. Or just look for an issue marked `help wanted
` or `good first issue`
