# rust-cc
[![Build status main branch](https://img.shields.io/github/check-runs/frengor/rust-cc/main?style=flat&label=main)](https://github.com/frengor/rust-cc/tree/main)
[![Build status dev branch](https://img.shields.io/github/check-runs/frengor/rust-cc/dev?style=flat&label=dev)](https://github.com/frengor/rust-cc/tree/dev)
[![docs.rs](https://img.shields.io/docsrs/rust-cc?style=flat)](https://docs.rs/rust-cc/latest/rust_cc/)
[![Crates.io Version](https://img.shields.io/crates/v/rust-cc?style=flat&color=blue)](https://crates.io/crates/rust-cc)
[![License](https://img.shields.io/crates/l/rust-cc?color=orange)](https://github.com/frengor/rust-cc#license)

A fast garbage collector based on cycle collection for Rust programs.

This crate provides a `Cc` (Cycle Collected) smart pointer, which is basically a `Rc` that automatically detects and 
deallocates reference cycles. If there are no reference cycles, then `Cc` behaves like `Rc` and deallocates 
immediately when the reference counter drops to zero.

Currently, the cycle collector is not concurrent. As such, `Cc` doesn't implement `Send` nor `Sync`.

## Features

* Fully customizable with [Cargo features](https://lib.rs/crates/rust-cc/features)
* Automatic execution of collections
* Finalization
* Weak pointers
* Cleaners
* No-std support (requires ELF TLS due to thread locals)

## Basic usage example

```rust
#[derive(Trace, Finalize)]
struct Data {
    a: Cc<u32>,
    b: RefCell<Option<Cc<Data>>>,
}

// Rc-like API
let my_cc = Cc::new(Data {
    a: Cc::new(42),
    b: RefCell::new(None),
});

let my_cc_2 = my_cc.clone();
let pointed: &Data = &*my_cc_2;
drop(my_cc_2);

// Create a cycle!
*my_cc.b.borrow_mut() = Some(my_cc.clone());

// Here, the allocated Data instance doesn't get immediately deallocated, since there is a cycle.
drop(my_cc);
// We have to call the cycle collector
collect_cycles();
// collect_cycles() is automatically called from time to time when creating new Ccs,
// calling it directly only ensures that a collection is run (like at the end of the program)
```

The derive macro for the `Finalize` trait generates an empty finalizer. To write custom finalizers implement the `Finalize` trait manually:

```rust
impl Finalize for Data {
    fn finalize(&self) {
        // Finalization code called when a Data object is about to be deallocated
        // to allow resource clean up (like closing file descriptors, etc)
    }
}
```

> [!NOTE]  
> Finalization adds an overhead to each collection execution. Cleaners provide a faster alternative to finalization.
>
> *When possible*, it's suggested to prefer cleaners and disable finalization.

For more information read [the docs](https://docs.rs/rust-cc/latest/rust_cc/).

## The collection algorithm

The main idea is to discover the roots (i.e. objects which are surely not garbage) making use of
the information contained in the reference counter, instead of having them from another source.  

Usually, in garbage collected languages, the runtime has always a way to know which objects are roots (knowing the roots allows
the garbage collector to know which objects are still accessible by the program and therefore which can and cannot be deallocated).  
However, since Rust has no runtime, this information isn't available! For this reason, garbage collectors implemented
in Rust for Rust programs have a very difficult time figuring out which objects can be deallocated and which
cannot because they can still be accessed by the program.

rust-cc, using the reference counters, is able to find the roots at runtime while collecting, eliminating the need to
constantly keep track of them. This is also the reason why the standard `RefCell` (instead of a custom one) can be
safely used inside cycle collected objects for interior mutability.

Moreover, the implemented cycle collector should be generally faster than mark-and-sweep garbage collectors on big 
(and fragmented) heaps, since there's no need to trace the whole heap every time it runs.

If you're interested in reading the source code, the algorithm is described more deeply in [CONTRIBUTING.md](./CONTRIBUTING.md#the-collection-algorithm).

## Benchmarks

Benchmarks comparing rust-cc to other collectors can be found at <https://github.com/frengor/rust-cc-benchmarks>.

## License

This project is licensed under either of

* Apache License, Version 2.0, ([LICENSE-APACHE](LICENSE-APACHE) or https://www.apache.org/licenses/LICENSE-2.0)
* MIT license ([LICENSE-MIT](LICENSE-MIT) or https://opensource.org/licenses/MIT)

at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in this crate by you, 
as defined in the Apache-2.0 license, shall be dual licensed as above, without any additional terms or conditions.
