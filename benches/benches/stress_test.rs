//! Benchmark adapted from the shredder crate, released under MIT license. Src: https://github.com/Others/shredder/blob/266de5a3775567463ee82febc42eed1c9a8b6197/benches/shredder_benchmark.rs

use std::cell::RefCell;

use rand::SeedableRng;
use rand::prelude::IndexedRandom;
use rand::rngs::StdRng;
use rust_cc::*;

// BENCHMARK 1: My janky stress test
// (It basically creates a graph where every node is rooted, then de-roots some nodes a few at a time)
#[derive(Trace, Finalize)]
struct DirectedGraphNode {
    _label: String,
    edges: Vec<Cc<RefCell<DirectedGraphNode>>>,
}

const NODE_COUNT: usize = 1 << 15;
const EDGE_COUNT: usize = 1 << 15;
const SHRINK_DIV: usize = 1 << 10;

pub fn stress_test(seed: u64) -> Vec<usize> {
    let rng = &mut StdRng::seed_from_u64(seed);
    let mut res = Vec::new();
    {
        let mut nodes = Vec::new();

        for i in 0..=NODE_COUNT {
            nodes.push(Cc::new(RefCell::new(DirectedGraphNode {
                _label: format!("Node {}", i),
                edges: Vec::new(),
            })));
        }

        for _ in 0..=EDGE_COUNT {
            let a = nodes.choose(rng).unwrap();
            let b = nodes.choose(rng).unwrap();

            a.borrow_mut().edges.push(Cc::clone(b));
        }

        for i in 0..NODE_COUNT {
            if i % SHRINK_DIV == 0 {
                nodes.truncate(NODE_COUNT - i);
                collect_cycles();
                res.push(nodes.len());
            }
        }
    }
    collect_cycles();
    res
}
