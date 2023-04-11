//! Benchmark adapted from the shredder crate, released under MIT license. Src: https://github.com/Others/shredder/blob/266de5a3775567463ee82febc42eed1c9a8b6197/benches/shredder_benchmark.rs

use std::hint::black_box;

use rust_cc::*;

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

pub fn benchmark_count_binary_trees_with_parent() {
    count_binary_trees_with_parent(black_box(11));
}
