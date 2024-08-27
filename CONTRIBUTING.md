# Contributing to `rust-cc`

Feel free to open issues or pull requests. Feature requests can be done opening an issue, the enhancement tag will be applied by maintainers.

For pull requests, use [rustftm](https://github.com/rust-lang/rustfmt) to format your files and make sure to
[allow edits by maintainers](https://docs.github.com/en/pull-requests/collaborating-with-pull-requests/working-with-forks/allowing-changes-to-a-pull-request-branch-created-from-a-fork).  
Also, remember to open the pull requests toward the `dev` branch. The `main` branch is only for releases!

## The collection algorithm

The core idea behind the algorithm is the same as the one presented by Lins in ["Cyclic Reference Counting With Lazy Mark-Scan"](https://kar.kent.ac.uk/22347/1/CyclicLin.pdf)
and by Bacon and Rajan in ["Concurrent Cycle Collection in Reference Counted Systems"](https://pages.cs.wisc.edu/~cymen/misc/interests/Bacon01Concurrent.pdf).  
However, the implementation differs in order to make the collector faster and more resilient to panics and failures.

> [!IMPORTANT]  
> `rust-cc` is *not* strictly an implementation of the algorithm shown in the linked papers and it's never been
> intended as such. Feel free to propose any kind of improvement!

The following is a summarized version of the collection algorithm:  
When a `Cc` smart pointer is dropped, the reference count (RC) is decreased by 1. If it reaches 0, then the allocated
object pointed by the `Cc` (called `CcBox`) is dropped, otherwise the `Cc` is put into the `POSSIBLE_CYCLES` list.  
The `POSSIBLE_CYCLES` is an (intrusive) list which contains the possible roots of cyclic garbage.  
Sometimes (see [`crate::trigger_collection`](./src/lib.rs)), when creating a new `Cc` or when `collect_cycles` is called,
the objects inside the `POSSIBLE_CYCLES` list are checked to see if they are part of a garbage cycle.

Therefore, they undergo two tracing phases:
- **Trace Counting:** during this phase, a breadth-first traversal of the heap is performed starting from the elements inside
  `POSSIBLE_CYCLES` to count the number of pointers to each `CcBox` that is reachable from the list's `Cc`s.  
  The `tracing_counter` "field" (see the [`counter_marker` module](./src/counter_marker.rs) for more info) is used to keep track of this number.
  <details>
  <summary>About tracing_counter</summary>
  <p>In the papers, Lins, Bacon and Rajan decrement the RC itself instead of using another counter. However, if during tracing there was a panic,
     it would be hard for `rust-cc` to restore the RC correctly. This is the reason for the choice of having another counter.
     The invariant regarding this second counter is that it must always be between 0 and RC (inclusively). 
  </p>
  </details>
- **Trace Roots:** now, every `CcBox` which has the RC strictly greater than `tracing_counter` can be considered a root,
  since it must exist a `Cc` pointer which points at it that hasn't been traced in the previous phase. Thus, a trace is started from these roots,
  and all objects not reached during this trace are finalized/deallocated (this is actually more complex due to possible
  object resurrections, see the comments in [lib.rs](./src/lib.rs)).  
  Note that this second phase is correct only if the graph formed by the pointers is not changed between the two phases. Thus,
  this is a key requirement of the `Trace` trait and one of the reasons it is marked `unsafe`.

## Using lists and queues

When debug assertions are enabled, the `add` method may panic before actually updating the list to contain the added object.
Similarly, the `remove` method may panic after having removed the object from the list.

Thus, marking should be done only:
  - after the call to the `add` method
  - before the call to the `remove` method.

## Writing tests

Every unit test (which makes use of a part of the collector) should start with a call to `tests::reset_state()` to make
sure errors in other tests don't impact the current one.

## Writing documentation

Docs are always built with every feature enabled. This makes it easier to write and maintain the documentation.

However, this also makes it more difficult to write examples, as those must pass CI even when a features they require is disabled.
As such, examples are marked as `ignore`d if a feature they need is missing:
```rust
#![cfg_attr(
    feature = "derive",
    doc = r"```rust"
)]
#![cfg_attr(
    not(feature = "derive"),
    doc = r"```rust,ignore"
)]
#![doc = r"# use rust_cc::*;
# use rust_cc_derive::*;

// Example code

```"]
```
