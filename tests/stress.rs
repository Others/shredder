use std::cell::RefCell;

use rand::rngs::StdRng;
use rand::seq::SliceRandom;
use rand::SeedableRng;

use shredder::{collect, number_of_tracked_allocations, Gc, Scan};

#[derive(Debug, Scan)]
struct DirectedGraphNode {
    label: String,
    edges: Vec<Gc<RefCell<DirectedGraphNode>>>,
}

const NODE_COUNT: usize = 1 << 18;
const EDGE_COUNT: usize = 1 << 18;
const SHRINK_DIV: usize = 1 << 13;

#[test]
fn stress_test() {
    println!("Creating nodes...");
    let mut nodes = Vec::new();

    for i in 0..=NODE_COUNT {
        nodes.push(Gc::new(RefCell::new(DirectedGraphNode {
            label: format!("Node {}", i),
            edges: Vec::new(),
        })));
    }

    println!("Adding edges...");
    let mut rng = StdRng::seed_from_u64(0xCAFE);
    for _ in 0..=EDGE_COUNT {
        let a = nodes.choose(&mut rng).unwrap();
        let b = nodes.choose(&mut rng).unwrap();

        a.borrow_mut().edges.push(Gc::clone(b));
    }

    println!("Doing the shrink...");
    for i in 0..NODE_COUNT {
        if i % SHRINK_DIV == 0 {
            nodes.truncate(NODE_COUNT - i);
            collect();
            println!("Now have {} datas", number_of_tracked_allocations());
        }
    }
}
