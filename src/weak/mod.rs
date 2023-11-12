use std::ops::Deref;
use std::ptr;
use std::ptr::NonNull;
#[cfg(feature = "nightly")]
use std::{
    marker::Unsize,
    ops::CoerceUnsized,
};

use crate::cc::CcOnHeap;
use crate::state::state;
use crate::{Cc, Context, Finalize, Trace};
use crate::utils::{alloc_other, dealloc_other};
use crate::weak::weak_metadata::WeakMetadata;

mod weak_metadata;

pub type WeakableCc<T> = Cc<Weakable<T>>;

pub struct Weak<T: ?Sized + Trace + 'static> {
    metadata: NonNull<WeakMetadata>,
    cc: NonNull<CcOnHeap<Weakable<T>>>,
}

#[cfg(feature = "nightly")]
impl<T, U> CoerceUnsized<Weak<U>> for Weak<T>
    where
    T: ?Sized + Trace + Unsize<U> + 'static,
    U: ?Sized + Trace + 'static,
{
}

impl<T: ?Sized + Trace + 'static> Weak<T> {
    #[inline]
    #[must_use = "newly created Cc is immediately dropped"]
    #[track_caller]
    pub fn upgrade(&self) -> Option<Cc<Weakable<T>>> {
        #[cfg(debug_assertions)]
        if state(|state| state.is_tracing()) {
            panic!("Cannot upgrade while tracing!");
        }

        if self.strong_count() == 0 {
            None
        } else {
            // SAFETY: cc is accessible
            if unsafe { self.cc.as_ref() }.counter_marker().increment_counter().is_err() {
                panic!("Too many references has been created to a single Cc");
            }

            let upgraded = Cc::__new_internal(self.cc);
            upgraded.mark_alive();
            Some(upgraded)
        }
    }

    #[inline]
    pub fn ptr_eq(this: &Weak<T>, other: &Weak<T>) -> bool {
        ptr::eq(this.metadata.as_ptr() as *const (), other.metadata.as_ptr() as *const ())
    }

    #[inline]
    pub fn strong_count(&self) -> u32 {
        if self.metadata().is_accessible() {
            // SAFETY: self.cc is still allocated and can be dereferenced
            unsafe { self.cc.as_ref() }.counter_marker().counter()
        } else {
            0
        }
    }

    #[inline]
    pub fn weak_count(&self) -> u32 {
        // This function returns an u32 although internally the weak counter is an u16 to have more flexibility for future expansions
        self.metadata().counter() as u32
    }

    #[inline(always)]
    fn metadata(&self) -> &WeakMetadata {
        unsafe { self.metadata.as_ref() }
    }
}

impl<T: ?Sized + Trace + 'static> Clone for Weak<T> {
    #[inline]
    #[track_caller]
    fn clone(&self) -> Self {
        #[cfg(debug_assertions)]
        if state(|state| state.is_tracing()) {
            panic!("Cannot clone while tracing!");
        }

        if self.metadata().increment_counter().is_err() {
            panic!("Too many references has been created to a single Weak");
        }

        Weak {
            metadata: self.metadata,
            cc: self.cc,
        }
    }
}

impl<T: ?Sized + Trace + 'static> Drop for Weak<T> {
    #[inline]
    fn drop(&mut self) {
        // Always decrement the weak counter
        let res = self.metadata().decrement_counter();
        debug_assert!(res.is_ok());

        if self.metadata().counter() == 0 && !self.metadata().is_accessible() {
            // No weak pointer is left and the CcOnHeap has been deallocated, so just deallocate the metadata
            unsafe {
                dealloc_other(self.metadata);
            }
        }
    }
}

unsafe impl<T: ?Sized + Trace + 'static> Trace for Weak<T> {
    #[inline(always)]
    fn trace(&self, _: &mut Context<'_>) {
        // Do not trace anything here, otherwise it wouldn't be a weak pointer
    }
}

impl<T: ?Sized + Trace + 'static> Finalize for Weak<T> {
}

pub struct Weakable<T: ?Sized + Trace + 'static> {
    metadata: NonNull<WeakMetadata>,
    elem: T,
}

impl<T: Trace + 'static> Weakable<T> {
    #[inline(always)]
    #[must_use = "newly created Weakable is immediately dropped"]
    pub fn new(t: T) -> Weakable<T> {
        unsafe {
            let metadata: NonNull<WeakMetadata> = alloc_other();
            ptr::write(
                metadata.as_ptr(),
                WeakMetadata::new(true),
            );
            Weakable {
                metadata,
                elem: t,
            }
        }
    }
}

impl<T: ?Sized + Trace + 'static> Weakable<T> {
    #[inline(always)]
    fn metadata(&self) -> &WeakMetadata {
        unsafe { self.metadata.as_ref() }
    }
}

impl<T: Trace + 'static> Cc<Weakable<T>> {
    #[inline(always)]
    #[must_use = "newly created Cc is immediately dropped"]
    pub fn new_weakable(t: T) -> Self {
        Cc::new(Weakable::new(t))
    }
}

impl<T: ?Sized + Trace + 'static> Cc<Weakable<T>> {
    #[inline]
    #[must_use = "newly created Weak is immediately dropped"]
    #[track_caller]
    pub fn downgrade(&self) -> Weak<T> {
        #[cfg(debug_assertions)]
        if state(|state| state.is_tracing()) {
            panic!("Cannot downgrade while tracing!");
        }

        if self.metadata().increment_counter().is_err() {
            panic!("Too many references has been created to a single Weak");
        }

        self.mark_alive();

        Weak {
            metadata: self.metadata,
            cc: NonNull::from(self.inner()),
        }
    }

    #[inline]
    pub fn weak_count(&self) -> u32 {
        // This function returns an u32 although internally the weak counter is an u16 to have more flexibility for future expansions
        self.metadata().counter() as u32
    }

    #[inline(always)]
    fn metadata(&self) -> &WeakMetadata {
        unsafe { self.metadata.as_ref() }
    }
}

impl<T: ?Sized + Trace + 'static> Deref for Weakable<T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.elem
    }
}

impl<T: ?Sized + Trace + 'static> Drop for Weakable<T> {
    #[inline]
    fn drop(&mut self) {
        unsafe {
            if self.metadata().counter() == 0 {
                // There are no weak pointers, deallocate the metadata
                dealloc_other(self.metadata);
            } else {
                // There exist weak pointers, set the CcOnHeap allocation not accessible
                self.metadata().set_accessible(false);
            }
        }
    }
}

unsafe impl<T: ?Sized + Trace + 'static> Trace for Weakable<T> {
    #[inline]
    fn trace(&self, ctx: &mut Context<'_>) {
        self.elem.trace(ctx);
    }
}

impl<T: ?Sized + Trace + 'static> Finalize for Weakable<T> {
    #[inline]
    fn finalize(&self) {
        // Weakable is intended to be a transparent wrapper, so just call finalize on elem
        self.elem.finalize();
    }
}
