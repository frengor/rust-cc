use std::marker::PhantomData;
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
    #[cfg(test)] // Only used in tests
    pub(crate) fn first(&self) -> Option<NonNull<CcOnHeap<()>>> {
        self.first
    }

    #[inline]
    pub(crate) fn add(&mut self, ptr: NonNull<CcOnHeap<()>>) {
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
    pub(crate) fn remove_first(&mut self) -> Option<NonNull<CcOnHeap<()>>> {
        match self.first {
            Some(first) => unsafe {
                self.first = *first.as_ref().get_next();
                if let Some(next) = self.first {
                    *next.as_ref().get_prev() = None;
                }
                *first.as_ref().get_next() = None;
                // prev is already None since it's the first element

                // Make sure the mark is correct
                first.as_ref().counter_marker().mark(Mark::NonMarked);

                Some(first)
            },
            None => {
                None
            },
        }
    }

    #[inline]
    pub(crate) fn is_empty(&self) -> bool {
        self.first.is_none()
    }

    #[inline]
    #[allow(unused)]
    pub(crate) fn contains(&self, ptr: NonNull<CcOnHeap<()>>) -> bool {
        self.iter().any(|elem| elem == ptr)
    }

    #[inline]
    pub(crate) fn iter(&self) -> Iter {
        self.into_iter()
    }

    #[inline]
    #[cfg(test)] // Only used in tests
    pub(crate) fn into_iter(self) -> ListIter {
        <Self as IntoIterator>::into_iter(self)
    }

    /// The elements in `to_append` are assumed to be already marked with `mark` mark.
    #[inline]
    #[cfg(feature = "finalization")]
    pub(crate) fn mark_self_and_append(&mut self, mark: Mark, to_append: List) {
        if let Some(mut prev) = self.first {
            for elem in self.iter() {
                unsafe {
                    elem.as_ref().counter_marker().mark(mark);
                }
                prev = elem;
            }
            unsafe {
                *prev.as_ref().get_next() = to_append.first;
                if let Some(ptr) = to_append.first {
                    *ptr.as_ref().get_prev() = Some(prev);
                }
            }
        } else {
            self.first = to_append.first;
            // to_append.first.prev is already None
        }
        std::mem::forget(to_append); // Don't run to_append destructor
    }
}

impl Drop for List {
    #[inline]
    fn drop(&mut self) {
        // Remove the remaining elements from the list
        while self.remove_first().is_some() {
            // remove_first already mark every removed element NonMarked
        }
    }
}

impl<'a> IntoIterator for &'a List {
    type Item = NonNull<CcOnHeap<()>>;
    type IntoIter = Iter<'a>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        Iter {
            next: self.first,
            _phantom: PhantomData,
        }
    }
}

impl IntoIterator for List {
    type Item = NonNull<CcOnHeap<()>>;
    type IntoIter = ListIter;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        ListIter {
            list: self,
        }
    }
}

pub(crate) struct Iter<'a> {
    next: Option<NonNull<CcOnHeap<()>>>,
    _phantom: PhantomData<&'a CcOnHeap<()>>,
}

impl<'a> Iterator for Iter<'a> {
    type Item = NonNull<CcOnHeap<()>>;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        match self.next {
            Some(ptr) => {
                unsafe {
                    self.next = *ptr.as_ref().get_next();
                }
                Some(ptr)
            },
            None => {
                None
            },
        }
    }
}

pub(crate) struct ListIter {
    list: List,
}

impl Iterator for ListIter {
    type Item = NonNull<CcOnHeap<()>>;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        self.list.remove_first()
    }
}
