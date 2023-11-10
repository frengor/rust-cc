use std::alloc::{alloc, dealloc, handle_alloc_error, Layout};
use std::ptr::NonNull;

use crate::{CcOnHeap, Trace};
use crate::state::State;

#[inline]
pub(crate) unsafe fn cc_alloc<T: Trace + 'static>(layout: Layout, state: &State) -> NonNull<CcOnHeap<T>> {
    state.record_allocation(layout);
    match NonNull::new(alloc(layout) as *mut CcOnHeap<T>) {
        Some(ptr) => ptr,
        None => handle_alloc_error(layout),
    }
}

#[inline]
pub(crate) unsafe fn cc_dealloc<T: ?Sized + Trace + 'static>(
    ptr: NonNull<CcOnHeap<T>>,
    layout: Layout,
    state: &State
) {
    state.record_deallocation(layout);
    dealloc(ptr.cast().as_ptr(), layout);
}

#[inline(always)]
#[cold]
pub(crate) fn cold() {}
