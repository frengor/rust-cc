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

use crate::cc::CcBox;
use crate::state::{state, try_state};
use crate::{Cc, Context, Finalize, Trace};
use crate::utils::{alloc_other, cc_dealloc, dealloc_other};
use crate::weak::weak_metadata::WeakMetadata;

mod weak_metadata;

pub type WeakableCc<T> = Cc<Weakable<T>>;

pub struct Weak<T: ?Sized + Trace + 'static> {
    metadata: NonNull<WeakMetadata>,
    cc: NonNull<CcBox<Weakable<T>>>,
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
            let counter_marker = unsafe { self.cc.as_ref() }.counter_marker();

            // Return 0 if the object is traced and the collector is dropping. This is necessary since it's UB to access
            // Ccs from destructors, so calling upgrade on weak ptrs to such Ccs must be prevented.
            // This check does this, since such Ccs will be traced at this point. Also, given that deallocations are done after
            // calling every destructor (this is an implementation detail), it's safe to access the counter_marker here.
            // Lastly, if the state cannot be accessed just return 0 to avoid giving a Cc when calling upgrade

            // Return 0 also in the case the object was dropped, since weak pointers can survive the object itself

            let counter = counter_marker.counter();
            // Checking if the counter is already 0 avoids doing extra useless work, since the returned value would be the same
            if counter == 0 || counter_marker.is_dropped() || (counter_marker.is_traced() && try_state(|state| state.is_dropping()).unwrap_or(true)) {
                0
            } else {
                counter
            }
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
            // No weak pointer is left and the CcBox has been deallocated, so just deallocate the metadata
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

    #[cfg(all(test, feature = "std"))] // Only used in unit tests
    pub(crate) fn has_allocated(&self) -> bool {
        self.metadata.get().is_some()
    }
}

impl<T: Trace + 'static> Cc<Weakable<T>> {
    #[inline(always)]
    #[must_use = "newly created Cc is immediately dropped"]
    #[track_caller]
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

        let cc = Cc::new(Weakable::new(NewCyclicWrapper::new()));

        // Immediately call inner_ptr and forget the Cc instance. Having a Cc instance is dangerous, since:
        // 1. The strong count will become 0
        // 2. The Cc::drop implementation might be accidentally called during an unwinding
        let invalid_cc: NonNull<CcBox<_>> = cc.inner_ptr();
        mem::forget(cc);

        let metadata: NonNull<WeakMetadata> = unsafe { invalid_cc.as_ref() }.get_elem().init_get_metadata();

        // Set weak counter to 1
        // This is done after creating the Cc to make sure that if Cc::new panics the metadata allocation isn't leaked
        let _ = unsafe { metadata.as_ref() }.increment_counter();

        {
            let counter_marker = unsafe { invalid_cc.as_ref() }.counter_marker();

            // The correctness of the decrement_counter() call depends on this, which should be always true
            debug_assert_eq!(1, counter_marker.counter());

            // Set strong count to 0
            let _ = counter_marker.decrement_counter();
        }

        let weak: Weak<T> = Weak {
            metadata,
            cc: invalid_cc.cast(), // This cast is correct since NewCyclicWrapper is repr(transparent) and contains a MaybeUninit<T>
        };

        // Panic guard to deallocate the metadata and the CcBox if the provided function f panics
        struct PanicGuard<T: Trace + 'static> {
            invalid_cc: NonNull<CcBox<Weakable<NewCyclicWrapper<T>>>>,
        }

        impl<T: Trace + 'static> Drop for PanicGuard<T> {
            fn drop(&mut self) {
                unsafe {
                    // Deallocate only the metadata allocation
                    (*self.invalid_cc.as_ref().get_elem_mut()).drop_metadata();
                    // Deallocate the CcBox. Use try_state to avoid panicking inside a Drop
                    let _ = try_state(|state| {
                        let layout = self.invalid_cc.as_ref().layout();
                        cc_dealloc(self.invalid_cc, layout, state);
                    });
                }
            }
        }

        let panic_guard = PanicGuard { invalid_cc };
        let to_write = f(&weak);
        mem::forget(panic_guard); // Panic guard is no longer useful

        unsafe {
            // Write the newly created T
            (*invalid_cc.as_ref().get_elem_mut()).elem.inner.write(to_write);
        }

        // Set strong count to 1
        // This cannot fail since upgrade() cannot be called
        let _ = unsafe { invalid_cc.as_ref() }.counter_marker().increment_counter();

        // Create the Cc again since it is now valid
        // Casting invalid_cc is correct since NewCyclicWrapper is repr(transparent) and contains a MaybeUninit<T>
        let cc: Cc<Weakable<T>> = Cc::__new_internal(invalid_cc.cast());

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

impl<T: ?Sized + Trace + 'static> Weakable<T> {
    fn drop_metadata(&mut self) {
        if let Some(metadata) = self.metadata.get() {
            unsafe {
                if metadata.as_ref().counter() == 0 {
                    // There are no weak pointers, deallocate the metadata
                    dealloc_other(metadata);
                } else {
                    // There exist weak pointers, set the CcBox allocation not accessible
                    metadata.as_ref().set_accessible(false);
                }
            }
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
        self.drop_metadata();
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

/// A **transparent** wrapper used to implement [`Cc::new_cyclic`].
#[repr(transparent)]
struct NewCyclicWrapper<T: Trace + 'static> {
    inner: MaybeUninit<T>,
}

impl<T: Trace + 'static> NewCyclicWrapper<T> {
    fn new() -> NewCyclicWrapper<T> {
        NewCyclicWrapper {
            inner: MaybeUninit::uninit(),
        }
    }
}

unsafe impl<T: Trace + 'static> Trace for NewCyclicWrapper<T> {
    fn trace(&self, ctx: &mut Context<'_>) {
        // SAFETY: NewCyclicWrapper is used only in new_cyclic and a traceable Cc instance is not constructed until the contents are initialized
        unsafe {
            self.inner.assume_init_ref().trace(ctx);
        }
    }
}

impl<T: Trace + 'static> Finalize for NewCyclicWrapper<T> {
    fn finalize(&self) {
        // SAFETY: NewCyclicWrapper is used only in new_cyclic and a traceable Cc instance is not constructed until the contents are initialized
        unsafe {
            self.inner.assume_init_ref().finalize();
        }
    }
}

impl<T: Trace + 'static> Drop for NewCyclicWrapper<T> {
    fn drop(&mut self) {
        // SAFETY: NewCyclicWrapper is used only in new_cyclic and a traceable Cc instance is not constructed until the contents are initialized
        unsafe {
            let ptr = self.inner.assume_init_mut() as *mut T;
            drop_in_place(ptr);
        }
    }
}
