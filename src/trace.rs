use alloc::boxed::Box;
use alloc::ffi::CString;
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::RefCell;
use core::ffi::CStr;
use core::marker::PhantomData;
use core::mem::ManuallyDrop;
use core::num::{
    NonZeroI8, NonZeroI16, NonZeroI32, NonZeroI64, NonZeroI128, NonZeroIsize, NonZeroU8,
    NonZeroU16, NonZeroU32, NonZeroU64, NonZeroU128, NonZeroUsize,
};
use core::panic::AssertUnwindSafe;
use core::sync::atomic::{
    AtomicBool, AtomicI8, AtomicI16, AtomicI32, AtomicI64, AtomicIsize, AtomicU8, AtomicU16,
    AtomicU32, AtomicU64, AtomicUsize,
};
#[cfg(feature = "std")]
use std::{
    ffi::{OsStr, OsString},
    path::{Path, PathBuf},
};

use crate::lists::{LinkedList, LinkedQueue};

/// Trait to finalize objects before freeing them.
///
/// Must be always implemented for every cycle-collectable object, even when `finalization` is disabled, to avoid cross-crate incompatibilities.
/// When `finalization` is disabled, the [`finalize`] method will *never* be called.
///
/// # Derive macro
///
/// The [`Finalize`][`macro@crate::Finalize`] derive macro can be used to implement an empty finalizer:
#[cfg_attr(feature = "derive", doc = r"```rust")]
#[cfg_attr(not(feature = "derive"), doc = r"```rust,ignore")]
#[doc = r"# use rust_cc::*;
# use rust_cc_derive::*;
#[derive(Finalize)]
struct Foo {
    // ...
}
```"]
///
/// [`finalize`]: Finalize::finalize
pub trait Finalize {
    /// The finalizer, which is called after an object becomes garbage and before [`drop`]ing it.
    ///
    /// By default, objects are finalized only once. Use the method [`Cc::finalize_again`] to make finalization happen again for a certain object.
    /// Also, objects created during the execution of a finalizer are not automatically finalized.
    ///
    /// # Default implementation
    ///
    /// The default implementation is empty.
    ///
    /// [`drop`]: core::ops::Drop::drop
    /// [`Cc::finalize_again`]: crate::Cc::finalize_again
    #[inline(always)]
    fn finalize(&self) {}
}

/// Trait to trace cycle-collectable objects.
///
/// This trait is unsafe to implement, but can be safely derived using the [`Trace`][`macro@crate::Trace`] derive macro, which calls the [`trace`] method on every field:
#[cfg_attr(feature = "derive", doc = r"```rust")]
#[cfg_attr(not(feature = "derive"), doc = r"```rust,ignore")]
#[doc = r"# use rust_cc::*;
# use rust_cc_derive::*;
# #[derive(Finalize)]
#[derive(Trace)]
struct Foo<A: Trace + 'static, B: Trace + 'static> {
    a_field: Cc<A>,
    another_field: Cc<B>,
}
```"]
///
/// This trait is already implemented for common types from the standard library.
///
/// # Safety
/// The implementations of this trait must uphold the following invariants:
///   * The [`trace`] implementation can trace (maximum once) every [`Cc`] instance *exclusively* owned by `self`.
///     No other [`Cc`] instance can be traced.
///   * It's always safe to panic.
///   * During the same tracing phase (see below), two different [`trace`] calls on the same value must *behave the same*, i.e. they must trace the same
///     [`Cc`] instances.  
///     If a panic happens during the second of such [`trace`] calls but not in the first one, then the [`Cc`] instances traced during the second call
///     must be a subset of the [`Cc`] instances traced in the first one.  
///     Tracing can be detected using the [`state::is_tracing`] function. If it never returned `false` between two [`trace`] calls
///     on the same value, then they are part of the same tracing phase.
///   * The [`trace`] implementation must not create, clone, move, dereference or drop any [`Cc`].
///   * If the implementing type implements [`Drop`], then the [`Drop::drop`] implementation must not create, clone, move, dereference, drop or call
///     any method on any [`Cc`] instance.
///
/// # Implementation tips
/// It is almost always preferable to use the derive macro `#[derive(Trace)]`, but in case a manual implementation is needed the following suggestions usually apply:
///   * If a field's type implements [`Trace`], then call its [`trace`] method.
///   * Try to avoid panicking if not strictly necessary, since it may lead to memory leaks.
///   * Avoid mixing [`Cc`]s with other shared-ownership smart pointers like [`Rc`] (a [`Cc`] contained inside an [`Rc`] cannot be traced,
///     since it's not owned *exclusively*).
///   * Never tracing a field is always safe.
///   * If you need to perform any clean up actions, you should do them in the [`Finalize::finalize`] implementation (instead of inside [`Drop::drop`])
///     or using a [cleaner](crate::cleaners).
///
/// # Derive macro compatibility
/// In order to improve the `Trace` derive macro usability and error messages, it is suggested to avoid implementing this trait for references or raw pointers
/// (also considering that no pointed [`Cc`] may be traced, since a reference doesn't own what it refers to).
///
/// [`trace`]: crate::Trace::trace
/// [`state::is_tracing`]: crate::state::is_tracing
/// [`Finalize::finalize`]: crate::Finalize::finalize
/// [`Cc`]: crate::Cc
/// [`Drop`]: core::ops::Drop
/// [`Rc`]: alloc::rc::Rc
/// [`Drop::drop`]: core::ops::Drop::drop
pub unsafe trait Trace: Finalize {
    /// Traces the contained [`Cc`]s. See [`Trace`] for more information.
    ///
    /// [`Cc`]: crate::Cc
    fn trace(&self, ctx: &mut Context<'_>);
}

/// The tracing context provided to every invocation of [`Trace::trace`].
pub struct Context<'a> {
    inner: ContextInner<'a>,
    _phantom: PhantomData<*mut ()>, // Make Context !Send and !Sync
}

pub(crate) enum ContextInner<'a> {
    Counting {
        root_list: &'a mut LinkedList,
        non_root_list: &'a mut LinkedList,
        queue: &'a mut LinkedQueue,
    },
    RootTracing {
        non_root_list: &'a mut LinkedList,
        queue: &'a mut LinkedQueue,
    },
}

impl<'b> Context<'b> {
    #[inline]
    #[must_use]
    pub(crate) const fn new(ctxi: ContextInner) -> Context {
        Context {
            inner: ctxi,
            _phantom: PhantomData,
        }
    }

    #[inline]
    pub(crate) fn inner<'a>(&'a mut self) -> &'a mut ContextInner<'b>
    where
        'b: 'a,
    {
        &mut self.inner
    }
}

// #################################
// #          Trace impls          #
// #################################

macro_rules! empty_trace {
    ($($this:ty),*,) => {
        $(
        unsafe impl $crate::trace::Trace for $this {
            #[inline(always)]
            fn trace(&self, _: &mut $crate::trace::Context<'_>) {}
        }

        impl $crate::trace::Finalize for $this {
        }
        )*
    };
}

empty_trace! {
    (),
    bool,
    isize,
    usize,
    i8,
    u8,
    i16,
    u16,
    i32,
    u32,
    i64,
    u64,
    i128,
    u128,
    f32,
    f64,
    char,
    str,
    CStr,
    String,
    CString,
    NonZeroIsize,
    NonZeroUsize,
    NonZeroI8,
    NonZeroU8,
    NonZeroI16,
    NonZeroU16,
    NonZeroI32,
    NonZeroU32,
    NonZeroI64,
    NonZeroU64,
    NonZeroI128,
    NonZeroU128,
    AtomicBool,
    AtomicIsize,
    AtomicUsize,
    AtomicI8,
    AtomicU8,
    AtomicI16,
    AtomicU16,
    AtomicI32,
    AtomicU32,
    AtomicI64,
    AtomicU64,
}

#[cfg(feature = "std")]
empty_trace! {
    Path,
    OsStr,
    PathBuf,
    OsString,
}

// Removed since these impls are error-prone. Making a Cc<MaybeUninit<T>> and then casting it to Cc<T>
// doesn't make T traced during tracing, since the impls for MaybeUninit are empty and the vtable is saved when calling Cc::new
/*unsafe impl<T> Trace for MaybeUninit<T> {
    /// This does nothing, since memory may be uninit.
    #[inline(always)]
    fn trace(&self, _: &mut Context<'_>) {}
}

impl<T> Finalize for MaybeUninit<T> {
    /// This does nothing, since memory may be uninit.
    #[inline(always)]
    fn finalize(&self) {}
}*/

unsafe impl<T: ?Sized> Trace for PhantomData<T> {
    #[inline(always)]
    fn trace(&self, _: &mut Context<'_>) {}
}

impl<T: ?Sized> Finalize for PhantomData<T> {}

macro_rules! deref_trace {
    ($generic:ident; $this:ty; $($bound:tt)*) => {
        unsafe impl<$generic: $($bound)* $crate::trace::Trace> $crate::trace::Trace for $this
        {
            #[inline]
            fn trace(&self, ctx: &mut $crate::trace::Context<'_>) {
                let deref: &$generic = <$this as ::core::ops::Deref>::deref(self);
                <$generic as $crate::trace::Trace>::trace(deref, ctx);
            }
        }

        impl<$generic: $($bound)* $crate::trace::Finalize> $crate::trace::Finalize for $this
        {
            #[inline]
            fn finalize(&self) {
                let deref: &$generic = <$this as ::core::ops::Deref>::deref(self);
                <$generic as $crate::trace::Finalize>::finalize(deref);
            }
        }
    }
}

macro_rules! deref_traces {
    ($($this:tt),*,) => {
        $(
            deref_trace!{T; $this<T>; ?::core::marker::Sized +}
        )*
    }
}

macro_rules! deref_traces_sized {
    ($($this:tt),*,) => {
        $(
            deref_trace!{T; $this<T>; }
        )*
    }
}

deref_traces! {
    Box,
    ManuallyDrop,
}

deref_traces_sized! {
    AssertUnwindSafe,
}

unsafe impl<T: ?Sized + Trace> Trace for RefCell<T> {
    #[inline]
    fn trace(&self, ctx: &mut Context<'_>) {
        if let Ok(borrow) = self.try_borrow_mut() {
            borrow.trace(ctx);
        }
    }
}

impl<T: ?Sized + Finalize> Finalize for RefCell<T> {
    #[inline]
    fn finalize(&self) {
        if let Ok(borrow) = self.try_borrow() {
            borrow.finalize();
        }
    }
}

unsafe impl<T: Trace> Trace for Option<T> {
    #[inline]
    fn trace(&self, ctx: &mut Context<'_>) {
        if let Some(inner) = self {
            inner.trace(ctx);
        }
    }
}

impl<T: Finalize> Finalize for Option<T> {
    #[inline]
    fn finalize(&self) {
        if let Some(value) = self {
            value.finalize();
        }
    }
}

unsafe impl<R: Trace, E: Trace> Trace for Result<R, E> {
    #[inline]
    fn trace(&self, ctx: &mut Context<'_>) {
        match self {
            Ok(ok) => ok.trace(ctx),
            Err(err) => err.trace(ctx),
        }
    }
}

impl<R: Finalize, E: Finalize> Finalize for Result<R, E> {
    #[inline]
    fn finalize(&self) {
        match self {
            Ok(value) => value.finalize(),
            Err(err) => err.finalize(),
        }
    }
}

unsafe impl<T: Trace, const N: usize> Trace for [T; N] {
    #[inline]
    fn trace(&self, ctx: &mut Context<'_>) {
        for elem in self {
            elem.trace(ctx);
        }
    }
}

impl<T: Finalize, const N: usize> Finalize for [T; N] {
    #[inline]
    fn finalize(&self) {
        for elem in self {
            elem.finalize();
        }
    }
}

unsafe impl<T: Trace> Trace for [T] {
    #[inline]
    fn trace(&self, ctx: &mut Context<'_>) {
        for elem in self {
            elem.trace(ctx);
        }
    }
}

impl<T: Finalize> Finalize for [T] {
    #[inline]
    fn finalize(&self) {
        for elem in self {
            elem.finalize();
        }
    }
}

unsafe impl<T: Trace> Trace for Vec<T> {
    #[inline]
    fn trace(&self, ctx: &mut Context<'_>) {
        for elem in self {
            elem.trace(ctx);
        }
    }
}

impl<T: Finalize> Finalize for Vec<T> {
    #[inline]
    fn finalize(&self) {
        for elem in self {
            elem.finalize();
        }
    }
}

macro_rules! tuple_finalize_trace {
    ($($args:ident),+) => {
        #[allow(non_snake_case)]
        unsafe impl<$($args),*> $crate::trace::Trace for ($($args,)*)
        where $($args: $crate::trace::Trace),*
        {
            #[inline]
            fn trace(&self, ctx: &mut $crate::trace::Context<'_>) {
                match self {
                    ($($args,)*) => {
                        $(
                            <$args as $crate::trace::Trace>::trace($args, ctx);
                        )*
                    }
                }
            }
        }

        #[allow(non_snake_case)]
        impl<$($args),*> $crate::trace::Finalize for ($($args,)*)
        where $($args: $crate::trace::Finalize),*
        {
            #[inline]
            fn finalize(&self) {
                match self {
                    ($($args,)*) => {
                        $(
                            <$args as $crate::trace::Finalize>::finalize($args);
                        )*
                    }
                }
            }
        }
    }
}

macro_rules! tuple_finalize_traces {
    ($(($($args:ident),+);)*) => {
        $(
            tuple_finalize_trace!($($args),*);
        )*
    }
}

tuple_finalize_traces! {
    (A);
    (A, B);
    (A, B, C);
    (A, B, C, D);
    (A, B, C, D, E);
    (A, B, C, D, E, F);
    (A, B, C, D, E, F, G);
    (A, B, C, D, E, F, G, H);
    (A, B, C, D, E, F, G, H, I);
    (A, B, C, D, E, F, G, H, I, J);
    (A, B, C, D, E, F, G, H, I, J, K);
    (A, B, C, D, E, F, G, H, I, J, K, L);
}
