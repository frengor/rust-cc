#![cfg_attr(feature = "nightly", feature(unsize, coerce_unsized, ptr_metadata))]
#![deny(rustdoc::broken_intra_doc_links)]

use std::cell::RefCell;
use std::ptr::NonNull;

use crate::cc::CcOnHeap;
use crate::config::config;
use crate::counter_marker::Mark;
use crate::list::List;
use crate::state::{replace_state_field, state, try_state, State};
use crate::trace::ContextInner;
use crate::utils::*;

#[cfg(test)]
mod tests;

mod cc;
pub mod config;
mod counter_marker;
mod graph;
mod list;
pub mod state;
mod trace;
mod utils;

pub use cc::Cc;
pub use trace::{Context, Finalize, Trace};

use crate::graph::Graph;

thread_local! {
    pub(crate) static POSSIBLE_CYCLES: RefCell<List> = RefCell::new(List::new());
}

pub fn collect_cycles() {
    if state(|state| state.is_collecting()) {
        return; // We're already collecting
    }

    collect();
    adjust_trigger_point();
}

#[inline(never)]
pub(crate) fn trigger_collection() {
    let should_collect = state(|state| {
        !state.is_collecting() && config(|config| config.should_collect(state)).unwrap_or(false)
    });

    if should_collect {
        collect();
        adjust_trigger_point();
    }
}

fn adjust_trigger_point() {
    let _ = config(|config| state(|state| config.adjust(state)));
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
                unsafe { (*first.as_ref().counter_marker()).mark(Mark::NonMarked) }; // Keep invariant
                Some(first)
            } else {
                None
            }
        });
        if let Ok(Some(ptr)) = ptr {
            let mut root_list = List::new();
            let mut non_root_list = List::new();
            let mut graph = Graph::new();

            // SAFETY: ptr comes from POSSIBLE_CYCLES list, so it is surely valid since lists contain only pointers to valid CcOnHeap<_>
            unsafe {
                trace_counting(ptr, &mut root_list, &mut non_root_list, &mut graph);
            }

            trace_roots(&mut root_list, &mut non_root_list, &mut graph);
            root_list.for_each_clearing(|ptr| unsafe {
                // Reset mark
                (*ptr.as_ref().counter_marker()).mark(Mark::NonMarked);

                debug_assert_ne!(
                    ptr.as_ref().get_tracing_counter(),
                    ptr.as_ref().get_counter()
                );
            });

            if !non_root_list.is_empty() {
                unsafe {
                    non_root_list.for_each(|ptr| {
                        debug_assert_eq!(
                            ptr.as_ref().get_tracing_counter(),
                            ptr.as_ref().get_counter()
                        );
                        (*ptr.as_ref().counter_marker()).mark(Mark::TraceRoots);
                    });

                    let mut has_finalized = false;
                    {
                        let _finalizing_guard = replace_state_field!(finalizing, true);

                        non_root_list.for_each(|ptr| {
                            // SAFETY: ptr comes from non_root_list, so it is surely valid since lists contain only pointers to valid CcOnHeap<_>
                            if CcOnHeap::finalize_inner(ptr.cast()) {
                                has_finalized = true;
                            }
                        });

                        // _finalizing_guard is dropped here, resetting state.finalizing
                    }

                    if !has_finalized {
                        deallocate_list(non_root_list);
                    } else {
                        trace_dropping(&mut non_root_list);

                        if trace_resurrecting(&mut non_root_list) {
                            let mut to_deallocate_list = List::new();

                            non_root_list.for_each_clearing(|ptr| {
                                if (*ptr.as_ref().counter_marker()).is_marked_trace_resurrecting() {
                                    // Don't drop it
                                    (*ptr.as_ref().counter_marker()).mark(Mark::NonMarked);
                                } else {
                                    to_deallocate_list.add(ptr);
                                }
                            });

                            deallocate_list(to_deallocate_list);
                        } else {
                            deallocate_list(non_root_list);
                        }
                    }
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
    unsafe {
        let _dropping_guard = replace_state_field!(dropping, true);

        to_deallocate_list.for_each(|ptr| {
            // Drop it
            CcOnHeap::drop_inner(ptr.cast());
            // Don't deallocate now since next drop calls may access this object
        });
        to_deallocate_list.for_each_clearing(|ptr| {
            let layout = ptr.as_ref().layout();
            cc_dealloc(ptr, layout);
        });

        // _dropping_guard is dropped here, resetting state.dropping
    }
}

/// SAFETY: ptr must be pointing to a valid CcOnHeap<_>. More formally, `ptr.as_ref().is_valid()` must return `true`.
unsafe fn trace_counting(
    ptr: NonNull<CcOnHeap<()>>,
    root_list: &mut List,
    non_root_list: &mut List,
    graph: &mut Graph,
) {
    let ctx_inner = RefCell::new(ContextInner::Counting {
        root_list,
        non_root_list,
        graph,
    });
    let mut ctx = Context::new(&ctx_inner, None);
    CcOnHeap::start_tracing(ptr, &mut ctx);
}

fn trace_roots(root_list: &mut List, non_root_list: &mut List, graph: &mut Graph) {
    fn filter(edge: &&NonNull<CcOnHeap<()>>, non_root_list: &mut List) -> bool {
        unsafe {
            let counter_marker = edge.as_ref().counter_marker();
            if !(*counter_marker).is_marked_trace_roots() {
                if !(*counter_marker).is_marked_trace_counting() {
                    // This CcOnHeap hasn't been traced during trace counting, so
                    // don't trace it now since it will surely not be deallocated
                    return false;
                }

                if (*counter_marker).tracing_counter() < (*counter_marker).counter() {
                    (*counter_marker).mark(Mark::TraceRoots);
                    return false;
                } else {
                    (*counter_marker).mark(Mark::NonMarked);
                    non_root_list.remove(**edge);
                    return true;
                }
            }
            false
        }
    }

    // TODO: maybe the allocation(s) for the Vec might be avoidable using a List.
    //       Also, graph.edges could be able to remove the node from the graph before tracing,
    //       since the filter function should make sure that no node will be traced twice
    let mut to_process: Vec<NonNull<CcOnHeap<()>>> = Vec::new();

    root_list.for_each(|root| {
        unsafe {
            (*root.as_ref().counter_marker()).mark(Mark::TraceRoots);
        }

        if let Some(edges) = graph.edges(root) {
            to_process.extend(edges.filter(|edge| filter(edge, non_root_list)));
        }
    });

    while let Some(node) = to_process.pop() {
        if let Some(edges) = graph.edges(node) {
            to_process.extend(edges.filter(|edge| filter(edge, non_root_list)));
        }
    }
}

// TODO: Update trace_dropping and trace_resurrecting to use the new graph algorithm

fn trace_dropping(non_root_list: &mut List) {
    let ctx_inner = RefCell::new(ContextInner::DropTracing);
    let ctx = &mut Context::new(&ctx_inner, None);
    non_root_list.for_each(|ptr| {
        // SAFETY: ptr comes from a list, so it is surely valid since lists contain only pointers to valid CcOnHeap<_>
        unsafe {
            CcOnHeap::start_tracing(ptr, ctx);
        }
    });
}

fn trace_resurrecting(non_root_list: &mut List) -> bool {
    let ctx_inner = RefCell::new(ContextInner::DropResurrecting);
    let mut has_resurrected = false;
    let ctx = &mut Context::new(&ctx_inner, None);
    non_root_list.for_each(|ptr| unsafe {
        if ptr.as_ref().get_tracing_counter() != 0 {
            has_resurrected = true;
            // SAFETY: ptr comes from a list, so it is surely valid since lists contain only pointers to valid CcOnHeap<_>
            CcOnHeap::start_tracing(ptr, ctx);
        }
    });
    has_resurrected
}
