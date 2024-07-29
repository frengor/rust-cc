use alloc::alloc::Layout;
use alloc::rc::Rc;
use core::cell::UnsafeCell;
use core::marker::PhantomData;
use core::mem;
use core::ops::Deref;
use core::ptr::{self, drop_in_place, NonNull};
use core::borrow::Borrow;
use core::cell::Cell;
use core::fmt::{self, Debug, Display, Formatter, Pointer};
use core::cmp::Ordering;
use core::hash::{Hash, Hasher};
use core::panic::{RefUnwindSafe, UnwindSafe};
#[cfg(feature = "nightly")]
use core::{
    marker::Unsize,
    ops::CoerceUnsized,
    ptr::{metadata, DynMetadata},
};

use crate::counter_marker::{CounterMarker, Mark};
use crate::state::{replace_state_field, state, State, try_state};
use crate::trace::{Context, ContextInner, Finalize, Trace};
use crate::list::ListMethods;
use crate::utils::*;
use crate::POSSIBLE_CYCLES;
#[cfg(feature = "weak-ptrs")]
use crate::weak::weak_counter_marker::WeakCounterMarker;

/// A thread-local cycle collected pointer.
///
/// See the [module-level documentation][`mod@crate`] for more details.
#[repr(transparent)]
pub struct Cc<T: ?Sized + Trace + 'static> {
    inner: NonNull<CcBox<T>>,
    _phantom: PhantomData<Rc<T>>, // Make Cc !Send and !Sync
}

#[cfg(feature = "nightly")]
impl<T, U> CoerceUnsized<Cc<U>> for Cc<T>
where
    T: ?Sized + Trace + Unsize<U> + 'static,
    U: ?Sized + Trace + 'static,
{
}

impl<T: Trace> Cc<T> {
    /// Creates a new `Cc`.
    /// 
    /// # Collection
    /// 
    /// This method may start a collection when the `auto-collect` feature is enabled.
    /// 
    /// See the [`config` module documentation][`mod@crate::config`] for more details.
    /// 
    /// # Panics
    /// 
    /// Panics if the automatically-stared collection panics.
    #[inline(always)]
    #[must_use = "newly created Cc is immediately dropped"]
    #[track_caller]
    pub fn new(t: T) -> Cc<T> {
        state(|state| {
            #[cfg(debug_assertions)]
            if state.is_tracing() {
                panic!("Cannot create a new Cc while tracing!");
            }

            #[cfg(feature = "auto-collect")]
            super::trigger_collection(state);

            Cc {
                inner: CcBox::new(t, state),
                _phantom: PhantomData,
            }
        })
    }

    /// Takes out the value inside a [`Cc`].
    ///
    /// # Panics
    /// Panics if the [`Cc`] is not unique (see [`is_unique`]).
    ///
    /// [`is_unique`]: fn@Cc::is_unique
    #[inline]
    #[track_caller]
    pub fn into_inner(self) -> T {
        assert!(self.is_unique(), "Cc<_> is not unique");

        assert!(
            !self.counter_marker().is_traced(),
            "Cc<_> is being used by the collector and inner value cannot be taken out (this might have happen inside Trace, Finalize or Drop implementations)."
        );

        // Make sure self is not into POSSIBLE_CYCLES before deallocating
        remove_from_list(self.inner.cast());

        // SAFETY: self is unique and is not inside any list
        unsafe {
            let t = ptr::read(self.inner().get_elem());
            let layout = self.inner().layout();
            let _ = try_state(|state| cc_dealloc(self.inner, layout, state));
            mem::forget(self); // Don't call drop on this Cc
            t
        }
    }
}

impl<T: ?Sized + Trace> Cc<T> {
    /// Returns `true` if the two [`Cc`]s point to the same allocation. This function ignores the metadata of `dyn Trait` pointers.
    #[inline]
    pub fn ptr_eq(this: &Cc<T>, other: &Cc<T>) -> bool {
        ptr::eq(this.inner.as_ptr() as *const (), other.inner.as_ptr() as *const ())
    }

    /// Returns the number of [`Cc`]s to the pointed allocation.
    #[inline]
    pub fn strong_count(&self) -> u32 {
        self.counter_marker().counter()
    }

    /// Returns `true` if the strong reference count is `1`, `false` otherwise.
    #[inline]
    pub fn is_unique(&self) -> bool {
        self.strong_count() == 1
    }

    /// Makes the value in the managed allocation finalizable again.
    /// 
    /// # Panics
    /// 
    /// Panics if called during a collection.
    #[cfg(feature = "finalization")]
    #[inline]
    #[track_caller]
    pub fn finalize_again(&mut self) {
        // The is_finalizing and is_dropping checks are necessary to avoid letting this function
        // be called from Cc::drop implementation, since it doesn't set is_collecting to true
        assert!(
            state(|state| !state.is_collecting() && !state.is_finalizing() && !state.is_dropping()),
            "Cc::finalize_again cannot be called while collecting"
        );

        self.counter_marker().set_finalized(false);
    }

    /// Returns `true` if the value in the managed allocation has already been finalized, `false` otherwise.
    #[cfg(feature = "finalization")]
    #[inline]
    pub fn already_finalized(&self) -> bool {
        !self.counter_marker().needs_finalization()
    }

    /// Marks the managed allocation as *alive*.
    /// 
    /// Every time a [`Cc`] is dropped, the pointed allocation is buffered to be processed in the next collection.
    /// This method simply removes the managed allocation from the buffer, potentially reducing the amount of work
    /// needed to be done by the collector.
    /// 
    /// This method is a no-op when called on a [`Cc`] pointing to an allocation which is not buffered.
    #[inline]
    pub fn mark_alive(&self) {
        remove_from_list(self.inner.cast());
    }

    #[inline(always)]
    fn counter_marker(&self) -> &CounterMarker {
        &self.inner().counter_marker
    }

    #[inline(always)]
    pub(crate) fn inner(&self) -> &CcBox<T> {
        unsafe { self.inner.as_ref() }
    }

    #[cfg(feature = "weak-ptrs")]
    #[inline(always)]
    pub(crate) fn inner_ptr(&self) -> NonNull<CcBox<T>> {
        self.inner
    }

    #[cfg(feature = "weak-ptrs")] // Currently used only here
    #[inline(always)]
    #[must_use]
    pub(crate) fn __new_internal(inner: NonNull<CcBox<T>>) -> Cc<T> {
        Cc {
            inner,
            _phantom: PhantomData,
        }
    }
}

impl<T: ?Sized + Trace> Clone for Cc<T> {
    /// Makes a clone of the [`Cc`] pointer.
    /// 
    /// This creates another pointer to the same allocation, increasing the strong reference count.
    /// 
    /// Cloning a [`Cc`] also marks the managed allocation as `alive`. See [`mark_alive`][`Cc::mark_alive`] for more details.
    ///
    /// # Panics
    ///
    /// Panics if the strong reference count exceeds the maximum supported.
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

        self.mark_alive();

        // It's always safe to clone a Cc
        Cc {
            inner: self.inner,
            _phantom: PhantomData,
        }
    }
}

impl<T: ?Sized + Trace> Deref for Cc<T> {
    type Target = T;

    #[inline]
    #[track_caller]
    fn deref(&self) -> &Self::Target {
        #[cfg(debug_assertions)]
        if state(|state| state.is_tracing()) {
            panic!("Cannot deref while tracing!");
        }

        //self.mark_alive();

        self.inner().get_elem()
    }
}

impl<T: ?Sized + Trace> Drop for Cc<T> {
    fn drop(&mut self) {
        #[cfg(debug_assertions)]
        if state(|state| state.is_tracing()) {
            panic!("Cannot drop while tracing!");
        }

        #[inline]
        fn decrement_counter<T: ?Sized + Trace>(cc: &Cc<T>) {
            // Always decrement the counter
            let res = cc.counter_marker().decrement_counter();
            debug_assert!(res.is_ok());
        }

        #[inline]
        fn handle_possible_cycle<T: ?Sized + Trace>(cc: &Cc<T>) {
            decrement_counter(cc);

            // We know that we're not part of either root_list or non_root_list, since the cc isn't traced
            add_to_list(cc.inner.cast());
        }

        // A CcBox can be marked traced only during collections while being into a list different than POSSIBLE_CYCLES.
        // In this case, no further action has to be taken, except decrementing the reference counter.
        // Skip the rest of the code also when the value has already been dropped
        if self.counter_marker().is_traced_or_dropped() {
            decrement_counter(self);
            return;
        }

        if self.counter_marker().counter() == 1 {
            // Only us have a pointer to this allocation, deallocate!

            state(|state| {
                #[cfg(feature = "finalization")]
                if self.counter_marker().needs_finalization() {
                    let _finalizing_guard = replace_state_field!(finalizing, true, state);

                    // Set finalized
                    self.counter_marker().set_finalized(true);

                    self.inner().get_elem().finalize();

                    if self.counter_marker().counter() != 1 {
                        // The object has been resurrected
                        handle_possible_cycle(self);
                        return;
                    }
                    // _finalizing_guard is dropped here, resetting state.finalizing
                }

                decrement_counter(self);
                remove_from_list(self.inner.cast());

                let _dropping_guard = replace_state_field!(dropping, true, state);
                let layout = self.inner().layout();

                #[cfg(feature = "weak-ptrs")]
                {
                    // Set the object as dropped before dropping and deallocating it
                    // This feature is used only in weak pointers, so do this only if they're enabled
                    self.counter_marker().mark(Mark::Dropped);

                    self.inner().drop_metadata();
                }

                // SAFETY: we're the only one to have a pointer to this allocation
                unsafe {
                    drop_in_place(self.inner().get_elem_mut());

                    #[cfg(feature = "pedantic-debug-assertions")]
                    debug_assert_eq!(
                        0, self.counter_marker().counter(),
                        "Trying to deallocate a CcBox with a reference counter > 0"
                    );

                    cc_dealloc(self.inner, layout, state);
                }
                // _dropping_guard is dropped here, resetting state.dropping
            });
        } else {
            handle_possible_cycle(self);
        }
    }
}

unsafe impl<T: ?Sized + Trace> Trace for Cc<T> {
    #[inline]
    #[track_caller]
    fn trace(&self, ctx: &mut Context<'_>) {
        if CcBox::trace(self.inner.cast(), ctx) {
            self.inner().get_elem().trace(ctx);
        }
    }
}

impl<T: ?Sized + Trace> Finalize for Cc<T> {}

#[repr(C)]
pub(crate) struct CcBox<T: ?Sized + Trace + 'static> {
    next: UnsafeCell<Option<NonNull<CcBox<()>>>>,
    prev: UnsafeCell<Option<NonNull<CcBox<()>>>>,

    metadata: Cell<Metadata>,

    counter_marker: CounterMarker,
    _phantom: PhantomData<Rc<()>>, // Make CcBox !Send and !Sync

    // This UnsafeCell is necessary, since we want to execute Drop::drop (which takes an &mut)
    // for elem but still have access to the other fields of CcBox
    elem: UnsafeCell<T>,
}

impl<T: Trace> CcBox<T> {
    #[inline(always)]
    #[must_use]
    fn new(t: T, state: &State) -> NonNull<CcBox<T>> {
        let layout = Layout::new::<CcBox<T>>();

        #[cfg(feature = "finalization")]
        let already_finalized = state.is_finalizing();
        #[cfg(not(feature = "finalization"))]
        let already_finalized = false;

        unsafe {
            let ptr: NonNull<CcBox<T>> = cc_alloc(layout, state);
            ptr::write(
                ptr.as_ptr(),
                CcBox {
                    next: UnsafeCell::new(None),
                    prev: UnsafeCell::new(None),
                    metadata: Metadata::new(ptr),
                    counter_marker: CounterMarker::new_with_counter_to_one(already_finalized),
                    _phantom: PhantomData,
                    elem: UnsafeCell::new(t),
                },
            );
            ptr
        }
    }

    #[inline(always)]
    #[cfg(all(test, feature = "std"))] // Only used in unit tests
    #[must_use]
    pub(crate) fn new_for_tests(t: T) -> NonNull<CcBox<T>> {
        state(|state| CcBox::new(t, state))
    }
}

impl<T: ?Sized + Trace> CcBox<T> {
    #[inline]
    pub(crate) fn get_elem(&self) -> &T {
        unsafe { &*self.elem.get() }
    }

    #[inline]
    pub(crate) fn get_elem_mut(&self) -> *mut T {
        self.elem.get()
    }

    #[inline]
    pub(crate) fn counter_marker(&self) -> &CounterMarker {
        &self.counter_marker
    }

    #[inline]
    pub(crate) fn layout(&self) -> Layout {
        #[cfg(feature = "nightly")]
        {
            self.vtable().vtable.layout()
        }

        #[cfg(not(feature = "nightly"))]
        unsafe {
            Layout::for_value(self.vtable().fat_ptr.as_ref())
        }
    }

    #[inline]
    fn vtable(&self) -> VTable {
        #[cfg(feature = "weak-ptrs")]
        unsafe {
            if self.counter_marker.has_allocated_for_metadata() {
                self.metadata.get().boxed_metadata.as_ref().vtable
            } else {
                self.metadata.get().vtable
            }
        }

        #[cfg(not(feature = "weak-ptrs"))]
        unsafe {
            self.metadata.get().vtable
        }
    }

    #[cfg(feature = "weak-ptrs")]
    #[inline]
    pub(crate) fn get_or_init_metadata(&self) -> NonNull<BoxedMetadata> {
        unsafe {
            if self.counter_marker.has_allocated_for_metadata() {
                self.metadata.get().boxed_metadata
            } else {
                let vtable = self.metadata.get().vtable;
                let ptr = BoxedMetadata::new(vtable, WeakCounterMarker::new(true));
                self.metadata.set(Metadata {
                    boxed_metadata: ptr,
                });
                self.counter_marker.set_allocated_for_metadata(true);
                ptr
            }
        }
    }

    /// # Safety
    /// The metadata must have been allocated.
    #[cfg(feature = "weak-ptrs")]
    #[inline(always)]
    pub(crate) unsafe fn get_metadata_unchecked(&self) -> NonNull<BoxedMetadata> {
        self.metadata.get().boxed_metadata
    }

    #[cfg(feature = "weak-ptrs")]
    #[inline]
    pub(crate) fn drop_metadata(&self) {
        if self.counter_marker.has_allocated_for_metadata() {
            unsafe {
                let boxed = self.get_metadata_unchecked();
                if boxed.as_ref().weak_counter_marker.counter() == 0 {
                    // There are no weak pointers, deallocate the metadata
                    dealloc_other(boxed);
                } else {
                    // There exist weak pointers, set the CcBox allocation not accessible
                    boxed.as_ref().weak_counter_marker.set_accessible(false);
                }
            }
        }
    }

    #[inline]
    pub(super) fn get_next(&self) -> *mut Option<NonNull<CcBox<()>>> {
        self.next.get()
    }

    #[inline]
    pub(super) fn get_prev(&self) -> *mut Option<NonNull<CcBox<()>>> {
        self.prev.get()
    }
}

unsafe impl<T: ?Sized + Trace> Trace for CcBox<T> {
    #[inline(always)]
    fn trace(&self, ctx: &mut Context<'_>) {
        self.get_elem().trace(ctx);
    }
}

impl<T: ?Sized + Trace> Finalize for CcBox<T> {
    #[inline(always)]
    fn finalize(&self) {
        self.get_elem().finalize();
    }
}

#[inline]
pub(crate) fn remove_from_list(ptr: NonNull<CcBox<()>>) {
    let counter_marker = unsafe { ptr.as_ref() }.counter_marker();

    // Check if ptr is in possible_cycles list
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
        // This is safe to do since we're not putting the CcBox into the list
        #[cfg(feature = "pedantic-debug-assertions")]
        debug_assert! {
            POSSIBLE_CYCLES.try_with(|pc| {
                !pc.borrow().contains(ptr)
            }).unwrap_or(true)
        };
    }
}

#[inline]
pub(crate) fn add_to_list(ptr: NonNull<CcBox<()>>) {
    let counter_marker = unsafe { ptr.as_ref() }.counter_marker();

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
        // Make sure this operation is the first after the if-else, since the CcBox is in
        // an invalid state now (it's marked Mark::PossibleCycles, but it isn't into the list)
        list.add(ptr);
    });
}

// Functions in common between every CcBox<_>
impl CcBox<()> {
    #[inline]
    pub(super) fn trace_inner(ptr: NonNull<Self>, ctx: &mut Context<'_>) {
        unsafe {
            CcBox::get_traceable(ptr).as_ref().trace(ctx);
        }
    }

    #[cfg(feature = "finalization")]
    #[inline]
    pub(super) fn finalize_inner(ptr: NonNull<Self>) -> bool {
        unsafe {
            if ptr.as_ref().counter_marker().needs_finalization() {
                // Set finalized
                ptr.as_ref().counter_marker().set_finalized(true);

                CcBox::get_traceable(ptr).as_ref().finalize_elem();
                true
            } else {
                false
            }
        }
    }

    /// SAFETY: `drop_in_place` conditions must be true.
    #[inline]
    pub(super) unsafe fn drop_inner(ptr: NonNull<Self>) {
        #[cfg(feature = "weak-ptrs")]
        {
            // Set the object as dropped before dropping it
            // This feature is used only in weak pointers, so do this only if they're enabled
            ptr.as_ref().counter_marker().mark(Mark::Dropped);
        }

        CcBox::get_traceable(ptr).as_mut().drop_elem();
    }

    #[inline]
    fn get_traceable(ptr: NonNull<Self>) -> NonNull<dyn InternalTrace> {
        #[cfg(feature = "nightly")]
        unsafe {
            let vtable = ptr.as_ref().vtable().vtable;
            NonNull::from_raw_parts(ptr.cast(), vtable)
        }

        #[cfg(not(feature = "nightly"))]
        unsafe {
            ptr.as_ref().vtable().fat_ptr
        }
    }

    pub(super) fn start_tracing(ptr: NonNull<Self>, ctx: &mut Context<'_>) {
        let counter_marker = unsafe { ptr.as_ref() }.counter_marker();
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
        // exact type of the element inside CcBox, so trace it using the vtable
        CcBox::trace_inner(ptr, ctx);
    }

    /// Returns whether `ptr.elem` should be traced.
    ///
    /// This function returns a `bool` instead of directly tracing the element inside the CcBox, since this way
    /// we can avoid using the vtable most of the times (the responsibility of tracing the inner element is passed
    /// to the caller, which *might* have more information on the type inside CcBox than us).
    #[inline(never)] // Don't inline this function, it's huge
    #[must_use = "the element inside ptr is not traced by CcBox::trace"]
    fn trace(ptr: NonNull<Self>, ctx: &mut Context<'_>) -> bool {
        #[inline(always)]
        fn non_root(counter_marker: &CounterMarker) -> bool {
            counter_marker.tracing_counter() == counter_marker.counter()
        }

        let counter_marker = unsafe { ptr.as_ref() }.counter_marker();
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
                    // NonMarked will avoid tracing this CcBox again (thanks to the if condition)
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

#[derive(Copy, Clone)]
union Metadata {
    vtable: VTable,
    #[cfg(feature = "weak-ptrs")]
    boxed_metadata: NonNull<BoxedMetadata>,
}

impl Metadata {
    #[inline]
    fn new<T: Trace>(cc_box: NonNull<CcBox<T>>) -> Cell<Metadata> {
        #[cfg(feature = "nightly")]
        let vtable = VTable {
            vtable: metadata(cc_box.as_ptr() as *mut dyn InternalTrace),
        };

        #[cfg(not(feature = "nightly"))]
        let vtable = VTable {
            fat_ptr: unsafe { // SAFETY: the ptr comes from a NotNull ptr
                NonNull::new_unchecked(cc_box.as_ptr() as *mut dyn InternalTrace)
            },
        };

        Cell::new(Metadata {
            vtable
        })
    }
}

#[derive(Copy, Clone)]
struct VTable {
    #[cfg(feature = "nightly")]
    vtable: DynMetadata<dyn InternalTrace>,

    #[cfg(not(feature = "nightly"))]
    fat_ptr: NonNull<dyn InternalTrace>,
}

#[cfg(feature = "weak-ptrs")]
pub(crate) struct BoxedMetadata {
    vtable: VTable,
    pub(crate) weak_counter_marker: WeakCounterMarker,
}

#[cfg(feature = "weak-ptrs")]
impl BoxedMetadata {
    #[inline]
    fn new(vtable: VTable, weak_counter_marker: WeakCounterMarker) -> NonNull<BoxedMetadata> {
        unsafe {
            let ptr: NonNull<BoxedMetadata> = alloc_other();
            ptr::write(
                ptr.as_ptr(),
                BoxedMetadata {
                    vtable,
                    weak_counter_marker,
                },
            );
            ptr
        }
    }
}

// Trait used to make it possible to drop/finalize only the elem field of CcBox
// and without taking a &mut reference to the whole CcBox
trait InternalTrace: Trace {
    #[cfg(feature = "finalization")]
    fn finalize_elem(&self);

    /// Safety: see `drop_in_place`
    unsafe fn drop_elem(&self);
}

impl<T: ?Sized + Trace> InternalTrace for CcBox<T> {
    #[cfg(feature = "finalization")]
    fn finalize_elem(&self) {
        self.get_elem().finalize();
    }

    unsafe fn drop_elem(&self) {
        drop_in_place(self.get_elem_mut());
    }
}

// ####################################
// #          Cc Trait impls          #
// ####################################

impl<T: ?Sized + Trace + Default> Default for Cc<T> {
    /// Creates a new [`Cc<T>`][`Cc`], with the [`Default`] value for `T`.
    ///
    /// # Collection
    ///
    /// This method may start a collection when the `auto-collect` feature is enabled.
    ///
    /// See the [`config` module documentation][`mod@crate::config`] for more details.
    #[inline]
    fn default() -> Self {
        Cc::new(<T as Default>::default())
    }
}

impl<T: ?Sized + Trace> AsRef<T> for Cc<T> {
    #[inline(always)]
    fn as_ref(&self) -> &T {
        self
    }
}

impl<T: ?Sized + Trace> Borrow<T> for Cc<T> {
    #[inline(always)]
    fn borrow(&self) -> &T {
        self
    }
}

impl<T: Trace> From<T> for Cc<T> {
    /// Converts a generic `T` into a [`Cc<T>`][`Cc`].
    ///
    /// # Collection
    ///
    /// This method may start a collection when the `auto-collect` feature is enabled.
    ///
    /// See the [`config` module documentation][`mod@crate::config`] for more details.
    #[inline(always)]
    fn from(value: T) -> Self {
        Cc::new(value)
    }
}

// TODO impl From<Box<T>> for Cc<T>
// TODO impl TryFrom<T> for Cc<T> when Cc::try_new will be implemented

impl<T: ?Sized + Trace + Debug> Debug for Cc<T> {
    #[inline]
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        Debug::fmt(&**self, f)
    }
}

impl<T: ?Sized + Trace + Display> Display for Cc<T> {
    #[inline]
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        Display::fmt(&**self, f)
    }
}

impl<T: ?Sized + Trace> Pointer for Cc<T> {
    #[inline]
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        Pointer::fmt(&ptr::addr_of!(**self), f)
    }
}

impl<T: ?Sized + Trace + PartialEq> PartialEq for Cc<T> {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        **self == **other
    }
}

impl<T: ?Sized + Trace + Eq> Eq for Cc<T> {}

impl<T: ?Sized + Trace + Ord> Ord for Cc<T> {
    #[inline]
    fn cmp(&self, other: &Self) -> Ordering {
        (**self).cmp(&**other)
    }
}

impl<T: ?Sized + Trace + PartialOrd> PartialOrd for Cc<T> {
    #[inline]
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        (**self).partial_cmp(&**other)
    }

    #[inline]
    fn lt(&self, other: &Self) -> bool {
        **self < **other
    }

    #[inline]
    fn le(&self, other: &Self) -> bool {
        **self <= **other
    }

    #[inline]
    fn gt(&self, other: &Self) -> bool {
        **self > **other
    }

    #[inline]
    fn ge(&self, other: &Self) -> bool {
        **self >= **other
    }
}

impl<T: ?Sized + Trace + Hash> Hash for Cc<T> {
    #[inline]
    fn hash<H: Hasher>(&self, state: &mut H) {
        (**self).hash(state);
    }
}

impl<T: ?Sized + Trace + UnwindSafe> UnwindSafe for Cc<T> {}

impl<T: ?Sized + Trace + RefUnwindSafe> RefUnwindSafe for Cc<T> {}
