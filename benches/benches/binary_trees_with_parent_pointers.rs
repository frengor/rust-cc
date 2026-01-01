//! Benchmark adapted from the shredder crate, released under MIT license. Src: https://github.com/Others/shredder/blob/266de5a3775567463ee82febc42eed1c9a8b6197/benches/shredder_benchmark.rs

use std::cell::RefCell;

use rust_cc::*;

// BENCHMARK 3: Same as benchmark 2, but with parent pointers. Added by rust-cc

pub fn count_binary_trees_with_parent(max_size: usize) -> Vec<usize> {
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

#[derive(Trace, Finalize)]
enum TreeNodeWithParent {
    Root {
        left: Cc<TreeNodeWithParent>,
        right: Cc<TreeNodeWithParent>,
    },
    Nested {
        parent: RefCell<Option<Cc<TreeNodeWithParent>>>,
        left: Cc<TreeNodeWithParent>,
        right: Cc<TreeNodeWithParent>,
    },
    End,
}

impl TreeNodeWithParent {
    fn new(depth: usize) -> Cc<Self> {
        if depth == 0 {
            return Cc::new(Self::End);
        }

        let root = Cc::new(Self::Root {
            left: Self::new_nested(depth - 1),
            right: Self::new_nested(depth - 1),
        });

        if let Self::Root { left, right } = &*root {
            if let Self::Nested { parent, .. } = &**left {
                *parent.borrow_mut() = Some(root.clone());
            }

            if let Self::Nested { parent, .. } = &**right {
                *parent.borrow_mut() = Some(root.clone());
            }
        }

        root
    }

    fn new_nested(depth: usize) -> Cc<Self> {
        if depth == 0 {
            return Cc::new(Self::End);
        }

        let cc = Cc::new(Self::Nested {
            left: Self::new_nested(depth - 1),
            right: Self::new_nested(depth - 1),
            parent: RefCell::new(None),
        });

        if let Self::Nested { left, right, .. } = &*cc {
            if let Self::Nested { parent, .. } = &**left {
                *parent.borrow_mut() = Some(cc.clone());
            }

            if let Self::Nested { parent, .. } = &**right {
                *parent.borrow_mut() = Some(cc.clone());
            }
        }

        cc
    }

    fn check(&self) -> usize {
        match self {
            Self::Root { left, right, .. } | Self::Nested { left, right, .. } => {
                left.check() + right.check() + 1
            },
            Self::End => 1,
        }
    }
}
