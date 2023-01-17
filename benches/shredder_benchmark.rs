//! Benchmarks adapted from the shredder crate, released under MIT license. Src: https://github.com/Others/shredder/blob/266de5a3775567463ee82febc42eed1c9a8b6197/benches/shredder_benchmark.rs

use std::cell::RefCell;
use std::hint::black_box;
use criterion::criterion_group;
use criterion::{criterion_main, Criterion};
use rand::rngs::StdRng;
use rand::seq::SliceRandom;
use rand::SeedableRng;

use rust_cc::*;

// BENCHMARK 1: My janky stress test
// (It basically creates a graph where every node is rooted, then de-roots some nodes a few at a time)
struct DirectedGraphNode {
    _label: String,
    edges: Vec<Cc<RefCell<DirectedGraphNode>>>,
}

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

enum TreeNode {
    Nested {
        left: Cc<TreeNode>,
        right: Cc<TreeNode>,
    },
    End,
}

unsafe impl Trace for TreeNode {
    fn trace(&self, ctx: &mut Context<'_>) {
        if let Self::Nested { left, right } = self {
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
            left: Cc::new(Self::new(depth - 1)),
            right: Cc::new(Self::new(depth - 1)),
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

// BENCHMARK 3: Same as benchmark 2, but with parent pointers. Added by rust-cc

fn count_binary_trees_with_parent(max_size: usize) -> Vec<usize> {
    let mut res = Vec::new();
    {
        let min_size = 4;

        for depth in (min_size..max_size).step_by(2) {
            let iterations = 1 << (max_size - depth + min_size);
            let mut check = 0;

            for _ in 1..=iterations {
                check += (TreeNodeWithParent::new(depth)).check();
            }

            res.push(check);
        }
    }
    collect_cycles();
    res
}

enum TreeNodeWithParent {
    Root {
        left: Cc<TreeNodeWithParent>,
        right: Cc<TreeNodeWithParent>,
    },
    Nested {
        parent: Cc<TreeNodeWithParent>,
        left: Cc<TreeNodeWithParent>,
        right: Cc<TreeNodeWithParent>,
    },
    End,
}

unsafe impl Trace for TreeNodeWithParent {
    fn trace(&self, ctx: &mut Context<'_>) {
        match self {
            Self::Root { left, right } => {
                left.trace(ctx);
                right.trace(ctx);
            }
            Self::Nested { parent, left, right } => {
                parent.trace(ctx);
                left.trace(ctx);
                right.trace(ctx);
            }
            Self::End => {},
        }
    }
}

impl Finalize for TreeNodeWithParent {}

impl TreeNodeWithParent {
    fn new(depth: usize) -> Cc<Self> {
        if depth == 0 {
            return Cc::new(Self::End);
        }

        Cc::<Self>::new_cyclic(|cc| Self::Root {
            left: Self::new_nested(depth - 1, cc.clone()),
            right: Self::new_nested(depth - 1, cc.clone()),
        })
    }

    fn new_nested(depth: usize, parent: Cc<Self>) -> Cc<Self> {
        if depth == 0 {
            return Cc::new(Self::End);
        }

        Cc::<Self>::new_cyclic(|cc| Self::Nested {
            left: Self::new_nested(depth - 1, cc.clone()),
            right: Self::new_nested(depth - 1, cc.clone()),
            parent,
        })
    }

    fn check(&self) -> usize {
        match self {
            Self::Root { left, right, .. }
            | Self::Nested { left, right, .. } => left.check() + right.check() + 1,
            Self::End => 1,
        }
    }
}

pub fn benchmark_count_binary_trees_with_parent(c: &mut Criterion) {
    c.bench_function("binary trees with parent pointers", |b| {
        b.iter(|| black_box(count_binary_trees_with_parent(11)))
    });
}

criterion_group!(benches, benchmark_stress_test, benchmark_count_binary_trees, benchmark_count_binary_trees_with_parent);
criterion_main!(benches);
