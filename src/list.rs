use core::marker::PhantomData;
use core::ptr::NonNull;

use crate::{CcBox, Mark};

/// Methods shared by lists
pub(crate) trait ListMethods: Sized {
    #[cfg(all(test, feature = "std"))] // Only used in unit tests
    fn first(&self) -> Option<NonNull<CcBox<()>>>;

    fn add(&mut self, ptr: NonNull<CcBox<()>>);

    fn remove(&mut self, ptr: NonNull<CcBox<()>>);

    fn remove_first(&mut self) -> Option<NonNull<CcBox<()>>>;

    fn is_empty(&self) -> bool;

    #[inline]
    #[cfg(any(feature = "pedantic-debug-assertions", all(test, feature = "std")))] // Only used in pedantic-debug-assertions or unit tests
    fn contains(&self, ptr: NonNull<CcBox<()>>) -> bool {
        self.iter().any(|elem| elem == ptr)
    }

    fn iter(&self) -> Iter;

    #[cfg(all(test, feature = "std"))] // Only used in unit tests
    fn into_iter(self) -> ListIter<Self>;
}

pub(crate) struct List {
    first: Option<NonNull<CcBox<()>>>,
}

impl List {
    #[inline]
    pub(crate) const fn new() -> List {
        List { first: None }
    }
}

impl ListMethods for List {
    #[inline]
    #[cfg(all(test, feature = "std"))] // Only used in unit tests
    fn first(&self) -> Option<NonNull<CcBox<()>>> {
        self.first
    }

    #[inline]
    fn add(&mut self, ptr: NonNull<CcBox<()>>) {
        if let Some(first) = &mut self.first {
            unsafe {
                *first.as_ref().get_prev() = Some(ptr);
                *ptr.as_ref().get_next() = Some(*first);
                debug_assert!((*ptr.as_ref().get_prev()).is_none());
            }
            *first = ptr;
        } else {
            self.first = Some(ptr);
            unsafe {
                debug_assert!((*ptr.as_ref().get_next()).is_none());
                debug_assert!((*ptr.as_ref().get_prev()).is_none());
            }
        }
    }

    #[inline]
    fn remove(&mut self, ptr: NonNull<CcBox<()>>) {
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
            debug_assert!((*ptr.as_ref().get_next()).is_none());
            debug_assert!((*ptr.as_ref().get_prev()).is_none());
        }
    }

    #[inline]
    fn remove_first(&mut self) -> Option<NonNull<CcBox<()>>> {
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
    fn is_empty(&self) -> bool {
        self.first.is_none()
    }

    #[inline]
    fn iter(&self) -> Iter {
        self.into_iter()
    }

    #[inline]
    #[cfg(all(test, feature = "std"))] // Only used in unit tests
    fn into_iter(self) -> ListIter<List> {
        <Self as IntoIterator>::into_iter(self)
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

impl IntoIterator for List {
    type Item = NonNull<CcBox<()>>;
    type IntoIter = ListIter<List>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        ListIter {
            list: self,
        }
    }
}

pub(crate) struct Iter<'a> {
    next: Option<NonNull<CcBox<()>>>,
    _phantom: PhantomData<&'a CcBox<()>>,
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
            None => {
                None
            },
        }
    }
}

pub(crate) struct ListIter<T: ListMethods> {
    list: T,
}

impl<T: ListMethods> Iterator for ListIter<T> {
    type Item = NonNull<CcBox<()>>;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        self.list.remove_first()
    }
}

/// A [`List`] which keeps track of its size. Used in [`POSSIBLE_CYCLES`].
///
/// [`POSSIBLE_CYCLES`]: crate::POSSIBLE_CYCLES
pub(crate) struct CountedList {
    list: List,
    size: usize,
}

impl CountedList {
    #[inline]
    pub(crate) const fn new() -> CountedList {
        CountedList {
            list: List::new(),
            size: 0,
        }
    }

    #[inline]
    pub(crate) fn size(&self) -> usize {
        self.size
    }

    /// # Safety
    /// * The elements in `to_append` must be already marked with `mark` mark
    /// * `to_append_size` must be the size of `to_append`
    #[inline]
    #[cfg(feature = "finalization")]
    pub(crate) unsafe fn mark_self_and_append(&mut self, mark: Mark, to_append: List, to_append_size: usize) {
        if let Some(mut prev) = self.list.first {
            for elem in self.list.iter() {
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
            self.list.first = to_append.first;
            // to_append.first.prev is already None
        }
        self.size += to_append_size;
        core::mem::forget(to_append); // Don't run to_append destructor
    }

    /// # Safety
    /// `to_swap_size` must be the size of `to_swap`.
    #[inline]
    #[cfg(feature = "finalization")]
    pub(crate) unsafe fn swap_list(&mut self, to_swap: &mut List, to_swap_size: usize) {
        self.size = to_swap_size;
        core::mem::swap(&mut self.list, to_swap);
    }
}

impl ListMethods for CountedList {
    #[inline]
    #[cfg(all(test, feature = "std"))] // Only used in unit tests
    fn first(&self) -> Option<NonNull<CcBox<()>>> {
        self.list.first()
    }

    #[inline]
    fn add(&mut self, ptr: NonNull<CcBox<()>>) {
        self.size += 1;
        self.list.add(ptr)
    }

    #[inline]
    fn remove(&mut self, ptr: NonNull<CcBox<()>>) {
        self.size -= 1;
        self.list.remove(ptr)
    }

    #[inline]
    fn remove_first(&mut self) -> Option<NonNull<CcBox<()>>> {
        let ptr = self.list.remove_first();
        if ptr.is_some() {
            self.size -= 1;
        }
        ptr
    }

    #[inline]
    fn is_empty(&self) -> bool {
        self.size == 0
    }

    #[inline]
    #[cfg(any(feature = "pedantic-debug-assertions", all(test, feature = "std")))] // Only used in pedantic-debug-assertions or unit tests
    fn contains(&self, ptr: NonNull<CcBox<()>>) -> bool {
        self.list.contains(ptr)
    }

    #[inline]
    fn iter(&self) -> Iter {
        self.list.iter()
    }

    #[inline]
    #[cfg(all(test, feature = "std"))] // Only used in unit tests
    fn into_iter(self) -> ListIter<CountedList> {
        <Self as IntoIterator>::into_iter(self)
    }
}

impl<'a> IntoIterator for &'a CountedList {
    type Item = NonNull<CcBox<()>>;
    type IntoIter = Iter<'a>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        self.list.iter()
    }
}

impl IntoIterator for CountedList {
    type Item = NonNull<CcBox<()>>;
    type IntoIter = ListIter<CountedList>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        ListIter {
            list: self,
        }
    }
}
