use std::collections::{HashMap, HashSet};
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
use std::sync::Arc;

use crate::List;

pub trait Finalize {
    fn finalize(&mut self);
}

/// Trace fields of structs.
///
/// # Safety
/// TODO
///
/// For the moment, see `rgc::trace::Trace` for safety. The same should apply here (where applicable).
/// Note however that, for `rcc`, a struct which implements [`Drop`] and [`Trace`] is likely to produce UB when traced.
/// The [`Drop`] implementation should **not** mutate the struct in any way and, most importantly, not move out any [`Cc<T>`] from inside.
/// Prefer the [`Finalize`] trait to [`Drop`].
///
/// [`Cc<T>`]: struct@crate::cc::Cc
pub unsafe trait Trace {
    fn trace(&self, ctx: &mut Context<'_>);
}

/// Struct for tracing context.
pub struct Context<'a> {
    inner: ContextInner<'a>,
}

impl<'a> !Send for Context<'a> {}
impl<'a> !Sync for Context<'a> {}

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
    fn trace(&self, _: &mut Context<'_>) {}
}

macro_rules! deref_trace {
    ($generic:ident; $this:ty; $($bound:tt) *) => {
        unsafe impl<$generic: $($bound)*> $crate::trace::Trace for $this
        {
            #[inline]
            fn trace(&self, ctx: &mut $crate::trace::Context<'_>) {
                let deref: &$generic = <$this as ::std::ops::Deref>::deref(self);
                <$generic as $crate::trace::Trace>::trace(deref, ctx);
            }
        }
    }
}

macro_rules! deref_traces {
    ($($this:tt),*,) => {
        $(
            deref_trace!{T; $this<T>; ?::std::marker::Sized + $crate::trace::Trace + 'static}
        )*
    }
}

macro_rules! deref_traces_sized {
    ($($this:tt),*,) => {
        $(
            deref_trace!{T; $this<T>; $crate::trace::Trace + 'static}
        )*
    }
}

deref_traces! {
    Rc,
    Arc,
    Box,
    ManuallyDrop,
}

unsafe impl<T: ?Sized + Trace + 'static> Trace for PhantomData<T> {
    fn trace(&self, _: &mut Context<'_>) {}
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

unsafe impl<R: Trace + 'static, E: Trace + 'static> Trace for Result<R, E> {
    #[inline]
    fn trace(&self, ctx: &mut Context<'_>) {
        match self {
            Ok(ok) => ok.trace(ctx),
            Err(err) => err.trace(ctx),
        }
    }
}

unsafe impl<T: Trace + 'static, const N: usize> Trace for [T; N] {
    #[inline]
    fn trace(&self, ctx: &mut Context<'_>) {
        for it in self {
            it.trace(ctx);
        }
    }
}

unsafe impl<T: Trace + 'static> Trace for [T] {
    #[inline]
    fn trace(&self, ctx: &mut Context<'_>) {
        for it in self {
            it.trace(ctx);
        }
    }
}

unsafe impl<T: Trace + 'static> Trace for Vec<T> {
    #[inline]
    fn trace(&self, ctx: &mut Context<'_>) {
        for it in self {
            it.trace(ctx);
        }
    }
}

unsafe impl<T: Trace + 'static> Trace for HashSet<T> {
    #[inline]
    fn trace(&self, ctx: &mut Context<'_>) {
        for it in self {
            it.trace(ctx);
        }
    }
}

unsafe impl<K: Trace + 'static, V> Trace for HashMap<K, V> {
    #[inline]
    fn trace(&self, ctx: &mut Context<'_>) {
        for it in self.keys() {
            it.trace(ctx);
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
