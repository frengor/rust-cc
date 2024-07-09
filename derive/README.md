# rust-cc-derive

Derive macros for the `rust-cc` crate.

## Example Usage

```rust
#[derive(Trace, Finalize)]
struct A<T: Trace + 'static> {
    a: Cc<T>,
    #[rust_cc(ignore)] // The b field won't be traced, safe to use!
    b: i32,
}

#[derive(Trace, Finalize)]
#[rust_cc(unsafe_no_drop)] // Allows to implement Drop for B, unsafe to use! (see Trace docs)
struct B {
    // fields
}
```
