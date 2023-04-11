#![cfg_attr(feature = "nightly", feature(unsize, coerce_unsized, ptr_metadata))]
#![deny(rustdoc::broken_intra_doc_links)]

use std::cell::RefCell;
use std::mem;
use std::ptr::NonNull;

use crate::cc::CcOnHeap;
use crate::counter_marker::Mark;
use crate::list::List;
use crate::state::{replace_state_field, state, State, try_state};
use crate::trace::ContextInner;
use crate::utils::*;

#[cfg(test)]
mod tests;

mod cc;
mod counter_marker;
mod list;
pub mod state;
mod trace;
mod utils;

#[cfg(feature = "auto-collect")]
pub mod config;

pub use cc::Cc;
pub use trace::{Context, Finalize, Trace};

thread_local! {
    pub(crate) static POSSIBLE_CYCLES: RefCell<List> = RefCell::new(List::new());
}

pub fn collect_cycles() {
    if state(|state| state.is_collecting()) {
        return; // We're already collecting
    }

    while let Ok(false) = POSSIBLE_CYCLES.try_with(|pc| pc.borrow().is_empty()) {
        collect();
    }

    #[cfg(feature = "auto-collect")]
    adjust_trigger_point();
}

#[cfg(feature = "auto-collect")]
#[inline(never)]
pub(crate) fn trigger_collection() {
    let should_collect = state(|state| {
        !state.is_collecting() && config::config(|config| config.should_collect(state)).unwrap_or(false)
    });

    if should_collect {
        while let Ok(false) = POSSIBLE_CYCLES.try_with(|pc| pc.borrow().is_empty()) {
            collect();
        }
        adjust_trigger_point();
    }
}

#[cfg(feature = "auto-collect")]
fn adjust_trigger_point() {
    let _ = config::config(|config| state(|state| config.adjust(state)));
}

fn collect() {
    // Used into try_state
    fn set_collecting(state: &mut State) {
        state.set_collecting(true);
        state.increment_execution_count();
    }

    if try_state(set_collecting).is_err() {
        // If state isn't accessible don't proceed with collection
        return;
    }

    struct DropGuard;

    impl Drop for DropGuard {
        fn drop(&mut self) {
            // Set state.collecting back to to false
            replace_state_field!(__drop_impl DropGuard, set_collecting, false);
        }
    }

    let _drop_guard = DropGuard;

    let mut non_root_list = List::new();
    {
        let mut root_list = List::new();

        while let Some(ptr) = POSSIBLE_CYCLES.with(|pc| pc.borrow_mut().remove_first()) {
            // remove_first already marks ptr as NonMarked

            // SAFETY: ptr comes from POSSIBLE_CYCLES list, so it is surely valid since lists contain only pointers to valid CcOnHeap<_>
            unsafe {
                trace_counting(ptr, &mut root_list, &mut non_root_list);
            }
        }

        trace_roots(&mut root_list, &mut non_root_list);

        #[cfg(feature = "pedantic-debug-assertions")]
        root_list.iter().for_each(|ptr| unsafe {
            let counter_marker = ptr.as_ref().counter_marker();

            debug_assert_ne!(
                counter_marker.tracing_counter(),
                counter_marker.counter()
            );
            debug_assert!(counter_marker.is_marked_trace_roots());
        });
    }

    if !non_root_list.is_empty() {
        #[cfg(feature = "pedantic-debug-assertions")]
        non_root_list.iter().for_each(|ptr| unsafe {
            let counter_marker = ptr.as_ref().counter_marker();

            debug_assert_eq!(
                counter_marker.tracing_counter(),
                counter_marker.counter()
            );
            debug_assert!(counter_marker.is_marked_trace_counting());
        });

        let mut has_finalized = false;
        {
            let _finalizing_guard = replace_state_field!(finalizing, true);

            non_root_list.iter().for_each(|ptr| {
                // SAFETY: ptr comes from non_root_list, so it is surely valid since lists contain only pointers to valid CcOnHeap<_>
                if unsafe { CcOnHeap::finalize_inner(ptr.cast()) } {
                    has_finalized = true;
                }
            });

            // _finalizing_guard is dropped here, resetting state.finalizing
        }

        if !has_finalized {
            deallocate_list(non_root_list);
        } else {
            // Put CcOnHeaps back into the possible cycles list. They will be re-processed in the
            // next iteration of the loop, which will automatically check for resurrected objects
            // using the same algorithm of the initial tracing. This makes it more difficult to
            // create memory leaks accidentally using finalizers than in the previous implementation.
            POSSIBLE_CYCLES.with(|pc| {
                let mut pc = pc.borrow_mut();

                // pc is already marked PossibleCycles, while non_root_list is not.
                // non_root_list have to be added to pc after having been marked.
                // It's good here to instead swap the two, mark the pc list (was non_root_list before) and then
                // append the other to it in O(1), since we already know the last element of pc from the marking.
                // This avoids iterating unnecessarily both lists and the need to update many pointers.
                mem::swap(&mut *pc, &mut non_root_list);
                pc.mark_self_and_append(Mark::PossibleCycles, non_root_list);
            });
        }
    }

    // _drop_guard is dropped here, setting state.collecting to false
}

#[inline]
fn deallocate_list(to_deallocate_list: List) {
    let _dropping_guard = replace_state_field!(dropping, true);

    // Drop every CcOnHeap before deallocating them (see comment below)
    to_deallocate_list.iter().for_each(|ptr| {
        // SAFETY: ptr comes from non_root_list, so it is surely valid since lists contain only pointers to valid CcOnHeap<_>.
        //         Also, it's valid to drop in place ptr
        unsafe { CcOnHeap::drop_inner(ptr.cast()) };

        // Don't deallocate now since next drop_inner calls will probably access this object while executing drop glues
    });

    to_deallocate_list.iter().for_each(|ptr| {
        // SAFETY: ptr.as_ref().elem is never read or written (only the layout information is read)
        //         and then the allocation gets deallocated immediately after
        unsafe {
            let layout = ptr.as_ref().layout();
            cc_dealloc(ptr, layout);
        }
    });

    // Don't remove the mark from dropped CcOnHeaps
    mem::forget(to_deallocate_list);

    // _dropping_guard is dropped here, resetting state.dropping
}

/// SAFETY: ptr must be pointing to a valid CcOnHeap<_>. More formally, `ptr.as_ref().is_valid()` must return `true`.
unsafe fn trace_counting(
    ptr: NonNull<CcOnHeap<()>>,
    root_list: &mut List,
    non_root_list: &mut List,
) {
    let mut ctx = Context::new(ContextInner::Counting {
        root_list,
        non_root_list,
    });

    // SAFETY: ptr is required to be valid
    CcOnHeap::start_tracing(ptr, &mut ctx);
}

fn trace_roots(root_list: &mut List, non_root_list: &mut List) {
    let mut ctx = Context::new(ContextInner::RootTracing { non_root_list });
    root_list.iter().for_each(|ptr| {
        // SAFETY: ptr comes from a list, so it is surely valid since lists contain only pointers to valid CcOnHeap<_>
        unsafe {
            CcOnHeap::start_tracing(ptr, &mut ctx);
        }
    });
}
