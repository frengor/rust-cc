use std::alloc::Layout;
use std::cell::UnsafeCell;
use std::marker::PhantomData;
use std::mem::{self, MaybeUninit};
use std::ops::Deref;
use std::ptr::{self, addr_of, addr_of_mut, drop_in_place, NonNull};
use std::rc::Rc;

#[cfg(feature = "nightly")]
use std::{
    marker::Unsize,
    ops::CoerceUnsized,
    ptr::{metadata, DynMetadata},
};

use crate::counter_marker::{CounterMarker, Mark, OverflowError};
use crate::state::{replace_state_field, state};
use crate::trace::{Context, ContextInner, Finalize, Trace};
use crate::utils::*;
use crate::{trigger_collection, try_state, POSSIBLE_CYCLES};

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
        if state(|state| state.is_tracing()) {
            panic!("Cannot create a new Cc while tracing!");
        }
        trigger_collection();
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
        if state(|state| state.is_tracing()) {
            panic!("Cannot create a new Cc while tracing!");
        }
        trigger_collection();

        let mut invalid = CcOnHeap::<T>::new_invalid();
        let cc = Cc {
            inner: invalid.cast(),
            _phantom: PhantomData,
        };

        unsafe {
            // Write the newly created T
            invalid.as_mut().elem.write(f(&cc));

            // Set valid
            (*cc.inner().counter_marker()).mark(Mark::NonMarked);
        }

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
            unsafe { !(*self.inner().counter_marker()).is_traced_or_invalid() },
            "Cc<_> is being used by the collector and inner value cannot be taken out (this might have happen inside Trace, Finalize or Drop implementations)."
        );

        // Make sure self is not into POSSIBLE_CYCLES before deallocating
        remove_from_list(self.inner.cast());

        // SAFETY: self is unique, valid and is not inside any list
        unsafe {
            let t = ptr::read(addr_of!((*self.inner.as_ptr()).elem));
            let layout = self.inner().layout();
            cc_dealloc(self.inner, layout);
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
            self.inner.as_mut().elem.write(value);
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
        self.inner().get_counter()
    }

    #[inline]
    pub fn is_unique(&self) -> bool {
        self.strong_count() == 1
    }

    #[inline]
    pub fn is_valid(&self) -> bool {
        self.inner().is_valid()
    }

    #[inline(always)]
    #[cfg(test)] // Don't expose is_valid to tests, expose this instead
    pub(crate) fn is_valid_for_test(&self) -> bool {
        self.is_valid()
    }

    /// Note: don't access self.inner().elem if CcOnHeap is not valid!
    #[inline(always)]
    fn inner(&self) -> &CcOnHeap<T> {
        // If Cc is alive then we can always access the underlying CcOnHeap
        unsafe { self.inner.as_ref() }
    }

    /// Note: don't access self.inner_mut().elem if CcOnHeap is not valid!
    #[inline(always)]
    fn inner_mut(&mut self) -> &mut CcOnHeap<T> {
        // If Cc is alive then we can always access the underlying CcOnHeap
        unsafe { self.inner.as_mut() }
    }
}

impl<T: ?Sized + Trace + 'static> Clone for Cc<T> {
    #[inline]
    #[track_caller]
    fn clone(&self) -> Self {
        if state(|state| state.is_tracing()) {
            panic!("Cannot clone while tracing!");
        }

        if self.inner().increment_counter().is_err() {
            panic!("Too many references has been created to a single Cc");
        }

        // Incrementing the tracing counter is necessary during finalization, always doing it
        // avoids the need to check whether state(|state| state.is_finalizing()) is true.
        // The result is discarded since the tracing counter may be greater than the counter,
        // however it is correct (i.e. equals to the counter) during finalization
        let _ = self.inner().increment_tracing_counter();

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
        if state(|state| state.is_tracing()) {
            panic!("Cannot deref while tracing!");
        }

        // Make sure we don't access an invalid Cc
        assert!(self.is_valid(), "This Cc is not valid!");

        remove_from_list(self.inner.cast());
        // We can do this since self is valid
        &self.inner().elem
    }
}

impl<T: ?Sized + Trace + 'static> Drop for Cc<T> {
    fn drop(&mut self) {
        #[inline(always)]
        fn counter_marker<T: ?Sized + Trace + 'static>(cc: &mut Cc<T>) -> &mut CounterMarker {
            // SAFETY: it's always safe to access the counter_marker
            unsafe {
                // Accessing directly the counter_marker avoids stacked borrows UB, since self.inner is borrowed
                // (uniquely) by CcOnHeap::drop_inner when calling drop_in_place (which calls this function).
                // Accessing the counter_marker field doesn't cause UB since it is inside an UnsafeCell.
                // (see https://github.com/rust-lang/unsafe-code-guidelines/blob/master/wip/stacked-borrows.md)
                (*addr_of_mut!((*cc.inner.as_ptr()).counter_marker)).get_mut()
            }
        }
        let counter_marker = counter_marker(self);

        // Always decrement the counter
        let res = counter_marker.decrement_counter();
        debug_assert!(res.is_ok());

        // If invalid no further actions are required
        if !counter_marker.is_valid() {
            return;
        }

        // If we're collecting and we're traced then we're part of a list different than POSSIBLE_CYCLES.
        // Almost no further action has to be taken, since the counter has been already decremented.
        // The last thing remaining to do is decrementing the tracing counter if we're both
        // collecting and finalizing to allow checking for resurrected objects.
        let res = try_state(|state| (state.is_collecting(), state.is_finalizing()));
        if let Ok((collecting, finalizing)) = res {
            // We know that inner is valid, so this is true only when collecting is true and counter_marker is traced
            if collecting && counter_marker.is_traced_or_invalid() {
                if finalizing {
                    let res = counter_marker.decrement_tracing_counter();
                    debug_assert!(res.is_ok());
                }
                return;
            }
        } else {
            // If state is not accessible then don't proceed further
            return;
        }

        if self.strong_count() == 0 {
            // Only us have a pointer to this allocation, deallocate!

            remove_from_list(self.inner.cast());

            let layout = self.inner().layout();

            let to_drop = if self.inner().is_finalizable() {
                let _finalizing_guard = replace_state_field!(finalizing, true);
                self.inner_mut().elem.finalize();
                self.strong_count() == 0
                // _finalizing_guard is dropped here, resetting state.finalizing
            } else {
                true
            };

            if to_drop {
                let _dropping_guard = replace_state_field!(dropping, true);
                // SAFETY: we're the only one to have a pointer to this allocation and we checked that inner is valid
                unsafe {
                    drop_in_place(self.inner.as_ptr());
                    cc_dealloc(self.inner, layout);
                }
                // _dropping_guard is dropped here, resetting state.dropping
            }
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
                self.inner().elem.trace(ctx);
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
    vtable: DynMetadata<dyn Trace>,

    #[cfg(not(feature = "nightly"))]
    fat_ptr: NonNull<dyn Trace>,

    counter_marker: UnsafeCell<CounterMarker>,
    _phantom: PhantomData<Rc<()>>, // Make CcOnHeap !Send and !Sync

    elem: T,
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
                    vtable: metadata(ptr.as_ptr() as *mut dyn Trace),
                    #[cfg(not(feature = "nightly"))]
                    fat_ptr: NonNull::new_unchecked(ptr.as_ptr() as *mut dyn Trace),
                    counter_marker: UnsafeCell::new(CounterMarker::new_with_counter_to_one(true)),
                    _phantom: PhantomData,
                    elem: t,
                },
            );
            ptr
        }
    }

    #[inline(always)]
    #[cfg(test)]
    #[must_use]
    pub(crate) fn new_for_test(t: T) -> NonNull<CcOnHeap<T>> {
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
                    vtable: metadata(ptr.cast::<CcOnHeap<T>>().as_ptr() as *mut dyn Trace),
                    #[cfg(not(feature = "nightly"))]
                    fat_ptr: NonNull::new_unchecked(
                        ptr.cast::<CcOnHeap<T>>().as_ptr() as *mut dyn Trace
                    ),
                    counter_marker: UnsafeCell::new({
                        let mut cm = CounterMarker::new_with_counter_to_one(true);
                        cm.mark(Mark::Invalid);
                        cm
                    }),
                    _phantom: PhantomData,
                    elem: MaybeUninit::uninit(),
                },
            );
            ptr
        }
    }
}

impl<T: ?Sized + Trace + 'static> CcOnHeap<T> {
    #[inline]
    pub(crate) fn is_valid(&self) -> bool {
        unsafe { (*self.counter_marker()).is_valid() }
    }

    #[inline]
    #[cfg(test)]
    pub(crate) fn get_elem(&self) -> &T {
        assert!(self.is_valid());
        &self.elem
    }

    #[inline]
    pub(crate) fn get_counter(&self) -> u32 {
        // SAFETY: it's always safe to access the counter_marker
        unsafe { (*self.counter_marker()).counter() }
    }

    #[inline]
    pub(crate) fn get_tracing_counter(&self) -> u32 {
        // SAFETY: it's always safe to access the counter_marker
        unsafe { (*self.counter_marker()).tracing_counter() }
    }

    #[inline]
    pub(crate) fn increment_counter(&self) -> Result<(), OverflowError> {
        // SAFETY: it's always safe to access the counter_marker
        unsafe { (*self.counter_marker()).increment_counter() }
    }

    #[inline]
    // Not currently used
    pub(crate) fn _decrement_counter(&self) -> Result<(), OverflowError> {
        // SAFETY: it's always safe to access the counter_marker
        unsafe { (*self.counter_marker()).decrement_counter() }
    }

    #[inline]
    pub(crate) fn increment_tracing_counter(&self) -> Result<(), OverflowError> {
        // SAFETY: it's always safe to access the counter_marker
        unsafe { (*self.counter_marker()).increment_tracing_counter() }
    }

    #[inline]
    pub(crate) fn counter_marker(&self) -> *mut CounterMarker {
        self.counter_marker.get()
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
    pub(crate) fn is_finalizable(&self) -> bool {
        // SAFETY: it's always safe to access the counter_marker
        unsafe { (*self.counter_marker()).is_finalizable() }
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

        self.elem.trace(ctx);
    }
}

impl<T: ?Sized + Trace + 'static> Finalize for CcOnHeap<T> {
    #[inline(always)]
    fn finalize(&mut self) {
        // This should never be called on an invalid instance.
        // The debug_assert should catch any bug related to this
        debug_assert!(self.is_valid());

        self.elem.finalize();
    }
}

pub(crate) fn remove_from_list(ptr: NonNull<CcOnHeap<()>>) {
    unsafe {
        // Check if ptr is in possible_cycles list. Note that if ptr points to an invalid CcOnHeap<_>,
        // then the if guard should never be true, since it is always marked as Mark::Invalid.
        // This is also the reason why this function is not marked as unsafe.
        if (*ptr.as_ref().counter_marker()).is_in_possible_cycles() {
            // ptr is in the list, remove it
            let _ = POSSIBLE_CYCLES.try_with(|pc| {
                let mut list = pc.borrow_mut();
                // Confirm is_in_possible_cycles() in debug builds
                debug_assert!(list.contains(ptr));

                list.remove(ptr);
                (*ptr.as_ref().counter_marker()).mark(Mark::NonMarked);
            });
        } else {
            // ptr is not in the list

            // Confirm !is_in_possible_cycles() in debug builds.
            // This is safe to do since we're not putting the CcOnHeap into the list
            debug_assert! {
                POSSIBLE_CYCLES.try_with(|pc| {
                    !pc.borrow().contains(ptr)
                }).unwrap_or(true)
            };
        }
    }
}

/// SAFETY: ptr must be pointing to a valid CcOnHeap<_>. More formally, `ptr.as_ref().is_valid()` must return `true`.
pub(crate) unsafe fn add_to_list(ptr: NonNull<CcOnHeap<()>>) {
    // Check if ptr can be added safely
    debug_assert!(ptr.as_ref().is_valid());

    let _ = POSSIBLE_CYCLES.try_with(|pc| {
        let mut list = pc.borrow_mut();
        // Check if ptr is in possible_cycles list since we have to move it at its start
        if (*ptr.as_ref().counter_marker()).is_in_possible_cycles() {
            // Confirm is_in_possible_cycles() in debug builds
            debug_assert!(list.contains(ptr));

            list.remove(ptr);
            // In this case we don't need to update the mark since we put it back into the list
        } else {
            // Confirm !is_in_possible_cycles() in debug builds
            debug_assert!(!list.contains(ptr));
            debug_assert!((*ptr.as_ref().counter_marker()).is_not_marked());

            // Mark it
            (*ptr.as_ref().counter_marker()).mark(Mark::PossibleCycles);
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
    /// SAFETY: self must be valid. More formally, `self.is_valid()` must return `true`.
    #[inline]
    pub(super) unsafe fn trace_inner(ptr: NonNull<Self>, ctx: &mut Context<'_>) {
        CcOnHeap::get_traceable(ptr).as_ref().trace(ctx);
    }

    /// SAFETY: self must be valid. More formally, `self.is_valid()` must return `true`.
    #[inline]
    pub(super) unsafe fn finalize_inner(ptr: NonNull<Self>) -> bool {
        CcOnHeap::get_traceable(ptr).as_mut().finalize();
        true
    }

    /// SAFETY: self must be valid. More formally, `self.is_valid()` must return `true`.
    #[inline]
    pub(super) unsafe fn drop_inner(ptr: NonNull<Self>) {
        drop_in_place(CcOnHeap::get_traceable(ptr).as_ptr());
    }

    /// SAFETY: self must be valid. More formally, `self.is_valid()` must return `true`.
    #[inline]
    unsafe fn get_traceable(ptr: NonNull<Self>) -> NonNull<dyn Trace> {
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
                // ptr is into POSSIBLE_CYCLES list

                remove_from_list(ptr); // Remove ptr from POSSIBLE_CYCLES list
                root_list.add(ptr);

                // Reset trace_counter
                (*counter_marker).reset_tracing_counter();

                // Element is surely not already marked, marking
                (*counter_marker).mark(Mark::TraceCounting);
            },
            ContextInner::RootTracing { .. } => {
                // ptr is into root_list

                // Element is not already marked, marking
                (*counter_marker).mark(Mark::TraceRoots);
            },
            ContextInner::DropTracing => {
                if (*counter_marker).is_marked_trace_dropping() {
                    return;
                }
                (*counter_marker).mark(Mark::TraceDropping);
            },
            ContextInner::DropResurrecting => {
                if (*counter_marker).is_marked_trace_resurrecting() {
                    return;
                }
                (*counter_marker).mark(Mark::TraceResurrecting);
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

        let counter_marker = ptr.as_ref().counter_marker();
        match ctx.inner() {
            ContextInner::Counting {
                root_list,
                non_root_list,
            } => {
                #[inline(always)]
                unsafe fn non_root(counter_marker: *mut CounterMarker) -> bool {
                    (*counter_marker).tracing_counter() == (*counter_marker).counter()
                }

                if !(*counter_marker).is_marked_trace_counting() {
                    // Not already marked

                    // Make sure ptr is not in POSSIBLE_CYCLES list
                    remove_from_list(ptr);

                    (*counter_marker).reset_tracing_counter();
                    let res = (*counter_marker).increment_tracing_counter();
                    debug_assert!(res.is_ok());

                    // Check invariant (tracing_counter is always less or equal to counter)
                    debug_assert!(
                        (*counter_marker).tracing_counter() <= (*counter_marker).counter()
                    );

                    if non_root(counter_marker) {
                        non_root_list.add(ptr);
                    } else {
                        root_list.add(ptr);
                    }

                    // Marking here since the previous debug_asserts might panic
                    // before ptr is actually added to root_list or non_root_list
                    (*counter_marker).mark(Mark::TraceCounting);

                    // Continue tracing
                    true
                } else {
                    // Check counters invariant (tracing_counter is always less or equal to counter)
                    // Only < is used here since tracing_counter will be incremented (by 1)
                    debug_assert!(
                        (*counter_marker).tracing_counter() < (*counter_marker).counter()
                    );

                    let res = (*counter_marker).increment_tracing_counter();
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
            ContextInner::RootTracing { non_root_list } => {
                if !(*counter_marker).is_marked_trace_roots() {
                    if !(*counter_marker).is_marked_trace_counting() {
                        // This CcOnHeap hasn't been traced during trace counting, so
                        // don't trace it now since it will surely not be deallocated
                        return false;
                    }

                    if (*counter_marker).tracing_counter() < (*counter_marker).counter() {
                        // If ptr is a root then stop tracing, since it will be handled
                        // at the next iteration of start_tracing

                        // Avoids tracing this CcOnHeap again
                        (*counter_marker).mark(Mark::TraceRoots);
                        return false;
                    }

                    // Else remove the element from non_root_list.
                    // Marking NonMarked since ptr will be removed from the list. Also, marking NonMarked
                    // will avoid tracing this CcOnHeap again, thanks to the 2 nested ifs above: at the
                    // next iteration this CcOnHeap won't be marked neither TraceRoots nor TraceCounting,
                    // so this function will return false and no tracing will happen
                    (*counter_marker).mark(Mark::NonMarked);
                    non_root_list.remove(ptr);

                    // Continue root tracing
                    return true;
                }
                // Don't continue trace in any other case
                false
            },
            ContextInner::DropTracing => {
                if (*counter_marker).is_marked_trace_dropping() {
                    let res = (*counter_marker).decrement_tracing_counter();
                    debug_assert!(res.is_ok());
                    false
                } else if (*counter_marker).is_marked_trace_roots() {
                    (*counter_marker).mark(Mark::TraceDropping);
                    let res = (*counter_marker).decrement_tracing_counter();
                    debug_assert!(res.is_ok());
                    true
                } else {
                    false
                }
            },
            ContextInner::DropResurrecting => {
                if !(*counter_marker).is_marked_trace_dropping()
                    || (*counter_marker).is_marked_trace_resurrecting()
                {
                    false
                } else {
                    (*counter_marker).mark(Mark::TraceResurrecting);
                    true
                }
            },
        }
    }
}
