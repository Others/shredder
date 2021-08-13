 shredder
 ========
[![Version](https://img.shields.io/crates/v/shredder)](https://crates.io/crates/shredder)
[![Docs](https://docs.rs/shredder/badge.svg)](https://docs.rs/shredder)
[![License](https://img.shields.io/github/license/Others/shredder)](https://github.com/Others/shredder/blob/master/LICENSE.md)
[![CircleCI](https://img.shields.io/circleci/build/github/Others/shredder)](https://app.circleci.com/pipelines/github/Others/shredder)
[![Coverage Status](https://coveralls.io/repos/github/Others/shredder/badge.svg?branch=master)](https://coveralls.io/github/Others/shredder?branch=master)
[![Dependencies](https://img.shields.io/librariesio/github/Others/shredder)](https://libraries.io/github/Others/shredder)

`shredder` is a library providing a garbage collected smart pointer: `Gc`.
This is useful for times when you want shared access to some data, but the structure
of the data has unpredictable cycles in it. (So Arc would not be appropriate.)

`shredder` has the following features:
- safe: detects error conditions on the fly, and protects you from undefined behavior
- ergonomic: no need to manually manage roots, just a regular smart pointer
- deref support: `DerefGc` gives you a garbage collected and `Deref` smart pointer where possible
- ready for fearless concurrency: works in multi-threaded contexts, with `AtomicGc` for cases where you need atomic operations
- limited stop-the world: regular processing will rarely be interrupted
- seamless destruction: regular `drop` for `'static` data
- clean finalization: optional `finalize` for non-`'static` data
- concurrent collection: collection happens in the background, improving performance
- concurrent destruction: destructors are run in the background, improving performance

`shredder` has the following limitations:
- guarded access: accessing `Gc` data requires acquiring a guard (although you can use `DerefGc` in many cases to avoid this)
- multiple collectors: only a single global collector is supported
- can't handle `Rc`/`Arc`: requires all `Gc` objects have straightforward ownership semantics
- collection optimized for speed, not memory use: `Gc` and internal metadata is small, but there is bloat during collection (will fix!)
- no no-std support: The collector requires threading and other `std` features (will fix!)

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

fn main() {
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
        b.borrow_mut().directed_edges.push(a);
        // We now have cyclical data!
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
