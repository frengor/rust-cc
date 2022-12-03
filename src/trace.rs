use std::ffi::{CStr, CString, OsStr, OsString};
use std::marker::PhantomData;
use std::mem::{ManuallyDrop, MaybeUninit};
use std::num::{
    NonZeroI128, NonZeroI16, NonZeroI32, NonZeroI64, NonZeroI8, NonZeroIsize, NonZeroU128,
    NonZeroU16, NonZeroU32, NonZeroU64, NonZeroU8, NonZeroUsize,
};
use std::panic::AssertUnwindSafe;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{
    AtomicBool, AtomicI16, AtomicI32, AtomicI64, AtomicI8, AtomicIsize, AtomicU16, AtomicU32,
    AtomicU64, AtomicU8, AtomicUsize,
};

use crate::List;

pub trait Finalize {
    #[inline(always)]
    fn finalize(&mut self) {}
}

/// Trait to trace cycle-collectable objects.
///
/// Implementors should only call the [`trace`] method of every [`Cc`] owned **only** by the implementing struct.
///
/// This trait is already implemented for common types from the standard lib.
///
/// Remember that creating, cloning, or accessing the contents of a [`Cc`] from inside of [`trace`] will produce a panic,
/// since it is run during cycle collection.
///
/// # Safety
/// This trait is unsafe *to implement* because it's not possible to check the following invariants:
///   * The [`trace`] function *should* be called **only once** on every [`Cc`] **owned only** by the implementing struct.
///
///     For example, a [`Cc`] inside a [`Box`] (so a `Box<Cc>`) is owned *only* by the implementing struct.
///     However, a [`Cc`] inside an [`Rc`] (so a `Rc<Cc>`) *isn't* owned *only* by the implementing struct, so it mustn't be traced.
///
///     In general, mixing other shared-ownership smart pointers with [`Cc`]s is not possible and will (almost surely) lead to UB.
///   * If a [`Cc`] is not traced in *any* execution, then it must be skipped in *every* execution. Skipping [`trace`] calls *may*
///     leak memory, but it's better than UB.
///
///     Another possibility is to *panic* instead of skipping. That will halt the collection (potentially leaking memory),
///     but it's safe.
///   * The [`trace`] function *must not* mutate the implementing struct's contents, even if it has [interior mutability].
///   * If the implementing struct implements [`Drop`], then the [`Drop`] implementation *must not* move any [`Cc`].
///     Ignoring this will almost surely produce use-after-free. If you need this feature, implement the [`Finalize`] trait
///     instead of [`Drop`]. Erroneous implementations of [`Drop`] are avoided using the `#[derive(Trace)]` macro,
///     since it always emits an empty [`Drop`] implementation for the implementing struct.
///
/// [interior mutability]: https://doc.rust-lang.org/reference/interior-mutability.html
/// [`self`]: https://doc.rust-lang.org/std/keyword.self.html
/// [`trace`]: crate::Trace::trace
/// [`Finalize`]: crate::Finalize
/// [`Cc`]: crate::Cc
/// [`Drop`]: std::ops::Drop
/// [`Rc`]: std::rc::Rc
/// [`Box`]: std::boxed::Box
/// [`drop`]: std::ops::Drop::drop
pub unsafe trait Trace: Finalize {
    fn trace(&self, ctx: &mut Context<'_>);
}

/// Struct for tracing context.
pub struct Context<'a> {
    inner: ContextInner<'a>,
    _phantom: PhantomData<Rc<()>>, // Make Context !Send and !Sync
}

pub(crate) enum ContextInner<'a> {
    Counting {
        root_list: &'a mut List,
        non_root_list: &'a mut List,
    },
    RootTracing {
        non_root_list: &'a mut List,
    },
    DropTracing,
    DropResurrecting,
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
    Path,
    CStr,
    OsStr,
    String,
    PathBuf,
    OsString,
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

unsafe impl<T> Trace for MaybeUninit<T> {
    /// This does nothing, since memory may be uninit.
    #[inline(always)]
    fn trace(&self, _: &mut Context<'_>) {}
}

impl<T> Finalize for MaybeUninit<T> {
    /// This does nothing, since memory may be uninit.
    #[inline(always)]
    fn finalize(&mut self) {}
}

unsafe impl<T> Trace for PhantomData<T> {
    #[inline(always)]
    fn trace(&self, _: &mut Context<'_>) {}
}

impl<T> Finalize for PhantomData<T> {}

macro_rules! deref_trace {
    ($generic:ident; $this:ty; $($bound:tt)*) => {
        unsafe impl<$generic: $($bound)* $crate::trace::Trace + 'static> $crate::trace::Trace for $this
        {
            #[inline]
            fn trace(&self, ctx: &mut $crate::trace::Context<'_>) {
                let deref: &$generic = <$this as ::std::ops::Deref>::deref(self);
                <$generic as $crate::trace::Trace>::trace(deref, ctx);
            }
        }

        impl<$generic: $($bound)* $crate::trace::Finalize + 'static> $crate::trace::Finalize for $this
        {
            #[inline]
            fn finalize(&mut self) {
                let deref: &mut $generic = <$this as ::std::ops::DerefMut>::deref_mut(self);
                <$generic as $crate::trace::Finalize>::finalize(deref);
            }
        }
    }
}

macro_rules! deref_traces {
    ($($this:tt),*,) => {
        $(
            deref_trace!{T; $this<T>; ?::std::marker::Sized +}
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

unsafe impl<T: Trace + 'static> Trace for Option<T> {
    #[inline]
    fn trace(&self, ctx: &mut Context<'_>) {
        if let Some(inner) = self {
            inner.trace(ctx);
        }
    }
}

impl<T: Finalize + 'static> Finalize for Option<T> {
    #[inline]
    fn finalize(&mut self) {
        if let Some(value) = self {
            value.finalize();
        }
    }
}

unsafe impl<R: Trace + 'static, E: Trace + 'static> Trace for Result<R, E> {
    #[inline]
    fn trace(&self, ctx: &mut Context<'_>) {
        match self {
            Ok(ok) => ok.trace(ctx),
            Err(err) => err.trace(ctx),
        }
    }
}

impl<R: Finalize + 'static, E: Finalize + 'static> Finalize for Result<R, E> {
    #[inline]
    fn finalize(&mut self) {
        match self {
            Ok(value) => value.finalize(),
            Err(err) => err.finalize(),
        }
    }
}

unsafe impl<T: Trace + 'static, const N: usize> Trace for [T; N] {
    #[inline]
    fn trace(&self, ctx: &mut Context<'_>) {
        for elem in self {
            elem.trace(ctx);
        }
    }
}

impl<T: Finalize + 'static, const N: usize> Finalize for [T; N] {
    #[inline]
    fn finalize(&mut self) {
        for elem in self {
            elem.finalize();
        }
    }
}

unsafe impl<T: Trace + 'static> Trace for [T] {
    #[inline]
    fn trace(&self, ctx: &mut Context<'_>) {
        for elem in self {
            elem.trace(ctx);
        }
    }
}

impl<T: Finalize + 'static> Finalize for [T] {
    #[inline]
    fn finalize(&mut self) {
        for elem in self {
            elem.finalize();
        }
    }
}

unsafe impl<T: Trace + 'static> Trace for Vec<T> {
    #[inline]
    fn trace(&self, ctx: &mut Context<'_>) {
        for elem in self {
            elem.trace(ctx);
        }
    }
}

impl<T: Finalize + 'static> Finalize for Vec<T> {
    #[inline]
    fn finalize(&mut self) {
        for elem in self {
            elem.finalize();
        }
    }
}

macro_rules! tuple_finalize_trace {
    ($($args:ident),+) => {
        #[allow(non_snake_case)]
        unsafe impl<$($args),*> $crate::trace::Trace for ($($args,)*)
        where $($args: $crate::trace::Trace + 'static),*
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
        where $($args: $crate::trace::Finalize + 'static),*
        {
            #[inline]
            fn finalize(&mut self) {
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
