//! Derive macros documentation.

/// Utility for deriving empty finalizers.
///
/// See the [`Finalize`][`trait@crate::Finalize`] trait for more information.
///
/// # Example
/// ```rust
///# use rust_cc::*;
///# use rust_cc_derive::*;
/// #[derive(Finalize)]
/// struct Foo {
///     // ...
/// }
/// ```
pub use rust_cc_derive::Finalize;
/// Derive macro for safely deriving [`Trace`][`trait@crate::Trace`] implementations.
///
/// The derived implementation calls the [`trace`][`method@crate::Trace::trace`] method on every field of the implementing type.
///
/// # Ignoring fields
/// The `#[rust_cc(ignore)]` attribute can be used to avoid tracing a field (or variant, in case of an enum).
/// This may be useful, for example, if the field's type doesn't implement [`Trace`][`trait@crate::Trace`], like external library types or some types from std.
///
/// Not tracing a field is *safe*, although it may lead to memory leaks if the ignored field contains any [`Cc`].
///
/// # Automatic `Drop` implementation
/// This macro enforces the [`Drop`]-related safety requirements of [`Trace`][`trait@crate::Trace`] by always emitting an empty [`Drop`]
/// implementation for the implementing type.
///
/// The `#[rust_cc(unsafe_no_drop)]` attribute can be used to suppress the automatic [`Drop`] implementation, allowing to implement a custom one. Using this attribute
/// is considered **unsafe** and **must** respect the safety requirements of [`Trace`][`trait@crate::Trace`].
///
/// Safe alternatives to `#[rust_cc(unsafe_no_drop)]` are [finalizers][`trait@crate::Finalize`] and [cleaners][`crate::cleaners`].
///
/// # Example
/// ```rust
///# use rust_cc::*;
///# use rust_cc_derive::*;
///# #[derive(Finalize)]
/// #[derive(Trace)]
/// struct Foo<A: Trace + 'static, B: Trace + 'static> {
///     a_field: Cc<A>,
///     another_field: Cc<B>,
/// }
/// ```
/// Ignoring a field:
/// ```rust
///# use std::cell::Cell;
///# use rust_cc::*;
///# use rust_cc_derive::*;
///# #[derive(Finalize)]
/// #[derive(Trace)]
/// struct Foo<T: Trace + 'static> {
///     traced_field: Cc<T>,
///     #[rust_cc(ignore)] // Cell doesn't implement Trace, let's ignore it
///     ignored_field: Cell<i32>, // ignored_field doesn't contain any Cc, so there will be no memory leak
/// }
///
///# #[derive(Finalize)]
/// #[derive(Trace)]
/// enum Bar<T: Trace + 'static> {
///     #[rust_cc(ignore)] // Ignores the A variant
///     A {
///         // ...
///     },
///     B(Cc<T>, #[rust_cc(ignore)] Cell<u32>), // Only the Cell is ignored
/// }
/// ```
/// Implementing a custom [`Drop`] implementation:
/// ```rust
///# use rust_cc::*;
///# use rust_cc_derive::*;
///# #[derive(Finalize)]
/// #[derive(Trace)]
/// #[rust_cc(unsafe_no_drop)] // UNSAFE!!!
/// struct Foo {
///     // ...
/// }
///
/// impl Drop for Foo {
///     fn drop(&mut self) {
///         // MUST respect the safety requirements of Trace
///     }
/// }
/// ```
///
/// [`Cc`]: crate::Cc
/// [`Drop`]: core::ops::Drop
pub use rust_cc_derive::Trace;
