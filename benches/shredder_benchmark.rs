//! Benchmarks adapted from the shredder crate, released under MIT license. Src: https://github.com/Others/shredder/blob/266de5a3775567463ee82febc42eed1c9a8b6197/benches/shredder_benchmark.rs

use criterion::criterion_group;
use criterion::{black_box, criterion_main, Criterion};
use rand::rngs::StdRng;
use rand::seq::SliceRandom;
use rand::SeedableRng;
use std::cell::RefMut;

use rust_cc::*;
use crate::TreeNode::Nested;

// BENCHMARK 1: My janky stress test
// (It basically creates a graph where every node is rooted, then de-roots some nodes a few at a time)
struct DirectedGraphNode {
    _label: String,
    edges: Vec<Cc<RefCell<DirectedGraphNode>>>,
}

struct RefCell<T> {
    cell: std::cell::RefCell<T>,
}

impl<T> RefCell<T> {
    fn new(t: T) -> RefCell<T> {
        RefCell {
            cell: std::cell::RefCell::new(t),
        }
    }

    fn borrow_mut(&self) -> RefMut<'_, T> {
        self.cell.borrow_mut()
    }
}

unsafe impl<T: Trace> Trace for RefCell<T> {
    fn trace(&self, ctx: &mut Context<'_>) {
        self.cell.borrow().trace(ctx);
    }
}

impl<T> Finalize for RefCell<T> {}

unsafe impl Trace for DirectedGraphNode {
    fn trace(&self, ctx: &mut Context<'_>) {
        self.edges.iter().for_each(|elem| elem.trace(ctx));
    }
}

impl Finalize for DirectedGraphNode {}

const NODE_COUNT: usize = 1 << 15;
const EDGE_COUNT: usize = 1 << 15;
const SHRINK_DIV: usize = 1 << 10;

fn stress_test() -> Vec<usize> {
    let mut res = Vec::new();
    {
        let mut nodes = Vec::new();

        for i in 0..=NODE_COUNT {
            nodes.push(Cc::new(RefCell::new(DirectedGraphNode {
                _label: format!("Node {}", i),
                edges: Vec::new(),
            })));
        }

        let mut rng = StdRng::seed_from_u64(0xCAFE);
        for _ in 0..=EDGE_COUNT {
            let a = nodes.choose(&mut rng).unwrap();
            let b = nodes.choose(&mut rng).unwrap();

            a.borrow_mut().edges.push(Cc::clone(b));
        }

        for i in 0..NODE_COUNT {
            if i % SHRINK_DIV == 0 {
                nodes.truncate(NODE_COUNT - i);
                collect_cycles();
                res.push(state::allocated_bytes());
            }
        }
    }
    collect_cycles();
    res
}

pub fn benchmark_stress_test(c: &mut Criterion) {
    c.bench_function("stress_test", |b| b.iter(|| black_box(stress_test())));
}

// BENCHMARK 2: It's binary-trees from the benchmarks game!

fn count_binary_trees(max_size: usize) -> Vec<usize> {
    let mut res = Vec::new();
    {
        let min_size = 4;

        for depth in (min_size..max_size).step_by(2) {
            let iterations = 1 << (max_size - depth + min_size);
            let mut check = 0;

            for _ in 1..=iterations {
                check += (TreeNode::new(depth)).check();
            }

            res.push(check);
        }
    }
    collect_cycles();
    res
}

// If were feeling idiomatic, we'd use GcDeref here
enum TreeNode {
    Nested {
        left: Cc<TreeNode>,
        right: Cc<TreeNode>,
    },
    End,
}

unsafe impl Trace for TreeNode {
    fn trace(&self, ctx: &mut Context<'_>) {
        if let Nested { left, right } = self {
            left.trace(ctx);
            right.trace(ctx);
        }
    }
}

impl Finalize for TreeNode {}

impl TreeNode {
    fn new(depth: usize) -> Self {
        if depth == 0 {
            return Self::End;
        }

        Self::Nested {
            left: Cc::new(TreeNode::new(depth - 1)),
            right: Cc::new(TreeNode::new(depth - 1)),
        }
    }

    fn check(&self) -> usize {
        match self {
            Self::End => 1,
            Self::Nested { left, right } => left.check() + right.check() + 1,
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
