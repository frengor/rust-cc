//! A fast garbage collector based on cycle collection for Rust programs.
//!
//! This crate provides a [`Cc`] (Cycle Collected) smart pointer, which is basically a [`Rc`] which automatically detects and
//! deallocates reference cycles. If there are no reference cycles, then [`Cc`] behaves like [`Rc`] and deallocates
//! immediately when the reference counter drops to zero.
//!
//! Currently, the cycle collector is not concurrent. As such, [`Cc`] doesn't implement [`Send`] nor [`Sync`].
//! 
//! ## Examples
//!
//! ### Basic usage
//!
#![cfg_attr(
    feature = "derive",
    doc = r"```rust"
)]
#![cfg_attr(
    not(feature = "derive"),
    doc = r"```rust,ignore"
)]
#![doc = r"# use rust_cc::*;
# use rust_cc_derive::*;
# use std::cell::RefCell;
#[derive(Trace, Finalize)]
struct Data {
    a: Cc<u32>,
    b: RefCell<Option<Cc<Data>>>,
}

// Rc-like API
let my_cc = Cc::new(Data {
    a: Cc::new(42),
    b: RefCell::new(None),
});

let my_cc_2 = my_cc.clone();
let pointed: &Data = &*my_cc_2;
drop(my_cc_2);

// Create a cycle!
*my_cc.b.borrow_mut() = Some(my_cc.clone());

// Here, the allocated Data instance doesn't get immediately deallocated, since there is a cycle.
drop(my_cc);
// We have to call the cycle collector
collect_cycles();
// collect_cycles() is automatically called from time to time when creating new Ccs,
// calling it directly only ensures that a collection is run (like at the end of the program)
```"]
//!
//! The derive macro for the `Finalize` trait generates an empty finalizer. To write custom finalizers implement the `Finalize` trait manually:
//! 
//! ```rust
//!# use rust_cc::*;
//!# struct Data;
//! impl Finalize for Data {
//!     fn finalize(&self) {
//!         // Finalization code called when a Data object is about to be deallocated
//!         // to allow resource clean up (like closing file descriptors, etc)
//!     }
//! }
//! ```
//! 
//! ### Weak pointers
//!
#![cfg_attr(
    feature = "weak-ptrs",
    doc = r"```rust"
)]
#![cfg_attr(
    not(feature = "weak-ptrs"),
    doc = r"```rust,ignore"
)]
#![doc = r"# use rust_cc::*;
# use rust_cc::weak::*;
let cc: Cc<i32> = Cc::new(5);
 
// Obtain a weak pointer
let weak_ptr: Weak<i32> = cc.downgrade();
 
// Upgrading a weak pointer cannot fail if the pointed allocation isn't deallocated
let upgraded: Option<Cc<i32>> = weak_ptr.upgrade();
assert!(upgraded.is_some());

// Deallocate the object
drop(cc);
drop(upgraded);

// Upgrading now fails
assert!(weak_ptr.upgrade().is_none());
```"]
//!
//! See the [`weak` module documentation][`mod@weak`] for more details.
//! 
//! ### Cleaners
//!
#![cfg_attr(
    all(feature = "cleaners", feature = "derive"),
    doc = r"```rust"
)]
#![cfg_attr(
    not(all(feature = "cleaners", feature = "derive")),
    doc = r"```rust,ignore"
)]
#![doc = r"# use rust_cc::*;
# use rust_cc_derive::*;
# use rust_cc::cleaners::*;
#[derive(Trace, Finalize)]
struct Foo {
    cleaner: Cleaner,
    // ...
}

let foo = Cc::new(Foo {
    cleaner: Cleaner::new(),
    // ...
});

let cleanable = foo.cleaner.register(move || {
    // Cleaning action code
    // Will be called automatically when foo.cleaner is dropped
});

// It's also possible to call the cleaning action manually
cleanable.clean();
```"]
//! 
//! See the [`cleaners` module documentation][`mod@cleaners`] for more details.
//! 
//! [`Send`]: `std::marker::Send`
//! [`Sync`]: `std::marker::Sync`
//! [`Rc`]: `std::rc::Rc`

#![cfg_attr(feature = "nightly", feature(unsize, coerce_unsized, ptr_metadata, derive_smart_pointer))]
#![cfg_attr(all(feature = "nightly", not(feature = "std")), feature(thread_local))] // no-std related unstable features
#![cfg_attr(doc_auto_cfg, feature(doc_auto_cfg))]
#![cfg_attr(not(feature = "std"), no_std)]

#![deny(rustdoc::broken_intra_doc_links)]
#![allow(clippy::thread_local_initializer_can_be_made_const)]

#[cfg(all(not(feature = "std"), not(feature = "nightly")))]
compile_error!("Feature \"std\" cannot be disabled without enabling feature \"nightly\" (due to #[thread_local] not being stable).");

extern crate alloc;

use core::cell::RefCell;
use core::mem;
use core::mem::ManuallyDrop;
use core::ptr::NonNull;
use core::ops::{Deref, DerefMut};

use crate::cc::CcBox;
use crate::counter_marker::Mark;
use crate::list::*;
use crate::state::{replace_state_field, State, try_state};
use crate::trace::ContextInner;
use crate::utils::*;

#[cfg(all(test, feature = "std"))]
mod tests;

mod cc;
mod counter_marker;
mod list;
pub mod state;
mod trace;
mod utils;

#[cfg(feature = "auto-collect")]
pub mod config;

#[cfg(feature = "derive")]
mod derives;

#[cfg(feature = "weak-ptrs")]
pub mod weak;

#[cfg(feature = "cleaners")]
pub mod cleaners;

#[cfg(feature = "derive")]
pub use derives::{Finalize, Trace};

pub use cc::Cc;
pub use trace::{Context, Finalize, Trace};

rust_cc_thread_local! {
    pub(crate) static POSSIBLE_CYCLES: RefCell<CountedList> = RefCell::new(CountedList::new());
}

/// Immediately executes the cycle collection algorithm and collects garbage cycles.
///
/// Calling this function during a collection won't start a new collection.
pub fn collect_cycles() {
    let _ = try_state(|state| {
        if state.is_collecting() {
            return;
        }

        let _ = POSSIBLE_CYCLES.try_with(|pc| {
            collect(state, pc);
        });

        #[cfg(feature = "auto-collect")]
        adjust_trigger_point(state);
    });
}

#[cfg(feature = "auto-collect")]
#[inline(never)]
pub(crate) fn trigger_collection(state: &State) {
    if state.is_collecting() {
        return;
    }

    let _ = POSSIBLE_CYCLES.try_with(|pc| {
        if config::config(|config| config.should_collect(state, pc)).unwrap_or(false) {
            collect(state, pc);

            adjust_trigger_point(state);
        }
    });
}

#[cfg(feature = "auto-collect")]
fn adjust_trigger_point(state: &State) {
    let _ = config::config(|config| config.adjust(state));
}

fn collect(state: &State, possible_cycles: &RefCell<CountedList>) {
    state.set_collecting(true);
    state.increment_executions_count();

    struct DropGuard<'a> {
        state: &'a State,
    }

    impl<'a> Drop for DropGuard<'a> {
        #[inline]
        fn drop(&mut self) {
            self.state.set_collecting(false);
        }
    }

    let _drop_guard = DropGuard { state };

    #[cfg(feature = "finalization")]
    for _ in 0..10 {
        // Limit to 10 executions. A collection usually completes in 2 executions, so passing
        // 10 and still having objects to clean up and finalize almost surely means that some
        // finalizer is doing something weird, like the following:
        //
        // thread_local! { static VEC: RefCell<Vec<Cc<MyStruct>>> = ... }
        // #[derive(Trace)]
        // struct MyStruct { ... }
        // impl Finalize for MyStruct {
        //     fn finalize(&self) {
        //         let _ = VEC.with(|vec| vec.borrow_mut().pop()); // Popping one at a time
        //     }
        // }
        // Insert 100 MyStruct into VEC and then drop one -> 100 executions
        //
        // Thus, it is fine to just leave the remaining objects into POSSIBLE_CYCLES for the
        // next collection execution. The program has already been stopped for too much time.

        if is_empty(possible_cycles) {
            break;
        }

        __collect(state, possible_cycles);
    }
    #[cfg(not(feature = "finalization"))]
    if !is_empty(possible_cycles) {
        __collect(state, possible_cycles);
    }

    // _drop_guard is dropped here, setting state.collecting to false
}

fn __collect(state: &State, possible_cycles: &RefCell<CountedList>) {
    let mut non_root_list = List::new();
    {
        let mut root_list = List::new();

        while let Some(ptr) = get_and_remove_first(possible_cycles) {
            // remove_first already marks ptr as NonMarked
            trace_counting(ptr, &mut root_list, &mut non_root_list);
        }

        trace_roots(root_list, &mut non_root_list);
    }

    if !non_root_list.is_empty() {
        #[cfg(feature = "pedantic-debug-assertions")]
        non_root_list.iter().for_each(|ptr| {
            let counter_marker = unsafe { ptr.as_ref() }.counter_marker();

            debug_assert_eq!(
                counter_marker.tracing_counter(),
                counter_marker.counter()
            );
            debug_assert!(counter_marker.is_traced());
        });

        #[cfg(feature = "finalization")]
        {
            let has_finalized: bool;
            let mut non_root_list_size = 0usize; // Counting the size of non_root only now since it is required by mark_self_and_append
            {
                let _finalizing_guard = replace_state_field!(finalizing, true, state);

                has_finalized = non_root_list.iter().fold(false, |has_finalized, ptr| {
                    non_root_list_size += 1;
                    CcBox::finalize_inner(ptr.cast()) || has_finalized
                });

                // _finalizing_guard is dropped here, resetting state.finalizing
            }

            if !has_finalized {
                deallocate_list(non_root_list, state);
            } else {
                // Put CcBoxes back into the possible cycles list. They will be re-processed in the
                // next iteration of the loop, which will automatically check for resurrected objects
                // using the same algorithm of the initial tracing. This makes it more difficult to
                // create memory leaks accidentally using finalizers than in the previous implementation.
                let mut pc = possible_cycles.borrow_mut();

                // pc is already marked PossibleCycles, while non_root_list is not.
                // non_root_list have to be added to pc after having been marked.
                // It's good here to instead swap the two, mark the pc list (was non_root_list before) and then
                // append the other to it in O(1), since we already know the last element of pc from the marking.
                // This avoids iterating unnecessarily both lists and the need to update many pointers.

                let old_size = pc.size();

                // SAFETY: non_root_list_size is calculated before and it's the size of non_root_list
                unsafe {
                    pc.swap_list(&mut non_root_list, non_root_list_size);
                }
                // SAFETY: swap_list swapped pc and non_root_list, so every element inside non_root_list is already
                //         marked PossibleCycles (because it was pc) and now old_size is the size of non_root_list
                unsafe {
                    pc.mark_self_and_append(Mark::PossibleCycles, non_root_list, old_size);
                }
                drop(pc); // Useless, but better be explicit here in case more code is added below this line
            }
        }

        #[cfg(not(feature = "finalization"))]
        {
            deallocate_list(non_root_list, state);
        }
    }
}

#[inline]
fn is_empty(list: &RefCell<CountedList>) -> bool {
    list.borrow().is_empty()
}

#[inline]
fn get_and_remove_first(list: &RefCell<CountedList>) -> Option<NonNull<CcBox<()>>> {
    list.borrow_mut().remove_first()
}

#[inline]
fn deallocate_list(to_deallocate_list: List, state: &State) {
    /// Just a wrapper used to handle the dropping of to_deallocate_list.
    /// When dropped, the objects inside are set as dropped
    struct ToDropList {
        list: ManuallyDrop<List>,
    }

    impl Deref for ToDropList {
        type Target = List;

        #[inline(always)]
        fn deref(&self) -> &Self::Target {
            &self.list
        }
    }

    impl DerefMut for ToDropList {
        #[inline(always)]
        fn deref_mut(&mut self) -> &mut Self::Target {
            &mut self.list
        }
    }

    impl Drop for ToDropList {
        #[inline]
        fn drop(&mut self) {
            // Remove the elements from the list, setting them as dropped
            // This feature is used only in weak pointers, so do this only if they're enabled
            #[cfg(feature = "weak-ptrs")]
            while let Some(ptr) = self.list.remove_first() {
                // Always set the mark, since it has been cleared by remove_first
                unsafe { ptr.as_ref() }.counter_marker().mark(Mark::Dropped);
            }

            // If not using weak pointers, just call the list's drop implementation
            #[cfg(not(feature = "weak-ptrs"))]
            unsafe {
                ManuallyDrop::drop(&mut self.list);
            }
        }
    }

    let _dropping_guard = replace_state_field!(dropping, true, state);

    // Redefine to_deallocate_list with the ToDropList wrapper
    let to_deallocate_list = ToDropList {
        list: ManuallyDrop::new(to_deallocate_list),
    };

    // Drop every CcBox before deallocating them (see comment below)
    to_deallocate_list.iter().for_each(|ptr| {
        // SAFETY: ptr is valid to access and drop in place
        unsafe {
            debug_assert!(ptr.as_ref().counter_marker().is_traced());

            #[cfg(feature = "weak-ptrs")]
            ptr.as_ref().drop_metadata();

            CcBox::drop_inner(ptr.cast());
        };

        // Don't deallocate now since next drop_inner calls will probably access this object while executing drop glues
    });

    // Don't drop the list now if a panic happens
    // No panic should ever happen, however cc_dealloc could in theory panic if state is not accessible
    // (which should never happen, but better be sure no UB is possible)
    let to_deallocate_list = ManuallyDrop::new(to_deallocate_list);

    to_deallocate_list.iter().for_each(|ptr| {
        #[cfg(feature = "pedantic-debug-assertions")]
        debug_assert_eq!(
            0, unsafe { ptr.as_ref().counter_marker().counter() },
            "Trying to deallocate a CcBox with a reference counter > 0"
        );

        // SAFETY: ptr.as_ref().elem is never read or written (only the layout information is read)
        //         and then the allocation gets deallocated immediately after.
        unsafe {
            let layout = ptr.as_ref().layout();
            cc_dealloc(ptr, layout, state);
        }
    });

    // _dropping_guard is dropped here, resetting state.dropping
}

fn trace_counting(
    ptr: NonNull<CcBox<()>>,
    root_list: &mut List,
    non_root_list: &mut List,
) {
    let mut ctx = Context::new(ContextInner::Counting {
        root_list,
        non_root_list,
    });

    CcBox::start_tracing(ptr, &mut ctx);
}

fn trace_roots(mut root_list: List, non_root_list: &mut List) {
    while let Some(ptr) = root_list.remove_first() {
        let mut ctx = Context::new(ContextInner::RootTracing { non_root_list, root_list: &mut root_list });
        CcBox::start_tracing(ptr, &mut ctx);
    }

    mem::forget(root_list); // root_list is empty, no need run List::drop
}
