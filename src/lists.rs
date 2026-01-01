use core::cell::Cell;
use core::marker::PhantomData;
use core::ptr::NonNull;

use crate::{CcBox, Mark};

pub(crate) struct LinkedList {
    first: Option<NonNull<CcBox<()>>>,
}

impl LinkedList {
    #[inline]
    pub(crate) const fn new() -> Self {
        Self { first: None }
    }

    #[inline]
    pub(crate) fn first(&self) -> Option<NonNull<CcBox<()>>> {
        self.first
    }

    #[inline]
    pub(crate) fn add(&mut self, ptr: NonNull<CcBox<()>>) {
        debug_assert_nones(ptr);

        if let Some(first) = self.first {
            unsafe {
                *first.as_ref().get_prev() = Some(ptr);
                *ptr.as_ref().get_next() = Some(first);
            }
        }

        self.first = Some(ptr);
    }

    #[inline]
    pub(crate) fn remove(&mut self, ptr: NonNull<CcBox<()>>) {
        unsafe {
            match (*ptr.as_ref().get_next(), *ptr.as_ref().get_prev()) {
                (Some(next), Some(prev)) => {
                    // ptr is in between two elements
                    *next.as_ref().get_prev() = Some(prev);
                    *prev.as_ref().get_next() = Some(next);

                    // Both next and prev are != None
                    *ptr.as_ref().get_next() = None;
                    *ptr.as_ref().get_prev() = None;
                },
                (Some(next), None) => {
                    // ptr is the first element
                    *next.as_ref().get_prev() = None;
                    self.first = Some(next);

                    // Only next is != None
                    *ptr.as_ref().get_next() = None;
                },
                (None, Some(prev)) => {
                    // ptr is the last element
                    *prev.as_ref().get_next() = None;

                    // Only prev is != None
                    *ptr.as_ref().get_prev() = None;
                },
                (None, None) => {
                    // ptr is the only one in the list
                    self.first = None;
                },
            }
            debug_assert_nones(ptr);
        }
    }

    #[inline]
    pub(crate) fn remove_first(&mut self) -> Option<NonNull<CcBox<()>>> {
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
            None => None,
        }
    }

    #[inline]
    pub(crate) fn is_empty(&self) -> bool {
        self.first().is_none()
    }

    #[inline]
    pub(crate) fn iter(&self) -> Iter<'_> {
        self.into_iter()
    }
}

impl Drop for LinkedList {
    #[inline]
    fn drop(&mut self) {
        // Remove the remaining elements from the list
        while self.remove_first().is_some() {
            // remove_first already marks every removed element NonMarked
        }
    }
}

impl<'a> IntoIterator for &'a LinkedList {
    type Item = NonNull<CcBox<()>>;
    type IntoIter = Iter<'a>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        Iter {
            next: self.first,
            _phantom: PhantomData,
        }
    }
}

impl IntoIterator for LinkedList {
    type Item = NonNull<CcBox<()>>;
    type IntoIter = ListIter;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        ListIter { list: self }
    }
}

pub(crate) struct Iter<'a> {
    next: Option<NonNull<CcBox<()>>>,
    _phantom: PhantomData<&'a CcBox<()>>,
}

impl Iter<'_> {
    #[inline]
    #[cfg(any(feature = "pedantic-debug-assertions", all(test, feature = "std")))] // Only used in pedantic-debug-assertions or unit tests
    pub(crate) fn contains(mut self, ptr: NonNull<CcBox<()>>) -> bool {
        self.any(|elem| elem == ptr)
    }
}

impl<'a> Iterator for Iter<'a> {
    type Item = NonNull<CcBox<()>>;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        match self.next {
            Some(ptr) => {
                unsafe {
                    self.next = *ptr.as_ref().get_next();
                }
                Some(ptr)
            },
            None => None,
        }
    }
}

pub(crate) struct ListIter {
    list: LinkedList,
}

impl Iterator for ListIter {
    type Item = NonNull<CcBox<()>>;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        self.list.remove_first()
    }
}

pub(crate) struct PossibleCycles {
    first: Cell<Option<NonNull<CcBox<()>>>>,
    size: Cell<usize>,
}

impl PossibleCycles {
    #[inline]
    pub(crate) const fn new() -> Self {
        Self {
            first: Cell::new(None),
            size: Cell::new(0),
        }
    }

    #[inline]
    #[cfg(all(test, feature = "std"))] // Only used in unit tests
    pub(crate) fn reset(&self) {
        self.first.set(None);
        self.size.set(0);
    }

    #[inline]
    pub(crate) fn size(&self) -> usize {
        self.size.get()
    }

    #[inline]
    pub(crate) fn first(&self) -> Option<NonNull<CcBox<()>>> {
        self.first.get()
    }

    #[inline]
    pub(crate) fn add(&self, ptr: NonNull<CcBox<()>>) {
        debug_assert_nones(ptr);

        self.size.set(self.size.get() + 1);

        if let Some(first) = self.first.get() {
            unsafe {
                *first.as_ref().get_prev() = Some(ptr);
                *ptr.as_ref().get_next() = Some(first);
            }
        }

        self.first.set(Some(ptr));
    }

    #[inline]
    pub(crate) fn remove(&self, ptr: NonNull<CcBox<()>>) {
        self.size.set(self.size.get() - 1);

        unsafe {
            match (*ptr.as_ref().get_next(), *ptr.as_ref().get_prev()) {
                (Some(next), Some(prev)) => {
                    // ptr is in between two elements
                    *next.as_ref().get_prev() = Some(prev);
                    *prev.as_ref().get_next() = Some(next);

                    // Both next and prev are != None
                    *ptr.as_ref().get_next() = None;
                    *ptr.as_ref().get_prev() = None;
                },
                (Some(next), None) => {
                    // ptr is the first element
                    *next.as_ref().get_prev() = None;
                    self.first.set(Some(next));

                    // Only next is != None
                    *ptr.as_ref().get_next() = None;
                },
                (None, Some(prev)) => {
                    // ptr is the last element
                    *prev.as_ref().get_next() = None;

                    // Only prev is != None
                    *ptr.as_ref().get_prev() = None;
                },
                (None, None) => {
                    // ptr is the only one in the list
                    self.first.set(None);
                },
            }
            debug_assert_nones(ptr);
        }
    }

    #[inline]
    pub(crate) fn remove_first(&self) -> Option<NonNull<CcBox<()>>> {
        match self.first.get() {
            Some(first) => unsafe {
                self.size.set(self.size.get() - 1);
                let new_first = *first.as_ref().get_next();
                self.first.set(new_first);
                if let Some(next) = new_first {
                    *next.as_ref().get_prev() = None;
                }
                *first.as_ref().get_next() = None;
                // prev is already None since it's the first element

                // Make sure the mark is correct
                first.as_ref().counter_marker().mark(Mark::NonMarked);

                Some(first)
            },
            None => None,
        }
    }

    #[inline]
    pub(crate) fn is_empty(&self) -> bool {
        self.first().is_none()
    }

    /// # Safety
    /// * The elements in `to_append` must be already marked with `mark` mark
    /// * `to_append_size` must be the size of `to_append`
    #[inline]
    #[cfg(feature = "finalization")]
    pub(crate) unsafe fn mark_self_and_append(
        &self,
        mark: Mark,
        to_append: LinkedList,
        to_append_size: usize,
    ) {
        if let Some(mut prev) = self.first.get() {
            for elem in self.iter() {
                unsafe {
                    elem.as_ref().counter_marker().reset_tracing_counter();
                    elem.as_ref().counter_marker().mark(mark);
                }
                prev = elem;
            }
            unsafe {
                if let Some(ptr) = to_append.first {
                    *prev.as_ref().get_next() = to_append.first;
                    *ptr.as_ref().get_prev() = Some(prev);
                }
            }
        } else {
            self.first.set(to_append.first);
            // to_append.first.prev is already None
        }
        self.size.set(self.size.get() + to_append_size);
        core::mem::forget(to_append); // Don't run to_append destructor
    }

    /// # Safety
    /// `to_swap_size` must be the size of `to_swap`.
    #[inline]
    #[cfg(feature = "finalization")]
    pub(crate) unsafe fn swap_list(&self, to_swap: &mut LinkedList, to_swap_size: usize) {
        self.size.set(to_swap_size);
        to_swap.first = self.first.replace(to_swap.first);
    }

    #[inline]
    #[cfg(any(
        feature = "pedantic-debug-assertions",
        feature = "finalization",
        all(test, feature = "std") // Unit tests
    ))]
    pub(crate) fn iter(&self) -> Iter<'_> {
        self.into_iter()
    }
}

impl Drop for PossibleCycles {
    #[inline]
    fn drop(&mut self) {
        // Remove the remaining elements from the list
        while self.remove_first().is_some() {
            // remove_first already marks every removed element NonMarked
        }
    }
}

impl<'a> IntoIterator for &'a PossibleCycles {
    type Item = NonNull<CcBox<()>>;
    type IntoIter = Iter<'a>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        Iter {
            next: self.first.get(),
            _phantom: PhantomData,
        }
    }
}

pub(crate) struct LinkedQueue {
    first: Option<NonNull<CcBox<()>>>,
    last: Option<NonNull<CcBox<()>>>,
}

impl LinkedQueue {
    #[inline]
    pub(crate) const fn new() -> Self {
        Self {
            first: None,
            last: None,
        }
    }

    #[inline]
    pub(crate) fn add(&mut self, ptr: NonNull<CcBox<()>>) {
        debug_assert_nones(ptr);

        if let Some(last) = self.last {
            unsafe {
                *last.as_ref().get_next() = Some(ptr);
            }
        } else {
            self.first = Some(ptr);
        }

        self.last = Some(ptr);
    }

    #[inline]
    pub(crate) fn peek(&self) -> Option<NonNull<CcBox<()>>> {
        self.first
    }

    #[inline]
    pub(crate) fn poll(&mut self) -> Option<NonNull<CcBox<()>>> {
        match self.first {
            Some(first) => unsafe {
                self.first = *first.as_ref().get_next();
                if self.first.is_none() {
                    // The last element is being removed
                    self.last = None;
                }
                *first.as_ref().get_next() = None;

                // Make sure the mark is correct
                first.as_ref().counter_marker().mark(Mark::NonMarked);

                Some(first)
            },
            None => None,
        }
    }

    #[inline]
    pub(crate) fn is_empty(&self) -> bool {
        self.peek().is_none()
    }
}

impl Drop for LinkedQueue {
    #[inline]
    fn drop(&mut self) {
        // Remove the remaining elements from the queue
        while self.poll().is_some() {
            // poll() already marks every removed element NonMarked
        }
    }
}

impl<'a> IntoIterator for &'a LinkedQueue {
    type Item = NonNull<CcBox<()>>;
    type IntoIter = Iter<'a>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        Iter {
            next: self.first,
            _phantom: PhantomData,
        }
    }
}

#[inline(always)] // The fn is always empty in release mode
fn debug_assert_nones(ptr: NonNull<CcBox<()>>) {
    unsafe {
        debug_assert!((*ptr.as_ref().get_next()).is_none());
        debug_assert!((*ptr.as_ref().get_prev()).is_none());
    }
}
