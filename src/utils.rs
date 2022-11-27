use std::alloc::{alloc, dealloc, handle_alloc_error, Layout};
use std::ptr::NonNull;

use crate::{state, CcOnHeap, Trace};

#[inline]
pub(crate) unsafe fn cc_alloc<T: Trace + 'static>(layout: Layout) -> NonNull<CcOnHeap<T>> {
    state(|state| state.record_allocation(layout));
    match NonNull::new(alloc(layout) as *mut CcOnHeap<T>) {
        Some(ptr) => ptr,
        None => handle_alloc_error(layout),
    }
}

#[inline]
pub(crate) unsafe fn cc_dealloc<T: ?Sized + Trace + 'static>(
    ptr: NonNull<CcOnHeap<T>>,
    layout: Layout,
) {
    state(|state| state.record_deallocation(layout));
    dealloc(ptr.cast().as_ptr(), layout)
}

#[inline(always)]
#[cold]
pub(crate) fn cold() {}

macro_rules! enum_error_impl {
    ($struct:ident, $($error:ident),+) => {
        #[non_exhaustive]
        pub enum $struct {
            $( $error { err: $error }, )+
        }

        $(impl ::std::convert::From<$error> for $struct {
            #[inline]
            fn from(err: $error) -> Self {
                Self::$error { err }
            }
        })+

        impl ::std::fmt::Debug for $struct {
            #[inline]
            fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                match self {
                    $(Self::$error { err } => <$error as ::std::fmt::Debug>::fmt(err, f),)+
                }
            }
        }

        impl ::std::fmt::Display for $struct {
            #[inline]
            fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                match self {
                    $(Self::$error { err } => <$error as ::std::fmt::Display>::fmt(err, f),)+
                }
            }
        }

        impl ::std::error::Error for $struct {
            #[inline]
            fn source(&self) -> ::std::option::Option<&(dyn ::std::error::Error + 'static)> {
                match self {
                    $(Self::$error { err } => ::std::option::Option::Some(err),)+
                }
            }
        }
    };
}

// This makes enum_error_impl macro usable across modules
pub(crate) use enum_error_impl;
