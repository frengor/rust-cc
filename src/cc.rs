use std::alloc::Layout;
use std::cell::UnsafeCell;
use std::marker::PhantomData;
use std::mem::{self, MaybeUninit};
use std::ops::Deref;
use std::ptr::{self, addr_of, drop_in_place, NonNull};
use std::rc::Rc;
#[cfg(feature = "nightly")]
use std::{
    marker::Unsize,
    ops::CoerceUnsized,
    ptr::{metadata, DynMetadata},
};

use crate::counter_marker::{CounterMarker, Mark};
use crate::state::{replace_state_field, state, try_state};
use crate::trace::{Context, ContextInner, Finalize, Trace};
use crate::utils::*;
use crate::POSSIBLE_CYCLES;

#[repr(transparent)]
pub struct Cc<T: ?Sized + Trace + 'static> {
    inner: NonNull<CcOnHeap<T>>,
    _phantom: PhantomData<Rc<T>>, // Make Cc !Send and !Sync
}

#[cfg(feature = "nightly")]
impl<T, U> CoerceUnsized<Cc<U>> for Cc<T>
where
    T: ?Sized + Trace + Unsize<U> + 'static,
    U: ?Sized + Trace + 'static,
{
}

impl<T: Trace + 'static> Cc<T> {
    #[inline(always)]
    #[must_use = "newly created Cc is immediately dropped"]
    #[track_caller]
    pub fn new(t: T) -> Cc<T> {
        #[cfg(debug_assertions)]
        if state(|state| state.is_tracing()) {
            panic!("Cannot create a new Cc while tracing!");
        }

        #[cfg(feature = "auto-collect")]
        super::trigger_collection();

        Cc {
            inner: CcOnHeap::new(t),
            _phantom: PhantomData,
        }
    }

    #[must_use = "newly created Cc is immediately dropped"]
    #[track_caller]
    pub fn new_cyclic<F>(f: F) -> Cc<T>
    where
        F: FnOnce(&Cc<T>) -> T,
    {
        #[cfg(debug_assertions)]
        if state(|state| state.is_tracing()) {
            panic!("Cannot create a new Cc while tracing!");
        }

        #[cfg(feature = "auto-collect")]
        super::trigger_collection();

        let mut invalid: NonNull<CcOnHeap<MaybeUninit<T>>> = CcOnHeap::<T>::new_invalid();
        let cc = Cc {
            inner: invalid.cast(),
            _phantom: PhantomData,
        };

        unsafe {
            // Write the newly created T
            invalid.as_mut().elem.get_mut().write(f(&cc));
        }

        // Set valid
        cc.counter_marker().mark(Mark::NonMarked);

        // Return cc, since it is now valid
        cc
    }

    /// Takes out the value inside this [`Cc`].
    ///
    /// This is safe since this function panics if this [`Cc`] is not unique (see [`is_unique`]).
    ///
    /// # Panics
    /// This function panics if this [`Cc`] is not valid (see [`is_valid`]) or is not unique (see [`is_unique`]).
    ///
    /// [`is_valid`]: fn@Cc::is_valid
    /// [`is_unique`]: fn@Cc::is_unique
    #[inline]
    #[track_caller]
    pub fn into_inner(self) -> T {
        assert!(self.is_valid(), "Cc<_> is not valid");
        assert!(self.is_unique(), "Cc<_> is not unique");

        assert!(
            !self.counter_marker().is_traced_or_invalid(),
            "Cc<_> is being used by the collector and inner value cannot be taken out (this might have happen inside Trace, Finalize or Drop implementations)."
        );

        // Make sure self is not into POSSIBLE_CYCLES before deallocating
        remove_from_list(self.inner.cast());

        // SAFETY: self is unique, valid and is not inside any list
        unsafe {
            let t = ptr::read(self.inner().get_elem());
            let layout = self.inner().layout();
            let _ = try_state(|state| cc_dealloc(self.inner, layout, state));
            mem::forget(self); // Don't call drop on this Cc
            t
        }
    }
}

impl<T: Trace + 'static> Cc<MaybeUninit<T>> {
    /// Assumes that this [`Cc`] has been initialized.
    ///
    /// # Safety
    /// See [`MaybeUninit::assume_init`].
    ///
    /// # Panics
    /// This function panics if called while tracing or this [`Cc`] isn't valid (see [`is_valid`]).
    ///
    /// [`MaybeUninit::assume_init`]: fn@MaybeUninit::assume_init
    /// [`is_unique`]: fn@Cc::is_unique
    /// [`is_valid`]: fn@Cc::is_valid
    #[inline]
    #[track_caller]
    pub unsafe fn assume_init(self) -> Cc<T> {
        if state(|state| state.is_tracing()) {
            panic!("Cannot initialize a Cc while tracing!");
        }

        assert!(self.is_valid());

        remove_from_list(self.inner.cast());

        // The counter should not be updated since we're taking self
        let cc = Cc {
            inner: self.inner.cast(), // Safe since we're requiring that it is initialized
            _phantom: PhantomData,
        };

        mem::forget(self); // Don't call drop on this Cc
        cc
    }

    /// Initialize this [`Cc`] with the provided value and returns the initialized [`Cc<T>`].
    ///
    /// This is safe since this function panics if this [`Cc`] is not unique (see [`is_unique`]).
    ///
    /// # Panics
    /// This function panics if this [`Cc`] is not valid (see [`is_valid`]), is not unique (see [`is_unique`]) or if it's called while tracing.
    ///
    /// [`Cc<T>`]: struct@Cc
    /// [`MaybeUninit::assume_init`]: fn@MaybeUninit::assume_init
    /// [`is_unique`]: fn@Cc::is_unique
    /// [`is_valid`]: fn@Cc::is_valid
    #[inline]
    #[track_caller]
    pub fn init(mut self, value: T) -> Cc<T> {
        if state(|state| state.is_tracing()) {
            panic!("Cannot initialize a Cc while tracing!");
        }

        assert!(self.is_valid());

        // This prevents race conditions
        assert!(self.is_unique(), "Cc is not unique");

        remove_from_list(self.inner.cast());

        unsafe {
            self.inner.as_mut().elem.get_mut().write(value);
        }

        // The counter should not be updated since we're taking self
        let cc = Cc {
            inner: self.inner.cast(), // Safe since we've just initialized it
            _phantom: PhantomData,
        };

        mem::forget(self); // Don't call drop on this Cc
        cc
    }
}

impl<T: ?Sized + Trace + 'static> Cc<T> {
    #[inline]
    pub fn ptr_eq(this: &Cc<T>, other: &Cc<T>) -> bool {
        this.inner.as_ptr() == other.inner.as_ptr()
    }

    #[inline]
    pub fn strong_count(&self) -> u32 {
        self.counter_marker().counter()
    }

    #[inline]
    pub fn is_unique(&self) -> bool {
        self.strong_count() == 1
    }

    #[inline]
    pub fn is_valid(&self) -> bool {
        self.counter_marker().is_valid()
    }

    #[cfg(feature = "finalization")]
    #[inline]
    #[track_caller]
    pub fn finalize_again(&mut self) {
        assert!(state(|state| !state.is_collecting()), "Cannot schedule finalization again while collecting");

        self.counter_marker().set_finalized(false);
    }

    #[cfg(feature = "finalization")]
    #[inline]
    pub fn already_finalized(&self) -> bool {
        !self.counter_marker().needs_finalization()
    }

    #[inline(always)]
    fn counter_marker(&self) -> &CounterMarker {
        // SAFETY: It's always safe to access the counter_marker if we're not dereferencing anything else
        unsafe {
            &*addr_of!((*self.inner.as_ptr()).counter_marker)
        }
    }

    /// Note: don't call if CcOnHeap is not valid!
    #[inline(always)]
    fn inner(&self) -> &CcOnHeap<T> {
        // SAFETY: since Cc is alive and the underlying CcOnHeap is valid then we can access it
        unsafe { self.inner.as_ref() }
    }
}

impl<T: ?Sized + Trace + 'static> Clone for Cc<T> {
    #[inline]
    #[track_caller]
    fn clone(&self) -> Self {
        #[cfg(debug_assertions)]
        if state(|state| state.is_tracing()) {
            panic!("Cannot clone while tracing!");
        }

        if self.counter_marker().increment_counter().is_err() {
            panic!("Too many references has been created to a single Cc");
        }

        remove_from_list(self.inner.cast());

        // It's always safe to clone a Cc, even if the underlying CcOnHeap is invalid
        Cc {
            inner: self.inner,
            _phantom: PhantomData,
        }
    }
}

impl<T: ?Sized + Trace + 'static> Deref for Cc<T> {
    type Target = T;

    #[inline]
    #[track_caller]
    fn deref(&self) -> &Self::Target {
        #[cfg(debug_assertions)]
        if state(|state| state.is_tracing()) {
            panic!("Cannot deref while tracing!");
        }

        // Make sure we don't access an invalid Cc
        assert!(self.is_valid(), "This Cc is not valid!");

        remove_from_list(self.inner.cast());
        // We can do this since self is valid
        self.inner().get_elem()
    }
}

impl<T: ?Sized + Trace + 'static> Drop for Cc<T> {
    fn drop(&mut self) {
        let counter_marker = self.counter_marker();

        // Always decrement the counter
        let res = counter_marker.decrement_counter();
        debug_assert!(res.is_ok());

        // A CcOnHeap can be marked traced only during collections while being into a list different than POSSIBLE_CYCLES.
        // In this case, or when invalid, no further action has to be taken, since the counter has been already decremented.
        if counter_marker.is_traced_or_invalid() {
            return;
        }

        if counter_marker.counter() == 0 {
            // Only us have a pointer to this allocation, deallocate!

            remove_from_list(self.inner.cast());

            state(|state| {
                let to_drop = if cfg!(feature = "finalization") && counter_marker.needs_finalization() {
                    // This cfg is necessary since the cfg! above still compiles the line below,
                    // however state doesn't contain the finalizing field when the finalization feature is off,
                    // so removing this cfg makes the crate to fail compilation
                    #[cfg(feature = "finalization")]
                    let _finalizing_guard = replace_state_field!(finalizing, true, state);

                    // Set finalized
                    counter_marker.set_finalized(true);

                    self.inner().get_elem().finalize();
                    counter_marker.counter() == 0
                    // _finalizing_guard is dropped here, resetting state.finalizing
                } else {
                    true
                };

                if to_drop {
                    let _dropping_guard = replace_state_field!(dropping, true, state);
                    let layout = self.inner().layout();

                    // SAFETY: we're the only one to have a pointer to this allocation and we checked that inner is valid
                    unsafe {
                        drop_in_place(self.inner().elem.get());
                        cc_dealloc(self.inner, layout, state);
                    }
                    // _dropping_guard is dropped here, resetting state.dropping
                }
            });
        } else {
            // SAFETY: we checked that inner is valid
            // We also know that we're not part of either root_list or non_root_list, since we haven't returned earlier
            unsafe { add_to_list(self.inner.cast()) };
        }
    }
}

unsafe impl<T: ?Sized + Trace + 'static> Trace for Cc<T> {
    #[inline]
    #[track_caller]
    fn trace(&self, ctx: &mut Context<'_>) {
        // This must be done, since it is possible to call this function on an invalid instance
        assert!(self.is_valid());

        // SAFETY: we have just checked that self is valid
        unsafe {
            if CcOnHeap::trace(self.inner.cast(), ctx) {
                self.inner().get_elem().trace(ctx);
            }
        }
    }
}

impl<T: ?Sized + Trace + 'static> Finalize for Cc<T> {}

#[repr(C)]
pub(crate) struct CcOnHeap<T: ?Sized + Trace + 'static> {
    next: UnsafeCell<Option<NonNull<CcOnHeap<()>>>>,
    prev: UnsafeCell<Option<NonNull<CcOnHeap<()>>>>,

    #[cfg(feature = "nightly")]
    vtable: DynMetadata<dyn InternalTrace>,

    #[cfg(not(feature = "nightly"))]
    fat_ptr: NonNull<dyn InternalTrace>,

    counter_marker: CounterMarker,
    _phantom: PhantomData<Rc<()>>, // Make CcOnHeap !Send and !Sync

    // This UnsafeCell is necessary, since we want to execute Drop::drop
    // for elem but still have access to the other fields of CcOnHeap
    elem: UnsafeCell<T>,
}

impl<T: Trace + 'static> CcOnHeap<T> {
    #[inline(always)]
    #[must_use]
    fn new(t: T) -> NonNull<CcOnHeap<T>> {
        let layout = Layout::new::<CcOnHeap<T>>();
        unsafe {
            let ptr: NonNull<CcOnHeap<T>> = cc_alloc(layout);
            ptr::write(
                ptr.as_ptr(),
                CcOnHeap {
                    next: UnsafeCell::new(None),
                    prev: UnsafeCell::new(None),
                    #[cfg(feature = "nightly")]
                    vtable: metadata(ptr.as_ptr() as *mut dyn InternalTrace),
                    #[cfg(not(feature = "nightly"))]
                    fat_ptr: NonNull::new_unchecked(ptr.as_ptr() as *mut dyn InternalTrace),
                    counter_marker: CounterMarker::new_with_counter_to_one(),
                    _phantom: PhantomData,
                    elem: UnsafeCell::new(t),
                },
            );
            ptr
        }
    }

    #[inline(always)]
    #[cfg(test)]
    #[must_use]
    pub(crate) fn new_for_tests(t: T) -> NonNull<CcOnHeap<T>> {
        CcOnHeap::new(t)
    }

    #[inline]
    #[must_use]
    fn new_invalid() -> NonNull<CcOnHeap<MaybeUninit<T>>> {
        let layout = Layout::new::<CcOnHeap<MaybeUninit<T>>>();
        unsafe {
            let ptr: NonNull<CcOnHeap<MaybeUninit<T>>> = cc_alloc(layout);
            ptr::write(
                ptr.as_ptr(),
                CcOnHeap {
                    next: UnsafeCell::new(None),
                    prev: UnsafeCell::new(None),
                    #[cfg(feature = "nightly")]
                    vtable: metadata(ptr.cast::<CcOnHeap<T>>().as_ptr() as *mut dyn InternalTrace),
                    #[cfg(not(feature = "nightly"))]
                    fat_ptr: NonNull::new_unchecked(
                        ptr.cast::<CcOnHeap<T>>().as_ptr() as *mut dyn InternalTrace
                    ),
                    counter_marker: {
                        let cm = CounterMarker::new_with_counter_to_one();
                        cm.mark(Mark::Invalid);
                        cm
                    },
                    _phantom: PhantomData,
                    elem: UnsafeCell::new(MaybeUninit::uninit()),
                },
            );
            ptr
        }
    }
}

impl<T: ?Sized + Trace + 'static> CcOnHeap<T> {
    #[inline]
    pub(crate) fn is_valid(&self) -> bool {
        self.counter_marker().is_valid()
    }

    #[inline]
    pub(crate) fn get_elem(&self) -> &T {
        debug_assert!(self.is_valid());
        unsafe { &*self.elem.get() }
    }

    #[inline]
    pub(crate) fn counter_marker(&self) -> &CounterMarker {
        &self.counter_marker
    }

    #[inline]
    pub(crate) fn layout(&self) -> Layout {
        #[cfg(feature = "nightly")]
        {
            self.vtable.layout()
        }

        #[cfg(not(feature = "nightly"))]
        unsafe {
            Layout::for_value(self.fat_ptr.as_ref())
        }
    }

    #[inline]
    pub(super) fn get_next(&self) -> *mut Option<NonNull<CcOnHeap<()>>> {
        self.next.get()
    }

    #[inline]
    pub(super) fn get_prev(&self) -> *mut Option<NonNull<CcOnHeap<()>>> {
        self.prev.get()
    }
}

unsafe impl<T: ?Sized + Trace + 'static> Trace for CcOnHeap<T> {
    #[inline(always)]
    fn trace(&self, ctx: &mut Context<'_>) {
        // This should never be called on an invalid instance.
        // The debug_assert should catch any bug related to this
        debug_assert!(self.is_valid());

        self.get_elem().trace(ctx);
    }
}

impl<T: ?Sized + Trace + 'static> Finalize for CcOnHeap<T> {
    #[inline(always)]
    fn finalize(&self) {
        // This should never be called on an invalid instance.
        // The debug_assert should catch any bug related to this
        debug_assert!(self.is_valid());

        self.get_elem().finalize();
    }
}

#[inline]
pub(crate) fn remove_from_list(ptr: NonNull<CcOnHeap<()>>) {
    unsafe {
        let counter_marker = ptr.as_ref().counter_marker();

        // Check if ptr is in possible_cycles list. Note that if ptr points to an invalid CcOnHeap<_>,
        // then the if guard should never be true, since it is always marked as Mark::Invalid.
        // This is also the reason why this function is not marked as unsafe.
        if counter_marker.is_in_possible_cycles() {
            // ptr is in the list, remove it
            let _ = POSSIBLE_CYCLES.try_with(|pc| {
                let mut list = pc.borrow_mut();
                // Confirm is_in_possible_cycles() in debug builds
                #[cfg(feature = "pedantic-debug-assertions")]
                debug_assert!(list.contains(ptr));

                counter_marker.mark(Mark::NonMarked);
                list.remove(ptr);
            });
        } else {
            // ptr is not in the list

            // Confirm !is_in_possible_cycles() in debug builds.
            // This is safe to do since we're not putting the CcOnHeap into the list
            #[cfg(feature = "pedantic-debug-assertions")]
            debug_assert! {
                POSSIBLE_CYCLES.try_with(|pc| {
                    !pc.borrow().contains(ptr)
                }).unwrap_or(true)
            };
        }
    }
}

/// SAFETY: ptr must be pointing to a valid CcOnHeap<_>. More formally, `ptr.as_ref().is_valid()` must return `true`.
#[inline]
pub(crate) unsafe fn add_to_list(ptr: NonNull<CcOnHeap<()>>) {
    let counter_marker = ptr.as_ref().counter_marker();

    // Check if ptr can be added safely
    debug_assert!(counter_marker.is_valid());

    let _ = POSSIBLE_CYCLES.try_with(|pc| {
        let mut list = pc.borrow_mut();

        // Check if ptr is in possible_cycles list since we have to move it at its start
        if counter_marker.is_in_possible_cycles() {
            // Confirm is_in_possible_cycles() in debug builds
            #[cfg(feature = "pedantic-debug-assertions")]
            debug_assert!(list.contains(ptr));

            list.remove(ptr);
            // In this case we don't need to update the mark since we put it back into the list
        } else {
            // Confirm !is_in_possible_cycles() in debug builds
            #[cfg(feature = "pedantic-debug-assertions")]
            debug_assert!(!list.contains(ptr));
            debug_assert!(counter_marker.is_not_marked());

            // Mark it
            counter_marker.mark(Mark::PossibleCycles);
        }
        // Add to the list
        //
        // Make sure this operation is the first after the if-else, since the CcOnHeap is in
        // an invalid state now (it's marked Mark::PossibleCycles, but it isn't into the list)
        list.add(ptr);
    });
}

// Functions in common between every CcOnHeap<_>
impl CcOnHeap<()> {
    /// SAFETY: ptr must be pointing to a valid CcOnHeap<_>. More formally, `ptr.as_ref().is_valid()` must return `true`.
    #[inline]
    pub(super) unsafe fn trace_inner(ptr: NonNull<Self>, ctx: &mut Context<'_>) {
        CcOnHeap::get_traceable(ptr).as_ref().trace(ctx);
    }

    /// SAFETY: ptr must be pointing to a valid CcOnHeap<_>. More formally, `ptr.as_ref().is_valid()` must return `true`.
    #[cfg(feature = "finalization")]
    #[inline]
    pub(super) unsafe fn finalize_inner(ptr: NonNull<Self>) -> bool {
        if ptr.as_ref().counter_marker().needs_finalization() {
            // Set finalized
            ptr.as_ref().counter_marker().set_finalized(true);

            CcOnHeap::get_traceable(ptr).as_ref().finalize_elem();
            true
        } else {
            false
        }
    }

    /// SAFETY: `drop_in_place` conditions must be true and ptr must be pointing to a valid CcOnHeap<_>.
    ///         More formally, `ptr.as_ref().is_valid()` must return `true`.
    #[inline]
    pub(super) unsafe fn drop_inner(ptr: NonNull<Self>) {
        CcOnHeap::get_traceable(ptr).as_mut().drop_elem();
    }

    /// SAFETY: ptr must be pointing to a valid CcOnHeap<_>. More formally, `ptr.as_ref().is_valid()` must return `true`.
    #[inline]
    unsafe fn get_traceable(ptr: NonNull<Self>) -> NonNull<dyn InternalTrace> {
        debug_assert!(ptr.as_ref().is_valid()); // Just to be sure

        #[cfg(feature = "nightly")]
        {
            let vtable = ptr.as_ref().vtable;
            NonNull::from_raw_parts(ptr.cast(), vtable)
        }

        #[cfg(not(feature = "nightly"))]
        {
            ptr.as_ref().fat_ptr
        }
    }

    /// SAFETY: ptr must be pointing to a valid CcOnHeap<_>. More formally, `ptr.as_ref().is_valid()` must return `true`.
    pub(super) unsafe fn start_tracing(ptr: NonNull<Self>, ctx: &mut Context<'_>) {
        debug_assert!(ptr.as_ref().is_valid());

        let counter_marker = ptr.as_ref().counter_marker();
        match ctx.inner() {
            ContextInner::Counting { root_list, .. } => {
                // ptr is NOT into POSSIBLE_CYCLES list: ptr has just been removed from
                // POSSIBLE_CYCLES by rust_cc::collect() (see lib.rs) before calling this function

                root_list.add(ptr);

                // Reset trace_counter
                counter_marker.reset_tracing_counter();

                // Element is surely not already marked, marking
                counter_marker.mark(Mark::Traced);
            },
            ContextInner::RootTracing { .. } => {
                // ptr is a root

                // Nothing to do here, ptr is already unmarked
                debug_assert!(counter_marker.is_not_marked());
            },
        }

        // ptr is surely to trace
        //
        // This function is called from collect_cycles(), which doesn't know the
        // exact type of the element inside CcOnHeap, so trace it using the vtable
        //
        // SAFETY: we require that ptr points to a valid CcOnHeap<_>
        CcOnHeap::trace_inner(ptr, ctx);
    }

    /// Returns whether `ptr.elem` should be traced.
    ///
    /// This function returns a `bool` instead of directly tracing the element inside the CcOnHeap, since this way
    /// we can avoid using the vtable most of the times (the responsibility of tracing the inner element is passed
    /// to the caller, which *might* have more information on the type inside CcOnHeap than us).
    ///
    /// SAFETY: ptr must be pointing to a valid CcOnHeap<_>. More formally, `ptr.as_ref().is_valid()` must return `true`.
    #[inline(never)] // Don't inline this function, it's huge
    #[must_use = "the element inside ptr is not traced by CcOnHeap::trace"]
    unsafe fn trace(ptr: NonNull<Self>, ctx: &mut Context<'_>) -> bool {
        debug_assert!(ptr.as_ref().is_valid());

        #[inline(always)]
        fn non_root(counter_marker: &CounterMarker) -> bool {
            counter_marker.tracing_counter() == counter_marker.counter()
        }

        let counter_marker = ptr.as_ref().counter_marker();
        match ctx.inner() {
            ContextInner::Counting {
                root_list,
                non_root_list,
            } => {
                if !counter_marker.is_traced() {
                    // Not already marked

                    // Make sure ptr is not in POSSIBLE_CYCLES list
                    remove_from_list(ptr);

                    counter_marker.reset_tracing_counter();
                    let res = counter_marker.increment_tracing_counter();
                    debug_assert!(res.is_ok());

                    // Check invariant (tracing_counter is always less or equal to counter)
                    debug_assert!(counter_marker.tracing_counter() <= counter_marker.counter());

                    if non_root(counter_marker) {
                        non_root_list.add(ptr);
                    } else {
                        root_list.add(ptr);
                    }

                    // Marking here since the previous debug_asserts might panic
                    // before ptr is actually added to root_list or non_root_list
                    counter_marker.mark(Mark::Traced);

                    // Continue tracing
                    true
                } else {
                    // Check counters invariant (tracing_counter is always less or equal to counter)
                    // Only < is used here since tracing_counter will be incremented (by 1)
                    debug_assert!(counter_marker.tracing_counter() < counter_marker.counter());

                    let res = counter_marker.increment_tracing_counter();
                    debug_assert!(res.is_ok());

                    if non_root(counter_marker) {
                        // Already marked, so ptr was put in root_list
                        root_list.remove(ptr);
                        non_root_list.add(ptr);
                    }

                    // Don't continue tracing
                    false
                }
            },
            ContextInner::RootTracing { non_root_list, root_list } => {
                if counter_marker.is_traced() {
                    // Marking NonMarked since ptr will be removed from any list it's into. Also, marking
                    // NonMarked will avoid tracing this CcOnHeap again (thanks to the if condition)
                    counter_marker.mark(Mark::NonMarked);

                    if non_root(counter_marker) {
                        non_root_list.remove(ptr);
                    } else {
                        root_list.remove(ptr);
                    }

                    // Continue root tracing
                    true
                } else {
                    // Don't continue tracing
                    false
                }
            },
        }
    }
}

// Trait used to make it possible to drop/finalize only the elem field of CcOnHeap
// and without taking a &mut reference to the whole CcOnHeap
trait InternalTrace: Trace {
    fn finalize_elem(&self);

    /// Safety: see `drop_in_place`
    unsafe fn drop_elem(&self);
}

impl<T: ?Sized + Trace + 'static> InternalTrace for CcOnHeap<T> {
    fn finalize_elem(&self) {
        self.get_elem().finalize();
    }

    unsafe fn drop_elem(&self) {
        drop_in_place(self.elem.get());
    }
}
