///! Same benchmarks of [rust-cc-benchmarks](https://github.com/frengor/rust-cc-benchmarks), but run with [iai](https://github.com/bheisler/iai).

mod benches {
    pub(super) mod stress_test;
    pub(super) mod binary_trees;
    pub(super) mod binary_trees_with_parent_pointers;
    pub(super) mod large_linked_list;
}

use benches::stress_test::benchmark_stress_test;
use benches::binary_trees::benchmark_count_binary_trees;
use benches::binary_trees_with_parent_pointers::benchmark_count_binary_trees_with_parent;
use benches::large_linked_list::benchmark_large_linked_list;

iai::main!(benchmark_stress_test, benchmark_count_binary_trees, benchmark_count_binary_trees_with_parent, benchmark_large_linked_list);
