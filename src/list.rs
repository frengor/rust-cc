use std::mem;
use std::ptr::NonNull;

use crate::{CcOnHeap, Mark};

pub(crate) struct List {
    first: Option<NonNull<CcOnHeap<()>>>,
}

impl List {
    #[inline]
    pub(crate) fn new() -> List {
        List { first: None }
    }

    #[inline]
    pub(crate) fn first(&self) -> Option<NonNull<CcOnHeap<()>>> {
        self.first
    }

    #[inline]
    pub(crate) fn add(&mut self, ptr: NonNull<CcOnHeap<()>>) {
        // Check if ptr can be added safely
        unsafe { debug_assert!(ptr.as_ref().is_valid()) };

        if let Some(first) = &mut self.first {
            unsafe {
                *first.as_ref().get_prev() = Some(ptr);
                *ptr.as_ref().get_next() = Some(*first);
                *ptr.as_ref().get_prev() = None; // Not really necessary
            }
            *first = ptr;
        } else {
            self.first = Some(ptr);
            unsafe {
                // Not really necessary
                *ptr.as_ref().get_next() = None;
                *ptr.as_ref().get_prev() = None;
            }
        }
    }

    #[inline]
    pub(crate) fn remove(&mut self, ptr: NonNull<CcOnHeap<()>>) {
        // Make sure ptr is valid. Since a pointer to an invalid CcOnHeap<_> cannot
        // be added to any list, if ptr is invalid then this fn shouldn't have been called
        unsafe { debug_assert!(ptr.as_ref().is_valid()) };

        // Remove from possible_cycles list
        unsafe {
            match (*ptr.as_ref().get_next(), *ptr.as_ref().get_prev()) {
                (Some(next), Some(prev)) => {
                    // ptr is in between two elements
                    *next.as_ref().get_prev() = Some(prev);
                    *prev.as_ref().get_next() = Some(next);
                },
                (Some(next), None) => {
                    // ptr is the first element
                    *next.as_ref().get_prev() = None;
                    self.first = Some(next);
                },
                (None, Some(prev)) => {
                    // ptr is the last element
                    *prev.as_ref().get_next() = None;
                },
                (None, None) => {
                    // ptr is the only one in the list
                    self.first = None;
                },
            }
            *ptr.as_ref().get_next() = None;
            *ptr.as_ref().get_prev() = None;
        }
    }

    #[inline]
    pub(crate) fn is_empty(&self) -> bool {
        self.first.is_none()
    }

    #[inline]
    pub(crate) fn for_each(&self, mut f: impl FnMut(NonNull<CcOnHeap<()>>)) {
        let mut current = self.first;
        while let Some(ptr) = current {
            unsafe {
                current = *ptr.as_ref().get_next();
            }
            f(ptr);
        }
    }

    #[inline]
    pub(crate) fn for_each_clearing(mut self, mut f: impl FnMut(NonNull<CcOnHeap<()>>)) {
        // Using self.first as tmp variable since if a call to f panics,
        // then the List's Drop implementation will take care of it
        while let Some(ptr) = self.first {
            unsafe {
                // Adjust next/prev pointers before running f in order to avoid accessing ptr
                // after calling f, which may have deallocated the CcOnHeap pointed by ptr.
                // Calling f as the last operation also avoids corner-cases when f panics
                self.first = *ptr.as_ref().get_next();
                *ptr.as_ref().get_next() = None;
                *ptr.as_ref().get_prev() = None;
            }
            f(ptr);
        }
        mem::forget(self); // The list is already empty, there's no need to run List's destructor
    }

    #[inline]
    pub(crate) fn contains(&self, ptr: NonNull<CcOnHeap<()>>) -> bool {
        let mut current = self.first;
        while let Some(current_ptr) = current {
            if ptr == current_ptr {
                return true;
            }
            unsafe {
                current = *current_ptr.as_ref().get_next();
            }
        }
        false
    }
}

impl Drop for List {
    fn drop(&mut self) {
        // The if condition should be true only when a panic occurred (or when the thread locals are dropped)
        if let Some(ptr) = self.first {
            #[inline(always)]
            fn remove_elem(ptr: NonNull<CcOnHeap<()>>) -> Option<NonNull<CcOnHeap<()>>> {
                unsafe {
                    // Reset the mark to avoid having an inconsistent CcOnHeap
                    ptr.as_ref().counter_marker().mark(Mark::NonMarked);
                    let next = *ptr.as_ref().get_next();
                    *ptr.as_ref().get_next() = None;
                    *ptr.as_ref().get_prev() = None;
                    next
                }
            }

            self.first = remove_elem(ptr);

            // Remove the remaining elements from the list
            while let Some(ptr) = self.first {
                self.first = remove_elem(ptr);
            }
        }
    }
}
