//! Same benchmarks of [rust-cc-benchmarks](https://github.com/frengor/rust-cc-benchmarks), but run with [iai-callgrind](https://github.com/iai-callgrind/iai-callgrind).

mod benches {
    pub(super) mod binary_trees;
    pub(super) mod binary_trees_with_parent_pointers;
    pub(super) mod large_linked_list;
    pub(super) mod stress_test;
}

use std::hint::black_box;

use iai_callgrind::{LibraryBenchmarkConfig, library_benchmark, library_benchmark_group, main};

use crate::benches::binary_trees::count_binary_trees;
use crate::benches::binary_trees_with_parent_pointers::count_binary_trees_with_parent;
use crate::benches::large_linked_list::large_linked_list;
use crate::benches::stress_test::stress_test;

#[library_benchmark]
#[bench::seed(0xCAFE)]
fn stress_test_bench(seed: u64) -> Vec<usize> {
    black_box(stress_test(seed))
}

#[library_benchmark]
#[bench::depth(11)]
fn count_binary_trees_bench(depth: usize) -> Vec<usize> {
    black_box(count_binary_trees(depth))
}

#[library_benchmark]
#[bench::depth(11)]
fn count_binary_trees_with_parent_bench(depth: usize) -> Vec<usize> {
    black_box(count_binary_trees_with_parent(depth))
}

#[library_benchmark]
#[bench::size(4096)]
fn large_linked_list_bench(size: usize) -> Vec<usize> {
    black_box(large_linked_list(size))
}

library_benchmark_group!(
    name = stress_tests_group;
    benchmarks = stress_test_bench
);

library_benchmark_group!(
    name = binary_trees_group;
    benchmarks = count_binary_trees_bench, count_binary_trees_with_parent_bench
);

library_benchmark_group!(
    name = linked_lists_group;
    benchmarks = large_linked_list_bench
);

main!(
    config = LibraryBenchmarkConfig::default().valgrind_args(["--branch-sim=yes"]);
    library_benchmark_groups = stress_tests_group, binary_trees_group, linked_lists_group
);
