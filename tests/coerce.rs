#![cfg(feature = "nightly-features")]

use shredder::*;
use std::cell::RefCell;

type GcNode = Gc<RefCell<dyn LinkedListNode + 'static>>;

struct LinkedListBuilder {
    head: GcNode,
    tail: Option<GcNode>,
}

impl LinkedListBuilder {
    fn new(head: GcNode) -> Self {
        let tail = head.get().borrow().tail();
        Self { head, tail }
    }

    fn next<T: LinkedListNode + 'static>(mut self, next: T) -> Self {
        let next = Gc::new(RefCell::new(next));
        let tail = match self.tail {
            Some(tail) => tail.clone(),
            None => self.head.clone(),
        };
        tail.get().borrow_mut().set_next(Some(next.clone()));
        self.tail = Some(next);
        self
    }

    fn finish(self) -> GcNode {
        self.head
    }
}

impl<T: LinkedListNode + 'static> From<T> for LinkedListBuilder {
    fn from(other: T) -> Self {
        Self::new(Gc::new(RefCell::new(other)))
    }
}

trait LinkedListNode: Scan {
    fn label(&self) -> &str;
    fn next(&self) -> Option<GcNode>;
    fn set_next(&mut self, next: Option<GcNode>);

    fn tail(&self) -> Option<GcNode> {
        match self.next() {
            Some(next) => {
                if next.get().borrow().next().is_some() {
                    next.get().borrow().tail()
                } else {
                    self.next()
                }
            }
            None => None,
        }
    }
}

macro_rules! make_node {
    ($name:ident, $label:expr) => {
        #[derive(Default, Debug, Scan)]
        struct $name {
            next: Option<GcNode>,
        }

        impl LinkedListNode for $name {
            fn label(&self) -> &str {
                $label
            }
            fn next(&self) -> Option<GcNode> {
                self.next.clone()
            }
            fn set_next(&mut self, next: Option<GcNode>) {
                self.next = next;
            }
        }
    };
}

make_node!(BlueNode, "blue");
make_node!(RedNode, "red");
make_node!(GreenNode, "green");
make_node!(BlackNode, "black");

#[test]
fn coerce_nodes() {
    run_with_gc_cleanup(|| {
        assert_eq!(number_of_tracked_allocations(), 0);

        let head = LinkedListBuilder::from(BlueNode::default())
            .next(RedNode::default())
            .next(BlueNode::default())
            .next(GreenNode::default())
            .next(BlackNode::default())
            .next(BlackNode::default())
            .next(GreenNode::default())
            .next(BlueNode::default())
            .next(RedNode::default())
            .next(BlueNode::default())
            .finish();

        assert_eq!(number_of_tracked_allocations(), 10);
        collect();
        assert_eq!(number_of_tracked_allocations(), 10);

        let mut node = Some(head.clone());

        const EXPECTED: &[&str] = &[
            "blue", "red", "blue", "green", "black", "black", "green", "blue", "red", "blue",
        ];
        let mut i = 0;

        while let Some(node_ptr) = node {
            let gc_node_ref = node_ptr.get();
            let node_ref = gc_node_ref.borrow();
            assert_eq!(node_ref.label(), EXPECTED[i], "at index {}", i);
            node = node_ref.next();
            i += 1;
        }

        // still holding on to all references
        collect();
        assert_eq!(number_of_tracked_allocations(), 10);
    });
}
