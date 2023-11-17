use core::ops::Deref;
use core::{mem, ptr};
use core::ptr::{drop_in_place, NonNull};
#[cfg(feature = "nightly")]
use core::{
    marker::Unsize,
    ops::CoerceUnsized,
};
use core::cell::Cell;
use core::mem::MaybeUninit;

use crate::cc::CcOnHeap;
use crate::state::state;
use crate::{Cc, Context, Finalize, Trace};
use crate::utils::{alloc_other, cc_dealloc, dealloc_other};
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
    metadata: Cell<Option<NonNull<WeakMetadata>>>, // the Option is used to avoid allocating until a Weak is created
    elem: T,
}

impl<T: Trace + 'static> Weakable<T> {
    #[inline(always)]
    #[must_use = "newly created Weakable is immediately dropped"]
    pub fn new(t: T) -> Weakable<T> {
        Weakable {
            metadata: Cell::new(None),
            elem: t,
        }
    }
}

#[inline]
fn alloc_metadata(metadata: WeakMetadata) -> NonNull<WeakMetadata> {
    unsafe {
        let ptr: NonNull<WeakMetadata> = alloc_other();
        ptr::write(
            ptr.as_ptr(),
            metadata,
        );
        ptr
    }
}

impl<T: ?Sized + Trace + 'static> Weakable<T> {
    #[inline]
    fn init_get_metadata(&self) -> NonNull<WeakMetadata> {
        match self.metadata.get() {
            Some(ptr) => ptr,
            None => {
                let ptr = alloc_metadata(WeakMetadata::new(true));
                self.metadata.set(Some(ptr));
                ptr
            }
        }
    }

    // For tests
    #[cfg(test)]
    pub(crate) fn has_allocated(&self) -> bool {
        self.metadata.get().is_some()
    }
}

impl<T: Trace + 'static> Cc<Weakable<T>> {
    #[inline(always)]
    #[must_use = "newly created Cc is immediately dropped"]
    pub fn new_weakable(t: T) -> Self {
        Cc::new(Weakable::new(t))
    }

    #[must_use = "newly created Cc is immediately dropped"]
    #[track_caller]
    pub fn new_cyclic<F>(f: F) -> Cc<Weakable<T>>
        where
        F: FnOnce(&Weak<T>) -> T,
    {
        #[cfg(debug_assertions)]
        if state(|state| state.is_tracing()) {
            panic!("Cannot create a new Cc while tracing!");
        }

        let invalid_cc = Cc::new(Weakable::new(MaybeUninit::uninit()));
        let metadata: NonNull<WeakMetadata> = invalid_cc.init_get_metadata();

        // Set weak counter to 1
        // This is done after creating the Cc to make sure that if Cc::new panics the metadata allocation isn't leaked
        let _ = unsafe { metadata.as_ref() }.increment_counter();

        {
            let counter_marker = invalid_cc.inner().counter_marker();

            // The correctness of the decrement_counter() call depends on this, which should be always true
            debug_assert_eq!(1, counter_marker.counter());

            // Set strong count to 0
            let _ = counter_marker.decrement_counter();
        }

        // Get rid of invalid_cc. Having a Cc instance is dangerous, since:
        // 1. The strong count is now 0
        // 2. The Cc::drop implementation might be accidentally called during an unwinding
        let invalid = invalid_cc.inner_ptr();
        mem::forget(invalid_cc); // Don't execute invalid_cc's drop

        let weak: Weak<T> = Weak {
            metadata,
            cc: invalid.cast(),
        };

        // Panic guard to deallocate the metadata and the CcOnHeap if the provided function f panics
        struct PanicGuard<T: Trace + 'static> {
            invalid: NonNull<CcOnHeap<Weakable<MaybeUninit<T>>>>,
        }

        impl<T: Trace + 'static> Drop for PanicGuard<T> {
            fn drop(&mut self) {
                unsafe {
                    // Drop only the Weakable
                    drop_in_place::<Weakable<MaybeUninit<T>>>(self.invalid.as_ref().get_elem_mut());
                    // Deallocate the CcOnHeap
                    state(|state| {
                        let layout = self.invalid.as_ref().layout();
                        cc_dealloc(self.invalid, layout, state);
                    });
                }
            }
        }

        let panic_guard = PanicGuard { invalid };

        unsafe {
            // Write the newly created T
            invalid.as_ref().get_elem_mut().as_mut().unwrap_unchecked().elem.write(f(&weak));
        }

        // panic_guard is no longer needed
        mem::forget(panic_guard);

        // Set strong count to 1
        // This cannot fail since upgrade() cannot be called
        let _ = unsafe { invalid.as_ref() }.counter_marker().increment_counter();

        // Create the Cc again since it is now valid
        let cc: Cc<Weakable<T>> = Cc::__new_internal(invalid.cast());

        debug_assert_eq!(1, cc.inner().counter_marker().counter());

        // weak is dropped here automatically

        cc
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

        let metadata = self.init_get_metadata();

        if unsafe { metadata.as_ref() }.increment_counter().is_err() {
            panic!("Too many references has been created to a single Weak");
        }

        self.mark_alive();

        Weak {
            metadata,
            cc: self.inner_ptr(),
        }
    }

    #[inline]
    pub fn weak_count(&self) -> u32 {
        // This function returns an u32 although internally the weak counter is an u16 to have more flexibility for future expansions
        if let Some(metadata) = self.metadata.get() {
            unsafe { metadata.as_ref().counter() as u32 }
        } else {
            0
        }
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
        if let Some(metadata) = self.metadata.get() {
            unsafe {
                if metadata.as_ref().counter() == 0 {
                    // There are no weak pointers, deallocate the metadata
                    dealloc_other(metadata);
                } else {
                    // There exist weak pointers, set the CcOnHeap allocation not accessible
                    metadata.as_ref().set_accessible(false);
                }
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
