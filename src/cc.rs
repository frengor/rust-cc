use alloc::alloc::Layout;
use alloc::rc::Rc;
use core::borrow::Borrow;
use core::cell::Cell;
use core::cell::UnsafeCell;
use core::cmp::Ordering;
use core::fmt::{self, Debug, Display, Formatter, Pointer};
use core::hash::{Hash, Hasher};
use core::marker::PhantomData;
use core::mem::ManuallyDrop;
use core::ops::Deref;
use core::panic::{RefUnwindSafe, UnwindSafe};
use core::ptr::{self, NonNull, drop_in_place};
#[cfg(feature = "nightly")]
use core::{
    marker::CoercePointee,
    ptr::{DynMetadata, metadata},
};

use crate::POSSIBLE_CYCLES;
use crate::counter_marker::{CounterMarker, Mark};
use crate::state::{State, replace_state_field, state, try_state};
use crate::trace::{Context, ContextInner, Finalize, Trace};
use crate::utils::*;
#[cfg(feature = "weak-ptrs")]
use crate::weak::weak_counter_marker::WeakCounterMarker;

/// A thread-local cycle collected pointer.
///
/// See the [module-level documentation][`mod@crate`] for more details.
#[cfg_attr(feature = "nightly", derive(CoercePointee))]
#[repr(transparent)]
pub struct Cc<#[cfg_attr(feature = "nightly", pointee)] T: ?Sized + Trace + 'static> {
    inner: NonNull<CcBox<T>>,
    _phantom: PhantomData<Rc<T>>, // Make Cc !Send and !Sync
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

    /// Returns the inner value, if the [`Cc`] has exactly one strong reference and the collector is not collecting, finalizing or dropping.
    ///
    /// Otherwise, an [`Err`] is returned with the same [`Cc`] this method was called on.
    ///
    /// This will succeed even if there are outstanding weak references.
    #[inline]
    #[track_caller]
    pub fn try_unwrap(self) -> Result<T, Self> {
        let cc = ManuallyDrop::new(self); // Never drop the Cc

        if cc.strong_count() != 1 {
            // cc is not unique
            // No need to access the state here
            return Err(ManuallyDrop::into_inner(cc));
        }

        let res = try_state(|state| {
            if state.is_collecting() || state.is_dropping() {
                return None;
            }

            #[cfg(feature = "finalization")]
            if state.is_finalizing() {
                // To make this method behave the same outside of collections, unwrapping while finalizing is prohibited
                return None;
            }

            remove_from_list(cc.inner.cast());

            // SAFETY: cc is unique
            let t = unsafe { ptr::read(cc.inner().get_elem()) };
            let layout = cc.inner().layout();

            // Disable upgrading weak ptrs (after having read the layout)
            #[cfg(feature = "weak-ptrs")]
            cc.inner().drop_metadata();
            // There's no reason here to call CounterMarker::set_dropped, since the pointed value will not be dropped and the allocation will be freed

            // SAFETY: cc is unique, is not inside any list and no weak pointer can be upgraded
            unsafe {
                cc_dealloc(cc.inner, layout, state);
            }
            Some(t)
        });

        match res {
            Ok(Some(t)) => Ok(t),
            _ => Err(ManuallyDrop::into_inner(cc)),
        }
    }
}

impl<T: ?Sized + Trace> Cc<T> {
    /// Returns `true` if the two [`Cc`]s point to the same allocation. This function ignores the metadata of `dyn Trait` pointers.
    #[inline]
    pub fn ptr_eq(this: &Cc<T>, other: &Cc<T>) -> bool {
        ptr::eq(
            this.inner.as_ptr() as *const (),
            other.inner.as_ptr() as *const (),
        )
    }

    /// Returns the number of [`Cc`]s to the pointed allocation.
    #[inline]
    pub fn strong_count(&self) -> u32 {
        self.counter_marker().counter() as u32
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

        // A CcBox can be in list or queue only during collections while being into a list different than POSSIBLE_CYCLES.
        // In this case, no further action has to be taken, except decrementing the reference counter.
        if self.counter_marker().is_in_list_or_queue() {
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
                    self.counter_marker().set_dropped(true);
                }

                // SAFETY: we're the only one to have a pointer to this allocation
                unsafe {
                    drop_in_place(self.inner().get_elem_mut());

                    #[cfg(feature = "pedantic-debug-assertions")]
                    debug_assert_eq!(
                        0,
                        self.counter_marker().counter(),
                        "Trying to deallocate a CcBox with a reference counter > 0"
                    );

                    // Free metadata (the layout has already been read). Necessary only with weak pointers enabled
                    #[cfg(feature = "weak-ptrs")]
                    self.inner().drop_metadata();

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
        CcBox::trace(self.inner.cast(), ctx);
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
        unsafe { self.metadata.get().boxed_metadata }
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

    if counter_marker.is_in_possible_cycles() {
        let _ = POSSIBLE_CYCLES.try_with(|pc| {
            #[cfg(feature = "pedantic-debug-assertions")]
            debug_assert!(pc.iter().contains(ptr));

            counter_marker.mark(Mark::NonMarked);
            pc.remove(ptr);
        });
    } else {
        #[cfg(feature = "pedantic-debug-assertions")]
        debug_assert! {
            POSSIBLE_CYCLES.try_with(|pc| {
                !pc.iter().contains(ptr)
            }).unwrap_or(true)
        };
    }
}

#[inline]
pub(crate) fn add_to_list(ptr: NonNull<CcBox<()>>) {
    let counter_marker = unsafe { ptr.as_ref() }.counter_marker();

    if !counter_marker.is_in_possible_cycles() {
        let _ = POSSIBLE_CYCLES.try_with(|pc| {
            #[cfg(feature = "pedantic-debug-assertions")]
            debug_assert!(!pc.iter().contains(ptr));

            debug_assert!(counter_marker.is_not_marked());

            pc.add(ptr);
            counter_marker.reset_tracing_counter();
            counter_marker.mark(Mark::PossibleCycles);
        });
    } else {
        // Already in the list

        #[cfg(feature = "pedantic-debug-assertions")]
        debug_assert! {
            POSSIBLE_CYCLES.try_with(|pc| {
                pc.iter().contains(ptr)
            }).unwrap_or(true)
        };
    }
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
            unsafe { ptr.as_ref().counter_marker().set_dropped(true) };
        }

        unsafe { CcBox::get_traceable(ptr).as_mut().drop_elem() };
    }

    #[inline]
    fn get_traceable(ptr: NonNull<Self>) -> NonNull<dyn InternalTrace> {
        #[cfg(feature = "nightly")]
        unsafe {
            let vtable = ptr.as_ref().vtable().vtable;
            NonNull::from_raw_parts(ptr.cast::<()>(), vtable)
        }

        #[cfg(not(feature = "nightly"))]
        unsafe {
            ptr.as_ref().vtable().fat_ptr
        }
    }

    #[inline(never)] // Don't inline this function, it's huge
    fn trace(ptr: NonNull<Self>, ctx: &mut Context<'_>) {
        let counter_marker = unsafe { ptr.as_ref() }.counter_marker();
        match ctx.inner() {
            ContextInner::Counting {
                root_list,
                non_root_list,
                queue,
            } => {
                if counter_marker.is_in_list_or_queue() {
                    // Check counters invariant (tracing_counter is always less or equal to counter)
                    // Only < is used here since tracing_counter will be incremented (by 1)
                    debug_assert!(counter_marker.tracing_counter() < counter_marker.counter());

                    let res = counter_marker.increment_tracing_counter();
                    debug_assert!(res.is_ok());

                    if counter_marker.is_in_list()
                        && counter_marker.counter() == counter_marker.tracing_counter()
                    {
                        // ptr is in root_list

                        #[cfg(feature = "pedantic-debug-assertions")]
                        debug_assert!(root_list.iter().contains(ptr));

                        root_list.remove(ptr);
                        non_root_list.add(ptr);
                    }
                } else {
                    if counter_marker.is_in_possible_cycles() {
                        let res = counter_marker.increment_tracing_counter();
                        debug_assert!(res.is_ok());
                        return;
                    }

                    counter_marker.reset_tracing_counter();
                    let res = counter_marker.increment_tracing_counter();
                    debug_assert!(res.is_ok());

                    queue.add(ptr);
                    counter_marker.mark(Mark::InQueue);
                }
            },
            ContextInner::RootTracing {
                non_root_list,
                queue,
            } => {
                if counter_marker.is_in_list()
                    && counter_marker.counter() == counter_marker.tracing_counter()
                {
                    // ptr is in non_root_list

                    #[cfg(feature = "pedantic-debug-assertions")]
                    debug_assert!(non_root_list.iter().contains(ptr));

                    counter_marker.mark(Mark::NonMarked);
                    non_root_list.remove(ptr);
                    queue.add(ptr);
                    counter_marker.mark(Mark::InQueue);
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
            // SAFETY: the ptr comes from a NotNull ptr
            fat_ptr: unsafe { NonNull::new_unchecked(cc_box.as_ptr() as *mut dyn InternalTrace) },
        };

        Cell::new(Metadata { vtable })
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
        unsafe { drop_in_place(self.get_elem_mut()) };
    }
}

// ####################################
// #          Cc Trait impls          #
// ####################################

impl<T: Trace + Default> Default for Cc<T> {
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
    #[inline]
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
