use alloc::alloc::{Layout, alloc, dealloc, handle_alloc_error};
use core::ptr::NonNull;

use crate::counter_marker::Mark;
use crate::state::State;
use crate::{CcBox, Trace};

#[inline]
pub(crate) unsafe fn cc_alloc<T: Trace + 'static>(
    layout: Layout,
    state: &State,
) -> NonNull<CcBox<T>> {
    state.record_allocation(layout);
    match NonNull::new(unsafe { alloc(layout) } as *mut CcBox<T>) {
        Some(ptr) => ptr,
        None => handle_alloc_error(layout),
    }
}

#[inline]
pub(crate) unsafe fn cc_dealloc<T: ?Sized + Trace + 'static>(
    ptr: NonNull<CcBox<T>>,
    layout: Layout,
    state: &State,
) {
    state.record_deallocation(layout);
    unsafe { dealloc(ptr.cast().as_ptr(), layout) };
}

#[cfg(any(feature = "weak-ptrs", feature = "cleaners"))]
#[inline]
pub(crate) unsafe fn alloc_other<T>() -> NonNull<T> {
    let layout = Layout::new::<T>();
    match NonNull::new(unsafe { alloc(layout) } as *mut T) {
        Some(ptr) => ptr,
        None => handle_alloc_error(layout),
    }
}

#[cfg(any(feature = "weak-ptrs", feature = "cleaners"))]
#[inline]
pub(crate) unsafe fn dealloc_other<T>(ptr: NonNull<T>) {
    let layout = Layout::new::<T>();
    unsafe { dealloc(ptr.cast().as_ptr(), layout) };
}

#[inline(always)]
#[cold]
pub(crate) fn cold() {}

pub(crate) struct ResetMarkDropGuard {
    ptr: NonNull<CcBox<()>>,
}

impl ResetMarkDropGuard {
    #[inline]
    pub(crate) fn new(ptr: NonNull<CcBox<()>>) -> Self {
        Self { ptr }
    }
}

impl Drop for ResetMarkDropGuard {
    #[inline]
    fn drop(&mut self) {
        unsafe { self.ptr.as_ref() }.counter_marker().mark(Mark::NonMarked);
    }
}

#[cfg(feature = "std")]
pub(crate) use std::thread_local as rust_cc_thread_local; // Use the std's macro when std is enabled

#[cfg(not(feature = "std"))]
macro_rules! rust_cc_thread_local {
    // Same cases as std's thread_local macro

    () => {};

    ($(#[$attr:meta])* $vis:vis static $name:ident: $t:ty = const { $init:expr }; $($rest:tt)*) => (
        $crate::utils::rust_cc_thread_local!($(#[$attr])* $vis static $name: $t = const { $init });
        $crate::utils::rust_cc_thread_local!($($rest)*);
    );

    ($(#[$attr:meta])* $vis:vis static $name:ident: $t:ty = const { $init:expr }) => (
        $crate::utils::rust_cc_thread_local!($(#[$attr])* $vis static $name: $t = $init);
    );

    ($(#[$attr:meta])* $vis:vis static $name:ident: $t:ty = $init:expr; $($rest:tt)*) => (
        $crate::utils::rust_cc_thread_local!($(#[$attr])* $vis static $name: $t = $init);
        $crate::utils::rust_cc_thread_local!($($rest)*);
    );

    ($(#[$attr:meta])* $vis:vis static $name:ident: $t:ty = $init:expr) => (
        #[allow(clippy::declare_interior_mutable_const)]
        const INIT: $t = $init;

        #[thread_local]
        $(#[$attr])* $vis static $name: $crate::utils::NoStdLocalKey<$t> = $crate::utils::NoStdLocalKey::new(INIT);
    );
}

#[cfg(not(feature = "std"))]
pub(crate) use {
    no_std_thread_locals::*,
    rust_cc_thread_local, // When std is not enabled, use the custom macro which uses the #[thread_local] attribute
};

#[cfg(not(feature = "std"))]
#[allow(dead_code)]
mod no_std_thread_locals {
    // Implementation of LocalKey for no-std to allow to easily switch from the std's thread_local macro to rust_cc_thread_local

    #[non_exhaustive]
    #[derive(Clone, Copy, Eq, PartialEq, Debug)]
    pub(crate) struct AccessError;

    pub(crate) struct NoStdLocalKey<T: 'static> {
        value: T,
    }

    impl<T> NoStdLocalKey<T> {
        #[inline(always)]
        pub(crate) const fn new(value: T) -> Self {
            NoStdLocalKey { value }
        }

        #[inline]
        pub(crate) fn with<F, R>(&self, f: F) -> R
        where
            F: FnOnce(&T) -> R,
        {
            f(&self.value)
        }

        #[inline]
        pub(crate) fn try_with<F, R>(&self, f: F) -> Result<R, AccessError>
        where
            F: FnOnce(&T) -> R,
        {
            Ok(f(&self.value))
        }
    }
}

#[cfg(all(test, not(feature = "std")))]
mod no_std_tests {
    use core::cell::Cell;

    use super::no_std_thread_locals::NoStdLocalKey;

    rust_cc_thread_local! {
        static VAL: Cell<i32> = Cell::new(3);
    }

    #[test]
    fn check_type() {
        // Make sure we're using the right macro
        fn a(_: &NoStdLocalKey<Cell<i32>>) {}
        a(&VAL);
    }

    #[test]
    fn test_with() {
        let i = VAL.with(|i| i.get());
        assert_eq!(3, i);
        let i = VAL.with(|i| {
            i.set(i.get() + 1);
            i.get()
        });
        assert_eq!(4, i);
    }

    #[test]
    fn test_try_with() {
        let i = VAL.try_with(|i| i.get()).unwrap();
        assert_eq!(3, i);
        let i = VAL
            .try_with(|i| {
                i.set(i.get() + 1);
                i.get()
            })
            .unwrap();
        assert_eq!(4, i);
    }

    #[test]
    fn test_with_nested() {
        let i = VAL.with(|i| {
            i.set(VAL.with(|ii| ii.get()) + 1);
            i.get()
        });
        assert_eq!(4, i);
    }

    #[test]
    fn test_try_with_nested() {
        let i = VAL
            .try_with(|i| {
                i.set(VAL.try_with(|ii| ii.get()).unwrap() + 1);
                i.get()
            })
            .unwrap();
        assert_eq!(4, i);
    }
}
