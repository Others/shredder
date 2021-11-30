use std::cell::RefCell;

use criterion::criterion_group;
use criterion::{black_box, criterion_main, Criterion};
use rand::rngs::StdRng;
use rand::seq::SliceRandom;
use rand::SeedableRng;

use shredder::{collect, number_of_tracked_allocations, run_with_gc_cleanup, Gc, Scan};

// BENCHMARK 1: My janky stress test
// (It basically creates a graph where every node is rooted, then de-roots some nodes a few at a time)
#[derive(Debug, Scan)]
struct DirectedGraphNode {
    label: String,
    edges: Vec<Gc<RefCell<DirectedGraphNode>>>,
}

const NODE_COUNT: usize = 1 << 15;
const EDGE_COUNT: usize = 1 << 15;
const SHRINK_DIV: usize = 1 << 10;

fn stress_test() -> Vec<usize> {
    run_with_gc_cleanup(|| {
        let mut nodes = Vec::new();

        for i in 0..=NODE_COUNT {
            nodes.push(Gc::new(RefCell::new(DirectedGraphNode {
                label: format!("Node {}", i),
                edges: Vec::new(),
            })));
        }

        let mut rng = StdRng::seed_from_u64(0xCAFE);
        for _ in 0..=EDGE_COUNT {
            let a = nodes.choose(&mut rng).unwrap();
            let b = nodes.choose(&mut rng).unwrap();

            a.borrow_mut().edges.push(Gc::clone(b));
        }

        let mut res = Vec::new();
        for i in 0..NODE_COUNT {
            if i % SHRINK_DIV == 0 {
                nodes.truncate(NODE_COUNT - i);
                collect();
                res.push(number_of_tracked_allocations());
            }
        }

        res
    })
}

pub fn benchmark_stress_test(c: &mut Criterion) {
    c.bench_function("stress_test", |b| b.iter(|| black_box(stress_test())));
}

// BENCHMARK 2: It's binary-trees from the benchmarks game!

fn count_binary_trees(max_size: usize) -> Vec<usize> {
    run_with_gc_cleanup(|| {
        let min_size = 4;

        let mut res = Vec::new();

        for depth in (min_size..max_size).step_by(2) {
            let iterations = 1 << (max_size - depth + min_size);
            let mut check = 0;

            for _ in 1..=iterations {
                check += (TreeNode::new(depth)).check();
            }

            res.push(check);
        }

        res
    })
}

// If were feeling idiomatic, we'd use GcDeref here
#[derive(Scan)]
enum TreeNode {
    Nested {
        left: Gc<TreeNode>,
        right: Gc<TreeNode>,
    },
    End,
}

impl TreeNode {
    fn new(depth: usize) -> Self {
        if depth == 0 {
            return Self::End;
        }

        Self::Nested {
            left: Gc::new(TreeNode::new(depth - 1)),
            right: Gc::new(TreeNode::new(depth - 1)),
        }
    }

    fn check(&self) -> usize {
        match self {
            Self::End => 1,
            Self::Nested { left, right } => left.get().check() + right.get().check() + 1,
        }
    }
}

pub fn benchmark_count_binary_trees(c: &mut Criterion) {
    c.bench_function("binary trees", |b| {
        b.iter(|| black_box(count_binary_trees(11)))
    });
}

// TODO: Benchmark with circular references
// TODO: Benchmark with DerefGc
// TODO: Do we want to cleanup in the benchmark?

criterion_group!(benches, benchmark_stress_test, benchmark_count_binary_trees);
criterion_main!(benches);
