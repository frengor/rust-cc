//! Benchmark adapted from the shredder crate, released under MIT license. Src: https://github.com/Others/shredder/blob/266de5a3775567463ee82febc42eed1c9a8b6197/benches/shredder_benchmark.rs

use rust_cc::*;

// BENCHMARK 2: It's binary-trees from the benchmarks game!

pub fn count_binary_trees(max_size: usize) -> Vec<usize> {
    let mut res = Vec::new();
    {
        let min_size = 4;

        for depth in (min_size..max_size).step_by(2) {
            let iterations = 1 << (max_size - depth + min_size);
            let mut check = 0;

            for _ in 1..=iterations {
                check += Cc::new(TreeNode::new(depth)).check();
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
