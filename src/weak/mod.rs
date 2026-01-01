//! Non-owning [`Weak`] pointers to an allocation.
//!
//! The [`downgrade`][`method@Cc::downgrade`] method can be used on a [`Cc`] to create a non-owning [`Weak`][`crate::weak::Weak`] pointer.
//! A [`Weak`][`crate::weak::Weak`] pointer can be [`upgrade`][`method@Weak::upgrade`]d to a [`Cc`], but this will return
//! [`None`] if the allocation has already been deallocated.

use alloc::rc::Rc;
use core::fmt::{self, Debug, Formatter};
use core::marker::PhantomData;
use core::mem::MaybeUninit;
use core::ptr::{NonNull, drop_in_place};
#[cfg(feature = "nightly")]
use core::{marker::Unsize, ops::CoerceUnsized};
use core::{mem, ptr};

use crate::cc::{BoxedMetadata, CcBox};
use crate::state::try_state;
use crate::utils::{cc_dealloc, dealloc_other};
use crate::weak::weak_counter_marker::WeakCounterMarker;
use crate::{Cc, Context, Finalize, Trace};

pub(crate) mod weak_counter_marker;

/// A non-owning pointer to the managed allocation.
pub struct Weak<T: ?Sized + Trace + 'static> {
    metadata: Option<NonNull<BoxedMetadata>>, // None when created using Weak::new()
    cc: NonNull<CcBox<T>>,
    _phantom: PhantomData<Rc<T>>, // Make Weak !Send and !Sync
}

#[cfg(feature = "nightly")]
impl<T, U> CoerceUnsized<Weak<U>> for Weak<T>
where
    T: ?Sized + Trace + Unsize<U> + 'static,
    U: ?Sized + Trace + 'static,
{
}

impl<T: Trace> Weak<T> {
    /// Constructs a new [`Weak<T>`][`Weak`], without allocating any memory. Calling [`upgrade`][`method@Weak::upgrade`] on the returned value always gives [`None`].
    #[inline]
    pub fn new() -> Self {
        Weak {
            metadata: None,
            cc: NonNull::dangling(),
            _phantom: PhantomData,
        }
    }
}

impl<T: ?Sized + Trace> Weak<T> {
    /// Tries to upgrade the weak pointer to a [`Cc`], returning [`None`] if the allocation has already been deallocated.
    ///
    /// This creates a [`Cc`] pointer to the managed allocation, increasing the strong reference count.
    ///
    /// # Panics
    ///
    /// Panics if the strong reference count exceeds the maximum supported.
    #[inline]
    #[must_use = "newly created Cc is immediately dropped"]
    #[track_caller]
    pub fn upgrade(&self) -> Option<Cc<T>> {
        #[cfg(debug_assertions)]
        if crate::state::state(|state| state.is_tracing()) {
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

    /// Returns `true` if the two [`Weak`]s point to the same allocation, or if both don’t point to any allocation
    /// (because they were created with [`Weak::new()`][`Weak::new`]). This function ignores the metadata of `dyn Trait` pointers.
    #[inline]
    pub fn ptr_eq(this: &Weak<T>, other: &Weak<T>) -> bool {
        match (this.metadata, other.metadata) {
            (None, None) => true,
            (None, Some(_)) => false,
            (Some(_), None) => false,
            // Only compare the metadata allocations since they're surely unique
            (Some(m1), Some(m2)) => ptr::eq(m1.as_ptr() as *const (), m2.as_ptr() as *const ()),
        }
    }

    /// Returns the number of [`Cc`]s to the pointed allocation.
    ///
    /// If `self` was created using [`Weak::new`], this will return 0.
    #[inline]
    pub fn strong_count(&self) -> u32 {
        if self.weak_counter_marker().is_some_and(|wcm| wcm.is_accessible()) {
            // SAFETY: self.cc is still allocated and can be dereferenced
            let counter_marker = unsafe { self.cc.as_ref() }.counter_marker();

            // Return 0 if the object is in a list (or queue) and the collector is dropping. This is necessary since it's UB to access
            // Ccs from destructors, so calling upgrade on weak ptrs to such Ccs must be prevented.
            // This check does this, since such Ccs will be in a list at this point. Also, given that deallocations are done after
            // calling every destructor (this is an implementation detail), it's safe to access the counter_marker here.
            // Lastly, if the state cannot be accessed just return 0 to avoid giving a Cc when calling upgrade.

            // Return 0 also in the case the object was dropped, since weak pointers can survive the object itself

            let counter = counter_marker.counter();
            // Checking if the counter is already 0 avoids doing extra useless work, since the returned value would be the same
            if counter == 0
                || counter_marker.is_dropped()
                || (counter_marker.is_in_list_or_queue()
                    && try_state(|state| state.is_dropping()).unwrap_or(true))
            {
                0
            } else {
                counter as u32
            }
        } else {
            0
        }
    }

    /// Returns the number of [`Weak`]s to the pointed allocation.
    ///
    /// If `self` was created using [`Weak::new`], this will return 0.
    #[inline]
    pub fn weak_count(&self) -> u32 {
        // This function returns an u32 although internally the weak counter is an u16 to have more flexibility for future expansions
        self.weak_counter_marker().map_or(0, |wcm| wcm.counter() as u32)
    }

    #[inline]
    fn weak_counter_marker(&self) -> Option<&WeakCounterMarker> {
        Some(unsafe { &self.metadata?.as_ref().weak_counter_marker })
    }
}

impl<T: ?Sized + Trace> Clone for Weak<T> {
    /// Makes a clone of the [`Weak`] pointer.
    ///
    /// This creates another [`Weak`] pointer to the same allocation, increasing the weak reference count.
    ///
    /// # Panics
    ///
    /// Panics if the weak reference count exceeds the maximum supported.
    #[inline]
    #[track_caller]
    fn clone(&self) -> Self {
        #[cfg(debug_assertions)]
        if crate::state::state(|state| state.is_tracing()) {
            panic!("Cannot clone while tracing!");
        }

        if let Some(wcm) = self.weak_counter_marker()
            && wcm.increment_counter().is_err()
        {
            panic!("Too many references has been created to a single Weak");
        }

        Weak {
            metadata: self.metadata,
            cc: self.cc,
            _phantom: PhantomData,
        }
    }
}

impl<T: ?Sized + Trace> Drop for Weak<T> {
    #[inline]
    fn drop(&mut self) {
        let Some(metadata) = self.metadata else {
            return;
        };

        unsafe {
            // Always decrement the weak counter
            let res = metadata.as_ref().weak_counter_marker.decrement_counter();
            debug_assert!(res.is_ok());

            if metadata.as_ref().weak_counter_marker.counter() == 0
                && !metadata.as_ref().weak_counter_marker.is_accessible()
            {
                // No weak pointer is left and the CcBox has been deallocated, so just deallocate the metadata
                dealloc_other(metadata);
            }
        }
    }
}

unsafe impl<T: ?Sized + Trace> Trace for Weak<T> {
    #[inline(always)]
    fn trace(&self, _: &mut Context<'_>) {
        // Do not trace anything here, otherwise it wouldn't be a weak pointer
    }
}

impl<T: ?Sized + Trace> Finalize for Weak<T> {}

impl<T: Trace> Cc<T> {
    /// Creates a new [`Cc<T>`][`Cc`] while providing a [`Weak<T>`][`Weak`] pointer to the allocation,
    /// to allow the creation of a `T` which holds a weak pointer to itself.
    ///
    /// # Collection
    ///
    /// This method may start a collection when the `auto-collect` feature is enabled.
    ///
    /// See the [`config` module documentation][`mod@crate::config`] for more details.
    ///
    /// # Panics
    ///
    /// Panics if the provided closure or the automatically-stared collection panics.
    ///
    /// # Example
    #[cfg_attr(feature = "derive", doc = r"```rust")]
    #[cfg_attr(not(feature = "derive"), doc = r"```rust,ignore")]
    #[doc = r"# use rust_cc::*;
# use rust_cc::*;
# use rust_cc::weak::*;
# use rust_cc_derive::*;
#[derive(Trace, Finalize)]
struct Cyclic {
    cyclic: Weak<Self>,
}

let cyclic = Cc::new_cyclic(|weak| {
    Cyclic {
         cyclic: weak.clone(),
    }
});
```"]
    #[must_use = "newly created Cc is immediately dropped"]
    #[track_caller]
    pub fn new_cyclic<F>(f: F) -> Cc<T>
    where
        F: FnOnce(&Weak<T>) -> T,
    {
        #[cfg(debug_assertions)]
        if crate::state::state(|state| state.is_tracing()) {
            panic!("Cannot create a new Cc while tracing!");
        }

        let cc = Cc::new(NewCyclicWrapper::new());

        // Immediately call inner_ptr and forget the Cc instance. Having a Cc instance is dangerous, since:
        // 1. The strong count will become 0
        // 2. The Cc::drop implementation might be accidentally called during an unwinding
        let invalid_cc: NonNull<CcBox<_>> = cc.inner_ptr();
        mem::forget(cc);

        let metadata: NonNull<BoxedMetadata> =
            unsafe { invalid_cc.as_ref() }.get_or_init_metadata();

        // Set weak counter to 1
        // This is done after creating the Cc to make sure that if Cc::new panics the metadata allocation isn't leaked
        let _ = unsafe { metadata.as_ref() }.weak_counter_marker.increment_counter();

        {
            let counter_marker = unsafe { invalid_cc.as_ref() }.counter_marker();

            // The correctness of the decrement_counter() call depends on this, which should be always true
            debug_assert_eq!(1, counter_marker.counter());

            // Set strong count to 0
            let _ = counter_marker.decrement_counter();
        }

        let weak: Weak<T> = Weak {
            metadata: Some(metadata),
            cc: invalid_cc.cast(), // This cast is correct since NewCyclicWrapper is repr(transparent) and contains a MaybeUninit<T>
            _phantom: PhantomData,
        };

        // Panic guard to deallocate the metadata and the CcBox if the provided function f panics
        struct PanicGuard<T: Trace + 'static> {
            invalid_cc: NonNull<CcBox<NewCyclicWrapper<T>>>,
        }

        impl<T: Trace> Drop for PanicGuard<T> {
            fn drop(&mut self) {
                unsafe {
                    let layout = self.invalid_cc.as_ref().layout();

                    // Deallocate only the metadata allocation (the layout has already been read)
                    self.invalid_cc.as_ref().drop_metadata();

                    // Deallocate the CcBox. Use try_state to avoid panicking inside a Drop
                    let _ = try_state(|state| {
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
            (*invalid_cc.as_ref().get_elem_mut()).inner.write(to_write);
        }

        // Set strong count to 1
        // This cannot fail since upgrade() cannot be called
        let _ = unsafe { invalid_cc.as_ref() }.counter_marker().increment_counter();

        // Create the Cc again since it is now valid
        // Casting invalid_cc is correct since NewCyclicWrapper is repr(transparent) and contains a MaybeUninit<T>
        let cc: Cc<T> = Cc::__new_internal(invalid_cc.cast());

        debug_assert_eq!(1, cc.inner().counter_marker().counter());

        // weak is dropped here automatically

        cc
    }
}

impl<T: ?Sized + Trace> Cc<T> {
    /// Creates a new [`Weak`] pointer to the managed allocation, increasing the weak reference count.
    ///
    /// # Panics
    ///
    /// Panics if the strong reference count exceeds the maximum supported.
    #[inline]
    #[must_use = "newly created Weak is immediately dropped"]
    #[track_caller]
    pub fn downgrade(&self) -> Weak<T> {
        #[cfg(debug_assertions)]
        if crate::state::state(|state| state.is_tracing()) {
            panic!("Cannot downgrade while tracing!");
        }

        let metadata = self.inner().get_or_init_metadata();

        if unsafe { metadata.as_ref() }.weak_counter_marker.increment_counter().is_err() {
            panic!("Too many references has been created to a single Weak");
        }

        self.mark_alive();

        Weak {
            metadata: Some(metadata),
            cc: self.inner_ptr(),
            _phantom: PhantomData,
        }
    }

    /// Returns the number of [`Weak`]s to the pointed allocation.
    #[inline]
    pub fn weak_count(&self) -> u32 {
        // This function returns an u32 although internally the weak counter is an u16 to have more flexibility for future expansions
        if self.inner().counter_marker().has_allocated_for_metadata() {
            // SAFETY: The metadata has been allocated
            unsafe { self.inner().get_metadata_unchecked().as_ref() }.weak_counter_marker.counter()
                as u32
        } else {
            0
        }
    }
}

/// A **transparent** wrapper used to implement [`Cc::new_cyclic`].
#[repr(transparent)]
struct NewCyclicWrapper<T: Trace + 'static> {
    inner: MaybeUninit<T>,
}

impl<T: Trace> NewCyclicWrapper<T> {
    #[inline(always)]
    fn new() -> NewCyclicWrapper<T> {
        NewCyclicWrapper {
            inner: MaybeUninit::uninit(),
        }
    }
}

unsafe impl<T: Trace> Trace for NewCyclicWrapper<T> {
    #[inline]
    fn trace(&self, ctx: &mut Context<'_>) {
        // SAFETY: NewCyclicWrapper is used only in new_cyclic and a traceable Cc instance is not constructed until the contents are initialized
        unsafe {
            self.inner.assume_init_ref().trace(ctx);
        }
    }
}

impl<T: Trace> Finalize for NewCyclicWrapper<T> {
    #[inline]
    fn finalize(&self) {
        // SAFETY: NewCyclicWrapper is used only in new_cyclic and a traceable Cc instance is not constructed until the contents are initialized
        unsafe {
            self.inner.assume_init_ref().finalize();
        }
    }
}

impl<T: Trace> Drop for NewCyclicWrapper<T> {
    #[inline]
    fn drop(&mut self) {
        // SAFETY: NewCyclicWrapper is used only in new_cyclic and a traceable Cc instance is not constructed until the contents are initialized
        unsafe {
            let ptr = self.inner.assume_init_mut() as *mut T;
            drop_in_place(ptr);
        }
    }
}

// ####################################
// #         Weak Trait impls         #
// ####################################

impl<T: Trace> Default for Weak<T> {
    #[inline]
    fn default() -> Self {
        Weak::new()
    }
}

impl<T: ?Sized + Trace + Debug> Debug for Weak<T> {
    #[inline]
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "(Weak)")
    }
}
