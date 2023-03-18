#![cfg_attr(feature = "nightly", feature(unsize, coerce_unsized, ptr_metadata))]
#![deny(rustdoc::broken_intra_doc_links)]

use std::cell::RefCell;
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

    collect();

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
        collect();
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

    loop {
        let ptr = POSSIBLE_CYCLES.try_with(|pc| {
            let mut pc = pc.borrow_mut();
            if let Some(first) = pc.first() {
                pc.remove(first);

                unsafe {
                    first.as_ref().counter_marker().mark(Mark::NonMarked); // Keep invariant
                }

                Some(first)
            } else {
                None
            }
        });
        if let Ok(Some(ptr)) = ptr {
            let mut root_list = List::new();
            let mut non_root_list = List::new();

            // SAFETY: ptr comes from POSSIBLE_CYCLES list, so it is surely valid since lists contain only pointers to valid CcOnHeap<_>
            unsafe {
                trace_counting(ptr, &mut root_list, &mut non_root_list);
            }

            trace_roots(&mut root_list, &mut non_root_list);

            root_list.for_each_clearing(|ptr| {
                let counter_marker = unsafe { ptr.as_ref().counter_marker() };

                // Reset mark
                counter_marker.mark(Mark::NonMarked);

                debug_assert_ne!(
                    counter_marker.tracing_counter(),
                    counter_marker.counter()
                );
            });

            if !non_root_list.is_empty() {
                non_root_list.for_each(|ptr| {
                    let counter_marker = unsafe { ptr.as_ref().counter_marker() };

                    debug_assert_eq!(
                        counter_marker.tracing_counter(),
                        counter_marker.counter()
                    );
                    counter_marker.mark(Mark::TraceRoots);
                });

                let mut has_finalized = false;
                {
                    let _finalizing_guard = replace_state_field!(finalizing, true);

                    non_root_list.for_each(|ptr| {
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
                        non_root_list.for_each_clearing(|ptr| {
                            unsafe {
                                ptr.as_ref().counter_marker().mark(Mark::PossibleCycles);
                            }
                            pc.add(ptr);
                        });
                    });
                }
            }
        } else {
            // If POSSIBLE_CYCLES is empty or inaccessible then stop collection
            break;
        }
    }

    // _drop_guard is dropped here, setting state.collecting to false
}

#[inline]
fn deallocate_list(to_deallocate_list: List) {
    let _dropping_guard = replace_state_field!(dropping, true);

    // Drop every CcOnHeap before deallocating them (see comment below)
    to_deallocate_list.for_each(|ptr| {
        // SAFETY: ptr comes from non_root_list, so it is surely valid since lists contain only pointers to valid CcOnHeap<_>.
        //         Also, it's valid to drop in place ptr
        unsafe { CcOnHeap::drop_inner(ptr.cast()) };

        // Don't deallocate now since next drop_inner calls will probably access this object while executing drop glues
    });

    to_deallocate_list.for_each_clearing(|ptr| {
        // SAFETY: ptr.as_ref().elem is never read or written (only the layout information is read)
        //         and then the allocation gets deallocated immediately after
        unsafe {
            let layout = ptr.as_ref().layout();
            cc_dealloc(ptr, layout);
        }
    });

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
    root_list.for_each(|ptr| {
        // SAFETY: ptr comes from a list, so it is surely valid since lists contain only pointers to valid CcOnHeap<_>
        unsafe {
            CcOnHeap::start_tracing(ptr, &mut ctx);
        }
    });
}
